use super::binary::{DecodeError, Result, byte_locator, checked_range, read_u32, record_locator};
use super::model::*;
use super::thrift::{self, Reader, Value};
use std::collections::BTreeMap;
use std::io::Read;

pub(crate) struct ParsedParquet {
    pub version: String,
    pub schema: ColumnarSchema,
    pub metadata: BTreeMap<String, String>,
    pub batches: Vec<ColumnarBatch>,
    pub dictionaries: Vec<ColumnarDictionary>,
}
#[derive(Clone)]
struct Elem {
    name: String,
    physical: Option<i32>,
    type_length: Option<i32>,
    repetition: i32,
    children: usize,
    converted: Option<i32>,
    scale: i32,
    precision: i32,
    field_id: Option<i32>,
}
#[derive(Clone)]
struct Leaf {
    field: ColumnarField,
    physical: i32,
    path: Vec<String>,
    max_def: u8,
    max_rep: u8,
}

pub(crate) fn parse(bytes: &[u8], options: &ColumnarOptions) -> Result<ParsedParquet> {
    if bytes.len() < 12 || !bytes.starts_with(b"PAR1") || !bytes.ends_with(b"PAR1") {
        return Err(DecodeError::new(
            "parquet.invalid_magic",
            "Parquet requires leading and trailing PAR1 signatures",
            0,
        ));
    }
    let footer_len = read_u32(bytes, bytes.len() - 8)? as usize;
    if footer_len > options.max_metadata_bytes {
        return Err(DecodeError::new(
            "parquet.metadata_limit",
            "Parquet footer exceeds max_metadata_bytes",
            bytes.len() - 8,
        ));
    }
    let footer_start = bytes.len().checked_sub(8 + footer_len).ok_or_else(|| {
        DecodeError::new(
            "parquet.invalid_footer",
            "Parquet footer length exceeds input",
            bytes.len() - 8,
        )
    })?;
    let mut reader = Reader::new(bytes, footer_start);
    let meta = reader.read_struct()?;
    if reader.pos > bytes.len() - 8 {
        return Err(DecodeError::new(
            "parquet.invalid_footer",
            "Parquet metadata overlaps trailing magic",
            reader.pos,
        ));
    }
    let version = thrift::int(&meta, 1, 0).to_string();
    let elems = thrift::structs(&meta, 2)
        .into_iter()
        .map(parse_elem)
        .collect::<Vec<_>>();
    if elems.is_empty() {
        return Err(DecodeError::new(
            "parquet.schema_required",
            "Parquet metadata contains no schema",
            footer_start,
        ));
    }
    let mut cursor = 1;
    let mut leaves = Vec::new();
    let fields = parse_children(
        &elems,
        &mut cursor,
        elems[0].children,
        &mut Vec::new(),
        0,
        0,
        &mut leaves,
    )?;
    if cursor != elems.len() {
        return Err(DecodeError::new(
            "parquet.invalid_schema",
            "Parquet flattened schema has unreachable elements",
            footer_start,
        ));
    }
    let mut metadata = parse_key_values(&meta, 5);
    if let Some(created) = thrift::string(&meta, 6) {
        metadata.insert("created_by".into(), created);
    }
    metadata.insert("num_rows".into(), thrift::int(&meta, 3, 0).to_string());
    let mut batches = Vec::new();
    let mut dictionaries = Vec::new();
    let mut source_row = 0u64;
    for (group_index, group) in thrift::structs(&meta, 4).into_iter().enumerate() {
        let row_count = u64::try_from(thrift::int(group, 3, 0)).map_err(|_| {
            DecodeError::new(
                "parquet.invalid_row_count",
                "negative row group length",
                footer_start,
            )
        })?;
        if options.selects_batch(group_index) {
            let (batch, mut dicts) = parse_row_group(
                bytes,
                group,
                &leaves,
                options,
                group_index + 1,
                source_row,
                row_count,
            )?;
            batches.push(batch);
            dictionaries.append(&mut dicts)
        }
        source_row = source_row.saturating_add(row_count);
    }
    Ok(ParsedParquet {
        version,
        schema: ColumnarSchema {
            fields,
            metadata: BTreeMap::new(),
        },
        metadata,
        batches,
        dictionaries,
    })
}
fn parse_elem(s: &BTreeMap<i16, Value>) -> Elem {
    Elem {
        name: thrift::string(s, 4).unwrap_or_default(),
        physical: thrift::field(s, 1).and_then(Value::int).map(|v| v as i32),
        type_length: thrift::field(s, 2).and_then(Value::int).map(|v| v as i32),
        repetition: thrift::int(s, 3, 0) as i32,
        children: thrift::int(s, 5, 0).max(0) as usize,
        converted: thrift::field(s, 6).and_then(Value::int).map(|v| v as i32),
        scale: thrift::int(s, 7, 0) as i32,
        precision: thrift::int(s, 8, 0) as i32,
        field_id: thrift::field(s, 9).and_then(Value::int).map(|v| v as i32),
    }
}
fn parse_children(
    elems: &[Elem],
    cursor: &mut usize,
    count: usize,
    parent: &mut Vec<String>,
    max_def: u8,
    max_rep: u8,
    leaves: &mut Vec<Leaf>,
) -> Result<Vec<ColumnarField>> {
    let mut out = Vec::new();
    for _ in 0..count {
        let elem = elems
            .get(*cursor)
            .ok_or_else(|| {
                DecodeError::new(
                    "parquet.invalid_schema",
                    "schema child count exceeds elements",
                    0,
                )
            })?
            .clone();
        *cursor += 1;
        parent.push(elem.name.clone());
        let def = max_def + u8::from(elem.repetition != 0);
        let rep = max_rep + u8::from(elem.repetition == 2);
        let logical = converted_name(elem.converted);
        let data_type = elem.physical.map_or(ColumnarDataType::Struct, |p| {
            ColumnarDataType::ParquetPrimitive {
                physical_type: physical_name(p).into(),
                logical_type: logical.map(str::to_string),
            }
        });
        let children = parse_children(elems, cursor, elem.children, parent, def, rep, leaves)?;
        let info = ParquetFieldInfo {
            path: parent.clone(),
            repetition: repetition_name(elem.repetition).into(),
            max_definition_level: def,
            max_repetition_level: rep,
            field_id: elem.field_id,
            converted_type: logical.map(str::to_string),
            type_length: elem.type_length,
            precision: (elem.precision > 0).then_some(elem.precision),
            scale: (elem.converted == Some(5)).then_some(elem.scale),
        };
        let field = ColumnarField {
            name: elem.name.clone(),
            nullable: elem.repetition != 0,
            data_type,
            children,
            metadata: BTreeMap::new(),
            dictionary: None,
            parquet: Some(info),
        };
        if let Some(physical) = elem.physical {
            leaves.push(Leaf {
                field: field.clone(),
                physical,
                path: parent.clone(),
                max_def: def,
                max_rep: rep,
            })
        }
        out.push(field);
        parent.pop();
    }
    Ok(out)
}
fn repetition_name(v: i32) -> &'static str {
    match v {
        0 => "required",
        1 => "optional",
        2 => "repeated",
        _ => "unknown",
    }
}
fn physical_name(v: i32) -> &'static str {
    match v {
        0 => "boolean",
        1 => "int32",
        2 => "int64",
        3 => "int96",
        4 => "float",
        5 => "double",
        6 => "byte_array",
        7 => "fixed_len_byte_array",
        _ => "unknown",
    }
}
fn converted_name(v: Option<i32>) -> Option<&'static str> {
    v.map(|v| match v {
        0 => "utf8",
        1 => "map",
        2 => "map_key_value",
        3 => "list",
        4 => "enum",
        5 => "decimal",
        6 => "date",
        7 => "time_millis",
        8 => "time_micros",
        9 => "timestamp_millis",
        10 => "timestamp_micros",
        11 => "uint8",
        12 => "uint16",
        13 => "uint32",
        14 => "uint64",
        15 => "int8",
        16 => "int16",
        17 => "int32",
        18 => "int64",
        19 => "json",
        20 => "bson",
        21 => "interval",
        _ => "unknown",
    })
}
fn parse_key_values(map: &BTreeMap<i16, Value>, id: i16) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for item in thrift::structs(map, id) {
        if let Some(key) = thrift::string(item, 1) {
            out.insert(key, thrift::string(item, 2).unwrap_or_default());
        }
    }
    out
}

fn parse_row_group(
    bytes: &[u8],
    group: &BTreeMap<i16, Value>,
    leaves: &[Leaf],
    options: &ColumnarOptions,
    index: usize,
    row_start: u64,
    row_count: u64,
) -> Result<(ColumnarBatch, Vec<ColumnarDictionary>)> {
    let chunks = thrift::structs(group, 1);
    let mut columns = Vec::new();
    let mut dictionaries = Vec::new();
    let mut min = usize::MAX;
    let mut max = 0usize;
    for (column_index, chunk) in chunks.into_iter().enumerate() {
        let meta = thrift::field(chunk, 3)
            .and_then(Value::structure)
            .ok_or_else(|| {
                DecodeError::new("parquet.column_metadata", "column chunk has no metadata", 0)
            })?;
        let path = thrift::strings(meta, 3);
        let leaf = leaves
            .iter()
            .find(|leaf| leaf.path == path)
            .or_else(|| leaves.get(column_index))
            .ok_or_else(|| {
                DecodeError::new(
                    "parquet.column_schema",
                    "column chunk does not match schema",
                    0,
                )
            })?;
        if !options.selects_column(&path) {
            continue;
        }
        let start = thrift::field(meta, 11)
            .and_then(Value::int)
            .unwrap_or_else(|| thrift::int(meta, 9, 0));
        let start = usize::try_from(start).map_err(|_| {
            DecodeError::new("parquet.invalid_column_offset", "negative column offset", 0)
        })?;
        let compressed = usize::try_from(thrift::int(meta, 7, 0)).map_err(|_| {
            DecodeError::new("parquet.invalid_column_size", "negative column size", start)
        })?;
        let end = start
            .checked_add(compressed)
            .ok_or_else(|| {
                DecodeError::new("parquet.offset_overflow", "column range overflow", start)
            })?
            .min(bytes.len());
        min = min.min(start);
        max = max.max(end);
        let (codec, codec_name) = codec(thrift::int(meta, 4, 0) as i32);
        let encodings = thrift::ints(meta, 2)
            .into_iter()
            .map(|e| encoding_name(e as i32).to_string())
            .collect::<Vec<_>>();
        let total = thrift::int(meta, 5, 0).max(0) as usize;
        let (values, dict) = decode_column(
            bytes, start, end, total, leaf, codec, options, index, row_start,
        )?;
        if let Some(values) = dict {
            dictionaries.push(ColumnarDictionary {
                id: ((index as i64) << 32) | column_index as i64,
                is_delta: false,
                values,
                locator: byte_locator(start, end),
            })
        }
        columns.push(ColumnarColumn {
            path: path.clone(),
            field_index: column_index,
            encoding: encodings,
            compression: Some(codec_name.into()),
            values,
            encoded_byte_start: start,
            encoded_byte_end: end,
            locator: byte_locator(start, end),
        })
    }
    if min == usize::MAX {
        min = 0
    }
    Ok((
        ColumnarBatch {
            index,
            kind: ColumnarBatchKind::RowGroup,
            source_row_start: row_start,
            source_row_count: row_count,
            columns,
            metadata: parse_key_values(group, 4),
            locator: byte_locator(min, max),
        },
        dictionaries,
    ))
}
#[derive(Clone, Copy)]
enum Codec {
    None,
    Snappy,
    Gzip,
    Unsupported(i32),
}
fn codec(v: i32) -> (Codec, &'static str) {
    match v {
        0 => (Codec::None, "uncompressed"),
        1 => (Codec::Snappy, "snappy"),
        2 => (Codec::Gzip, "gzip"),
        3 => (Codec::Unsupported(v), "lzo"),
        4 => (Codec::Unsupported(v), "brotli"),
        5 => (Codec::Unsupported(v), "lz4"),
        6 => (Codec::Unsupported(v), "zstd"),
        7 => (Codec::Unsupported(v), "lz4_raw"),
        _ => (Codec::Unsupported(v), "unknown"),
    }
}
fn encoding_name(v: i32) -> &'static str {
    match v {
        0 => "plain",
        1 => "group_var_int",
        2 => "plain_dictionary",
        3 => "rle",
        4 => "bit_packed",
        5 => "delta_binary_packed",
        6 => "delta_length_byte_array",
        7 => "delta_byte_array",
        8 => "rle_dictionary",
        9 => "byte_stream_split",
        _ => "unknown",
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_column(
    bytes: &[u8],
    mut pos: usize,
    end: usize,
    total: usize,
    leaf: &Leaf,
    codec: Codec,
    options: &ColumnarOptions,
    group: usize,
    row_start: u64,
) -> Result<(Vec<ColumnarCell>, Option<Vec<ColumnarValue>>)> {
    let mut dictionary = None;
    let mut cells = Vec::new();
    let mut consumed = 0usize;
    let mut logical_row = row_start;
    let mut repetitions = 0u32;
    while pos < end && consumed < total {
        let mut reader = Reader::new(bytes, pos);
        let header = reader.read_struct()?;
        let page_type = thrift::int(&header, 1, -1) as i32;
        let uncompressed = usize::try_from(thrift::int(&header, 2, 0)).map_err(|_| {
            DecodeError::new("parquet.invalid_page_size", "negative page size", pos)
        })?;
        let compressed = usize::try_from(thrift::int(&header, 3, 0)).map_err(|_| {
            DecodeError::new("parquet.invalid_page_size", "negative page size", pos)
        })?;
        if compressed > options.max_encoded_page_bytes {
            return Err(DecodeError::new(
                "parquet.page_limit",
                "encoded page exceeds max_encoded_page_bytes",
                pos,
            ));
        }
        let payload = checked_range(bytes, reader.pos, compressed)?;
        let decoded = decompress(payload, codec, uncompressed, reader.pos)?;
        match page_type {
            2 => {
                let d = thrift::field(&header, 7)
                    .and_then(Value::structure)
                    .ok_or_else(|| {
                        DecodeError::new(
                            "parquet.dictionary_header",
                            "dictionary page header missing",
                            pos,
                        )
                    })?;
                let count = thrift::int(d, 1, 0).max(0) as usize;
                dictionary = Some(decode_plain(&decoded, count, leaf)?);
            }
            0 | 3 => {
                let (info, data_offset, definitions, repetitions_levels, count, encoding) =
                    if page_type == 0 {
                        let d = thrift::field(&header, 5)
                            .and_then(Value::structure)
                            .ok_or_else(|| {
                                DecodeError::new(
                                    "parquet.data_header",
                                    "data page header missing",
                                    pos,
                                )
                            })?;
                        let count = thrift::int(d, 1, 0).max(0) as usize;
                        let mut offset = 0;
                        let (reps, used) = decode_levels_v1(
                            &decoded[offset..],
                            count,
                            leaf.max_rep,
                            thrift::int(d, 4, 3) as i32,
                        )?;
                        offset += used;
                        let (defs, used) = decode_levels_v1(
                            &decoded[offset..],
                            count,
                            leaf.max_def,
                            thrift::int(d, 3, 3) as i32,
                        )?;
                        offset += used;
                        (d, offset, defs, reps, count, thrift::int(d, 2, 0) as i32)
                    } else {
                        let d = thrift::field(&header, 8)
                            .and_then(Value::structure)
                            .ok_or_else(|| {
                                DecodeError::new(
                                    "parquet.data_header",
                                    "data page v2 header missing",
                                    pos,
                                )
                            })?;
                        let count = thrift::int(d, 1, 0).max(0) as usize;
                        let rep_len = thrift::int(d, 6, 0).max(0) as usize;
                        let def_len = thrift::int(d, 5, 0).max(0) as usize;
                        let reps = decode_hybrid(
                            checked_range(&decoded, 0, rep_len)?,
                            count,
                            bit_width(leaf.max_rep),
                        )?;
                        let defs = decode_hybrid(
                            checked_range(&decoded, rep_len, def_len)?,
                            count,
                            bit_width(leaf.max_def),
                        )?;
                        (
                            d,
                            rep_len + def_len,
                            defs,
                            reps,
                            count,
                            thrift::int(d, 4, 0) as i32,
                        )
                    };
                let _ = info;
                let non_null = definitions
                    .iter()
                    .filter(|level| **level == leaf.max_def)
                    .count();
                let values = decode_encoded(
                    &decoded[data_offset..],
                    non_null,
                    leaf,
                    encoding,
                    dictionary.as_deref(),
                )?;
                let mut value_index = 0;
                for i in 0..count {
                    let rep = *repetitions_levels.get(i).unwrap_or(&0);
                    if consumed > 0 && rep == 0 {
                        logical_row = logical_row.saturating_add(1);
                        repetitions = 0
                    } else if rep > 0 {
                        repetitions = repetitions.saturating_add(1)
                    }
                    let def = *definitions.get(i).unwrap_or(&leaf.max_def);
                    let value = if def == leaf.max_def {
                        let value = values.get(value_index).cloned().ok_or_else(|| {
                            DecodeError::new(
                                "parquet.value_count",
                                "encoded page has fewer values than definition levels",
                                pos,
                            )
                        })?;
                        value_index += 1;
                        value
                    } else {
                        ColumnarValue::Null
                    };
                    if options.selects_row(logical_row) {
                        cells.push(ColumnarCell {
                            row: logical_row,
                            repetition_index: repetitions,
                            definition_level: def,
                            repetition_level: rep,
                            value,
                            locator: record_locator(
                                format!("parquet.row_group.{group}"),
                                logical_row,
                                Some(leaf.path.join(".")),
                            ),
                        })
                    }
                    consumed += 1
                }
            }
            1 => {}
            _ => {
                return Err(DecodeError::new(
                    "parquet.unknown_page",
                    format!("unknown Parquet page type {page_type}"),
                    pos,
                ));
            }
        }
        pos = reader.pos + compressed;
    }
    Ok((cells, dictionary))
}
fn decode_levels_v1(
    bytes: &[u8],
    count: usize,
    max: u8,
    encoding: i32,
) -> Result<(Vec<u8>, usize)> {
    if max == 0 {
        return Ok((vec![0; count], 0));
    }
    if encoding == 3 {
        let len = read_u32(bytes, 0)? as usize;
        let data = checked_range(bytes, 4, len)?;
        Ok((decode_hybrid(data, count, bit_width(max))?, 4 + len))
    } else {
        Err(DecodeError::new(
            "parquet.level_encoding",
            format!("unsupported level encoding {}", encoding_name(encoding)),
            0,
        ))
    }
}
fn bit_width(max: u8) -> u8 {
    if max == 0 {
        0
    } else {
        (8 - max.leading_zeros()) as u8
    }
}
fn decode_hybrid(bytes: &[u8], count: usize, width: u8) -> Result<Vec<u8>> {
    if width == 0 {
        return Ok(vec![0; count]);
    }
    let mut pos = 0;
    let mut out = Vec::with_capacity(count);
    while out.len() < count && pos < bytes.len() {
        let (header, used) = varint(&bytes[pos..])?;
        pos += used;
        if header & 1 == 0 {
            let run = (header >> 1) as usize;
            let byte_width = (width as usize).div_ceil(8);
            let raw = checked_range(bytes, pos, byte_width)?;
            pos += byte_width;
            let mut value = 0u64;
            for (i, b) in raw.iter().enumerate() {
                value |= (*b as u64) << (8 * i)
            }
            for _ in 0..run.min(count - out.len()) {
                out.push(value as u8)
            }
        } else {
            let groups = (header >> 1) as usize;
            let values = groups * 8;
            let byte_len = groups * width as usize;
            let raw = checked_range(bytes, pos, byte_len)?;
            pos += byte_len;
            for i in 0..values.min(count - out.len()) {
                let bit = i * width as usize;
                let mut value = 0u8;
                for b in 0..width as usize {
                    if raw[(bit + b) / 8] & (1 << ((bit + b) % 8)) != 0 {
                        value |= 1 << b
                    }
                }
                out.push(value)
            }
        }
    }
    if out.len() != count {
        return Err(DecodeError::new(
            "parquet.level_count",
            "hybrid stream ended before requested values",
            pos,
        ));
    }
    Ok(out)
}
fn varint(bytes: &[u8]) -> Result<(u64, usize)> {
    let mut out = 0;
    for (i, b) in bytes.iter().copied().take(10).enumerate() {
        out |= ((b & 127) as u64) << (7 * i);
        if b & 128 == 0 {
            return Ok((out, i + 1));
        }
    }
    Err(DecodeError::new(
        "parquet.invalid_varint",
        "invalid hybrid varint",
        0,
    ))
}
fn decode_encoded(
    bytes: &[u8],
    count: usize,
    leaf: &Leaf,
    encoding: i32,
    dictionary: Option<&[ColumnarValue]>,
) -> Result<Vec<ColumnarValue>> {
    match encoding {
        0 => decode_plain(bytes, count, leaf),
        2 | 8 => {
            let width = *bytes.first().ok_or_else(|| {
                DecodeError::new(
                    "parquet.dictionary_indices",
                    "missing dictionary bit width",
                    0,
                )
            })?;
            let indices = decode_hybrid(&bytes[1..], count, width)?;
            let dictionary = dictionary.ok_or_else(|| {
                DecodeError::new(
                    "parquet.dictionary_required",
                    "dictionary encoded page precedes dictionary",
                    0,
                )
            })?;
            indices
                .into_iter()
                .map(|i| {
                    dictionary.get(i as usize).cloned().ok_or_else(|| {
                        DecodeError::new(
                            "parquet.dictionary_index",
                            format!("dictionary index {i} out of range"),
                            0,
                        )
                    })
                })
                .collect()
        }
        _ => Err(DecodeError::new(
            "parquet.unsupported_encoding",
            format!(
                "value encoding {} is not supported",
                encoding_name(encoding)
            ),
            0,
        )),
    }
}
fn decode_plain(bytes: &[u8], count: usize, leaf: &Leaf) -> Result<Vec<ColumnarValue>> {
    let mut pos = 0;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let value = match leaf.physical {
            0 => {
                let value = bytes.get(i / 8).copied().unwrap_or(0) & (1 << (i % 8)) != 0;
                pos = count.div_ceil(8);
                ColumnarValue::Boolean { value }
            }
            1 => {
                let raw = checked_range(bytes, pos, 4)?;
                pos += 4;
                logical_i64(i32::from_le_bytes(raw.try_into().unwrap()) as i64, leaf)
            }
            2 => {
                let raw = checked_range(bytes, pos, 8)?;
                pos += 8;
                logical_i64(i64::from_le_bytes(raw.try_into().unwrap()), leaf)
            }
            3 => {
                let raw = checked_range(bytes, pos, 12)?;
                pos += 12;
                ColumnarValue::Binary {
                    hex: super::model::hex(raw),
                    length: 12,
                }
            }
            4 => {
                let raw = checked_range(bytes, pos, 4)?;
                pos += 4;
                let bits = u32::from_le_bytes(raw.try_into().unwrap());
                let v = f32::from_bits(bits);
                ColumnarValue::Float {
                    canonical: v.to_string(),
                    finite: v.is_finite(),
                    bit_width: 32,
                    raw_bits_hex: format!("{bits:08x}"),
                }
            }
            5 => {
                let raw = checked_range(bytes, pos, 8)?;
                pos += 8;
                let bits = u64::from_le_bytes(raw.try_into().unwrap());
                let v = f64::from_bits(bits);
                ColumnarValue::Float {
                    canonical: v.to_string(),
                    finite: v.is_finite(),
                    bit_width: 64,
                    raw_bits_hex: format!("{bits:016x}"),
                }
            }
            6 => {
                let len = read_u32(bytes, pos)? as usize;
                pos += 4;
                let raw = checked_range(bytes, pos, len)?;
                pos += len;
                binary_value(raw, leaf)?
            }
            7 => {
                let len = leaf
                    .field
                    .parquet
                    .as_ref()
                    .and_then(|p| p.type_length)
                    .unwrap_or(0)
                    .max(0) as usize;
                let raw = checked_range(bytes, pos, len)?;
                pos += len;
                binary_value(raw, leaf)?
            }
            _ => {
                return Err(DecodeError::new(
                    "parquet.physical_type",
                    "unknown physical type",
                    pos,
                ));
            }
        };
        out.push(value)
    }
    Ok(out)
}
fn logical_i64(v: i64, leaf: &Leaf) -> ColumnarValue {
    match leaf
        .field
        .parquet
        .as_ref()
        .and_then(|p| p.converted_type.as_deref())
    {
        Some("decimal") => ColumnarValue::Decimal {
            unscaled: v.to_string(),
            precision: leaf
                .field
                .parquet
                .as_ref()
                .and_then(|p| p.precision)
                .unwrap_or(0) as u32,
            scale: leaf
                .field
                .parquet
                .as_ref()
                .and_then(|p| p.scale)
                .unwrap_or(0),
        },
        Some("date") => ColumnarValue::Date {
            value: v,
            unit: "day".into(),
        },
        Some("time_millis") => ColumnarValue::Time {
            value: v,
            unit: "millisecond".into(),
        },
        Some("time_micros") => ColumnarValue::Time {
            value: v,
            unit: "microsecond".into(),
        },
        Some("timestamp_millis") => ColumnarValue::Timestamp {
            value: v,
            unit: "millisecond".into(),
            timezone: None,
        },
        Some("timestamp_micros") => ColumnarValue::Timestamp {
            value: v,
            unit: "microsecond".into(),
            timezone: None,
        },
        Some(t) if t.starts_with("uint") => ColumnarValue::UnsignedInteger {
            canonical: (v as u64).to_string(),
        },
        _ => ColumnarValue::SignedInteger {
            canonical: v.to_string(),
        },
    }
}
fn binary_value(raw: &[u8], leaf: &Leaf) -> Result<ColumnarValue> {
    match leaf
        .field
        .parquet
        .as_ref()
        .and_then(|p| p.converted_type.as_deref())
    {
        Some("utf8") | Some("json") | Some("enum") => Ok(ColumnarValue::Utf8 {
            value: String::from_utf8(raw.to_vec()).map_err(|_| {
                DecodeError::new("parquet.invalid_utf8", "invalid UTF-8 logical value", 0)
            })?,
        }),
        Some("decimal") => Ok(ColumnarValue::Decimal {
            unscaled: big_endian_signed(raw),
            precision: leaf
                .field
                .parquet
                .as_ref()
                .and_then(|p| p.precision)
                .unwrap_or(0) as u32,
            scale: leaf
                .field
                .parquet
                .as_ref()
                .and_then(|p| p.scale)
                .unwrap_or(0),
        }),
        _ => Ok(ColumnarValue::Binary {
            hex: super::model::hex(raw),
            length: raw.len(),
        }),
    }
}
fn big_endian_signed(raw: &[u8]) -> String {
    if raw.len() <= 16 {
        let negative = raw.first().is_some_and(|b| b & 128 != 0);
        let mut padded = if negative { [255; 16] } else { [0; 16] };
        padded[16 - raw.len()..].copy_from_slice(raw);
        i128::from_be_bytes(padded).to_string()
    } else {
        format!("0x{}", super::model::hex(raw))
    }
}
fn decompress(payload: &[u8], codec: Codec, expected: usize, offset: usize) -> Result<Vec<u8>> {
    let out = match codec {
        Codec::None => payload.to_vec(),
        Codec::Snappy => snappy(payload)?,
        Codec::Gzip => {
            let mut decoder = flate2::read::GzDecoder::new(payload);
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|e| DecodeError::new("parquet.gzip", e.to_string(), offset))?;
            out
        }
        Codec::Unsupported(id) => {
            return Err(DecodeError::new(
                "parquet.unsupported_codec",
                format!("compression codec {id} is not enabled"),
                offset,
            ));
        }
    };
    if out.len() != expected {
        return Err(DecodeError::new(
            "parquet.decompressed_size",
            format!("expected {expected} decompressed bytes, got {}", out.len()),
            offset,
        ));
    }
    Ok(out)
}
fn snappy(bytes: &[u8]) -> Result<Vec<u8>> {
    let (expected, mut pos) = varint(bytes)?;
    let mut out = Vec::with_capacity(expected as usize);
    while pos < bytes.len() && out.len() < expected as usize {
        let tag = bytes[pos];
        pos += 1;
        match tag & 3 {
            0 => {
                let mut len = (tag >> 2) as usize;
                if len < 60 {
                    len += 1
                } else {
                    let count = len - 59;
                    let raw = checked_range(bytes, pos, count)?;
                    pos += count;
                    len = raw
                        .iter()
                        .enumerate()
                        .fold(0usize, |n, (i, b)| n | ((*b as usize) << (8 * i)))
                        + 1
                }
                out.extend_from_slice(checked_range(bytes, pos, len)?);
                pos += len
            }
            kind => {
                let (len, offset) = match kind {
                    1 => (
                        4 + ((tag >> 2) & 7) as usize,
                        (((tag as usize) & 0xe0) << 3)
                            | *bytes.get(pos).ok_or_else(|| {
                                DecodeError::new("parquet.snappy", "truncated copy", pos)
                            })? as usize,
                    ),
                    2 => (
                        (tag >> 2) as usize + 1,
                        u16::from_le_bytes(checked_range(bytes, pos, 2)?.try_into().unwrap())
                            as usize,
                    ),
                    _ => (
                        (tag >> 2) as usize + 1,
                        u32::from_le_bytes(checked_range(bytes, pos, 4)?.try_into().unwrap())
                            as usize,
                    ),
                };
                pos += match kind {
                    1 => 1,
                    2 => 2,
                    _ => 4,
                };
                if offset == 0 || offset > out.len() {
                    return Err(DecodeError::new(
                        "parquet.snappy",
                        "invalid copy offset",
                        pos,
                    ));
                }
                for _ in 0..len {
                    let b = out[out.len() - offset];
                    out.push(b)
                }
            }
        }
    }
    if out.len() != expected as usize {
        return Err(DecodeError::new(
            "parquet.snappy",
            "decoded size mismatch",
            pos,
        ));
    }
    Ok(out)
}
