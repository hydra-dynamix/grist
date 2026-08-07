//! Public authoritative model for OpenDocument text packages.

use crate::container::EmbeddedArtifact;
use crate::core::{ContentIdentity, SourceLocator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OdfWordOptions {
    pub inline_embedded_artifact_bytes: bool,
}

impl crate::core::FormatOptions for OdfWordOptions {
    const FORMAT: &'static str = "odf_word";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OdfPackageKind {
    Document,
    Template,
}

impl OdfPackageKind {
    pub const fn format_id(self) -> &'static str {
        match self {
            Self::Document => "odt",
            Self::Template => "ott",
        }
    }

    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Document => "application/vnd.oasis.opendocument.text",
            Self::Template => "application/vnd.oasis.opendocument.text-template",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfWordDocument {
    pub schema_version: String,
    pub package_kind: OdfPackageKind,
    pub package_media_type: String,
    pub version: Option<String>,
    pub parts: Vec<OdfPackagePart>,
    pub manifest: Vec<OdfManifestEntry>,
    pub metadata: Vec<OdfProperty>,
    pub settings: Vec<OdfProperty>,
    pub styles: Vec<OdfStyle>,
    pub list_styles: Vec<OdfListStyle>,
    pub master_pages: Vec<OdfMasterPage>,
    pub body: OdfNode,
    pub revisions: Vec<OdfRevision>,
    pub links: Vec<OdfLink>,
    pub notes: Vec<OdfNote>,
    pub annotations: Vec<OdfAnnotation>,
    pub drawings: Vec<OdfDrawing>,
    pub equations: Vec<OdfEquation>,
    pub embedded_objects: Vec<OdfEmbeddedObject>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub views: OdfTextViews,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OdfPartStatus {
    Available,
    Directory,
    Encrypted,
    Rejected,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfPackagePart {
    pub package_index: usize,
    pub path: String,
    pub media_type: Option<String>,
    pub compression: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub status: OdfPartStatus,
    pub rejection_code: Option<String>,
    pub identity: Option<ContentIdentity>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfManifestEntry {
    pub full_path: String,
    pub media_type: Option<String>,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub encrypted: bool,
    pub checksum: Option<String>,
    pub checksum_type: Option<String>,
    pub encryption_attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfProperty {
    pub qualified_name: String,
    pub value: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfStyle {
    pub qualified_name: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub family: Option<String>,
    pub parent_style_name: Option<String>,
    pub next_style_name: Option<String>,
    pub list_style_name: Option<String>,
    pub data_style_name: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub properties: BTreeMap<String, BTreeMap<String, String>>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfListStyle {
    pub name: Option<String>,
    pub consecutive_numbering: Option<String>,
    pub levels: Vec<OdfListLevel>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfListLevel {
    pub level: Option<u32>,
    pub kind: String,
    pub number_format: Option<String>,
    pub bullet_character: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub start_value: Option<i64>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfMasterPage {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub page_layout_name: Option<String>,
    pub next_style_name: Option<String>,
    pub headers: Vec<OdfNode>,
    pub footers: Vec<OdfNode>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OdfNodeKind {
    Document,
    Section,
    Heading,
    Paragraph,
    Span,
    List,
    ListItem,
    Table,
    TableRow,
    TableCell,
    CoveredTableCell,
    Link,
    Bookmark,
    Reference,
    Field,
    Footnote,
    Endnote,
    NoteCitation,
    NoteBody,
    Header,
    Footer,
    Annotation,
    AnnotationEnd,
    RevisionContainer,
    RevisionRegion,
    ChangeStart,
    ChangeEnd,
    Change,
    Drawing,
    Image,
    TextBox,
    EmbeddedObject,
    Equation,
    Space,
    Tab,
    LineBreak,
    PageBreak,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfNode {
    pub id: String,
    pub kind: OdfNodeKind,
    pub qualified_name: String,
    pub attributes: BTreeMap<String, String>,
    pub style_name: Option<String>,
    pub change_id: Option<String>,
    pub content: Vec<OdfContent>,
    pub raw_xml: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OdfContent {
    Text {
        value: String,
        locator: SourceLocator,
    },
    Element {
        node: Box<OdfNode>,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OdfRevisionKind {
    Insertion,
    Deletion,
    FormatChange,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfRevision {
    pub id: String,
    pub kind: OdfRevisionKind,
    pub creator: Option<String>,
    pub date: Option<String>,
    pub comment: Option<String>,
    pub deleted_content: Vec<OdfContent>,
    pub deleted_text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OdfTextViews {
    pub visible: String,
    pub original: String,
    pub accepted: String,
    pub rejected: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfLink {
    pub source_node_id: String,
    pub href: String,
    pub resolved_member: Option<String>,
    pub fragment: Option<String>,
    pub external: bool,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfNote {
    pub source_node_id: String,
    pub id: Option<String>,
    pub class: String,
    pub citation: String,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfAnnotation {
    pub source_node_id: String,
    pub name: Option<String>,
    pub creator: Option<String>,
    pub date: Option<String>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OdfDrawingKind {
    Frame,
    Image,
    TextBox,
    Object,
    OleObject,
    Shape,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfDrawing {
    pub source_node_id: String,
    pub kind: OdfDrawingKind,
    pub name: Option<String>,
    pub href: Option<String>,
    pub resolved_member: Option<String>,
    pub mime_type: Option<String>,
    pub alt_text: Option<String>,
    pub text: String,
    pub geometry: BTreeMap<String, String>,
    pub artifact_ids: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfEquation {
    pub source_node_id: String,
    pub text: String,
    pub mathml: String,
    pub embedded_member: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OdfEmbeddedObject {
    pub source_node_id: String,
    pub href: String,
    pub resolved_members: Vec<String>,
    pub media_type: Option<String>,
    pub artifact_ids: Vec<String>,
    pub locator: SourceLocator,
}
