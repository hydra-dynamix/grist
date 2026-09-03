//! Public authoritative PresentationML package model.

use crate::container::EmbeddedArtifact;
use crate::core::{ContentIdentity, SourceLocator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PresentationOoxmlOptions {
    pub inline_child_artifact_bytes: bool,
    pub extract_macro_bytes: bool,
}

impl crate::core::FormatOptions for PresentationOoxmlOptions {
    const FORMAT: &'static str = "presentation_ooxml";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PresentationPackageKind {
    Presentation,
    MacroEnabledPresentation,
    Template,
    Slideshow,
}

impl PresentationPackageKind {
    pub const fn format_id(self) -> &'static str {
        match self {
            Self::Presentation => "pptx",
            Self::MacroEnabledPresentation => "pptm",
            Self::Template => "potx",
            Self::Slideshow => "ppsx",
        }
    }

    pub const fn macro_enabled(self) -> bool {
        matches!(self, Self::MacroEnabledPresentation)
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationOoxmlDocument {
    pub schema_version: String,
    pub package_kind: PresentationPackageKind,
    pub package_media_type: String,
    pub main_presentation_part: String,
    pub main_presentation_locator: SourceLocator,
    pub content_types: PresentationContentTypes,
    pub parts: Vec<PresentationPackagePart>,
    pub relationships: Vec<PresentationRelationship>,
    pub properties: PresentationProperties,
    pub slides: Vec<PresentationSlideReference>,
    /// Authoritative per-slide structure in presentation order.
    #[serde(default)]
    pub slide_contents: Vec<PresentationSlideContent>,
    pub masters: Vec<PresentationStructuralPart>,
    pub layouts: Vec<PresentationStructuralPart>,
    pub themes: Vec<PresentationStructuralPart>,
    pub actions: Vec<PresentationAction>,
    pub macro_projects: Vec<PresentationMacroProject>,
    pub child_artifacts: Vec<PresentationChildArtifact>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationContentTypes {
    pub locator: SourceLocator,
    pub defaults: Vec<PresentationContentTypeDefault>,
    pub overrides: Vec<PresentationContentTypeOverride>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationContentTypeDefault {
    pub extension: String,
    pub content_type: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationContentTypeOverride {
    pub part_name: String,
    pub content_type: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationPartStatus {
    Available,
    Directory,
    Encrypted,
    Rejected,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationPackagePart {
    pub package_index: usize,
    pub path: String,
    pub content_type: Option<String>,
    pub compression: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub status: PresentationPartStatus,
    pub rejection_code: Option<String>,
    pub identity: Option<ContentIdentity>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationRelationshipTargetMode {
    Internal,
    External,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationRelationship {
    pub relationship_part: String,
    pub source_part: Option<String>,
    pub id: String,
    pub relationship_type: String,
    pub target: String,
    pub target_mode: PresentationRelationshipTargetMode,
    pub resolved_part: Option<String>,
    pub target_exists: Option<bool>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PresentationProperties {
    pub core: Vec<PresentationProperty>,
    pub extended: Vec<PresentationProperty>,
    pub custom: Vec<PresentationCustomProperty>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationProperty {
    pub part: String,
    pub namespace: Option<String>,
    pub name: String,
    pub value: String,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationCustomProperty {
    pub part: String,
    pub name: Option<String>,
    pub property_id: Option<i32>,
    pub format_id: Option<String>,
    pub link_target: Option<String>,
    pub value_type: Option<String>,
    pub value: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationSlideReference {
    pub order: usize,
    pub slide_id: String,
    pub relationship_id: String,
    pub part: Option<String>,
    pub part_identity: Option<ContentIdentity>,
    pub hidden: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationStructuralPartKind {
    Master,
    Layout,
    Theme,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationStructuralPart {
    pub kind: PresentationStructuralPartKind,
    pub native_id: Option<String>,
    pub relationship_id: Option<String>,
    pub source_part: String,
    pub part: String,
    pub part_identity: ContentIdentity,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationActionKind {
    Click,
    Hover,
    Hyperlink,
    Action,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationAction {
    pub source_part: String,
    pub action_kind: PresentationActionKind,
    pub relationship_id: Option<String>,
    pub action: Option<String>,
    pub target: Option<String>,
    pub external: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationMacroProject {
    pub part: String,
    pub content_type: Option<String>,
    pub relationship_ids: Vec<String>,
    pub identity: ContentIdentity,
    pub locator: SourceLocator,
    pub artifact: EmbeddedArtifact,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationChildArtifact {
    pub part: String,
    pub content_type: Option<String>,
    pub relationship_ids: Vec<String>,
    pub locator: SourceLocator,
    pub artifact: EmbeddedArtifact,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationSlideContent {
    pub order: usize,
    pub slide_id: String,
    pub part: String,
    pub locator: SourceLocator,
    pub shapes: Vec<PresentationShape>,
    pub notes: Vec<PresentationNote>,
    pub comments: Vec<PresentationComment>,
    pub tables: Vec<PresentationTable>,
    pub charts: Vec<PresentationChart>,
    pub equations: Vec<PresentationEquation>,
    pub images: Vec<PresentationImage>,
    pub links: Vec<PresentationLink>,
    pub transition: Option<PresentationTransition>,
    pub animations: Vec<PresentationAnimation>,
    pub embedded_objects: Vec<PresentationEmbeddedObject>,
    pub reading_order: PresentationReadingOrder,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationShapeKind {
    Shape,
    Picture,
    GraphicFrame,
    Group,
    Connector,
    ContentPart,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationShape {
    pub shape_id: String,
    pub source_element: String,
    pub kind: PresentationShapeKind,
    pub parent_shape_id: Option<String>,
    pub z_order: usize,
    pub name: Option<String>,
    pub placeholder_type: Option<String>,
    pub placeholder_index: Option<String>,
    pub alt_text: Option<String>,
    pub alt_title: Option<String>,
    pub decorative: bool,
    pub hidden: bool,
    pub geometry: Option<PresentationShapeGeometry>,
    pub text_body: Option<PresentationTextBody>,
    pub relationship_ids: Vec<String>,
    pub metadata: Vec<PresentationXmlMetadata>,
    pub locator: SourceLocator,
}

/// Raw PresentationML transform values. Coordinates and extents are EMUs;
/// rotation is stored in native 1/60000-degree units.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationShapeGeometry {
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub child_x: Option<i64>,
    pub child_y: Option<i64>,
    pub child_width: Option<i64>,
    pub child_height: Option<i64>,
    pub rotation: Option<i64>,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    pub preset_geometry: Option<String>,
    pub has_custom_geometry: bool,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationTextBody {
    pub paragraphs: Vec<PresentationTextParagraph>,
    pub text: String,
    pub properties: Vec<PresentationXmlMetadata>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationTextParagraph {
    pub index: usize,
    pub level: Option<u32>,
    pub alignment: Option<String>,
    pub text: String,
    pub runs: Vec<PresentationTextRun>,
    pub properties: Vec<PresentationXmlMetadata>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresentationTextRunKind {
    Text,
    Field,
    Break,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationTextRun {
    pub index: usize,
    pub kind: PresentationTextRunKind,
    pub text: String,
    pub field_id: Option<String>,
    pub field_type: Option<String>,
    pub language: Option<String>,
    pub font_size: Option<i64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<String>,
    pub typeface: Option<String>,
    pub color: Option<String>,
    pub hyperlink_relationship_id: Option<String>,
    pub hyperlink_action: Option<String>,
    pub properties: Vec<PresentationXmlMetadata>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationXmlMetadata {
    pub element: String,
    pub attributes: BTreeMap<String, String>,
    pub text: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationNote {
    pub part: String,
    pub text: String,
    pub shapes: Vec<PresentationShape>,
    pub tables: Vec<PresentationTable>,
    pub charts: Vec<PresentationChart>,
    pub equations: Vec<PresentationEquation>,
    pub images: Vec<PresentationImage>,
    pub links: Vec<PresentationLink>,
    pub embedded_objects: Vec<PresentationEmbeddedObject>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationComment {
    pub comment_id: String,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    pub author_initials: Option<String>,
    pub created_at: Option<String>,
    pub parent_comment_id: Option<String>,
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub text: String,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationTable {
    pub shape_id: String,
    pub grid_columns: Vec<i64>,
    pub rows: Vec<PresentationTableRow>,
    pub style_id: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationTableRow {
    pub index: usize,
    pub height: Option<i64>,
    pub cells: Vec<PresentationTableCell>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationTableCell {
    pub row: usize,
    pub column: usize,
    pub grid_span: usize,
    pub row_span: usize,
    pub horizontal_merge: bool,
    pub vertical_merge: bool,
    pub text_body: Option<PresentationTextBody>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationChart {
    pub shape_id: String,
    pub relationship_id: String,
    pub part: Option<String>,
    pub chart_types: Vec<String>,
    pub title: Option<String>,
    pub series: Vec<PresentationChartSeries>,
    pub external_data_relationship_id: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationChartSeries {
    pub index: usize,
    pub name: Option<String>,
    pub categories: Vec<String>,
    pub values: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationEquation {
    pub shape_id: String,
    pub display: bool,
    pub text: String,
    pub metadata: Vec<PresentationXmlMetadata>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationImage {
    pub shape_id: String,
    pub relationship_id: Option<String>,
    pub linked_relationship_id: Option<String>,
    pub part: Option<String>,
    pub content_type: Option<String>,
    pub part_identity: Option<ContentIdentity>,
    pub alt_text: Option<String>,
    pub alt_title: Option<String>,
    pub crop: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationLink {
    pub source_shape_id: String,
    pub source_run: Option<usize>,
    pub kind: PresentationActionKind,
    pub relationship_id: Option<String>,
    pub action: Option<String>,
    pub target: Option<String>,
    pub external: bool,
    pub tooltip: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationTransition {
    pub kind: Option<String>,
    pub advance_on_click: Option<bool>,
    pub advance_after_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub speed: Option<String>,
    pub sound_relationship_id: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub metadata: Vec<PresentationXmlMetadata>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationAnimation {
    pub index: usize,
    pub element: String,
    pub target_shape_ids: Vec<String>,
    pub relationship_ids: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub text: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationEmbeddedObject {
    pub shape_id: String,
    pub relationship_id: Option<String>,
    pub program_id: Option<String>,
    pub name: Option<String>,
    pub show_as_icon: bool,
    pub part: Option<String>,
    pub content_type: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationReadingOrder {
    pub method: String,
    pub confidence: f64,
    pub entries: Vec<PresentationReadingOrderEntry>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentationReadingOrderEntry {
    pub rank: usize,
    pub shape_id: String,
    pub source_z_order: usize,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub locator: SourceLocator,
}
