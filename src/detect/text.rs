use super::Signal;
use crate::core::{DetectionEvidenceKind, Diagnostic};
use crate::decode::{DecodeContext, DecodeOptions, decode_text};

pub(super) struct TextSample {
    pub text: String,
    charset: String,
    bom: bool,
    lossy: bool,
}

pub(super) fn sample(bytes: &[u8], max_bytes: usize) -> Option<TextSample> {
    let prefix = &bytes[..bytes.len().min(max_bytes)];
    let has_bom = prefix.starts_with(&[0x00, 0x00, 0xfe, 0xff])
        || prefix.starts_with(&[0xff, 0xfe, 0x00, 0x00])
        || prefix.starts_with(&[0xfe, 0xff])
        || prefix.starts_with(&[0xff, 0xfe])
        || prefix.starts_with(&[0xef, 0xbb, 0xbf]);
    if !has_bom && binary_like(prefix) {
        return None;
    }
    if !has_bom && std::str::from_utf8(prefix).is_err() && !windows_1252_text_like(prefix) {
        return None;
    }
    let decoded = decode_text(
        prefix,
        &DecodeOptions {
            context: DecodeContext::PlainText,
            transport_encoding: None,
        },
    )
    .ok()?;
    Some(TextSample {
        charset: decoded.report.encoding.label().to_string(),
        bom: decoded.report.bom.is_some(),
        lossy: decoded.report.is_lossy(),
        text: decoded.text,
    })
}

pub(super) fn signals(
    bytes: &[u8],
    max_bytes: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Signal> {
    let Some(sample) = sample(bytes, max_bytes) else {
        return if binary_like(&bytes[..bytes.len().min(max_bytes)]) {
            vec![Signal::new(
                "binary",
                Some("application/octet-stream"),
                0.70,
                DetectionEvidenceKind::Fallback,
                "binary-like control-byte distribution",
            )]
        } else {
            Vec::new()
        };
    };
    if sample.lossy {
        diagnostics.push(
            Diagnostic::warning(
                "grist.detect",
                "detect.charset.lossy_probe",
                format!(
                    "{} probe prefix required replacement characters",
                    sample.charset
                ),
            )
            .partial(),
        );
    }
    let mut signals = structure_signals(&sample.text);
    signals.extend(shebang_signals(&sample.text));
    let charset_description = if sample.bom {
        format!("{} BOM", sample.charset)
    } else {
        format!("valid {} text prefix", sample.charset)
    };
    if signals.is_empty() {
        signals.push(Signal::new(
            "text",
            Some("text/plain"),
            if sample.bom { 0.55 } else { 0.44 },
            DetectionEvidenceKind::Charset,
            charset_description,
        ));
    } else {
        let identities = signals
            .iter()
            .map(|signal| signal.identity.clone())
            .collect::<Vec<_>>();
        for identity in identities {
            signals.push(Signal::new(
                &identity.format,
                identity.media_type.as_deref(),
                if sample.bom { 0.24 } else { 0.12 },
                DetectionEvidenceKind::Charset,
                charset_description.clone(),
            ));
        }
    }
    signals
}

fn structure_signals(text: &str) -> Vec<Signal> {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let mut signals = Vec::new();
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        signals.push(structure(
            "json",
            "application/json",
            0.86,
            "valid JSON structure",
        ));
    } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
        signals.push(Signal::new(
            "json",
            Some("application/json"),
            0.34,
            DetectionEvidenceKind::GrammarProbe,
            "JSON opener present but JSON grammar is malformed",
        ));
    }
    let lines = trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(8)
        .collect::<Vec<_>>();
    if lines.len() >= 2
        && lines
            .iter()
            .all(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
    {
        signals.push(structure(
            "jsonl",
            "application/x-ndjson",
            0.88,
            "multiple valid JSON records",
        ));
    }
    if looks_like_html(trimmed) {
        signals.push(structure(
            "html",
            "text/html",
            0.82,
            "HTML document or element structure",
        ));
        add_declared_encoding_signal(&mut signals, "html", "text/html", trimmed);
    } else if trimmed.starts_with("<?xml") || looks_like_xml(trimmed) {
        signals.push(structure("xml", "application/xml", 0.84, "XML declaration"));
        add_declared_encoding_signal(&mut signals, "xml", "application/xml", trimmed);
    }
    if looks_like_csv(&lines) {
        signals.push(structure(
            "csv",
            "text/csv",
            0.66,
            "consistent comma-delimited records",
        ));
    }
    if looks_like_tsv(&lines) {
        signals.push(structure(
            "tsv",
            "text/tab-separated-values",
            0.68,
            "consistent tab-delimited records",
        ));
    }
    if trimmed.starts_with("\\documentclass") || trimmed.contains("\\begin{document}") {
        signals.push(structure(
            "latex",
            "application/x-latex",
            0.84,
            "LaTeX document commands",
        ));
    } else {
        let latex_markers = [
            "\\section{",
            "\\chapter{",
            "\\newcommand",
            "\\begin{equation",
            "\\begin{figure",
            "\\begin{table",
            "\\input{",
            "\\include{",
            "\\cite{",
        ]
        .into_iter()
        .filter(|marker| trimmed.contains(marker))
        .count();
        if latex_markers >= 2 {
            signals.push(structure(
                "latex",
                "application/x-latex",
                0.74,
                "multiple LaTeX structural commands",
            ));
        }
    }
    if looks_like_bibliography(trimmed) {
        signals.push(structure(
            "bibtex",
            "application/x-bibtex",
            0.86,
            "BibTeX or BibLaTeX database entries",
        ));
    }
    if trimmed.starts_with("---") {
        signals.push(structure(
            "yaml",
            "application/yaml",
            0.48,
            "YAML document marker",
        ));
    }
    if looks_like_toml(&lines) {
        signals.push(structure(
            "toml",
            "application/toml",
            0.58,
            "TOML table and assignment structure",
        ));
    }
    if looks_like_asciidoc(&lines) {
        signals.push(structure(
            "asciidoc",
            "text/asciidoc",
            0.78,
            "AsciiDoc heading, attribute, macro, or delimited block",
        ));
    }
    if looks_like_restructured_text(&lines) {
        signals.push(structure(
            "restructured-text",
            "text/x-rst",
            0.76,
            "reStructuredText explicit markup or section adornment",
        ));
    }
    if looks_like_markdown(trimmed) {
        signals.push(structure(
            "markdown",
            "text/markdown",
            0.54,
            "Markdown block structure",
        ));
    }
    signals
}

fn add_declared_encoding_signal(signals: &mut Vec<Signal>, format: &str, media: &str, text: &str) {
    let prefix = text.chars().take(1024).collect::<String>();
    let lower = prefix.to_ascii_lowercase();
    for marker in ["charset=", "encoding="] {
        let Some(offset) = lower.find(marker) else {
            continue;
        };
        let value = prefix[offset + marker.len()..]
            .trim_start_matches([' ', '\'', '"'])
            .split([' ', '\'', '"', '>', ';', '?'])
            .next()
            .unwrap_or_default();
        if !value.is_empty() {
            signals.push(Signal::new(
                format,
                Some(media),
                0.18,
                DetectionEvidenceKind::Charset,
                format!("source declares charset {}", value.to_ascii_lowercase()),
            ));
            break;
        }
    }
}

fn structure(format: &str, media: &str, weight: f32, description: &str) -> Signal {
    Signal::new(
        format,
        Some(media),
        weight,
        DetectionEvidenceKind::Structure,
        description,
    )
}

fn shebang_signals(text: &str) -> Vec<Signal> {
    let Some(line) = text.lines().next().filter(|line| line.starts_with("#!")) else {
        return Vec::new();
    };
    let lower = line.to_ascii_lowercase();
    let identity = if lower.contains("python") {
        Some(("python", "text/x-python"))
    } else if lower.contains("node") || lower.contains("deno") {
        Some(("javascript", "text/javascript"))
    } else if lower.contains("bash") || lower.contains("/sh") {
        Some(("shell", "text/x-shellscript"))
    } else {
        None
    };
    identity
        .map(|(format, media)| {
            vec![Signal::new(
                format,
                Some(media),
                0.88,
                DetectionEvidenceKind::Shebang,
                format!("language shebang {}", line.trim()),
            )]
        })
        .unwrap_or_default()
}

fn looks_like_html(text: &str) -> bool {
    let prefix = text
        .chars()
        .take(128)
        .collect::<String>()
        .to_ascii_lowercase();
    [
        "<!doctype html",
        "<html",
        "<body",
        "<div",
        "<section",
        "<template",
    ]
    .iter()
    .any(|tag| prefix.starts_with(tag))
}

fn looks_like_xml(text: &str) -> bool {
    let text = text.trim_start();
    if text.starts_with("<!DOCTYPE") {
        return true;
    }
    let Some(rest) = text.strip_prefix('<') else {
        return false;
    };
    if rest.starts_with(['!', '?', '/']) {
        return false;
    }
    let name = rest
        .split(|c: char| c.is_whitespace() || matches!(c, '>' | '/'))
        .next()
        .unwrap_or("");
    if name.is_empty()
        || [
            "html", "head", "body", "title", "meta", "link", "script", "style", "div", "span",
            "section", "main", "nav", "header", "footer", "p", "a", "img", "picture", "video",
            "audio", "form", "input", "button", "table", "thead", "tbody", "tfoot", "tr", "th",
            "td", "ul", "ol", "li", "dl", "dt", "dd", "template",
        ]
        .iter()
        .any(|tag| name.eq_ignore_ascii_case(tag))
    {
        return false;
    }
    text.contains(&format!("</{name}>"))
        || rest.contains("/>")
        || matches!(name, "article" | "book" | "root")
}
fn looks_like_csv(lines: &[&str]) -> bool {
    let Some(first) = lines.first() else {
        return false;
    };
    let columns = first.matches(',').count() + 1;
    columns > 1
        && lines.len() >= 2
        && lines
            .iter()
            .all(|line| line.matches(',').count() + 1 == columns)
}

fn looks_like_tsv(lines: &[&str]) -> bool {
    let Some(first) = lines.first() else {
        return false;
    };
    let columns = first.matches(char::from(9)).count() + 1;
    columns > 1
        && lines.len() >= 2
        && lines
            .iter()
            .all(|line| line.matches(char::from(9)).count() + 1 == columns)
}

fn looks_like_toml(lines: &[&str]) -> bool {
    lines.iter().any(|line| {
        let line = line.trim();
        (line.starts_with('[') && line.ends_with(']'))
            || line.split_once('=').is_some_and(|(key, _)| {
                !key.trim().is_empty() && !key.contains('{') && !key.contains(':')
            })
    })
}

fn looks_like_markdown(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("# ")
            || line.starts_with("## ")
            || line.as_bytes().starts_with(&[0x60, 0x60, 0x60])
    })
}

fn looks_like_bibliography(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let entry = [
        "@article{",
        "@book{",
        "@inproceedings{",
        "@online{",
        "@misc{",
        "@string{",
        "@xdata{",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    entry && (lower.contains("title") || lower.contains("author") || lower.contains("@string"))
}

fn looks_like_asciidoc(lines: &[&str]) -> bool {
    lines.iter().any(|line| {
        let line = line.trim();
        let heading = line
            .find(' ')
            .map(|offset| &line[..offset])
            .is_some_and(|marker| {
                !marker.is_empty() && marker.len() <= 6 && marker.bytes().all(|value| value == b'=')
            });
        heading
            || line.starts_with("include::")
            || line == "|==="
            || matches!(line, "----" | "...." | "++++")
            || (line.starts_with(':') && line[1..].contains(':'))
            || (line.contains("::") && line.ends_with(']'))
    })
}
fn looks_like_restructured_text(lines: &[&str]) -> bool {
    if lines.iter().any(|line| {
        let line = line.trim_start();
        line.starts_with(".. ")
            && (line.contains("::") || line.starts_with(".. _") || line.starts_with(".. ["))
    }) {
        return true;
    }
    lines.windows(2).any(|pair| {
        let title = pair[0].trim();
        let underline = pair[1].trim();
        let Some(marker) = underline.chars().next() else {
            return false;
        };
        !title.is_empty()
            && underline.chars().count() >= 3
            && !marker.is_ascii_alphanumeric()
            && !marker.is_whitespace()
            && underline.chars().all(|value| value == marker)
            && !matches!(marker, '-' | '`')
    })
}

fn binary_like(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.contains(&0) {
        return true;
    }
    let controls = bytes
        .iter()
        .filter(|byte| **byte < 0x09 || (**byte > 0x0d && **byte < 0x20))
        .count();
    controls > bytes.len() / 20
}

fn windows_1252_text_like(bytes: &[u8]) -> bool {
    let high_bytes = bytes.iter().filter(|byte| **byte >= 0x80).count();
    let printable = bytes
        .iter()
        .filter(|byte| {
            byte.is_ascii_graphic()
                || matches!(**byte, b' ' | b'\t' | b'\r' | b'\n')
                || **byte >= 0x80
        })
        .count();
    high_bytes.saturating_mul(4) <= bytes.len()
        && printable.saturating_mul(20) >= bytes.len().saturating_mul(19)
}
