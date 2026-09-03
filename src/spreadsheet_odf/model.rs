//! Authoritative, source-preserving model for ODS and OTS packages.

use crate::core::{ContentIdentity, SourceLocator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SpreadsheetOdfOptions {
    pub include_parts: bool,
}

impl Default for SpreadsheetOdfOptions {
    fn default() -> Self {
        Self {
            include_parts: true,
        }
    }
}

impl crate::core::FormatOptions for SpreadsheetOdfOptions {
    const FORMAT: &'static str = "spreadsheet_odf";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetOdfPackageKind {
    Workbook,
    Template,
}

impl SpreadsheetOdfPackageKind {
    pub const fn format_id(self) -> &'static str {
        match self {
            Self::Workbook => "ods",
            Self::Template => "ots",
        }
    }
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Workbook => "application/vnd.oasis.opendocument.spreadsheet",
            Self::Template => "application/vnd.oasis.opendocument.spreadsheet-template",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfDocument {
    pub schema_version: String,
    pub package_kind: SpreadsheetOdfPackageKind,
    pub package_media_type: String,
    pub version: Option<String>,
    pub workbook_locator: SourceLocator,
    pub calculation: SpreadsheetOdfCalculation,
    pub active_table: Option<String>,
    pub panes: Vec<SpreadsheetOdfPane>,
    pub sheets: Vec<SpreadsheetOdfSheet>,
    pub named_ranges: Vec<SpreadsheetOdfNamedRange>,
    pub styles: Vec<SpreadsheetOdfStyle>,
    pub metadata: Vec<SpreadsheetOdfMetadata>,
    pub manifest: Vec<SpreadsheetOdfManifestEntry>,
    pub raw_elements: Vec<SpreadsheetOdfRawElement>,
    pub parts: Vec<SpreadsheetOdfPart>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpreadsheetOdfCalculation {
    pub case_sensitive: Option<bool>,
    pub precision_as_shown: Option<bool>,
    pub search_criteria_must_apply_to_whole_cell: Option<bool>,
    pub automatic_find_labels: Option<bool>,
    pub null_year: Option<u32>,
    pub iteration_enabled: Option<bool>,
    pub iteration_steps: Option<u32>,
    pub iteration_maximum_difference: Option<String>,
    pub formulas_calculated_by_grist: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetOdfVisibility {
    Visible,
    Hidden,
    Filtered,
    Collapsed,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfSheet {
    pub order: usize,
    pub name: String,
    pub style_name: Option<String>,
    pub visibility: SpreadsheetOdfVisibility,
    pub protected: bool,
    pub print_ranges: Option<String>,
    pub columns: Vec<SpreadsheetOdfColumn>,
    pub rows: Vec<SpreadsheetOdfRow>,
    pub merges: Vec<SpreadsheetOdfMerge>,
    pub comments: Vec<SpreadsheetOdfComment>,
    pub links: Vec<SpreadsheetOdfLink>,
    pub objects: Vec<SpreadsheetOdfObject>,
    pub raw_elements: Vec<SpreadsheetOdfRawElement>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfColumn {
    pub column: u32,
    pub repeated: u32,
    pub style_name: Option<String>,
    pub default_cell_style_name: Option<String>,
    pub visibility: SpreadsheetOdfVisibility,
    pub width: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfRow {
    pub row: u32,
    pub repeated: u32,
    pub style_name: Option<String>,
    pub default_cell_style_name: Option<String>,
    pub visibility: SpreadsheetOdfVisibility,
    pub height: Option<String>,
    pub cells: Vec<SpreadsheetOdfCell>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetOdfCellKind {
    Cell,
    Covered,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfCell {
    pub source_xml: String,
    pub reference: String,
    pub row: u32,
    pub column: u32,
    pub repeated: u32,
    pub kind: SpreadsheetOdfCellKind,
    pub value_type: Option<String>,
    pub stored_value: Option<String>,
    pub displayed_value: Option<String>,
    pub currency: Option<String>,
    pub formula: Option<SpreadsheetOdfFormula>,
    pub cached_value: Option<SpreadsheetOdfCachedValue>,
    pub style_name: Option<String>,
    pub validation_name: Option<String>,
    pub columns_spanned: u32,
    pub rows_spanned: u32,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfFormula {
    pub source: String,
    pub namespace_prefix: Option<String>,
    pub calculate: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpreadsheetOdfCachedValue {
    pub stored_value: String,
    pub displayed_value: Option<String>,
    pub value_type: Option<String>,
    pub source: SpreadsheetOdfCachedValueSource,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetOdfCachedValueSource {
    PackageStoredFormulaResult,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfMerge {
    pub range: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfComment {
    pub reference: String,
    pub creator: Option<String>,
    pub date: Option<String>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfLink {
    pub reference: String,
    pub target: String,
    pub label: Option<String>,
    pub external: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetOdfObjectKind {
    Chart,
    Image,
    EmbeddedObject,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfObject {
    pub kind: SpreadsheetOdfObjectKind,
    pub name: Option<String>,
    pub title: Option<String>,
    pub href: Option<String>,
    pub media_type: Option<String>,
    pub identity: Option<ContentIdentity>,
    pub anchor_cell: String,
    pub end_cell: Option<String>,
    pub source_ranges: Vec<String>,
    pub cached_values: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfNamedRange {
    pub name: String,
    pub expression: String,
    pub base_cell_address: Option<String>,
    pub range_usable_as: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfStyle {
    pub name: String,
    pub family: Option<String>,
    pub parent_style_name: Option<String>,
    pub data_style_name: Option<String>,
    pub origin_part: String,
    pub properties: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfPane {
    pub view_name: Option<String>,
    pub active_table: Option<String>,
    pub horizontal_split_mode: Option<String>,
    pub vertical_split_mode: Option<String>,
    pub horizontal_split_position: Option<u32>,
    pub vertical_split_position: Option<u32>,
    pub frozen: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfMetadata {
    pub name: String,
    pub value: String,
    pub value_type: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfManifestEntry {
    pub full_path: String,
    pub media_type: Option<String>,
    pub version: Option<String>,
    pub encrypted: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfRawElement {
    pub name: String,
    pub raw_xml: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOdfPart {
    pub package_index: usize,
    pub path: String,
    pub media_type: Option<String>,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub compression: String,
    pub status: String,
    pub encrypted: bool,
    pub rejection_code: Option<String>,
    pub rejection_message: Option<String>,
    pub identity: Option<ContentIdentity>,
    pub locator: SourceLocator,
}
