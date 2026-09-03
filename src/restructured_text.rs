//! Inert, source-preserving reStructuredText parser.
//!
//! Directives never execute and remote resources are never fetched. Local
//! ``include`` directives are resolved only below a caller-supplied project
//! root after canonicalization.

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, Envelope, FormatIdentity, LineIndex, OperationKind,
    OperationStatus, ParserInfo, SchemaVersion, SourceInfo, SourceLocator, SourceRange,
    options_digest, sha256_hex,
};
use crate::decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeReport, DecodedByteRange, DecodedText,
    RawByteRange, TextEncoding, decode_text,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestructuredTextDocument {
    pub schema_version: String,
    pub raw_bytes: Vec<u8>,
    pub raw_range: RawByteRange,
    pub decoded_text: String,
    pub decoded_range: SourceRange,
    pub locator: SourceLocator,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub nodes: Vec<RestructuredTextNode>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestructuredTextNode {
    pub id: String,
    pub kind: RestructuredTextNodeKind,
    pub range: SourceRange,
    pub locator: SourceLocator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_range: Option<RawByteRange>,
    pub raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<RestructuredTextTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<IncludeReference>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub known_syntax: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestructuredTextNodeKind {
    Heading,
    Paragraph,
    Directive,
    Role,
    Include,
    CodeBlock,
    LiteralBlock,
    DoctestBlock,
    Table,
    TableRow,
    TableCell,
    FootnoteDefinition,
    FootnoteReference,
    CitationDefinition,
    CitationReference,
    Target,
    CrossReference,
    Hyperlink,
    List,
    ListItem,
    DefinitionList,
    DefinitionTerm,
    DefinitionDescription,
    FieldList,
    Field,
    Emphasis,
    Strong,
    InlineCode,
    SubstitutionDefinition,
    SubstitutionReference,
    Transition,
    Comment,
    RawBlock,
    RawInline,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestructuredTextTable {
    pub style: RestructuredTextTableStyle,
    pub rows: Vec<RestructuredTextTableRow>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestructuredTextTableStyle {
    Grid,
    Simple,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestructuredTextTableRow {
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub header: bool,
    pub cells: Vec<RestructuredTextTableCell>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestructuredTextTableCell {
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub text: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IncludeStatus {
    Resolved,
    ReferenceOnly,
    RemoteDisabled,
    OutsideProjectRoot,
    Missing,
    NotFile,
    Cycle,
    DepthExceeded,
    BudgetExceeded,
    DecodeFailed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IncludeReference {
    pub target: String,
    pub status: IncludeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<ResolvedInclude>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedInclude {
    pub source: SourceInfo,
    pub raw_bytes: Vec<u8>,
    pub content_sha256: String,
    pub decoded_text: String,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub nodes: Vec<RestructuredTextNode>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RestructuredTextOptions {
    pub encoding: Option<String>,
    /// Explicit filesystem boundary. No root means includes remain references.
    pub project_root: Option<PathBuf>,
    pub resolve_includes: bool,
    pub max_include_depth: u16,
    pub max_include_bytes: u64,
    pub retain_comments: bool,
}

impl Default for RestructuredTextOptions {
    fn default() -> Self {
        Self {
            encoding: None,
            project_root: None,
            resolve_includes: true,
            max_include_depth: 16,
            max_include_bytes: 8 * 1024 * 1024,
            retain_comments: true,
        }
    }
}

impl crate::core::FormatOptions for RestructuredTextOptions {
    const FORMAT: &'static str = "restructured-text";
}

pub type RestructuredTextEnvelope = Envelope<RestructuredTextDocument>;

pub fn parse_restructured_text(text: &str, source: SourceInfo) -> RestructuredTextEnvelope {
    parse_restructured_text_with_options(text, source, &RestructuredTextOptions::default())
}

pub fn parse_restructured_text_with_options(
    text: &str,
    source: SourceInfo,
    options: &RestructuredTextOptions,
) -> RestructuredTextEnvelope {
    parse_restructured_text_bytes(text.as_bytes(), source, options)
}

pub fn parse_restructured_text_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &RestructuredTextOptions,
) -> RestructuredTextEnvelope {
    let mut decode_options = DecodeOptions::for_media_type(
        source.declared_mime_type.as_deref(),
        Some("restructured_text"),
    );
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
    ParserInfo::new("grist.restructured_text")
        .with_implementation("grist-rst", env!("CARGO_PKG_VERSION"))
        .with_specification_version("Docutils reStructuredText syntax; inert directives")
        .with_feature("restructured-text")
}

fn envelope_from_decoded(
    decoded: &DecodedText,
    source: SourceInfo,
    options: &RestructuredTextOptions,
) -> RestructuredTextEnvelope {
    let (payload, mut diagnostics) = document_from_decoded(decoded, &source, options);
    let partial = decoded.report.makes_operation_partial()
        || diagnostics.iter().any(|diagnostic| diagnostic.partial);
    let mut all_diagnostics = decoded.report.diagnostics.clone();
    all_diagnostics.append(&mut diagnostics);
    let digest = options_digest(options).expect("reStructuredText options serialize");
    let mut envelope = if partial {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::RestructuredText,
            source,
            parser_info(),
            digest,
            SchemaVersion::RESTRUCTURED_TEXT_V1,
            Some(payload),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::RestructuredText,
            source,
            parser_info(),
            digest,
            SchemaVersion::RESTRUCTURED_TEXT_V1,
            payload,
        )
    };
    envelope.diagnostics = all_diagnostics;
    envelope
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    envelope
        .with_identity(
            ContentIdentity::for_raw_bytes(decoded.raw_bytes())
                .with_decoded(
                    &decoded.text,
                    decoded.report.encoding.label(),
                    decoded.report.is_lossy(),
                )
                .with_format(FormatIdentity::new("restructured-text", Some("text/x-rst"))),
        )
        .with_canonical_payload_identity()
        .expect("reStructuredText payload canonicalization is infallible")
}

fn failed_decode_envelope(
    bytes: &[u8],
    source: SourceInfo,
    options: &RestructuredTextOptions,
    error: DecodeError,
) -> RestructuredTextEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::RestructuredText,
        OperationStatus::Failed,
        source,
        parser_info(),
        options_digest(options).expect("reStructuredText options serialize"),
        SchemaVersion::RESTRUCTURED_TEXT_V1,
    )
    .expect("failed reStructuredText decode has valid envelope status")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes)
            .with_format(FormatIdentity::new("restructured-text", Some("text/x-rst"))),
    )
    .with_diagnostics(vec![
        error.diagnostic().with_parser("grist.restructured_text"),
    ])
}

pub(crate) fn document_from_decoded(
    decoded: &DecodedText,
    source: &SourceInfo,
    options: &RestructuredTextOptions,
) -> (RestructuredTextDocument, Vec<Diagnostic>) {
    let mut resolver = IncludeResolver::new(source, options);
    let (nodes, diagnostics) = parse_nodes(decoded, source, options, 0, &mut resolver);
    let index = LineIndex::new(&decoded.text);
    let decoded_range = SourceRange::new(0, decoded.text.len(), &index);
    let locator = SourceLocator::exact(decoded_range.clone())
        .expect("whole reStructuredText source range is exact");
    let document = RestructuredTextDocument {
        schema_version: SchemaVersion::RESTRUCTURED_TEXT_V1.to_string(),
        raw_bytes: decoded.raw_bytes().to_vec(),
        raw_range: RawByteRange {
            start: 0,
            end: decoded.raw_bytes().len() as u64,
        },
        decoded_text: decoded.text.clone(),
        decoded_range,
        locator,
        encoding: decoded.report.encoding.clone(),
        decoding: decoded.report.clone(),
        nodes,
    };
    (document, diagnostics)
}

pub(crate) fn payload_node_count(document: &RestructuredTextDocument) -> usize {
    fn count(nodes: &[RestructuredTextNode]) -> usize {
        nodes
            .iter()
            .map(|node| {
                1 + node
                    .include
                    .as_ref()
                    .and_then(|include| include.resolved.as_ref())
                    .map(|resolved| count(&resolved.nodes))
                    .unwrap_or(0)
            })
            .sum()
    }
    count(&document.nodes).saturating_add(1)
}

pub(crate) fn payload_decoded_char_count(document: &RestructuredTextDocument) -> usize {
    fn included(nodes: &[RestructuredTextNode]) -> usize {
        nodes
            .iter()
            .filter_map(|node| node.include.as_ref()?.resolved.as_ref())
            .map(|resolved| resolved.decoded_text.chars().count() + included(&resolved.nodes))
            .sum()
    }
    document.decoded_text.chars().count() + included(&document.nodes)
}

#[derive(Debug, Clone)]
struct SourceLine<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

fn source_lines(text: &str) -> Vec<SourceLine<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = text[start..]
            .find('\n')
            .map(|relative| start + relative + 1)
            .unwrap_or(text.len());
        let content_end = text[start..end].trim_end_matches(['\r', '\n']).len() + start;
        lines.push(SourceLine {
            start,
            end,
            text: &text[start..content_end],
        });
        start = end;
    }
    if text.is_empty() {
        lines.push(SourceLine {
            start: 0,
            end: 0,
            text: "",
        });
    }
    lines
}

struct NodeBuilder<'a> {
    decoded: &'a DecodedText,
    index: LineIndex,
    nodes: Vec<RestructuredTextNode>,
}

impl<'a> NodeBuilder<'a> {
    fn new(decoded: &'a DecodedText) -> Self {
        Self {
            decoded,
            index: LineIndex::new(&decoded.text),
            nodes: Vec::new(),
        }
    }

    fn push(
        &mut self,
        kind: RestructuredTextNodeKind,
        range: Range<usize>,
        parent_id: Option<String>,
    ) -> usize {
        let source_range = SourceRange::new(range.start, range.end, &self.index);
        let locator = SourceLocator::exact(source_range.clone())
            .expect("parser-created reStructuredText range is exact");
        let raw_range = self.decoded.raw_range_for_decoded(DecodedByteRange {
            start: range.start as u64,
            end: range.end as u64,
        });
        let id = format!("rst-node-{:06}", self.nodes.len());
        self.nodes.push(RestructuredTextNode {
            id,
            kind,
            range: source_range,
            locator,
            raw_range,
            raw: self.decoded.text[range].to_string(),
            text: None,
            level: None,
            name: None,
            argument: None,
            target: None,
            role: None,
            options: BTreeMap::new(),
            parent_id,
            children: Vec::new(),
            table: None,
            include: None,
            known_syntax: false,
        });
        self.nodes.len() - 1
    }

    fn link_child(&mut self, parent: usize, child: usize) {
        let parent_id = self.nodes[parent].id.clone();
        let child_id = self.nodes[child].id.clone();
        self.nodes[child].parent_id = Some(parent_id);
        self.nodes[parent].children.push(child_id);
    }
}

fn attach_to_parent(builder: &mut NodeBuilder<'_>, parent: Option<usize>, child: usize) {
    if let Some(parent) = parent {
        let child_id = builder.nodes[child].id.clone();
        if !builder.nodes[parent].children.contains(&child_id) {
            builder.nodes[parent].children.push(child_id);
        }
    }
}

fn parse_nodes(
    decoded: &DecodedText,
    source: &SourceInfo,
    options: &RestructuredTextOptions,
    depth: u16,
    resolver: &mut IncludeResolver,
) -> (Vec<RestructuredTextNode>, Vec<Diagnostic>) {
    let lines = source_lines(&decoded.text);
    let mut builder = NodeBuilder::new(decoded);
    let mut diagnostics = Vec::new();
    let mut heading_styles = BTreeMap::<String, u8>::new();
    let mut next_heading_level = 1_u8;
    let mut heading_stack = Vec::<(u8, usize)>::new();
    let mut current_parent = None;
    let mut cursor = 0;

    while cursor < lines.len() {
        if lines[cursor].text.trim().is_empty() {
            cursor += 1;
            continue;
        }
        if let Some((end, title, style)) = heading_at(&lines, cursor) {
            let level = *heading_styles.entry(style).or_insert_with(|| {
                let assigned = next_heading_level;
                next_heading_level = next_heading_level.saturating_add(1);
                assigned
            });
            while heading_stack.last().is_some_and(|(open, _)| *open >= level) {
                heading_stack.pop();
            }
            let parent = heading_stack.last().map(|(_, index)| *index);
            let node = builder.push(
                RestructuredTextNodeKind::Heading,
                lines[cursor].start..lines[end - 1].end,
                parent.map(|value| builder.nodes[value].id.clone()),
            );
            builder.nodes[node].text = Some(title);
            builder.nodes[node].level = Some(level);
            attach_to_parent(&mut builder, parent, node);
            heading_stack.push((level, node));
            current_parent = Some(node);
            cursor = end;
            continue;
        }
        let trimmed = lines[cursor].text.trim_start();
        if trimmed.starts_with(".. ") || trimmed == ".." {
            cursor = handle_explicit(
                &lines,
                cursor,
                current_parent,
                &mut builder,
                &mut diagnostics,
                source,
                options,
                depth,
                resolver,
            );
            continue;
        }
        if grid_table_start(lines[cursor].text) {
            let end = grid_table_end(&lines, cursor);
            handle_table(
                &lines,
                cursor,
                end,
                current_parent,
                &mut builder,
                &mut diagnostics,
                RestructuredTextTableStyle::Grid,
            );
            cursor = end;
            continue;
        }
        if simple_table_delimiter(lines[cursor].text).is_some() {
            let end = simple_table_end(&lines, cursor);
            if end > cursor + 2
                && handle_table(
                    &lines,
                    cursor,
                    end,
                    current_parent,
                    &mut builder,
                    &mut diagnostics,
                    RestructuredTextTableStyle::Simple,
                )
            {
                cursor = end;
                continue;
            }
        }
        if is_transition(lines[cursor].text) {
            let node = builder.push(
                RestructuredTextNodeKind::Transition,
                lines[cursor].start..lines[cursor].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            attach_to_parent(&mut builder, current_parent, node);
            cursor += 1;
            continue;
        }
        if list_marker(lines[cursor].text).is_some() {
            let (end, items) = list_extent(&lines, cursor);
            let list = builder.push(
                RestructuredTextNodeKind::List,
                lines[cursor].start..lines[end - 1].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            attach_to_parent(&mut builder, current_parent, list);
            for (item_start, item_end, marker_len) in items {
                let item = builder.push(
                    RestructuredTextNodeKind::ListItem,
                    lines[item_start].start..lines[item_end - 1].end,
                    Some(builder.nodes[list].id.clone()),
                );
                builder.nodes[item].text = Some(
                    lines[item_start].text[marker_len.min(lines[item_start].text.len())..]
                        .trim()
                        .to_string(),
                );
                builder.link_child(list, item);
                scan_inline_children(&mut builder, item, &mut diagnostics);
            }
            cursor = end;
            continue;
        }
        if field_marker(lines[cursor].text).is_some() {
            let (end, fields) = field_list_extent(&lines, cursor);
            let list = builder.push(
                RestructuredTextNodeKind::FieldList,
                lines[cursor].start..lines[end - 1].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            attach_to_parent(&mut builder, current_parent, list);
            for (line_index, name, value) in fields {
                let field = builder.push(
                    RestructuredTextNodeKind::Field,
                    lines[line_index].start..lines[line_index].end,
                    Some(builder.nodes[list].id.clone()),
                );
                builder.nodes[field].name = Some(name);
                builder.nodes[field].text = Some(value);
                builder.link_child(list, field);
                scan_inline_children(&mut builder, field, &mut diagnostics);
            }
            cursor = end;
            continue;
        }
        if trimmed.starts_with(">>> ") || trimmed == ">>>" {
            let end = doctest_extent(&lines, cursor);
            let node = builder.push(
                RestructuredTextNodeKind::DoctestBlock,
                lines[cursor].start..lines[end - 1].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            builder.nodes[node].text = Some(builder.nodes[node].raw.clone());
            attach_to_parent(&mut builder, current_parent, node);
            cursor = end;
            continue;
        }
        let paragraph_end = paragraph_extent(&lines, cursor);
        let paragraph = builder.push(
            RestructuredTextNodeKind::Paragraph,
            lines[cursor].start..lines[paragraph_end - 1].end,
            current_parent.map(|value| builder.nodes[value].id.clone()),
        );
        builder.nodes[paragraph].text = Some(
            lines[cursor..paragraph_end]
                .iter()
                .map(|line| line.text.trim())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        attach_to_parent(&mut builder, current_parent, paragraph);
        scan_inline_children(&mut builder, paragraph, &mut diagnostics);
        let literal = lines[paragraph_end - 1].text.trim_end().ends_with("::");
        cursor = paragraph_end;
        if literal {
            while cursor < lines.len() && lines[cursor].text.trim().is_empty() {
                cursor += 1;
            }
            if cursor < lines.len() && indentation(lines[cursor].text) > 0 {
                let end = indented_extent(&lines, cursor);
                let node = builder.push(
                    RestructuredTextNodeKind::LiteralBlock,
                    lines[cursor].start..lines[end - 1].end,
                    current_parent.map(|value| builder.nodes[value].id.clone()),
                );
                builder.nodes[node].text = Some(dedent(&lines[cursor..end]));
                attach_to_parent(&mut builder, current_parent, node);
                cursor = end;
            } else {
                diagnostics.push(
                    Diagnostic::warning(
                        "grist.restructured_text.literal",
                        "literal_block.missing",
                        "paragraph ending in `::` has no indented literal block",
                    )
                    .with_range(builder.nodes[paragraph].range.clone())
                    .with_locator(builder.nodes[paragraph].locator.clone())
                    .partial(),
                );
            }
        }
    }
    (builder.nodes, diagnostics)
}

fn heading_at(lines: &[SourceLine<'_>], cursor: usize) -> Option<(usize, String, String)> {
    let current = lines.get(cursor)?.text.trim();
    if let Some(marker) = adornment(current)
        && let (Some(title), Some(underline)) = (lines.get(cursor + 1), lines.get(cursor + 2))
        && !title.text.trim().is_empty()
        && adornment(underline.text.trim()) == Some(marker)
    {
        return Some((
            cursor + 3,
            title.text.trim().to_string(),
            format!("overline:{marker}"),
        ));
    }
    let marker = adornment(lines.get(cursor + 1)?.text.trim())?;
    (!current.is_empty()).then(|| {
        (
            cursor + 2,
            current.to_string(),
            format!("underline:{marker}"),
        )
    })
}

fn adornment(value: &str) -> Option<char> {
    let mut chars = value.chars();
    let marker = chars.next()?;
    (value.chars().count() >= 3
        && !marker.is_ascii_alphanumeric()
        && !marker.is_whitespace()
        && chars.all(|value| value == marker))
    .then_some(marker)
}

enum ExplicitMarkup {
    Footnote {
        label: String,
        body: String,
    },
    Citation {
        label: String,
        body: String,
    },
    Target {
        label: String,
        target: String,
    },
    Substitution {
        name: String,
        directive: String,
    },
    Directive {
        name: String,
        argument: String,
        fields: BTreeMap<String, String>,
        body: String,
    },
    Comment,
    Malformed,
}

fn explicit_markup_extent(lines: &[SourceLine<'_>], cursor: usize) -> (usize, ExplicitMarkup) {
    let base_indent = indentation(lines[cursor].text);
    let mut end = cursor + 1;
    while end < lines.len() {
        if lines[end].text.trim().is_empty() {
            let mut probe = end + 1;
            while probe < lines.len() && lines[probe].text.trim().is_empty() {
                probe += 1;
            }
            if probe < lines.len() && indentation(lines[probe].text) > base_indent {
                end = probe + 1;
                continue;
            }
            break;
        }
        if indentation(lines[end].text) > base_indent {
            end += 1;
        } else {
            break;
        }
    }
    let first = lines[cursor].text.trim_start();
    let continuation = dedent(&lines[cursor + 1..end]).trim().to_string();
    if let Some(rest) = first.strip_prefix(".. [")
        && let Some(close) = rest.find(']')
    {
        let label = rest[..close].trim().to_string();
        let mut body = rest[close + 1..].trim().to_string();
        if !continuation.is_empty() {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(&continuation);
        }
        if footnote_label(&label) {
            return (end, ExplicitMarkup::Footnote { label, body });
        }
        return (end, ExplicitMarkup::Citation { label, body });
    }
    if let Some(rest) = first.strip_prefix(".. _")
        && let Some((label, target)) = rest.split_once(':')
    {
        return (
            end,
            ExplicitMarkup::Target {
                label: label.trim().to_string(),
                target: target.trim().to_string(),
            },
        );
    }
    if let Some(rest) = first.strip_prefix(".. |")
        && let Some((name, directive)) = rest.split_once("| ")
        && directive.contains("::")
    {
        return (
            end,
            ExplicitMarkup::Substitution {
                name: name.trim().to_string(),
                directive: directive.trim().to_string(),
            },
        );
    }
    if let Some(rest) = first.strip_prefix(".. ") {
        if rest.trim().is_empty() {
            return (end, ExplicitMarkup::Comment);
        }
        if let Some((name, argument)) = rest.split_once("::") {
            let name = name.trim();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '+'))
            {
                return (end, ExplicitMarkup::Malformed);
            }
            let mut fields = BTreeMap::new();
            let mut body = Vec::new();
            for line in &lines[cursor + 1..end] {
                let trimmed = line.text.trim();
                if let Some((field, value)) = field_marker(trimmed) {
                    fields.insert(field, value);
                } else if !trimmed.is_empty() {
                    body.push(trimmed);
                }
            }
            return (
                end,
                ExplicitMarkup::Directive {
                    name: name.to_string(),
                    argument: argument.trim().to_string(),
                    fields,
                    body: body.join("\n"),
                },
            );
        }
        return (end, ExplicitMarkup::Comment);
    }
    (end, ExplicitMarkup::Malformed)
}

fn footnote_label(label: &str) -> bool {
    label.chars().all(|value| value.is_ascii_digit()) || label.starts_with('#') || label == "*"
}

#[allow(clippy::too_many_arguments)]
fn handle_explicit(
    lines: &[SourceLine<'_>],
    cursor: usize,
    parent: Option<usize>,
    builder: &mut NodeBuilder<'_>,
    diagnostics: &mut Vec<Diagnostic>,
    source: &SourceInfo,
    options: &RestructuredTextOptions,
    depth: u16,
    resolver: &mut IncludeResolver,
) -> usize {
    let (end, markup) = explicit_markup_extent(lines, cursor);
    let range = lines[cursor].start..lines[end - 1].end;
    let parent_id = parent.map(|value| builder.nodes[value].id.clone());
    match markup {
        ExplicitMarkup::Footnote { label, body } => {
            let node = builder.push(
                RestructuredTextNodeKind::FootnoteDefinition,
                range,
                parent_id,
            );
            builder.nodes[node].name = Some(label);
            builder.nodes[node].text = Some(body);
            attach_to_parent(builder, parent, node);
            scan_inline_children(builder, node, diagnostics);
        }
        ExplicitMarkup::Citation { label, body } => {
            let node = builder.push(
                RestructuredTextNodeKind::CitationDefinition,
                range,
                parent_id,
            );
            builder.nodes[node].name = Some(label);
            builder.nodes[node].text = Some(body);
            attach_to_parent(builder, parent, node);
            scan_inline_children(builder, node, diagnostics);
        }
        ExplicitMarkup::Target { label, target } => {
            let node = builder.push(RestructuredTextNodeKind::Target, range, parent_id);
            builder.nodes[node].name = Some(label);
            builder.nodes[node].target = nonempty(target);
            attach_to_parent(builder, parent, node);
        }
        ExplicitMarkup::Substitution { name, directive } => {
            let node = builder.push(
                RestructuredTextNodeKind::SubstitutionDefinition,
                range,
                parent_id,
            );
            builder.nodes[node].name = Some(name);
            builder.nodes[node].argument = Some(directive);
            builder.nodes[node].known_syntax = true;
            attach_to_parent(builder, parent, node);
        }
        ExplicitMarkup::Directive {
            name,
            argument,
            fields,
            body,
        } => {
            let lower = name.to_ascii_lowercase();
            let kind = match lower.as_str() {
                "include" => RestructuredTextNodeKind::Include,
                "code" | "code-block" | "sourcecode" | "parsed-literal" => {
                    RestructuredTextNodeKind::CodeBlock
                }
                _ => RestructuredTextNodeKind::Directive,
            };
            let node = builder.push(kind, range, parent_id);
            builder.nodes[node].name = Some(name.clone());
            builder.nodes[node].argument = nonempty(argument.clone());
            builder.nodes[node].options = fields;
            builder.nodes[node].text = nonempty(body);
            builder.nodes[node].known_syntax = known_directive(&lower);
            attach_to_parent(builder, parent, node);
            if lower == "include" {
                let (include, mut include_diagnostics) = resolver.resolve(
                    argument.trim().to_string(),
                    &builder.nodes[node].range,
                    &builder.nodes[node].locator,
                    source,
                    options,
                    depth,
                );
                builder.nodes[node].include = Some(include);
                diagnostics.append(&mut include_diagnostics);
            } else if matches!(
                lower.as_str(),
                "code" | "code-block" | "sourcecode" | "parsed-literal"
            ) {
                builder.nodes[node].role = nonempty(argument);
            } else if !builder.nodes[node].known_syntax {
                diagnostics.push(
                    Diagnostic::info(
                        "grist.restructured_text.directive",
                        "directive.unknown_inert",
                        format!("unknown directive `{name}` was retained without execution"),
                    )
                    .with_range(builder.nodes[node].range.clone())
                    .with_locator(builder.nodes[node].locator.clone()),
                );
            }
        }
        ExplicitMarkup::Comment => {
            if options.retain_comments {
                let node = builder.push(RestructuredTextNodeKind::Comment, range, parent_id);
                attach_to_parent(builder, parent, node);
            }
        }
        ExplicitMarkup::Malformed => {
            let node = builder.push(RestructuredTextNodeKind::RawBlock, range, parent_id);
            attach_to_parent(builder, parent, node);
            diagnostics.push(
                Diagnostic::warning(
                    "grist.restructured_text.explicit_markup",
                    "explicit_markup.malformed",
                    "malformed explicit markup was retained as raw source",
                )
                .with_range(builder.nodes[node].range.clone())
                .with_locator(builder.nodes[node].locator.clone())
                .partial(),
            );
        }
    }
    end
}

fn known_directive(name: &str) -> bool {
    matches!(
        name,
        "admonition"
            | "attention"
            | "caution"
            | "code"
            | "code-block"
            | "compound"
            | "container"
            | "contents"
            | "csv-table"
            | "danger"
            | "date"
            | "default-role"
            | "epigraph"
            | "error"
            | "figure"
            | "footer"
            | "header"
            | "highlights"
            | "hint"
            | "image"
            | "important"
            | "include"
            | "list-table"
            | "math"
            | "note"
            | "parsed-literal"
            | "pull-quote"
            | "raw"
            | "replace"
            | "role"
            | "rubric"
            | "sectnum"
            | "sidebar"
            | "sourcecode"
            | "table"
            | "target-notes"
            | "title"
            | "tip"
            | "topic"
            | "unicode"
            | "warning"
    )
}

type InlineMatch = (
    usize,
    usize,
    RestructuredTextNodeKind,
    Option<String>,
    Option<String>,
);

fn scan_inline_children(
    builder: &mut NodeBuilder<'_>,
    parent: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let start = builder.nodes[parent].range.byte_start;
    let end = builder.nodes[parent].range.byte_end;
    let raw = builder.decoded.text[start..end].to_string();
    let mut matches = Vec::<InlineMatch>::new();
    let mut cursor = 0;
    while let Some(relative) = raw[cursor..].find(":`") {
        let separator = cursor + relative;
        let Some(role_start) = raw[..separator].rfind(':') else {
            cursor = separator + 2;
            continue;
        };
        let role = &raw[role_start + 1..separator];
        if role.is_empty()
            || !role
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '+'))
        {
            cursor = separator + 2;
            continue;
        }
        let content_start = separator + 2;
        let Some(close) = raw[content_start..].find('`') else {
            diagnostics.push(
                Diagnostic::warning(
                    "grist.restructured_text.inline",
                    "role.unclosed",
                    format!("role `{role}` has no closing backtick"),
                )
                .with_range(builder.nodes[parent].range.clone())
                .with_locator(builder.nodes[parent].locator.clone())
                .partial(),
            );
            break;
        };
        let close = content_start + close;
        matches.push((
            start + role_start,
            start + close + 1,
            RestructuredTextNodeKind::Role,
            Some(role.to_string()),
            Some(raw[content_start..close].to_string()),
        ));
        cursor = close + 1;
    }
    cursor = 0;
    while cursor < raw.len() {
        let Some(open_relative) = raw[cursor..].find('`') else {
            break;
        };
        let open = cursor + open_relative;
        let Some(close_relative) = raw[open + 1..].find('`') else {
            break;
        };
        let close = open + 1 + close_relative;
        let content = &raw[open + 1..close];
        let after = &raw[close + 1..];
        if let Some(rest) = after.strip_prefix(':')
            && let Some(role_end) = rest.find(':')
        {
            let role = &rest[..role_end];
            matches.push((
                start + open,
                start + close + role_end + 3,
                RestructuredTextNodeKind::Role,
                Some(role.to_string()),
                Some(content.to_string()),
            ));
            cursor = close + role_end + 3;
            continue;
        }
        if after.starts_with('_') {
            if let Some(target_open) = content.rfind(" <")
                && content.ends_with('>')
            {
                matches.push((
                    start + open,
                    start + close + 2,
                    RestructuredTextNodeKind::Hyperlink,
                    Some(content[..target_open].to_string()),
                    Some(content[target_open + 2..content.len() - 1].to_string()),
                ));
            } else {
                matches.push((
                    start + open,
                    start + close + 2,
                    RestructuredTextNodeKind::CrossReference,
                    Some(content.to_string()),
                    None,
                ));
            }
            cursor = close + 2;
            continue;
        }
        cursor = close + 1;
    }
    cursor = 0;
    while cursor < raw.len() {
        if raw.as_bytes()[cursor] == b'['
            && let Some(close_relative) = raw[cursor + 1..].find("]_")
        {
            let close = cursor + 1 + close_relative;
            let label = raw[cursor + 1..close].to_string();
            let kind = if footnote_label(&label) {
                RestructuredTextNodeKind::FootnoteReference
            } else {
                RestructuredTextNodeKind::CitationReference
            };
            matches.push((start + cursor, start + close + 2, kind, Some(label), None));
            cursor = close + 2;
            continue;
        }
        if raw.as_bytes()[cursor] == b'|'
            && let Some(close_relative) = raw[cursor + 1..].find('|')
        {
            let close = cursor + 1 + close_relative;
            let name = raw[cursor + 1..close].trim();
            if !name.is_empty() {
                matches.push((
                    start + cursor,
                    start + close + 1,
                    RestructuredTextNodeKind::SubstitutionReference,
                    Some(name.to_string()),
                    None,
                ));
                cursor = close + 1;
                continue;
            }
        }
        cursor += raw[cursor..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
    }
    for (opening, closing, kind) in [
        ("**", "**", RestructuredTextNodeKind::Strong),
        ("``", "``", RestructuredTextNodeKind::InlineCode),
        ("*", "*", RestructuredTextNodeKind::Emphasis),
    ] {
        let mut offset = 0;
        while let Some(relative) = raw[offset..].find(opening) {
            let open = offset + relative;
            let content_start = open + opening.len();
            let Some(close_relative) = raw[content_start..].find(closing) else {
                break;
            };
            let close = content_start + close_relative;
            matches.push((
                start + open,
                start + close + closing.len(),
                kind.clone(),
                None,
                Some(raw[content_start..close].to_string()),
            ));
            offset = close + closing.len();
        }
    }
    for (underscore, _) in raw.match_indices('_') {
        if underscore == 0
            || !raw.as_bytes()[underscore - 1].is_ascii_alphanumeric()
            || raw
                .as_bytes()
                .get(underscore + 1)
                .is_some_and(u8::is_ascii_alphanumeric)
        {
            continue;
        }
        let label_start = raw[..underscore]
            .rfind(|value: char| !value.is_ascii_alphanumeric() && !matches!(value, '-' | '.'))
            .map(|offset| offset + 1)
            .unwrap_or(0);
        let label = &raw[label_start..underscore];
        if !label.is_empty() {
            matches.push((
                start + label_start,
                start + underscore + 1,
                RestructuredTextNodeKind::CrossReference,
                Some(label.to_string()),
                None,
            ));
        }
    }
    matches.sort_by_key(|value| (value.0, value.1));
    matches.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1 && left.2 == right.2);
    for (range_start, range_end, kind, name, value) in matches {
        if range_start >= range_end || range_end > builder.decoded.text.len() {
            continue;
        }
        let child = builder.push(
            kind.clone(),
            range_start..range_end,
            Some(builder.nodes[parent].id.clone()),
        );
        match kind {
            RestructuredTextNodeKind::Role => {
                builder.nodes[child].role = name.clone();
                builder.nodes[child].known_syntax = name.as_deref().is_some_and(known_role);
                builder.nodes[child].text = value;
            }
            RestructuredTextNodeKind::Hyperlink => {
                builder.nodes[child].text = name;
                builder.nodes[child].target = value;
            }
            RestructuredTextNodeKind::CrossReference
            | RestructuredTextNodeKind::FootnoteReference
            | RestructuredTextNodeKind::CitationReference
            | RestructuredTextNodeKind::SubstitutionReference => {
                builder.nodes[child].name = name;
            }
            _ => builder.nodes[child].text = value,
        }
        builder.link_child(parent, child);
    }
}

fn known_role(role: &str) -> bool {
    matches!(
        role.to_ascii_lowercase().as_str(),
        "abbreviation"
            | "acronym"
            | "code"
            | "emphasis"
            | "index"
            | "literal"
            | "math"
            | "pep-reference"
            | "raw"
            | "rfc-reference"
            | "strong"
            | "subscript"
            | "superscript"
            | "title-reference"
    )
}

#[derive(Debug, Clone)]
struct ParsedTableRow {
    range: Range<usize>,
    header: bool,
    cells: Vec<(Range<usize>, String)>,
}

fn grid_table_start(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('+')
        && trimmed.ends_with('+')
        && trimmed.chars().filter(|value| *value == '+').count() >= 3
        && trimmed
            .chars()
            .all(|value| matches!(value, '+' | '-' | '=' | ':' | ' '))
}

fn grid_table_end(lines: &[SourceLine<'_>], start: usize) -> usize {
    let mut end = start;
    while end < lines.len() {
        let trimmed = lines[end].text.trim();
        if trimmed.starts_with('|') || grid_table_start(trimmed) {
            end += 1;
        } else {
            break;
        }
    }
    end.max(start + 1)
}

fn parse_grid_table(
    lines: &[SourceLine<'_>],
    start: usize,
    end: usize,
) -> Option<Vec<ParsedTableRow>> {
    let border = lines[start].text.trim();
    let columns = border
        .char_indices()
        .filter_map(|(offset, value)| (value == '+').then_some(offset))
        .collect::<Vec<_>>();
    if columns.len() < 3 {
        return None;
    }
    let indent = lines[start].text.find('+')?;
    let mut rows = Vec::new();
    let mut header_next = false;
    for line in lines.iter().take(end).skip(start + 1) {
        let trimmed = line.text.trim();
        if grid_table_start(trimmed) {
            header_next |= trimmed.contains('=');
            continue;
        }
        if !trimmed.starts_with('|') {
            return None;
        }
        let mut cells = Vec::new();
        for window in columns.windows(2) {
            let left = indent + window[0] + 1;
            let right = indent + window[1];
            if right > line.text.len() || left > right {
                return None;
            }
            cells.push(cell_from_slice(line, left, right));
        }
        rows.push(ParsedTableRow {
            range: line.start..line.end,
            header: header_next || rows.is_empty(),
            cells,
        });
        header_next = false;
    }
    (!rows.is_empty()).then_some(rows)
}

fn cell_from_slice(line: &SourceLine<'_>, left: usize, right: usize) -> (Range<usize>, String) {
    let raw = &line.text[left..right];
    let leading = raw.len() - raw.trim_start().len();
    let trailing = raw.len() - raw.trim_end().len();
    let start = line.start + left + leading;
    let end = line.start + right.saturating_sub(trailing);
    (start..end.max(start), raw.trim().to_string())
}

fn simple_table_delimiter(line: &str) -> Option<Vec<(usize, usize)>> {
    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len() && matches!(bytes[cursor], b'=' | b'-') {
            cursor += 1;
        }
        if cursor > start {
            spans.push((start, cursor));
        } else if cursor < bytes.len() {
            return None;
        }
    }
    (spans.len() >= 2).then_some(spans)
}

fn simple_table_end(lines: &[SourceLine<'_>], start: usize) -> usize {
    let mut end = start + 1;
    while end < lines.len() && !lines[end].text.trim().is_empty() {
        end += 1;
    }
    end
}

fn parse_simple_table(
    lines: &[SourceLine<'_>],
    start: usize,
    end: usize,
) -> Option<Vec<ParsedTableRow>> {
    let spans = simple_table_delimiter(lines[start].text)?;
    let mut rows = Vec::new();
    let mut header = true;
    for line in lines.iter().take(end).skip(start + 1) {
        if simple_table_delimiter(line.text).is_some() {
            header = false;
            continue;
        }
        let mut cells = Vec::new();
        for (column, (left, right)) in spans.iter().enumerate() {
            let right = if column + 1 == spans.len() {
                line.text.len()
            } else {
                (*right).min(line.text.len())
            };
            cells.push(cell_from_slice(line, (*left).min(right), right));
        }
        rows.push(ParsedTableRow {
            range: line.start..line.end,
            header,
            cells,
        });
    }
    (!rows.is_empty()).then_some(rows)
}

#[allow(clippy::too_many_arguments)]
fn handle_table(
    lines: &[SourceLine<'_>],
    start: usize,
    end: usize,
    parent: Option<usize>,
    builder: &mut NodeBuilder<'_>,
    diagnostics: &mut Vec<Diagnostic>,
    style: RestructuredTextTableStyle,
) -> bool {
    let rows = match style {
        RestructuredTextTableStyle::Grid => parse_grid_table(lines, start, end),
        RestructuredTextTableStyle::Simple => parse_simple_table(lines, start, end),
    };
    let Some(rows) = rows else {
        let node = builder.push(
            RestructuredTextNodeKind::RawBlock,
            lines[start].start..lines[end - 1].end,
            parent.map(|value| builder.nodes[value].id.clone()),
        );
        attach_to_parent(builder, parent, node);
        diagnostics.push(
            Diagnostic::warning(
                "grist.restructured_text.table",
                "table.malformed",
                "malformed reStructuredText table was retained as raw source",
            )
            .with_range(builder.nodes[node].range.clone())
            .with_locator(builder.nodes[node].locator.clone())
            .partial(),
        );
        return false;
    };
    let details = table_details(&rows, &builder.index);
    let table = builder.push(
        RestructuredTextNodeKind::Table,
        lines[start].start..lines[end - 1].end,
        parent.map(|value| builder.nodes[value].id.clone()),
    );
    builder.nodes[table].table = Some(RestructuredTextTable {
        style,
        rows: details,
    });
    attach_to_parent(builder, parent, table);
    add_table_nodes(builder, table, rows);
    true
}

fn table_details(rows: &[ParsedTableRow], index: &LineIndex) -> Vec<RestructuredTextTableRow> {
    rows.iter()
        .map(|row| {
            let range = SourceRange::new(row.range.start, row.range.end, index);
            RestructuredTextTableRow {
                locator: SourceLocator::exact(range.clone()).expect("table row range is exact"),
                range,
                header: row.header,
                cells: row
                    .cells
                    .iter()
                    .map(|(cell, text)| {
                        let range = SourceRange::new(cell.start, cell.end, index);
                        RestructuredTextTableCell {
                            locator: SourceLocator::exact(range.clone())
                                .expect("table cell range is exact"),
                            range,
                            text: text.clone(),
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

fn add_table_nodes(builder: &mut NodeBuilder<'_>, table: usize, rows: Vec<ParsedTableRow>) {
    for row in rows {
        let row_node = builder.push(
            RestructuredTextNodeKind::TableRow,
            row.range,
            Some(builder.nodes[table].id.clone()),
        );
        builder.nodes[row_node].known_syntax = row.header;
        builder.link_child(table, row_node);
        for (range, text) in row.cells {
            let cell = builder.push(
                RestructuredTextNodeKind::TableCell,
                range,
                Some(builder.nodes[row_node].id.clone()),
            );
            builder.nodes[cell].text = Some(text);
            builder.link_child(row_node, cell);
        }
    }
}

fn is_transition(line: &str) -> bool {
    adornment(line.trim()).is_some() && line.trim().chars().count() >= 4
}

fn list_marker(line: &str) -> Option<(usize, bool)> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    for marker in ["- ", "+ ", "* "] {
        if trimmed.starts_with(marker) {
            return Some((indent + marker.len(), false));
        }
    }
    let token_end = trimmed.find(char::is_whitespace)?;
    let token = &trimmed[..token_end];
    let ordered = token == "#."
        || token
            .trim_matches(['(', ')', '.'])
            .chars()
            .all(|value| value.is_ascii_digit());
    ordered.then_some((indent + token_end + 1, true))
}

fn list_extent(lines: &[SourceLine<'_>], start: usize) -> (usize, Vec<(usize, usize, usize)>) {
    let base_indent = indentation(lines[start].text);
    let mut cursor = start;
    let mut items = Vec::new();
    while cursor < lines.len() {
        let Some((marker_len, _)) = list_marker(lines[cursor].text) else {
            break;
        };
        if indentation(lines[cursor].text) != base_indent {
            break;
        }
        let item_start = cursor;
        cursor += 1;
        while cursor < lines.len() {
            if lines[cursor].text.trim().is_empty() || indentation(lines[cursor].text) > base_indent
            {
                cursor += 1;
            } else {
                break;
            }
        }
        items.push((item_start, cursor, marker_len));
    }
    (cursor, items)
}

fn field_marker(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix(':')?;
    let close = rest.find(':')?;
    let name = rest[..close].trim();
    if name.is_empty() || name.contains('`') {
        return None;
    }
    Some((name.to_string(), rest[close + 1..].trim().to_string()))
}

fn field_list_extent(
    lines: &[SourceLine<'_>],
    start: usize,
) -> (usize, Vec<(usize, String, String)>) {
    let indent = indentation(lines[start].text);
    let mut cursor = start;
    let mut fields = Vec::new();
    while cursor < lines.len() && indentation(lines[cursor].text) == indent {
        let Some((name, value)) = field_marker(lines[cursor].text) else {
            break;
        };
        fields.push((cursor, name, value));
        cursor += 1;
    }
    (cursor, fields)
}

fn doctest_extent(lines: &[SourceLine<'_>], start: usize) -> usize {
    let mut cursor = start + 1;
    while cursor < lines.len() && !lines[cursor].text.trim().is_empty() {
        cursor += 1;
    }
    cursor
}

fn paragraph_extent(lines: &[SourceLine<'_>], start: usize) -> usize {
    let mut cursor = start + 1;
    while cursor < lines.len() {
        if lines[cursor].text.trim().is_empty()
            || lines[cursor].text.trim_start().starts_with(".. ")
            || grid_table_start(lines[cursor].text)
            || list_marker(lines[cursor].text).is_some()
            || field_marker(lines[cursor].text).is_some()
            || heading_at(lines, cursor).is_some()
        {
            break;
        }
        cursor += 1;
    }
    cursor
}

fn indented_extent(lines: &[SourceLine<'_>], start: usize) -> usize {
    let base = indentation(lines[start].text);
    let mut cursor = start + 1;
    while cursor < lines.len() {
        if lines[cursor].text.trim().is_empty() || indentation(lines[cursor].text) >= base {
            cursor += 1;
        } else {
            break;
        }
    }
    cursor
}

fn indentation(line: &str) -> usize {
    line.bytes()
        .take_while(|value| matches!(value, b' ' | b'\t'))
        .count()
}

fn dedent(lines: &[SourceLine<'_>]) -> String {
    let indent = lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .map(|line| indentation(line.text))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|line| line.text[indent.min(line.text.len())..].to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn nonempty(value: impl Into<String>) -> Option<String> {
    let value = value.into();
    (!value.is_empty()).then_some(value)
}

struct IncludeResolver {
    root: Option<PathBuf>,
    root_error: Option<String>,
    visited: BTreeSet<PathBuf>,
}

impl IncludeResolver {
    fn new(source: &SourceInfo, options: &RestructuredTextOptions) -> Self {
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
        let mut visited = BTreeSet::new();
        if let Some(path) = source.path.as_deref().map(PathBuf::from)
            && let Ok(canonical) = fs::canonicalize(path)
        {
            visited.insert(canonical);
        }
        Self {
            root,
            root_error,
            visited,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &mut self,
        target: String,
        range: &SourceRange,
        locator: &SourceLocator,
        source: &SourceInfo,
        options: &RestructuredTextOptions,
        depth: u16,
    ) -> (IncludeReference, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let unresolved = |status| IncludeReference {
            target: target.clone(),
            status,
            resolved_path: None,
            resolved: None,
        };
        let diagnostic = |code: &str, message: String| {
            Diagnostic::warning("grist.restructured_text.include", code, message)
                .with_range(range.clone())
                .with_locator(locator.clone())
                .partial()
        };
        if !options.resolve_includes {
            diagnostics.push(diagnostic(
                "include.resolution_disabled",
                "include was retained because local resolution is disabled".to_string(),
            ));
            return (unresolved(IncludeStatus::ReferenceOnly), diagnostics);
        }
        if looks_remote(&target) {
            diagnostics.push(diagnostic(
                "include.remote_disabled",
                format!("remote include `{target}` was retained without a network request"),
            ));
            return (unresolved(IncludeStatus::RemoteDisabled), diagnostics);
        }
        let Some(root) = self.root.clone() else {
            diagnostics.push(diagnostic(
                "include.project_root_required",
                self.root_error.clone().unwrap_or_else(|| {
                    "include was retained because no explicit project root was supplied".to_string()
                }),
            ));
            return (unresolved(IncludeStatus::ReferenceOnly), diagnostics);
        };
        if depth >= options.max_include_depth {
            diagnostics.push(diagnostic(
                "include.depth_exceeded",
                format!(
                    "include depth exceeds the configured limit {}",
                    options.max_include_depth
                ),
            ));
            return (unresolved(IncludeStatus::DepthExceeded), diagnostics);
        }
        if target.trim().is_empty() || contains_parent_component(Path::new(&target)) {
            diagnostics.push(diagnostic(
                "include.outside_project_root",
                format!("include target `{target}` is not a safe project-relative reference"),
            ));
            return (unresolved(IncludeStatus::OutsideProjectRoot), diagnostics);
        }
        let base = source
            .path
            .as_deref()
            .and_then(|path| fs::canonicalize(path).ok())
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .filter(|path| path.starts_with(&root))
            .unwrap_or_else(|| root.clone());
        let candidate = if Path::new(&target).is_absolute() {
            PathBuf::from(&target)
        } else {
            base.join(&target)
        };
        let canonical = match fs::canonicalize(&candidate) {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(diagnostic(
                    "include.not_found",
                    format!("include `{target}` could not be opened: {error}"),
                ));
                return (unresolved(IncludeStatus::Missing), diagnostics);
            }
        };
        if !canonical.starts_with(&root) {
            diagnostics.push(diagnostic(
                "include.outside_project_root",
                format!("include `{target}` resolves outside the supplied project root"),
            ));
            return (unresolved(IncludeStatus::OutsideProjectRoot), diagnostics);
        }
        if !canonical.is_file() {
            diagnostics.push(diagnostic(
                "include.not_file",
                format!("include `{target}` does not resolve to a regular file"),
            ));
            return (unresolved(IncludeStatus::NotFile), diagnostics);
        }
        let relative = canonical
            .strip_prefix(&root)
            .unwrap_or(&canonical)
            .to_string_lossy()
            .replace('\\', "/");
        if !self.visited.insert(canonical.clone()) {
            diagnostics.push(diagnostic(
                "include.cycle",
                format!("include cycle detected at `{relative}`"),
            ));
            let mut reference = unresolved(IncludeStatus::Cycle);
            reference.resolved_path = Some(relative);
            return (reference, diagnostics);
        }
        let metadata = match fs::metadata(&canonical) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.visited.remove(&canonical);
                diagnostics.push(diagnostic(
                    "include.not_found",
                    format!("include `{relative}` metadata could not be read: {error}"),
                ));
                return (unresolved(IncludeStatus::Missing), diagnostics);
            }
        };
        if metadata.len() > options.max_include_bytes {
            self.visited.remove(&canonical);
            diagnostics.push(diagnostic(
                "include.budget_exceeded",
                format!(
                    "include `{relative}` is {} bytes, above the configured {} byte limit",
                    metadata.len(),
                    options.max_include_bytes
                ),
            ));
            let mut reference = unresolved(IncludeStatus::BudgetExceeded);
            reference.resolved_path = Some(relative);
            return (reference, diagnostics);
        }
        let bytes = match fs::read(&canonical) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.visited.remove(&canonical);
                diagnostics.push(diagnostic(
                    "include.not_found",
                    format!("include `{relative}` could not be read: {error}"),
                ));
                return (unresolved(IncludeStatus::Missing), diagnostics);
            }
        };
        let mut decode_options =
            DecodeOptions::for_media_type(Some("text/x-rst"), Some("restructured_text"));
        decode_options.context = DecodeContext::PlainText;
        if let Some(encoding) = &options.encoding {
            decode_options.transport_encoding = Some(encoding.clone());
        }
        let decoded = match decode_text(&bytes, &decode_options) {
            Ok(decoded) => decoded,
            Err(error) => {
                self.visited.remove(&canonical);
                diagnostics.push(
                    error
                        .diagnostic()
                        .with_parser("grist.restructured_text")
                        .with_range(range.clone())
                        .with_locator(locator.clone())
                        .partial(),
                );
                let mut reference = unresolved(IncludeStatus::DecodeFailed);
                reference.resolved_path = Some(relative);
                return (reference, diagnostics);
            }
        };
        let child_source = SourceInfo::new(
            canonical
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| relative.clone()),
        )
        .with_path(&canonical)
        .with_repository_relative_path(relative.clone())
        .with_parent(source.clone());
        let (nodes, mut child_diagnostics) =
            parse_nodes(&decoded, &child_source, options, depth + 1, self);
        diagnostics.append(&mut child_diagnostics);
        diagnostics.extend(decoded.report.diagnostics.clone());
        self.visited.remove(&canonical);
        (
            IncludeReference {
                target,
                status: IncludeStatus::Resolved,
                resolved_path: Some(relative),
                resolved: Some(ResolvedInclude {
                    source: child_source,
                    raw_bytes: bytes.clone(),
                    content_sha256: sha256_hex(&bytes),
                    decoded_text: decoded.text,
                    encoding: decoded.report.encoding.clone(),
                    decoding: decoded.report,
                    nodes,
                }),
            },
            diagnostics,
        )
    }
}

fn looks_remote(target: &str) -> bool {
    let lower = target.trim().to_ascii_lowercase();
    lower.contains("://")
        || lower.starts_with("mailto:")
        || lower.starts_with("data:")
        || lower.starts_with("file:")
        || lower.starts_with("//")
}

fn contains_parent_component(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RICH: &str = "Title\n=====\n\n.. warning:: inert\n\nParagraph :code:`x` with [1]_ and `target`_.\n\n.. [1] note\n.. _target: https://example.test\n\n+---+---+\n| A | B |\n+===+===+\n| 1 | 2 |\n+---+---+\n";

    #[test]
    fn parses_core_constructs_with_exact_locations() {
        let envelope = parse_restructured_text(RICH, SourceInfo::stdin("demo.rst"));
        assert_eq!(envelope.status, OperationStatus::Complete);
        let document = envelope.payload.unwrap();
        for kind in [
            RestructuredTextNodeKind::Heading,
            RestructuredTextNodeKind::Directive,
            RestructuredTextNodeKind::Role,
            RestructuredTextNodeKind::FootnoteReference,
            RestructuredTextNodeKind::FootnoteDefinition,
            RestructuredTextNodeKind::Target,
            RestructuredTextNodeKind::Table,
        ] {
            assert!(
                document.nodes.iter().any(|node| node.kind == kind),
                "{kind:?}"
            );
        }
        assert!(
            document
                .nodes
                .iter()
                .all(|node| node.locator.validate().is_ok())
        );
    }

    #[test]
    fn include_without_root_is_retained_and_partial() {
        let envelope =
            parse_restructured_text(".. include:: child.rst\n", SourceInfo::stdin("root.rst"));
        assert_eq!(envelope.status, OperationStatus::Partial);
        let include = envelope.payload.unwrap().nodes[0].include.clone().unwrap();
        assert_eq!(include.status, IncludeStatus::ReferenceOnly);
        assert!(include.resolved.is_none());
        assert!(
            envelope
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "include.project_root_required" })
        );
    }

    #[test]
    fn include_resolution_is_recursive_and_root_bounded() {
        let base = std::env::temp_dir().join(format!("grist-rst-{}", std::process::id()));
        let root = base.join("project");
        fs::create_dir_all(&root).unwrap();
        let main = root.join("main.rst");
        let child = root.join("child.rst");
        let outside = base.join("outside.rst");
        fs::write(&main, ".. include:: child.rst\n").unwrap();
        fs::write(&child, "Child\n-----\n").unwrap();
        fs::write(&outside, "outside").unwrap();
        let options = RestructuredTextOptions {
            project_root: Some(root.clone()),
            ..Default::default()
        };
        let resolved = parse_restructured_text_with_options(
            ".. include:: child.rst\n",
            SourceInfo::from_path(&main),
            &options,
        );
        assert_eq!(resolved.status, OperationStatus::Complete);
        let include = resolved.payload.unwrap().nodes[0].include.clone().unwrap();
        assert_eq!(include.status, IncludeStatus::Resolved);
        assert_eq!(include.resolved_path.as_deref(), Some("child.rst"));
        assert!(
            include
                .resolved
                .unwrap()
                .nodes
                .iter()
                .any(|node| { node.kind == RestructuredTextNodeKind::Heading })
        );
        let rejected = parse_restructured_text_with_options(
            ".. include:: ../outside.rst\n",
            SourceInfo::from_path(&main),
            &options,
        );
        assert_eq!(rejected.status, OperationStatus::Partial);
        assert_eq!(
            rejected.payload.unwrap().nodes[0]
                .include
                .as_ref()
                .unwrap()
                .status,
            IncludeStatus::OutsideProjectRoot
        );
        fs::remove_dir_all(base).unwrap();
    }
}
