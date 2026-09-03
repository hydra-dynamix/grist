use crate::core::{ContentIdentity, SourceLocator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpreadsheetOoxmlOptions {
    pub include_parts: bool,
}

impl Default for SpreadsheetOoxmlOptions {
    fn default() -> Self {
        Self {
            include_parts: true,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetPackageKind {
    Workbook,
    MacroEnabledWorkbook,
}

impl SpreadsheetPackageKind {
    pub fn format_id(self) -> &'static str {
        match self {
            Self::Workbook => "xlsx",
            Self::MacroEnabledWorkbook => "xlsm",
        }
    }
    pub fn media_type(self) -> &'static str {
        match self {
            Self::Workbook => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::MacroEnabledWorkbook => "application/vnd.ms-excel.sheet.macroEnabled.12",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetOoxmlDocument {
    pub schema_version: String,
    pub package_kind: SpreadsheetPackageKind,
    pub package_media_type: String,
    pub workbook_part: String,
    pub workbook_locator: SourceLocator,
    pub date_system: SpreadsheetDateSystem,
    pub calculation: SpreadsheetCalculationProperties,
    pub sheets: Vec<SpreadsheetSheet>,
    pub named_ranges: Vec<SpreadsheetNamedRange>,
    pub styles: SpreadsheetStyles,
    pub properties: Vec<SpreadsheetProperty>,
    pub relationships: Vec<SpreadsheetRelationship>,
    pub parts: Vec<SpreadsheetPackagePart>,
    pub macro_projects: Vec<SpreadsheetMacroProject>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetDateSystem {
    #[default]
    Excel1900,
    Excel1904,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SpreadsheetCalculationProperties {
    pub calculation_id: Option<String>,
    pub mode: Option<String>,
    pub full_calculation_on_load: Option<bool>,
    pub force_full_calculation: Option<bool>,
    pub formulas_calculated_by_grist: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetVisibility {
    Visible,
    Hidden,
    VeryHidden,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetSheet {
    pub order: usize,
    pub sheet_id: String,
    pub name: String,
    pub visibility: SpreadsheetVisibility,
    pub part: Option<String>,
    pub dimension: Option<String>,
    pub pane: Option<SpreadsheetPane>,
    pub selections: Vec<SpreadsheetSelection>,
    pub columns: Vec<SpreadsheetColumn>,
    pub rows: Vec<SpreadsheetRow>,
    pub merges: Vec<SpreadsheetMerge>,
    pub hyperlinks: Vec<SpreadsheetHyperlink>,
    pub comments: Vec<SpreadsheetComment>,
    pub tables: Vec<SpreadsheetTable>,
    pub objects: Vec<SpreadsheetObject>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetPane {
    pub state: Option<String>,
    pub top_left_cell: Option<String>,
    pub active_pane: Option<String>,
    pub horizontal_split: Option<f64>,
    pub vertical_split: Option<f64>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetSelection {
    pub pane: Option<String>,
    pub active_cell: Option<String>,
    pub ranges: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetColumn {
    pub min: u32,
    pub max: u32,
    pub width: Option<f64>,
    pub style_id: Option<u32>,
    pub hidden: bool,
    pub outline_level: Option<u8>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetRow {
    pub row: u32,
    pub height: Option<f64>,
    pub style_id: Option<u32>,
    pub hidden: bool,
    pub outline_level: Option<u8>,
    pub cells: Vec<SpreadsheetCell>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetCell {
    pub reference: String,
    pub row: u32,
    pub column: u32,
    pub cell_type: SpreadsheetCellType,
    pub stored_value: Option<String>,
    pub displayed_value: Option<String>,
    pub formula: Option<SpreadsheetFormula>,
    pub cached_value: Option<SpreadsheetCachedValue>,
    pub style_id: Option<u32>,
    pub style: Option<SpreadsheetCellStyle>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetCellType {
    Number,
    SharedString,
    InlineString,
    String,
    Boolean,
    Error,
    Date,
    Blank,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetFormula {
    pub source: String,
    pub formula_type: Option<String>,
    pub reference: Option<String>,
    pub shared_index: Option<u32>,
    pub calculate: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpreadsheetCachedValue {
    pub stored_value: String,
    pub displayed_value: Option<String>,
    pub source: SpreadsheetCachedValueSource,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetCachedValueSource {
    WorkbookStoredFormulaCache,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpreadsheetCellStyle {
    pub style_id: u32,
    pub number_format_id: Option<u32>,
    pub number_format_code: Option<String>,
    pub font_id: Option<u32>,
    pub fill_id: Option<u32>,
    pub border_id: Option<u32>,
    pub horizontal_alignment: Option<String>,
    pub vertical_alignment: Option<String>,
    pub wrap_text: Option<bool>,
    pub text_rotation: Option<i16>,
    pub locked: Option<bool>,
    pub hidden_formula: Option<bool>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SpreadsheetStyles {
    pub number_formats: BTreeMap<u32, String>,
    pub cell_formats: Vec<SpreadsheetCellStyle>,
    pub fonts: Vec<SpreadsheetStyleRecord>,
    pub fills: Vec<SpreadsheetStyleRecord>,
    pub borders: Vec<SpreadsheetStyleRecord>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetStyleRecord {
    pub index: u32,
    pub attributes: BTreeMap<String, String>,
    pub values: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetMerge {
    pub range: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetHyperlink {
    pub range: String,
    pub target: Option<String>,
    pub location: Option<String>,
    pub display: Option<String>,
    pub tooltip: Option<String>,
    pub external: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetComment {
    pub reference: String,
    pub author: Option<String>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetTable {
    pub id: Option<u32>,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub range: String,
    pub header_rows: u32,
    pub totals_rows: u32,
    pub columns: Vec<SpreadsheetTableColumn>,
    pub style_name: Option<String>,
    pub part: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpreadsheetTableColumn {
    pub id: Option<u32>,
    pub name: Option<String>,
    pub totals_row_function: Option<String>,
    pub calculated_column_formula: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetObjectKind {
    Chart,
    Image,
    Drawing,
    EmbeddedObject,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetObject {
    pub kind: SpreadsheetObjectKind,
    pub name: Option<String>,
    pub part: Option<String>,
    pub content_type: Option<String>,
    pub identity: Option<ContentIdentity>,
    pub anchor_from: Option<String>,
    pub anchor_to: Option<String>,
    pub title: Option<String>,
    pub series_formulas: Vec<String>,
    pub cached_values: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetNamedRange {
    pub name: String,
    pub formula: String,
    pub local_sheet_id: Option<usize>,
    pub hidden: bool,
    pub comment: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetProperty {
    pub part: String,
    pub name: String,
    pub value: String,
    pub value_type: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetRelationshipTargetMode {
    Internal,
    External,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetRelationship {
    pub source_part: Option<String>,
    pub id: String,
    pub relationship_type: String,
    pub target: String,
    pub target_mode: SpreadsheetRelationshipTargetMode,
    pub resolved_part: Option<String>,
    pub target_exists: Option<bool>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetPackagePart {
    pub package_index: usize,
    pub path: String,
    pub content_type: Option<String>,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub status: String,
    pub identity: Option<ContentIdentity>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpreadsheetMacroProject {
    pub part: String,
    pub content_type: Option<String>,
    pub identity: ContentIdentity,
    pub byte_length: u64,
    pub classification: String,
    pub quarantined: bool,
    pub executable: bool,
    pub locator: SourceLocator,
}
