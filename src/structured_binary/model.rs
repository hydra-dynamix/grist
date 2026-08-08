use crate::core::{Diagnostic, SourceLocator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuredBinaryFormat {
    Cbor,
    MessagePack,
    Protobuf,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredBinaryDocument {
    pub schema_version: String,
    pub format: StructuredBinaryFormat,
    pub schema_identity: BinarySchemaIdentity,
    /// Top-level values in byte order. Concatenated CBOR/MessagePack values are
    /// records; Protobuf has exactly one message record.
    pub records: Vec<BinaryRecord>,
    pub tags: Vec<CborTag>,
    pub extensions: Vec<MessagePackExtension>,
    pub unknown_fields: Vec<ProtobufUnknownField>,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
    /// Deterministic, loss-aware JSON projection. The typed records remain
    /// authoritative.
    pub json_projection: Option<Value>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "identity_kind", rename_all = "snake_case")]
pub enum BinarySchemaIdentity {
    Cbor {
        specification: String,
        self_described: bool,
    },
    MessagePack {
        specification: String,
    },
    ProtobufDescriptor {
        descriptor_sha256: String,
        descriptor_size: usize,
        message_name: String,
        syntax: ProtobufSyntax,
        files: Vec<String>,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtobufSyntax {
    Proto2,
    Proto3,
    Editions,
    #[default]
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryRecord {
    /// One-based public record ordinal.
    pub index: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub locator: SourceLocator,
    pub value: BinaryValue,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryValue {
    pub id: String,
    pub kind: BinaryValueKind,
    pub path: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub locator: SourceLocator,
    pub scalar: Option<BinaryScalar>,
    pub entries: Vec<BinaryEntry>,
    pub items: Vec<BinaryValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cbor_tag: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messagepack_extension: Option<MessagePackExtensionValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protobuf_field: Option<ProtobufFieldIdentity>,
    pub indefinite: bool,
    pub recovered: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BinaryValueKind {
    Null,
    Undefined,
    Boolean,
    Integer,
    Float,
    Text,
    Bytes,
    Array,
    Map,
    Simple,
    Message,
    Enum,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "scalar_kind", rename_all = "snake_case")]
pub enum BinaryScalar {
    Null,
    Undefined,
    Boolean {
        value: bool,
    },
    Integer {
        canonical: String,
    },
    Float {
        canonical: String,
        finite: bool,
        width_bits: u8,
        raw_bits_hex: String,
    },
    Text {
        value: String,
    },
    Bytes {
        hex: String,
        length: usize,
    },
    Simple {
        value: u8,
    },
    Enum {
        number: i32,
        name: Option<String>,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryEntry {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Box<BinaryValue>>,
    pub value: Box<BinaryValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_number: Option<u32>,
    pub duplicate_ordinal: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CborTag {
    pub tag: u64,
    pub path: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePackExtension {
    pub type_code: i8,
    pub path: String,
    pub data_hex: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub locator: SourceLocator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<MessagePackTimestamp>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePackExtensionValue {
    pub type_code: i8,
    pub data_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<MessagePackTimestamp>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessagePackTimestamp {
    pub seconds: i64,
    pub nanoseconds: u32,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtobufFieldIdentity {
    pub message_name: String,
    pub field_name: String,
    pub json_name: String,
    pub number: u32,
    pub declared_type: String,
    pub repeated: bool,
    pub packed: bool,
    pub extension: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oneof: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtobufUnknownField {
    pub message_name: String,
    pub field_number: u32,
    pub wire_type: u8,
    pub raw_hex: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub locator: SourceLocator,
    pub reason: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BinaryMalformedRecovery {
    #[default]
    Strict,
    PreserveRaw,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct StructuredBinaryOptions {
    pub max_nesting_depth: usize,
    pub max_values: usize,
    pub max_collection_items: usize,
    pub max_blob_bytes: usize,
    pub allow_sequence: bool,
    pub malformed_recovery: BinaryMalformedRecovery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protobuf: Option<ProtobufDecodeOptions>,
}

impl Default for StructuredBinaryOptions {
    fn default() -> Self {
        Self {
            max_nesting_depth: 256,
            max_values: 1_000_000,
            max_collection_items: 1_000_000,
            max_blob_bytes: 64 * 1024 * 1024,
            allow_sequence: true,
            malformed_recovery: BinaryMalformedRecovery::Strict,
            protobuf: None,
        }
    }
}

impl crate::core::FormatOptions for StructuredBinaryOptions {
    const FORMAT: &'static str = "structured_binary";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtobufDecodeOptions {
    /// Serialized google.protobuf.FileDescriptorSet bytes.
    pub descriptor_set: Vec<u8>,
    /// Fully-qualified message name, with or without a leading dot.
    pub message_name: String,
    #[serde(default = "default_true")]
    pub preserve_unknown_fields: bool,
}

const fn default_true() -> bool {
    true
}

impl BinaryValue {
    pub fn json_projection(&self) -> Value {
        let projected = match self.kind {
            BinaryValueKind::Null => Value::Null,
            BinaryValueKind::Undefined => serde_json::json!({"$undefined": true}),
            BinaryValueKind::Boolean => match &self.scalar {
                Some(BinaryScalar::Boolean { value }) => Value::Bool(*value),
                _ => Value::Null,
            },
            BinaryValueKind::Integer => match &self.scalar {
                Some(BinaryScalar::Integer { canonical }) => canonical
                    .parse::<i64>()
                    .map(Number::from)
                    .or_else(|_| canonical.parse::<u64>().map(Number::from))
                    .map(Value::Number)
                    .unwrap_or_else(|_| serde_json::json!({"$integer": canonical})),
                _ => Value::Null,
            },
            BinaryValueKind::Float => match &self.scalar {
                Some(BinaryScalar::Float {
                    canonical,
                    finite: true,
                    ..
                }) => canonical
                    .parse::<f64>()
                    .ok()
                    .and_then(Number::from_f64)
                    .map(Value::Number)
                    .unwrap_or_else(|| serde_json::json!({"$float": canonical})),
                Some(BinaryScalar::Float { canonical, .. }) => {
                    serde_json::json!({"$float": canonical})
                }
                _ => Value::Null,
            },
            BinaryValueKind::Text => match &self.scalar {
                Some(BinaryScalar::Text { value }) => Value::String(value.clone()),
                _ => Value::Null,
            },
            BinaryValueKind::Bytes => match &self.scalar {
                Some(BinaryScalar::Bytes { hex, length }) => {
                    serde_json::json!({"$bytes": hex, "length": length})
                }
                _ => Value::Null,
            },
            BinaryValueKind::Simple => match &self.scalar {
                Some(BinaryScalar::Simple { value }) => serde_json::json!({"$simple": value}),
                _ => Value::Null,
            },
            BinaryValueKind::Enum => match &self.scalar {
                Some(BinaryScalar::Enum {
                    number: _,
                    name: Some(name),
                }) => Value::String(name.clone()),
                Some(BinaryScalar::Enum { number, name: None }) => Number::from(*number).into(),
                _ => Value::Null,
            },
            BinaryValueKind::Array => {
                Value::Array(self.items.iter().map(Self::json_projection).collect())
            }
            BinaryValueKind::Map | BinaryValueKind::Message => project_entries(&self.entries),
            BinaryValueKind::Unknown => match &self.scalar {
                Some(BinaryScalar::Bytes { hex, length }) => {
                    serde_json::json!({"$unknown": hex, "length": length})
                }
                _ => serde_json::json!({"$unknown": true}),
            },
        };
        if let Some(tag) = self.cbor_tag {
            serde_json::json!({"$tag": tag, "value": projected})
        } else if let Some(extension) = &self.messagepack_extension {
            serde_json::json!({
                "$extension": extension.type_code,
                "data": extension.data_hex,
                "timestamp": extension.timestamp,
            })
        } else {
            projected
        }
    }
}

fn project_entries(entries: &[BinaryEntry]) -> Value {
    let protobuf = entries.iter().any(|entry| entry.field_number.is_some());
    if protobuf {
        let mut object = Map::new();
        for entry in entries {
            let key = entry
                .field_name
                .clone()
                .unwrap_or_else(|| entry.field_number.unwrap_or_default().to_string());
            let value = entry.value.json_projection();
            if entry
                .value
                .protobuf_field
                .as_ref()
                .is_some_and(|field| field.repeated)
            {
                match object
                    .entry(key)
                    .or_insert_with(|| Value::Array(Vec::new()))
                {
                    Value::Array(values) => values.push(value),
                    other => *other = Value::Array(vec![other.take(), value]),
                }
            } else {
                object.insert(key, value);
            }
        }
        return Value::Object(object);
    }
    let all_string_keys = entries.iter().all(|entry| {
        matches!(
            entry.key.as_deref().and_then(|key| key.scalar.as_ref()),
            Some(BinaryScalar::Text { .. })
        ) && entry.duplicate_ordinal <= 1
    });
    if all_string_keys {
        let mut object = Map::new();
        for entry in entries {
            let Some(BinaryScalar::Text { value: key }) =
                entry.key.as_deref().and_then(|key| key.scalar.as_ref())
            else {
                continue;
            };
            object.insert(key.clone(), entry.value.json_projection());
        }
        Value::Object(object)
    } else {
        serde_json::json!({
            "$map": entries.iter().map(|entry| serde_json::json!({
                "key": entry.key.as_deref().map(BinaryValue::json_projection),
                "value": entry.value.json_projection(),
                "duplicate_ordinal": entry.duplicate_ordinal,
            })).collect::<Vec<_>>()
        })
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}
