use super::model::{
    BinaryEntry, BinaryRecord, BinaryScalar, BinaryValue, BinaryValueKind, CborTag,
    StructuredBinaryOptions, hex,
};
use super::wire::{Cursor, DecodeError, DecodeState, locator};
use std::collections::BTreeMap;

pub(crate) fn parse_records(
    bytes: &[u8],
    options: &StructuredBinaryOptions,
    state: &mut DecodeState<'_>,
) -> Result<Vec<BinaryRecord>, DecodeError> {
    let mut cursor = Cursor::new(bytes);
    let mut records = Vec::new();
    while !cursor.is_empty() {
        if !records.is_empty() && !options.allow_sequence {
            return Err(DecodeError::new(
                "cbor.trailing_data",
                "a second CBOR data item is present while sequences are disabled",
                cursor.position(),
            ));
        }
        state.check_collection(records.len().saturating_add(1), cursor.position())?;
        let index = records.len() + 1;
        let start = cursor.position();
        let value = parse_value(
            &mut cursor,
            state,
            index,
            start,
            format!("/records/{index}/value"),
            1,
        )?;
        let end = cursor.position();
        records.push(BinaryRecord {
            index,
            byte_start: start,
            byte_end: end,
            locator: locator("cbor-sequence", index, start, start, end, None),
            value,
        });
    }
    if records.is_empty() {
        return Err(DecodeError::new(
            "cbor.empty",
            "CBOR input does not contain a data item",
            0,
        ));
    }
    Ok(records)
}

#[allow(clippy::too_many_arguments)]
fn parse_value(
    cursor: &mut Cursor<'_>,
    state: &mut DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    depth: usize,
) -> Result<BinaryValue, DecodeError> {
    let start = cursor.position();
    state.start_value(depth, start)?;
    let initial = cursor.read_u8("cbor.truncated")?;
    let major = initial >> 5;
    let additional = initial & 0x1f;
    match major {
        0 => {
            let value = argument(cursor, additional, start, false)?.expect("definite integer");
            Ok(scalar(
                BinaryValueKind::Integer,
                BinaryScalar::Integer {
                    canonical: value.to_string(),
                },
                "cbor",
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        1 => {
            let value = argument(cursor, additional, start, false)?.expect("definite integer");
            let negative = -(i128::from(value)) - 1;
            Ok(scalar(
                BinaryValueKind::Integer,
                BinaryScalar::Integer {
                    canonical: negative.to_string(),
                },
                "cbor",
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        2 => parse_bytes(
            cursor,
            state,
            record_index,
            record_start,
            path,
            start,
            additional,
        ),
        3 => parse_text(
            cursor,
            state,
            record_index,
            record_start,
            path,
            start,
            additional,
        ),
        4 => parse_array(
            cursor,
            state,
            record_index,
            record_start,
            path,
            start,
            additional,
            depth,
        ),
        5 => parse_map(
            cursor,
            state,
            record_index,
            record_start,
            path,
            start,
            additional,
            depth,
        ),
        6 => {
            let tag = argument(cursor, additional, start, false)?.expect("definite tag");
            let mut value = parse_value(
                cursor,
                state,
                record_index,
                record_start,
                path.clone(),
                depth + 1,
            )?;
            value.byte_start = start;
            value.locator = locator(
                "cbor-sequence",
                record_index,
                record_start,
                start,
                value.byte_end,
                Some(path.clone()),
            );
            value.id = format!("cbor:{path}@{start}");
            value.cbor_tag = Some(tag);
            state.tags.push(CborTag {
                tag,
                path,
                byte_start: start,
                byte_end: value.byte_end,
                locator: value.locator.clone(),
            });
            Ok(value)
        }
        7 => parse_simple(cursor, record_index, record_start, path, start, additional),
        _ => unreachable!("CBOR major type is three bits"),
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_bytes(
    cursor: &mut Cursor<'_>,
    state: &DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    additional: u8,
) -> Result<BinaryValue, DecodeError> {
    let length = argument(cursor, additional, start, true)?;
    let (data, indefinite) = if let Some(length) = length {
        let length = usize::try_from(length).map_err(|_| {
            DecodeError::new(
                "cbor.length_overflow",
                "byte string length exceeds usize",
                start,
            )
        })?;
        state.check_blob(length, start)?;
        (cursor.take(length, "cbor.truncated_bytes")?.to_vec(), false)
    } else {
        let mut output = Vec::new();
        loop {
            if cursor.peek() == Some(0xff) {
                cursor.read_u8("cbor.truncated_bytes")?;
                break;
            }
            let chunk_start = cursor.position();
            let header = cursor.read_u8("cbor.truncated_bytes")?;
            if header >> 5 != 2 || header & 0x1f == 31 {
                return Err(DecodeError::new(
                    "cbor.invalid_indefinite_chunk",
                    "indefinite byte strings require definite byte-string chunks",
                    chunk_start,
                ));
            }
            let chunk_len =
                argument(cursor, header & 0x1f, chunk_start, false)?.expect("chunk is definite");
            let chunk_len = usize::try_from(chunk_len).map_err(|_| {
                DecodeError::new(
                    "cbor.length_overflow",
                    "chunk length exceeds usize",
                    chunk_start,
                )
            })?;
            let total = output.len().checked_add(chunk_len).ok_or_else(|| {
                DecodeError::new(
                    "cbor.length_overflow",
                    "chunked byte length overflow",
                    chunk_start,
                )
            })?;
            state.check_blob(total, chunk_start)?;
            output.extend_from_slice(cursor.take(chunk_len, "cbor.truncated_bytes")?);
        }
        (output, true)
    };
    let mut value = scalar(
        BinaryValueKind::Bytes,
        BinaryScalar::Bytes {
            hex: hex(&data),
            length: data.len(),
        },
        "cbor",
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
    );
    value.indefinite = indefinite;
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn parse_text(
    cursor: &mut Cursor<'_>,
    state: &DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    additional: u8,
) -> Result<BinaryValue, DecodeError> {
    let length = argument(cursor, additional, start, true)?;
    let (data, indefinite) = if let Some(length) = length {
        let length = usize::try_from(length).map_err(|_| {
            DecodeError::new("cbor.length_overflow", "text length exceeds usize", start)
        })?;
        state.check_blob(length, start)?;
        (cursor.take(length, "cbor.truncated_text")?.to_vec(), false)
    } else {
        let mut output = Vec::new();
        loop {
            if cursor.peek() == Some(0xff) {
                cursor.read_u8("cbor.truncated_text")?;
                break;
            }
            let chunk_start = cursor.position();
            let header = cursor.read_u8("cbor.truncated_text")?;
            if header >> 5 != 3 || header & 0x1f == 31 {
                return Err(DecodeError::new(
                    "cbor.invalid_indefinite_chunk",
                    "indefinite text requires definite text-string chunks",
                    chunk_start,
                ));
            }
            let chunk_len =
                argument(cursor, header & 0x1f, chunk_start, false)?.expect("chunk is definite");
            let chunk_len = usize::try_from(chunk_len).map_err(|_| {
                DecodeError::new(
                    "cbor.length_overflow",
                    "chunk length exceeds usize",
                    chunk_start,
                )
            })?;
            let total = output.len().checked_add(chunk_len).ok_or_else(|| {
                DecodeError::new(
                    "cbor.length_overflow",
                    "chunked text length overflow",
                    chunk_start,
                )
            })?;
            state.check_blob(total, chunk_start)?;
            output.extend_from_slice(cursor.take(chunk_len, "cbor.truncated_text")?);
        }
        (output, true)
    };
    let text = String::from_utf8(data).map_err(|error| {
        DecodeError::new(
            "cbor.invalid_utf8",
            format!("CBOR text string is not UTF-8: {error}"),
            start,
        )
    })?;
    let mut value = scalar(
        BinaryValueKind::Text,
        BinaryScalar::Text { value: text },
        "cbor",
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
    );
    value.indefinite = indefinite;
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn parse_array(
    cursor: &mut Cursor<'_>,
    state: &mut DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    additional: u8,
    depth: usize,
) -> Result<BinaryValue, DecodeError> {
    let declared = argument(cursor, additional, start, true)?
        .map(|value| {
            usize::try_from(value).map_err(|_| {
                DecodeError::new("cbor.length_overflow", "array length exceeds usize", start)
            })
        })
        .transpose()?;
    if let Some(count) = declared {
        state.check_collection(count, start)?;
    }
    let mut items = Vec::with_capacity(declared.unwrap_or(0).min(4096));
    loop {
        if let Some(count) = declared {
            if items.len() == count {
                break;
            }
        } else if cursor.peek() == Some(0xff) {
            cursor.read_u8("cbor.truncated_array")?;
            break;
        }
        state.check_collection(items.len().saturating_add(1), cursor.position())?;
        let index = items.len();
        items.push(parse_value(
            cursor,
            state,
            record_index,
            record_start,
            format!("{path}/{index}"),
            depth + 1,
        )?);
    }
    Ok(container(
        BinaryValueKind::Array,
        "cbor",
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
        Vec::new(),
        items,
        declared.is_none(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn parse_map(
    cursor: &mut Cursor<'_>,
    state: &mut DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    additional: u8,
    depth: usize,
) -> Result<BinaryValue, DecodeError> {
    let declared = argument(cursor, additional, start, true)?
        .map(|value| {
            usize::try_from(value).map_err(|_| {
                DecodeError::new("cbor.length_overflow", "map length exceeds usize", start)
            })
        })
        .transpose()?;
    if let Some(count) = declared {
        state.check_collection(count, start)?;
    }
    let mut entries = Vec::with_capacity(declared.unwrap_or(0).min(4096));
    let mut occurrences = BTreeMap::<String, usize>::new();
    loop {
        if let Some(count) = declared {
            if entries.len() == count {
                break;
            }
        } else if cursor.peek() == Some(0xff) {
            cursor.read_u8("cbor.truncated_map")?;
            break;
        }
        state.check_collection(entries.len().saturating_add(1), cursor.position())?;
        let index = entries.len();
        let key = parse_value(
            cursor,
            state,
            record_index,
            record_start,
            format!("{path}/entries/{index}/key"),
            depth + 1,
        )?;
        if cursor.peek() == Some(0xff) {
            return Err(DecodeError::new(
                "cbor.map_missing_value",
                "indefinite map ended after a key without a value",
                cursor.position(),
            ));
        }
        let value = parse_value(
            cursor,
            state,
            record_index,
            record_start,
            format!("{path}/entries/{index}/value"),
            depth + 1,
        )?;
        let identity = serde_json::to_string(&key.json_projection()).unwrap_or_default();
        let ordinal = occurrences.entry(identity).or_default();
        *ordinal += 1;
        entries.push(BinaryEntry {
            index,
            key: Some(Box::new(key)),
            value: Box::new(value),
            field_name: None,
            field_number: None,
            duplicate_ordinal: *ordinal,
        });
    }
    Ok(container(
        BinaryValueKind::Map,
        "cbor",
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
        entries,
        Vec::new(),
        declared.is_none(),
    ))
}

fn parse_simple(
    cursor: &mut Cursor<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    additional: u8,
) -> Result<BinaryValue, DecodeError> {
    let (kind, scalar_value) = match additional {
        0..=19 => (
            BinaryValueKind::Simple,
            BinaryScalar::Simple { value: additional },
        ),
        20 => (
            BinaryValueKind::Boolean,
            BinaryScalar::Boolean { value: false },
        ),
        21 => (
            BinaryValueKind::Boolean,
            BinaryScalar::Boolean { value: true },
        ),
        22 => (BinaryValueKind::Null, BinaryScalar::Null),
        23 => (BinaryValueKind::Undefined, BinaryScalar::Undefined),
        24 => {
            let value = cursor.read_u8("cbor.truncated_simple")?;
            if value < 32 {
                return Err(DecodeError::new(
                    "cbor.noncanonical_simple",
                    "two-byte simple value must be at least 32",
                    start,
                ));
            }
            (BinaryValueKind::Simple, BinaryScalar::Simple { value })
        }
        25 => {
            let bits = cursor.read_be_u16("cbor.truncated_float")?;
            let value = half_to_f64(bits);
            (
                BinaryValueKind::Float,
                float_scalar(value, 16, &bits.to_be_bytes()),
            )
        }
        26 => {
            let bits = cursor.read_be_u32("cbor.truncated_float")?;
            let value = f32::from_bits(bits) as f64;
            (
                BinaryValueKind::Float,
                float_scalar(value, 32, &bits.to_be_bytes()),
            )
        }
        27 => {
            let bits = cursor.read_be_u64("cbor.truncated_float")?;
            let value = f64::from_bits(bits);
            (
                BinaryValueKind::Float,
                float_scalar(value, 64, &bits.to_be_bytes()),
            )
        }
        28..=30 => {
            return Err(DecodeError::new(
                "cbor.reserved_additional_info",
                format!("reserved CBOR additional information {additional}"),
                start,
            ));
        }
        31 => {
            return Err(DecodeError::new(
                "cbor.unexpected_break",
                "break marker is only valid inside an indefinite item",
                start,
            ));
        }
        _ => unreachable!(),
    };
    Ok(scalar(
        kind,
        scalar_value,
        "cbor",
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
    ))
}

fn argument(
    cursor: &mut Cursor<'_>,
    additional: u8,
    start: usize,
    allow_indefinite: bool,
) -> Result<Option<u64>, DecodeError> {
    Ok(match additional {
        0..=23 => Some(u64::from(additional)),
        24 => Some(u64::from(cursor.read_u8("cbor.truncated_argument")?)),
        25 => Some(u64::from(cursor.read_be_u16("cbor.truncated_argument")?)),
        26 => Some(u64::from(cursor.read_be_u32("cbor.truncated_argument")?)),
        27 => Some(cursor.read_be_u64("cbor.truncated_argument")?),
        28..=30 => {
            return Err(DecodeError::new(
                "cbor.reserved_additional_info",
                format!("reserved CBOR additional information {additional}"),
                start,
            ));
        }
        31 if allow_indefinite => None,
        31 => {
            return Err(DecodeError::new(
                "cbor.invalid_indefinite",
                "indefinite length is invalid for this CBOR major type",
                start,
            ));
        }
        _ => unreachable!(),
    })
}

#[allow(clippy::too_many_arguments)]
fn scalar(
    kind: BinaryValueKind,
    scalar: BinaryScalar,
    collection: &str,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    end: usize,
) -> BinaryValue {
    BinaryValue {
        id: format!("{collection}:{path}@{start}"),
        kind,
        locator: locator(
            &format!("{collection}-sequence"),
            record_index,
            record_start,
            start,
            end,
            Some(path.clone()),
        ),
        path,
        byte_start: start,
        byte_end: end,
        scalar: Some(scalar),
        entries: Vec::new(),
        items: Vec::new(),
        cbor_tag: None,
        messagepack_extension: None,
        protobuf_field: None,
        indefinite: false,
        recovered: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn container(
    kind: BinaryValueKind,
    collection: &str,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    end: usize,
    entries: Vec<BinaryEntry>,
    items: Vec<BinaryValue>,
    indefinite: bool,
) -> BinaryValue {
    BinaryValue {
        id: format!("{collection}:{path}@{start}"),
        kind,
        locator: locator(
            &format!("{collection}-sequence"),
            record_index,
            record_start,
            start,
            end,
            Some(path.clone()),
        ),
        path,
        byte_start: start,
        byte_end: end,
        scalar: None,
        entries,
        items,
        cbor_tag: None,
        messagepack_extension: None,
        protobuf_field: None,
        indefinite,
        recovered: false,
    }
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

fn half_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let fraction = bits & 0x03ff;
    match exponent {
        0 if fraction == 0 => sign * 0.0,
        0 => sign * 2f64.powi(-14) * (f64::from(fraction) / 1024.0),
        31 if fraction == 0 => sign * f64::INFINITY,
        31 => f64::NAN,
        _ => sign * 2f64.powi(i32::from(exponent) - 15) * (1.0 + f64::from(fraction) / 1024.0),
    }
}
