use super::model::{
    BinaryEntry, BinaryRecord, BinaryScalar, BinaryValue, BinaryValueKind, MessagePackExtension,
    MessagePackExtensionValue, MessagePackTimestamp, StructuredBinaryOptions, hex,
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
                "messagepack.trailing_data",
                "a second MessagePack object is present while sequences are disabled",
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
            locator: locator("messagepack-sequence", index, start, start, end, None),
            value,
        });
    }
    if records.is_empty() {
        return Err(DecodeError::new(
            "messagepack.empty",
            "MessagePack input does not contain an object",
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
    let marker = cursor.read_u8("messagepack.truncated")?;
    match marker {
        0x00..=0x7f => Ok(integer(
            u64::from(marker).to_string(),
            record_index,
            record_start,
            path,
            start,
            cursor.position(),
        )),
        0x80..=0x8f => parse_map(
            cursor,
            state,
            record_index,
            record_start,
            path,
            start,
            usize::from(marker & 0x0f),
            depth,
        ),
        0x90..=0x9f => parse_array(
            cursor,
            state,
            record_index,
            record_start,
            path,
            start,
            usize::from(marker & 0x0f),
            depth,
        ),
        0xa0..=0xbf => parse_text(
            cursor,
            state,
            record_index,
            record_start,
            path,
            start,
            usize::from(marker & 0x1f),
        ),
        0xc0 => Ok(scalar(
            BinaryValueKind::Null,
            BinaryScalar::Null,
            record_index,
            record_start,
            path,
            start,
            cursor.position(),
        )),
        0xc1 => Err(DecodeError::new(
            "messagepack.reserved_marker",
            "0xc1 is reserved and is not a MessagePack value",
            start,
        )),
        0xc2 | 0xc3 => Ok(scalar(
            BinaryValueKind::Boolean,
            BinaryScalar::Boolean {
                value: marker == 0xc3,
            },
            record_index,
            record_start,
            path,
            start,
            cursor.position(),
        )),
        0xc4 => {
            let length = usize::from(cursor.read_u8("messagepack.truncated_length")?);
            parse_bytes(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xc5 => {
            let length = usize::from(cursor.read_be_u16("messagepack.truncated_length")?);
            parse_bytes(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xc6 => {
            let length = usize::try_from(cursor.read_be_u32("messagepack.truncated_length")?)
                .map_err(|_| {
                    DecodeError::new(
                        "messagepack.length_overflow",
                        "binary length exceeds usize",
                        start,
                    )
                })?;
            parse_bytes(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xc7 => {
            let length = usize::from(cursor.read_u8("messagepack.truncated_length")?);
            parse_extension(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xc8 => {
            let length = usize::from(cursor.read_be_u16("messagepack.truncated_length")?);
            parse_extension(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xc9 => {
            let length = usize::try_from(cursor.read_be_u32("messagepack.truncated_length")?)
                .map_err(|_| {
                    DecodeError::new(
                        "messagepack.length_overflow",
                        "extension length exceeds usize",
                        start,
                    )
                })?;
            parse_extension(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xca => {
            let bits = cursor.read_be_u32("messagepack.truncated_float")?;
            Ok(float(
                f32::from_bits(bits) as f64,
                32,
                &bits.to_be_bytes(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xcb => {
            let bits = cursor.read_be_u64("messagepack.truncated_float")?;
            Ok(float(
                f64::from_bits(bits),
                64,
                &bits.to_be_bytes(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xcc => {
            let value = cursor.read_u8("messagepack.truncated_integer")?;
            Ok(integer(
                value.to_string(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xcd => {
            let value = cursor.read_be_u16("messagepack.truncated_integer")?;
            Ok(integer(
                value.to_string(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xce => {
            let value = cursor.read_be_u32("messagepack.truncated_integer")?;
            Ok(integer(
                value.to_string(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xcf => {
            let value = cursor.read_be_u64("messagepack.truncated_integer")?;
            Ok(integer(
                value.to_string(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xd0 => {
            let value = cursor.read_u8("messagepack.truncated_integer")? as i8;
            Ok(integer(
                value.to_string(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xd1 => {
            let value = cursor.read_be_u16("messagepack.truncated_integer")? as i16;
            Ok(integer(
                value.to_string(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xd2 => {
            let value = cursor.read_be_u32("messagepack.truncated_integer")? as i32;
            Ok(integer(
                value.to_string(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xd3 => {
            let value = cursor.read_be_u64("messagepack.truncated_integer")? as i64;
            Ok(integer(
                value.to_string(),
                record_index,
                record_start,
                path,
                start,
                cursor.position(),
            ))
        }
        0xd4..=0xd8 => {
            let length = match marker {
                0xd4 => 1,
                0xd5 => 2,
                0xd6 => 4,
                0xd7 => 8,
                0xd8 => 16,
                _ => unreachable!(),
            };
            parse_extension(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xd9 => {
            let length = usize::from(cursor.read_u8("messagepack.truncated_length")?);
            parse_text(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xda => {
            let length = usize::from(cursor.read_be_u16("messagepack.truncated_length")?);
            parse_text(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xdb => {
            let length = usize::try_from(cursor.read_be_u32("messagepack.truncated_length")?)
                .map_err(|_| {
                    DecodeError::new(
                        "messagepack.length_overflow",
                        "string length exceeds usize",
                        start,
                    )
                })?;
            parse_text(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                length,
            )
        }
        0xdc | 0xdd => {
            let count = if marker == 0xdc {
                usize::from(cursor.read_be_u16("messagepack.truncated_length")?)
            } else {
                usize::try_from(cursor.read_be_u32("messagepack.truncated_length")?).map_err(
                    |_| {
                        DecodeError::new(
                            "messagepack.length_overflow",
                            "array length exceeds usize",
                            start,
                        )
                    },
                )?
            };
            parse_array(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                count,
                depth,
            )
        }
        0xde | 0xdf => {
            let count = if marker == 0xde {
                usize::from(cursor.read_be_u16("messagepack.truncated_length")?)
            } else {
                usize::try_from(cursor.read_be_u32("messagepack.truncated_length")?).map_err(
                    |_| {
                        DecodeError::new(
                            "messagepack.length_overflow",
                            "map length exceeds usize",
                            start,
                        )
                    },
                )?
            };
            parse_map(
                cursor,
                state,
                record_index,
                record_start,
                path,
                start,
                count,
                depth,
            )
        }
        0xe0..=0xff => Ok(integer(
            (marker as i8).to_string(),
            record_index,
            record_start,
            path,
            start,
            cursor.position(),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_text(
    cursor: &mut Cursor<'_>,
    state: &DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    length: usize,
) -> Result<BinaryValue, DecodeError> {
    state.check_blob(length, start)?;
    let bytes = cursor.take(length, "messagepack.truncated_string")?;
    let value = std::str::from_utf8(bytes).map_err(|error| {
        DecodeError::new(
            "messagepack.invalid_utf8",
            format!("MessagePack string is not UTF-8: {error}"),
            start,
        )
    })?;
    Ok(scalar(
        BinaryValueKind::Text,
        BinaryScalar::Text {
            value: value.to_string(),
        },
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn parse_bytes(
    cursor: &mut Cursor<'_>,
    state: &DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    length: usize,
) -> Result<BinaryValue, DecodeError> {
    state.check_blob(length, start)?;
    let bytes = cursor.take(length, "messagepack.truncated_binary")?;
    Ok(scalar(
        BinaryValueKind::Bytes,
        BinaryScalar::Bytes {
            hex: hex(bytes),
            length,
        },
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn parse_array(
    cursor: &mut Cursor<'_>,
    state: &mut DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    count: usize,
    depth: usize,
) -> Result<BinaryValue, DecodeError> {
    state.check_collection(count, start)?;
    let mut items = Vec::with_capacity(count.min(4096));
    for index in 0..count {
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
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
        Vec::new(),
        items,
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
    count: usize,
    depth: usize,
) -> Result<BinaryValue, DecodeError> {
    state.check_collection(count, start)?;
    let mut entries = Vec::with_capacity(count.min(4096));
    let mut occurrences = BTreeMap::<String, usize>::new();
    for index in 0..count {
        let key = parse_value(
            cursor,
            state,
            record_index,
            record_start,
            format!("{path}/entries/{index}/key"),
            depth + 1,
        )?;
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
        record_index,
        record_start,
        path,
        start,
        cursor.position(),
        entries,
        Vec::new(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn parse_extension(
    cursor: &mut Cursor<'_>,
    state: &mut DecodeState<'_>,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    length: usize,
) -> Result<BinaryValue, DecodeError> {
    state.check_blob(length, start)?;
    let type_code = cursor.read_u8("messagepack.truncated_extension")? as i8;
    let data = cursor.take(length, "messagepack.truncated_extension")?;
    let timestamp = (type_code == -1)
        .then(|| decode_timestamp(data, start))
        .transpose()?;
    let data_hex = hex(data);
    let end = cursor.position();
    let extension = MessagePackExtension {
        type_code,
        path: path.clone(),
        data_hex: data_hex.clone(),
        byte_start: start,
        byte_end: end,
        locator: locator(
            "messagepack-sequence",
            record_index,
            record_start,
            start,
            end,
            Some(path.clone()),
        ),
        timestamp: timestamp.clone(),
    };
    state.extensions.push(extension);
    let mut value = scalar(
        BinaryValueKind::Bytes,
        BinaryScalar::Bytes {
            hex: data_hex.clone(),
            length,
        },
        record_index,
        record_start,
        path,
        start,
        end,
    );
    value.messagepack_extension = Some(MessagePackExtensionValue {
        type_code,
        data_hex,
        timestamp,
    });
    Ok(value)
}

fn decode_timestamp(data: &[u8], offset: usize) -> Result<MessagePackTimestamp, DecodeError> {
    match data.len() {
        4 => Ok(MessagePackTimestamp {
            seconds: i64::from(u32::from_be_bytes(data.try_into().expect("length checked"))),
            nanoseconds: 0,
        }),
        8 => {
            let packed = u64::from_be_bytes(data.try_into().expect("length checked"));
            let nanoseconds = (packed >> 34) as u32;
            if nanoseconds >= 1_000_000_000 {
                return Err(DecodeError::new(
                    "messagepack.invalid_timestamp",
                    "timestamp nanoseconds must be below one billion",
                    offset,
                ));
            }
            Ok(MessagePackTimestamp {
                seconds: (packed & 0x3_ffff_ffff) as i64,
                nanoseconds,
            })
        }
        12 => {
            let nanoseconds = u32::from_be_bytes(data[..4].try_into().expect("length checked"));
            if nanoseconds >= 1_000_000_000 {
                return Err(DecodeError::new(
                    "messagepack.invalid_timestamp",
                    "timestamp nanoseconds must be below one billion",
                    offset,
                ));
            }
            Ok(MessagePackTimestamp {
                seconds: i64::from_be_bytes(data[4..].try_into().expect("length checked")),
                nanoseconds,
            })
        }
        length => Err(DecodeError::new(
            "messagepack.invalid_timestamp",
            format!("timestamp extension length must be 4, 8, or 12 bytes, got {length}"),
            0,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn scalar(
    kind: BinaryValueKind,
    scalar: BinaryScalar,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    end: usize,
) -> BinaryValue {
    BinaryValue {
        id: format!("messagepack:{path}@{start}"),
        kind,
        locator: locator(
            "messagepack-sequence",
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
fn integer(
    canonical: String,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    end: usize,
) -> BinaryValue {
    scalar(
        BinaryValueKind::Integer,
        BinaryScalar::Integer { canonical },
        record_index,
        record_start,
        path,
        start,
        end,
    )
}

#[allow(clippy::too_many_arguments)]
fn float(
    value: f64,
    width_bits: u8,
    raw: &[u8],
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    end: usize,
) -> BinaryValue {
    let canonical = if value.is_nan() {
        "nan".to_string()
    } else if value == f64::INFINITY {
        "infinity".to_string()
    } else if value == f64::NEG_INFINITY {
        "-infinity".to_string()
    } else {
        value.to_string()
    };
    scalar(
        BinaryValueKind::Float,
        BinaryScalar::Float {
            canonical,
            finite: value.is_finite(),
            width_bits,
            raw_bits_hex: hex(raw),
        },
        record_index,
        record_start,
        path,
        start,
        end,
    )
}

#[allow(clippy::too_many_arguments)]
fn container(
    kind: BinaryValueKind,
    record_index: usize,
    record_start: usize,
    path: String,
    start: usize,
    end: usize,
    entries: Vec<BinaryEntry>,
    items: Vec<BinaryValue>,
) -> BinaryValue {
    BinaryValue {
        id: format!("messagepack:{path}@{start}"),
        kind,
        locator: locator(
            "messagepack-sequence",
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
        indefinite: false,
        recovered: false,
    }
}
