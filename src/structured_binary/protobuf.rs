use super::descriptor::{
    DescriptorPool, FieldDescriptor, FieldKind, FieldLabel, MessageDescriptor,
};
use super::model::{
    BinaryEntry, BinaryRecord, BinaryScalar, BinaryValue, BinaryValueKind, ProtobufFieldIdentity,
    ProtobufUnknownField, hex,
};
use super::wire::{Cursor, DecodeError, DecodeState, locator};
use crate::core::{Diagnostic, DiagnosticCode};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn parse_record(
    bytes: &[u8],
    pool: &DescriptorPool,
    message_name: &str,
    preserve_unknown_fields: bool,
    state: &mut DecodeState<'_>,
) -> Result<BinaryRecord, DecodeError> {
    let normalized = message_name.trim().trim_start_matches('.');
    if normalized.is_empty() {
        return Err(DecodeError::new(
            "protobuf.message_name_required",
            "Protobuf decoding requires a fully-qualified message name",
            0,
        ));
    }
    let descriptor = pool.messages.get(normalized).ok_or_else(|| {
        DecodeError::new(
            "protobuf.message_not_found",
            format!("message {normalized} is not present in the descriptor set"),
            0,
        )
    })?;
    let mut cursor = Cursor::new(bytes);
    let value = parse_message(
        &mut cursor,
        pool,
        descriptor,
        preserve_unknown_fields,
        state,
        1,
        0,
        format!("/messages/{normalized}"),
        1,
        None,
    )?;
    Ok(BinaryRecord {
        index: 1,
        byte_start: 0,
        byte_end: bytes.len(),
        locator: locator("protobuf-message", 1, 0, 0, bytes.len(), None),
        value,
    })
}

#[allow(clippy::too_many_arguments)]
fn parse_message(
    cursor: &mut Cursor<'_>,
    pool: &DescriptorPool,
    descriptor: &MessageDescriptor,
    preserve_unknown_fields: bool,
    state: &mut DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    depth: usize,
    enclosing_field: Option<ProtobufFieldIdentity>,
) -> Result<BinaryValue, DecodeError> {
    let start = cursor.position();
    state.start_value(depth, start)?;
    let mut entries = Vec::new();
    let mut seen = BTreeMap::<u32, usize>::new();
    let mut seen_oneofs = BTreeMap::<String, String>::new();
    while !cursor.is_empty() {
        state.check_collection(entries.len().saturating_add(1), cursor.position())?;
        let field_start = cursor.position();
        let key = cursor.read_varint("protobuf.truncated_key")?;
        let field_number = u32::try_from(key >> 3).map_err(|_| {
            DecodeError::new(
                "protobuf.invalid_field_number",
                "field number exceeds u32",
                field_start,
            )
        })?;
        let wire_type = (key & 7) as u8;
        if field_number == 0 || field_number > 536_870_911 {
            return Err(DecodeError::new(
                "protobuf.invalid_field_number",
                format!("field number {field_number} is invalid"),
                field_start,
            ));
        }
        if wire_type == 4 {
            return Err(DecodeError::new(
                "protobuf.unexpected_end_group",
                "end-group marker appeared outside a group",
                field_start,
            ));
        }
        let field = descriptor.fields.get(&field_number).cloned();
        let Some(field) = field else {
            let end = skip_value(
                cursor,
                wire_type,
                field_number,
                "protobuf.malformed_unknown",
            )?;
            if preserve_unknown_fields {
                push_unknown(
                    cursor,
                    state,
                    descriptor,
                    record_index,
                    record_start,
                    &path,
                    field_number,
                    wire_type,
                    field_start,
                    end,
                    "field number is absent from the supplied descriptor",
                    &mut entries,
                );
            } else {
                state.diagnostics.push(
                    Diagnostic::warning(
                        "grist.structured-binary.protobuf",
                        "protobuf.unknown_field_dropped",
                        format!(
                            "unknown field {field_number} in {} was dropped by explicit option",
                            descriptor.full_name
                        ),
                    )
                    .partial(),
                );
            }
            continue;
        };
        if let Some(oneof) = &field.oneof
            && let Some(previous) = seen_oneofs.insert(oneof.clone(), field.name.clone())
            && previous != field.name
        {
            state.diagnostics.push(Diagnostic::warning(
                "grist.structured-binary.protobuf",
                "protobuf.oneof_multiple_members",
                format!(
                    "oneof {oneof} in {} contains both {previous} and {} on the wire",
                    descriptor.full_name, field.name
                ),
            ));
        }
        let occurrence = seen.entry(field.number).or_default();
        *occurrence += 1;
        if field.label != FieldLabel::Repeated && *occurrence > 1 {
            state.diagnostics.push(Diagnostic::warning(
                "grist.structured-binary.protobuf",
                "protobuf.singular_field_repeated",
                format!(
                    "singular field {}.{} occurs {} times; source occurrences are retained",
                    descriptor.full_name, field.name, occurrence
                ),
            ));
        }
        let expected = field.kind.wire_type();
        let packed = wire_type == 2 && field.label == FieldLabel::Repeated && field.kind.packable();
        if wire_type != expected && !packed {
            let end = skip_value(
                cursor,
                wire_type,
                field_number,
                "protobuf.wire_type_mismatch",
            )?;
            let field_locator = locator(
                "protobuf-message",
                record_index,
                record_start,
                field_start,
                end,
                Some(field.name.clone()),
            );
            let mut diagnostic = Diagnostic::malformed(
                "grist.structured-binary.protobuf",
                format!(
                    "field {}.{} expects wire type {expected}, found {wire_type}",
                    descriptor.full_name, field.name
                ),
            )
            .with_locator(field_locator)
            .partial();
            diagnostic.code = DiagnosticCode::new("protobuf.wire_type_mismatch");
            state.diagnostics.push(diagnostic);
            if preserve_unknown_fields {
                push_unknown(
                    cursor,
                    state,
                    descriptor,
                    record_index,
                    record_start,
                    &path,
                    field_number,
                    wire_type,
                    field_start,
                    end,
                    "wire type does not match the supplied descriptor",
                    &mut entries,
                );
            }
            continue;
        }
        if packed {
            let length_offset = cursor.position();
            let length = usize::try_from(cursor.read_varint("protobuf.truncated_packed")?)
                .map_err(|_| {
                    DecodeError::new(
                        "protobuf.length_overflow",
                        "packed field length exceeds usize",
                        length_offset,
                    )
                })?;
            state.check_blob(length, length_offset)?;
            let packed_start = cursor.position();
            let packed_end = packed_start.checked_add(length).ok_or_else(|| {
                DecodeError::new(
                    "protobuf.length_overflow",
                    "packed field range overflows usize",
                    length_offset,
                )
            })?;
            let mut packed_cursor = Cursor::bounded_bytes(
                cursor,
                packed_start,
                packed_end,
                "protobuf.truncated_packed",
            )?;
            let mut packed_index = 0usize;
            while !packed_cursor.is_empty() {
                let value_start = packed_cursor.position();
                let value = decode_scalar(
                    &mut packed_cursor,
                    pool,
                    descriptor,
                    &field,
                    field.kind.wire_type(),
                    preserve_unknown_fields,
                    state,
                    record_index,
                    record_start,
                    format!("{path}/fields/{}/{}", field.name, packed_index),
                    depth + 1,
                    value_start,
                )?;
                entries.push(entry(entries.len(), &field, value, *occurrence));
                packed_index += 1;
                state.check_collection(packed_index, value_start)?;
            }
            cursor.advance_to(packed_end, "protobuf.truncated_packed")?;
        } else {
            let value = decode_scalar(
                cursor,
                pool,
                descriptor,
                &field,
                wire_type,
                preserve_unknown_fields,
                state,
                record_index,
                record_start,
                format!("{path}/fields/{}/{}", field.name, occurrence),
                depth + 1,
                field_start,
            )?;
            entries.push(entry(entries.len(), &field, value, *occurrence));
        }
    }
    let present = seen.keys().copied().collect::<BTreeSet<_>>();
    for required in descriptor
        .fields
        .values()
        .filter(|field| field.label == FieldLabel::Required)
    {
        if !present.contains(&required.number) {
            let mut diagnostic = Diagnostic::malformed(
                "grist.structured-binary.protobuf",
                format!(
                    "required proto2 field {}.{} is missing",
                    descriptor.full_name, required.name
                ),
            )
            .partial();
            diagnostic.code = DiagnosticCode::new("protobuf.required_field_missing");
            state.diagnostics.push(diagnostic);
        }
    }
    let end = cursor.position();
    Ok(BinaryValue {
        id: format!("protobuf:{path}@{start}"),
        kind: BinaryValueKind::Message,
        path: path.clone(),
        byte_start: start,
        byte_end: end,
        locator: locator(
            "protobuf-message",
            record_index,
            record_start,
            start,
            end,
            Some(path),
        ),
        scalar: None,
        entries,
        items: Vec::new(),
        cbor_tag: None,
        messagepack_extension: None,
        protobuf_field: enclosing_field,
        indefinite: false,
        recovered: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn decode_scalar(
    cursor: &mut Cursor<'_>,
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    field: &FieldDescriptor,
    wire_type: u8,
    preserve_unknown_fields: bool,
    state: &mut DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    depth: usize,
    field_start: usize,
) -> Result<BinaryValue, DecodeError> {
    let identity = field_identity(message, field);
    if field.kind == FieldKind::Group {
        let end = skip_value(cursor, wire_type, field.number, "protobuf.malformed_group")?;
        let raw = cursor.bytes_range(field_start, end, "protobuf.malformed_group")?;
        state.diagnostics.push(
            Diagnostic::warning(
                "grist.structured-binary.protobuf",
                "protobuf.group_preserved_raw",
                format!(
                    "deprecated group field {}.{} is preserved as raw bytes",
                    message.full_name, field.name
                ),
            )
            .partial(),
        );
        return Ok(BinaryValue {
            id: format!("protobuf:{path}@{field_start}"),
            kind: BinaryValueKind::Unknown,
            path: path.clone(),
            byte_start: field_start,
            byte_end: end,
            locator: locator(
                "protobuf-message",
                record_index,
                record_start,
                field_start,
                end,
                Some(field.name.clone()),
            ),
            scalar: Some(BinaryScalar::Bytes {
                hex: hex(raw),
                length: raw.len(),
            }),
            entries: Vec::new(),
            items: Vec::new(),
            cbor_tag: None,
            messagepack_extension: None,
            protobuf_field: Some(identity),
            indefinite: false,
            recovered: true,
        });
    }
    let (kind, scalar, end, nested) = match field.kind {
        FieldKind::Double => {
            let bits = cursor.read_le_u64("protobuf.truncated_fixed64")?;
            let value = f64::from_bits(bits);
            (
                BinaryValueKind::Float,
                Some(float_scalar(value, 64, &bits.to_le_bytes())),
                cursor.position(),
                None,
            )
        }
        FieldKind::Float => {
            let bits = cursor.read_le_u32("protobuf.truncated_fixed32")?;
            let value = f32::from_bits(bits) as f64;
            (
                BinaryValueKind::Float,
                Some(float_scalar(value, 32, &bits.to_le_bytes())),
                cursor.position(),
                None,
            )
        }
        FieldKind::Fixed64 => {
            let value = cursor.read_le_u64("protobuf.truncated_fixed64")?;
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Sfixed64 => {
            let value = cursor.read_le_u64("protobuf.truncated_fixed64")? as i64;
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Fixed32 => {
            let value = cursor.read_le_u32("protobuf.truncated_fixed32")?;
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Sfixed32 => {
            let value = cursor.read_le_u32("protobuf.truncated_fixed32")? as i32;
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Int64 => {
            let value = cursor.read_varint("protobuf.truncated_varint")? as i64;
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Uint64 => {
            let value = cursor.read_varint("protobuf.truncated_varint")?;
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Int32 => {
            let value = cursor.read_varint("protobuf.truncated_varint")? as u32 as i32;
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Uint32 => {
            let value = cursor.read_varint("protobuf.truncated_varint")? as u32;
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Sint32 => {
            let raw = cursor.read_varint("protobuf.truncated_varint")? as u32;
            let value = ((raw >> 1) as i32) ^ -((raw & 1) as i32);
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Sint64 => {
            let raw = cursor.read_varint("protobuf.truncated_varint")?;
            let value = ((raw >> 1) as i64) ^ -((raw & 1) as i64);
            integer_tuple(value.to_string(), cursor.position())
        }
        FieldKind::Bool => {
            let raw = cursor.read_varint("protobuf.truncated_varint")?;
            (
                BinaryValueKind::Boolean,
                Some(BinaryScalar::Boolean { value: raw != 0 }),
                cursor.position(),
                None,
            )
        }
        FieldKind::Enum => {
            let number = cursor.read_varint("protobuf.truncated_varint")? as u32 as i32;
            let name = field
                .type_name
                .as_deref()
                .and_then(|name| pool.enums.get(name))
                .and_then(|descriptor| descriptor.values.get(&number))
                .cloned();
            (
                BinaryValueKind::Enum,
                Some(BinaryScalar::Enum { number, name }),
                cursor.position(),
                None,
            )
        }
        FieldKind::String | FieldKind::Bytes | FieldKind::Message => {
            let length_offset = cursor.position();
            let length = usize::try_from(cursor.read_varint("protobuf.truncated_length")?)
                .map_err(|_| {
                    DecodeError::new(
                        "protobuf.length_overflow",
                        "length-delimited field exceeds usize",
                        length_offset,
                    )
                })?;
            state.check_blob(length, length_offset)?;
            let data_start = cursor.position();
            let data_end = data_start.checked_add(length).ok_or_else(|| {
                DecodeError::new(
                    "protobuf.length_overflow",
                    "length-delimited field range overflows usize",
                    length_offset,
                )
            })?;
            let data = cursor.bytes_range(data_start, data_end, "protobuf.truncated_length")?;
            match field.kind {
                FieldKind::String => {
                    let text = std::str::from_utf8(data).map_err(|error| {
                        DecodeError::new(
                            "protobuf.invalid_utf8",
                            format!(
                                "string field {}.{} is not UTF-8: {error}",
                                message.full_name, field.name
                            ),
                            data_start,
                        )
                    })?;
                    cursor.advance_to(data_end, "protobuf.truncated_length")?;
                    (
                        BinaryValueKind::Text,
                        Some(BinaryScalar::Text {
                            value: text.to_string(),
                        }),
                        data_end,
                        None,
                    )
                }
                FieldKind::Bytes => {
                    cursor.advance_to(data_end, "protobuf.truncated_length")?;
                    (
                        BinaryValueKind::Bytes,
                        Some(BinaryScalar::Bytes {
                            hex: hex(data),
                            length,
                        }),
                        data_end,
                        None,
                    )
                }
                FieldKind::Message => {
                    let type_name = field.type_name.as_deref().ok_or_else(|| {
                        DecodeError::new(
                            "protobuf.descriptor_invalid_field",
                            format!("message field {} has no type name", field.name),
                            field_start,
                        )
                    })?;
                    let nested_descriptor = pool.messages.get(type_name).ok_or_else(|| {
                        DecodeError::new(
                            "protobuf.descriptor_missing_type",
                            format!(
                                "message field {}.{} references missing type {type_name}",
                                message.full_name, field.name
                            ),
                            field_start,
                        )
                    })?;
                    let map_entry = nested_descriptor.map_entry;
                    let mut nested_cursor = Cursor::bounded_bytes(
                        cursor,
                        data_start,
                        data_end,
                        "protobuf.truncated_message",
                    )?;
                    let mut nested = parse_message(
                        &mut nested_cursor,
                        pool,
                        nested_descriptor,
                        preserve_unknown_fields,
                        state,
                        record_index,
                        record_start,
                        path.clone(),
                        depth + 1,
                        Some(identity.clone()),
                    )?;
                    if map_entry {
                        nested.kind = BinaryValueKind::Map;
                    }
                    cursor.advance_to(data_end, "protobuf.truncated_message")?;
                    (BinaryValueKind::Message, None, data_end, Some(nested))
                }
                _ => unreachable!(),
            }
        }
        FieldKind::Group => unreachable!("groups handled above"),
    };
    if let Some(mut nested) = nested {
        nested.byte_start = field_start;
        nested.byte_end = end;
        nested.locator = locator(
            "protobuf-message",
            record_index,
            record_start,
            field_start,
            end,
            Some(field.name.clone()),
        );
        nested.id = format!("protobuf:{path}@{field_start}");
        return Ok(nested);
    }
    Ok(BinaryValue {
        id: format!("protobuf:{path}@{field_start}"),
        kind,
        path,
        byte_start: field_start,
        byte_end: end,
        locator: locator(
            "protobuf-message",
            record_index,
            record_start,
            field_start,
            end,
            Some(field.name.clone()),
        ),
        scalar,
        entries: Vec::new(),
        items: Vec::new(),
        cbor_tag: None,
        messagepack_extension: None,
        protobuf_field: Some(identity),
        indefinite: false,
        recovered: false,
    })
}

fn integer_tuple(
    canonical: String,
    end: usize,
) -> (
    BinaryValueKind,
    Option<BinaryScalar>,
    usize,
    Option<BinaryValue>,
) {
    (
        BinaryValueKind::Integer,
        Some(BinaryScalar::Integer { canonical }),
        end,
        None,
    )
}

fn float_scalar(value: f64, width_bits: u8, raw: &[u8]) -> BinaryScalar {
    let canonical = if value.is_nan() {
        "nan".to_string()
    } else if value == f64::INFINITY {
        "infinity".to_string()
    } else if value == f64::NEG_INFINITY {
        "-infinity".to_string()
    } else {
        value.to_string()
    };
    BinaryScalar::Float {
        canonical,
        finite: value.is_finite(),
        width_bits,
        raw_bits_hex: hex(raw),
    }
}

fn field_identity(message: &MessageDescriptor, field: &FieldDescriptor) -> ProtobufFieldIdentity {
    ProtobufFieldIdentity {
        message_name: message.full_name.clone(),
        field_name: field.name.clone(),
        json_name: field.json_name.clone(),
        number: field.number,
        declared_type: field.kind.name().to_string(),
        repeated: field.label == FieldLabel::Repeated,
        packed: field.packed,
        extension: field.extension,
        oneof: field.oneof.clone(),
    }
}

fn entry(
    index: usize,
    field: &FieldDescriptor,
    mut value: BinaryValue,
    occurrence: usize,
) -> BinaryEntry {
    if let Some(identity) = &mut value.protobuf_field {
        identity.repeated = field.label == FieldLabel::Repeated;
    }
    BinaryEntry {
        index,
        key: None,
        value: Box::new(value),
        field_name: Some(field.json_name.clone()),
        field_number: Some(field.number),
        duplicate_ordinal: occurrence,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_unknown(
    cursor: &Cursor<'_>,
    state: &mut DecodeState<'_>,
    descriptor: &MessageDescriptor,
    record_index: usize,
    record_start: usize,
    path: &str,
    field_number: u32,
    wire_type: u8,
    start: usize,
    end: usize,
    reason: &str,
    entries: &mut Vec<BinaryEntry>,
) {
    let raw = cursor
        .bytes_range(start, end, "protobuf.unknown_field_range")
        .unwrap_or_default();
    let field_path = format!(
        "{path}/unknown/{field_number}/{}",
        state.unknown_fields.len()
    );
    let field_locator = locator(
        "protobuf-message",
        record_index,
        record_start,
        start,
        end,
        Some(field_number.to_string()),
    );
    state.unknown_fields.push(ProtobufUnknownField {
        message_name: descriptor.full_name.clone(),
        field_number,
        wire_type,
        raw_hex: hex(raw),
        byte_start: start,
        byte_end: end,
        locator: field_locator.clone(),
        reason: reason.to_string(),
    });
    let value = BinaryValue {
        id: format!("protobuf:{field_path}@{start}"),
        kind: BinaryValueKind::Unknown,
        path: field_path,
        byte_start: start,
        byte_end: end,
        locator: field_locator,
        scalar: Some(BinaryScalar::Bytes {
            hex: hex(raw),
            length: raw.len(),
        }),
        entries: Vec::new(),
        items: Vec::new(),
        cbor_tag: None,
        messagepack_extension: None,
        protobuf_field: None,
        indefinite: false,
        recovered: false,
    };
    entries.push(BinaryEntry {
        index: entries.len(),
        key: None,
        value: Box::new(value),
        field_name: Some(format!("$unknown_{field_number}")),
        field_number: Some(field_number),
        duplicate_ordinal: state
            .unknown_fields
            .iter()
            .filter(|unknown| {
                unknown.message_name == descriptor.full_name && unknown.field_number == field_number
            })
            .count(),
    });
}

fn skip_value(
    cursor: &mut Cursor<'_>,
    wire_type: u8,
    field_number: u32,
    code: &'static str,
) -> Result<usize, DecodeError> {
    match wire_type {
        0 => {
            cursor.read_varint(code)?;
        }
        1 => {
            cursor.take(8, code)?;
        }
        2 => {
            let offset = cursor.position();
            let length = usize::try_from(cursor.read_varint(code)?)
                .map_err(|_| DecodeError::new(code, "field length exceeds usize", offset))?;
            cursor.take(length, code)?;
        }
        3 => loop {
            let start = cursor.position();
            let key = cursor.read_varint(code)?;
            let nested_number = u32::try_from(key >> 3)
                .map_err(|_| DecodeError::new(code, "group field number exceeds u32", start))?;
            let nested_wire = (key & 7) as u8;
            if nested_wire == 4 {
                if nested_number != field_number {
                    return Err(DecodeError::new(
                        code,
                        format!("group {field_number} ended with field number {nested_number}"),
                        start,
                    ));
                }
                break;
            }
            skip_value(cursor, nested_wire, nested_number, code)?;
        },
        4 => {
            return Err(DecodeError::new(
                code,
                "unexpected end-group marker",
                cursor.position(),
            ));
        }
        5 => {
            cursor.take(4, code)?;
        }
        _ => {
            return Err(DecodeError::new(
                code,
                format!("reserved wire type {wire_type}"),
                cursor.position(),
            ));
        }
    }
    Ok(cursor.position())
}
