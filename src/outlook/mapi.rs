use super::cfb::{CfbEntry, CfbEntryKind, CfbFile};
use super::{
    MapiNamedProperty, MapiNamedPropertyKind, MapiProperty, MapiPropertyType, MapiValue, MsgBinary,
    MsgDate, OutlookMsgOptions,
};
use crate::core::{
    Diagnostic, IndexBase, IndexPosition, LocationComponent, SourceLocator, sha256_hex,
};
use std::collections::{BTreeMap, BTreeSet};

const PARSER: &str = "grist.outlook.msg";
const PROPERTIES_STREAM: &str = "__properties_version1.0";
const SUBSTG_PREFIX: &str = "__substg1.0_";

pub(super) struct PropertySet {
    pub properties: Vec<MapiProperty>,
    pub consumed_paths: BTreeSet<String>,
    pub codepage: u32,
}

pub(super) fn parse_named_properties(
    cfb: &CfbFile,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<MapiNamedProperty> {
    let storage = "__nameid_version1.0";
    let Some(entry_stream) = cfb.child(storage, "__substg1.0_00030102") else {
        return Vec::new();
    };
    let guid_stream = cfb
        .child(storage, "__substg1.0_00020102")
        .map(|entry| entry.data.as_slice())
        .unwrap_or_default();
    let string_stream = cfb
        .child(storage, "__substg1.0_00040102")
        .map(|entry| entry.data.as_slice())
        .unwrap_or_default();
    let mut named = Vec::new();
    for (ordinal, raw) in entry_stream.data.chunks_exact(8).enumerate() {
        let name_or_id = le_u32(raw, 0).expect("eight-byte entry");
        let guid_and_kind = le_u16(raw, 4).expect("eight-byte entry");
        let property_index = le_u16(raw, 6).expect("eight-byte entry");
        let property_id = 0x8000u16.saturating_add(property_index);
        let guid_index = usize::from(guid_and_kind >> 1);
        let property_set = match guid_index {
            1 => "00020328-0000-0000-c000-000000000046".to_string(),
            2 => "00020329-0000-0000-c000-000000000046".to_string(),
            index if index >= 3 => guid_stream
                .get((index - 3) * 16..(index - 2) * 16)
                .and_then(guid)
                .unwrap_or_else(|| format!("unknown-guid-index-{index}")),
            _ => "00020328-0000-0000-c000-000000000046".to_string(),
        };
        let kind = if guid_and_kind & 1 == 0 {
            MapiNamedPropertyKind::Numeric { id: name_or_id }
        } else if let Some(name) = named_string(string_stream, name_or_id as usize) {
            MapiNamedPropertyKind::String { name }
        } else {
            diagnostics.push(partial(
                "outlook.msg.mapi.named_string_malformed",
                format!("named property 0x{property_id:04X} has an invalid string offset"),
            ));
            MapiNamedPropertyKind::Unknown {
                raw_name_or_id: name_or_id,
            }
        };
        named.push(MapiNamedProperty {
            property_id,
            property_set,
            kind,
            locator: stream_range_locator(entry_stream, ordinal * 8, ordinal * 8 + 8),
        });
    }
    if entry_stream.data.len() % 8 != 0 {
        diagnostics.push(partial(
            "outlook.msg.mapi.named_entry_truncated",
            "named-property entry stream ends with an incomplete record",
        ));
    }
    named
}

pub(super) fn parse_property_set(
    cfb: &CfbFile,
    storage_path: &str,
    named: &[MapiNamedProperty],
    inherited_codepage: u32,
    options: &OutlookMsgOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> PropertySet {
    let mut consumed_paths = BTreeSet::new();
    let properties_stream = cfb.child(storage_path, PROPERTIES_STREAM);
    if let Some(stream) = properties_stream {
        consumed_paths.insert(stream.path.clone());
    }
    let header_size = properties_stream
        .map(|stream| property_header_size(storage_path, stream.data.len()))
        .unwrap_or(0);
    let entries = properties_stream
        .and_then(|stream| stream.data.get(header_size..))
        .unwrap_or_default();
    let codepage = entries
        .chunks_exact(16)
        .find_map(|entry| {
            let tag = le_u32(entry, 0)?;
            (tag == 0x3ffd_0003).then(|| le_u32(entry, 8)).flatten()
        })
        .unwrap_or(inherited_codepage);
    let mut properties = Vec::new();
    for (ordinal, entry) in entries
        .chunks_exact(16)
        .take(options.max_properties_per_object)
        .enumerate()
    {
        let tag = le_u32(entry, 0).expect("property entry tag");
        let flags = le_u32(entry, 4).expect("property entry flags");
        let property_id = (tag >> 16) as u16;
        let type_code = tag as u16;
        let tag_name = format!("{property_id:04X}{type_code:04X}");
        let base_name = format!("{SUBSTG_PREFIX}{tag_name}");
        let mut sources = cfb
            .direct_children(storage_path)
            .filter(|child| {
                child.name.eq_ignore_ascii_case(&base_name)
                    || child
                        .name
                        .to_ascii_uppercase()
                        .starts_with(&(base_name.to_ascii_uppercase() + "-"))
            })
            .collect::<Vec<_>>();
        sources.sort_by_key(|source| source.id);
        for source in &sources {
            consumed_paths.insert(source.path.clone());
        }
        let inline = &entry[8..16];
        let value =
            decode_property_value(type_code, inline, &sources, codepage, options, diagnostics);
        let locator = sources
            .first()
            .map(|source| stream_range_locator(source, 0, source.data.len()))
            .or_else(|| {
                properties_stream.map(|stream| {
                    stream_range_locator(
                        stream,
                        header_size + ordinal * 16,
                        header_size + ordinal * 16 + 16,
                    )
                })
            })
            .expect("property entry or value stream supplies a locator");
        properties.push(MapiProperty {
            ordinal,
            property_tag: format!("0x{tag_name}"),
            property_id,
            property_type: property_type(type_code),
            flags,
            table_value: binary(inline, true),
            canonical_name: canonical_property_name(property_id).map(str::to_string),
            named: named
                .iter()
                .find(|item| item.property_id == property_id)
                .cloned(),
            value,
            stream_paths: sources.iter().map(|source| source.path.clone()).collect(),
            locator,
        });
    }
    if entries.len() / 16 > options.max_properties_per_object {
        diagnostics.push(partial(
            "outlook.msg.limit.properties",
            format!("property set at {storage_path:?} exceeds max_properties_per_object"),
        ));
    }
    if entries.len() % 16 != 0 {
        diagnostics.push(partial(
            "outlook.msg.mapi.property_entry_truncated",
            format!("property stream at {storage_path:?} ends with an incomplete entry"),
        ));
    }
    synthesize_unlisted_properties(
        cfb,
        storage_path,
        &mut properties,
        &mut consumed_paths,
        named,
        codepage,
        options,
        diagnostics,
    );
    PropertySet {
        properties,
        consumed_paths,
        codepage,
    }
}

#[allow(clippy::too_many_arguments)]
fn synthesize_unlisted_properties(
    cfb: &CfbFile,
    storage_path: &str,
    properties: &mut Vec<MapiProperty>,
    consumed_paths: &mut BTreeSet<String>,
    named: &[MapiNamedProperty],
    codepage: u32,
    options: &OutlookMsgOptions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let listed = properties
        .iter()
        .map(|property| property.property_tag.trim_start_matches("0x").to_string())
        .collect::<BTreeSet<_>>();
    let mut groups = BTreeMap::<String, Vec<&CfbEntry>>::new();
    for child in cfb.direct_children(storage_path) {
        let upper = child.name.to_ascii_uppercase();
        let Some(rest) = upper.strip_prefix(&SUBSTG_PREFIX.to_ascii_uppercase()) else {
            continue;
        };
        let tag = rest.split('-').next().unwrap_or(rest);
        if tag.len() == 8 && tag.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            groups.entry(tag.to_string()).or_default().push(child);
        }
    }
    for (tag_name, mut sources) in groups {
        if listed.contains(&tag_name) || properties.len() >= options.max_properties_per_object {
            continue;
        }
        sources.sort_by_key(|entry| entry.id);
        for source in &sources {
            consumed_paths.insert(source.path.clone());
        }
        let Ok(tag) = u32::from_str_radix(&tag_name, 16) else {
            continue;
        };
        let property_id = (tag >> 16) as u16;
        let type_code = tag as u16;
        let value =
            decode_property_value(type_code, &[0; 8], &sources, codepage, options, diagnostics);
        diagnostics.push(partial(
            "outlook.msg.mapi.property_entry_missing",
            format!("value stream for 0x{tag_name} has no property-table entry and was recovered"),
        ));
        properties.push(MapiProperty {
            ordinal: properties.len(),
            property_tag: format!("0x{tag_name}"),
            property_id,
            property_type: property_type(type_code),
            flags: 0,
            table_value: binary(&[], true),
            canonical_name: canonical_property_name(property_id).map(str::to_string),
            named: named
                .iter()
                .find(|item| item.property_id == property_id)
                .cloned(),
            value,
            stream_paths: sources.iter().map(|source| source.path.clone()).collect(),
            locator: stream_range_locator(sources[0], 0, sources[0].data.len()),
        });
    }
}

fn decode_property_value(
    type_code: u16,
    inline: &[u8],
    sources: &[&CfbEntry],
    codepage: u32,
    options: &OutlookMsgOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> MapiValue {
    let multi = type_code & 0x1000 != 0;
    let base_type = type_code & !0x1000;
    if multi {
        return decode_multi_value(base_type, sources, codepage, options, diagnostics);
    }
    let stream = sources.first().map(|source| source.data.as_slice());
    let expects_stream = matches!(base_type, 0x001e | 0x001f | 0x0048 | 0x0102 | 0x000d);
    if expects_stream && stream.is_none() {
        diagnostics.push(partial(
            "outlook.msg.mapi.value_stream_missing",
            format!("property type 0x{type_code:04X} has no value stream"),
        ));
    }
    let raw = if expects_stream {
        stream.unwrap_or_default()
    } else {
        stream.unwrap_or(inline)
    };
    match base_type {
        0x0000 => MapiValue::Unspecified {
            raw: binary(raw, options.inline_property_binary),
        },
        0x0001 => MapiValue::Null,
        0x0002 => MapiValue::Integer16 {
            value: le_i16(inline, 0).unwrap_or_default(),
        },
        0x0003 => MapiValue::Integer32 {
            value: le_i32(inline, 0).unwrap_or_default(),
        },
        0x0004 => {
            let bits = le_u32(inline, 0).unwrap_or_default();
            MapiValue::Float32 {
                bits,
                value: f32::from_bits(bits),
            }
        }
        0x0005 => {
            let bits = le_u64(inline, 0).unwrap_or_default();
            MapiValue::Float64 {
                bits,
                value: f64::from_bits(bits),
            }
        }
        0x0006 => MapiValue::Currency {
            scaled_value: le_i64(inline, 0).unwrap_or_default(),
        },
        0x0007 => {
            let bits = le_u64(inline, 0).unwrap_or_default();
            MapiValue::FloatingTime {
                bits,
                value: f64::from_bits(bits),
            }
        }
        0x000a => MapiValue::Error {
            code: le_u32(inline, 0).unwrap_or_default(),
        },
        0x000b => {
            let raw = le_u16(inline, 0).unwrap_or_default();
            MapiValue::Boolean {
                raw,
                value: raw != 0,
            }
        }
        0x0014 => MapiValue::Integer64 {
            value: le_i64(inline, 0).unwrap_or_default(),
        },
        0x001e => {
            let (text, encoding, lossy) = decode_string8(raw, codepage);
            if lossy {
                diagnostics.push(partial(
                    "outlook.msg.mapi.codepage_fallback",
                    format!("String8 value in code page {codepage} required a lossy fallback"),
                ));
            }
            MapiValue::String {
                text,
                encoding,
                lossy,
                raw: binary(raw, options.inline_property_binary),
            }
        }
        0x001f => {
            let (text, lossy) = decode_utf16le(raw);
            if raw.len() % 2 != 0 {
                diagnostics.push(partial(
                    "outlook.msg.mapi.unicode_odd_length",
                    "Unicode property has an odd byte length",
                ));
            }
            if lossy && raw.len() % 2 == 0 {
                diagnostics.push(partial(
                    "outlook.msg.mapi.unicode_decode_loss",
                    "Unicode property contains an unpaired UTF-16 surrogate",
                ));
            }
            MapiValue::String {
                text,
                encoding: "utf-16le".to_string(),
                lossy,
                raw: binary(raw, options.inline_property_binary),
            }
        }
        0x0040 => MapiValue::SystemTime {
            value: filetime(le_u64(inline, 0).unwrap_or_default()),
        },
        0x0048 => MapiValue::Guid {
            value: guid(raw).unwrap_or_else(|| "malformed-guid".to_string()),
            raw: binary(raw, options.inline_property_binary),
        },
        0x0102 => MapiValue::Binary {
            value: binary(raw, options.inline_property_binary),
        },
        0x000d => MapiValue::Object {
            storage_path: sources
                .iter()
                .find(|source| source.kind == CfbEntryKind::Storage)
                .map(|source| source.path.clone()),
            raw: binary(raw, options.inline_property_binary),
        },
        _ => MapiValue::Unknown {
            type_code,
            raw: binary(raw, options.inline_property_binary),
        },
    }
}

fn decode_multi_value(
    base_type: u16,
    sources: &[&CfbEntry],
    codepage: u32,
    options: &OutlookMsgOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> MapiValue {
    let base = sources
        .iter()
        .find(|source| !source.name.contains('-'))
        .map(|source| source.data.as_slice())
        .unwrap_or_default();
    let mut fragments = sources
        .iter()
        .filter(|source| source.name.contains('-'))
        .copied()
        .collect::<Vec<_>>();
    fragments.sort_by(|left, right| left.name.cmp(&right.name));
    let mut values = Vec::new();
    if matches!(base_type, 0x001e | 0x001f | 0x0102 | 0x000d) {
        for fragment in fragments {
            values.push(decode_property_value(
                base_type,
                &[0; 8],
                &[fragment],
                codepage,
                options,
                diagnostics,
            ));
        }
    } else if let Some(width) = fixed_width(base_type) {
        for chunk in base.chunks_exact(width) {
            let mut inline = [0u8; 8];
            inline[..width.min(8)].copy_from_slice(&chunk[..width.min(8)]);
            values.push(decode_property_value(
                base_type,
                &inline,
                &[],
                codepage,
                options,
                diagnostics,
            ));
        }
    }
    MapiValue::MultiValue {
        values,
        raw: binary(base, options.inline_property_binary),
    }
}

fn fixed_width(type_code: u16) -> Option<usize> {
    Some(match type_code {
        0x0002 | 0x000b => 2,
        0x0003 | 0x0004 | 0x000a => 4,
        0x0005 | 0x0006 | 0x0007 | 0x0014 | 0x0040 => 8,
        0x0048 => 16,
        _ => return None,
    })
}

pub(super) fn property(properties: &[MapiProperty], id: u16) -> Option<&MapiProperty> {
    properties
        .iter()
        .find(|property| property.property_id == id)
}

pub(super) fn property_text(properties: &[MapiProperty], id: u16) -> Option<super::MsgTextFact> {
    let property = property(properties, id)?;
    let MapiValue::String {
        text,
        encoding,
        lossy,
        raw,
    } = &property.value
    else {
        return None;
    };
    Some(super::MsgTextFact {
        property_tag: property.property_tag.clone(),
        raw: raw.clone(),
        text: text.clone(),
        encoding: encoding.clone(),
        lossy: *lossy,
        locator: property.locator.clone(),
    })
}

pub(super) fn property_i32(properties: &[MapiProperty], id: u16) -> Option<i32> {
    match &property(properties, id)?.value {
        MapiValue::Integer32 { value } => Some(*value),
        _ => None,
    }
}

pub(super) fn property_binary(properties: &[MapiProperty], id: u16) -> Option<MsgBinary> {
    match &property(properties, id)?.value {
        MapiValue::Binary { value } => Some(value.clone()),
        MapiValue::Object { raw, .. } => Some(raw.clone()),
        _ => None,
    }
}

pub(super) fn property_date(properties: &[MapiProperty], id: u16) -> Option<MsgDate> {
    match &property(properties, id)?.value {
        MapiValue::SystemTime { value } => Some(value.clone()),
        _ => None,
    }
}

pub(super) fn binary(bytes: &[u8], inline: bool) -> MsgBinary {
    MsgBinary {
        byte_length: bytes.len(),
        sha256: sha256_hex(bytes),
        bytes: inline.then(|| bytes.to_vec()),
    }
}

pub(super) fn entry_locator(entry: &CfbEntry) -> SourceLocator {
    stream_range_locator(entry, 0, entry.data.len())
}

fn stream_range_locator(entry: &CfbEntry, start: usize, end: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: entry.path.clone(),
        member_index: IndexPosition::new(entry.id as u64, IndexBase::Zero)
            .expect("CFB directory IDs are valid zero-based indexes"),
    })
    .expect("CFB member path is non-empty")
    .nested(LocationComponent::ByteRange {
        byte_start: start,
        byte_end: end,
    })
    .expect("stream-relative byte range is valid")
}

fn property_header_size(storage_path: &str, length: usize) -> usize {
    let embedded_message = storage_path
        .to_ascii_uppercase()
        .ends_with("__SUBSTG1.0_3701000D");
    let preferred = if storage_path.is_empty() {
        32
    } else if embedded_message {
        24
    } else {
        8
    };
    if length >= preferred && (length - preferred) % 16 == 0 {
        preferred
    } else if length >= 24 && (length - 24) % 16 == 0 {
        24
    } else if length >= 8 && (length - 8) % 16 == 0 {
        8
    } else if length >= 32 && (length - 32) % 16 == 0 {
        32
    } else {
        preferred.min(length)
    }
}

fn property_type(code: u16) -> MapiPropertyType {
    let base = code & !0x1000;
    let name = match base {
        0x0000 => "unspecified",
        0x0001 => "null",
        0x0002 => "integer16",
        0x0003 => "integer32",
        0x0004 => "float32",
        0x0005 => "float64",
        0x0006 => "currency",
        0x0007 => "floating_time",
        0x000a => "error",
        0x000b => "boolean",
        0x000d => "object",
        0x0014 => "integer64",
        0x001e => "string8",
        0x001f => "unicode",
        0x0040 => "system_time",
        0x0048 => "guid",
        0x0102 => "binary",
        _ => "unknown",
    };
    MapiPropertyType {
        code,
        name: name.to_string(),
        multi_valued: code & 0x1000 != 0,
        known: name != "unknown",
    }
}

fn canonical_property_name(id: u16) -> Option<&'static str> {
    Some(match id {
        0x0017 => "PR_IMPORTANCE",
        0x001a => "PR_MESSAGE_CLASS",
        0x0036 => "PR_SENSITIVITY",
        0x0037 => "PR_SUBJECT",
        0x0039 => "PR_CLIENT_SUBMIT_TIME",
        0x0042 => "PR_SENT_REPRESENTING_NAME",
        0x0064 => "PR_SENT_REPRESENTING_ADDRTYPE",
        0x0065 => "PR_SENT_REPRESENTING_EMAIL_ADDRESS",
        0x0070 => "PR_CONVERSATION_TOPIC",
        0x0071 => "PR_CONVERSATION_INDEX",
        0x007d => "PR_TRANSPORT_MESSAGE_HEADERS",
        0x0c15 => "PR_RECIPIENT_TYPE",
        0x0c1a => "PR_SENDER_NAME",
        0x0c1e => "PR_SENDER_ADDRTYPE",
        0x0c1f => "PR_SENDER_EMAIL_ADDRESS",
        0x0e04 => "PR_DISPLAY_TO",
        0x0e03 => "PR_DISPLAY_CC",
        0x0e02 => "PR_DISPLAY_BCC",
        0x0e06 => "PR_MESSAGE_DELIVERY_TIME",
        0x0e07 => "PR_MESSAGE_FLAGS",
        0x0e1b => "PR_HASATTACH",
        0x0e1d => "PR_NORMALIZED_SUBJECT",
        0x0e20 => "PR_ATTACH_SIZE",
        0x0fff => "PR_ENTRYID",
        0x1000 => "PR_BODY",
        0x1009 => "PR_RTF_COMPRESSED",
        0x1013 => "PR_HTML",
        0x1035 => "PR_INTERNET_MESSAGE_ID",
        0x1039 => "PR_INTERNET_REFERENCES",
        0x1042 => "PR_IN_REPLY_TO_ID",
        0x3001 => "PR_DISPLAY_NAME",
        0x3002 => "PR_ADDRTYPE",
        0x3003 => "PR_EMAIL_ADDRESS",
        0x3007 => "PR_CREATION_TIME",
        0x3008 => "PR_LAST_MODIFICATION_TIME",
        0x3701 => "PR_ATTACH_DATA",
        0x3704 => "PR_ATTACH_FILENAME",
        0x3705 => "PR_ATTACH_METHOD",
        0x3707 => "PR_ATTACH_LONG_FILENAME",
        0x370b => "PR_RENDERING_POSITION",
        0x370e => "PR_ATTACH_MIME_TAG",
        0x3712 => "PR_ATTACH_CONTENT_ID",
        0x3713 => "PR_ATTACH_CONTENT_LOCATION",
        0x3714 => "PR_ATTACH_FLAGS",
        0x39fe => "PR_SMTP_ADDRESS",
        0x3ffd => "PR_MESSAGE_CODEPAGE",
        0x5d01 => "PR_SENDER_SMTP_ADDRESS",
        _ => return None,
    })
}

fn decode_utf16le(bytes: &[u8]) -> (String, bool) {
    let mut units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    while units.last() == Some(&0) {
        units.pop();
    }
    let mut lossy = bytes.len() % 2 != 0;
    let text = std::char::decode_utf16(units)
        .map(|value| match value {
            Ok(character) => character,
            Err(_) => {
                lossy = true;
                char::REPLACEMENT_CHARACTER
            }
        })
        .collect();
    (text, lossy)
}

fn decode_string8(bytes: &[u8], codepage: u32) -> (String, String, bool) {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
    if codepage == 65001 {
        let text = String::from_utf8_lossy(bytes);
        let lossy = matches!(text, std::borrow::Cow::Owned(_));
        return (text.into_owned(), "utf-8".to_string(), lossy);
    }
    let text = bytes.iter().map(|byte| windows_1252(*byte)).collect();
    (
        text,
        if codepage == 1252 || codepage == 0 {
            "windows-1252".to_string()
        } else {
            format!("windows-codepage-{codepage}-decoded-as-1252")
        },
        codepage != 1252 && codepage != 0,
    )
}

pub(super) fn decode_bytes(bytes: &[u8], codepage: u32) -> (String, String, bool) {
    if bytes.starts_with(&[0xff, 0xfe]) {
        let (text, lossy) = decode_utf16le(&bytes[2..]);
        (text, "utf-16le".to_string(), lossy)
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        let text = String::from_utf8_lossy(&bytes[3..]);
        let lossy = matches!(text, std::borrow::Cow::Owned(_));
        (text.into_owned(), "utf-8".to_string(), lossy)
    } else {
        decode_string8(bytes, codepage)
    }
}

fn windows_1252(byte: u8) -> char {
    match byte {
        0x80 => '\u{20ac}',
        0x82 => '\u{201a}',
        0x83 => '\u{0192}',
        0x84 => '\u{201e}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02c6}',
        0x89 => '\u{2030}',
        0x8a => '\u{0160}',
        0x8b => '\u{2039}',
        0x8c => '\u{0152}',
        0x8e => '\u{017d}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02dc}',
        0x99 => '\u{2122}',
        0x9a => '\u{0161}',
        0x9b => '\u{203a}',
        0x9c => '\u{0153}',
        0x9e => '\u{017e}',
        0x9f => '\u{0178}',
        other => char::from(other),
    }
}

fn named_string(bytes: &[u8], offset: usize) -> Option<String> {
    let length = le_u32(bytes, offset)? as usize;
    let start = offset.checked_add(4)?;
    let end = start.checked_add(length)?;
    let (value, _) = decode_utf16le(bytes.get(start..end)?);
    Some(value)
}

fn filetime(ticks: u64) -> MsgDate {
    const EPOCH_DIFFERENCE_SECONDS: i128 = 11_644_473_600;
    let seconds = i128::from(ticks / 10_000_000) - EPOCH_DIFFERENCE_SECONDS;
    let unix_seconds = i64::try_from(seconds).ok();
    MsgDate {
        filetime_ticks: ticks,
        unix_seconds,
        rfc3339_utc: unix_seconds.and_then(format_unix_utc),
    }
}

fn format_unix_utc(seconds: i64) -> Option<String> {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days)?;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn civil_from_days(days_since_unix: i64) -> Option<(i64, i64, i64)> {
    let z = days_since_unix.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    Some((year, month, day))
}

fn guid(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 16 {
        return None;
    }
    Some(format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        le_u32(bytes, 0)?,
        le_u16(bytes, 4)?,
        le_u16(bytes, 6)?,
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

fn le_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn le_i16(bytes: &[u8], offset: usize) -> Option<i16> {
    Some(i16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn le_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn le_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn le_i64(bytes: &[u8], offset: usize) -> Option<i64> {
    Some(i64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn partial(code: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::warning(PARSER, code, message).partial()
}
