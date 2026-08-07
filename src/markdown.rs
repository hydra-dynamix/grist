//! CommonMark/GFM and extension-preserving Markdown parser.
//!
//! The typed payload is authoritative. Graph, segment, and render output are
//! deterministic projections of this lossless source model.

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, Envelope, FormatIdentity, LineIndex, OperationKind,
    OperationStatus, ParserInfo, SchemaVersion, SourceInfo, SourceLocator, SourceRange,
    options_digest,
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
use std::ops::Range;

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
    RawBlock,
    RawInline,
    Text,
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
    let (payload, mut diagnostics) = document_from_decoded(decoded, options);
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
    options: &MarkdownOptions,
) -> (MarkdownDocument, Vec<Diagnostic>) {
    let text = decoded.text.as_str();
    let line_index = LineIndex::new(text);
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
