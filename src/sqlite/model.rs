use crate::core::{Diagnostic, SourceLocator};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SqliteOptions {
    /// Record extraction is disabled when absent. Supplying it requires both
    /// explicit table names and finite table/row limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_selection: Option<SqliteRecordSelection>,
    #[serde(default)]
    pub include_internal_schema: bool,
}

impl crate::core::FormatOptions for SqliteOptions {
    const FORMAT: &'static str = "sqlite";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SqliteRecordSelection {
    pub tables: Vec<String>,
    pub max_tables: usize,
    pub max_rows_per_table: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqliteDocument {
    pub schema_version: String,
    pub header: SqliteHeader,
    pub schema: Vec<SqliteSchemaObject>,
    pub tables: Vec<SqliteTable>,
    pub views: Vec<SqliteView>,
    pub indexes: Vec<SqliteIndex>,
    pub record_sets: Vec<SqliteRecordSet>,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SqliteHeader {
    pub page_size: u32,
    pub usable_page_size: u32,
    pub page_count: u32,
    pub header_page_count: u32,
    pub read_version: SqliteJournalMode,
    pub write_version: SqliteJournalMode,
    pub text_encoding: SqliteTextEncoding,
    pub schema_cookie: u32,
    pub schema_format: u32,
    pub user_version: u32,
    pub application_id: u32,
    pub sqlite_version_number: u32,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SqliteJournalMode {
    Legacy,
    Wal,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SqliteTextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SqliteSchemaObjectKind {
    Table,
    Index,
    View,
    Trigger,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqliteSchemaObject {
    pub kind: SqliteSchemaObjectKind,
    pub source_type: String,
    pub name: String,
    pub table_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_page: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqliteTable {
    pub name: String,
    pub root_page: u32,
    pub definition: String,
    pub columns: Vec<SqliteColumn>,
    pub without_rowid: bool,
    pub strict: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SqliteColumn {
    pub ordinal: usize,
    pub name: String,
    pub declaration: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_type: Option<String>,
    pub primary_key: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqliteView {
    pub name: String,
    pub definition: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqliteIndex {
    pub name: String,
    pub table_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_page: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    pub unique: bool,
    pub partial: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqliteRecordSet {
    pub table: String,
    pub root_page: u32,
    pub columns: Vec<String>,
    pub records: Vec<SqliteRecord>,
    pub row_budget: usize,
    pub truncated: bool,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqliteRecord {
    pub ordinal: usize,
    pub stable_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rowid: Option<i64>,
    pub fields: Vec<SqliteField>,
    pub page: u32,
    pub cell: usize,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqliteField {
    pub ordinal: usize,
    pub name: String,
    pub value: SqliteValue,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SqliteValue {
    Null,
    Integer {
        value: i64,
    },
    Real {
        value: f64,
    },
    Text {
        value: String,
        encoding: SqliteTextEncoding,
        lossy: bool,
    },
    Blob {
        hex: String,
        byte_length: usize,
    },
    Reserved {
        serial_type: u64,
    },
}

impl SqliteValue {
    pub fn display_text(&self) -> Option<String> {
        match self {
            Self::Null => None,
            Self::Integer { value } => Some(value.to_string()),
            Self::Real { value } => Some(value.to_string()),
            Self::Text { value, .. } => Some(value.clone()),
            Self::Blob { hex, .. } => Some(hex.clone()),
            Self::Reserved { serial_type } => Some(format!("reserved:{serial_type}")),
        }
    }
}
