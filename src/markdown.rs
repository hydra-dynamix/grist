//! CommonMark/GFM and extension-preserving Markdown parser.
//!
//! The typed payload is authoritative. Graph, segment, and render output are
//! deterministic projections of this lossless source model.

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, Envelope, FormatIdentity, LineIndex, OperationKind,
    OperationStatus, ParserInfo, SchemaVersion, SourceInfo, SourceLocator, SourceRange,
    options_digest, sha256_hex,
};
use crate::decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeReport, DecodedByteRange, DecodedText,
    RawByteRange, TextEncoding, decode_text,
};
use pulldown_cmark::{
    Alignment, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::Read;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownDocument {
    pub schema_version: String,
    pub raw_bytes: Vec<u8>,
    pub raw_range: RawByteRange,
    pub decoded_text: String,
    pub decoded_range: SourceRange,
    pub locator: SourceLocator,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub nodes: Vec<MarkdownNode>,
    pub frontmatter: Option<Frontmatter>,
    /// Resolved source dialect. CommonMark is omitted to preserve the Markdown v2 wire shape.
    #[serde(default, skip_serializing_if = "MarkdownDialect::is_common_mark")]
    pub dialect: MarkdownDialect,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownNode {
    pub id: String,
    pub kind: MarkdownNodeKind,
    pub range: Option<SourceRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_range: Option<RawByteRange>,
    /// Exact decoded source slice, including syntax markers.
    #[serde(default)]
    pub raw: String,
    pub text: Option<String>,
    pub level: Option<u8>,
    pub language: Option<String>,
    pub info: Option<String>,
    pub destination: Option<String>,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(default)]
    pub classes: Vec<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, Option<String>>,
    pub table: Option<MarkdownTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<ExecutableBlockMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation: Option<MarkdownCitation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub figure: Option<MarkdownFigure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_reference: Option<MarkdownLocalReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_output: Option<MarkdownStoredOutput>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownNodeKind {
    Heading,
    Paragraph,
    BlockQuote,
    CodeFence,
    IndentedCode,
    List,
    ListItem,
    TaskListMarker,
    Link,
    Image,
    Table,
    TableRow,
    TableCell,
    FootnoteDefinition,
    FootnoteReference,
    DefinitionList,
    DefinitionTerm,
    DefinitionDescription,
    Emphasis,
    Strong,
    Strikethrough,
    InlineCode,
    InlineMath,
    DisplayMath,
    HtmlBlock,
    HtmlInline,
    ThematicBreak,
    SoftBreak,
    HardBreak,
    DirectiveBlock,
    ExtensionInline,
    Citation,
    Include,
    Figure,
    StoredOutput,
    RawBlock,
    RawInline,
    Text,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownDialect {
    CommonMark,
    RMarkdown,
    Quarto,
}

impl MarkdownDialect {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommonMark => "common_mark",
            Self::RMarkdown => "r_markdown",
            Self::Quarto => "quarto",
        }
    }

    fn is_common_mark(&self) -> bool {
        *self == Self::CommonMark
    }
}

impl Default for MarkdownDialect {
    fn default() -> Self {
        Self::CommonMark
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutableBlockMetadata {
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub options: BTreeMap<String, Value>,
    /// Always false. The parser has no execution path.
    pub executed: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkdownCitation {
    pub keys: Vec<String>,
    pub textual: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkdownFigure {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkdownStoredOutput {
    pub output_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_label: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalReferenceKind {
    Include,
    Bibliography,
    CitationStyle,
    Figure,
    Resource,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalReferenceStatus {
    Resolved,
    ReferenceOnly,
    RemoteDisabled,
    OutsideProjectRoot,
    Missing,
    NotFile,
    BudgetExceeded,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkdownLocalReference {
    pub kind: LocalReferenceKind,
    pub target: String,
    pub status: LocalReferenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<u8>>,
}
impl MarkdownNodeKind {
    fn accumulates_text(&self) -> bool {
        !matches!(
            self,
            Self::List
                | Self::Table
                | Self::TableRow
                | Self::DefinitionList
                | Self::ThematicBreak
                | Self::SoftBreak
                | Self::HardBreak
                | Self::TaskListMarker
        )
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownTable {
    pub rows: Vec<Vec<String>>,
    pub alignments: Vec<MarkdownTableAlignment>,
    pub row_details: Vec<MarkdownTableRow>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownTableAlignment {
    None,
    Left,
    Center,
    Right,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownTableRow {
    pub range: Option<SourceRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
    pub header: bool,
    pub cells: Vec<MarkdownTableCell>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownTableCell {
    pub range: Option<SourceRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
    pub text: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrontmatterKind {
    Yaml,
    Toml,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Frontmatter {
    pub kind: FrontmatterKind,
    pub delimiter: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub raw_range: RawByteRange,
    /// Body only, retained exactly for v1 source compatibility.
    pub raw: String,
    pub value: Option<Value>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MarkdownOptions {
    pub encoding: Option<String>,
    pub gfm: bool,
    pub tables: bool,
    pub footnotes: bool,
    pub task_lists: bool,
    pub strikethrough: bool,
    pub math: bool,
    pub heading_attributes: bool,
    pub definition_lists: bool,
    pub frontmatter: bool,
    pub retain_extensions: bool,
    /// Explicit dialect, or filename-based detection when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<MarkdownDialect>,
    /// Explicit filesystem boundary. No root means local references stay inert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<PathBuf>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub resolve_local_references: bool,
    #[serde(
        default = "default_max_reference_bytes",
        skip_serializing_if = "is_default_max_reference_bytes"
    )]
    pub max_reference_bytes: u64,
    /// Aggregate retained reference bytes across the complete document.
    #[serde(
        default = "default_max_total_reference_bytes",
        skip_serializing_if = "is_default_max_total_reference_bytes"
    )]
    pub max_total_reference_bytes: u64,
}

const DEFAULT_MAX_REFERENCE_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_REFERENCE_BYTES: u64 = 32 * 1024 * 1024;

const fn default_true() -> bool {
    true
}

const fn is_true(value: &bool) -> bool {
    *value
}

const fn default_max_reference_bytes() -> u64 {
    DEFAULT_MAX_REFERENCE_BYTES
}

const fn is_default_max_reference_bytes(value: &u64) -> bool {
    *value == DEFAULT_MAX_REFERENCE_BYTES
}

const fn default_max_total_reference_bytes() -> u64 {
    DEFAULT_MAX_TOTAL_REFERENCE_BYTES
}

const fn is_default_max_total_reference_bytes(value: &u64) -> bool {
    *value == DEFAULT_MAX_TOTAL_REFERENCE_BYTES
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self {
            encoding: None,
            gfm: true,
            tables: true,
            footnotes: true,
            task_lists: true,
            strikethrough: true,
            math: true,
            heading_attributes: true,
            definition_lists: true,
            frontmatter: true,
            retain_extensions: true,
            dialect: None,
            project_root: None,
            resolve_local_references: true,
            max_reference_bytes: DEFAULT_MAX_REFERENCE_BYTES,
            max_total_reference_bytes: DEFAULT_MAX_TOTAL_REFERENCE_BYTES,
        }
    }
}

impl crate::core::FormatOptions for MarkdownOptions {
    const FORMAT: &'static str = "markdown";
}

pub type MarkdownEnvelope = Envelope<MarkdownDocument>;

pub fn parse_markdown(text: &str, source: SourceInfo) -> MarkdownEnvelope {
    parse_markdown_with_options(text, source, &MarkdownOptions::default())
}

pub fn parse_markdown_with_options(
    text: &str,
    source: SourceInfo,
    options: &MarkdownOptions,
) -> MarkdownEnvelope {
    parse_markdown_bytes(text.as_bytes(), source, options)
}

pub fn parse_markdown_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &MarkdownOptions,
) -> MarkdownEnvelope {
    let mut decode_options =
        DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("markdown"));
    decode_options.context = DecodeContext::PlainText;
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    match decode_text(bytes, &decode_options) {
        Ok(decoded) => envelope_from_decoded(&decoded, source, options),
        Err(error) => failed_decode_envelope(bytes, source, options, error),
    }
}

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new("grist.markdown")
        .with_implementation("pulldown-cmark", "0.12.2")
        .with_specification_version("CommonMark 0.31.2 + GFM + Grist extensions")
        .with_feature("markdown")
}

fn envelope_from_decoded(
    decoded: &DecodedText,
    source: SourceInfo,
    options: &MarkdownOptions,
) -> MarkdownEnvelope {
    let (payload, mut diagnostics) = document_from_decoded(decoded, &source, options);
    let digest = options_digest(options).expect("Markdown options always serialize");
    let partial = decoded.report.makes_operation_partial()
        || diagnostics.iter().any(|diagnostic| diagnostic.partial);
    let mut decode_diagnostics = decoded.report.diagnostics.clone();
    decode_diagnostics.append(&mut diagnostics);
    let mut envelope = if partial {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Markdown,
            source,
            parser_info(),
            digest,
            SchemaVersion::MARKDOWN_V2,
            Some(payload),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Markdown,
            source,
            parser_info(),
            digest,
            SchemaVersion::MARKDOWN_V2,
            payload,
        )
    };
    envelope.diagnostics = decode_diagnostics;
    envelope
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    let identity = ContentIdentity::for_raw_bytes(decoded.raw_bytes())
        .with_decoded(
            &decoded.text,
            decoded.report.encoding.label(),
            decoded.report.is_lossy(),
        )
        .with_format(FormatIdentity::new("markdown", Some("text/markdown")));
    envelope
        .with_identity(identity)
        .with_canonical_payload_identity()
        .expect("Markdown payload canonicalization is infallible")
}

fn failed_decode_envelope(
    bytes: &[u8],
    source: SourceInfo,
    options: &MarkdownOptions,
    error: DecodeError,
) -> MarkdownEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Markdown,
        OperationStatus::Failed,
        source,
        parser_info(),
        options_digest(options).expect("Markdown options always serialize"),
        SchemaVersion::MARKDOWN_V2,
    )
    .expect("failed Markdown decode has valid envelope status")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes)
            .with_format(FormatIdentity::new("markdown", Some("text/markdown"))),
    )
    .with_diagnostics(vec![error.diagnostic().with_parser("grist.markdown")])
}

pub(crate) fn document_from_decoded(
    decoded: &DecodedText,
    source: &SourceInfo,
    options: &MarkdownOptions,
) -> (MarkdownDocument, Vec<Diagnostic>) {
    let text = decoded.text.as_str();
    let line_index = LineIndex::new(text);
    let dialect = resolve_dialect(source, options);
    let (frontmatter, mut diagnostics, body_start) = if options.frontmatter {
        parse_frontmatter(decoded, &line_index)
    } else {
        (None, Vec::new(), 0)
    };
    diagnostics.extend(scan_unclosed_fences(text, body_start, &line_index));

    let mut parser_options = Options::empty();
    if options.tables {
        parser_options.insert(Options::ENABLE_TABLES);
    }
    if options.footnotes {
        parser_options.insert(Options::ENABLE_FOOTNOTES);
    }
    if options.strikethrough {
        parser_options.insert(Options::ENABLE_STRIKETHROUGH);
    }
    if options.task_lists {
        parser_options.insert(Options::ENABLE_TASKLISTS);
    }
    if options.gfm {
        parser_options.insert(Options::ENABLE_GFM);
    }
    if options.math {
        parser_options.insert(Options::ENABLE_MATH);
    }
    if options.heading_attributes {
        parser_options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    }
    if options.definition_lists {
        parser_options.insert(Options::ENABLE_DEFINITION_LIST);
    }

    let mut builder = AstBuilder::new(decoded, &line_index);
    let parser = Parser::new_ext(&text[body_start..], parser_options).into_offset_iter();
    for (event, range) in parser {
        let absolute = (range.start + body_start)..(range.end + body_start);
        handle_event(&mut builder, event, absolute);
    }
    if options.retain_extensions {
        let (mut extensions, mut extension_diagnostics) =
            scan_extensions(decoded, &line_index, body_start);
        for (ordinal, node) in extensions.iter_mut().enumerate() {
            node.id = format!("md-extension-{ordinal:06}");
        }
        builder.nodes.extend(extensions);
        diagnostics.append(&mut extension_diagnostics);
    }
    if dialect != MarkdownDialect::CommonMark {
        let mut dialect_diagnostics = annotate_notebook_constructs(
            &mut builder.nodes,
            decoded,
            &line_index,
            source,
            options,
            frontmatter.as_ref(),
        );
        diagnostics.append(&mut dialect_diagnostics);
    }
    builder.finish();

    let decoded_range = SourceRange::new(0, text.len(), &line_index);
    let locator = SourceLocator::exact(decoded_range.clone())
        .expect("the whole decoded Markdown range is exact");
    (
        MarkdownDocument {
            schema_version: SchemaVersion::MARKDOWN_V2.to_string(),
            raw_bytes: decoded.raw_bytes().to_vec(),
            raw_range: RawByteRange {
                start: 0,
                end: decoded.raw_bytes().len() as u64,
            },
            decoded_text: text.to_string(),
            decoded_range,
            locator,
            encoding: decoded.report.encoding.clone(),
            decoding: decoded.report.clone(),
            nodes: builder.nodes,
            frontmatter,
            dialect,
        },
        diagnostics,
    )
}

struct AstBuilder<'a> {
    decoded: &'a DecodedText,
    line_index: &'a LineIndex,
    nodes: Vec<MarkdownNode>,
    stack: Vec<usize>,
    table_alignments: HashMap<usize, Vec<MarkdownTableAlignment>>,
}

impl<'a> AstBuilder<'a> {
    fn new(decoded: &'a DecodedText, line_index: &'a LineIndex) -> Self {
        Self {
            decoded,
            line_index,
            nodes: Vec::new(),
            stack: Vec::new(),
            table_alignments: HashMap::new(),
        }
    }

    fn start(&mut self, kind: MarkdownNodeKind, range: Range<usize>) -> usize {
        let parent_id = self.stack.last().map(|index| self.nodes[*index].id.clone());
        let node = source_node(
            self.nodes.len(),
            kind,
            range,
            self.decoded,
            self.line_index,
            parent_id,
        );
        let index = self.nodes.len();
        self.nodes.push(node);
        self.stack.push(index);
        index
    }

    fn leaf(&mut self, kind: MarkdownNodeKind, range: Range<usize>, text: Option<String>) -> usize {
        let parent_id = self.stack.last().map(|index| self.nodes[*index].id.clone());
        let mut node = source_node(
            self.nodes.len(),
            kind,
            range,
            self.decoded,
            self.line_index,
            parent_id,
        );
        node.text = text;
        let index = self.nodes.len();
        self.nodes.push(node);
        index
    }

    fn append_text(&mut self, value: &str) {
        for index in &self.stack {
            let node = &mut self.nodes[*index];
            if node.kind.accumulates_text() {
                node.text.get_or_insert_with(String::new).push_str(value);
            }
        }
    }

    fn finish(&mut self) {
        self.stack.clear();
        self.annotate_lists();
        self.build_tables();
        self.reindex_source_order();
    }

    fn annotate_lists(&mut self) {
        let by_id = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let mut positions: HashMap<String, u64> = HashMap::new();
        for index in 0..self.nodes.len() {
            if self.nodes[index].kind != MarkdownNodeKind::ListItem {
                continue;
            }
            let Some(parent_id) = self.nodes[index].parent_id.clone() else {
                continue;
            };
            let Some(parent_index) = by_id.get(&parent_id).copied() else {
                continue;
            };
            if self.nodes[parent_index].kind != MarkdownNodeKind::List {
                continue;
            }
            let position = positions.entry(parent_id).or_default();
            let ordered = self.nodes[parent_index].ordered.unwrap_or(false);
            let start = self.nodes[parent_index].start_number.unwrap_or(1);
            self.nodes[index].ordered = Some(ordered);
            self.nodes[index].start_number = ordered.then_some(start);
            self.nodes[index].item_number = ordered.then_some(start.saturating_add(*position));
            *position = position.saturating_add(1);
        }
    }

    fn build_tables(&mut self) {
        for table_index in 0..self.nodes.len() {
            if self.nodes[table_index].kind != MarkdownNodeKind::Table {
                continue;
            }
            let table_id = self.nodes[table_index].id.clone();
            let rows = self
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == MarkdownNodeKind::TableRow
                        && node.parent_id.as_deref() == Some(table_id.as_str())
                })
                .map(|row| {
                    let cells = self
                        .nodes
                        .iter()
                        .filter(|node| {
                            node.kind == MarkdownNodeKind::TableCell
                                && node.parent_id.as_deref() == Some(row.id.as_str())
                        })
                        .map(|cell| MarkdownTableCell {
                            range: cell.range.clone(),
                            locator: cell.locator.clone(),
                            text: cell.text.clone().unwrap_or_default(),
                        })
                        .collect::<Vec<_>>();
                    MarkdownTableRow {
                        range: row.range.clone(),
                        locator: row.locator.clone(),
                        header: row.info.as_deref() == Some("header"),
                        cells,
                    }
                })
                .collect::<Vec<_>>();
            let plain = rows
                .iter()
                .map(|row| row.cells.iter().map(|cell| cell.text.clone()).collect())
                .collect();
            self.nodes[table_index].table = Some(MarkdownTable {
                rows: plain,
                alignments: self
                    .table_alignments
                    .get(&table_index)
                    .cloned()
                    .unwrap_or_default(),
                row_details: rows,
            });
        }
    }

    fn reindex_source_order(&mut self) {
        self.nodes.sort_by_key(|node| {
            let range = node.range.as_ref();
            (
                range.map(|value| value.byte_start).unwrap_or(usize::MAX),
                std::cmp::Reverse(
                    range
                        .map(|value| value.byte_end.saturating_sub(value.byte_start))
                        .unwrap_or(0),
                ),
                node.id.clone(),
            )
        });
        let replacements = self
            .nodes
            .iter()
            .enumerate()
            .map(|(ordinal, node)| (node.id.clone(), format!("md-node-{ordinal:06}")))
            .collect::<HashMap<_, _>>();
        for node in &mut self.nodes {
            node.id = replacements[&node.id].clone();
            node.parent_id = node
                .parent_id
                .as_ref()
                .and_then(|value| replacements.get(value).cloned());
            node.children.clear();
        }
        let index_by_id = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<HashMap<_, _>>();
        for index in 0..self.nodes.len() {
            let child_id = self.nodes[index].id.clone();
            if let Some(parent_id) = self.nodes[index].parent_id.clone()
                && let Some(parent_index) = index_by_id.get(&parent_id).copied()
            {
                self.nodes[parent_index].children.push(child_id);
            }
        }
    }
}

fn handle_event(builder: &mut AstBuilder<'_>, event: Event<'_>, range: Range<usize>) {
    match event {
        Event::Start(tag) => handle_start(builder, tag, range),
        Event::End(_) => {
            builder.stack.pop();
        }
        Event::Text(value) => {
            builder.append_text(&value);
            builder.leaf(MarkdownNodeKind::Text, range, Some(value.to_string()));
        }
        Event::Code(value) => {
            builder.append_text(&value);
            builder.leaf(MarkdownNodeKind::InlineCode, range, Some(value.to_string()));
        }
        Event::InlineMath(value) => {
            builder.append_text(&value);
            builder.leaf(MarkdownNodeKind::InlineMath, range, Some(value.to_string()));
        }
        Event::DisplayMath(value) => {
            builder.append_text(&value);
            builder.leaf(
                MarkdownNodeKind::DisplayMath,
                range,
                Some(value.to_string()),
            );
        }
        Event::Html(value) => {
            builder.append_text(&value);
            if !builder
                .stack
                .last()
                .is_some_and(|index| builder.nodes[*index].kind == MarkdownNodeKind::HtmlBlock)
            {
                builder.leaf(MarkdownNodeKind::HtmlBlock, range, Some(value.to_string()));
            }
        }
        Event::InlineHtml(value) => {
            builder.append_text(&value);
            builder.leaf(MarkdownNodeKind::HtmlInline, range, Some(value.to_string()));
        }
        Event::FootnoteReference(label) => {
            builder.append_text(&format!("[^{label}]"));
            let index = builder.leaf(
                MarkdownNodeKind::FootnoteReference,
                range,
                Some(label.to_string()),
            );
            builder.nodes[index].label = Some(label.to_string());
        }
        Event::SoftBreak => {
            builder.append_text("\n");
            builder.leaf(MarkdownNodeKind::SoftBreak, range, Some("\n".into()));
        }
        Event::HardBreak => {
            builder.append_text("\n");
            builder.leaf(MarkdownNodeKind::HardBreak, range, Some("  \n".into()));
        }
        Event::Rule => {
            builder.leaf(MarkdownNodeKind::ThematicBreak, range, None);
        }
        Event::TaskListMarker(checked) => {
            let index = builder.leaf(MarkdownNodeKind::TaskListMarker, range, None);
            builder.nodes[index].checked = Some(checked);
            if let Some(item) = builder
                .stack
                .iter()
                .rev()
                .find(|index| builder.nodes[**index].kind == MarkdownNodeKind::ListItem)
            {
                builder.nodes[*item].checked = Some(checked);
            }
        }
    }
}

fn handle_start(builder: &mut AstBuilder<'_>, tag: Tag<'_>, range: Range<usize>) {
    match tag {
        Tag::Paragraph => {
            builder.start(MarkdownNodeKind::Paragraph, range);
        }
        Tag::Heading {
            level,
            id,
            classes,
            attrs,
        } => {
            let index = builder.start(MarkdownNodeKind::Heading, range);
            let node = &mut builder.nodes[index];
            node.level = Some(heading_level(level));
            node.label = id.map(|value| value.to_string());
            node.classes = classes.into_iter().map(|value| value.to_string()).collect();
            node.attributes = attrs
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.map(|value| value.to_string())))
                .collect();
        }
        Tag::BlockQuote(kind) => {
            let index = builder.start(MarkdownNodeKind::BlockQuote, range);
            builder.nodes[index].info = kind.map(|value| format!("{value:?}"));
        }
        Tag::CodeBlock(kind) => match kind {
            CodeBlockKind::Indented => {
                builder.start(MarkdownNodeKind::IndentedCode, range);
            }
            CodeBlockKind::Fenced(info) => {
                let info = info.to_string();
                if let Some(name) = directive_name(&info) {
                    let index = builder.start(MarkdownNodeKind::DirectiveBlock, range);
                    builder.nodes[index].info = Some(info);
                    builder.nodes[index].label = Some(name);
                } else {
                    let index = builder.start(MarkdownNodeKind::CodeFence, range);
                    builder.nodes[index].language = info
                        .split_whitespace()
                        .next()
                        .filter(|value| !value.is_empty())
                        .map(str::to_string);
                    builder.nodes[index].info = Some(info);
                }
            }
        },
        Tag::HtmlBlock => {
            builder.start(MarkdownNodeKind::HtmlBlock, range);
        }
        Tag::List(start) => {
            let index = builder.start(MarkdownNodeKind::List, range);
            builder.nodes[index].ordered = Some(start.is_some());
            builder.nodes[index].start_number = start;
        }
        Tag::Item => {
            builder.start(MarkdownNodeKind::ListItem, range);
        }
        Tag::FootnoteDefinition(label) => {
            let index = builder.start(MarkdownNodeKind::FootnoteDefinition, range);
            builder.nodes[index].label = Some(label.to_string());
        }
        Tag::DefinitionList => {
            builder.start(MarkdownNodeKind::DefinitionList, range);
        }
        Tag::DefinitionListTitle => {
            builder.start(MarkdownNodeKind::DefinitionTerm, range);
        }
        Tag::DefinitionListDefinition => {
            builder.start(MarkdownNodeKind::DefinitionDescription, range);
        }
        Tag::Table(alignments) => {
            let index = builder.start(MarkdownNodeKind::Table, range);
            builder.table_alignments.insert(
                index,
                alignments.into_iter().map(markdown_alignment).collect(),
            );
        }
        Tag::TableHead => {
            let index = builder.start(MarkdownNodeKind::TableRow, range);
            builder.nodes[index].info = Some("header".into());
        }
        Tag::TableRow => {
            builder.start(MarkdownNodeKind::TableRow, range);
        }
        Tag::TableCell => {
            builder.start(MarkdownNodeKind::TableCell, range);
        }
        Tag::Emphasis => {
            builder.start(MarkdownNodeKind::Emphasis, range);
        }
        Tag::Strong => {
            builder.start(MarkdownNodeKind::Strong, range);
        }
        Tag::Strikethrough => {
            builder.start(MarkdownNodeKind::Strikethrough, range);
        }
        Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        } => {
            let index = builder.start(MarkdownNodeKind::Link, range);
            let node = &mut builder.nodes[index];
            node.destination = Some(dest_url.to_string());
            node.title = nonempty(&title);
            node.label = nonempty(&id);
            node.reference_kind = Some(link_type_name(link_type).into());
        }
        Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        } => {
            let index = builder.start(MarkdownNodeKind::Image, range);
            let node = &mut builder.nodes[index];
            node.destination = Some(dest_url.to_string());
            node.title = nonempty(&title);
            node.label = nonempty(&id);
            node.reference_kind = Some(link_type_name(link_type).into());
        }
        Tag::MetadataBlock(kind) => {
            let index = builder.start(MarkdownNodeKind::RawBlock, range);
            builder.nodes[index].info = Some(format!("{kind:?}"));
        }
    }
}

fn source_node(
    ordinal: usize,
    kind: MarkdownNodeKind,
    range: Range<usize>,
    decoded: &DecodedText,
    line_index: &LineIndex,
    parent_id: Option<String>,
) -> MarkdownNode {
    let source_range = SourceRange::new(range.start, range.end, line_index);
    let locator = SourceLocator::exact(source_range.clone()).ok();
    let raw_range = decoded.raw_range_for_decoded(DecodedByteRange {
        start: range.start as u64,
        end: range.end as u64,
    });
    MarkdownNode {
        id: format!("md-node-{ordinal:06}"),
        kind,
        range: Some(source_range),
        locator,
        raw_range,
        raw: decoded.text[range].to_string(),
        text: None,
        level: None,
        language: None,
        info: None,
        destination: None,
        title: None,
        label: None,
        reference_kind: None,
        parent_id,
        children: Vec::new(),
        ordered: None,
        start_number: None,
        item_number: None,
        checked: None,
        classes: Vec::new(),
        attributes: BTreeMap::new(),
        table: None,
        executable: None,
        citation: None,
        figure: None,
        local_reference: None,
        stored_output: None,
    }
}

fn parse_frontmatter(
    decoded: &DecodedText,
    line_index: &LineIndex,
) -> (Option<Frontmatter>, Vec<Diagnostic>, usize) {
    let text = decoded.text.as_str();
    let (kind, marker, opening_len) = if text.starts_with("---\r\n") {
        (FrontmatterKind::Yaml, "---", 5)
    } else if text.starts_with("---\n") {
        (FrontmatterKind::Yaml, "---", 4)
    } else if text.starts_with("+++\r\n") {
        (FrontmatterKind::Toml, "+++", 5)
    } else if text.starts_with("+++\n") {
        (FrontmatterKind::Toml, "+++", 4)
    } else {
        return (None, Vec::new(), 0);
    };
    let mut offset = opening_len;
    let mut closing_end = None;
    let mut content_end = text.len();
    while offset < text.len() {
        let line_end = next_line_end(text, offset);
        let line = text[offset..line_end].trim_end_matches(['\r', '\n']);
        if line == marker || (kind == FrontmatterKind::Yaml && line == "...") {
            content_end = offset;
            closing_end = Some(line_end);
            break;
        }
        offset = line_end;
    }
    let end = closing_end.unwrap_or(text.len());
    let raw = text[opening_len..content_end].to_string();
    let mut diagnostics = Vec::new();
    let parsed = match kind {
        FrontmatterKind::Yaml => serde_yaml::from_str::<serde_yaml::Value>(&raw)
            .map_err(|error| error.to_string())
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
        FrontmatterKind::Toml => toml::from_str::<toml::Value>(&raw)
            .map_err(|error| error.to_string())
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
    };
    let value = match parsed {
        Ok(value) => Some(value),
        Err(message) => {
            diagnostics.push(
                Diagnostic::error(
                    "grist.markdown.frontmatter",
                    "frontmatter.parse",
                    format!("{kind:?} frontmatter parse failed: {message}"),
                )
                .with_range(SourceRange::new(opening_len, content_end, line_index))
                .partial(),
            );
            None
        }
    };
    if closing_end.is_none() {
        diagnostics.push(
            Diagnostic::warning(
                "grist.markdown.frontmatter",
                "frontmatter.unclosed",
                format!("{marker} frontmatter has no closing delimiter"),
            )
            .with_range(SourceRange::new(0, opening_len, line_index))
            .partial(),
        );
    }
    let range = SourceRange::new(0, end, line_index);
    let locator = SourceLocator::exact(range.clone()).expect("frontmatter range is exact");
    let raw_range = decoded
        .raw_range_for_decoded(DecodedByteRange {
            start: 0,
            end: end as u64,
        })
        .expect("frontmatter decoded range maps to input bytes");
    (
        Some(Frontmatter {
            kind,
            delimiter: marker.into(),
            range,
            locator,
            raw_range,
            raw,
            value,
        }),
        diagnostics,
        end,
    )
}

fn resolve_dialect(source: &SourceInfo, options: &MarkdownOptions) -> MarkdownDialect {
    if let Some(dialect) = options.dialect {
        return dialect;
    }
    let name = source
        .path
        .as_deref()
        .unwrap_or(source.display_name.as_str())
        .to_ascii_lowercase();
    if name.ends_with(".rmd") {
        MarkdownDialect::RMarkdown
    } else if name.ends_with(".qmd") {
        MarkdownDialect::Quarto
    } else {
        MarkdownDialect::CommonMark
    }
}

fn annotate_notebook_constructs(
    nodes: &mut Vec<MarkdownNode>,
    decoded: &DecodedText,
    line_index: &LineIndex,
    source: &SourceInfo,
    options: &MarkdownOptions,
    frontmatter: Option<&Frontmatter>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut resolver = LocalReferenceResolver::new(source, options);
    let mut added = Vec::new();

    for node in nodes.iter_mut() {
        if matches!(
            node.kind,
            MarkdownNodeKind::CodeFence | MarkdownNodeKind::DirectiveBlock
        ) {
            let info = node.info.clone().unwrap_or_default();
            if info.contains("cell-output") {
                node.kind = MarkdownNodeKind::StoredOutput;
                node.stored_output = Some(MarkdownStoredOutput {
                    output_kind: output_kind(&info),
                    cell_label: None,
                });
                continue;
            }
            if let Some((metadata, malformed)) = parse_executable_metadata(&info, &node.raw) {
                node.kind = MarkdownNodeKind::CodeFence;
                for message in malformed {
                    diagnostics.push(node_diagnostic(node, "executable.metadata", message));
                }
                node.language = Some(metadata.engine.clone());
                node.label.clone_from(&metadata.label);
                node.executable = Some(metadata.clone());
                if metadata
                    .label
                    .as_deref()
                    .is_some_and(|label| label.starts_with("fig-"))
                    || metadata.options.contains_key("fig-cap")
                    || metadata.options.contains_key("fig.cap")
                {
                    let caption = metadata
                        .options
                        .get("fig-cap")
                        .or_else(|| metadata.options.get("fig.cap"))
                        .and_then(value_text);
                    node.figure = Some(MarkdownFigure {
                        identifier: metadata.label.clone(),
                        caption,
                        source: None,
                    });
                    added.push(overlay_node(
                        node,
                        MarkdownNodeKind::Figure,
                        node.figure.clone(),
                        None,
                    ));
                }
                for key in ["child", "dependson"] {
                    if let Some(target) = metadata.options.get(key).and_then(value_text) {
                        let (reference, diagnostic) =
                            resolver.resolve(LocalReferenceKind::Include, target, node, options);
                        if let Some(diagnostic) = diagnostic {
                            diagnostics.push(diagnostic);
                        }
                        added.push(reference_node(node, reference));
                    }
                }
            }
        }

        if node.kind == MarkdownNodeKind::Image {
            let target = node.destination.clone().unwrap_or_default();
            let figure = MarkdownFigure {
                identifier: node.label.clone(),
                caption: node.text.clone(),
                source: Some(target.clone()),
            };
            let (reference, diagnostic) =
                resolver.resolve(LocalReferenceKind::Figure, target, node, options);
            if let Some(diagnostic) = diagnostic {
                diagnostics.push(diagnostic);
            }
            node.figure = Some(figure.clone());
            node.local_reference = Some(reference);
            added.push(overlay_node(
                node,
                MarkdownNodeKind::Figure,
                Some(figure),
                None,
            ));
        }

        if node.kind == MarkdownNodeKind::Link {
            let target = node.destination.clone().unwrap_or_default();
            if is_local_reference_candidate(&target) {
                let (reference, diagnostic) =
                    resolver.resolve(LocalReferenceKind::Resource, target, node, options);
                if let Some(diagnostic) = diagnostic {
                    diagnostics.push(diagnostic);
                }
                node.local_reference = Some(reference);
            }
        }

        if node.kind == MarkdownNodeKind::DirectiveBlock
            && node
                .info
                .as_deref()
                .is_some_and(|info| info.contains("cell-output"))
        {
            node.kind = MarkdownNodeKind::StoredOutput;
            node.stored_output = Some(MarkdownStoredOutput {
                output_kind: output_kind(node.info.as_deref().unwrap_or("output")),
                cell_label: None,
            });
        }

        if node.kind == MarkdownNodeKind::ExtensionInline
            && node.raw.trim_start().starts_with("{{< include ")
        {
            if let Some(target) = shortcode_include_target(&node.raw) {
                let (reference, diagnostic) =
                    resolver.resolve(LocalReferenceKind::Include, target, node, options);
                if let Some(diagnostic) = diagnostic {
                    diagnostics.push(diagnostic);
                }
                node.kind = MarkdownNodeKind::Include;
                node.destination = Some(reference.target.clone());
                node.local_reference = Some(reference);
            }
        }
    }

    for (range, citation) in citation_ranges(&decoded.text, nodes, frontmatter) {
        let mut node = source_node(
            nodes.len() + added.len(),
            MarkdownNodeKind::Citation,
            range,
            decoded,
            line_index,
            None,
        );
        node.label = citation.keys.first().cloned();
        node.text = Some(citation.keys.join("; "));
        node.citation = Some(citation);
        added.push(node);
    }

    if let Some(frontmatter) = frontmatter
        && let Some(value) = &frontmatter.value
    {
        for (kind, target) in frontmatter_references(value) {
            let mut anchor = source_node(
                nodes.len() + added.len(),
                MarkdownNodeKind::Include,
                frontmatter.range.byte_start..frontmatter.range.byte_end,
                decoded,
                line_index,
                None,
            );
            let (reference, diagnostic) = resolver.resolve(kind, target, &anchor, options);
            if let Some(diagnostic) = diagnostic {
                diagnostics.push(diagnostic);
            }
            anchor.destination = Some(reference.target.clone());
            anchor.local_reference = Some(reference);
            added.push(anchor);
        }
    }

    nodes.extend(added);
    diagnostics
}

fn parse_executable_metadata(
    info: &str,
    raw: &str,
) -> Option<(ExecutableBlockMetadata, Vec<String>)> {
    let header = info.trim();
    if !(header.starts_with('{') && header.ends_with('}')) {
        return None;
    }
    let header = header[1..header.len() - 1].trim();
    if header.starts_with('.') || header.is_empty() {
        return None;
    }
    let first_end = header
        .find(|value: char| value.is_whitespace() || value == ',')
        .unwrap_or(header.len());
    let engine = header[..first_end].trim().to_string();
    if engine.is_empty() {
        return None;
    }
    let mut label = None;
    let mut options = BTreeMap::new();
    let mut malformed = Vec::new();
    let (tokens, balanced) = split_executable_header_tokens(&header[first_end..]);
    if !balanced {
        malformed.push("unbalanced quotes or delimiters in executable header metadata".to_string());
    }
    for token in tokens
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if let Some((key, value)) = token.split_once('=') {
            options.insert(
                key.trim().replace('.', "-"),
                parse_metadata_value(value.trim()),
            );
        } else if label.is_none() && !token.contains(char::is_whitespace) {
            label = Some(token.to_string());
        } else {
            malformed.push(format!("unrecognized executable header metadata `{token}`"));
        }
    }
    for line in raw.lines() {
        let Some(metadata) = line.trim_start().strip_prefix("#|") else {
            continue;
        };
        let Some((key, value)) = metadata.split_once(':') else {
            malformed.push(format!("malformed executable option `{}`", metadata.trim()));
            continue;
        };
        let key = key.trim().to_string();
        if key.is_empty() {
            malformed.push("executable option has an empty key".to_string());
            continue;
        }
        let value = match parse_metadata_value_checked(value.trim()) {
            Ok(value) => value,
            Err(error) => {
                malformed.push(format!("malformed executable option `{key}`: {error}"));
                Value::String(value.trim().to_string())
            }
        };
        if key == "label" {
            label = value_text(&value);
        }
        options.insert(key, value);
    }
    Some((
        ExecutableBlockMetadata {
            engine,
            label,
            options,
            executed: false,
        },
        malformed,
    ))
}

fn parse_metadata_value(value: &str) -> Value {
    serde_yaml::from_str::<serde_yaml::Value>(value)
        .ok()
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or_else(|| Value::String(value.trim_matches(['\'', '"']).to_string()))
}

fn parse_metadata_value_checked(value: &str) -> Result<Value, String> {
    serde_yaml::from_str::<serde_yaml::Value>(value)
        .map_err(|error| error.to_string())
        .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
}

fn split_executable_header_tokens(input: &str) -> (Vec<String>, bool) {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut delimiters = Vec::new();
    for character in input.chars() {
        if let Some(active_quote) = quote {
            current.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                current.push(character);
            }
            '(' | '[' | '{' => {
                delimiters.push(character);
                current.push(character);
            }
            ')' | ']' | '}' => {
                let expected = match character {
                    ')' => '(',
                    ']' => '[',
                    '}' => '{',
                    _ => unreachable!(),
                };
                if delimiters.last().copied() == Some(expected) {
                    delimiters.pop();
                } else {
                    delimiters.push(character);
                }
                current.push(character);
            }
            ',' if delimiters.is_empty() => {
                tokens.push(std::mem::take(&mut current));
            }
            _ => current.push(character),
        }
    }
    tokens.push(current);
    (tokens, quote.is_none() && delimiters.is_empty())
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn output_kind(info: &str) -> String {
    info.split(|value: char| value.is_whitespace() || matches!(value, '{' | '}' | '.'))
        .find(|value| value.starts_with("cell-output"))
        .unwrap_or("cell-output")
        .to_string()
}

fn overlay_node(
    source: &MarkdownNode,
    kind: MarkdownNodeKind,
    figure: Option<MarkdownFigure>,
    stored_output: Option<MarkdownStoredOutput>,
) -> MarkdownNode {
    let mut node = source.clone();
    node.id = format!("{}-overlay", source.id);
    node.kind = kind;
    node.parent_id = None;
    node.children.clear();
    node.executable = None;
    node.citation = None;
    node.local_reference = None;
    node.figure = figure;
    node.stored_output = stored_output;
    node
}

fn reference_node(source: &MarkdownNode, reference: MarkdownLocalReference) -> MarkdownNode {
    let mut node = overlay_node(source, MarkdownNodeKind::Include, None, None);
    node.id = format!("{}-reference-{}", source.id, reference.target);
    node.destination = Some(reference.target.clone());
    node.local_reference = Some(reference);
    node
}

fn node_diagnostic(node: &MarkdownNode, code: &str, message: String) -> Diagnostic {
    let mut diagnostic = Diagnostic::warning("grist.markdown.notebook", code, message).partial();
    if let Some(range) = node.range.clone() {
        diagnostic = diagnostic.with_range(range);
    }
    if let Some(locator) = node.locator.clone() {
        diagnostic = diagnostic.with_locator(locator);
    }
    diagnostic
}

fn shortcode_include_target(raw: &str) -> Option<String> {
    raw.trim()
        .strip_prefix("{{<")?
        .strip_suffix(">}}")?
        .trim()
        .strip_prefix("include")?
        .split_whitespace()
        .next()
        .map(|value| value.trim_matches(['\'', '"']).to_string())
}

fn citation_ranges(
    text: &str,
    nodes: &[MarkdownNode],
    frontmatter: Option<&Frontmatter>,
) -> Vec<(Range<usize>, MarkdownCitation)> {
    let mut excluded = nodes
        .iter()
        .filter(|node| {
            matches!(
                node.kind,
                MarkdownNodeKind::CodeFence
                    | MarkdownNodeKind::InlineCode
                    | MarkdownNodeKind::StoredOutput
                    | MarkdownNodeKind::RawBlock
                    | MarkdownNodeKind::RawInline
                    | MarkdownNodeKind::DirectiveBlock
                    | MarkdownNodeKind::ExtensionInline
                    | MarkdownNodeKind::Link
                    | MarkdownNodeKind::Image
            )
        })
        .filter_map(|node| {
            node.range
                .as_ref()
                .map(|range| range.byte_start..range.byte_end)
        })
        .collect::<Vec<_>>();
    if let Some(frontmatter) = frontmatter {
        excluded.push(frontmatter.range.byte_start..frontmatter.range.byte_end);
    }
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'@'
            || excluded.iter().any(|range| range.contains(&cursor))
            || cursor.checked_sub(1).is_some_and(|index| {
                bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'/' | b'\\')
            })
        {
            cursor += 1;
            continue;
        }
        let mut end = cursor + 1;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric()
                || matches!(bytes[end], b'_' | b'-' | b':' | b'.'))
        {
            end += 1;
        }
        while end > cursor + 1 && bytes[end - 1] == b'.' {
            end -= 1;
        }
        if end == cursor + 1 {
            cursor += 1;
            continue;
        }
        let textual = cursor == 0 || !matches!(bytes[cursor - 1], b'[' | b';');
        ranges.push((
            cursor..end,
            MarkdownCitation {
                keys: vec![text[cursor + 1..end].to_string()],
                textual,
            },
        ));
        cursor = end;
    }
    ranges
}

fn frontmatter_references(value: &Value) -> Vec<(LocalReferenceKind, String)> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut references = Vec::new();
    for (key, kind) in [
        ("bibliography", LocalReferenceKind::Bibliography),
        ("csl", LocalReferenceKind::CitationStyle),
        ("resources", LocalReferenceKind::Resource),
        ("include-before-body", LocalReferenceKind::Include),
        ("include-after-body", LocalReferenceKind::Include),
        ("include-in-header", LocalReferenceKind::Include),
    ] {
        let Some(value) = object.get(key) else {
            continue;
        };
        match value {
            Value::String(target) => references.push((kind.clone(), target.clone())),
            Value::Array(values) => references.extend(values.iter().filter_map(|value| {
                value
                    .as_str()
                    .map(|target| (kind.clone(), target.to_string()))
            })),
            _ => {}
        }
    }
    references
}

fn is_local_reference_candidate(target: &str) -> bool {
    let target = target.trim();
    !target.is_empty() && !target.starts_with('#') && !looks_remote_reference(target)
}

struct LocalReferenceResolver {
    root: Option<PathBuf>,
    root_error: Option<String>,
    base: Option<PathBuf>,
    cache: HashMap<PathBuf, CachedReference>,
    retained_bytes: u64,
}

#[derive(Clone)]
struct CachedReference {
    resolved_path: Option<String>,
    content_sha256: String,
    content: Vec<u8>,
}

impl LocalReferenceResolver {
    fn new(source: &SourceInfo, options: &MarkdownOptions) -> Self {
        let (root, root_error) = match options.project_root.as_ref() {
            Some(root) => match fs::canonicalize(root) {
                Ok(root) if root.is_dir() => (Some(root), None),
                Ok(_) => (
                    None,
                    Some("supplied project root is not a directory".to_string()),
                ),
                Err(error) => (
                    None,
                    Some(format!("supplied project root is unavailable: {error}")),
                ),
            },
            None => (None, None),
        };
        let base = source.path.as_deref().and_then(|path| {
            let source_path = Path::new(path);
            let parent = source_path.parent()?;
            if source_path.is_absolute() {
                return fs::canonicalize(parent).ok();
            }

            let root = root.as_ref()?;
            // A relative source path may be either cwd-relative (as returned by
            // `SourceInfo::from_path`) or project-root-relative (a virtual
            // source label). Prefer an existing cwd-relative parent when it is
            // contained by the explicit root, then fall back to root-relative.
            fs::canonicalize(parent)
                .ok()
                .filter(|candidate| candidate.starts_with(root))
                .or_else(|| fs::canonicalize(root.join(parent)).ok())
        });
        Self {
            root,
            root_error,
            base,
            cache: HashMap::new(),
            retained_bytes: 0,
        }
    }

    fn resolve(
        &mut self,
        kind: LocalReferenceKind,
        target: String,
        node: &MarkdownNode,
        options: &MarkdownOptions,
    ) -> (MarkdownLocalReference, Option<Diagnostic>) {
        let unresolved = |status| MarkdownLocalReference {
            kind: kind.clone(),
            target: target.clone(),
            status,
            resolved_path: None,
            content_sha256: None,
            content: None,
        };
        let diagnostic = |code: &str, message: String| node_diagnostic(node, code, message);
        if !options.resolve_local_references {
            return (
                unresolved(LocalReferenceStatus::ReferenceOnly),
                Some(diagnostic(
                    "reference.resolution_disabled",
                    "local reference resolution is disabled".to_string(),
                )),
            );
        }
        if looks_remote_reference(&target) {
            return (
                unresolved(LocalReferenceStatus::RemoteDisabled),
                Some(diagnostic(
                    "reference.remote_disabled",
                    format!("remote reference `{target}` was retained without a network request"),
                )),
            );
        }
        let Some(root) = self.root.clone() else {
            return (
                unresolved(LocalReferenceStatus::ReferenceOnly),
                Some(diagnostic(
                    "reference.project_root_required",
                    self.root_error.clone().unwrap_or_else(|| {
                        "reference was retained because no explicit project root was supplied"
                            .to_string()
                    }),
                )),
            );
        };
        let target_path = Path::new(target.split(['#', '?']).next().unwrap_or(&target));
        if target_path.as_os_str().is_empty()
            || target_path.is_absolute()
            || target_path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return (
                unresolved(LocalReferenceStatus::OutsideProjectRoot),
                Some(diagnostic(
                    "reference.outside_project_root",
                    format!("reference `{target}` is not a safe project-relative path"),
                )),
            );
        }
        let base = self
            .base
            .as_ref()
            .filter(|base| base.starts_with(&root))
            .unwrap_or(&root);
        let candidate = base.join(target_path);
        let canonical = match fs::canonicalize(&candidate) {
            Ok(canonical) => canonical,
            Err(error) => {
                return (
                    unresolved(LocalReferenceStatus::Missing),
                    Some(diagnostic(
                        "reference.not_found",
                        format!("reference `{target}` could not be opened: {error}"),
                    )),
                );
            }
        };
        if !canonical.starts_with(&root) {
            return (
                unresolved(LocalReferenceStatus::OutsideProjectRoot),
                Some(diagnostic(
                    "reference.outside_project_root",
                    format!("reference `{target}` resolves outside the supplied project root"),
                )),
            );
        }
        if let Some(cached) = self.cache.get(&canonical).cloned() {
            if !self.reserve_retained_bytes(cached.content.len() as u64, options) {
                return (
                    unresolved(LocalReferenceStatus::BudgetExceeded),
                    Some(diagnostic(
                        "reference.aggregate_budget_exceeded",
                        format!(
                            "reference `{target}` would exceed the configured {} byte aggregate reference limit",
                            options.max_total_reference_bytes
                        ),
                    )),
                );
            }
            return (
                MarkdownLocalReference {
                    kind,
                    target,
                    status: LocalReferenceStatus::Resolved,
                    resolved_path: cached.resolved_path,
                    content_sha256: Some(cached.content_sha256),
                    content: Some(cached.content),
                },
                None,
            );
        }
        let file = match File::open(&canonical) {
            Ok(file) => file,
            Err(error) => {
                return (
                    unresolved(LocalReferenceStatus::Missing),
                    Some(diagnostic(
                        "reference.not_found",
                        format!("reference `{target}` could not be opened: {error}"),
                    )),
                );
            }
        };
        let metadata = match file.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => {
                return (
                    unresolved(LocalReferenceStatus::NotFile),
                    Some(diagnostic(
                        "reference.not_file",
                        format!("reference `{target}` does not resolve to a regular file"),
                    )),
                );
            }
        };
        if metadata.len() > options.max_reference_bytes {
            return (
                unresolved(LocalReferenceStatus::BudgetExceeded),
                Some(diagnostic(
                    "reference.budget_exceeded",
                    format!(
                        "reference `{target}` is {} bytes, above the configured {} byte limit",
                        metadata.len(),
                        options.max_reference_bytes
                    ),
                )),
            );
        }
        let mut content = Vec::with_capacity(
            metadata
                .len()
                .min(options.max_reference_bytes)
                .min(usize::MAX as u64) as usize,
        );
        let mut bounded = file.take(options.max_reference_bytes.saturating_add(1));
        if let Err(error) = bounded.read_to_end(&mut content) {
            return (
                unresolved(LocalReferenceStatus::Missing),
                Some(diagnostic(
                    "reference.not_found",
                    format!("reference `{target}` could not be read: {error}"),
                )),
            );
        }
        if content.len() as u64 > options.max_reference_bytes {
            return (
                unresolved(LocalReferenceStatus::BudgetExceeded),
                Some(diagnostic(
                    "reference.budget_exceeded",
                    format!(
                        "reference `{target}` grew beyond the configured {} byte limit while being read",
                        options.max_reference_bytes
                    ),
                )),
            );
        }
        if !self.reserve_retained_bytes(content.len() as u64, options) {
            return (
                unresolved(LocalReferenceStatus::BudgetExceeded),
                Some(diagnostic(
                    "reference.aggregate_budget_exceeded",
                    format!(
                        "reference `{target}` would exceed the configured {} byte aggregate reference limit",
                        options.max_total_reference_bytes
                    ),
                )),
            );
        }
        let resolved_path = canonical
            .strip_prefix(&root)
            .ok()
            .map(|path| path.to_string_lossy().replace('\\', "/"));
        let content_sha256 = sha256_hex(&content);
        self.cache.insert(
            canonical,
            CachedReference {
                resolved_path: resolved_path.clone(),
                content_sha256: content_sha256.clone(),
                content: content.clone(),
            },
        );
        (
            MarkdownLocalReference {
                kind,
                target,
                status: LocalReferenceStatus::Resolved,
                resolved_path,
                content_sha256: Some(content_sha256),
                content: Some(content),
            },
            None,
        )
    }

    fn reserve_retained_bytes(&mut self, bytes: u64, options: &MarkdownOptions) -> bool {
        let Some(next) = self.retained_bytes.checked_add(bytes) else {
            return false;
        };
        if next > options.max_total_reference_bytes {
            return false;
        }
        self.retained_bytes = next;
        true
    }
}

pub(crate) fn resolved_reference_input_bytes(document: &MarkdownDocument) -> u64 {
    let mut seen = std::collections::BTreeSet::new();
    document
        .nodes
        .iter()
        .filter_map(|node| node.local_reference.as_ref())
        .filter_map(|reference| {
            let content = reference.content.as_ref()?;
            let identity = (
                reference.resolved_path.clone(),
                reference.content_sha256.clone(),
            );
            seen.insert(identity).then_some(content.len() as u64)
        })
        .fold(0_u64, u64::saturating_add)
}

fn looks_remote_reference(target: &str) -> bool {
    let lower = target.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("//")
        || lower.starts_with("data:")
}
fn scan_extensions(
    decoded: &DecodedText,
    line_index: &LineIndex,
    body_start: usize,
) -> (Vec<MarkdownNode>, Vec<Diagnostic>) {
    let text = decoded.text.as_str();
    let mut nodes = Vec::new();
    let mut diagnostics = Vec::new();
    let mut offset = body_start;
    while offset < text.len() {
        let line_end = next_line_end(text, offset);
        let line = text[offset..line_end].trim_end_matches(['\r', '\n']);
        let trimmed = line.trim_start_matches(' ');
        if let Some(info) = trimmed.strip_prefix(":::").map(str::trim)
            && !info.is_empty()
        {
            let mut end = line_end;
            let mut cursor = line_end;
            let mut closed = false;
            while cursor < text.len() {
                let candidate_end = next_line_end(text, cursor);
                if text[cursor..candidate_end]
                    .trim_end_matches(['\r', '\n'])
                    .trim()
                    == ":::"
                {
                    end = candidate_end;
                    closed = true;
                    break;
                }
                end = candidate_end;
                cursor = candidate_end;
            }
            let mut node = source_node(
                nodes.len(),
                MarkdownNodeKind::DirectiveBlock,
                offset..end,
                decoded,
                line_index,
                None,
            );
            node.label = info
                .split_whitespace()
                .next()
                .map(|value| value.trim_matches(['{', '}']).to_string());
            node.info = Some(info.to_string());
            node.text = Some(text[line_end..end].to_string());
            nodes.push(node);
            if !closed {
                diagnostics.push(
                    Diagnostic::warning(
                        "grist.markdown.extension",
                        "directive.unclosed",
                        "colon-fenced directive has no closing ::: delimiter",
                    )
                    .with_range(SourceRange::new(offset, line_end, line_index))
                    .partial(),
                );
            }
            offset = end;
            continue;
        }
        for (start, end, name) in inline_extension_ranges(line, offset) {
            let mut node = source_node(
                nodes.len(),
                MarkdownNodeKind::ExtensionInline,
                start..end,
                decoded,
                line_index,
                None,
            );
            node.label = Some(name);
            node.text = Some(text[start..end].to_string());
            nodes.push(node);
        }
        offset = line_end;
    }
    (nodes, diagnostics)
}

fn inline_extension_ranges(line: &str, base: usize) -> Vec<(usize, usize, String)> {
    let mut ranges = Vec::new();
    for (opening, closing, name) in [
        ("{{<", ">}}", "shortcode"),
        ("{{%", "%}}", "shortcode"),
        ("{%", "%}", "template"),
        ("[[", "]]", "wiki_link"),
    ] {
        let mut cursor = 0;
        while let Some(relative) = line[cursor..].find(opening) {
            let start = cursor + relative;
            let content = start + opening.len();
            let Some(close) = line[content..].find(closing) else {
                break;
            };
            let end = content + close + closing.len();
            ranges.push((base + start, base + end, name.to_string()));
            cursor = end;
        }
    }
    ranges
}

fn scan_unclosed_fences(text: &str, body_start: usize, line_index: &LineIndex) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut open: Option<(u8, usize, usize)> = None;
    let mut offset = body_start;
    while offset < text.len() {
        let line_end = next_line_end(text, offset);
        let content = text[offset..line_end].trim_end_matches(['\r', '\n']);
        let indent = content.bytes().take_while(|byte| *byte == b' ').count();
        if indent <= 3 {
            let trimmed = &content[indent..];
            if let Some((marker, len)) = fence_marker(trimmed) {
                match open {
                    Some((open_marker, open_len, _))
                        if marker == open_marker
                            && len >= open_len
                            && trimmed[len..].trim().is_empty() =>
                    {
                        open = None;
                    }
                    None => open = Some((marker, len, offset + indent)),
                    _ => {}
                }
            }
        }
        offset = line_end;
    }
    if let Some((marker, len, start)) = open {
        let range = SourceRange::new(start, start + len, line_index);
        let locator = SourceLocator::exact(range.clone()).expect("fence diagnostic range is exact");
        diagnostics.push(
            Diagnostic::warning(
                "grist.markdown.fence",
                "fence.unclosed",
                format!(
                    "fenced code block starting with {} has no closing fence",
                    String::from_utf8_lossy(&vec![marker; len])
                ),
            )
            .with_range(range)
            .with_locator(locator)
            .partial(),
        );
    }
    diagnostics
}

fn next_line_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .find('\n')
        .map(|relative| offset + relative + 1)
        .unwrap_or(text.len())
}

fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let first = *line.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let len = line.bytes().take_while(|byte| *byte == first).count();
    (len >= 3).then_some((first, len))
}

fn directive_name(info: &str) -> Option<String> {
    let trimmed = info.trim();
    (trimmed.starts_with('{') && trimmed.ends_with('}') && trimmed.len() > 2)
        .then(|| trimmed[1..trimmed.len() - 1].trim().to_string())
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn link_type_name(value: LinkType) -> &'static str {
    match value {
        LinkType::Inline => "inline",
        LinkType::Reference => "reference",
        LinkType::ReferenceUnknown => "reference_unknown",
        LinkType::Collapsed => "collapsed",
        LinkType::CollapsedUnknown => "collapsed_unknown",
        LinkType::Shortcut => "shortcut",
        LinkType::ShortcutUnknown => "shortcut_unknown",
        LinkType::Autolink => "autolink",
        LinkType::Email => "email",
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn markdown_alignment(alignment: Alignment) -> MarkdownTableAlignment {
    match alignment {
        Alignment::None => MarkdownTableAlignment::None,
        Alignment::Left => MarkdownTableAlignment::Left,
        Alignment::Center => MarkdownTableAlignment::Center,
        Alignment::Right => MarkdownTableAlignment::Right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fixture(text: &str) -> MarkdownEnvelope {
        parse_markdown(text, SourceInfo::stdin("README.md"))
    }

    #[test]
    fn preserves_full_gfm_and_extended_structure_with_exact_ranges() {
        let source = "---\ntitle: Test\n---\n# Heading *em* {#intro .wide}\n\n- [x] **done**\n- [ ] ~~todo~~ with [link](https://example.test) and ![alt](img.png)\n\n> quote\n\n| name | score |\n| :--- | ---: |\n| alpha | 1 |\n\nFootnote[^n].\n\n[^n]: note\n\n<div>html</div>\n\n```{note}\nbody\n```\n\n::: warning\nextension\n:::\n";
        let report = parse_fixture(source);
        assert_eq!(report.status, OperationStatus::Complete);
        let payload = report.payload.unwrap();
        for kind in [
            MarkdownNodeKind::Heading,
            MarkdownNodeKind::Emphasis,
            MarkdownNodeKind::List,
            MarkdownNodeKind::TaskListMarker,
            MarkdownNodeKind::Strong,
            MarkdownNodeKind::Strikethrough,
            MarkdownNodeKind::Link,
            MarkdownNodeKind::Image,
            MarkdownNodeKind::BlockQuote,
            MarkdownNodeKind::Table,
            MarkdownNodeKind::FootnoteReference,
            MarkdownNodeKind::FootnoteDefinition,
            MarkdownNodeKind::HtmlBlock,
            MarkdownNodeKind::DirectiveBlock,
        ] {
            assert!(
                payload.nodes.iter().any(|node| node.kind == kind),
                "{kind:?}"
            );
        }
        assert!(payload.nodes.iter().all(|node| {
            let range = node.range.as_ref().unwrap();
            node.locator.is_some()
                && node.raw_range.is_some()
                && node.raw == source[range.byte_start..range.byte_end]
        }));
    }

    #[test]
    fn preserves_yaml_and_toml_frontmatter_variants() {
        let yaml = parse_fixture("---\ntitle: YAML\n...\n# Body\n")
            .payload
            .unwrap()
            .frontmatter
            .unwrap();
        assert_eq!(yaml.kind, FrontmatterKind::Yaml);
        assert_eq!(yaml.value.unwrap()["title"], "YAML");
        let toml_source = format!(
            "+++\ntitle = {}TOML{}\n+++\n# Body\n",
            char::from(34),
            char::from(34)
        );
        let toml = parse_fixture(&toml_source)
            .payload
            .unwrap()
            .frontmatter
            .unwrap();
        assert_eq!(toml.kind, FrontmatterKind::Toml);
        assert_eq!(toml.value.unwrap()["title"], "TOML");
    }

    #[test]
    fn malformed_constructs_are_partial_and_raw_is_retained() {
        let report = parse_fixture("---\ntitle: [bad\n---\n```rust\nunclosed\n");
        assert_eq!(report.status, OperationStatus::Partial);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|value| value.code == "frontmatter.parse")
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|value| value.code == "fence.unclosed")
        );
        assert!(
            report
                .payload
                .unwrap()
                .nodes
                .iter()
                .any(|node| node.kind == MarkdownNodeKind::CodeFence && !node.raw.is_empty())
        );
    }

    #[test]
    fn byte_entry_point_retains_decode_fidelity() {
        let source = b"# before \xff after";
        let report = parse_markdown_bytes(
            source,
            SourceInfo::stdin("mixed.md"),
            &MarkdownOptions {
                encoding: Some("utf-8".into()),
                ..MarkdownOptions::default()
            },
        );
        assert_eq!(report.status, OperationStatus::Partial);
        let payload = report.payload.unwrap();
        assert_eq!(payload.raw_bytes, source);
        assert!(payload.decoding.is_lossy());
    }

    #[test]
    fn table_projection_preserves_alignment_and_cell_ranges() {
        let payload = parse_fixture("| name | score |\n| :--- | ---: |\n| alpha | 1 |\n")
            .payload
            .unwrap();
        let table = payload
            .nodes
            .iter()
            .find(|node| node.kind == MarkdownNodeKind::Table)
            .and_then(|node| node.table.as_ref())
            .unwrap();
        assert_eq!(table.rows[0], vec!["name", "score"]);
        assert_eq!(table.rows[1], vec!["alpha", "1"]);
        assert_eq!(
            table.alignments,
            vec![MarkdownTableAlignment::Left, MarkdownTableAlignment::Right]
        );
        assert!(table.row_details[0].header);
        assert!(table.row_details[0].cells[0].locator.is_some());
    }
}
