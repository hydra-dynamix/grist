//! HTML5 document/fragment and XHTML parser.
//!
//! html5ever supplies standards-based tree construction and malformed-input
//! recovery. A parallel lexical stream retains every decoded source byte,
//! attribute spelling, active-content token, and unknown declaration without
//! executing or fetching anything.

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, Envelope, FormatIdentity, LineIndex,
    LocationComponent, LocatorConfidence, LocatorPrecision, OperationKind, OperationStatus,
    ParserInfo, SchemaVersion, SourceInfo, SourceLocator, SourceRange, options_digest,
};
use crate::decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeReport, DecodedByteRange, DecodedText,
    RawByteRange, TextEncoding, decode_text,
};
use html5ever::tendril::{StrTendril, TendrilSink};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{
    Attribute, ExpandedName, LocalName, QualName, namespace_url, ns, parse_document, parse_fragment,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlDocument {
    pub schema_version: String,
    pub raw_bytes: Vec<u8>,
    pub raw_range: RawByteRange,
    pub decoded_text: String,
    pub decoded_range: SourceRange,
    pub locator: SourceLocator,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub mode: HtmlParseMode,
    pub syntax: HtmlSyntax,
    /// Recovered DOM in depth-first pre-order. Parent and child IDs make the
    /// authoritative tree explicit even when HTML5 recovery reparents nodes.
    pub nodes: Vec<HtmlNode>,
    /// Exact lexical order, including end tags and declarations not represented
    /// by the recovered DOM.
    pub source_tokens: Vec<HtmlSourceToken>,
    pub metadata: Vec<HtmlMetadata>,
    pub links: Vec<HtmlLink>,
    pub tables: Vec<HtmlTable>,
    pub media: Vec<HtmlMediaReference>,
    pub sections: Vec<HtmlSection>,
    pub active_content: Vec<HtmlActiveContent>,
    pub htmx_attributes: Vec<HtmxAttribute>,
    pub root_element_count: usize,
    pub has_doctype: bool,
    pub quirks_mode: HtmlQuirksMode,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlParseMode {
    Auto,
    Document,
    Fragment,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlSyntax {
    Auto,
    Html5,
    Xhtml,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlQuirksMode {
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlNode {
    pub id: String,
    pub kind: HtmlNodeKind,
    pub range: SourceRange,
    pub locator: SourceLocator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_range: Option<RawByteRange>,
    pub depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub children: Vec<String>,
    pub dom_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
    #[serde(default)]
    pub attributes: Vec<HtmlAttribute>,
    #[serde(default)]
    pub htmx_attributes: Vec<HtmxAttribute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Exact source slice where available. Synthetic recovery nodes use an
    /// empty string and an approximate locator.
    #[serde(default)]
    pub raw: String,
    pub self_closing: bool,
    pub synthetic: bool,
    pub known_element: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlNodeKind {
    Document,
    Element,
    Text,
    Comment,
    Doctype,
    ProcessingInstruction,
    RawUnknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlAttribute {
    pub name: String,
    pub local_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<char>,
    pub name_range: SourceRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_range: Option<SourceRange>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmxAttribute {
    pub element_id: String,
    pub tag_name: String,
    pub name: String,
    pub normalized_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub range: SourceRange,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlSourceToken {
    pub kind: HtmlSourceTokenKind,
    pub range: SourceRange,
    pub locator: SourceLocator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_range: Option<RawByteRange>,
    pub raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub attributes: Vec<HtmlAttribute>,
    pub self_closing: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlSourceTokenKind {
    StartTag,
    EndTag,
    Text,
    Comment,
    Doctype,
    ProcessingInstruction,
    Declaration,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlMetadata {
    pub node_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlLink {
    pub node_id: String,
    pub tag_name: String,
    pub attribute: String,
    pub destination: String,
    #[serde(default)]
    pub rel: Vec<String>,
    pub remote: bool,
    pub active_scheme: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlTable {
    pub node_id: String,
    pub rows: Vec<HtmlTableRow>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlTableRow {
    pub node_id: String,
    pub header: bool,
    pub cells: Vec<HtmlTableCell>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlTableCell {
    pub node_id: String,
    pub text: String,
    pub header: bool,
    pub row_span: usize,
    pub column_span: usize,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlMediaReference {
    pub node_id: String,
    pub tag_name: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlSection {
    pub node_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlActiveContent {
    pub node_id: String,
    pub kind: HtmlActiveContentKind,
    pub source: String,
    pub disposition: HtmlActiveContentDisposition,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlActiveContentKind {
    Script,
    InlineEventHandler,
    JavascriptUrl,
    Form,
    EmbeddedBrowsingContext,
    PluginObject,
    Refresh,
    Style,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlActiveContentDisposition {
    Inert,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HtmlOptions {
    pub mode: HtmlParseMode,
    pub syntax: HtmlSyntax,
    pub encoding: Option<String>,
    pub fragment_context: String,
    pub retain_comments: bool,
}

impl crate::core::FormatOptions for HtmlOptions {
    const FORMAT: &'static str = "html";
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            mode: HtmlParseMode::Auto,
            syntax: HtmlSyntax::Auto,
            encoding: None,
            fragment_context: "body".to_string(),
            retain_comments: true,
        }
    }
}

pub type HtmlEnvelope = Envelope<HtmlDocument>;

pub fn parse_html(text: &str, source: SourceInfo, options: &HtmlOptions) -> HtmlEnvelope {
    parse_html_bytes(text.as_bytes(), source, options)
}

pub fn parse_html_bytes(bytes: &[u8], source: SourceInfo, options: &HtmlOptions) -> HtmlEnvelope {
    let decoding = decode_options(&source, options);
    match decode_text(bytes, &decoding) {
        Ok(decoded) => envelope_from_decoded(&decoded, source, options),
        Err(error) => failed_decode_envelope(bytes, source, options, error),
    }
}

pub(crate) fn decode_options(source: &SourceInfo, options: &HtmlOptions) -> DecodeOptions {
    let mut decoding =
        DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("html"));
    decoding.context = if options.syntax == HtmlSyntax::Xhtml
        || source.declared_mime_type.as_deref().map(media_type_essence)
            == Some("application/xhtml+xml")
    {
        DecodeContext::Xml
    } else if options.syntax == HtmlSyntax::Auto {
        DecodeContext::Auto
    } else {
        DecodeContext::Html
    };
    if let Some(encoding) = &options.encoding {
        decoding.transport_encoding = Some(encoding.clone());
    }
    decoding
}

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new("grist.html")
        .with_implementation("html5ever", "0.29.1")
        .with_specification_version("WHATWG HTML Living Standard + XHTML 1.x inert profile")
        .with_feature("html")
}

fn envelope_from_decoded(
    decoded: &DecodedText,
    source: SourceInfo,
    options: &HtmlOptions,
) -> HtmlEnvelope {
    let (payload, mut diagnostics) = document_from_decoded(decoded, &source, options);
    let partial = decoded.report.makes_operation_partial()
        || diagnostics.iter().any(|diagnostic| diagnostic.partial);
    let mut all_diagnostics = decoded.report.diagnostics.clone();
    all_diagnostics.append(&mut diagnostics);
    let digest = options_digest(options).expect("HTML options always serialize");
    let mut envelope = if partial {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Html,
            source,
            parser_info(),
            digest,
            SchemaVersion::HTML_V2,
            Some(payload),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Html,
            source,
            parser_info(),
            digest,
            SchemaVersion::HTML_V2,
            payload,
        )
    };
    envelope.diagnostics = all_diagnostics;
    envelope
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    let identity = ContentIdentity::for_raw_bytes(decoded.raw_bytes())
        .with_decoded(
            &decoded.text,
            decoded.report.encoding.label(),
            decoded.report.is_lossy(),
        )
        .with_format(FormatIdentity::new("html", Some("text/html")));
    envelope
        .with_identity(identity)
        .with_canonical_payload_identity()
        .expect("HTML payload canonicalization is infallible")
}

fn failed_decode_envelope(
    bytes: &[u8],
    source: SourceInfo,
    options: &HtmlOptions,
    error: DecodeError,
) -> HtmlEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Html,
        OperationStatus::Failed,
        source,
        parser_info(),
        options_digest(options).expect("HTML options serialize"),
        SchemaVersion::HTML_V2,
    )
    .expect("HTML decode failure status is terminal")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes)
            .with_format(FormatIdentity::new("html", Some("text/html"))),
    )
    .with_diagnostics(vec![error.diagnostic().with_parser("grist.html")])
}

pub(crate) fn payload_node_count(document: &HtmlDocument) -> usize {
    document
        .nodes
        .len()
        .saturating_add(document.source_tokens.len())
        .saturating_add(1)
}

fn media_type_essence(value: &str) -> &str {
    value.split(';').next().unwrap_or(value).trim()
}

#[derive(Debug, Clone)]
struct LexResult {
    tokens: Vec<HtmlSourceToken>,
    diagnostics: Vec<Diagnostic>,
    root_element_count: usize,
    has_doctype: bool,
}

fn lex_source(decoded: &DecodedText) -> LexResult {
    let text = decoded.text.as_str();
    let line_index = LineIndex::new(text);
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();
    let mut open = Vec::<String>::new();
    let mut root_element_count = 0usize;
    let mut has_doctype = false;
    let mut cursor = 0usize;
    let mut raw_text_element: Option<String> = None;

    while cursor < text.len() {
        if let Some(raw_name) = raw_text_element.take() {
            if raw_name == "plaintext" {
                push_source_token(
                    &mut tokens,
                    decoded,
                    &line_index,
                    HtmlSourceTokenKind::Text,
                    cursor,
                    text.len(),
                    None,
                    Vec::new(),
                    false,
                );
                break;
            }
            let closing = format!("</{raw_name}");
            if let Some(relative) = find_ascii_case_insensitive(&text[cursor..], &closing) {
                let end = cursor + relative;
                push_source_token(
                    &mut tokens,
                    decoded,
                    &line_index,
                    HtmlSourceTokenKind::Text,
                    cursor,
                    end,
                    None,
                    Vec::new(),
                    false,
                );
                cursor = end;
            } else {
                push_source_token(
                    &mut tokens,
                    decoded,
                    &line_index,
                    HtmlSourceTokenKind::Text,
                    cursor,
                    text.len(),
                    None,
                    Vec::new(),
                    false,
                );
                diagnostics.push(
                    malformed_at(
                        "html.raw_text_unclosed",
                        format!("raw-text element <{raw_name}> has no closing tag"),
                        SourceRange::new(cursor, text.len(), &line_index),
                    )
                    .partial(),
                );
                break;
            }
        }

        let Some(relative_start) = text[cursor..].find('<') else {
            push_source_token(
                &mut tokens,
                decoded,
                &line_index,
                HtmlSourceTokenKind::Text,
                cursor,
                text.len(),
                None,
                Vec::new(),
                false,
            );
            break;
        };
        let start = cursor + relative_start;
        push_source_token(
            &mut tokens,
            decoded,
            &line_index,
            HtmlSourceTokenKind::Text,
            cursor,
            start,
            None,
            Vec::new(),
            false,
        );

        if text[start..].starts_with("<!--") {
            let end = text[start + 4..]
                .find("-->")
                .map(|relative| start + 4 + relative + 3)
                .unwrap_or(text.len());
            push_source_token(
                &mut tokens,
                decoded,
                &line_index,
                HtmlSourceTokenKind::Comment,
                start,
                end,
                None,
                Vec::new(),
                false,
            );
            if end == text.len() && !text[start..].ends_with("-->") {
                diagnostics.push(
                    malformed_at(
                        "html.comment_unclosed",
                        "comment has no closing marker",
                        SourceRange::new(start, end, &line_index),
                    )
                    .partial(),
                );
            }
            cursor = end;
            continue;
        }

        let end = match scan_markup_end(text, start) {
            Some(end) => end,
            None => {
                let range = SourceRange::new(start, text.len(), &line_index);
                push_source_token(
                    &mut tokens,
                    decoded,
                    &line_index,
                    HtmlSourceTokenKind::Declaration,
                    start,
                    text.len(),
                    None,
                    Vec::new(),
                    false,
                );
                diagnostics.push(
                    malformed_at(
                        "html.markup_unclosed",
                        "markup token has no closing delimiter",
                        range,
                    )
                    .partial(),
                );
                break;
            }
        };
        let raw = &text[start..end];
        let inner = raw
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .unwrap_or_default();
        let trimmed = inner.trim();

        if trimmed.starts_with('?') {
            push_source_token(
                &mut tokens,
                decoded,
                &line_index,
                HtmlSourceTokenKind::ProcessingInstruction,
                start,
                end,
                processing_instruction_name(trimmed),
                Vec::new(),
                false,
            );
        } else if trimmed.starts_with('!') {
            let doctype = trimmed.get(1..).is_some_and(|value| {
                value
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("doctype")
            });
            has_doctype |= doctype;
            push_source_token(
                &mut tokens,
                decoded,
                &line_index,
                if doctype {
                    HtmlSourceTokenKind::Doctype
                } else {
                    HtmlSourceTokenKind::Declaration
                },
                start,
                end,
                None,
                Vec::new(),
                false,
            );
        } else if let Some(rest) = trimmed.strip_prefix('/') {
            let name = read_markup_name(rest).to_string();
            push_source_token(
                &mut tokens,
                decoded,
                &line_index,
                HtmlSourceTokenKind::EndTag,
                start,
                end,
                Some(name.clone()),
                Vec::new(),
                false,
            );
            let normalized = name.to_ascii_lowercase();
            if let Some(position) = open.iter().rposition(|candidate| candidate == &normalized) {
                open.truncate(position);
            }
        } else {
            let source_name = read_markup_name(trimmed).to_string();
            if source_name.is_empty() {
                push_source_token(
                    &mut tokens,
                    decoded,
                    &line_index,
                    HtmlSourceTokenKind::Text,
                    start,
                    end,
                    None,
                    Vec::new(),
                    false,
                );
                cursor = end;
                continue;
            }
            let normalized = source_name.to_ascii_lowercase();
            let self_closing =
                trimmed.trim_end().ends_with('/') || is_void_element(normalized.as_str());
            let attributes =
                parse_source_attributes(decoded, &line_index, start + 1, end.saturating_sub(1));
            if open.is_empty() {
                root_element_count = root_element_count.saturating_add(1);
            }
            push_source_token(
                &mut tokens,
                decoded,
                &line_index,
                HtmlSourceTokenKind::StartTag,
                start,
                end,
                Some(source_name),
                attributes,
                self_closing,
            );
            if !self_closing {
                open.push(normalized.clone());
                if is_raw_text_element(&normalized) {
                    raw_text_element = Some(normalized);
                }
            }
        }
        cursor = end;
    }

    LexResult {
        tokens,
        diagnostics,
        root_element_count,
        has_doctype,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_source_token(
    tokens: &mut Vec<HtmlSourceToken>,
    decoded: &DecodedText,
    line_index: &LineIndex,
    kind: HtmlSourceTokenKind,
    start: usize,
    end: usize,
    name: Option<String>,
    attributes: Vec<HtmlAttribute>,
    self_closing: bool,
) {
    if start >= end {
        return;
    }
    let range = SourceRange::new(start, end, line_index);
    tokens.push(HtmlSourceToken {
        kind,
        locator: SourceLocator::exact(range.clone()).expect("valid lexical HTML range"),
        raw_range: decoded.raw_range_for_decoded(DecodedByteRange::from_usize(start, end)),
        raw: decoded.text[start..end].to_string(),
        range,
        name,
        attributes,
        self_closing,
    });
}

fn scan_markup_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut cursor = start.saturating_add(1);
    let mut quote = None;
    while cursor < bytes.len() {
        match (quote, bytes[cursor]) {
            (Some(expected), found) if expected == found => quote = None,
            (None, b'\'' | b'"') => quote = Some(bytes[cursor]),
            (None, b'>') => return Some(cursor + 1),
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn processing_instruction_name(value: &str) -> Option<String> {
    let value = value.trim_start_matches('?').trim_start();
    let name = read_markup_name(value);
    (!name.is_empty()).then(|| name.to_string())
}

fn read_markup_name(value: &str) -> &str {
    let end = value
        .char_indices()
        .find_map(|(index, character)| {
            (character.is_whitespace() || matches!(character, '/' | '>' | '?')).then_some(index)
        })
        .unwrap_or(value.len());
    &value[..end]
}

fn parse_source_attributes(
    decoded: &DecodedText,
    line_index: &LineIndex,
    inner_start: usize,
    inner_end: usize,
) -> Vec<HtmlAttribute> {
    let source = decoded.text.as_str();
    let inner = &source[inner_start..inner_end];
    let mut cursor = read_markup_name(inner).len();
    let mut attributes = Vec::new();
    while cursor < inner.len() {
        cursor = skip_ascii_whitespace(inner, cursor);
        if cursor >= inner.len() || inner[cursor..].starts_with('/') {
            break;
        }
        let name_start = cursor;
        while cursor < inner.len() {
            let byte = inner.as_bytes()[cursor];
            if byte.is_ascii_whitespace() || matches!(byte, b'=' | b'/' | b'>') {
                break;
            }
            cursor += 1;
        }
        if cursor == name_start {
            cursor += 1;
            continue;
        }
        let name_end = cursor;
        let name = inner[name_start..name_end].to_string();
        cursor = skip_ascii_whitespace(inner, cursor);
        let mut value = None;
        let mut raw_value = None;
        let mut value_range = None;
        let mut quote_value = None;
        if cursor < inner.len() && inner.as_bytes()[cursor] == b'=' {
            cursor += 1;
            cursor = skip_ascii_whitespace(inner, cursor);
            if cursor < inner.len() {
                let quote = inner.as_bytes()[cursor];
                if matches!(quote, b'\'' | b'"') {
                    quote_value = Some(char::from(quote));
                    cursor += 1;
                    let value_start = cursor;
                    while cursor < inner.len() && inner.as_bytes()[cursor] != quote {
                        cursor += 1;
                    }
                    let value_end = cursor;
                    let found = inner[value_start..value_end].to_string();
                    value = Some(found.clone());
                    raw_value = Some(found);
                    value_range = Some(SourceRange::new(
                        inner_start + value_start,
                        inner_start + value_end,
                        line_index,
                    ));
                    if cursor < inner.len() {
                        cursor += 1;
                    }
                } else {
                    let value_start = cursor;
                    while cursor < inner.len() {
                        let byte = inner.as_bytes()[cursor];
                        if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>') {
                            break;
                        }
                        cursor += 1;
                    }
                    let value_end = cursor;
                    let found = inner[value_start..value_end].to_string();
                    value = Some(found.clone());
                    raw_value = Some(found);
                    value_range = Some(SourceRange::new(
                        inner_start + value_start,
                        inner_start + value_end,
                        line_index,
                    ));
                }
            }
        }
        let absolute_name_start = inner_start + name_start;
        let absolute_name_end = inner_start + name_end;
        let name_range = SourceRange::new(absolute_name_start, absolute_name_end, line_index);
        let (prefix, local_name, namespace) = split_attribute_name(&name);
        attributes.push(HtmlAttribute {
            name,
            local_name,
            prefix,
            namespace,
            value,
            raw_value,
            quote: quote_value,
            locator: SourceLocator::exact(name_range.clone())
                .expect("valid HTML attribute name range"),
            name_range,
            value_range,
        });
    }
    attributes
}

fn split_attribute_name(name: &str) -> (Option<String>, String, Option<String>) {
    let (prefix, local) = name
        .split_once(':')
        .map_or((None, name), |(prefix, local)| (Some(prefix), local));
    let namespace = match (prefix, name) {
        (Some("xml"), _) => Some("http://www.w3.org/XML/1998/namespace"),
        (Some("xmlns"), _) | (None, "xmlns") => Some("http://www.w3.org/2000/xmlns/"),
        (Some("xlink"), _) => Some("http://www.w3.org/1999/xlink"),
        _ => None,
    };
    (
        prefix.map(str::to_string),
        local.to_string(),
        namespace.map(str::to_string),
    )
}

fn skip_ascii_whitespace(value: &str, mut cursor: usize) -> usize {
    while cursor < value.len() && value.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

fn malformed_at(code: &str, message: impl Into<String>, range: SourceRange) -> Diagnostic {
    Diagnostic::malformed("grist.html", message)
        .with_range(range.clone())
        .with_locator(SourceLocator::exact(range).expect("valid malformed HTML range"))
        .with_explanation_key(code)
}

fn is_raw_text_element(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "textarea"
            | "title"
            | "xmp"
            | "iframe"
            | "noembed"
            | "noframes"
            | "plaintext"
    )
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

type DomHandle = Rc<DomNode>;

struct DomNode {
    parent: RefCell<Weak<DomNode>>,
    children: RefCell<Vec<DomHandle>>,
    data: DomData,
    source_tokens: RefCell<Vec<usize>>,
}

enum DomData {
    Document,
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    Text {
        contents: RefCell<String>,
    },
    Comment {
        contents: String,
    },
    ProcessingInstruction {
        target: String,
        contents: String,
    },
    Element {
        name: QualName,
        attrs: RefCell<Vec<Attribute>>,
        template_contents: Option<DomHandle>,
        mathml_annotation_xml_integration_point: bool,
    },
}

impl DomNode {
    fn new(data: DomData) -> DomHandle {
        Rc::new(Self {
            parent: RefCell::new(Weak::new()),
            children: RefCell::new(Vec::new()),
            data,
            source_tokens: RefCell::new(Vec::new()),
        })
    }
}

struct SourceClaims {
    tokens: Rc<Vec<HtmlSourceToken>>,
    used: RefCell<Vec<bool>>,
    suppress_element_claims: Cell<usize>,
}

impl SourceClaims {
    fn new(tokens: Rc<Vec<HtmlSourceToken>>, suppress_element_claims: usize) -> Self {
        Self {
            used: RefCell::new(vec![false; tokens.len()]),
            tokens,
            suppress_element_claims: Cell::new(suppress_element_claims),
        }
    }

    fn claim_element(&self, name: &str) -> Option<usize> {
        let suppressed = self.suppress_element_claims.get();
        if suppressed > 0 {
            self.suppress_element_claims.set(suppressed - 1);
            return None;
        }
        self.claim(|token| {
            token.kind == HtmlSourceTokenKind::StartTag
                && token
                    .name
                    .as_deref()
                    .is_some_and(|source| source.eq_ignore_ascii_case(name))
        })
    }

    fn claim_kind(&self, kinds: &[HtmlSourceTokenKind]) -> Option<usize> {
        self.claim(|token| kinds.contains(&token.kind))
    }

    fn claim(&self, predicate: impl Fn(&HtmlSourceToken) -> bool) -> Option<usize> {
        let mut used = self.used.borrow_mut();
        let found = self
            .tokens
            .iter()
            .enumerate()
            .find_map(|(index, token)| (!used[index] && predicate(token)).then_some(index));
        if let Some(index) = found {
            used[index] = true;
        }
        found
    }
}

struct DomSink {
    document: DomHandle,
    claims: SourceClaims,
    errors: RefCell<Vec<String>>,
    quirks: Cell<QuirksMode>,
}

struct DomOutput {
    document: DomHandle,
    errors: Vec<String>,
    quirks: QuirksMode,
}

impl DomSink {
    fn new(tokens: Rc<Vec<HtmlSourceToken>>, suppress_element_claims: usize) -> Self {
        Self {
            document: DomNode::new(DomData::Document),
            claims: SourceClaims::new(tokens, suppress_element_claims),
            errors: RefCell::new(Vec::new()),
            quirks: Cell::new(QuirksMode::NoQuirks),
        }
    }

    fn create_text(&self, contents: StrTendril) -> DomHandle {
        let node = DomNode::new(DomData::Text {
            contents: RefCell::new(contents.to_string()),
        });
        if let Some(token) = self.claims.claim_kind(&[HtmlSourceTokenKind::Text]) {
            node.source_tokens.borrow_mut().push(token);
        }
        node
    }

    fn detach(&self, node: &DomHandle) {
        let Some(parent) = node.parent.borrow().upgrade() else {
            return;
        };
        parent
            .children
            .borrow_mut()
            .retain(|candidate| !Rc::ptr_eq(candidate, node));
        *node.parent.borrow_mut() = Weak::new();
    }

    fn append_node(&self, parent: &DomHandle, child: DomHandle) {
        self.detach(&child);
        *child.parent.borrow_mut() = Rc::downgrade(parent);
        parent.children.borrow_mut().push(child);
    }

    fn insert_before(&self, sibling: &DomHandle, child: DomHandle) {
        let Some(parent) = sibling.parent.borrow().upgrade() else {
            return;
        };
        self.detach(&child);
        let mut children = parent.children.borrow_mut();
        let position = children
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, sibling))
            .unwrap_or(children.len());
        *child.parent.borrow_mut() = Rc::downgrade(&parent);
        children.insert(position, child);
    }

    fn append_text(&self, parent: &DomHandle, text: StrTendril) {
        if let Some(previous) = parent.children.borrow().last().cloned()
            && let DomData::Text { contents } = &previous.data
        {
            contents.borrow_mut().push_str(text.as_ref());
            if let Some(token) = self.claims.claim_kind(&[HtmlSourceTokenKind::Text]) {
                previous.source_tokens.borrow_mut().push(token);
            }
            return;
        }
        let node = self.create_text(text);
        self.append_node(parent, node);
    }

    fn append_text_before(&self, sibling: &DomHandle, text: StrTendril) {
        if let Some(parent) = sibling.parent.borrow().upgrade() {
            let previous = {
                let children = parent.children.borrow();
                let position = children
                    .iter()
                    .position(|candidate| Rc::ptr_eq(candidate, sibling));
                position
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| children.get(index).cloned())
            };
            if let Some(previous) = previous
                && let DomData::Text { contents } = &previous.data
            {
                contents.borrow_mut().push_str(text.as_ref());
                if let Some(token) = self.claims.claim_kind(&[HtmlSourceTokenKind::Text]) {
                    previous.source_tokens.borrow_mut().push(token);
                }
                return;
            }
        }
        let node = self.create_text(text);
        self.insert_before(sibling, node);
    }
}

impl TreeSink for DomSink {
    type Handle = DomHandle;
    type Output = DomOutput;
    type ElemName<'a>
        = ExpandedName<'a>
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        DomOutput {
            document: self.document,
            errors: self.errors.into_inner(),
            quirks: self.quirks.get(),
        }
    }

    fn parse_error(&self, message: Cow<'static, str>) {
        self.errors.borrow_mut().push(message.into_owned());
    }

    fn get_document(&self) -> Self::Handle {
        self.document.clone()
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        match &target.data {
            DomData::Element { name, .. } => name.expanded(),
            _ => panic!("html5ever requested an element name for a non-element node"),
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> Self::Handle {
        let source_token = self.claims.claim_element(name.local.as_ref());
        let template_contents = flags.template.then(|| DomNode::new(DomData::Document));
        let node = DomNode::new(DomData::Element {
            name,
            attrs: RefCell::new(attrs),
            template_contents,
            mathml_annotation_xml_integration_point: flags.mathml_annotation_xml_integration_point,
        });
        if let Some(token) = source_token {
            node.source_tokens.borrow_mut().push(token);
        }
        node
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        let node = DomNode::new(DomData::Comment {
            contents: text.to_string(),
        });
        if let Some(token) = self.claims.claim_kind(&[
            HtmlSourceTokenKind::Comment,
            HtmlSourceTokenKind::ProcessingInstruction,
            HtmlSourceTokenKind::Declaration,
        ]) {
            node.source_tokens.borrow_mut().push(token);
        }
        node
    }

    fn create_pi(&self, target: StrTendril, contents: StrTendril) -> Self::Handle {
        let node = DomNode::new(DomData::ProcessingInstruction {
            target: target.to_string(),
            contents: contents.to_string(),
        });
        if let Some(token) = self
            .claims
            .claim_kind(&[HtmlSourceTokenKind::ProcessingInstruction])
        {
            node.source_tokens.borrow_mut().push(token);
        }
        node
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => self.append_node(parent, node),
            NodeOrText::AppendText(text) => self.append_text(parent, text),
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        previous_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        if element.parent.borrow().upgrade().is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(previous_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        let node = DomNode::new(DomData::Doctype {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        });
        if let Some(token) = self.claims.claim_kind(&[HtmlSourceTokenKind::Doctype]) {
            node.source_tokens.borrow_mut().push(token);
        }
        self.append_node(&self.document, node);
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        match &target.data {
            DomData::Element {
                template_contents: Some(contents),
                ..
            } => contents.clone(),
            _ => panic!("html5ever requested template contents for a non-template node"),
        }
    }

    fn same_node(&self, left: &Self::Handle, right: &Self::Handle) -> bool {
        Rc::ptr_eq(left, right)
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks.set(mode);
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => self.insert_before(sibling, node),
            NodeOrText::AppendText(text) => self.append_text_before(sibling, text),
        }
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let DomData::Element {
            name,
            attrs: existing,
            ..
        } = &target.data
        else {
            panic!("html5ever added attributes to a non-element");
        };
        if target.source_tokens.borrow().is_empty()
            && let Some(token) = self.claims.claim_element(name.local.as_ref())
        {
            target.source_tokens.borrow_mut().push(token);
        }
        let mut existing = existing.borrow_mut();
        let names = existing
            .iter()
            .map(|attribute| attribute.name.clone())
            .collect::<HashSet<_>>();
        existing.extend(
            attrs
                .into_iter()
                .filter(|attribute| !names.contains(&attribute.name)),
        );
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.detach(target);
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let children = std::mem::take(&mut *node.children.borrow_mut());
        for child in children {
            *child.parent.borrow_mut() = Rc::downgrade(new_parent);
            new_parent.children.borrow_mut().push(child);
        }
    }

    fn is_mathml_annotation_xml_integration_point(&self, target: &Self::Handle) -> bool {
        match &target.data {
            DomData::Element {
                mathml_annotation_xml_integration_point,
                ..
            } => *mathml_annotation_xml_integration_point,
            _ => false,
        }
    }
}

pub(crate) fn document_from_decoded(
    decoded: &DecodedText,
    source: &SourceInfo,
    options: &HtmlOptions,
) -> (HtmlDocument, Vec<Diagnostic>) {
    let syntax = resolve_syntax(decoded.text.as_str(), source, options.syntax);
    let mode = resolve_mode(decoded.text.as_str(), options.mode);
    let lexed = lex_source(decoded);
    let token_source = Rc::new(lexed.tokens.clone());
    let sink = DomSink::new(token_source, usize::from(mode == HtmlParseMode::Fragment));
    let output = if mode == HtmlParseMode::Fragment {
        let context = if options.fragment_context.trim().is_empty() {
            "body"
        } else {
            options.fragment_context.trim()
        };
        parse_fragment(
            sink,
            Default::default(),
            QualName::new(None, ns!(html), LocalName::from(context)),
            Vec::new(),
        )
        .one(decoded.text.clone())
    } else {
        parse_document(sink, Default::default()).one(decoded.text.clone())
    };

    let line_index = LineIndex::new(decoded.text.as_str());
    let close_ends = source_element_close_ends(&lexed.tokens);
    let mut nodes = materialize_dom(
        &output.document,
        decoded,
        &line_index,
        &lexed.tokens,
        &close_ends,
        options.retain_comments,
    );
    hydrate_children(&mut nodes);
    let mut diagnostics = lexed.diagnostics;
    diagnostics.extend(output.errors.into_iter().map(|message| {
        let range = SourceRange::new(0, decoded.text.len(), &line_index);
        if syntax == HtmlSyntax::Xhtml {
            Diagnostic::info(
                "grist.html",
                "xhtml.html5_projection_adjustment",
                format!("HTML5 DOM projection reported: {message}"),
            )
            .with_range(range.clone())
            .with_locator(approximate_dom_locator(range, "/"))
            .with_explanation_key("xhtml.html5_projection_adjustment")
        } else {
            Diagnostic::malformed(
                "grist.html",
                format!("HTML5 tree construction recovered from: {message}"),
            )
            .with_range(range.clone())
            .with_locator(approximate_dom_locator(range, "/"))
            .with_explanation_key("html.tree_recovery")
            .partial()
        }
    }));
    if syntax == HtmlSyntax::Xhtml {
        diagnostics.extend(validate_xhtml(
            decoded.text.as_str(),
            &line_index,
            &lexed.tokens,
        ));
    }
    if mode == HtmlParseMode::Document && syntax == HtmlSyntax::Html5 {
        if !lexed.has_doctype {
            diagnostics.push(
                Diagnostic::warning(
                    "grist.html",
                    "html.document_missing_doctype",
                    "document mode input has no doctype and may use quirks mode",
                )
                .with_explanation_key("html.document_missing_doctype"),
            );
        }
        if !lexed.tokens.iter().any(|token| {
            token.kind == HtmlSourceTokenKind::StartTag
                && token
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("html"))
        }) {
            diagnostics.push(
                Diagnostic::info(
                    "grist.html",
                    "html.implied_html_element",
                    "HTML5 tree construction supplied an implied html element",
                )
                .with_explanation_key("html.implied_html_element"),
            );
        }
    }

    let metadata = extract_metadata(&nodes);
    let links = extract_links(&nodes);
    let tables = extract_tables(&nodes);
    let media = extract_media(&nodes);
    let sections = extract_sections(&nodes);
    let active_content = extract_active_content(&nodes);
    if !active_content.is_empty() {
        diagnostics.push(
            Diagnostic::info(
                "grist.html",
                "html.active_content_inert",
                format!(
                    "{} active-content construct(s) were retained as inert source data",
                    active_content.len()
                ),
            )
            .with_explanation_key("html.active_content_inert"),
        );
    }
    let htmx_attributes = nodes
        .iter()
        .flat_map(|node| node.htmx_attributes.iter().cloned())
        .collect();
    let decoded_range = SourceRange::new(0, decoded.text.len(), &line_index);
    (
        HtmlDocument {
            schema_version: SchemaVersion::HTML_V2.to_string(),
            raw_bytes: decoded.raw_bytes().to_vec(),
            raw_range: RawByteRange::from_usize(0, decoded.raw_bytes().len()),
            decoded_text: decoded.text.clone(),
            decoded_range: decoded_range.clone(),
            locator: SourceLocator::exact(decoded_range).expect("valid HTML document range"),
            encoding: decoded.report.encoding.clone(),
            decoding: decoded.report.clone(),
            mode,
            syntax,
            nodes,
            source_tokens: lexed.tokens,
            metadata,
            links,
            tables,
            media,
            sections,
            active_content,
            htmx_attributes,
            root_element_count: lexed.root_element_count,
            has_doctype: lexed.has_doctype,
            quirks_mode: match output.quirks {
                QuirksMode::NoQuirks => HtmlQuirksMode::NoQuirks,
                QuirksMode::LimitedQuirks => HtmlQuirksMode::LimitedQuirks,
                QuirksMode::Quirks => HtmlQuirksMode::Quirks,
            },
        },
        diagnostics,
    )
}

fn resolve_syntax(text: &str, source: &SourceInfo, requested: HtmlSyntax) -> HtmlSyntax {
    if requested != HtmlSyntax::Auto {
        return requested;
    }
    if source.declared_mime_type.as_deref().map(media_type_essence) == Some("application/xhtml+xml")
        || text.trim_start().starts_with("<?xml")
        || text
            .get(..text.len().min(1024))
            .is_some_and(|prefix| prefix.contains(r#"xmlns="http://www.w3.org/1999/xhtml""#))
    {
        HtmlSyntax::Xhtml
    } else {
        HtmlSyntax::Html5
    }
}

fn resolve_mode(text: &str, requested: HtmlParseMode) -> HtmlParseMode {
    if requested != HtmlParseMode::Auto {
        return requested;
    }
    let prefix = text
        .trim_start()
        .chars()
        .take(256)
        .collect::<String>()
        .to_ascii_lowercase();
    if prefix.starts_with("<!doctype") || prefix.starts_with("<html") || prefix.starts_with("<?xml")
    {
        HtmlParseMode::Document
    } else {
        HtmlParseMode::Fragment
    }
}

fn source_element_close_ends(tokens: &[HtmlSourceToken]) -> HashMap<usize, usize> {
    let mut stack = Vec::<usize>::new();
    let mut ends = HashMap::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            HtmlSourceTokenKind::StartTag if !token.self_closing => stack.push(index),
            HtmlSourceTokenKind::EndTag => {
                let Some(name) = token.name.as_deref() else {
                    continue;
                };
                if let Some(position) = stack.iter().rposition(|open_index| {
                    tokens[*open_index]
                        .name
                        .as_deref()
                        .is_some_and(|open| open.eq_ignore_ascii_case(name))
                }) {
                    let open_index = stack.remove(position);
                    ends.insert(open_index, token.range.byte_end);
                }
            }
            _ => {}
        }
    }
    ends
}

#[derive(Clone)]
struct DomVisit {
    handle: DomHandle,
    depth: usize,
    parent: Option<usize>,
    path: String,
}

fn collect_dom_visits(root: &DomHandle) -> Vec<DomVisit> {
    fn walk(
        handle: &DomHandle,
        depth: usize,
        parent: Option<usize>,
        path: String,
        visits: &mut Vec<DomVisit>,
    ) {
        let index = visits.len();
        visits.push(DomVisit {
            handle: handle.clone(),
            depth,
            parent,
            path: path.clone(),
        });
        let mut ordinals = HashMap::<String, usize>::new();
        let mut children = handle.children.borrow().clone();
        if let DomData::Element {
            template_contents: Some(contents),
            ..
        } = &handle.data
        {
            children.extend(contents.children.borrow().iter().cloned());
        }
        for child in children {
            let component = dom_path_component(&child);
            let ordinal = ordinals.entry(component.clone()).or_default();
            *ordinal += 1;
            let child_path = if path == "/" {
                format!("/{component}[{ordinal}]")
            } else {
                format!("{path}/{component}[{ordinal}]")
            };
            walk(&child, depth + 1, Some(index), child_path, visits);
        }
    }
    let mut visits = Vec::new();
    walk(root, 0, None, "/".to_string(), &mut visits);
    visits
}

fn dom_path_component(node: &DomHandle) -> String {
    match &node.data {
        DomData::Document => "document-fragment".to_string(),
        DomData::Doctype { .. } => "doctype()".to_string(),
        DomData::Text { .. } => "text()".to_string(),
        DomData::Comment { .. } => "comment()".to_string(),
        DomData::ProcessingInstruction { target, .. } => {
            format!("processing-instruction({target})")
        }
        DomData::Element { name, .. } => name.local.to_string(),
    }
}

fn materialize_dom(
    root: &DomHandle,
    decoded: &DecodedText,
    line_index: &LineIndex,
    tokens: &[HtmlSourceToken],
    close_ends: &HashMap<usize, usize>,
    retain_comments: bool,
) -> Vec<HtmlNode> {
    let visits = collect_dom_visits(root);
    let included = visits
        .iter()
        .enumerate()
        .filter(|(_, visit)| {
            retain_comments || !matches!(visit.handle.data, DomData::Comment { .. })
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let ids = included
        .iter()
        .enumerate()
        .map(|(ordinal, index)| (*index, format!("html-node-{ordinal}")))
        .collect::<HashMap<_, _>>();
    included
        .iter()
        .enumerate()
        .map(|(ordinal, visit_index)| {
            let visit = &visits[*visit_index];
            let id = ids[visit_index].clone();
            let parent_id = visit.parent.and_then(|parent| ids.get(&parent).cloned());
            materialize_node(
                visit, id, parent_id, ordinal, decoded, line_index, tokens, close_ends,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn materialize_node(
    visit: &DomVisit,
    id: String,
    parent_id: Option<String>,
    _ordinal: usize,
    decoded: &DecodedText,
    line_index: &LineIndex,
    tokens: &[HtmlSourceToken],
    close_ends: &HashMap<usize, usize>,
) -> HtmlNode {
    let claims = visit.handle.source_tokens.borrow();
    let first_claim = claims.first().copied();
    let claimed_start = claims
        .iter()
        .filter_map(|index| tokens.get(*index))
        .map(|token| token.range.byte_start)
        .min();
    let claimed_end = claims
        .iter()
        .filter_map(|index| {
            let token = tokens.get(*index)?;
            Some(
                close_ends
                    .get(index)
                    .copied()
                    .unwrap_or(token.range.byte_end),
            )
        })
        .max();
    let exact = claimed_start.zip(claimed_end);
    let range = exact.map_or_else(
        || SourceRange::new(0, decoded.text.len(), line_index),
        |(start, end)| SourceRange::new(start, end, line_index),
    );
    let locator = dom_locator(range.clone(), visit.path.as_str(), exact.is_some());
    let raw = exact
        .map(|(start, end)| decoded.text[start..end].to_string())
        .unwrap_or_default();
    let raw_range = exact.and_then(|(start, end)| {
        decoded.raw_range_for_decoded(DecodedByteRange::from_usize(start, end))
    });
    let source_token = first_claim.and_then(|index| tokens.get(index));
    let (
        kind,
        namespace,
        prefix,
        tag_name,
        source_name,
        attributes,
        text,
        self_closing,
        known_element,
    ) = match &visit.handle.data {
        DomData::Document => (
            HtmlNodeKind::Document,
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            false,
            true,
        ),
        DomData::Doctype {
            name,
            public_id,
            system_id,
        } => (
            HtmlNodeKind::Doctype,
            None,
            None,
            None,
            None,
            Vec::new(),
            Some(format!("{name}|{public_id}|{system_id}")),
            false,
            true,
        ),
        DomData::Text { contents } => (
            HtmlNodeKind::Text,
            None,
            None,
            None,
            None,
            Vec::new(),
            Some(contents.borrow().clone()),
            false,
            true,
        ),
        DomData::Comment { contents } => (
            HtmlNodeKind::Comment,
            None,
            None,
            None,
            None,
            Vec::new(),
            Some(contents.clone()),
            false,
            true,
        ),
        DomData::ProcessingInstruction { target, contents } => (
            HtmlNodeKind::ProcessingInstruction,
            None,
            None,
            None,
            Some(target.clone()),
            Vec::new(),
            Some(contents.clone()),
            false,
            true,
        ),
        DomData::Element { name, attrs, .. } => {
            let local = name.local.to_string();
            let attributes = materialize_attributes(
                source_token,
                attrs.borrow().as_slice(),
                range.clone(),
                exact.is_some(),
            );
            let known = name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
                || known_html_element(local.as_str());
            (
                HtmlNodeKind::Element,
                Some(name.ns.to_string()),
                name.prefix.as_ref().map(ToString::to_string),
                Some(local.clone()),
                source_token.and_then(|token| token.name.clone()),
                attributes,
                None,
                source_token.is_some_and(|token| token.self_closing),
                known,
            )
        }
    };
    let htmx_attributes = tag_name
        .as_deref()
        .map(|tag| {
            attributes
                .iter()
                .filter_map(|attribute| htmx_attribute(&id, tag, attribute))
                .collect()
        })
        .unwrap_or_default();
    HtmlNode {
        id,
        kind,
        range,
        locator,
        raw_range,
        depth: visit.depth,
        parent_id,
        children: Vec::new(),
        dom_path: visit.path.clone(),
        namespace,
        prefix,
        tag_name,
        source_name,
        attributes,
        htmx_attributes,
        text,
        raw,
        self_closing,
        synthetic: exact.is_none(),
        known_element,
    }
}

fn dom_locator(range: SourceRange, path: &str, exact: bool) -> SourceLocator {
    let precision = if exact {
        LocatorPrecision::Exact { derived_from: None }
    } else {
        LocatorPrecision::Approximate {
            confidence: LocatorConfidence::new(0.5).expect("constant confidence is valid"),
            derived_from: None,
        }
    };
    SourceLocator::new(
        vec![
            LocationComponent::from(range),
            LocationComponent::XmlPath {
                path: path.to_string(),
            },
        ],
        precision,
    )
    .expect("HTML DOM locators are valid")
}

fn approximate_dom_locator(range: SourceRange, path: &str) -> SourceLocator {
    dom_locator(range, path, false)
}

fn materialize_attributes(
    source_token: Option<&HtmlSourceToken>,
    dom_attributes: &[Attribute],
    element_range: SourceRange,
    exact_element: bool,
) -> Vec<HtmlAttribute> {
    let mut attributes = source_token
        .map(|token| token.attributes.clone())
        .unwrap_or_default();
    let mut consumed = HashSet::<usize>::new();
    for attribute in &mut attributes {
        if let Some((index, dom)) = dom_attributes.iter().enumerate().find(|(index, dom)| {
            !consumed.contains(index)
                && dom
                    .name
                    .local
                    .as_ref()
                    .eq_ignore_ascii_case(attribute.local_name.as_str())
        }) {
            consumed.insert(index);
            attribute.value = Some(dom.value.to_string());
            attribute.prefix = dom.name.prefix.as_ref().map(ToString::to_string);
            let namespace = dom.name.ns.to_string();
            if !namespace.is_empty() {
                attribute.namespace = Some(namespace);
            }
        }
    }
    for (index, dom) in dom_attributes.iter().enumerate() {
        if consumed.contains(&index) {
            continue;
        }
        let name_range = element_range.clone();
        let locator = if exact_element {
            SourceLocator::exact(name_range.clone()).expect("valid element range")
        } else {
            approximate_dom_locator(name_range.clone(), "/")
        };
        attributes.push(HtmlAttribute {
            name: dom.name.local.to_string(),
            local_name: dom.name.local.to_string(),
            prefix: dom.name.prefix.as_ref().map(ToString::to_string),
            namespace: {
                let value = dom.name.ns.to_string();
                (!value.is_empty()).then_some(value)
            },
            value: Some(dom.value.to_string()),
            raw_value: None,
            quote: None,
            name_range,
            value_range: None,
            locator,
        });
    }
    attributes
}

fn hydrate_children(nodes: &mut [HtmlNode]) {
    let mut children = HashMap::<String, Vec<String>>::new();
    for node in nodes.iter() {
        if let Some(parent) = &node.parent_id {
            children
                .entry(parent.clone())
                .or_default()
                .push(node.id.clone());
        }
    }
    for node in nodes {
        node.children = children.remove(&node.id).unwrap_or_default();
    }
}

fn htmx_attribute(
    element_id: &str,
    tag_name: &str,
    attribute: &HtmlAttribute,
) -> Option<HtmxAttribute> {
    let lower = attribute.name.to_ascii_lowercase();
    let normalized_name = if lower.starts_with("hx-") {
        lower
    } else if let Some(stripped) = lower.strip_prefix("data-hx-") {
        format!("hx-{stripped}")
    } else {
        return None;
    };
    let range = attribute
        .value_range
        .as_ref()
        .map(|value| SourceRange {
            byte_start: attribute.name_range.byte_start,
            byte_end: value.byte_end,
            start_line: attribute.name_range.start_line,
            start_column: attribute.name_range.start_column,
            end_line: value.end_line,
            end_column: value.end_column,
        })
        .unwrap_or_else(|| attribute.name_range.clone());
    Some(HtmxAttribute {
        element_id: element_id.to_string(),
        tag_name: tag_name.to_string(),
        name: attribute.name.clone(),
        normalized_name,
        value: attribute.value.clone(),
        locator: SourceLocator::exact(range.clone()).expect("valid htmx attribute range"),
        range,
    })
}

fn validate_xhtml(
    text: &str,
    line_index: &LineIndex,
    tokens: &[HtmlSourceToken],
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut stack = Vec::<&HtmlSourceToken>::new();
    let has_namespace = tokens
        .iter()
        .filter(|token| token.kind == HtmlSourceTokenKind::StartTag)
        .flat_map(|token| &token.attributes)
        .any(|attribute| {
            attribute.name == "xmlns"
                && attribute.value.as_deref() == Some("http://www.w3.org/1999/xhtml")
        });
    if !has_namespace {
        diagnostics.push(
            Diagnostic::malformed(
                "grist.html",
                "XHTML root does not declare the XHTML namespace",
            )
            .with_explanation_key("xhtml.namespace_missing")
            .partial(),
        );
    }

    for token in tokens {
        match token.kind {
            HtmlSourceTokenKind::StartTag => {
                let mut names = HashSet::new();
                for attribute in &token.attributes {
                    if attribute.value.is_some() && attribute.quote.is_none() {
                        diagnostics.push(
                            malformed_at(
                                "xhtml.attribute_unquoted",
                                format!(
                                    "XHTML attribute {} must use a quoted value",
                                    attribute.name
                                ),
                                attribute.name_range.clone(),
                            )
                            .partial(),
                        );
                    }
                    if !names.insert(attribute.name.as_str()) {
                        diagnostics.push(
                            malformed_at(
                                "xhtml.attribute_duplicate",
                                format!("duplicate XHTML attribute {}", attribute.name),
                                attribute.name_range.clone(),
                            )
                            .partial(),
                        );
                    }
                }
                let explicitly_empty = token.raw.trim_end().ends_with("/>");
                if !explicitly_empty {
                    stack.push(token);
                }
            }
            HtmlSourceTokenKind::EndTag => {
                let name = token.name.as_deref().unwrap_or_default();
                match stack.pop() {
                    Some(open) if open.name.as_deref() == Some(name) => {}
                    Some(open) => diagnostics.push(
                        malformed_at(
                            "xhtml.element_mismatch",
                            format!(
                                "XHTML closing tag </{name}> does not match <{}>",
                                open.name.as_deref().unwrap_or_default()
                            ),
                            token.range.clone(),
                        )
                        .partial(),
                    ),
                    None => diagnostics.push(
                        malformed_at(
                            "xhtml.end_tag_unmatched",
                            format!("XHTML closing tag </{name}> has no open element"),
                            token.range.clone(),
                        )
                        .partial(),
                    ),
                }
            }
            HtmlSourceTokenKind::Doctype
                if token.raw.to_ascii_lowercase().contains(" system ")
                    || token.raw.to_ascii_lowercase().contains(" public ") =>
            {
                diagnostics.push(
                    Diagnostic::info(
                        "grist.html",
                        "xhtml.external_identifier_inert",
                        "XHTML external identifier was retained but never resolved",
                    )
                    .with_range(token.range.clone())
                    .with_locator(token.locator.clone())
                    .with_explanation_key("xhtml.external_identifier_inert"),
                );
            }
            _ => {}
        }
    }
    for token in stack {
        let name = token.name.as_deref().unwrap_or_default();
        diagnostics.push(
            malformed_at(
                "xhtml.element_unclosed",
                format!("XHTML element <{name}> is not closed"),
                token.range.clone(),
            )
            .partial(),
        );
    }
    if text.is_empty() {
        diagnostics.push(
            malformed_at(
                "xhtml.empty_document",
                "XHTML document is empty",
                SourceRange::new(0, 0, line_index),
            )
            .partial(),
        );
    }
    diagnostics
}

fn attr<'a>(node: &'a HtmlNode, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|attribute| {
            attribute.name.eq_ignore_ascii_case(name)
                || attribute.local_name.eq_ignore_ascii_case(name)
        })
        .and_then(|attribute| attribute.value.as_deref())
}

fn node_index(nodes: &[HtmlNode]) -> HashMap<&str, usize> {
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect()
}

fn descendant_text(nodes: &[HtmlNode], root_id: &str) -> String {
    fn append(
        nodes: &[HtmlNode],
        index: &HashMap<&str, usize>,
        node_id: &str,
        output: &mut String,
    ) {
        let Some(node) = index.get(node_id).and_then(|position| nodes.get(*position)) else {
            return;
        };
        if node.kind == HtmlNodeKind::Text
            && let Some(text) = &node.text
        {
            output.push_str(text);
        }
        for child in &node.children {
            append(nodes, index, child, output);
        }
    }
    let index = node_index(nodes);
    let mut output = String::new();
    append(nodes, &index, root_id, &mut output);
    output
}

fn extract_metadata(nodes: &[HtmlNode]) -> Vec<HtmlMetadata> {
    let mut metadata = Vec::new();
    for node in nodes {
        let Some(tag) = node.tag_name.as_deref() else {
            continue;
        };
        match tag {
            "title" => metadata.push(HtmlMetadata {
                node_id: node.id.clone(),
                kind: "title".to_string(),
                name: None,
                property: None,
                content: None,
                value: Some(descendant_text(nodes, &node.id)),
                locator: node.locator.clone(),
            }),
            "meta" => metadata.push(HtmlMetadata {
                node_id: node.id.clone(),
                kind: "meta".to_string(),
                name: attr(node, "name")
                    .or_else(|| attr(node, "http-equiv"))
                    .map(str::to_string),
                property: attr(node, "property").map(str::to_string),
                content: attr(node, "content").map(str::to_string),
                value: attr(node, "charset").map(str::to_string),
                locator: node.locator.clone(),
            }),
            "html" => {
                for name in ["lang", "dir"] {
                    if let Some(value) = attr(node, name) {
                        metadata.push(HtmlMetadata {
                            node_id: node.id.clone(),
                            kind: format!("document_{name}"),
                            name: Some(name.to_string()),
                            property: None,
                            content: None,
                            value: Some(value.to_string()),
                            locator: node.locator.clone(),
                        });
                    }
                }
            }
            "base" => {
                if let Some(value) = attr(node, "href") {
                    metadata.push(HtmlMetadata {
                        node_id: node.id.clone(),
                        kind: "base".to_string(),
                        name: Some("href".to_string()),
                        property: None,
                        content: None,
                        value: Some(value.to_string()),
                        locator: node.locator.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    metadata
}

fn extract_links(nodes: &[HtmlNode]) -> Vec<HtmlLink> {
    let mut links = Vec::new();
    for node in nodes {
        let Some(tag_name) = node.tag_name.as_deref() else {
            continue;
        };
        for attribute_name in ["href", "src", "action", "formaction", "xlink:href"] {
            let Some(destination) = attr(node, attribute_name) else {
                continue;
            };
            let normalized = destination.trim().to_ascii_lowercase();
            links.push(HtmlLink {
                node_id: node.id.clone(),
                tag_name: tag_name.to_string(),
                attribute: attribute_name.to_string(),
                destination: destination.to_string(),
                rel: attr(node, "rel")
                    .map(|value| value.split_ascii_whitespace().map(str::to_string).collect())
                    .unwrap_or_default(),
                remote: normalized.starts_with("http://")
                    || normalized.starts_with("https://")
                    || normalized.starts_with("//"),
                active_scheme: is_active_url(normalized.as_str()),
                locator: node.locator.clone(),
            });
        }
    }
    links
}

fn extract_tables(nodes: &[HtmlNode]) -> Vec<HtmlTable> {
    let by_id = node_index(nodes);
    nodes
        .iter()
        .filter(|node| node.tag_name.as_deref() == Some("table"))
        .map(|table| {
            let rows = nodes
                .iter()
                .filter(|node| {
                    node.tag_name.as_deref() == Some("tr")
                        && nearest_ancestor_tag(node, nodes, &by_id, "table")
                            == Some(table.id.as_str())
                })
                .map(|row| {
                    let cells = row
                        .children
                        .iter()
                        .filter_map(|id| by_id.get(id.as_str()).and_then(|index| nodes.get(*index)))
                        .filter(|node| matches!(node.tag_name.as_deref(), Some("td" | "th")))
                        .map(|cell| HtmlTableCell {
                            node_id: cell.id.clone(),
                            text: descendant_text(nodes, &cell.id),
                            header: cell.tag_name.as_deref() == Some("th"),
                            row_span: positive_span(attr(cell, "rowspan")),
                            column_span: positive_span(attr(cell, "colspan")),
                            locator: cell.locator.clone(),
                        })
                        .collect::<Vec<_>>();
                    HtmlTableRow {
                        node_id: row.id.clone(),
                        header: !cells.is_empty() && cells.iter().all(|cell| cell.header),
                        cells,
                        locator: row.locator.clone(),
                    }
                })
                .collect();
            HtmlTable {
                node_id: table.id.clone(),
                rows,
                locator: table.locator.clone(),
            }
        })
        .collect()
}

fn nearest_ancestor_tag<'a>(
    node: &'a HtmlNode,
    nodes: &'a [HtmlNode],
    by_id: &HashMap<&str, usize>,
    tag: &str,
) -> Option<&'a str> {
    let mut parent = node.parent_id.as_deref();
    while let Some(parent_id) = parent {
        let candidate = by_id.get(parent_id).and_then(|index| nodes.get(*index))?;
        if candidate.tag_name.as_deref() == Some(tag) {
            return Some(candidate.id.as_str());
        }
        parent = candidate.parent_id.as_deref();
    }
    None
}

fn positive_span(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1)
}

fn extract_media(nodes: &[HtmlNode]) -> Vec<HtmlMediaReference> {
    nodes
        .iter()
        .filter_map(|node| {
            let tag = node.tag_name.as_deref()?;
            matches!(
                tag,
                "img"
                    | "picture"
                    | "audio"
                    | "video"
                    | "source"
                    | "track"
                    | "svg"
                    | "canvas"
                    | "object"
                    | "embed"
                    | "iframe"
            )
            .then(|| {
                let mut sources = ["src", "srcset", "poster", "data", "href", "xlink:href"]
                    .into_iter()
                    .filter_map(|name| attr(node, name).map(str::to_string))
                    .collect::<Vec<_>>();
                sources.dedup();
                HtmlMediaReference {
                    node_id: node.id.clone(),
                    tag_name: tag.to_string(),
                    sources,
                    media_type: attr(node, "type").map(str::to_string),
                    alt_text: attr(node, "alt")
                        .or_else(|| attr(node, "aria-label"))
                        .map(str::to_string),
                    locator: node.locator.clone(),
                }
            })
        })
        .collect()
}

fn extract_sections(nodes: &[HtmlNode]) -> Vec<HtmlSection> {
    nodes
        .iter()
        .filter_map(|node| {
            let tag = node.tag_name.as_deref()?;
            (is_semantic_section(tag) || is_heading(tag)).then(|| HtmlSection {
                node_id: node.id.clone(),
                kind: tag.to_string(),
                heading: if is_heading(tag) {
                    Some(descendant_text(nodes, &node.id))
                } else {
                    first_descendant_heading(nodes, node)
                },
                label: attr(node, "aria-label")
                    .or_else(|| attr(node, "id"))
                    .map(str::to_string),
                locator: node.locator.clone(),
            })
        })
        .collect()
}

fn first_descendant_heading(nodes: &[HtmlNode], root: &HtmlNode) -> Option<String> {
    nodes
        .iter()
        .find(|candidate| {
            candidate
                .dom_path
                .starts_with(&(root.dom_path.clone() + "/"))
                && candidate.tag_name.as_deref().is_some_and(is_heading)
        })
        .map(|heading| descendant_text(nodes, &heading.id))
}

fn extract_active_content(nodes: &[HtmlNode]) -> Vec<HtmlActiveContent> {
    let mut active = Vec::new();
    for node in nodes {
        let Some(tag) = node.tag_name.as_deref() else {
            continue;
        };
        let element_kind = match tag {
            "script" => Some(HtmlActiveContentKind::Script),
            "style" => Some(HtmlActiveContentKind::Style),
            "form" | "input" | "button" | "select" | "textarea" | "option" => {
                Some(HtmlActiveContentKind::Form)
            }
            "iframe" | "frame" => Some(HtmlActiveContentKind::EmbeddedBrowsingContext),
            "object" | "embed" | "applet" => Some(HtmlActiveContentKind::PluginObject),
            "meta"
                if attr(node, "http-equiv")
                    .is_some_and(|value| value.eq_ignore_ascii_case("refresh")) =>
            {
                Some(HtmlActiveContentKind::Refresh)
            }
            _ => None,
        };
        if let Some(kind) = element_kind {
            active.push(active_record(node, kind, node.raw.clone()));
        }
        for attribute in &node.attributes {
            if attribute.name.to_ascii_lowercase().starts_with("on") {
                active.push(active_record(
                    node,
                    HtmlActiveContentKind::InlineEventHandler,
                    format!(
                        "{}={}",
                        attribute.name,
                        attribute.value.as_deref().unwrap_or_default()
                    ),
                ));
            }
            if attribute
                .value
                .as_deref()
                .is_some_and(|value| is_active_url(&value.trim().to_ascii_lowercase()))
            {
                active.push(active_record(
                    node,
                    HtmlActiveContentKind::JavascriptUrl,
                    format!(
                        "{}={}",
                        attribute.name,
                        attribute.value.as_deref().unwrap_or_default()
                    ),
                ));
            }
        }
    }
    active
}

fn active_record(
    node: &HtmlNode,
    kind: HtmlActiveContentKind,
    source: String,
) -> HtmlActiveContent {
    HtmlActiveContent {
        node_id: node.id.clone(),
        kind,
        source,
        disposition: HtmlActiveContentDisposition::Inert,
        locator: node.locator.clone(),
    }
}

fn is_active_url(value: &str) -> bool {
    value.starts_with("javascript:")
        || value.starts_with("vbscript:")
        || value.starts_with("data:text/html")
}

fn is_heading(tag: &str) -> bool {
    matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn is_semantic_section(tag: &str) -> bool {
    matches!(
        tag,
        "section"
            | "article"
            | "nav"
            | "main"
            | "aside"
            | "header"
            | "footer"
            | "address"
            | "search"
    )
}

fn known_html_element(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "abbr"
            | "address"
            | "area"
            | "article"
            | "aside"
            | "audio"
            | "b"
            | "base"
            | "bdi"
            | "bdo"
            | "blockquote"
            | "body"
            | "br"
            | "button"
            | "canvas"
            | "caption"
            | "cite"
            | "code"
            | "col"
            | "colgroup"
            | "data"
            | "datalist"
            | "dd"
            | "del"
            | "details"
            | "dfn"
            | "dialog"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "embed"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "header"
            | "hgroup"
            | "hr"
            | "html"
            | "i"
            | "iframe"
            | "img"
            | "input"
            | "ins"
            | "kbd"
            | "label"
            | "legend"
            | "li"
            | "link"
            | "main"
            | "map"
            | "mark"
            | "menu"
            | "meta"
            | "meter"
            | "nav"
            | "noscript"
            | "object"
            | "ol"
            | "optgroup"
            | "option"
            | "output"
            | "p"
            | "picture"
            | "pre"
            | "progress"
            | "q"
            | "rp"
            | "rt"
            | "ruby"
            | "s"
            | "samp"
            | "script"
            | "search"
            | "section"
            | "select"
            | "slot"
            | "small"
            | "source"
            | "span"
            | "strong"
            | "style"
            | "sub"
            | "summary"
            | "sup"
            | "table"
            | "tbody"
            | "td"
            | "template"
            | "textarea"
            | "tfoot"
            | "th"
            | "thead"
            | "time"
            | "title"
            | "tr"
            | "track"
            | "u"
            | "ul"
            | "var"
            | "video"
            | "wbr"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html5_tree_preserves_semantics_and_inert_active_content() {
        let source = r#"<!doctype html><html lang="en"><head><title>Example</title><meta name="author" content="Ada"></head><body><main><h1>Heading</h1><table><tr><th>A</th><td rowspan="2">B</td></tr></table><img src="figure.png" alt="Figure"><script>window.never_runs = true;</script><form action="https://network.invalid/post"><button onclick="never()">Go</button></form><custom-widget data-x="1">raw</custom-widget></main></body></html>"#;
        let envelope = parse_html(
            source,
            SourceInfo::stdin("rich.html"),
            &HtmlOptions::default(),
        );
        assert_eq!(
            envelope.status,
            OperationStatus::Complete,
            "{:#?}",
            envelope.diagnostics
        );
        let document = envelope.payload.expect("HTML payload");
        assert_eq!(document.mode, HtmlParseMode::Document);
        assert_eq!(document.syntax, HtmlSyntax::Html5);
        assert!(!document.tables[0].rows[0].cells.is_empty());
        assert!(document.media.iter().any(|media| media.tag_name == "img"));
        assert!(
            document
                .sections
                .iter()
                .any(|section| section.heading.as_deref() == Some("Heading"))
        );
        assert!(document.active_content.len() >= 3);
        assert!(
            document
                .active_content
                .iter()
                .all(|content| content.disposition == HtmlActiveContentDisposition::Inert)
        );
        assert!(
            document
                .nodes
                .iter()
                .any(|node| node.tag_name.as_deref() == Some("custom-widget")
                    && !node.known_element
                    && node.raw.contains("<custom-widget"))
        );
    }

    #[test]
    fn fragment_recovery_is_partial_and_dom_order_is_explicit() {
        let envelope = parse_html(
            "<table><td>cell</table><p>after",
            SourceInfo::stdin("fragment.html"),
            &HtmlOptions {
                mode: HtmlParseMode::Fragment,
                ..Default::default()
            },
        );
        assert_eq!(envelope.status, OperationStatus::Partial);
        let document = envelope.payload.expect("recovered HTML payload");
        assert!(
            envelope
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.explanation_key.as_deref()
                    == Some("html.tree_recovery"))
        );
        assert!(
            document
                .nodes
                .iter()
                .all(|node| node.locator.validate().is_ok())
        );
        let ordinals = document
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<HashMap<_, _>>();
        assert!(document.nodes.iter().all(|node| {
            node.parent_id
                .as_ref()
                .is_none_or(|parent| ordinals[parent] < ordinals[&node.id])
        }));
    }

    #[test]
    fn xhtml_and_declared_encoding_are_retained() {
        let mut xhtml = br#"<?xml version="1.0" encoding="windows-1252"?><html xmlns="http://www.w3.org/1999/xhtml"><body><p title="caf&#233;">caf"#.to_vec();
        xhtml.push(0xe9);
        xhtml.extend_from_slice(b"</p></body></html>");
        let envelope = parse_html_bytes(
            &xhtml,
            SourceInfo::stdin("document.xhtml").with_declared_mime_type("application/xhtml+xml"),
            &HtmlOptions::default(),
        );
        assert_eq!(
            envelope.status,
            OperationStatus::Complete,
            "{:#?}",
            envelope.diagnostics
        );
        let document = envelope.payload.expect("XHTML payload");
        assert_eq!(document.syntax, HtmlSyntax::Xhtml);
        assert_eq!(document.encoding, TextEncoding::Windows1252);
        assert!(
            document
                .nodes
                .iter()
                .filter_map(|node| node.namespace.as_deref())
                .any(|namespace| namespace == "http://www.w3.org/1999/xhtml")
        );
        assert!(
            document
                .source_tokens
                .iter()
                .any(|token| token.kind == HtmlSourceTokenKind::ProcessingInstruction)
        );
    }
}
