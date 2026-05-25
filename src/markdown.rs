use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, LineIndex, ParserInfo, SchemaVersion, SourceInfo,
    SourceRange,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownDocument {
    pub schema_version: String,
    pub nodes: Vec<MarkdownNode>,
    pub frontmatter: Option<Frontmatter>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownNode {
    pub id: String,
    pub kind: MarkdownNodeKind,
    pub range: Option<SourceRange>,
    pub text: Option<String>,
    pub level: Option<u8>,
    pub language: Option<String>,
    pub info: Option<String>,
    pub destination: Option<String>,
    pub title: Option<String>,
    pub table: Option<MarkdownTable>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownNodeKind {
    Heading,
    Paragraph,
    CodeFence,
    Link,
    Table,
    Text,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownTable {
    pub rows: Vec<Vec<String>>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Frontmatter {
    pub range: SourceRange,
    pub raw: String,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct MarkdownOptions;

pub type MarkdownEnvelope = Envelope<MarkdownDocument>;

pub fn parse_markdown(text: &str, source: SourceInfo) -> MarkdownEnvelope {
    let line_index = LineIndex::new(text);
    let (frontmatter, diagnostics, body_start) = parse_frontmatter(text, &line_index);
    let mut nodes = Vec::new();
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(&text[body_start..], options).into_offset_iter();
    let mut heading: Option<(u8, std::ops::Range<usize>, String)> = None;
    let mut paragraph: Option<(std::ops::Range<usize>, String)> = None;
    let mut code: Option<(String, String, std::ops::Range<usize>)> = None;
    let mut link: Option<(String, String, String, std::ops::Range<usize>)> = None;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_cell = String::new();
    let mut table_range: Option<std::ops::Range<usize>> = None;

    for (event, range) in parser {
        let absolute = (range.start + body_start)..(range.end + body_start);
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((heading_level(level), absolute, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, range, text_value)) = heading.take() {
                    nodes.push(
                        node(
                            nodes.len(),
                            MarkdownNodeKind::Heading,
                            Some(SourceRange::new(range.start, range.end, &line_index)),
                        )
                        .with_text(text_value)
                        .with_level(level),
                    );
                }
            }
            Event::Start(Tag::Paragraph) => paragraph = Some((absolute, String::new())),
            Event::End(TagEnd::Paragraph) => {
                if let Some((range, text_value)) = paragraph.take() {
                    nodes.push(
                        node(
                            nodes.len(),
                            MarkdownNodeKind::Paragraph,
                            Some(SourceRange::new(range.start, range.end, &line_index)),
                        )
                        .with_text(text_value),
                    );
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let info = match kind {
                    CodeBlockKind::Fenced(info) => info.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((info, String::new(), absolute));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((info, text_value, range)) = code.take() {
                    let language = info
                        .split_whitespace()
                        .next()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    nodes.push(
                        node(
                            nodes.len(),
                            MarkdownNodeKind::CodeFence,
                            Some(SourceRange::new(range.start, range.end, &line_index)),
                        )
                        .with_text(text_value)
                        .with_info(info)
                        .with_language(language),
                    );
                }
            }
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) => {
                link = Some((
                    dest_url.to_string(),
                    title.to_string(),
                    String::new(),
                    absolute,
                ));
            }
            Event::End(TagEnd::Link) => {
                if let Some((dest, title, text_value, range)) = link.take() {
                    nodes.push(
                        node(
                            nodes.len(),
                            MarkdownNodeKind::Link,
                            Some(SourceRange::new(range.start, range.end, &line_index)),
                        )
                        .with_text(text_value)
                        .with_destination(dest)
                        .with_title(title),
                    );
                }
            }
            Event::Start(Tag::Table(_)) => {
                table_rows.clear();
                table_range = Some(absolute);
            }
            Event::End(TagEnd::Table) => {
                let range = table_range.take();
                nodes.push(
                    node(
                        nodes.len(),
                        MarkdownNodeKind::Table,
                        range.map(|range| SourceRange::new(range.start, range.end, &line_index)),
                    )
                    .with_table(MarkdownTable {
                        rows: table_rows.clone(),
                    }),
                );
            }
            Event::Start(Tag::TableRow) => current_row.clear(),
            Event::End(TagEnd::TableRow) => table_rows.push(current_row.clone()),
            Event::Start(Tag::TableCell) => current_cell.clear(),
            Event::End(TagEnd::TableCell) => current_row.push(current_cell.clone()),
            Event::Text(value) | Event::Code(value) => {
                let value = value.to_string();
                if let Some((_, _, text_value)) = heading.as_mut() {
                    text_value.push_str(&value);
                } else if let Some((_, text_value)) = paragraph.as_mut() {
                    text_value.push_str(&value);
                } else if let Some((_, text_value, _)) = code.as_mut() {
                    text_value.push_str(&value);
                } else if let Some((_, _, text_value, _)) = link.as_mut() {
                    text_value.push_str(&value);
                }
                if table_range.is_some() {
                    current_cell.push_str(&value);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, text_value)) = paragraph.as_mut() {
                    text_value.push('\n');
                }
                if let Some((_, _, text_value)) = heading.as_mut() {
                    text_value.push('\n');
                }
            }
            _ => {}
        }
    }

    Envelope::new(
        ArtifactKind::Markdown,
        source,
        ParserInfo::new("grist.markdown"),
        SchemaVersion::MARKDOWN_V1,
        MarkdownDocument {
            schema_version: SchemaVersion::MARKDOWN_V1.to_string(),
            nodes,
            frontmatter,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(diagnostics)
}

fn parse_frontmatter(
    text: &str,
    index: &LineIndex,
) -> (Option<Frontmatter>, Vec<Diagnostic>, usize) {
    let mut diagnostics = Vec::new();
    if !text.starts_with("---\n") {
        return (None, diagnostics, 0);
    }
    if let Some(end_relative) = text[4..].find("\n---") {
        let raw_start = 4;
        let raw_end = 4 + end_relative;
        let marker_end = raw_end + 4;
        let raw = text[raw_start..raw_end].to_string();
        let value = match serde_yaml::from_str::<serde_yaml::Value>(&raw) {
            Ok(value) => serde_json::to_value(value).ok(),
            Err(err) => {
                diagnostics.push(Diagnostic::error(
                    "grist.markdown.frontmatter",
                    "frontmatter.yaml_parse",
                    format!("frontmatter YAML parse failed: {err}"),
                ));
                None
            }
        };
        return (
            Some(Frontmatter {
                range: SourceRange::new(0, marker_end, index),
                raw,
                value,
            }),
            diagnostics,
            if text[marker_end..].starts_with('\n') {
                marker_end + 1
            } else {
                marker_end
            },
        );
    }
    diagnostics.push(Diagnostic::warning(
        "grist.markdown.frontmatter",
        "frontmatter.unclosed",
        "frontmatter start marker was found without a closing marker",
    ));
    (None, diagnostics, 0)
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

fn node(id: usize, kind: MarkdownNodeKind, range: Option<SourceRange>) -> MarkdownNode {
    MarkdownNode {
        id: format!("md-node-{id}"),
        kind,
        range,
        text: None,
        level: None,
        language: None,
        info: None,
        destination: None,
        title: None,
        table: None,
    }
}

impl MarkdownNode {
    fn with_text(mut self, text: String) -> Self {
        self.text = Some(text);
        self
    }
    fn with_level(mut self, level: u8) -> Self {
        self.level = Some(level);
        self
    }
    fn with_info(mut self, info: String) -> Self {
        self.info = Some(info);
        self
    }
    fn with_language(mut self, language: Option<String>) -> Self {
        self.language = language;
        self
    }
    fn with_destination(mut self, destination: String) -> Self {
        self.destination = Some(destination);
        self
    }
    fn with_title(mut self, title: String) -> Self {
        if !title.is_empty() {
            self.title = Some(title);
        }
        self
    }
    fn with_table(mut self, table: MarkdownTable) -> Self {
        self.table = Some(table);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heading_fence_and_frontmatter() {
        let report = parse_markdown(
            "---\ntitle: Test\n---\n# Heading\n\n```rust\nfn main() {}\n```",
            SourceInfo::stdin("README.md"),
        );
        assert!(report.payload.frontmatter.is_some());
        assert!(
            report
                .payload
                .nodes
                .iter()
                .any(|n| n.kind == MarkdownNodeKind::Heading)
        );
        assert!(report.payload.nodes.iter().any(
            |n| n.kind == MarkdownNodeKind::CodeFence && n.language.as_deref() == Some("rust")
        ));
    }
}
