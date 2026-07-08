use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, LineIndex, ParserInfo, SchemaVersion, SourceInfo,
    SourceRange,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexDocument {
    pub schema_version: String,
    pub nodes: Vec<LatexNode>,
    pub parse_errors: Vec<LatexParseError>,
    pub detail: Option<LatexSyntaxDetail>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexNode {
    pub id: String,
    pub kind: LatexNodeKind,
    pub range: SourceRange,
    pub command: Option<String>,
    pub name: Option<String>,
    pub argument: Option<String>,
    pub text: Option<String>,
    pub attrs: BTreeMap<String, Value>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LatexNodeKind {
    Section,
    Paragraph,
    Text,
    Command,
    Environment,
    MathInline,
    MathBlock,
    Label,
    Ref,
    Citation,
    Comment,
    RawCommand,
    RawInline,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexParseError {
    pub range: SourceRange,
    pub message: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexSyntaxDetail {
    pub node_count: usize,
    pub raw_command_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LatexDetailMode {
    #[default]
    Semantic,
    SemanticWithSyntax,
}

#[derive(Debug, Clone, Default)]
pub struct LatexOptions {
    pub detail: LatexDetailMode,
}

pub type LatexEnvelope = Envelope<LatexDocument>;

pub fn parse_latex(text: &str, source: SourceInfo, options: &LatexOptions) -> LatexEnvelope {
    let line_index = LineIndex::new(text);
    let mut nodes = Vec::new();
    let mut parse_errors = Vec::new();
    let mut raw_command_count = 0_usize;
    let mut paragraph_start: Option<usize> = None;
    let mut paragraph_text = String::new();

    for line in line_ranges(text) {
        let raw = &text[line.clone()];
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            flush_paragraph(
                text,
                &line_index,
                &mut nodes,
                &mut paragraph_start,
                &mut paragraph_text,
                line.start,
            );
            continue;
        }

        if trimmed.starts_with('%') {
            flush_paragraph(
                text,
                &line_index,
                &mut nodes,
                &mut paragraph_start,
                &mut paragraph_text,
                line.start,
            );
            push_node(
                &mut nodes,
                LatexNodeKind::Comment,
                line.clone(),
                &line_index,
                None,
                None,
                None,
                Some(trimmed.trim_start_matches('%').trim().to_string()),
                BTreeMap::new(),
            );
            continue;
        }

        if let Some((command, arg)) = first_braced_command(
            trimmed,
            &["section", "subsection", "subsubsection", "paragraph"],
        ) {
            flush_paragraph(
                text,
                &line_index,
                &mut nodes,
                &mut paragraph_start,
                &mut paragraph_text,
                line.start,
            );
            let mut attrs = BTreeMap::new();
            attrs.insert("level".into(), Value::from(section_level(command)));
            push_node(
                &mut nodes,
                LatexNodeKind::Section,
                line.clone(),
                &line_index,
                Some(command.to_string()),
                Some(arg.to_string()),
                Some(arg.to_string()),
                Some(arg.to_string()),
                attrs,
            );
            continue;
        }

        if let Some((env, body, end_offset)) = environment_at(text, line.start) {
            flush_paragraph(
                text,
                &line_index,
                &mut nodes,
                &mut paragraph_start,
                &mut paragraph_text,
                line.start,
            );
            let mut attrs = BTreeMap::new();
            attrs.insert("environment".into(), Value::from(env.clone()));
            push_node(
                &mut nodes,
                LatexNodeKind::Environment,
                line.start..end_offset,
                &line_index,
                Some("begin".to_string()),
                Some(env),
                None,
                Some(body),
                attrs,
            );
            continue;
        }

        if trimmed.starts_with("\\[") || trimmed.starts_with("$$") {
            flush_paragraph(
                text,
                &line_index,
                &mut nodes,
                &mut paragraph_start,
                &mut paragraph_text,
                line.start,
            );
            push_node(
                &mut nodes,
                LatexNodeKind::MathBlock,
                line.clone(),
                &line_index,
                None,
                None,
                None,
                Some(trim_math_block(trimmed).to_string()),
                BTreeMap::new(),
            );
            continue;
        }

        if paragraph_start.is_none() {
            paragraph_start = Some(line.start);
        }
        if !paragraph_text.is_empty() {
            paragraph_text.push('\n');
        }
        paragraph_text.push_str(trimmed);

        extract_inline_nodes(
            trimmed,
            line.start + raw.find(trimmed).unwrap_or(0),
            &line_index,
            &mut nodes,
            &mut raw_command_count,
        );
    }

    flush_paragraph(
        text,
        &line_index,
        &mut nodes,
        &mut paragraph_start,
        &mut paragraph_text,
        text.len(),
    );

    if text.matches("\\begin{").count() != text.matches("\\end{").count() {
        parse_errors.push(LatexParseError {
            range: SourceRange::new(0, text.len(), &line_index),
            message: "unbalanced LaTeX begin/end environment markers".to_string(),
        });
    }

    let detail =
        (options.detail == LatexDetailMode::SemanticWithSyntax).then(|| LatexSyntaxDetail {
            node_count: nodes.len(),
            raw_command_count,
        });
    let diagnostics = parse_errors
        .iter()
        .map(|err| {
            Diagnostic::error("grist.latex", "latex.parse", err.message.clone())
                .with_range(err.range.clone())
        })
        .collect::<Vec<_>>();

    Envelope::new(
        ArtifactKind::Latex,
        source,
        ParserInfo::new("grist.latex"),
        SchemaVersion::LATEX_V1,
        LatexDocument {
            schema_version: SchemaVersion::LATEX_V1.to_string(),
            nodes,
            parse_errors,
            detail,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(diagnostics)
}

fn line_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            out.push(start..idx);
            start = idx + 1;
        }
    }
    if start <= text.len() {
        out.push(start..text.len());
    }
    out
}

fn flush_paragraph(
    text: &str,
    line_index: &LineIndex,
    nodes: &mut Vec<LatexNode>,
    paragraph_start: &mut Option<usize>,
    paragraph_text: &mut String,
    end: usize,
) {
    let Some(start) = paragraph_start.take() else {
        return;
    };
    if paragraph_text.trim().is_empty() {
        paragraph_text.clear();
        return;
    }
    push_node(
        nodes,
        LatexNodeKind::Paragraph,
        start..end.min(text.len()),
        line_index,
        None,
        None,
        None,
        Some(paragraph_text.trim().to_string()),
        BTreeMap::new(),
    );
    paragraph_text.clear();
}

fn push_node(
    nodes: &mut Vec<LatexNode>,
    kind: LatexNodeKind,
    range: std::ops::Range<usize>,
    line_index: &LineIndex,
    command: Option<String>,
    name: Option<String>,
    argument: Option<String>,
    text: Option<String>,
    attrs: BTreeMap<String, Value>,
) {
    nodes.push(LatexNode {
        id: format!("latex-node-{}", nodes.len()),
        kind,
        range: SourceRange::new(range.start, range.end, line_index),
        command,
        name,
        argument,
        text,
        attrs,
    });
}

fn first_braced_command<'a>(src: &'a str, allowed: &[&'a str]) -> Option<(&'a str, &'a str)> {
    for command in allowed {
        let prefix = format!("\\{}{{", command);
        if let Some(rest) = src.strip_prefix(&prefix) {
            let end = rest.find('}')?;
            return Some((command, &rest[..end]));
        }
    }
    None
}

fn section_level(command: &str) -> u8 {
    match command {
        "section" => 1,
        "subsection" => 2,
        "subsubsection" => 3,
        "paragraph" => 4,
        _ => 1,
    }
}

fn environment_at(text: &str, start: usize) -> Option<(String, String, usize)> {
    let rest = &text[start..];
    let env_name = rest
        .strip_prefix("\\begin{")?
        .split_once('}')?
        .0
        .to_string();
    let begin_end = format!("\\begin{{{}}}", env_name).len();
    let end_marker = format!("\\end{{{}}}", env_name);
    let end_pos = rest.find(&end_marker)?;
    let body = rest[begin_end..end_pos].trim().to_string();
    Some((env_name, body, start + end_pos + end_marker.len()))
}

fn trim_math_block(src: &str) -> &str {
    src.trim()
        .trim_start_matches("\\[")
        .trim_end_matches("\\]")
        .trim_start_matches("$$")
        .trim_end_matches("$$")
        .trim()
}

fn extract_inline_nodes(
    src: &str,
    byte_offset: usize,
    line_index: &LineIndex,
    nodes: &mut Vec<LatexNode>,
    raw_command_count: &mut usize,
) {
    for (kind, command) in [
        (LatexNodeKind::Label, "label"),
        (LatexNodeKind::Ref, "ref"),
        (LatexNodeKind::Citation, "cite"),
        (LatexNodeKind::Command, "textbf"),
        (LatexNodeKind::Command, "emph"),
    ] {
        let mut search_start = 0;
        let needle = format!("\\{}{{", command);
        while let Some(rel) = src[search_start..].find(&needle) {
            let start = search_start + rel;
            let Some(end_rel) = src[start + needle.len()..].find('}') else {
                break;
            };
            let arg_start = start + needle.len();
            let end = arg_start + end_rel + 1;
            let arg = &src[arg_start..arg_start + end_rel];
            push_node(
                nodes,
                kind.clone(),
                byte_offset + start..byte_offset + end,
                line_index,
                Some(command.to_string()),
                Some(arg.to_string()),
                Some(arg.to_string()),
                Some(arg.to_string()),
                BTreeMap::new(),
            );
            search_start = end;
        }
    }

    let mut search_start = 0;
    while let Some(rel) = src[search_start..].find('$') {
        let start = search_start + rel;
        if src[start..].starts_with("$$") {
            search_start = start + 2;
            continue;
        }
        let Some(end_rel) = src[start + 1..].find('$') else {
            break;
        };
        let end = start + 1 + end_rel + 1;
        push_node(
            nodes,
            LatexNodeKind::MathInline,
            byte_offset + start..byte_offset + end,
            line_index,
            None,
            None,
            None,
            Some(src[start + 1..end - 1].to_string()),
            BTreeMap::new(),
        );
        search_start = end;
    }

    for command in unknown_commands(src) {
        *raw_command_count += 1;
        if let Some(start) = src.find(&format!("\\{}", command)) {
            let after_command = start + command.len() + 1;
            let (end, argument) = if src[after_command..].starts_with('{') {
                if let Some(end_rel) = src[after_command + 1..].find('}') {
                    let arg_start = after_command + 1;
                    let arg_end = arg_start + end_rel;
                    (arg_end + 1, Some(src[arg_start..arg_end].to_string()))
                } else {
                    (after_command, None)
                }
            } else {
                (after_command, None)
            };
            push_node(
                nodes,
                LatexNodeKind::RawCommand,
                byte_offset + start..byte_offset + end,
                line_index,
                Some(command.clone()),
                argument.clone(),
                argument,
                None,
                BTreeMap::new(),
            );
        }
    }
}

fn unknown_commands(src: &str) -> Vec<String> {
    let known = [
        "section",
        "subsection",
        "subsubsection",
        "paragraph",
        "begin",
        "end",
        "label",
        "ref",
        "cite",
        "textbf",
        "emph",
    ];
    let mut out = Vec::new();
    let mut chars = src.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch != '\\' {
            continue;
        }
        let start = idx + 1;
        let mut end = start;
        while let Some((next_idx, next_ch)) = chars.peek().copied() {
            if !next_ch.is_ascii_alphabetic() {
                break;
            }
            end = next_idx + next_ch.len_utf8();
            chars.next();
        }
        if end > start {
            let command = &src[start..end];
            if !known.contains(&command) {
                out.push(command.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latex_sections_refs_math_and_raw_commands() {
        let src = "% intro\n\\section{Intro}\nSee \\textbf{bold} and $x+1$. \\label{sec:intro} \\cite{paper} \\unknowncmd{raw}\n\\[ y = 2 \\]\n\\begin{quote}\nhello\n\\end{quote}\n";
        let envelope = parse_latex(
            src,
            SourceInfo::stdin("paper.tex"),
            &LatexOptions {
                detail: LatexDetailMode::SemanticWithSyntax,
            },
        );
        assert!(envelope.diagnostics.is_empty());
        assert!(
            envelope
                .payload
                .nodes
                .iter()
                .any(|n| n.kind == LatexNodeKind::Comment)
        );
        assert!(envelope.payload.nodes.iter().any(|n| n.kind == LatexNodeKind::Section && n.argument.as_deref() == Some("Intro")));
        assert!(
            envelope
                .payload
                .nodes
                .iter()
                .any(|n| n.kind == LatexNodeKind::MathInline && n.text.as_deref() == Some("x+1"))
        );
        assert!(
            envelope
                .payload
                .nodes
                .iter()
                .any(|n| n.kind == LatexNodeKind::MathBlock)
        );
        assert!(
            envelope
                .payload
                .nodes
                .iter()
                .any(|n| n.kind == LatexNodeKind::Label)
        );
        assert!(
            envelope
                .payload
                .nodes
                .iter()
                .any(|n| n.kind == LatexNodeKind::Citation)
        );
        assert!(envelope.payload.nodes.iter().any(|n| n.kind == LatexNodeKind::Environment && n.name.as_deref() == Some("quote")));
        assert!(
            envelope
                .payload
                .nodes
                .iter()
                .any(|n| n.kind == LatexNodeKind::RawCommand
                    && n.command.as_deref() == Some("unknowncmd"))
        );
        assert_eq!(
            envelope.payload.detail.as_ref().unwrap().raw_command_count,
            1
        );
    }
}
