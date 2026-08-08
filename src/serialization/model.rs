use crate::core::{Diagnostic, SourceLocator, SourceRange};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredTextDocument {
    pub schema_version: String,
    pub format: StructuredTextFormat,
    /// Values in source-document order. JSON, TOML, and XML normally contain one.
    pub documents: Vec<StructuredValue>,
    /// Non-empty only for record-oriented JSONL/NDJSON input.
    pub records: Vec<StructuredRecord>,
    pub ordering: StructuredOrdering,
    pub duplicate_keys: Vec<DuplicateKey>,
    pub aliases: Vec<StructuredAlias>,
    pub raw_unknowns: Vec<RawStructuredUnknown>,
    pub validation: Option<SchemaValidationResult>,
    pub complete: bool,
    /// Compatibility JSON projection. The typed tree above is authoritative.
    pub value: Option<Value>,
    /// Compatibility JSONL projection retained for the v1 Rust API.
    pub jsonl_records: Vec<JsonlRecord>,
}

pub type SerializationPayload = StructuredTextDocument;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuredTextFormat {
    Json,
    Jsonl,
    Yaml,
    Toml,
    Xml,
}

pub type SerializationFormat = StructuredTextFormat;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOrdering {
    /// Object entries, sequence items, documents, and records follow source order.
    Source,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredValue {
    pub id: String,
    pub kind: StructuredValueKind,
    pub path: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
    /// Exact source spelling for this node.
    pub raw: String,
    pub scalar: Option<StructuredScalar>,
    pub entries: Vec<StructuredEntry>,
    pub items: Vec<StructuredValue>,
    pub anchor: Option<String>,
    pub tag: Option<String>,
    pub alias: Option<String>,
    pub alias_target_id: Option<String>,
    pub recovered: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuredValueKind {
    Null,
    Boolean,
    Integer,
    Float,
    String,
    Date,
    Time,
    DateTime,
    Array,
    Object,
    Alias,
    XmlElement,
    XmlText,
    XmlComment,
    XmlProcessingInstruction,
    RawUnknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "scalar_kind", rename_all = "snake_case")]
pub enum StructuredScalar {
    Null,
    Boolean {
        value: bool,
    },
    /// Canonical decimal spelling; may exceed JSON's integer range.
    Integer {
        canonical: String,
    },
    /// Canonical spelling, including non-JSON YAML values `.inf` and `.nan`.
    Float {
        canonical: String,
        finite: bool,
    },
    String {
        value: String,
    },
    Date {
        value: String,
    },
    Time {
        value: String,
    },
    DateTime {
        value: String,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredEntry {
    pub index: usize,
    pub key: Box<StructuredValue>,
    pub value: Box<StructuredValue>,
    pub key_text: Option<String>,
    pub duplicate_ordinal: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredRecord {
    pub index: usize,
    pub source_line: usize,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub raw: String,
    pub value: Option<StructuredValue>,
    pub diagnostic: Option<Diagnostic>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DuplicateKey {
    pub path: String,
    pub key: String,
    pub occurrence: usize,
    pub first_locator: SourceLocator,
    pub duplicate_locator: SourceLocator,
    pub disposition: DuplicateKeyDisposition,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateKeyDisposition {
    Preserved,
    RejectedBySyntax,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredAlias {
    pub name: String,
    pub path: String,
    pub locator: SourceLocator,
    pub target_id: Option<String>,
    pub resolved: bool,
    /// Aliases are references and are never recursively expanded by Grist.
    pub expanded: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawStructuredUnknown {
    pub path: String,
    pub raw: String,
    pub reason: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonlRecord {
    pub line: usize,
    pub value: Option<Value>,
    pub diagnostic: Option<Diagnostic>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchemaValidationResult {
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MalformedRecoveryPolicy {
    /// Reject a malformed single-document input; JSONL still isolates bad records.
    #[default]
    Strict,
    /// Retain the unparsed region as a raw unknown and return a partial payload.
    PreserveRaw,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JsonDuplicateProjection {
    FirstWins,
    #[default]
    LastWins,
    Reject,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SerializationOptions {
    pub schema: Option<Value>,
    pub malformed_recovery: MalformedRecoveryPolicy,
    /// Governs only the compatibility JSON projection; source entries are always retained.
    pub duplicate_projection: JsonDuplicateProjection,
    /// Parser-stack safety ceiling used by legacy convenience entry points.
    /// Registry calls also enforce the shared `ResourceBudget` and use the lower ceiling.
    pub max_nesting_depth: usize,
}

impl Default for SerializationOptions {
    fn default() -> Self {
        Self {
            schema: None,
            malformed_recovery: MalformedRecoveryPolicy::Strict,
            duplicate_projection: JsonDuplicateProjection::LastWins,
            max_nesting_depth: 256,
        }
    }
}

impl crate::core::FormatOptions for SerializationOptions {
    const FORMAT: &'static str = "structured_text";
}

impl StructuredValue {
    pub(crate) fn json_projection(&self, duplicate: JsonDuplicateProjection) -> Option<Value> {
        match self.kind {
            StructuredValueKind::Null => Some(Value::Null),
            StructuredValueKind::Boolean => match &self.scalar {
                Some(StructuredScalar::Boolean { value }) => Some(Value::Bool(*value)),
                _ => None,
            },
            StructuredValueKind::Integer => match &self.scalar {
                Some(StructuredScalar::Integer { canonical }) => canonical
                    .parse::<i64>()
                    .map(Number::from)
                    .or_else(|_| canonical.parse::<u64>().map(Number::from))
                    .ok()
                    .map(Value::Number),
                _ => None,
            },
            StructuredValueKind::Float => match &self.scalar {
                Some(StructuredScalar::Float {
                    canonical,
                    finite: true,
                }) => canonical
                    .parse::<f64>()
                    .ok()
                    .and_then(Number::from_f64)
                    .map(Value::Number),
                _ => None,
            },
            StructuredValueKind::String
            | StructuredValueKind::Date
            | StructuredValueKind::Time
            | StructuredValueKind::DateTime
            | StructuredValueKind::XmlText
            | StructuredValueKind::XmlComment
            | StructuredValueKind::XmlProcessingInstruction => {
                self.scalar.as_ref().and_then(|s| match s {
                    StructuredScalar::String { value }
                    | StructuredScalar::Date { value }
                    | StructuredScalar::Time { value }
                    | StructuredScalar::DateTime { value } => Some(Value::String(value.clone())),
                    _ => None,
                })
            }
            StructuredValueKind::Array => self
                .items
                .iter()
                .map(|item| item.json_projection(duplicate))
                .collect::<Option<Vec<_>>>()
                .map(Value::Array),
            StructuredValueKind::Object | StructuredValueKind::XmlElement => {
                let mut map = Map::new();
                for entry in &self.entries {
                    let key = entry.key_text.as_ref()?;
                    let value = entry.value.json_projection(duplicate)?;
                    match duplicate {
                        JsonDuplicateProjection::FirstWins => {
                            map.entry(key.clone()).or_insert(value);
                        }
                        JsonDuplicateProjection::LastWins => {
                            map.insert(key.clone(), value);
                        }
                        JsonDuplicateProjection::Reject if map.contains_key(key) => return None,
                        JsonDuplicateProjection::Reject => {
                            map.insert(key.clone(), value);
                        }
                    }
                }
                if !self.items.is_empty() {
                    map.insert(
                        "$content".into(),
                        Value::Array(
                            self.items
                                .iter()
                                .filter_map(|item| item.json_projection(duplicate))
                                .collect(),
                        ),
                    );
                }
                Some(Value::Object(map))
            }
            StructuredValueKind::Alias | StructuredValueKind::RawUnknown => None,
        }
    }
}

pub(crate) fn pointer_escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
