use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, LineIndex, ParserInfo, SchemaVersion, SourceInfo,
    SourceRange,
};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
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
    /// Backward-compatible plain text projection of every parsed row.
    pub rows: Vec<Vec<String>>,
    /// Alignment metadata from the delimiter row, one entry per column where available.
    pub alignments: Vec<MarkdownTableAlignment>,
    /// Structured row/cell representation with source ranges where pulldown-cmark exposes them.
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
    pub header: bool,
    pub cells: Vec<MarkdownTableCell>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownTableCell {
    pub range: Option<SourceRange>,
    pub text: String,
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

type TextBlock = (std::ops::Range<usize>, String);
type HeadingBlock = (u8, std::ops::Range<usize>, String);
type CodeBlock = (String, String, std::ops::Range<usize>);
type LinkBlock = (String, String, String, std::ops::Range<usize>);

pub fn parse_markdown(text: &str, source: SourceInfo) -> MarkdownEnvelope {
    let line_index = LineIndex::new(text);
    let (frontmatter, mut diagnostics, body_start) = parse_frontmatter(text, &line_index);
    diagnostics.extend(scan_unclosed_fences(text, body_start, &line_index));

    let mut nodes = Vec::new();
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(&text[body_start..], options).into_offset_iter();
    let mut heading: Option<HeadingBlock> = None;
    let mut paragraph: Option<TextBlock> = None;
    let mut code: Option<CodeBlock> = None;
    let mut link_stack: Vec<LinkBlock> = Vec::new();
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut table_row_details: Vec<MarkdownTableRow> = Vec::new();
    let mut table_alignments: Vec<MarkdownTableAlignment> = Vec::new();
    let mut current_row: Vec<MarkdownTableCell> = Vec::new();
    let mut current_row_range: Option<std::ops::Range<usize>> = None;
    let mut current_cell: Option<TextBlock> = None;
    let mut table_range: Option<std::ops::Range<usize>> = None;
    let mut in_table_head = false;

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
                link_stack.push((
                    dest_url.to_string(),
                    title.to_string(),
                    String::new(),
                    absolute,
                ));
            }
            Event::End(TagEnd::Link) => {
                if let Some((dest, title, text_value, range)) = link_stack.pop() {
                    nodes.push(
                        node(
                            nodes.len(),
                            MarkdownNodeKind::Link,
                            Some(SourceRange::new(range.start, range.end, &line_index)),
                        )
                        .with_text(text_value.clone())
                        .with_destination(dest)
                        .with_title(title),
                    );
                    if let Some((_, _, parent_text, _)) = link_stack.last_mut() {
                        parent_text.push_str(&text_value);
                    }
                }
            }
            Event::Start(Tag::Table(alignments)) => {
                table_rows.clear();
                table_row_details.clear();
                table_alignments = alignments.into_iter().map(markdown_alignment).collect();
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
                        alignments: table_alignments.clone(),
                        row_details: table_row_details.clone(),
                    }),
                );
            }
            Event::Start(Tag::TableHead) => {
                in_table_head = true;
                current_row.clear();
                current_row_range = Some(absolute);
            }
            Event::End(TagEnd::TableHead) => {
                if !current_row.is_empty() {
                    table_rows.push(current_row.iter().map(|cell| cell.text.clone()).collect());
                    let range = current_row_range
                        .take()
                        .map(|range| SourceRange::new(range.start, range.end, &line_index));
                    table_row_details.push(MarkdownTableRow {
                        range,
                        header: true,
                        cells: current_row.clone(),
                    });
                    current_row.clear();
                }
                in_table_head = false;
            }
            Event::Start(Tag::TableRow) => {
                current_row.clear();
                current_row_range = Some(absolute);
            }
            Event::End(TagEnd::TableRow) => {
                table_rows.push(current_row.iter().map(|cell| cell.text.clone()).collect());
                let range = current_row_range
                    .take()
                    .map(|range| SourceRange::new(range.start, range.end, &line_index));
                table_row_details.push(MarkdownTableRow {
                    range,
                    header: in_table_head,
                    cells: current_row.clone(),
                });
            }
            Event::Start(Tag::TableCell) => current_cell = Some((absolute, String::new())),
            Event::End(TagEnd::TableCell) => {
                if let Some((range, text_value)) = current_cell.take() {
                    current_row.push(MarkdownTableCell {
                        range: Some(SourceRange::new(range.start, range.end, &line_index)),
                        text: text_value,
                    });
                }
            }
            Event::Text(value) | Event::Code(value) => append_text(
                &value,
                &mut heading,
                &mut paragraph,
                &mut code,
                &mut link_stack,
                &mut current_cell,
            ),
            Event::SoftBreak | Event::HardBreak => append_text(
                "\n",
                &mut heading,
                &mut paragraph,
                &mut code,
                &mut link_stack,
                &mut current_cell,
            ),
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

fn append_text(
    value: &str,
    heading: &mut Option<HeadingBlock>,
    paragraph: &mut Option<TextBlock>,
    code: &mut Option<CodeBlock>,
    link_stack: &mut [LinkBlock],
    current_cell: &mut Option<TextBlock>,
) {
    if let Some((_, _, text_value)) = heading.as_mut() {
        text_value.push_str(value);
    }
    if let Some((_, text_value)) = paragraph.as_mut() {
        text_value.push_str(value);
    }
    if let Some((_, text_value, _)) = code.as_mut() {
        text_value.push_str(value);
    }
    if let Some((_, _, text_value, _)) = link_stack.last_mut() {
        text_value.push_str(value);
    }
    if let Some((_, text_value)) = current_cell.as_mut() {
        text_value.push_str(value);
    }
}

fn parse_frontmatter(
    text: &str,
    index: &LineIndex,
) -> (Option<Frontmatter>, Vec<Diagnostic>, usize) {
    let mut diagnostics = Vec::new();
    let Some(opening_len) = frontmatter_marker_len_at_start(text) else {
        return (None, diagnostics, 0);
    };

    let mut offset = opening_len;
    while offset < text.len() {
        let line_end = text[offset..]
            .find('\n')
            .map(|relative| offset + relative + 1)
            .unwrap_or(text.len());
        let line = &text[offset..line_end];
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let raw = text[opening_len..offset].to_string();
            let value = match serde_yaml::from_str::<serde_yaml::Value>(&raw) {
                Ok(value) => serde_json::to_value(value).ok(),
                Err(err) => {
                    diagnostics.push(
                        Diagnostic::error(
                            "grist.markdown.frontmatter",
                            "frontmatter.yaml_parse",
                            format!("frontmatter YAML parse failed: {err}"),
                        )
                        .with_range(SourceRange::new(
                            opening_len,
                            offset,
                            index,
                        )),
                    );
                    None
                }
            };
            return (
                Some(Frontmatter {
                    range: SourceRange::new(0, line_end, index),
                    raw,
                    value,
                }),
                diagnostics,
                line_end,
            );
        }
        offset = line_end;
    }

    diagnostics.push(
        Diagnostic::warning(
            "grist.markdown.frontmatter",
            "frontmatter.unclosed",
            "frontmatter start marker was found without a closing marker",
        )
        .with_range(SourceRange::new(0, opening_len, index))
        .partial(),
    );
    (None, diagnostics, 0)
}

fn frontmatter_marker_len_at_start(text: &str) -> Option<usize> {
    if text.starts_with("---\r\n") {
        Some(5)
    } else if text.starts_with("---\n") {
        Some(4)
    } else {
        None
    }
}

fn scan_unclosed_fences(text: &str, body_start: usize, index: &LineIndex) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut open: Option<(u8, usize, usize)> = None;
    let mut offset = body_start;
    while offset < text.len() {
        let line_end = text[offset..]
            .find('\n')
            .map(|relative| offset + relative + 1)
            .unwrap_or(text.len());
        let line = &text[offset..line_end];
        let content = line.trim_end_matches(['\r', '\n']);
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
        let marker_text = std::str::from_utf8(&vec![marker; len])
            .unwrap_or("fence")
            .to_string();
        diagnostics.push(
            Diagnostic::warning(
                "grist.markdown.fence",
                "fence.unclosed",
                format!("fenced code block starting with {marker_text} has no closing fence"),
            )
            .with_range(SourceRange::new(start, start + len, index))
            .partial(),
        );
    }
    diagnostics
}

fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let first = *line.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let len = line.bytes().take_while(|byte| *byte == first).count();
    (len >= 3).then_some((first, len))
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

    fn parse_fixture(text: &str) -> MarkdownEnvelope {
        parse_markdown(text, SourceInfo::stdin("README.md"))
    }

    #[test]
    fn parses_heading_fence_and_frontmatter() {
        let report =
            parse_fixture("---\ntitle: Test\n---\n# Heading\n\n```rust\nfn main() {}\n```");
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

    #[test]
    fn preserves_ranges_for_blocks_and_frontmatter_crlf() {
        let report = parse_fixture("---\r\ntitle: Test\r\n---\r\n# Heading\n\nParagraph text\n");
        let frontmatter = report.payload.frontmatter.as_ref().unwrap();
        assert_eq!(frontmatter.value.as_ref().unwrap()["title"], "Test");
        assert_eq!(frontmatter.range.start_line, 1);
        assert_eq!(frontmatter.range.end_line, 4);

        let heading = report
            .payload
            .nodes
            .iter()
            .find(|node| node.kind == MarkdownNodeKind::Heading)
            .unwrap();
        assert_eq!(heading.text.as_deref(), Some("Heading"));
        assert!(heading.range.is_some());

        let paragraph = report
            .payload
            .nodes
            .iter()
            .find(|node| node.kind == MarkdownNodeKind::Paragraph)
            .unwrap();
        assert_eq!(paragraph.text.as_deref(), Some("Paragraph text"));
        assert!(paragraph.range.is_some());
    }

    #[test]
    fn keeps_link_text_in_paragraph_and_emits_link_node() {
        let report = parse_fixture("See [Grist](https://example.test \"docs\") today.");
        let paragraph = report
            .payload
            .nodes
            .iter()
            .find(|node| node.kind == MarkdownNodeKind::Paragraph)
            .unwrap();
        assert_eq!(paragraph.text.as_deref(), Some("See Grist today."));
        let link = report
            .payload
            .nodes
            .iter()
            .find(|node| node.kind == MarkdownNodeKind::Link)
            .unwrap();
        assert_eq!(link.text.as_deref(), Some("Grist"));
        assert_eq!(link.destination.as_deref(), Some("https://example.test"));
        assert_eq!(link.title.as_deref(), Some("docs"));
        assert!(link.range.is_some());
    }

    #[test]
    fn parses_tables_with_alignment_and_cell_ranges() {
        let report = parse_fixture("| name | score |\n| :--- | ---: |\n| alpha | 1 |\n");
        let table = report
            .payload
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
        assert!(table.row_details[0].cells[0].range.is_some());
    }

    #[test]
    fn reports_yaml_and_unclosed_fence_diagnostics() {
        let report = parse_fixture("---\ntitle: [unterminated\n---\n```rust\nfn main() {}\n");
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.parser == "grist.markdown.frontmatter"
                && diagnostic.code == "frontmatter.yaml_parse"
                && diagnostic.range.is_some()
        }));
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.parser == "grist.markdown.fence"
                && diagnostic.code == "fence.unclosed"
                && diagnostic.partial
                && diagnostic.range.is_some()
        }));
    }
}
