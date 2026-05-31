use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, LineIndex, ParserInfo, SchemaVersion, SourceInfo,
    SourceRange,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlDocument {
    pub schema_version: String,
    pub mode: HtmlParseMode,
    pub nodes: Vec<HtmlNode>,
    pub htmx_attributes: Vec<HtmxAttribute>,
    pub root_element_count: usize,
    pub has_doctype: bool,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlNode {
    pub id: String,
    pub kind: HtmlNodeKind,
    pub range: SourceRange,
    pub depth: usize,
    pub tag_name: Option<String>,
    pub attributes: Vec<HtmlAttribute>,
    pub htmx_attributes: Vec<HtmxAttribute>,
    pub text: Option<String>,
    pub self_closing: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlNodeKind {
    ElementOpen,
    ElementClose,
    VoidElement,
    Text,
    Comment,
    Doctype,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlAttribute {
    pub name: String,
    pub value: Option<String>,
    pub name_range: SourceRange,
    pub value_range: Option<SourceRange>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmxAttribute {
    pub element_id: String,
    pub tag_name: String,
    pub name: String,
    pub normalized_name: String,
    pub value: Option<String>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, Copy)]
pub struct HtmlOptions {
    pub mode: HtmlParseMode,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            mode: HtmlParseMode::Auto,
        }
    }
}

pub type HtmlEnvelope = Envelope<HtmlDocument>;

pub fn parse_html(text: &str, source: SourceInfo, options: &HtmlOptions) -> HtmlEnvelope {
    let line_index = LineIndex::new(text);
    let resolved_mode = resolve_mode(text, options.mode);
    let mut parser = HtmlParser {
        text,
        line_index: &line_index,
        nodes: Vec::new(),
        diagnostics: Vec::new(),
        open_elements: Vec::new(),
        root_element_count: 0,
        has_doctype: false,
    };
    parser.parse();
    parser.finish(resolved_mode);

    let htmx_attributes = parser
        .nodes
        .iter()
        .flat_map(|node| node.htmx_attributes.iter().cloned())
        .collect();

    Envelope::new(
        ArtifactKind::Html,
        source,
        ParserInfo::new("grist.html"),
        SchemaVersion::HTML_V1,
        HtmlDocument {
            schema_version: SchemaVersion::HTML_V1.to_string(),
            mode: resolved_mode,
            nodes: parser.nodes,
            htmx_attributes,
            root_element_count: parser.root_element_count,
            has_doctype: parser.has_doctype,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(parser.diagnostics)
}

struct HtmlParser<'a> {
    text: &'a str,
    line_index: &'a LineIndex,
    nodes: Vec<HtmlNode>,
    diagnostics: Vec<Diagnostic>,
    open_elements: Vec<OpenElement>,
    root_element_count: usize,
    has_doctype: bool,
}

#[derive(Debug, Clone)]
struct OpenElement {
    tag_name: String,
    range: SourceRange,
}

impl HtmlParser<'_> {
    fn parse(&mut self) {
        let mut cursor = 0;
        while cursor < self.text.len() {
            let Some(relative_tag_start) = self.text[cursor..].find('<') else {
                self.push_text(cursor, self.text.len());
                break;
            };
            let tag_start = cursor + relative_tag_start;
            self.push_text(cursor, tag_start);

            if self.text[tag_start..].starts_with("<!--") {
                cursor = self.parse_comment(tag_start);
                continue;
            }
            let Some(relative_tag_end) = self.text[tag_start..].find('>') else {
                self.diagnostics.push(
                    Diagnostic::error(
                        "grist.html",
                        "html.unclosed_tag",
                        "HTML tag start was found without a closing `>`",
                    )
                    .with_range(SourceRange::new(
                        tag_start,
                        self.text.len(),
                        self.line_index,
                    ))
                    .partial(),
                );
                self.push_text(tag_start, self.text.len());
                break;
            };
            let tag_end = tag_start + relative_tag_end + 1;
            self.parse_tag(tag_start, tag_end);
            cursor = tag_end;
        }
    }

    fn finish(&mut self, mode: HtmlParseMode) {
        while let Some(open_element) = self.open_elements.pop() {
            self.diagnostics.push(
                Diagnostic::warning(
                    "grist.html",
                    "html.unclosed_element",
                    format!("element <{}> was not closed", open_element.tag_name),
                )
                .with_range(open_element.range),
            );
        }
        if mode == HtmlParseMode::Document {
            let has_html = self
                .nodes
                .iter()
                .any(|node| node.tag_name.as_deref() == Some("html"));
            if !self.has_doctype {
                self.diagnostics.push(Diagnostic::warning(
                    "grist.html",
                    "html.document_missing_doctype",
                    "document mode input does not include a doctype",
                ));
            }
            if !has_html {
                self.diagnostics.push(Diagnostic::warning(
                    "grist.html",
                    "html.document_missing_html_element",
                    "document mode input does not include an html element",
                ));
            }
        }
    }

    fn parse_comment(&mut self, start: usize) -> usize {
        if let Some(relative_end) = self.text[start + 4..].find("-->") {
            let end = start + 4 + relative_end + 3;
            let comment_text = self.text[start + 4..end - 3].to_string();
            self.nodes.push(HtmlNode {
                id: format!("html-node-{}", self.nodes.len()),
                kind: HtmlNodeKind::Comment,
                range: SourceRange::new(start, end, self.line_index),
                depth: self.open_elements.len(),
                tag_name: None,
                attributes: Vec::new(),
                htmx_attributes: Vec::new(),
                text: Some(comment_text),
                self_closing: false,
            });
            end
        } else {
            self.diagnostics.push(
                Diagnostic::warning(
                    "grist.html",
                    "html.unclosed_comment",
                    "HTML comment start was found without a closing marker",
                )
                .with_range(SourceRange::new(start, self.text.len(), self.line_index))
                .partial(),
            );
            self.push_text(start, self.text.len());
            self.text.len()
        }
    }

    fn parse_tag(&mut self, start: usize, end: usize) {
        let raw_inner = &self.text[start + 1..end - 1];
        let trimmed_inner = raw_inner.trim();
        if trimmed_inner.is_empty() {
            self.push_text(start, end);
            return;
        }
        if trimmed_inner.starts_with('!') {
            self.parse_bang_tag(start, end, trimmed_inner);
            return;
        }
        if trimmed_inner.starts_with('/') {
            self.parse_close_tag(start, end, trimmed_inner);
            return;
        }
        self.parse_open_tag(start, end);
    }

    fn parse_bang_tag(&mut self, start: usize, end: usize, inner: &str) {
        let lower_inner = inner.to_ascii_lowercase();
        let kind = if lower_inner.starts_with("!doctype") {
            self.has_doctype = true;
            HtmlNodeKind::Doctype
        } else {
            HtmlNodeKind::Text
        };
        self.nodes.push(HtmlNode {
            id: format!("html-node-{}", self.nodes.len()),
            kind,
            range: SourceRange::new(start, end, self.line_index),
            depth: self.open_elements.len(),
            tag_name: None,
            attributes: Vec::new(),
            htmx_attributes: Vec::new(),
            text: Some(self.text[start..end].to_string()),
            self_closing: false,
        });
    }

    fn parse_close_tag(&mut self, start: usize, end: usize, inner: &str) {
        let tag_name = inner[1..]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches('/')
            .to_ascii_lowercase();
        let mut depth = self.open_elements.len();
        match self.open_elements.pop() {
            Some(open_element) if open_element.tag_name == tag_name => {
                depth = depth.saturating_sub(1);
            }
            Some(open_element) => {
                self.diagnostics.push(
                    Diagnostic::warning(
                        "grist.html",
                        "html.mismatched_close",
                        format!(
                            "closing </{}> did not match open <{}>",
                            tag_name, open_element.tag_name
                        ),
                    )
                    .with_range(SourceRange::new(start, end, self.line_index)),
                );
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::warning(
                        "grist.html",
                        "html.unmatched_close",
                        format!("closing </{}> has no matching open element", tag_name),
                    )
                    .with_range(SourceRange::new(start, end, self.line_index)),
                );
            }
        }
        self.nodes.push(HtmlNode {
            id: format!("html-node-{}", self.nodes.len()),
            kind: HtmlNodeKind::ElementClose,
            range: SourceRange::new(start, end, self.line_index),
            depth,
            tag_name: Some(tag_name),
            attributes: Vec::new(),
            htmx_attributes: Vec::new(),
            text: None,
            self_closing: false,
        });
    }

    fn parse_open_tag(&mut self, start: usize, end: usize) {
        let inner_start = start + 1;
        let inner_end = end - 1;
        let raw_inner = &self.text[inner_start..inner_end];
        let tag_name = read_tag_name(raw_inner).to_ascii_lowercase();
        if tag_name.is_empty() {
            self.push_text(start, end);
            return;
        }
        let self_closing = raw_inner.trim_end().ends_with('/') || is_void_element(&tag_name);
        let attributes = parse_attributes(self.text, inner_start, inner_end, self.line_index);
        let node_id = format!("html-node-{}", self.nodes.len());
        let htmx_attributes = attributes
            .iter()
            .filter_map(|attribute| htmx_attribute(&node_id, &tag_name, attribute, self.line_index))
            .collect::<Vec<_>>();
        let kind = if self_closing {
            HtmlNodeKind::VoidElement
        } else {
            HtmlNodeKind::ElementOpen
        };
        let depth = self.open_elements.len();
        if depth == 0 {
            self.root_element_count += 1;
        }
        let range = SourceRange::new(start, end, self.line_index);
        self.nodes.push(HtmlNode {
            id: node_id,
            kind,
            range: range.clone(),
            depth,
            tag_name: Some(tag_name.clone()),
            attributes,
            htmx_attributes,
            text: None,
            self_closing,
        });
        if !self_closing {
            self.open_elements.push(OpenElement { tag_name, range });
        }
    }

    fn push_text(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let text_value = &self.text[start..end];
        if text_value.trim().is_empty() {
            return;
        }
        self.nodes.push(HtmlNode {
            id: format!("html-node-{}", self.nodes.len()),
            kind: HtmlNodeKind::Text,
            range: SourceRange::new(start, end, self.line_index),
            depth: self.open_elements.len(),
            tag_name: None,
            attributes: Vec::new(),
            htmx_attributes: Vec::new(),
            text: Some(text_value.to_string()),
            self_closing: false,
        });
    }
}

fn resolve_mode(text: &str, requested: HtmlParseMode) -> HtmlParseMode {
    if requested != HtmlParseMode::Auto {
        return requested;
    }
    let prefix = text
        .trim_start()
        .chars()
        .take(128)
        .collect::<String>()
        .to_ascii_lowercase();
    if prefix.starts_with("<!doctype") || prefix.starts_with("<html") {
        HtmlParseMode::Document
    } else {
        HtmlParseMode::Fragment
    }
}

fn read_tag_name(raw_inner: &str) -> &str {
    raw_inner
        .split(|character: char| character.is_whitespace() || character == '/' || character == '>')
        .next()
        .unwrap_or_default()
}

fn parse_attributes(
    source: &str,
    inner_start: usize,
    inner_end: usize,
    line_index: &LineIndex,
) -> Vec<HtmlAttribute> {
    let inner = &source[inner_start..inner_end];
    let tag_name_len = read_tag_name(inner).len();
    let mut cursor = tag_name_len;
    let mut attributes = Vec::new();
    while cursor < inner.len() {
        cursor = skip_ascii_whitespace(inner, cursor);
        if cursor >= inner.len() || inner[cursor..].starts_with('/') {
            break;
        }
        let name_start = cursor;
        while cursor < inner.len() {
            let byte_value = inner.as_bytes()[cursor];
            if byte_value.is_ascii_whitespace() || matches!(byte_value, b'=' | b'/' | b'>') {
                break;
            }
            cursor += 1;
        }
        if cursor == name_start {
            cursor += 1;
            continue;
        }
        let name_end = cursor;
        cursor = skip_ascii_whitespace(inner, cursor);
        let mut value = None;
        let mut value_range = None;
        if cursor < inner.len() && inner.as_bytes()[cursor] == b'=' {
            cursor += 1;
            cursor = skip_ascii_whitespace(inner, cursor);
            if cursor < inner.len() {
                let quote = inner.as_bytes()[cursor];
                if quote == b'"' || quote == b'\'' {
                    cursor += 1;
                    let value_start = cursor;
                    while cursor < inner.len() && inner.as_bytes()[cursor] != quote {
                        cursor += 1;
                    }
                    let value_end = cursor;
                    value = Some(inner[value_start..value_end].to_string());
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
                        let byte_value = inner.as_bytes()[cursor];
                        if byte_value.is_ascii_whitespace() || matches!(byte_value, b'/' | b'>') {
                            break;
                        }
                        cursor += 1;
                    }
                    let value_end = cursor;
                    value = Some(inner[value_start..value_end].to_string());
                    value_range = Some(SourceRange::new(
                        inner_start + value_start,
                        inner_start + value_end,
                        line_index,
                    ));
                }
            }
        }
        attributes.push(HtmlAttribute {
            name: inner[name_start..name_end].to_string(),
            value,
            name_range: SourceRange::new(
                inner_start + name_start,
                inner_start + name_end,
                line_index,
            ),
            value_range,
        });
    }
    attributes
}

fn skip_ascii_whitespace(value: &str, mut cursor: usize) -> usize {
    while cursor < value.len() && value.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn htmx_attribute(
    element_id: &str,
    tag_name: &str,
    attribute: &HtmlAttribute,
    line_index: &LineIndex,
) -> Option<HtmxAttribute> {
    let lower_name = attribute.name.to_ascii_lowercase();
    let normalized_name = if lower_name.starts_with("hx-") {
        lower_name.clone()
    } else if let Some(stripped) = lower_name.strip_prefix("data-hx-") {
        format!("hx-{stripped}")
    } else {
        return None;
    };
    let range = if let Some(value_range) = &attribute.value_range {
        SourceRange::new(
            attribute.name_range.byte_start,
            value_range.byte_end,
            line_index,
        )
    } else {
        attribute.name_range.clone()
    };
    Some(HtmxAttribute {
        element_id: element_id.to_string(),
        tag_name: tag_name.to_string(),
        name: attribute.name.clone(),
        normalized_name,
        value: attribute.value.clone(),
        range,
    })
}

fn is_void_element(tag_name: &str) -> bool {
    matches!(
        tag_name,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_htmx_fragment_attributes() {
        let report = parse_html(
            r##"<button hx-post="/save" hx-target="#result">Save</button><div id="result"></div>"##,
            SourceInfo::stdin("fragment.html"),
            &HtmlOptions {
                mode: HtmlParseMode::Fragment,
            },
        );
        assert_eq!(report.kind, ArtifactKind::Html);
        assert_eq!(report.payload.mode, HtmlParseMode::Fragment);
        assert_eq!(report.payload.htmx_attributes.len(), 2);
        assert_eq!(report.payload.htmx_attributes[0].normalized_name, "hx-post");
    }

    #[test]
    fn parses_document_mode_doctype() {
        let report = parse_html(
            "<!doctype html><html><body>Hello</body></html>",
            SourceInfo::stdin("index.html"),
            &HtmlOptions::default(),
        );
        assert_eq!(report.payload.mode, HtmlParseMode::Document);
        assert!(report.payload.has_doctype);
        assert!(report.diagnostics.is_empty());
    }
}
