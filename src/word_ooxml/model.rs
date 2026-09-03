//! Public authoritative Word OOXML package model.

use crate::container::EmbeddedArtifact;
use crate::core::{ContentIdentity, SourceLocator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WordOoxmlOptions {
    pub inline_child_artifact_bytes: bool,
    pub extract_macro_bytes: bool,
}

impl crate::core::FormatOptions for WordOoxmlOptions {
    const FORMAT: &'static str = "word_ooxml";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WordPackageKind {
    Document,
    MacroEnabledDocument,
    Template,
    MacroEnabledTemplate,
}

impl WordPackageKind {
    pub const fn format_id(self) -> &'static str {
        match self {
            Self::Document => "docx",
            Self::MacroEnabledDocument => "docm",
            Self::Template => "dotx",
            Self::MacroEnabledTemplate => "dotm",
        }
    }

    pub const fn macro_enabled(self) -> bool {
        matches!(
            self,
            Self::MacroEnabledDocument | Self::MacroEnabledTemplate
        )
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordOoxmlDocument {
    pub schema_version: String,
    pub package_kind: WordPackageKind,
    pub package_media_type: String,
    pub main_document_part: String,
    pub main_document_locator: SourceLocator,
    pub content_types: WordContentTypes,
    pub parts: Vec<WordPackagePart>,
    pub relationships: Vec<WordRelationship>,
    pub properties: WordDocumentProperties,
    pub macro_projects: Vec<WordMacroProject>,
    pub child_artifacts: Vec<WordChildArtifact>,
    #[serde(default)]
    pub styles: Vec<WordStyle>,
    #[serde(default)]
    pub numbering: WordNumbering,
    #[serde(default)]
    pub body: WordStory,
    #[serde(default)]
    pub footnotes: Vec<WordNote>,
    #[serde(default)]
    pub endnotes: Vec<WordNote>,
    #[serde(default)]
    pub headers: Vec<WordHeaderFooter>,
    #[serde(default)]
    pub footers: Vec<WordHeaderFooter>,
    #[serde(default)]
    pub revision_graph: WordRevisionGraph,
    #[serde(default)]
    pub comments: Vec<WordComment>,
    #[serde(default)]
    pub content_controls: Vec<WordContentControl>,
    #[serde(default)]
    pub equations: Vec<WordEquation>,
    #[serde(default)]
    pub drawings: Vec<WordDrawing>,
    #[serde(default)]
    pub charts: Vec<WordChart>,
    #[serde(default)]
    pub captions: Vec<WordCaption>,
    #[serde(default)]
    pub text_boxes: Vec<WordTextBox>,
    #[serde(default)]
    pub embedded_objects: Vec<WordEmbeddedObject>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordContentTypes {
    pub locator: SourceLocator,
    pub defaults: Vec<WordContentTypeDefault>,
    pub overrides: Vec<WordContentTypeOverride>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordContentTypeDefault {
    pub extension: String,
    pub content_type: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordContentTypeOverride {
    pub part_name: String,
    pub content_type: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordPartStatus {
    Available,
    Directory,
    Encrypted,
    Rejected,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordPackagePart {
    pub package_index: usize,
    pub path: String,
    pub content_type: Option<String>,
    pub compression: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub status: WordPartStatus,
    pub rejection_code: Option<String>,
    pub identity: Option<ContentIdentity>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordRelationshipTargetMode {
    Internal,
    External,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordRelationship {
    pub relationship_part: String,
    pub source_part: Option<String>,
    pub id: String,
    pub relationship_type: String,
    pub target: String,
    pub target_mode: WordRelationshipTargetMode,
    pub resolved_part: Option<String>,
    pub target_exists: Option<bool>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WordDocumentProperties {
    pub core: Vec<WordProperty>,
    pub extended: Vec<WordProperty>,
    pub custom: Vec<WordCustomProperty>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordProperty {
    pub part: String,
    pub namespace: Option<String>,
    pub name: String,
    pub value: String,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordCustomProperty {
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
pub struct WordMacroProject {
    pub part: String,
    pub content_type: Option<String>,
    pub relationship_ids: Vec<String>,
    pub identity: ContentIdentity,
    pub locator: SourceLocator,
    pub artifact: EmbeddedArtifact,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordChildArtifact {
    pub part: String,
    pub content_type: Option<String>,
    pub relationship_ids: Vec<String>,
    pub locator: SourceLocator,
    pub artifact: EmbeddedArtifact,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WordStory {
    pub part: String,
    pub blocks: Vec<WordBlock>,
    pub sections: Vec<WordSection>,
    pub visible_text: String,
    pub locator: Option<SourceLocator>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WordBlock {
    Paragraph(Box<WordParagraph>),
    Table(Box<WordTable>),
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordParagraph {
    pub index: usize,
    pub text: String,
    pub style_id: Option<String>,
    pub style_name: Option<String>,
    pub heading_level: Option<u8>,
    pub direct_properties: WordParagraphProperties,
    pub properties: WordParagraphProperties,
    pub inlines: Vec<WordInline>,
    pub fields: Vec<WordField>,
    pub citations: Vec<WordCitation>,
    pub cross_references: Vec<WordCrossReference>,
    pub section: Option<WordSection>,
    pub numbering_reference: Option<WordNumberingReference>,
    pub numbering: Option<WordResolvedNumbering>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WordInline {
    Run(Box<WordRun>),
    Hyperlink(WordHyperlink),
    Bookmark(WordBookmark),
    Field(WordField),
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordRun {
    pub index: usize,
    pub text: String,
    pub style_id: Option<String>,
    pub direct_formatting: WordRunFormatting,
    pub effective_formatting: WordRunFormatting,
    pub contents: Vec<WordRunContent>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WordRunContent {
    Text {
        value: String,
        preserve_space: bool,
        locator: SourceLocator,
    },
    Tab {
        locator: SourceLocator,
    },
    Break(WordBreak),
    CarriageReturn {
        locator: SourceLocator,
    },
    SoftHyphen {
        locator: SourceLocator,
    },
    NoBreakHyphen {
        locator: SourceLocator,
    },
    Symbol {
        font: Option<String>,
        character: Option<String>,
        locator: SourceLocator,
    },
    FootnoteReference {
        id: String,
        locator: SourceLocator,
    },
    EndnoteReference {
        id: String,
        locator: SourceLocator,
    },
    FieldInstruction {
        instruction: String,
        locator: SourceLocator,
    },
    FieldCharacter {
        character_type: String,
        locked: Option<bool>,
        dirty: Option<bool>,
        locator: SourceLocator,
    },
    LastRenderedPageBreak {
        locator: SourceLocator,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordBreak {
    pub break_type: String,
    pub clear: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordHyperlink {
    pub relationship_id: Option<String>,
    pub target: Option<String>,
    pub anchor: Option<String>,
    pub tooltip: Option<String>,
    pub history: Option<bool>,
    pub text: String,
    pub runs: Vec<WordRun>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordBookmarkKind {
    Start,
    End,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordBookmark {
    pub id: String,
    pub name: Option<String>,
    pub bookmark_kind: WordBookmarkKind,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordField {
    pub instruction: String,
    pub field_type: String,
    pub result_text: String,
    #[serde(default)]
    pub result_runs: Vec<WordRun>,
    pub locked: Option<bool>,
    pub dirty: Option<bool>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordCitation {
    pub instruction: String,
    pub tags: Vec<String>,
    pub result_text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordCrossReference {
    pub reference_type: String,
    pub target: String,
    pub result_text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WordRunFormatting {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<String>,
    pub strike: Option<bool>,
    pub double_strike: Option<bool>,
    pub color: Option<String>,
    pub highlight: Option<String>,
    pub font_size_half_points: Option<i32>,
    pub fonts: BTreeMap<String, String>,
    pub language: BTreeMap<String, String>,
    pub vertical_alignment: Option<String>,
    pub hidden: Option<bool>,
    pub all_caps: Option<bool>,
    pub small_caps: Option<bool>,
    pub properties: Vec<WordXmlProperty>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WordParagraphProperties {
    pub alignment: Option<String>,
    pub outline_level: Option<u8>,
    pub keep_next: Option<bool>,
    pub keep_lines: Option<bool>,
    pub page_break_before: Option<bool>,
    pub widow_control: Option<bool>,
    pub contextual_spacing: Option<bool>,
    pub indentation: BTreeMap<String, String>,
    pub spacing: BTreeMap<String, String>,
    pub properties: Vec<WordXmlProperty>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordXmlProperty {
    pub name: String,
    pub value: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordStyle {
    pub style_id: String,
    pub style_type: String,
    pub name: Option<String>,
    pub based_on: Option<String>,
    pub next: Option<String>,
    pub linked_style: Option<String>,
    pub is_default: bool,
    pub custom: bool,
    pub ui_priority: Option<i32>,
    pub hidden: Option<bool>,
    pub semi_hidden: Option<bool>,
    pub paragraph_properties: WordParagraphProperties,
    pub run_formatting: WordRunFormatting,
    pub effective_paragraph_properties: WordParagraphProperties,
    pub effective_run_formatting: WordRunFormatting,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WordNumbering {
    pub abstract_definitions: Vec<WordAbstractNumbering>,
    pub instances: Vec<WordNumberingInstance>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordAbstractNumbering {
    pub abstract_num_id: i64,
    pub multi_level_type: Option<String>,
    pub number_style_link: Option<String>,
    pub style_link: Option<String>,
    pub levels: Vec<WordNumberingLevel>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordNumberingLevel {
    pub level: u8,
    pub start: i64,
    pub number_format: String,
    pub level_text: String,
    pub suffix: Option<String>,
    pub alignment: Option<String>,
    pub paragraph_style: Option<String>,
    pub restart_after_level: Option<u8>,
    pub picture_bullet_id: Option<i64>,
    pub paragraph_properties: WordParagraphProperties,
    pub run_formatting: WordRunFormatting,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordNumberingInstance {
    pub num_id: i64,
    pub abstract_num_id: i64,
    pub overrides: Vec<WordNumberingOverride>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordNumberingOverride {
    pub level: u8,
    pub start_override: Option<i64>,
    pub level_definition: Option<WordNumberingLevel>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordResolvedNumbering {
    pub num_id: i64,
    pub abstract_num_id: i64,
    pub level: u8,
    pub ordinal: i64,
    pub label: String,
    pub ordered: bool,
    pub number_format: String,
    pub level_text: String,
    pub definition_locator: SourceLocator,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordNumberingReference {
    pub num_id: i64,
    pub level: u8,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordTable {
    pub index: usize,
    pub style_id: Option<String>,
    pub width: Option<String>,
    pub layout: Option<String>,
    pub grid_columns: Vec<Option<String>>,
    pub rows: Vec<WordTableRow>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordTableRow {
    pub index: usize,
    pub is_header: bool,
    pub cant_split: bool,
    pub height: Option<String>,
    pub cells: Vec<WordTableCell>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordTableCell {
    pub index: usize,
    pub grid_column: usize,
    pub column_span: usize,
    pub row_span: usize,
    pub vertical_merge: Option<String>,
    pub horizontal_merge: Option<String>,
    pub merged_into: Option<(usize, usize)>,
    pub width: Option<String>,
    pub blocks: Vec<WordBlock>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordSection {
    pub index: usize,
    pub break_type: Option<String>,
    pub columns: WordColumns,
    pub page_size: BTreeMap<String, String>,
    pub page_margins: BTreeMap<String, String>,
    pub title_page: bool,
    pub header_references: Vec<WordStoryReference>,
    pub footer_references: Vec<WordStoryReference>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WordColumns {
    pub count: usize,
    pub spacing: Option<String>,
    pub equal_width: Option<bool>,
    pub separator: Option<bool>,
    pub definitions: Vec<BTreeMap<String, String>>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordStoryReference {
    pub reference_type: String,
    pub relationship_id: String,
    pub part: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordNote {
    pub id: String,
    pub note_type: Option<String>,
    pub story: WordStory,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordHeaderFooter {
    pub part: String,
    pub relationship_ids: Vec<String>,
    pub story: WordStory,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WordRevisionGraph {
    pub revisions: Vec<WordRevision>,
    pub edges: Vec<WordRevisionEdge>,
    pub projections: WordRevisionProjections,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordRevisionKind {
    Insertion,
    Deletion,
    MoveFrom,
    MoveTo,
    MoveFromRangeStart,
    MoveFromRangeEnd,
    MoveToRangeStart,
    MoveToRangeEnd,
    CustomXmlInsertionRangeStart,
    CustomXmlInsertionRangeEnd,
    CustomXmlDeletionRangeStart,
    CustomXmlDeletionRangeEnd,
    CustomXmlMoveFromRangeStart,
    CustomXmlMoveFromRangeEnd,
    CustomXmlMoveToRangeStart,
    CustomXmlMoveToRangeEnd,
    RunProperties,
    ParagraphProperties,
    TableProperties,
    TableGridProperties,
    TableRowProperties,
    TableCellProperties,
    CellInsertion,
    CellDeletion,
    CellMerge,
    SectionProperties,
    NumberingProperties,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordRevision {
    pub node_id: String,
    pub revision_id: Option<String>,
    pub revision_kind: WordRevisionKind,
    pub author: Option<String>,
    pub date: Option<String>,
    pub story_part: String,
    pub target_xml_path: String,
    pub parent_revision_node_id: Option<String>,
    pub text: String,
    pub content: WordRichXmlNode,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordRevisionRelation {
    Contains,
    MovePair,
    RangePair,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordRevisionEdge {
    pub source_revision_node_id: String,
    pub relation: WordRevisionRelation,
    pub target_revision_node_id: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WordRevisionProjections {
    pub original: Vec<WordStoryTextProjection>,
    pub accepted: Vec<WordStoryTextProjection>,
    pub rejected: Vec<WordStoryTextProjection>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordRevisionView {
    Original,
    Accepted,
    Rejected,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordStoryTextProjection {
    pub view: WordRevisionView,
    pub story_part: String,
    pub text: String,
    pub source_revision_node_ids: Vec<String>,
    pub locator: SourceLocator,
}

/// Namespace-preserving structural retention for rich OOXML constructs.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordRichXmlNode {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<WordRichXmlNode>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordCommentAnchorKind {
    RangeStart,
    RangeEnd,
    Reference,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordCommentAnchor {
    pub anchor_kind: WordCommentAnchorKind,
    pub story_part: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordComment {
    pub id: String,
    pub durable_id: Option<String>,
    pub parent_comment_id: Option<String>,
    pub reply_ids: Vec<String>,
    pub author: Option<String>,
    pub initials: Option<String>,
    pub date: Option<String>,
    pub resolved: Option<bool>,
    pub paragraph_id: Option<String>,
    pub text: String,
    pub anchors: Vec<WordCommentAnchor>,
    pub content: WordRichXmlNode,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordContentControl {
    pub control_id: String,
    pub parent_control_id: Option<String>,
    pub story_part: String,
    pub alias: Option<String>,
    pub tag: Option<String>,
    pub control_type: String,
    pub lock: Option<String>,
    pub placeholder: Option<String>,
    pub showing_placeholder: bool,
    pub data_binding: BTreeMap<String, String>,
    pub text: String,
    pub properties: Option<WordRichXmlNode>,
    pub content: WordRichXmlNode,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordEquation {
    pub equation_id: String,
    pub story_part: String,
    pub display: bool,
    pub text: String,
    pub omml: WordRichXmlNode,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WordDrawingKind {
    Image,
    Chart,
    TextBox,
    OleObject,
    Shape,
    Group,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordObjectRelationship {
    pub relationship_id: String,
    pub relationship_type: String,
    pub target: String,
    pub resolved_part: Option<String>,
    pub external: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordDrawing {
    pub drawing_id: String,
    pub story_part: String,
    pub drawing_kind: WordDrawingKind,
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub alt_text: Option<String>,
    pub relationships: Vec<WordObjectRelationship>,
    pub content: WordRichXmlNode,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordChart {
    pub chart_id: String,
    pub relationship_id: String,
    pub source_part: String,
    pub chart_part: Option<String>,
    pub chart_types: Vec<String>,
    pub title: Option<String>,
    pub text: String,
    pub content: Option<WordRichXmlNode>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordCaption {
    pub caption_id: String,
    pub story_part: String,
    pub text: String,
    pub label: Option<String>,
    pub target_object_id: Option<String>,
    pub inference_rule: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordTextBox {
    pub text_box_id: String,
    pub drawing_id: Option<String>,
    pub story_part: String,
    pub text: String,
    pub content: WordRichXmlNode,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WordEmbeddedObject {
    pub object_id: String,
    pub story_part: String,
    pub relationship_id: Option<String>,
    pub relationship: Option<WordObjectRelationship>,
    pub program_id: Option<String>,
    pub object_type: Option<String>,
    pub draw_aspect: Option<String>,
    pub shape_id: Option<String>,
    pub child_artifact_part: Option<String>,
    pub content: WordRichXmlNode,
    pub locator: SourceLocator,
}
