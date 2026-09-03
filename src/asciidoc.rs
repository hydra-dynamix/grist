//! Inert, source-preserving AsciiDoc parser.
//!
//! Directives never execute and remote resources are never fetched. Local
//! `include::` directives are resolved only below a caller-supplied project
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
pub struct AsciiDocDocument {
    pub schema_version: String,
    pub raw_bytes: Vec<u8>,
    pub raw_range: RawByteRange,
    pub decoded_text: String,
    pub decoded_range: SourceRange,
    pub locator: SourceLocator,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub nodes: Vec<AsciiDocNode>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AsciiDocNode {
    pub id: String,
    pub kind: AsciiDocNodeKind,
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
    pub table: Option<AsciiDocTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<IncludeReference>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub known_syntax: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AsciiDocNodeKind {
    Heading,
    Paragraph,
    Attribute,
    BlockAttribute,
    Directive,
    Role,
    Include,
    CodeBlock,
    LiteralBlock,
    Table,
    TableRow,
    TableCell,
    FootnoteDefinition,
    FootnoteReference,
    Target,
    CrossReference,
    Hyperlink,
    List,
    ListItem,
    Emphasis,
    Strong,
    InlineCode,
    Transition,
    Comment,
    RawBlock,
    RawInline,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AsciiDocTable {
    pub style: AsciiDocTableStyle,
    pub rows: Vec<AsciiDocTableRow>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AsciiDocTableStyle {
    Pipe,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AsciiDocTableRow {
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub header: bool,
    pub cells: Vec<AsciiDocTableCell>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AsciiDocTableCell {
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
    pub nodes: Vec<AsciiDocNode>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AsciiDocOptions {
    pub encoding: Option<String>,
    /// Explicit filesystem boundary. No root means includes remain references.
    pub project_root: Option<PathBuf>,
    pub resolve_includes: bool,
    pub max_include_depth: u16,
    pub max_include_bytes: u64,
    pub retain_comments: bool,
}

impl Default for AsciiDocOptions {
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

impl crate::core::FormatOptions for AsciiDocOptions {
    const FORMAT: &'static str = "asciidoc";
}

pub type AsciiDocEnvelope = Envelope<AsciiDocDocument>;

pub fn parse_asciidoc(text: &str, source: SourceInfo) -> AsciiDocEnvelope {
    parse_asciidoc_with_options(text, source, &AsciiDocOptions::default())
}

pub fn parse_asciidoc_with_options(
    text: &str,
    source: SourceInfo,
    options: &AsciiDocOptions,
) -> AsciiDocEnvelope {
    parse_asciidoc_bytes(text.as_bytes(), source, options)
}

pub fn parse_asciidoc_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &AsciiDocOptions,
) -> AsciiDocEnvelope {
    let mut decode_options =
        DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("asciidoc"));
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
    ParserInfo::new("grist.asciidoc")
        .with_implementation("grist-asciidoc", env!("CARGO_PKG_VERSION"))
        .with_specification_version("AsciiDoc language; inert macros and attributes")
        .with_feature("asciidoc")
}

fn envelope_from_decoded(
    decoded: &DecodedText,
    source: SourceInfo,
    options: &AsciiDocOptions,
) -> AsciiDocEnvelope {
    let (payload, mut diagnostics) = document_from_decoded(decoded, &source, options);
    let partial = decoded.report.makes_operation_partial()
        || diagnostics.iter().any(|diagnostic| diagnostic.partial);
    let mut all_diagnostics = decoded.report.diagnostics.clone();
    all_diagnostics.append(&mut diagnostics);
    let digest = options_digest(options).expect("AsciiDoc options serialize");
    let mut envelope = if partial {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::AsciiDoc,
            source,
            parser_info(),
            digest,
            SchemaVersion::ASCIIDOC_V1,
            Some(payload),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::AsciiDoc,
            source,
            parser_info(),
            digest,
            SchemaVersion::ASCIIDOC_V1,
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
                .with_format(FormatIdentity::new("asciidoc", Some("text/asciidoc"))),
        )
        .with_canonical_payload_identity()
        .expect("AsciiDoc payload canonicalization is infallible")
}

fn failed_decode_envelope(
    bytes: &[u8],
    source: SourceInfo,
    options: &AsciiDocOptions,
    error: DecodeError,
) -> AsciiDocEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::AsciiDoc,
        OperationStatus::Failed,
        source,
        parser_info(),
        options_digest(options).expect("AsciiDoc options serialize"),
        SchemaVersion::ASCIIDOC_V1,
    )
    .expect("failed AsciiDoc decode has valid envelope status")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes)
            .with_format(FormatIdentity::new("asciidoc", Some("text/asciidoc"))),
    )
    .with_diagnostics(vec![error.diagnostic().with_parser("grist.asciidoc")])
}

pub(crate) fn document_from_decoded(
    decoded: &DecodedText,
    source: &SourceInfo,
    options: &AsciiDocOptions,
) -> (AsciiDocDocument, Vec<Diagnostic>) {
    let mut resolver = IncludeResolver::new(source, options);
    let (nodes, diagnostics) = parse_nodes(decoded, source, options, 0, &mut resolver);
    let index = LineIndex::new(&decoded.text);
    let decoded_range = SourceRange::new(0, decoded.text.len(), &index);
    let locator =
        SourceLocator::exact(decoded_range.clone()).expect("whole AsciiDoc source range is exact");
    let document = AsciiDocDocument {
        schema_version: SchemaVersion::ASCIIDOC_V1.to_string(),
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

pub(crate) fn payload_node_count(document: &AsciiDocDocument) -> usize {
    fn count(nodes: &[AsciiDocNode]) -> usize {
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

pub(crate) fn payload_decoded_char_count(document: &AsciiDocDocument) -> usize {
    fn included(nodes: &[AsciiDocNode]) -> usize {
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
    nodes: Vec<AsciiDocNode>,
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
        kind: AsciiDocNodeKind,
        range: Range<usize>,
        parent_id: Option<String>,
    ) -> usize {
        let source_range = SourceRange::new(range.start, range.end, &self.index);
        let locator = SourceLocator::exact(source_range.clone())
            .expect("parser-created AsciiDoc range is exact");
        let raw_range = self.decoded.raw_range_for_decoded(DecodedByteRange {
            start: range.start as u64,
            end: range.end as u64,
        });
        let id = format!("adoc-node-{:06}", self.nodes.len());
        self.nodes.push(AsciiDocNode {
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
    options: &AsciiDocOptions,
    depth: u16,
    resolver: &mut IncludeResolver,
) -> (Vec<AsciiDocNode>, Vec<Diagnostic>) {
    let lines = source_lines(&decoded.text);
    let mut builder = NodeBuilder::new(decoded);
    let mut diagnostics = Vec::new();
    let mut heading_stack = Vec::<(u8, usize)>::new();
    let mut current_parent = None;
    let mut cursor = 0;

    while cursor < lines.len() {
        if lines[cursor].text.trim().is_empty() {
            cursor += 1;
            continue;
        }
        if let Some((level, title)) = heading_at(lines[cursor].text) {
            while heading_stack.last().is_some_and(|(open, _)| *open >= level) {
                heading_stack.pop();
            }
            let parent = heading_stack.last().map(|(_, index)| *index);
            let node = builder.push(
                AsciiDocNodeKind::Heading,
                lines[cursor].start..lines[cursor].end,
                parent.map(|value| builder.nodes[value].id.clone()),
            );
            builder.nodes[node].text = Some(title);
            builder.nodes[node].level = Some(level);
            builder.nodes[node].known_syntax = true;
            attach_to_parent(&mut builder, parent, node);
            heading_stack.push((level, node));
            current_parent = Some(node);
            cursor += 1;
            continue;
        }
        if let Some((name, value)) = document_attribute(lines[cursor].text) {
            let node = builder.push(
                AsciiDocNodeKind::Attribute,
                lines[cursor].start..lines[cursor].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            builder.nodes[node].name = Some(name);
            builder.nodes[node].text = value;
            builder.nodes[node].known_syntax = true;
            attach_to_parent(&mut builder, current_parent, node);
            cursor += 1;
            continue;
        }
        if is_comment_start(lines[cursor].text) {
            let end = comment_extent(&lines, cursor);
            if options.retain_comments {
                let node = builder.push(
                    AsciiDocNodeKind::Comment,
                    lines[cursor].start..lines[end - 1].end,
                    current_parent.map(|value| builder.nodes[value].id.clone()),
                );
                attach_to_parent(&mut builder, current_parent, node);
            }
            cursor = end;
            continue;
        }
        if let Some(attributes) = block_attribute(lines[cursor].text) {
            if cursor + 1 < lines.len()
                && let Some(delimiter) = block_delimiter(lines[cursor + 1].text)
            {
                let (end, closed) = delimited_extent(&lines, cursor + 1, delimiter);
                let style = attributes.first().map(String::as_str).unwrap_or_default();
                let kind = if matches!(style, "source" | "listing") {
                    AsciiDocNodeKind::CodeBlock
                } else if matches!(style, "literal" | "verse") {
                    AsciiDocNodeKind::LiteralBlock
                } else {
                    AsciiDocNodeKind::RawBlock
                };
                let attribute = builder.push(
                    AsciiDocNodeKind::BlockAttribute,
                    lines[cursor].start..lines[cursor].end,
                    current_parent.map(|value| builder.nodes[value].id.clone()),
                );
                builder.nodes[attribute].argument = Some(attributes.join(","));
                builder.nodes[attribute].options = positional_options(&attributes);
                builder.nodes[attribute].role = role_from_attributes(&attributes);
                builder.nodes[attribute].known_syntax = true;
                attach_to_parent(&mut builder, current_parent, attribute);
                let node = builder.push(
                    kind,
                    lines[cursor].start..lines[end - 1].end,
                    current_parent.map(|value| builder.nodes[value].id.clone()),
                );
                builder.nodes[node].name = nonempty(style);
                builder.nodes[node].role = attributes.get(1).cloned().and_then(nonempty);
                builder.nodes[node].options = positional_options(&attributes);
                builder.nodes[node].text = Some(delimited_body(&lines, cursor + 1, end, closed));
                builder.nodes[node].known_syntax = matches!(
                    style,
                    "source"
                        | "listing"
                        | "literal"
                        | "verse"
                        | "example"
                        | "quote"
                        | "sidebar"
                        | "open"
                        | "pass"
                );
                attach_to_parent(&mut builder, current_parent, node);
                if !closed {
                    diagnostics.push(raw_diagnostic(
                        &builder.nodes[node],
                        "block.unclosed",
                        "unclosed AsciiDoc delimited block was retained",
                    ));
                }
                cursor = end;
                continue;
            }
            let node = builder.push(
                AsciiDocNodeKind::BlockAttribute,
                lines[cursor].start..lines[cursor].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            builder.nodes[node].argument = Some(attributes.join(","));
            builder.nodes[node].options = positional_options(&attributes);
            builder.nodes[node].role = role_from_attributes(&attributes);
            builder.nodes[node].known_syntax = true;
            attach_to_parent(&mut builder, current_parent, node);
            cursor += 1;
            continue;
        }
        if let Some((name, target, macro_options)) = block_macro(lines[cursor].text) {
            let lower = name.to_ascii_lowercase();
            let kind = if lower == "include" {
                AsciiDocNodeKind::Include
            } else if known_directive(&lower) {
                AsciiDocNodeKind::Directive
            } else {
                AsciiDocNodeKind::RawBlock
            };
            let node = builder.push(
                kind,
                lines[cursor].start..lines[cursor].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            builder.nodes[node].name = Some(name.clone());
            builder.nodes[node].argument = nonempty(target.clone());
            builder.nodes[node].options = parse_attribute_options(&macro_options);
            builder.nodes[node].known_syntax = lower == "include" || known_directive(&lower);
            attach_to_parent(&mut builder, current_parent, node);
            if lower == "include" {
                let (include, mut include_diagnostics) = resolver.resolve(
                    target,
                    &builder.nodes[node].range,
                    &builder.nodes[node].locator,
                    source,
                    options,
                    depth,
                );
                builder.nodes[node].include = Some(include);
                diagnostics.append(&mut include_diagnostics);
            } else if !known_directive(&lower) {
                diagnostics.push(
                    Diagnostic::info(
                        "grist.asciidoc.directive",
                        "directive.unknown_inert",
                        format!("unknown block macro `{name}` was retained without execution"),
                    )
                    .with_range(builder.nodes[node].range.clone())
                    .with_locator(builder.nodes[node].locator.clone()),
                );
            }
            cursor += 1;
            continue;
        }
        if lines[cursor].text.trim() == "|===" {
            let end = table_extent(&lines, cursor);
            handle_table(
                &lines,
                cursor,
                end,
                current_parent,
                &mut builder,
                &mut diagnostics,
            );
            cursor = end;
            continue;
        }
        if let Some(delimiter) = block_delimiter(lines[cursor].text) {
            let (end, closed) = delimited_extent(&lines, cursor, delimiter);
            let kind = match delimiter {
                "----" => AsciiDocNodeKind::CodeBlock,
                "...." => AsciiDocNodeKind::LiteralBlock,
                _ => AsciiDocNodeKind::RawBlock,
            };
            let node = builder.push(
                kind,
                lines[cursor].start..lines[end - 1].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            builder.nodes[node].text = Some(delimited_body(&lines, cursor, end, closed));
            builder.nodes[node].known_syntax = delimiter != "++++";
            attach_to_parent(&mut builder, current_parent, node);
            if !closed {
                diagnostics.push(raw_diagnostic(
                    &builder.nodes[node],
                    "block.unclosed",
                    "unclosed AsciiDoc delimited block was retained",
                ));
            }
            cursor = end;
            continue;
        }
        if let Some(name) = anchor(lines[cursor].text) {
            let node = builder.push(
                AsciiDocNodeKind::Target,
                lines[cursor].start..lines[cursor].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            builder.nodes[node].name = Some(name);
            builder.nodes[node].known_syntax = true;
            attach_to_parent(&mut builder, current_parent, node);
            cursor += 1;
            continue;
        }
        if lines[cursor].text.trim() == "'''" {
            let node = builder.push(
                AsciiDocNodeKind::Transition,
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
                AsciiDocNodeKind::List,
                lines[cursor].start..lines[end - 1].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            attach_to_parent(&mut builder, current_parent, list);
            for (item_start, item_end, marker_len) in items {
                let item = builder.push(
                    AsciiDocNodeKind::ListItem,
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
        if lines[cursor].text.trim_start().starts_with('[')
            && !lines[cursor].text.trim_end().ends_with(']')
        {
            let node = builder.push(
                AsciiDocNodeKind::RawBlock,
                lines[cursor].start..lines[cursor].end,
                current_parent.map(|value| builder.nodes[value].id.clone()),
            );
            attach_to_parent(&mut builder, current_parent, node);
            diagnostics.push(raw_diagnostic(
                &builder.nodes[node],
                "block_attribute.malformed",
                "malformed AsciiDoc block attribute was retained as raw source",
            ));
            cursor += 1;
            continue;
        }
        let paragraph_end = paragraph_extent(&lines, cursor);
        let paragraph = builder.push(
            AsciiDocNodeKind::Paragraph,
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
        cursor = paragraph_end;
    }
    (builder.nodes, diagnostics)
}

fn heading_at(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim();
    let marker_end = trimmed.find(' ')?;
    let marker = &trimmed[..marker_end];
    if marker.is_empty() || marker.len() > 6 || !marker.bytes().all(|value| value == b'=') {
        return None;
    }
    let title = trimmed[marker_end + 1..].trim();
    (!title.is_empty()).then(|| (marker.len() as u8, title.to_string()))
}

fn document_attribute(line: &str) -> Option<(String, Option<String>)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix(':')?;
    let close = rest.find(':')?;
    let mut name = rest[..close].trim().to_string();
    if name.is_empty()
        || !name
            .trim_end_matches('!')
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
    {
        return None;
    }
    let unset = name.ends_with('!');
    if unset {
        name.pop();
    }
    let value = if unset {
        None
    } else {
        nonempty(rest[close + 1..].trim())
    };
    Some((name, value))
}

fn block_attribute(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    let body = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    if body.is_empty() || body.starts_with('[') {
        return None;
    }
    Some(split_attributes(body))
}

fn split_attributes(value: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for character in value.chars() {
        match (character, quote) {
            ('\'' | '"', None) => {
                quote = Some(character);
                current.push(character);
            }
            (value, Some(open)) if value == open => {
                quote = None;
                current.push(value);
            }
            (',', None) => {
                fields.push(current.trim().trim_matches(['\'', '"']).to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    fields.push(current.trim().trim_matches(['\'', '"']).to_string());
    fields
}

fn positional_options(attributes: &[String]) -> BTreeMap<String, String> {
    attributes
        .iter()
        .enumerate()
        .map(|(index, value)| (index.to_string(), value.clone()))
        .collect()
}

fn parse_attribute_options(value: &str) -> BTreeMap<String, String> {
    split_attributes(value)
        .into_iter()
        .enumerate()
        .map(|(index, field)| {
            field
                .split_once('=')
                .map(|(name, value)| {
                    (
                        name.trim().to_string(),
                        value.trim().trim_matches(['\'', '"']).to_string(),
                    )
                })
                .unwrap_or_else(|| (index.to_string(), field))
        })
        .collect()
}

fn role_from_attributes(attributes: &[String]) -> Option<String> {
    attributes.iter().find_map(|attribute| {
        attribute
            .strip_prefix('.')
            .or_else(|| attribute.strip_prefix("role="))
            .map(|value| value.trim_matches(['\'', '"']).to_string())
    })
}

fn block_macro(line: &str) -> Option<(String, String, String)> {
    let trimmed = line.trim();
    let (name, rest) = trimmed.split_once("::")?;
    if name.is_empty()
        || !name
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
    {
        return None;
    }
    let open = rest.rfind('[')?;
    let options = rest.get(open + 1..)?.strip_suffix(']')?;
    Some((
        name.to_string(),
        rest[..open].trim().to_string(),
        options.to_string(),
    ))
}

fn known_directive(name: &str) -> bool {
    matches!(
        name,
        "audio" | "icon" | "image" | "kbd" | "link" | "mailto" | "menu" | "pass" | "video"
    )
}

fn block_delimiter(line: &str) -> Option<&str> {
    match line.trim() {
        "----" => Some("----"),
        "...." => Some("...."),
        "++++" => Some("++++"),
        "====" => Some("===="),
        "****" => Some("****"),
        "____" => Some("____"),
        _ => None,
    }
}

fn delimited_extent(lines: &[SourceLine<'_>], start: usize, delimiter: &str) -> (usize, bool) {
    let mut cursor = start + 1;
    while cursor < lines.len() {
        if lines[cursor].text.trim() == delimiter {
            return (cursor + 1, true);
        }
        cursor += 1;
    }
    (lines.len(), false)
}

fn delimited_body(lines: &[SourceLine<'_>], start: usize, end: usize, closed: bool) -> String {
    let body_end = end.saturating_sub(usize::from(closed));
    lines[start + 1..body_end]
        .iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn anchor(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(body) = trimmed
        .strip_prefix("[[")
        .and_then(|value| value.strip_suffix("]]"))
    {
        return nonempty(body.split(',').next()?.trim());
    }
    trimmed
        .strip_prefix("[#")
        .and_then(|value| value.strip_suffix(']'))
        .and_then(|value| nonempty(value.trim()))
}

fn is_comment_start(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

fn comment_extent(lines: &[SourceLine<'_>], start: usize) -> usize {
    if lines[start].text.trim() != "////" {
        return start + 1;
    }
    let mut cursor = start + 1;
    while cursor < lines.len() {
        if lines[cursor].text.trim() == "////" {
            return cursor + 1;
        }
        cursor += 1;
    }
    lines.len()
}

type InlineMatch = (
    usize,
    usize,
    AsciiDocNodeKind,
    Option<String>,
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

    collect_bracket_macro(
        &raw,
        start,
        "footnote:",
        AsciiDocNodeKind::FootnoteDefinition,
        &mut matches,
    );
    collect_bracket_macro(
        &raw,
        start,
        "xref:",
        AsciiDocNodeKind::CrossReference,
        &mut matches,
    );
    collect_angle_xrefs(&raw, start, &mut matches);
    collect_links(&raw, start, &mut matches);
    collect_attribute_references(&raw, start, &mut matches);
    collect_roles(&raw, start, &mut matches);
    collect_quoted(&raw, start, "**", AsciiDocNodeKind::Strong, &mut matches);
    collect_quoted(&raw, start, "*", AsciiDocNodeKind::Strong, &mut matches);
    collect_quoted(&raw, start, "__", AsciiDocNodeKind::Emphasis, &mut matches);
    collect_quoted(&raw, start, "_", AsciiDocNodeKind::Emphasis, &mut matches);
    collect_quoted(&raw, start, "`", AsciiDocNodeKind::InlineCode, &mut matches);
    collect_unknown_inline_macros(&raw, start, &mut matches);

    matches.sort_by_key(|value| (value.0, value.1));
    matches.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1 && left.2 == right.2);
    for (range_start, range_end, kind, name, value, target) in matches {
        if range_start >= range_end || range_end > builder.decoded.text.len() {
            continue;
        }
        let child = builder.push(
            kind.clone(),
            range_start..range_end,
            Some(builder.nodes[parent].id.clone()),
        );
        builder.nodes[child].name = name;
        builder.nodes[child].text = value;
        builder.nodes[child].target = target;
        builder.nodes[child].known_syntax = kind != AsciiDocNodeKind::RawInline;
        if kind == AsciiDocNodeKind::Role {
            builder.nodes[child].role = builder.nodes[child].name.clone();
        }
        if kind == AsciiDocNodeKind::RawInline {
            diagnostics.push(
                Diagnostic::info(
                    "grist.asciidoc.inline",
                    "inline_macro.unknown_inert",
                    "unknown inline macro was retained without execution",
                )
                .with_range(builder.nodes[child].range.clone())
                .with_locator(builder.nodes[child].locator.clone()),
            );
        }
        builder.link_child(parent, child);
    }
}

fn collect_bracket_macro(
    raw: &str,
    source_start: usize,
    prefix: &str,
    kind: AsciiDocNodeKind,
    matches: &mut Vec<InlineMatch>,
) {
    let mut cursor = 0;
    while let Some(relative) = raw[cursor..].find(prefix) {
        let open = cursor + relative;
        let Some(bracket) = raw[open + prefix.len()..].find('[') else {
            break;
        };
        let bracket = open + prefix.len() + bracket;
        let Some(close_relative) = raw[bracket + 1..].find(']') else {
            break;
        };
        let close = bracket + 1 + close_relative;
        let id = raw[open + prefix.len()..bracket].trim();
        let body = raw[bracket + 1..close].to_string();
        let actual_kind = if prefix == "footnote:" && body.is_empty() {
            AsciiDocNodeKind::FootnoteReference
        } else {
            kind.clone()
        };
        matches.push((
            source_start + open,
            source_start + close + 1,
            actual_kind,
            nonempty(id),
            nonempty(body),
            None,
        ));
        cursor = close + 1;
    }
}

fn collect_angle_xrefs(raw: &str, source_start: usize, matches: &mut Vec<InlineMatch>) {
    let mut cursor = 0;
    while let Some(relative) = raw[cursor..].find("<<") {
        let open = cursor + relative;
        let Some(close_relative) = raw[open + 2..].find(">>") else {
            break;
        };
        let close = open + 2 + close_relative;
        let body = &raw[open + 2..close];
        let (target, text) = body
            .split_once(',')
            .map(|(target, text)| (target, nonempty(text.trim())))
            .unwrap_or((body, None));
        matches.push((
            source_start + open,
            source_start + close + 2,
            AsciiDocNodeKind::CrossReference,
            nonempty(target.trim()),
            text,
            None,
        ));
        cursor = close + 2;
    }
}

fn collect_links(raw: &str, source_start: usize, matches: &mut Vec<InlineMatch>) {
    for scheme in ["https://", "http://", "mailto:"] {
        let mut cursor = 0;
        while let Some(relative) = raw[cursor..].find(scheme) {
            let open = cursor + relative;
            let target_end = raw[open..]
                .find(|value: char| value.is_whitespace() || value == '[')
                .map(|value| open + value)
                .unwrap_or(raw.len());
            let (end, text) = if raw.as_bytes().get(target_end) == Some(&b'[') {
                raw[target_end + 1..]
                    .find(']')
                    .map(|close| {
                        let close = target_end + 1 + close;
                        (close + 1, nonempty(raw[target_end + 1..close].trim()))
                    })
                    .unwrap_or((target_end, None))
            } else {
                (target_end, None)
            };
            matches.push((
                source_start + open,
                source_start + end,
                AsciiDocNodeKind::Hyperlink,
                None,
                text,
                Some(raw[open..target_end].to_string()),
            ));
            cursor = end.max(open + scheme.len());
        }
    }
}

fn collect_attribute_references(raw: &str, source_start: usize, matches: &mut Vec<InlineMatch>) {
    let mut cursor = 0;
    while let Some(relative) = raw[cursor..].find('{') {
        let open = cursor + relative;
        let Some(close_relative) = raw[open + 1..].find('}') else {
            break;
        };
        let close = open + 1 + close_relative;
        let name = &raw[open + 1..close];
        if !name.is_empty()
            && name
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
        {
            matches.push((
                source_start + open,
                source_start + close + 1,
                AsciiDocNodeKind::CrossReference,
                Some(name.to_string()),
                None,
                None,
            ));
        }
        cursor = close + 1;
    }
}

fn collect_roles(raw: &str, source_start: usize, matches: &mut Vec<InlineMatch>) {
    let mut cursor = 0;
    while let Some(relative) = raw[cursor..].find("[.") {
        let open = cursor + relative;
        let Some(attribute_close) = raw[open + 2..].find(']') else {
            break;
        };
        let attribute_close = open + 2 + attribute_close;
        let role = &raw[open + 2..attribute_close];
        let Some(marker) = raw[attribute_close + 1..].chars().next() else {
            cursor = attribute_close + 1;
            continue;
        };
        if !matches!(marker, '#' | '*' | '_' | '`') {
            cursor = attribute_close + 1;
            continue;
        }
        let body_start = attribute_close + 1 + marker.len_utf8();
        let Some(close_relative) = raw[body_start..].find(marker) else {
            break;
        };
        let close = body_start + close_relative;
        matches.push((
            source_start + open,
            source_start + close + marker.len_utf8(),
            AsciiDocNodeKind::Role,
            nonempty(role),
            nonempty(raw[body_start..close].to_string()),
            None,
        ));
        cursor = close + marker.len_utf8();
    }
}

fn collect_quoted(
    raw: &str,
    source_start: usize,
    marker: &str,
    kind: AsciiDocNodeKind,
    matches: &mut Vec<InlineMatch>,
) {
    let mut cursor = 0;
    while let Some(relative) = raw[cursor..].find(marker) {
        let open = cursor + relative;
        let body_start = open + marker.len();
        let Some(close_relative) = raw[body_start..].find(marker) else {
            break;
        };
        let close = body_start + close_relative;
        if close > body_start {
            matches.push((
                source_start + open,
                source_start + close + marker.len(),
                kind.clone(),
                None,
                Some(raw[body_start..close].to_string()),
                None,
            ));
        }
        cursor = close + marker.len();
    }
}

fn collect_unknown_inline_macros(raw: &str, source_start: usize, matches: &mut Vec<InlineMatch>) {
    for (offset, _) in raw.match_indices(':') {
        let name_start = raw[..offset]
            .rfind(|value: char| !value.is_ascii_alphanumeric() && !matches!(value, '-' | '_'))
            .map(|value| value + 1)
            .unwrap_or(0);
        let name = &raw[name_start..offset];
        if name.is_empty() || matches!(name, "http" | "https" | "mailto" | "footnote" | "xref") {
            continue;
        }
        let Some(bracket_relative) = raw[offset + 1..].find('[') else {
            continue;
        };
        let bracket = offset + 1 + bracket_relative;
        if raw[offset + 1..bracket].chars().any(char::is_whitespace) {
            continue;
        }
        let Some(close_relative) = raw[bracket + 1..].find(']') else {
            continue;
        };
        let close = bracket + 1 + close_relative;
        matches.push((
            source_start + name_start,
            source_start + close + 1,
            AsciiDocNodeKind::RawInline,
            Some(name.to_string()),
            None,
            None,
        ));
    }
}

fn raw_diagnostic(node: &AsciiDocNode, code: &str, message: &str) -> Diagnostic {
    Diagnostic::warning("grist.asciidoc.syntax", code, message)
        .with_range(node.range.clone())
        .with_locator(node.locator.clone())
        .partial()
}

fn table_extent(lines: &[SourceLine<'_>], start: usize) -> usize {
    let mut cursor = start + 1;
    while cursor < lines.len() {
        if lines[cursor].text.trim() == "|===" {
            return cursor + 1;
        }
        cursor += 1;
    }
    lines.len()
}

fn handle_table(
    lines: &[SourceLine<'_>],
    start: usize,
    end: usize,
    parent: Option<usize>,
    builder: &mut NodeBuilder<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let closed = end > start + 1 && lines[end - 1].text.trim() == "|===";
    if !closed {
        let node = builder.push(
            AsciiDocNodeKind::RawBlock,
            lines[start].start..lines[end - 1].end,
            parent.map(|value| builder.nodes[value].id.clone()),
        );
        attach_to_parent(builder, parent, node);
        diagnostics.push(raw_diagnostic(
            &builder.nodes[node],
            "table.unclosed",
            "unclosed AsciiDoc table was retained as raw source",
        ));
        return;
    }
    let parsed_rows = parse_pipe_table(lines, start + 1, end - 1);
    if parsed_rows.is_empty() {
        let node = builder.push(
            AsciiDocNodeKind::RawBlock,
            lines[start].start..lines[end - 1].end,
            parent.map(|value| builder.nodes[value].id.clone()),
        );
        attach_to_parent(builder, parent, node);
        diagnostics.push(raw_diagnostic(
            &builder.nodes[node],
            "table.malformed",
            "malformed AsciiDoc table was retained as raw source",
        ));
        return;
    }
    let table = builder.push(
        AsciiDocNodeKind::Table,
        lines[start].start..lines[end - 1].end,
        parent.map(|value| builder.nodes[value].id.clone()),
    );
    let details = table_details(&parsed_rows, &builder.index);
    builder.nodes[table].table = Some(AsciiDocTable {
        style: AsciiDocTableStyle::Pipe,
        rows: details,
    });
    builder.nodes[table].known_syntax = true;
    attach_to_parent(builder, parent, table);
    add_table_nodes(builder, table, parsed_rows);
}

fn parse_pipe_table(lines: &[SourceLine<'_>], start: usize, end: usize) -> Vec<ParsedTableRow> {
    let mut rows = Vec::new();
    for (row_index, line) in lines.iter().enumerate().take(end).skip(start) {
        if line.text.trim().is_empty() {
            continue;
        }
        let mut cells = Vec::new();
        for (offset, part) in line
            .text
            .match_indices('|')
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
        {
            let left = part + 1;
            let right = line.text[left..]
                .find('|')
                .map(|value| left + value)
                .unwrap_or(line.text.len());
            if offset > 0 && left == line.text.len() {
                continue;
            }
            let raw = &line.text[left..right];
            let leading = raw.len() - raw.trim_start().len();
            let trailing = raw.len() - raw.trim_end().len();
            let cell_start = line.start + left + leading;
            let cell_end = line.start + right.saturating_sub(trailing);
            cells.push((cell_start..cell_end.max(cell_start), raw.trim().to_string()));
        }
        if !cells.is_empty() {
            rows.push(ParsedTableRow {
                range: line.start..line.end,
                header: row_index == start,
                cells,
            });
        }
    }
    rows
}
#[derive(Debug, Clone)]
struct ParsedTableRow {
    range: Range<usize>,
    header: bool,
    cells: Vec<(Range<usize>, String)>,
}

fn table_details(rows: &[ParsedTableRow], index: &LineIndex) -> Vec<AsciiDocTableRow> {
    rows.iter()
        .map(|row| {
            let range = SourceRange::new(row.range.start, row.range.end, index);
            AsciiDocTableRow {
                locator: SourceLocator::exact(range.clone()).expect("table row range is exact"),
                range,
                header: row.header,
                cells: row
                    .cells
                    .iter()
                    .map(|(cell, text)| {
                        let range = SourceRange::new(cell.start, cell.end, index);
                        AsciiDocTableCell {
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
            AsciiDocNodeKind::TableRow,
            row.range,
            Some(builder.nodes[table].id.clone()),
        );
        builder.nodes[row_node].known_syntax = row.header;
        builder.link_child(table, row_node);
        for (range, text) in row.cells {
            let cell = builder.push(
                AsciiDocNodeKind::TableCell,
                range,
                Some(builder.nodes[row_node].id.clone()),
            );
            builder.nodes[cell].text = Some(text);
            builder.link_child(row_node, cell);
        }
    }
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

fn paragraph_extent(lines: &[SourceLine<'_>], start: usize) -> usize {
    let mut cursor = start + 1;
    while cursor < lines.len() {
        let text = lines[cursor].text;
        if text.trim().is_empty()
            || heading_at(text).is_some()
            || document_attribute(text).is_some()
            || block_attribute(text).is_some()
            || block_macro(text).is_some()
            || block_delimiter(text).is_some()
            || text.trim() == "|==="
            || anchor(text).is_some()
            || is_comment_start(text)
            || list_marker(text).is_some()
        {
            break;
        }
        cursor += 1;
    }
    cursor
}

fn indentation(line: &str) -> usize {
    line.bytes()
        .take_while(|value| matches!(value, b' ' | b'\t'))
        .count()
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
    fn new(source: &SourceInfo, options: &AsciiDocOptions) -> Self {
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
        options: &AsciiDocOptions,
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
            Diagnostic::warning("grist.asciidoc.include", code, message)
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
            DecodeOptions::for_media_type(Some("text/asciidoc"), Some("asciidoc"));
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
                        .with_parser("grist.asciidoc")
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

    const RICH: &str = "= Demo\n:toc: left\n\n== Section\n\nParagraph [.lead]#important# with footnote:[note], <<target>>, and https://example.test[link].\n\n[[target]]\n[source,rust]\n----\nfn inert() {}\n----\n\n|===\n|Name |Value\n|one |1\n|===\n";

    #[test]
    fn parses_core_constructs_with_exact_locations() {
        let envelope = parse_asciidoc(RICH, SourceInfo::stdin("demo.adoc"));
        assert_eq!(envelope.status, OperationStatus::Complete);
        let document = envelope.payload.unwrap();
        for kind in [
            AsciiDocNodeKind::Heading,
            AsciiDocNodeKind::Attribute,
            AsciiDocNodeKind::Role,
            AsciiDocNodeKind::FootnoteDefinition,
            AsciiDocNodeKind::CrossReference,
            AsciiDocNodeKind::Target,
            AsciiDocNodeKind::CodeBlock,
            AsciiDocNodeKind::Table,
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
        let envelope = parse_asciidoc("include::child.adoc[]\n", SourceInfo::stdin("root.adoc"));
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
        let base = std::env::temp_dir().join(format!("grist-asciidoc-{}", std::process::id()));
        let root = base.join("project");
        fs::create_dir_all(&root).unwrap();
        let main = root.join("main.adoc");
        let child = root.join("child.adoc");
        let outside = base.join("outside.adoc");
        fs::write(&main, "include::child.adoc[]\n").unwrap();
        fs::write(&child, "= Child\n").unwrap();
        fs::write(&outside, "outside").unwrap();
        let options = AsciiDocOptions {
            project_root: Some(root.clone()),
            ..Default::default()
        };
        let resolved = parse_asciidoc_with_options(
            "include::child.adoc[]\n",
            SourceInfo::from_path(&main),
            &options,
        );
        assert_eq!(resolved.status, OperationStatus::Complete);
        let include = resolved.payload.unwrap().nodes[0].include.clone().unwrap();
        assert_eq!(include.status, IncludeStatus::Resolved);
        assert_eq!(include.resolved_path.as_deref(), Some("child.adoc"));
        assert!(
            include
                .resolved
                .unwrap()
                .nodes
                .iter()
                .any(|node| { node.kind == AsciiDocNodeKind::Heading })
        );
        let rejected = parse_asciidoc_with_options(
            "include::../outside.adoc[]\n",
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
