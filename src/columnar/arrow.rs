use super::binary::{
    DecodeError, Result, byte_locator, checked_range, read_i32, read_i64, read_u32, record_locator,
};
use super::flatbuffer::{Table, key_values};
use super::model::*;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug)]
pub(crate) struct ParsedArrow {
    pub format: ColumnarFormat,
    pub version: String,
    pub schema: ColumnarSchema,
    pub metadata: BTreeMap<String, String>,
    pub batches: Vec<ColumnarBatch>,
    pub dictionaries: Vec<ColumnarDictionary>,
}

pub(crate) fn is_arrow(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"ARROW1") {
        return true;
    }
    let Ok((prefix, len, false)) = message_length(bytes, 0) else {
        return false;
    };
    let Ok(metadata) = checked_range(bytes, prefix, len) else {
        return false;
    };
    let Ok(message) = Table::root(metadata, prefix) else {
        return false;
    };
    message.u8(1, 0).is_ok_and(|kind| (1..=6).contains(&kind))
        && message.table(2).is_ok_and(|header| header.is_some())
}

pub(crate) fn parse(bytes: &[u8], options: &ColumnarOptions) -> Result<ParsedArrow> {
    let file = bytes.len() >= 10 && bytes.starts_with(b"ARROW1") && bytes.ends_with(b"ARROW1");
    let (mut pos, end, format) = if file {
        let footer_len = read_u32(bytes, bytes.len() - 10)? as usize;
        let footer_start = bytes.len().checked_sub(10 + footer_len).ok_or_else(|| {
            DecodeError::new(
                "arrow.invalid_footer",
                "Arrow footer length exceeds input",
                bytes.len() - 10,
            )
        })?;
        (8, footer_start, ColumnarFormat::ArrowIpcFile)
    } else {
        (0, bytes.len(), ColumnarFormat::ArrowIpcStream)
    };
    let mut schema = None;
    let mut metadata = BTreeMap::new();
    let mut batches = Vec::new();
    let mut dictionaries = Vec::new();
    let mut dictionary_values: HashMap<i64, Vec<ColumnarValue>> = HashMap::new();
    let mut source_row = 0u64;
    let mut version = String::new();
    while pos < end {
        let (prefix, metadata_len, terminal) = message_length(bytes, pos)?;
        if terminal {
            break;
        }
        if metadata_len > options.max_metadata_bytes {
            return Err(DecodeError::new(
                "arrow.metadata_limit",
                "Arrow message metadata exceeds max_metadata_bytes",
                pos,
            ));
        }
        let meta_start = pos + prefix;
        let meta = checked_range(bytes, meta_start, metadata_len)?;
        let message = Table::root(meta, meta_start)?;
        version = format!("V{}", message.i16(0, 0)?);
        let header_type = message.u8(1, 0)?;
        let header = message.table(2)?.ok_or_else(|| {
            DecodeError::new(
                "arrow.missing_header",
                "Arrow message has no header",
                meta_start,
            )
        })?;
        let body_len = usize::try_from(message.i64(3, 0)?).map_err(|_| {
            DecodeError::new(
                "arrow.invalid_body_length",
                "negative Arrow body length",
                meta_start,
            )
        })?;
        let body_start = align8(meta_start + metadata_len);
        let body = checked_range(bytes, body_start, body_len)?;
        match header_type {
            1 => {
                let parsed = parse_schema(header)?;
                metadata = parsed.metadata.clone();
                schema = Some(parsed);
            }
            2 => {
                let id = header.i64(0, 0)?;
                let batch = header.table(1)?.ok_or_else(|| {
                    DecodeError::new(
                        "arrow.invalid_dictionary",
                        "dictionary message has no record batch",
                        meta_start,
                    )
                })?;
                let is_delta = header.bool(2, false)?;
                let field = find_dictionary_field(
                    schema.as_ref().ok_or_else(|| {
                        DecodeError::new(
                            "arrow.schema_required",
                            "dictionary precedes schema",
                            meta_start,
                        )
                    })?,
                    id,
                )
                .ok_or_else(|| {
                    DecodeError::new(
                        "arrow.unknown_dictionary",
                        "dictionary id is absent from schema",
                        meta_start,
                    )
                })?;
                let mut value_field = field.clone();
                value_field.dictionary = None;
                let values = decode_single_array(body, body_start, batch, &value_field, None)?;
                if is_delta {
                    dictionary_values
                        .entry(id)
                        .or_default()
                        .extend(values.clone());
                } else {
                    dictionary_values.insert(id, values.clone());
                }
                dictionaries.push(ColumnarDictionary {
                    id,
                    is_delta,
                    values,
                    locator: byte_locator(pos, body_start + body_len),
                });
            }
            3 => {
                let schema_ref = schema.as_ref().ok_or_else(|| {
                    DecodeError::new(
                        "arrow.schema_required",
                        "record batch precedes schema",
                        meta_start,
                    )
                })?;
                let length = u64::try_from(header.i64(0, 0)?).map_err(|_| {
                    DecodeError::new(
                        "arrow.invalid_row_count",
                        "negative record-batch length",
                        meta_start,
                    )
                })?;
                let batch_index = batches.len();
                if options.selects_batch(batch_index) {
                    batches.push(decode_batch(
                        body,
                        body_start,
                        header,
                        schema_ref,
                        &dictionary_values,
                        options,
                        batch_index + 1,
                        source_row,
                    )?);
                }
                source_row = source_row.saturating_add(length);
            }
            4..=6 => {
                return Err(DecodeError::new(
                    "arrow.unsupported_message",
                    "Arrow tensor/sparse-tensor messages are outside the tabular IPC contract",
                    meta_start,
                ));
            }
            _ => {
                return Err(DecodeError::new(
                    "arrow.unknown_message",
                    format!("unknown Arrow IPC message header type {header_type}"),
                    meta_start,
                ));
            }
        }
        pos = align8(body_start + body_len);
    }
    let schema = schema.ok_or_else(|| {
        DecodeError::new(
            "arrow.schema_required",
            "Arrow IPC input contains no schema message",
            0,
        )
    })?;
    Ok(ParsedArrow {
        format,
        version,
        schema,
        metadata,
        batches,
        dictionaries,
    })
}

fn message_length(bytes: &[u8], pos: usize) -> Result<(usize, usize, bool)> {
    let first = read_u32(bytes, pos)?;
    if first == 0 {
        return Ok((4, 0, true));
    }
    if first == u32::MAX {
        let len = read_u32(bytes, pos + 4)? as usize;
        Ok((8, len, len == 0))
    } else {
        Ok((4, first as usize, false))
    }
}
fn align8(value: usize) -> usize {
    value.saturating_add(7) & !7
}

fn parse_schema(table: Table<'_>) -> Result<ColumnarSchema> {
    let fields = table
        .table_vec(1)?
        .into_iter()
        .map(parse_field)
        .collect::<Result<Vec<_>>>()?;
    Ok(ColumnarSchema {
        fields,
        metadata: key_values(&table, 2)?,
    })
}
fn parse_field(table: Table<'_>) -> Result<ColumnarField> {
    let name = table.string(0)?.unwrap_or_default();
    let nullable = table.bool(1, false)?;
    let type_id = table.u8(2, 0)?;
    let ty = table.table(3)?;
    let children = table
        .table_vec(5)?
        .into_iter()
        .map(parse_field)
        .collect::<Result<Vec<_>>>()?;
    let dictionary = table.table(4)?.map(|d| parse_dictionary(d)).transpose()?;
    Ok(ColumnarField {
        name,
        nullable,
        data_type: parse_type(type_id, ty)?,
        children,
        metadata: key_values(&table, 6)?,
        dictionary,
        parquet: None,
    })
}
fn parse_dictionary(table: Table<'_>) -> Result<DictionaryEncoding> {
    let index = table.table(1)?.ok_or_else(|| {
        DecodeError::new(
            "arrow.invalid_dictionary",
            "dictionary index type missing",
            0,
        )
    })?;
    let bits = index.i32(0, 32)? as u16;
    let index_type = if index.bool(1, true)? {
        ColumnarDataType::SignedInteger { bit_width: bits }
    } else {
        ColumnarDataType::UnsignedInteger { bit_width: bits }
    };
    Ok(DictionaryEncoding {
        id: table.i64(0, 0)?,
        index_type: Box::new(index_type),
        ordered: table.bool(2, false)?,
    })
}
fn parse_type(id: u8, table: Option<Table<'_>>) -> Result<ColumnarDataType> {
    let need = || {
        table.ok_or_else(|| {
            DecodeError::new("arrow.invalid_type", "Arrow type metadata is missing", 0)
        })
    };
    Ok(match id {
        1 => ColumnarDataType::Null,
        2 => {
            let t = need()?;
            let bits = t.i32(0, 0)? as u16;
            if t.bool(1, false)? {
                ColumnarDataType::SignedInteger { bit_width: bits }
            } else {
                ColumnarDataType::UnsignedInteger { bit_width: bits }
            }
        }
        3 => {
            let p = need()?.i16(0, 2)?;
            ColumnarDataType::Float {
                bit_width: match p {
                    0 => 16,
                    1 => 32,
                    _ => 64,
                },
            }
        }
        4 => ColumnarDataType::Binary,
        5 => ColumnarDataType::Utf8,
        6 => ColumnarDataType::Boolean,
        7 => {
            let t = need()?;
            ColumnarDataType::Decimal {
                precision: t.i32(0, 0)? as u32,
                scale: t.i32(1, 0)?,
                bit_width: t.i32(2, 128)? as u16,
            }
        }
        8 => ColumnarDataType::Date {
            unit: match need()?.i16(0, 0)? {
                0 => "day",
                _ => "millisecond",
            }
            .into(),
        },
        9 => {
            let t = need()?;
            ColumnarDataType::Time {
                unit: time_unit(t.i16(0, 0)?).into(),
                bit_width: t.i32(1, 32)? as u16,
            }
        }
        10 => {
            let t = need()?;
            ColumnarDataType::Timestamp {
                unit: time_unit(t.i16(0, 0)?).into(),
                timezone: t.string(1)?,
            }
        }
        11 => ColumnarDataType::Interval {
            unit: match need()?.i16(0, 0)? {
                0 => "year_month",
                1 => "day_time",
                _ => "month_day_nano",
            }
            .into(),
        },
        12 => ColumnarDataType::List,
        13 => ColumnarDataType::Struct,
        14 => {
            let t = need()?;
            ColumnarDataType::Union {
                mode: if t.i16(0, 0)? == 0 { "sparse" } else { "dense" }.into(),
                type_ids: t.i32_vec(1)?,
            }
        }
        15 => ColumnarDataType::FixedSizeBinary {
            byte_width: need()?.i32(0, 0)? as u32,
        },
        16 => ColumnarDataType::FixedSizeList {
            length: need()?.i32(0, 0)? as u32,
        },
        17 => ColumnarDataType::Map {
            keys_sorted: need()?.bool(0, false)?,
        },
        18 => ColumnarDataType::Duration {
            unit: time_unit(need()?.i16(0, 0)?).into(),
        },
        19 => ColumnarDataType::LargeBinary,
        20 => ColumnarDataType::LargeUtf8,
        21 => ColumnarDataType::LargeList,
        _ => ColumnarDataType::Unknown { type_id: id as i32 },
    })
}
fn time_unit(unit: i16) -> &'static str {
    match unit {
        0 => "second",
        1 => "millisecond",
        2 => "microsecond",
        _ => "nanosecond",
    }
}
fn find_dictionary_field(schema: &ColumnarSchema, id: i64) -> Option<&ColumnarField> {
    fn find(fields: &[ColumnarField], id: i64) -> Option<&ColumnarField> {
        for f in fields {
            if f.dictionary.as_ref().is_some_and(|d| d.id == id) {
                return Some(f);
            }
            if let Some(v) = find(&f.children, id) {
                return Some(v);
            }
        }
        None
    }
    find(&schema.fields, id)
}

struct ArrayCursor {
    node: usize,
    buffer: usize,
}
fn decode_single_array(
    body: &[u8],
    base: usize,
    batch: Table<'_>,
    field: &ColumnarField,
    dictionaries: Option<&HashMap<i64, Vec<ColumnarValue>>>,
) -> Result<Vec<ColumnarValue>> {
    if batch.table(3)?.is_some() {
        return Err(DecodeError::new(
            "arrow.unsupported_compression",
            "compressed Arrow IPC buffers require LZ4_FRAME or ZSTD support",
            base,
        ));
    }
    let nodes = batch.struct_vec_16(1)?;
    let buffers = batch.struct_vec_16(2)?;
    let mut cursor = ArrayCursor { node: 0, buffer: 0 };
    decode_array(
        body,
        base,
        field,
        &nodes,
        &buffers,
        &mut cursor,
        dictionaries,
    )
}
#[allow(clippy::too_many_arguments)]
fn decode_batch(
    body: &[u8],
    base: usize,
    batch: Table<'_>,
    schema: &ColumnarSchema,
    dictionaries: &HashMap<i64, Vec<ColumnarValue>>,
    options: &ColumnarOptions,
    index: usize,
    row_start: u64,
) -> Result<ColumnarBatch> {
    if batch.table(3)?.is_some() {
        return Err(DecodeError::new(
            "arrow.unsupported_compression",
            "compressed Arrow IPC buffers require LZ4_FRAME or ZSTD support",
            base,
        ));
    }
    let length = u64::try_from(batch.i64(0, 0)?).map_err(|_| {
        DecodeError::new(
            "arrow.invalid_row_count",
            "negative record-batch length",
            base,
        )
    })?;
    let nodes = batch.struct_vec_16(1)?;
    let buffers = batch.struct_vec_16(2)?;
    let mut cursor = ArrayCursor { node: 0, buffer: 0 };
    let mut columns = Vec::new();
    for (field_index, field) in schema.fields.iter().enumerate() {
        let start_buffer = cursor.buffer;
        let values = decode_array(
            body,
            base,
            field,
            &nodes,
            &buffers,
            &mut cursor,
            Some(dictionaries),
        )?;
        let path = vec![field.name.clone()];
        if !options.selects_column(&path) {
            continue;
        }
        let encoded_start = buffers
            .get(start_buffer)
            .map_or(base, |b| base + b.0.max(0) as usize);
        let encoded_end = buffers
            .get(cursor.buffer.saturating_sub(1))
            .map_or(encoded_start, |b| base + (b.0 + b.1).max(0) as usize);
        let cells = values
            .into_iter()
            .enumerate()
            .filter_map(|(row, value)| {
                let absolute = row_start + row as u64;
                options.selects_row(absolute).then(|| ColumnarCell {
                    row: absolute,
                    repetition_index: 0,
                    definition_level: u8::from(!matches!(value, ColumnarValue::Null)),
                    repetition_level: 0,
                    value,
                    locator: record_locator(
                        format!("arrow.batch.{index}"),
                        absolute,
                        Some(field.name.clone()),
                    ),
                })
            })
            .collect();
        columns.push(ColumnarColumn {
            path,
            field_index,
            encoding: vec!["arrow_ipc_buffer".into()],
            compression: None,
            values: cells,
            encoded_byte_start: encoded_start,
            encoded_byte_end: encoded_end,
            locator: byte_locator(encoded_start, encoded_end),
        });
    }
    Ok(ColumnarBatch {
        index,
        kind: ColumnarBatchKind::RecordBatch,
        source_row_start: row_start,
        source_row_count: length,
        columns,
        metadata: BTreeMap::new(),
        locator: byte_locator(base, base + body.len()),
    })
}

fn decode_array(
    body: &[u8],
    base: usize,
    field: &ColumnarField,
    nodes: &[(i64, i64)],
    buffers: &[(i64, i64)],
    cursor: &mut ArrayCursor,
    dictionaries: Option<&HashMap<i64, Vec<ColumnarValue>>>,
) -> Result<Vec<ColumnarValue>> {
    let (length, _nulls) = *nodes.get(cursor.node).ok_or_else(|| {
        DecodeError::new(
            "arrow.missing_field_node",
            "record batch has fewer field nodes than schema",
            base,
        )
    })?;
    cursor.node += 1;
    let length = usize::try_from(length).map_err(|_| {
        DecodeError::new("arrow.invalid_array_length", "negative array length", base)
    })?;
    if matches!(field.data_type, ColumnarDataType::Null) {
        return Ok(vec![ColumnarValue::Null; length]);
    }
    if let ColumnarDataType::Union { mode, type_ids } = &field.data_type {
        return decode_union(
            body,
            base,
            field,
            length,
            mode,
            type_ids,
            nodes,
            buffers,
            cursor,
            dictionaries,
        );
    }
    let validity = take_buffer(body, base, buffers, cursor)?;
    if let Some(dict) = &field.dictionary {
        let index_field = ColumnarField {
            name: field.name.clone(),
            nullable: field.nullable,
            data_type: (*dict.index_type).clone(),
            children: Vec::new(),
            metadata: BTreeMap::new(),
            dictionary: None,
            parquet: None,
        };
        let mut values = decode_values_after_validity(
            body,
            base,
            &index_field,
            length,
            validity,
            nodes,
            buffers,
            cursor,
            dictionaries,
        )?;
        let dictionary = dictionaries.and_then(|d| d.get(&dict.id)).ok_or_else(|| {
            DecodeError::new(
                "arrow.dictionary_not_loaded",
                format!("dictionary {} is not loaded", dict.id),
                base,
            )
        })?;
        for value in &mut values {
            if let Some(index) = integer_value(value) {
                let resolved = dictionary.get(index as usize).cloned().ok_or_else(|| {
                    DecodeError::new(
                        "arrow.dictionary_index",
                        format!("dictionary index {index} is out of range"),
                        base,
                    )
                })?;
                *value = ColumnarValue::Dictionary {
                    id: dict.id,
                    index,
                    value: Box::new(resolved),
                }
            }
        }
        return Ok(values);
    }
    decode_values_after_validity(
        body,
        base,
        field,
        length,
        validity,
        nodes,
        buffers,
        cursor,
        dictionaries,
    )
}
#[allow(clippy::too_many_arguments)]
fn decode_values_after_validity(
    body: &[u8],
    base: usize,
    field: &ColumnarField,
    length: usize,
    validity: &[u8],
    nodes: &[(i64, i64)],
    buffers: &[(i64, i64)],
    cursor: &mut ArrayCursor,
    dictionaries: Option<&HashMap<i64, Vec<ColumnarValue>>>,
) -> Result<Vec<ColumnarValue>> {
    let valid = |i: usize| {
        validity.is_empty() || (validity.get(i / 8).copied().unwrap_or(0) & (1 << (i % 8))) != 0
    };
    let nullify = |mut values: Vec<ColumnarValue>| {
        for (i, v) in values.iter_mut().enumerate() {
            if !valid(i) {
                *v = ColumnarValue::Null
            }
        }
        values
    };
    Ok(match &field.data_type {
        ColumnarDataType::Null => vec![ColumnarValue::Null; length],
        ColumnarDataType::Boolean => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            nullify(
                (0..length)
                    .map(|i| ColumnarValue::Boolean {
                        value: raw.get(i / 8).copied().unwrap_or(0) & (1 << (i % 8)) != 0,
                    })
                    .collect(),
            )
        }
        ColumnarDataType::SignedInteger { bit_width } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            nullify(decode_ints(raw, length, *bit_width, true)?)
        }
        ColumnarDataType::UnsignedInteger { bit_width } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            nullify(decode_ints(raw, length, *bit_width, false)?)
        }
        ColumnarDataType::Float { bit_width } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            nullify(decode_floats(raw, length, *bit_width)?)
        }
        ColumnarDataType::Utf8
        | ColumnarDataType::Binary
        | ColumnarDataType::LargeUtf8
        | ColumnarDataType::LargeBinary => {
            let offsets = take_buffer(body, base, buffers, cursor)?;
            let raw = take_buffer(body, base, buffers, cursor)?;
            let large = matches!(
                field.data_type,
                ColumnarDataType::LargeUtf8 | ColumnarDataType::LargeBinary
            );
            let mut out = Vec::with_capacity(length);
            for i in 0..length {
                let a = offset(offsets, i, large)?;
                let b = offset(offsets, i + 1, large)?;
                let value = checked_range(raw, a, b.saturating_sub(a))?;
                out.push(
                    if matches!(
                        field.data_type,
                        ColumnarDataType::Utf8 | ColumnarDataType::LargeUtf8
                    ) {
                        ColumnarValue::Utf8 {
                            value: String::from_utf8(value.to_vec()).map_err(|_| {
                                DecodeError::new(
                                    "arrow.invalid_utf8",
                                    "invalid UTF-8 array value",
                                    base + a,
                                )
                            })?,
                        }
                    } else {
                        ColumnarValue::Binary {
                            hex: super::model::hex(value),
                            length: value.len(),
                        }
                    },
                )
            }
            nullify(out)
        }
        ColumnarDataType::FixedSizeBinary { byte_width } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            let width = *byte_width as usize;
            let mut out = Vec::new();
            for i in 0..length {
                let v = checked_range(raw, i * width, width)?;
                out.push(ColumnarValue::Binary {
                    hex: super::model::hex(v),
                    length: v.len(),
                })
            }
            nullify(out)
        }
        ColumnarDataType::Date { unit } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            let width = if unit == "day" { 32 } else { 64 };
            nullify(
                decode_signed(raw, length, width)?
                    .into_iter()
                    .map(|v| ColumnarValue::Date {
                        value: v,
                        unit: unit.clone(),
                    })
                    .collect(),
            )
        }
        ColumnarDataType::Time { unit, bit_width } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            nullify(
                decode_signed(raw, length, *bit_width)?
                    .into_iter()
                    .map(|v| ColumnarValue::Time {
                        value: v,
                        unit: unit.clone(),
                    })
                    .collect(),
            )
        }
        ColumnarDataType::Timestamp { unit, timezone } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            nullify(
                decode_signed(raw, length, 64)?
                    .into_iter()
                    .map(|v| ColumnarValue::Timestamp {
                        value: v,
                        unit: unit.clone(),
                        timezone: timezone.clone(),
                    })
                    .collect(),
            )
        }
        ColumnarDataType::Duration { unit } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            nullify(
                decode_signed(raw, length, 64)?
                    .into_iter()
                    .map(|v| ColumnarValue::Duration {
                        value: v,
                        unit: unit.clone(),
                    })
                    .collect(),
            )
        }
        ColumnarDataType::Interval { unit } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            let width = match unit.as_str() {
                "year_month" => 4,
                "day_time" => 8,
                _ => 16,
            };
            let mut output = Vec::with_capacity(length);
            for index in 0..length {
                let value = checked_range(raw, index * width, width)?;
                let canonical = match unit.as_str() {
                    "year_month" => {
                        i32::from_le_bytes(value.try_into().expect("four bytes")).to_string()
                    }
                    "day_time" => format!(
                        "days={},milliseconds={}",
                        i32::from_le_bytes(value[0..4].try_into().expect("four bytes")),
                        i32::from_le_bytes(value[4..8].try_into().expect("four bytes"))
                    ),
                    _ => format!(
                        "months={},days={},nanoseconds={}",
                        i32::from_le_bytes(value[0..4].try_into().expect("four bytes")),
                        i32::from_le_bytes(value[4..8].try_into().expect("four bytes")),
                        i64::from_le_bytes(value[8..16].try_into().expect("eight bytes"))
                    ),
                };
                output.push(ColumnarValue::Interval { canonical });
            }
            nullify(output)
        }
        ColumnarDataType::Decimal {
            precision,
            scale,
            bit_width,
        } => {
            let raw = take_buffer(body, base, buffers, cursor)?;
            let width = (*bit_width / 8) as usize;
            if width > 16 {
                return Err(DecodeError::new(
                    "arrow.decimal256_unsupported",
                    "decimal256 exceeds built-in integer width",
                    base,
                ));
            }
            let mut out = Vec::new();
            for i in 0..length {
                let chunk = checked_range(raw, i * width, width)?;
                let negative = chunk.last().is_some_and(|b| b & 0x80 != 0);
                let mut padded = if negative { [0xff; 16] } else { [0; 16] };
                padded[..width].copy_from_slice(chunk);
                out.push(ColumnarValue::Decimal {
                    unscaled: i128::from_le_bytes(padded).to_string(),
                    precision: *precision,
                    scale: *scale,
                })
            }
            nullify(out)
        }
        ColumnarDataType::List | ColumnarDataType::LargeList | ColumnarDataType::Map { .. } => {
            let offsets = take_buffer(body, base, buffers, cursor)?;
            let large = matches!(field.data_type, ColumnarDataType::LargeList);
            let child = field.children.first().ok_or_else(|| {
                DecodeError::new(
                    "arrow.missing_child",
                    "nested array has no child field",
                    base,
                )
            })?;
            let child_values =
                decode_array(body, base, child, nodes, buffers, cursor, dictionaries)?;
            let mut out = Vec::new();
            for i in 0..length {
                let a = offset(offsets, i, large)?;
                let b = offset(offsets, i + 1, large)?;
                let values = child_values
                    .get(a..b)
                    .ok_or_else(|| {
                        DecodeError::new(
                            "arrow.invalid_offsets",
                            "nested offsets exceed child array",
                            base,
                        )
                    })?
                    .to_vec();
                if matches!(field.data_type, ColumnarDataType::Map { .. }) {
                    let entries = values
                        .into_iter()
                        .map(|value| {
                            let ColumnarValue::Struct { fields } = value else {
                                return Err(DecodeError::new(
                                    "arrow.invalid_map",
                                    "map entries child must be a struct",
                                    base,
                                ));
                            };
                            let key = fields
                                .iter()
                                .find(|field| field.name == "key")
                                .map(|field| field.value.clone())
                                .unwrap_or(ColumnarValue::Null);
                            let value = fields
                                .iter()
                                .find(|field| field.name == "value")
                                .map(|field| field.value.clone())
                                .unwrap_or(ColumnarValue::Null);
                            Ok(ColumnarMapEntry { key, value })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    out.push(ColumnarValue::Map { entries });
                } else {
                    out.push(ColumnarValue::List { values });
                }
            }
            nullify(out)
        }
        ColumnarDataType::FixedSizeList { length: list_len } => {
            let child = field.children.first().ok_or_else(|| {
                DecodeError::new("arrow.missing_child", "fixed-size list has no child", base)
            })?;
            let child_values =
                decode_array(body, base, child, nodes, buffers, cursor, dictionaries)?;
            let width = *list_len as usize;
            let mut out = Vec::new();
            for i in 0..length {
                out.push(ColumnarValue::List {
                    values: child_values
                        .get(i * width..(i + 1) * width)
                        .ok_or_else(|| {
                            DecodeError::new(
                                "arrow.invalid_child_length",
                                "fixed-size list child too short",
                                base,
                            )
                        })?
                        .to_vec(),
                })
            }
            nullify(out)
        }
        ColumnarDataType::Struct => {
            let child_arrays = field
                .children
                .iter()
                .map(|child| decode_array(body, base, child, nodes, buffers, cursor, dictionaries))
                .collect::<Result<Vec<_>>>()?;
            let mut out = Vec::new();
            for row in 0..length {
                out.push(ColumnarValue::Struct {
                    fields: field
                        .children
                        .iter()
                        .zip(&child_arrays)
                        .map(|(f, a)| ColumnarNamedValue {
                            name: f.name.clone(),
                            value: a.get(row).cloned().unwrap_or(ColumnarValue::Null),
                        })
                        .collect(),
                })
            }
            nullify(out)
        }
        other => {
            return Err(DecodeError::new(
                "arrow.unsupported_type",
                format!("decoding {other:?} is not supported"),
                base,
            ));
        }
    })
}
#[allow(clippy::too_many_arguments)]
fn decode_union(
    body: &[u8],
    base: usize,
    field: &ColumnarField,
    length: usize,
    mode: &str,
    type_ids: &[i32],
    nodes: &[(i64, i64)],
    buffers: &[(i64, i64)],
    cursor: &mut ArrayCursor,
    dictionaries: Option<&HashMap<i64, Vec<ColumnarValue>>>,
) -> Result<Vec<ColumnarValue>> {
    let encoded_types = take_buffer(body, base, buffers, cursor)?;
    let offsets = if mode == "dense" {
        Some(take_buffer(body, base, buffers, cursor)?)
    } else {
        None
    };
    let children = field
        .children
        .iter()
        .map(|child| decode_array(body, base, child, nodes, buffers, cursor, dictionaries))
        .collect::<Result<Vec<_>>>()?;
    let mut output = Vec::with_capacity(length);
    for row in 0..length {
        let type_id = *encoded_types.get(row).ok_or_else(|| {
            DecodeError::new(
                "arrow.union_type_ids",
                "union type-id buffer is too short",
                base,
            )
        })? as i8;
        let child_index = type_ids
            .iter()
            .position(|candidate| *candidate == type_id as i32)
            .unwrap_or(type_id.max(0) as usize);
        let value_index = if let Some(offsets) = offsets {
            usize::try_from(read_i32(offsets, row * 4)?).map_err(|_| {
                DecodeError::new(
                    "arrow.union_offset",
                    "dense union has a negative offset",
                    base,
                )
            })?
        } else {
            row
        };
        let value = children
            .get(child_index)
            .and_then(|child| child.get(value_index))
            .cloned()
            .ok_or_else(|| {
                DecodeError::new(
                    "arrow.union_child",
                    "union child index is out of range",
                    base,
                )
            })?;
        output.push(ColumnarValue::Union {
            type_id,
            value: Box::new(value),
        });
    }
    Ok(output)
}

fn take_buffer<'a>(
    body: &'a [u8],
    base: usize,
    buffers: &[(i64, i64)],
    cursor: &mut ArrayCursor,
) -> Result<&'a [u8]> {
    let (offset, len) = *buffers.get(cursor.buffer).ok_or_else(|| {
        DecodeError::new(
            "arrow.missing_buffer",
            "record batch has fewer buffers than schema",
            base,
        )
    })?;
    cursor.buffer += 1;
    let offset = usize::try_from(offset)
        .map_err(|_| DecodeError::new("arrow.invalid_buffer", "negative buffer offset", base))?;
    let len = usize::try_from(len)
        .map_err(|_| DecodeError::new("arrow.invalid_buffer", "negative buffer length", base))?;
    checked_range(body, offset, len)
}
fn offset(raw: &[u8], index: usize, large: bool) -> Result<usize> {
    if large {
        usize::try_from(read_i64(raw, index * 8)?).map_err(|_| {
            DecodeError::new("arrow.invalid_offsets", "negative large offset", index * 8)
        })
    } else {
        usize::try_from(read_i32(raw, index * 4)?)
            .map_err(|_| DecodeError::new("arrow.invalid_offsets", "negative offset", index * 4))
    }
}
fn decode_signed(raw: &[u8], length: usize, bits: u16) -> Result<Vec<i64>> {
    let width = (bits / 8) as usize;
    (0..length)
        .map(|i| {
            let b = checked_range(raw, i * width, width)?;
            Ok(match bits {
                8 => b[0] as i8 as i64,
                16 => i16::from_le_bytes(b.try_into().unwrap()) as i64,
                32 => i32::from_le_bytes(b.try_into().unwrap()) as i64,
                64 => i64::from_le_bytes(b.try_into().unwrap()),
                _ => {
                    return Err(DecodeError::new(
                        "arrow.invalid_integer_width",
                        format!("integer width {bits}"),
                        i * width,
                    ));
                }
            })
        })
        .collect()
}
fn decode_ints(raw: &[u8], length: usize, bits: u16, signed: bool) -> Result<Vec<ColumnarValue>> {
    if signed {
        Ok(decode_signed(raw, length, bits)?
            .into_iter()
            .map(|v| ColumnarValue::SignedInteger {
                canonical: v.to_string(),
            })
            .collect())
    } else {
        let width = (bits / 8) as usize;
        (0..length)
            .map(|i| {
                let b = checked_range(raw, i * width, width)?;
                let v = match bits {
                    8 => b[0] as u64,
                    16 => u16::from_le_bytes(b.try_into().unwrap()) as u64,
                    32 => u32::from_le_bytes(b.try_into().unwrap()) as u64,
                    64 => u64::from_le_bytes(b.try_into().unwrap()),
                    _ => {
                        return Err(DecodeError::new(
                            "arrow.invalid_integer_width",
                            format!("integer width {bits}"),
                            i * width,
                        ));
                    }
                };
                Ok(ColumnarValue::UnsignedInteger {
                    canonical: v.to_string(),
                })
            })
            .collect()
    }
}
fn integer_value(v: &ColumnarValue) -> Option<i64> {
    match v {
        ColumnarValue::SignedInteger { canonical }
        | ColumnarValue::UnsignedInteger { canonical } => canonical.parse().ok(),
        ColumnarValue::Null => None,
        _ => None,
    }
}
fn decode_floats(raw: &[u8], length: usize, bits: u16) -> Result<Vec<ColumnarValue>> {
    (0..length)
        .map(|i| match bits {
            16 => {
                let b = checked_range(raw, i * 2, 2)?;
                let raw_bits = u16::from_le_bytes(b.try_into().expect("two bytes"));
                let value = half_to_f32(raw_bits);
                Ok(ColumnarValue::Float {
                    canonical: value.to_string(),
                    finite: value.is_finite(),
                    bit_width: 16,
                    raw_bits_hex: format!("{raw_bits:04x}"),
                })
            }
            32 => {
                let b = checked_range(raw, i * 4, 4)?;
                let raw_bits = u32::from_le_bytes(b.try_into().unwrap());
                let v = f32::from_bits(raw_bits);
                Ok(ColumnarValue::Float {
                    canonical: v.to_string(),
                    finite: v.is_finite(),
                    bit_width: 32,
                    raw_bits_hex: format!("{raw_bits:08x}"),
                })
            }
            64 => {
                let b = checked_range(raw, i * 8, 8)?;
                let raw_bits = u64::from_le_bytes(b.try_into().unwrap());
                let v = f64::from_bits(raw_bits);
                Ok(ColumnarValue::Float {
                    canonical: v.to_string(),
                    finite: v.is_finite(),
                    bit_width: 64,
                    raw_bits_hex: format!("{raw_bits:016x}"),
                })
            }
            _ => Err(DecodeError::new(
                "arrow.unsupported_float",
                format!("float width {bits}"),
                i,
            )),
        })
        .collect()
}
fn half_to_f32(bits: u16) -> f32 {
    let sign = ((bits as u32) & 0x8000) << 16;
    let exponent = (bits >> 10) & 0x1f;
    let fraction = (bits & 0x03ff) as u32;
    let value = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let shift = fraction.leading_zeros() - 21;
            sign | ((127 - 15 - shift) << 23) | ((fraction << (shift + 1)) & 0x7f_ffff)
        }
        31 => sign | 0x7f80_0000 | (fraction << 13),
        _ => sign | (((exponent as u32) + 112) << 23) | (fraction << 13),
    };
    f32::from_bits(value)
}
