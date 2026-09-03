use crate::core::{Diagnostic, SourceLocator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ColumnarFormat {
    ArrowIpcFile,
    ArrowIpcStream,
    Parquet,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarDocument {
    pub schema_version: String,
    pub format: ColumnarFormat,
    pub format_version: String,
    pub schema: ColumnarSchema,
    pub metadata: BTreeMap<String, String>,
    pub batches: Vec<ColumnarBatch>,
    pub dictionaries: Vec<ColumnarDictionary>,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
    pub projected_rows: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarSchema {
    pub fields: Vec<ColumnarField>,
    pub metadata: BTreeMap<String, String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarField {
    pub name: String,
    pub nullable: bool,
    pub data_type: ColumnarDataType,
    pub children: Vec<ColumnarField>,
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictionary: Option<DictionaryEncoding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parquet: Option<ParquetFieldInfo>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ColumnarDataType {
    Null,
    Boolean,
    SignedInteger {
        bit_width: u16,
    },
    UnsignedInteger {
        bit_width: u16,
    },
    Float {
        bit_width: u16,
    },
    Decimal {
        precision: u32,
        scale: i32,
        bit_width: u16,
    },
    Binary,
    LargeBinary,
    FixedSizeBinary {
        byte_width: u32,
    },
    Utf8,
    LargeUtf8,
    Date {
        unit: String,
    },
    Time {
        unit: String,
        bit_width: u16,
    },
    Timestamp {
        unit: String,
        timezone: Option<String>,
    },
    Duration {
        unit: String,
    },
    Interval {
        unit: String,
    },
    List,
    LargeList,
    FixedSizeList {
        length: u32,
    },
    Struct,
    Map {
        keys_sorted: bool,
    },
    Union {
        mode: String,
        type_ids: Vec<i32>,
    },
    Dictionary,
    ParquetPrimitive {
        physical_type: String,
        logical_type: Option<String>,
    },
    Unknown {
        type_id: i32,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DictionaryEncoding {
    pub id: i64,
    pub index_type: Box<ColumnarDataType>,
    pub ordered: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParquetFieldInfo {
    pub path: Vec<String>,
    pub repetition: String,
    pub max_definition_level: u8,
    pub max_repetition_level: u8,
    pub field_id: Option<i32>,
    pub converted_type: Option<String>,
    pub type_length: Option<i32>,
    pub precision: Option<i32>,
    pub scale: Option<i32>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarBatch {
    pub index: usize,
    pub kind: ColumnarBatchKind,
    pub source_row_start: u64,
    pub source_row_count: u64,
    pub columns: Vec<ColumnarColumn>,
    pub metadata: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ColumnarBatchKind {
    RecordBatch,
    RowGroup,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarColumn {
    pub path: Vec<String>,
    pub field_index: usize,
    pub encoding: Vec<String>,
    pub compression: Option<String>,
    pub values: Vec<ColumnarCell>,
    pub encoded_byte_start: usize,
    pub encoded_byte_end: usize,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarCell {
    pub row: u64,
    pub repetition_index: u32,
    pub definition_level: u8,
    pub repetition_level: u8,
    pub value: ColumnarValue,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "value_kind", rename_all = "snake_case")]
pub enum ColumnarValue {
    Null,
    Boolean {
        value: bool,
    },
    SignedInteger {
        canonical: String,
    },
    UnsignedInteger {
        canonical: String,
    },
    Float {
        canonical: String,
        finite: bool,
        bit_width: u16,
        raw_bits_hex: String,
    },
    Decimal {
        unscaled: String,
        precision: u32,
        scale: i32,
    },
    Utf8 {
        value: String,
    },
    Binary {
        hex: String,
        length: usize,
    },
    Date {
        value: i64,
        unit: String,
    },
    Time {
        value: i64,
        unit: String,
    },
    Timestamp {
        value: i64,
        unit: String,
        timezone: Option<String>,
    },
    Duration {
        value: i64,
        unit: String,
    },
    Interval {
        canonical: String,
    },
    List {
        values: Vec<ColumnarValue>,
    },
    Struct {
        fields: Vec<ColumnarNamedValue>,
    },
    Map {
        entries: Vec<ColumnarMapEntry>,
    },
    Dictionary {
        id: i64,
        index: i64,
        value: Box<ColumnarValue>,
    },
    Union {
        type_id: i8,
        value: Box<ColumnarValue>,
    },
    Unknown {
        raw_hex: String,
        reason: String,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarNamedValue {
    pub name: String,
    pub value: ColumnarValue,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarMapEntry {
    pub key: ColumnarValue,
    pub value: ColumnarValue,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnarDictionary {
    pub id: i64,
    pub is_delta: bool,
    pub values: Vec<ColumnarValue>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ColumnarOptions {
    pub row_start: u64,
    pub row_limit: Option<u64>,
    pub columns: Vec<String>,
    pub batch_start: usize,
    pub batch_limit: Option<usize>,
    pub max_metadata_bytes: usize,
    pub max_encoded_page_bytes: usize,
    pub preserve_unselected_schema: bool,
}
impl Default for ColumnarOptions {
    fn default() -> Self {
        Self {
            row_start: 0,
            row_limit: Some(1_000_000),
            columns: Vec::new(),
            batch_start: 0,
            batch_limit: None,
            max_metadata_bytes: 64 * 1024 * 1024,
            max_encoded_page_bytes: 256 * 1024 * 1024,
            preserve_unselected_schema: true,
        }
    }
}
impl crate::core::FormatOptions for ColumnarOptions {
    const FORMAT: &'static str = "columnar";
}
impl ColumnarOptions {
    pub(crate) fn selects_column(&self, path: &[String]) -> bool {
        if self.columns.is_empty() {
            return true;
        }
        let joined = path.join(".");
        self.columns
            .iter()
            .any(|column| column == &joined || path.first() == Some(column))
    }
    pub(crate) fn selects_row(&self, row: u64) -> bool {
        row >= self.row_start
            && self
                .row_limit
                .is_none_or(|limit| row < self.row_start.saturating_add(limit))
    }
    pub(crate) fn selects_batch(&self, index: usize) -> bool {
        index >= self.batch_start
            && self
                .batch_limit
                .is_none_or(|limit| index < self.batch_start.saturating_add(limit))
    }
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}
