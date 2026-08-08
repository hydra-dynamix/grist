use crate::core::{Diagnostic, SourceLocator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NotebookOptions {
    pub max_json_depth: usize,
    pub max_cells: usize,
    pub max_outputs_per_cell: usize,
    pub max_mime_entries: usize,
}

impl Default for NotebookOptions {
    fn default() -> Self {
        Self {
            max_json_depth: 128,
            max_cells: 100_000,
            max_outputs_per_cell: 100_000,
            max_mime_entries: 10_000,
        }
    }
}

impl crate::core::FormatOptions for NotebookOptions {
    const FORMAT: &'static str = "ipynb";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotebookAttachment {
    pub name: String,
    /// Exact MIME values; base64 binary remains inert text.
    pub data: BTreeMap<String, Value>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotebookOutput {
    pub ordinal: usize,
    pub cell_stable_id: String,
    pub output_type: String,
    pub normalized_output_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_count: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default)]
    pub data: BTreeMap<String, Value>,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub transient: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_value: Option<String>,
    #[serde(default)]
    pub traceback: Vec<String>,
    #[serde(default)]
    pub extra: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_view: Option<Value>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotebookCell {
    pub ordinal: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worksheet_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub stable_id: String,
    pub cell_type: String,
    pub source: String,
    /// Exact source JSON (`string`, `array`, or malformed value).
    pub source_raw: Value,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_count: Option<Value>,
    #[serde(default)]
    pub attachments: Vec<NotebookAttachment>,
    #[serde(default)]
    pub outputs: Vec<NotebookOutput>,
    #[serde(default)]
    pub extra: Map<String, Value>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotebookWorksheet {
    pub ordinal: usize,
    #[serde(default)]
    pub metadata: Value,
    pub cell_stable_ids: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NotebookDocument {
    pub schema_version: String,
    pub nbformat: u32,
    pub nbformat_minor: u32,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widgets: Option<Value>,
    pub cells: Vec<NotebookCell>,
    #[serde(default)]
    pub worksheets: Vec<NotebookWorksheet>,
    #[serde(default)]
    pub extra: Map<String, Value>,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
    pub locator: SourceLocator,
}
