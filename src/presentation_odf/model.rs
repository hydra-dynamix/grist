//! Public authoritative model for OpenDocument presentation packages.

use crate::container::EmbeddedArtifact;
use crate::core::{BoundingBox, ContentIdentity, SourceLocator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct OdfPresentationOptions {
    pub inline_embedded_artifact_bytes: bool,
}

impl crate::core::FormatOptions for OdfPresentationOptions {
    const FORMAT: &'static str = "presentation_odf";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OdfPresentationPackageKind {
    Presentation,
    Template,
}

impl OdfPresentationPackageKind {
    pub const fn format_id(self) -> &'static str {
        match self {
            Self::Presentation => "odp",
            Self::Template => "otp",
        }
    }

    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Presentation => "application/vnd.oasis.opendocument.presentation",
            Self::Template => "application/vnd.oasis.opendocument.presentation-template",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationDocument {
    pub schema_version: String,
    pub package_kind: OdfPresentationPackageKind,
    pub package_media_type: String,
    pub version: Option<String>,
    pub parts: Vec<OdfPresentationPart>,
    pub manifest: Vec<OdfPresentationManifestEntry>,
    pub metadata: Vec<OdfPresentationProperty>,
    pub settings: Vec<OdfPresentationProperty>,
    pub styles: Vec<OdfPresentationStyle>,
    pub page_layouts: Vec<OdfPresentationPageLayout>,
    pub master_pages: Vec<OdfPresentationMasterPage>,
    pub slides: Vec<OdfPresentationSlide>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub raw_elements: Vec<OdfPresentationXmlMetadata>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OdfPresentationPartStatus {
    Available,
    Directory,
    Encrypted,
    Rejected,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationPart {
    pub package_index: usize,
    pub path: String,
    pub media_type: Option<String>,
    pub compression: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub status: OdfPresentationPartStatus,
    pub rejection_code: Option<String>,
    pub identity: Option<ContentIdentity>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationManifestEntry {
    pub full_path: String,
    pub media_type: Option<String>,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub encrypted: bool,
    pub checksum: Option<String>,
    pub checksum_type: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationProperty {
    pub qualified_name: String,
    pub value: String,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationStyle {
    pub qualified_name: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub family: Option<String>,
    pub parent_style_name: Option<String>,
    pub page_layout_name: Option<String>,
    pub presentation_page_layout_name: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub properties: BTreeMap<String, BTreeMap<String, String>>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationPageLayout {
    pub name: Option<String>,
    pub page_width: Option<String>,
    pub page_height: Option<String>,
    pub orientation: Option<String>,
    pub print_orientation: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub placeholders: Vec<OdfPresentationPlaceholder>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationPlaceholder {
    pub presentation_class: Option<String>,
    pub object: Option<String>,
    pub geometry: OdfPresentationGeometry,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationMasterPage {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub page_layout_name: Option<String>,
    pub presentation_page_layout_name: Option<String>,
    pub style_name: Option<String>,
    pub shapes: Vec<OdfPresentationShape>,
    pub raw_elements: Vec<OdfPresentationXmlMetadata>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationSlide {
    pub order: usize,
    pub slide_id: String,
    pub name: Option<String>,
    pub master_page_name: Option<String>,
    pub style_name: Option<String>,
    pub presentation_page_layout_name: Option<String>,
    pub visible: bool,
    pub attributes: BTreeMap<String, String>,
    pub shapes: Vec<OdfPresentationShape>,
    pub notes: Vec<OdfPresentationNote>,
    pub comments: Vec<OdfPresentationComment>,
    pub tables: Vec<OdfPresentationTable>,
    pub charts: Vec<OdfPresentationChart>,
    pub images: Vec<OdfPresentationImage>,
    pub links: Vec<OdfPresentationLink>,
    pub transition: Option<OdfPresentationTransition>,
    pub animations: Vec<OdfPresentationAnimation>,
    pub embedded_objects: Vec<OdfPresentationEmbeddedObject>,
    pub reading_order: OdfPresentationReadingOrder,
    pub raw_elements: Vec<OdfPresentationXmlMetadata>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OdfPresentationShapeKind {
    Frame,
    Group,
    Rectangle,
    Ellipse,
    Line,
    Connector,
    CustomShape,
    Polygon,
    Polyline,
    Path,
    Caption,
    Measure,
    Control,
    PageThumbnail,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationShape {
    pub shape_id: String,
    pub qualified_name: String,
    pub kind: OdfPresentationShapeKind,
    pub parent_shape_id: Option<String>,
    pub z_order: usize,
    pub name: Option<String>,
    pub presentation_class: Option<String>,
    pub style_name: Option<String>,
    pub text_style_name: Option<String>,
    pub layer: Option<String>,
    pub geometry: OdfPresentationGeometry,
    pub text: Option<OdfPresentationTextBody>,
    pub alt_title: Option<String>,
    pub alt_description: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub raw_xml: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationGeometry {
    pub x: Option<String>,
    pub y: Option<String>,
    pub width: Option<String>,
    pub height: Option<String>,
    pub transform: Option<String>,
    pub view_box: Option<String>,
    pub points: Option<String>,
    pub path: Option<String>,
    pub rotation_angle: Option<String>,
    pub bbox_points: Option<BoundingBox>,
    pub attributes: BTreeMap<String, String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationTextBody {
    pub paragraphs: Vec<OdfPresentationTextParagraph>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationTextParagraph {
    pub index: usize,
    pub kind: String,
    pub level: Option<u32>,
    pub style_name: Option<String>,
    pub text: String,
    pub runs: Vec<OdfPresentationTextRun>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationTextRun {
    pub index: usize,
    pub kind: String,
    pub text: String,
    pub style_name: Option<String>,
    pub href: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationNote {
    pub text: String,
    pub shapes: Vec<OdfPresentationShape>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationComment {
    pub name: Option<String>,
    pub creator: Option<String>,
    pub created_at: Option<String>,
    pub text: String,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationTable {
    pub shape_id: String,
    pub name: Option<String>,
    pub columns: Vec<OdfPresentationTableColumn>,
    pub rows: Vec<OdfPresentationTableRow>,
    pub style_name: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationTableColumn {
    pub index: usize,
    pub repeated: usize,
    pub style_name: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationTableRow {
    pub index: usize,
    pub repeated: usize,
    pub cells: Vec<OdfPresentationTableCell>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationTableCell {
    pub row: usize,
    pub column: usize,
    pub repeated: usize,
    pub column_span: usize,
    pub row_span: usize,
    pub covered: bool,
    pub value_type: Option<String>,
    pub value: Option<String>,
    pub formula: Option<String>,
    pub text: String,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationChart {
    pub shape_id: String,
    pub href: String,
    pub resolved_members: Vec<String>,
    pub class: Option<String>,
    pub title: Option<String>,
    pub series: Vec<OdfPresentationChartSeries>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationChartSeries {
    pub index: usize,
    pub values_range: Option<String>,
    pub label_cell: Option<String>,
    pub class: Option<String>,
    pub categories_range: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationImage {
    pub shape_id: String,
    pub href: String,
    pub resolved_member: Option<String>,
    pub media_type: Option<String>,
    pub identity: Option<ContentIdentity>,
    pub alt_title: Option<String>,
    pub alt_description: Option<String>,
    pub artifact_ids: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationLink {
    pub source_shape_id: Option<String>,
    pub source_run: Option<usize>,
    pub href: String,
    pub resolved_member: Option<String>,
    pub external: bool,
    pub action: Option<String>,
    pub show: Option<String>,
    pub text: String,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationTransition {
    pub style: Option<String>,
    pub type_name: Option<String>,
    pub subtype: Option<String>,
    pub direction: Option<String>,
    pub duration: Option<String>,
    pub speed: Option<String>,
    pub advance_on_click: Option<bool>,
    pub advance_after: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationAnimation {
    pub index: usize,
    pub qualified_name: String,
    pub target_element: Option<String>,
    pub begin: Option<String>,
    pub duration: Option<String>,
    pub attribute_name: Option<String>,
    pub values: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub text: String,
    pub raw_xml: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationEmbeddedObject {
    pub shape_id: String,
    pub kind: String,
    pub href: Option<String>,
    pub resolved_members: Vec<String>,
    pub media_type: Option<String>,
    pub artifact_ids: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationReadingOrder {
    pub method: String,
    pub confidence: f64,
    pub entries: Vec<OdfPresentationReadingOrderEntry>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationReadingOrderEntry {
    pub rank: usize,
    pub shape_id: String,
    pub source_z_order: usize,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPresentationXmlMetadata {
    pub qualified_name: String,
    pub attributes: BTreeMap<String, String>,
    pub text: String,
    pub raw_xml: String,
    pub locator: SourceLocator,
}
