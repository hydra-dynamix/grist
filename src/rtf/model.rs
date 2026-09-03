//! Authoritative, loss-retaining Rich Text Format model.

use crate::container::EmbeddedArtifact;
use crate::core::SourceLocator;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RtfOptions {
    pub inline_embedded_artifact_bytes: bool,
    pub retain_raw_source_bytes: bool,
}

impl Default for RtfOptions {
    fn default() -> Self {
        Self {
            inline_embedded_artifact_bytes: false,
            retain_raw_source_bytes: true,
        }
    }
}

impl crate::core::FormatOptions for RtfOptions {
    const FORMAT: &'static str = "rtf";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfDocument {
    pub schema_version: String,
    pub rtf_version: i32,
    pub charset: RtfCharset,
    pub ansi_code_page: u16,
    pub default_font: Option<i32>,
    pub generator: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub root: RtfGroup,
    pub destinations: Vec<RtfDestinationOccurrence>,
    pub fonts: Vec<RtfFont>,
    pub colors: Vec<RtfColor>,
    pub styles: Vec<RtfStyleDefinition>,
    pub lists: Vec<RtfListDefinition>,
    pub list_overrides: Vec<RtfListOverride>,
    pub paragraphs: Vec<RtfParagraph>,
    pub list_items: Vec<RtfListItem>,
    pub tables: Vec<RtfTable>,
    pub fields: Vec<RtfField>,
    pub images: Vec<RtfImage>,
    pub objects: Vec<RtfObject>,
    pub revisions: Vec<RtfRevision>,
    pub comments: Vec<RtfComment>,
    pub unknown_controls: Vec<RtfControl>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub views: RtfTextViews,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_source_bytes: Option<Vec<u8>>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RtfCharset {
    Ansi,
    Mac,
    Pc,
    Pca,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfGroup {
    pub id: String,
    pub destination: Option<String>,
    pub ignorable: bool,
    pub closed: bool,
    pub contents: Vec<RtfElement>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RtfElement {
    Group { group: Box<RtfGroup> },
    Control { control: RtfControl },
    Text { text: RtfText },
    Binary { binary: RtfBinary },
    Malformed { malformed: RtfMalformed },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfControl {
    pub name: String,
    pub parameter: Option<i32>,
    pub control_symbol: bool,
    pub known: bool,
    pub raw_bytes: Vec<u8>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfText {
    pub decoded: String,
    pub raw_bytes: Vec<u8>,
    pub source_syntax: Vec<u8>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfBinary {
    pub declared_length: usize,
    pub bytes: Vec<u8>,
    pub truncated: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfMalformed {
    pub reason: String,
    pub raw_bytes: Vec<u8>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfDestinationOccurrence {
    pub group_id: String,
    pub name: String,
    pub ignorable: bool,
    pub recognized: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RtfCharacterStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub hidden: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub font: Option<i32>,
    pub font_size_half_points: Option<i32>,
    pub foreground_color: Option<i32>,
    pub background_color: Option<i32>,
    pub character_style: Option<i32>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfRun {
    pub id: String,
    pub text: String,
    pub style: RtfCharacterStyle,
    pub revision: Option<RtfRevisionKind>,
    pub revision_author: Option<i32>,
    pub revision_timestamp: Option<i64>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfParagraph {
    pub id: String,
    pub runs: Vec<RtfRun>,
    pub paragraph_style: Option<i32>,
    pub list_override: Option<i32>,
    pub list_level: Option<i32>,
    pub table_position: Option<RtfTablePosition>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RtfTablePosition {
    pub table_index: usize,
    pub row_index: usize,
    pub cell_index: usize,
    pub nesting_level: u32,
    pub cell_right_twips: Option<i32>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfFont {
    pub index: i32,
    pub name: String,
    pub family: Option<String>,
    pub charset: Option<i32>,
    pub code_page: Option<i32>,
    pub pitch: Option<i32>,
    pub alternate_name: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RtfColor {
    pub index: usize,
    pub red: Option<u8>,
    pub green: Option<u8>,
    pub blue: Option<u8>,
    pub auto: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfStyleDefinition {
    pub number: i32,
    pub kind: RtfStyleKind,
    pub name: String,
    pub based_on: Option<i32>,
    pub next_style: Option<i32>,
    pub additive: bool,
    pub controls: Vec<RtfControl>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RtfStyleKind {
    Paragraph,
    Character,
    Section,
    Table,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfListDefinition {
    pub list_id: i32,
    pub template_id: Option<i32>,
    pub simple: bool,
    pub hybrid: bool,
    pub levels: Vec<RtfListLevel>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfListLevel {
    pub level: usize,
    pub number_format: Option<i32>,
    pub start_at: Option<i32>,
    pub alignment: Option<i32>,
    pub follow: Option<i32>,
    pub level_text: String,
    pub level_numbers: Vec<u8>,
    pub controls: Vec<RtfControl>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfListOverride {
    pub list_id: Option<i32>,
    pub override_id: i32,
    pub override_count: Option<i32>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfListItem {
    pub paragraph_id: String,
    pub override_id: i32,
    pub level: i32,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfTable {
    pub id: String,
    pub nesting_level: u32,
    pub rows: Vec<RtfTableRow>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfTableRow {
    pub index: usize,
    pub cells: Vec<RtfTableCell>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfTableCell {
    pub index: usize,
    pub right_boundary_twips: Option<i32>,
    pub paragraph_ids: Vec<String>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfField {
    pub group_id: String,
    pub instruction: String,
    pub result: String,
    pub field_type: String,
    pub target: Option<String>,
    pub locked: bool,
    pub dirty: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfImage {
    pub group_id: String,
    pub media_type: Option<String>,
    pub width_pixels: Option<i32>,
    pub height_pixels: Option<i32>,
    pub width_goal_twips: Option<i32>,
    pub height_goal_twips: Option<i32>,
    pub scale_x_percent: Option<i32>,
    pub scale_y_percent: Option<i32>,
    pub binary_length: usize,
    pub artifact_id: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfObject {
    pub group_id: String,
    pub object_type: Option<String>,
    pub class_name: Option<String>,
    pub object_name: Option<String>,
    pub result_text: String,
    pub binary_length: usize,
    pub artifact_id: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RtfRevisionKind {
    Inserted,
    Deleted,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfRevision {
    pub run_id: String,
    pub kind: RtfRevisionKind,
    pub author_index: Option<i32>,
    pub timestamp: Option<i64>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RtfComment {
    pub group_id: String,
    pub annotation_id: Option<i32>,
    pub author: Option<String>,
    pub initials: Option<String>,
    pub text: String,
    pub range_start: Option<i32>,
    pub range_end: Option<i32>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RtfTextViews {
    pub visible: String,
    pub original: String,
    pub accepted: String,
    pub rejected: String,
}
