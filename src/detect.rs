use crate::core::{Diagnostic, Limits};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Source,
    Test,
    Manifest,
    Lockfile,
    Generated,
    Binary,
    Documentation,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Markdown,
    Html,
    Rust,
    Python,
    #[serde(rename = "typescript")]
    TypeScript,
    Tsx,
    Jsx,
    Csv,
    Json,
    Jsonl,
    Yaml,
    Toml,
    Text,
    Binary,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Detection {
    pub file_kind: FileKind,
    pub content_kind: ContentKind,
    pub language: Option<String>,
    pub confidence: f32,
    pub reasons: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Detection {
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            file_kind: FileKind::Unknown,
            content_kind: ContentKind::Unknown,
            language: None,
            confidence: 0.0,
            reasons: vec![reason.into()],
            diagnostics: Vec::new(),
        }
    }
}

pub fn detect_path(path: &Path, bytes: &[u8], limits: &Limits) -> Detection {
    let mut reasons = Vec::new();
    let mut diagnostics = Vec::new();
    if bytes.len() > limits.max_file_bytes {
        diagnostics.push(
            Diagnostic::error(
                "grist.detect",
                "limit.max_file_bytes",
                format!(
                    "file has {} bytes, exceeding max_file_bytes {}",
                    bytes.len(),
                    limits.max_file_bytes
                ),
            )
            .partial(),
        );
    }
    if is_binary(bytes) {
        reasons.push("content contains NUL bytes or binary-like control bytes".to_string());
        return Detection {
            file_kind: FileKind::Binary,
            content_kind: ContentKind::Binary,
            language: None,
            confidence: 0.95,
            reasons,
            diagnostics,
        };
    }

    let filename = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let text_prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);

    let mut file_kind = classify_file_kind(&filename, &extension, path);
    let mut content_kind = ContentKind::Unknown;
    let mut language = None;
    let mut confidence = 0.6;

    match filename.as_str() {
        "cargo.toml" => {
            content_kind = ContentKind::Toml;
            file_kind = FileKind::Manifest;
            confidence = 0.99;
            reasons.push("special filename Cargo.toml".to_string());
        }
        "cargo.lock" => {
            content_kind = ContentKind::Toml;
            file_kind = FileKind::Lockfile;
            confidence = 0.99;
            reasons.push("special filename Cargo.lock".to_string());
        }
        "readme.md" | "readme.markdown" => {
            content_kind = ContentKind::Markdown;
            file_kind = FileKind::Documentation;
            confidence = 0.99;
            reasons.push("README markdown filename".to_string());
        }
        _ => {}
    }

    if matches!(content_kind, ContentKind::Unknown) {
        match extension.as_str() {
            "rs" => {
                content_kind = ContentKind::Rust;
                language = Some("rust".to_string());
                confidence = 0.98;
                reasons.push(".rs extension".to_string());
            }
            "py" | "pyi" => {
                content_kind = ContentKind::Python;
                language = Some("python".to_string());
                confidence = 0.98;
                reasons.push("Python extension".to_string());
            }
            "ts" | "mts" | "cts" => {
                content_kind = ContentKind::TypeScript;
                language = Some("typescript".to_string());
                confidence = 0.98;
                reasons.push("TypeScript extension".to_string());
            }
            "tsx" => {
                content_kind = ContentKind::Tsx;
                language = Some("typescript".to_string());
                confidence = 0.98;
                reasons.push("TSX extension".to_string());
            }
            "jsx" => {
                content_kind = ContentKind::Jsx;
                language = Some("jsx".to_string());
                confidence = 0.98;
                reasons.push("JSX extension".to_string());
            }
            "md" | "markdown" => {
                content_kind = ContentKind::Markdown;
                confidence = 0.98;
                reasons.push("markdown extension".to_string());
            }
            "html" | "htm" => {
                content_kind = ContentKind::Html;
                language = Some("html".to_string());
                confidence = 0.98;
                reasons.push("HTML extension".to_string());
            }
            "csv" => {
                content_kind = ContentKind::Csv;
                confidence = 0.98;
                reasons.push("CSV extension".to_string());
            }
            "json" => {
                content_kind = ContentKind::Json;
                confidence = 0.98;
                reasons.push(".json extension".to_string());
            }
            "jsonl" | "ndjson" => {
                content_kind = ContentKind::Jsonl;
                confidence = 0.98;
                reasons.push("JSONL extension".to_string());
            }
            "yaml" | "yml" => {
                content_kind = ContentKind::Yaml;
                confidence = 0.98;
                reasons.push("YAML extension".to_string());
            }
            "toml" => {
                content_kind = ContentKind::Toml;
                confidence = 0.98;
                reasons.push("TOML extension".to_string());
            }
            "txt" => {
                content_kind = ContentKind::Text;
                confidence = 0.9;
                reasons.push("text extension".to_string());
            }
            _ => {}
        }
    }

    if matches!(content_kind, ContentKind::Unknown) {
        let trimmed = text_prefix.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            content_kind = ContentKind::Json;
            confidence = 0.7;
            reasons.push("content looks like JSON".to_string());
        } else if looks_like_html(trimmed) {
            content_kind = ContentKind::Html;
            language = Some("html".to_string());
            confidence = 0.7;
            reasons.push("content looks like HTML".to_string());
        } else if looks_like_csv(&text_prefix) {
            content_kind = ContentKind::Csv;
            confidence = 0.62;
            reasons.push("content looks like CSV".to_string());
        } else if trimmed.starts_with("---") {
            content_kind = ContentKind::Yaml;
            confidence = 0.65;
            reasons.push("content starts with YAML/frontmatter marker".to_string());
        } else if text_prefix.starts_with("#!") && text_prefix.contains("rust") {
            content_kind = ContentKind::Rust;
            language = Some("rust".to_string());
            confidence = 0.65;
            reasons.push("shebang mentions rust".to_string());
        } else if text_prefix.starts_with("#!") && text_prefix.contains("python") {
            content_kind = ContentKind::Python;
            language = Some("python".to_string());
            confidence = 0.65;
            reasons.push("shebang mentions python".to_string());
        } else if text_prefix.contains("# ") || text_prefix.contains("```") {
            content_kind = ContentKind::Markdown;
            confidence = 0.55;
            reasons.push("content contains markdown-like heading or fence".to_string());
        } else if std::str::from_utf8(bytes).is_ok() {
            content_kind = ContentKind::Text;
            confidence = 0.5;
            reasons.push("valid UTF-8 text".to_string());
        }
    }

    Detection {
        file_kind,
        content_kind,
        language,
        confidence,
        reasons,
        diagnostics,
    }
}

fn classify_file_kind(filename: &str, extension: &str, path: &Path) -> FileKind {
    let path_string = path.to_string_lossy().to_ascii_lowercase();
    if matches!(filename, "cargo.toml" | "package.json" | "pyproject.toml") {
        return FileKind::Manifest;
    }
    if filename.ends_with(".lock") || matches!(filename, "cargo.lock" | "pnpm-lock.yaml") {
        return FileKind::Lockfile;
    }
    if (extension == "py" || extension == "ts" || extension == "tsx" || extension == "jsx")
        && (path_string.contains("/tests/")
            || filename.starts_with("test_")
            || filename.ends_with(".test.ts")
            || filename.ends_with(".spec.ts")
            || filename.ends_with(".test.tsx")
            || filename.ends_with(".spec.tsx")
            || filename.ends_with(".test.jsx")
            || filename.ends_with(".spec.jsx"))
    {
        return FileKind::Test;
    }
    if matches!(extension, "md" | "markdown" | "html" | "htm") {
        return FileKind::Documentation;
    }
    if path_string.contains("/target/")
        || path_string.contains("/dist/")
        || path_string.contains("/generated/")
        || filename.ends_with(".generated.rs")
        || filename.ends_with(".generated.ts")
        || filename.ends_with(".generated.tsx")
        || filename.ends_with(".generated.jsx")
    {
        return FileKind::Generated;
    }
    if path_string.contains("/tests/")
        || filename.ends_with("_test.rs")
        || filename.ends_with(".test.rs")
    {
        return FileKind::Test;
    }
    if matches!(
        extension,
        "rs" | "py" | "pyi" | "ts" | "tsx" | "mts" | "cts" | "jsx"
    ) {
        return FileKind::Source;
    }
    FileKind::Unknown
}

fn looks_like_html(trimmed: &str) -> bool {
    let prefix = trimmed
        .chars()
        .take(128)
        .collect::<String>()
        .to_ascii_lowercase();
    prefix.starts_with("<!doctype html")
        || prefix.starts_with("<html")
        || prefix.starts_with("<div")
        || prefix.starts_with("<section")
        || prefix.starts_with("<template")
        || prefix.starts_with("<form")
        || prefix.starts_with("<button")
        || prefix.starts_with("<input")
}

fn looks_like_csv(text: &str) -> bool {
    let mut non_empty_lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(6);
    let Some(first_line) = non_empty_lines.next() else {
        return false;
    };
    let delimiter_count = first_line.matches(',').count();
    if delimiter_count == 0 {
        return false;
    }
    let expected_columns = delimiter_count + 1;
    let mut matching_lines = 1usize;
    let mut observed_lines = 1usize;
    for line in non_empty_lines {
        observed_lines += 1;
        if line.matches(',').count() + 1 == expected_columns {
            matching_lines += 1;
        }
    }
    observed_lines >= 2 && matching_lines >= 2
}

fn is_binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return true;
    }
    let control = bytes
        .iter()
        .take(8192)
        .filter(|byte| **byte < 0x09 || (**byte > 0x0d && **byte < 0x20))
        .count();
    control > bytes.len().min(8192) / 20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_source() {
        let detection = detect_path(
            Path::new("src/lib.rs"),
            b"pub fn x() {}",
            &Limits::default(),
        );
        assert_eq!(detection.content_kind, ContentKind::Rust);
        assert_eq!(detection.file_kind, FileKind::Source);
    }

    #[test]
    fn detects_binary() {
        let detection = detect_path(Path::new("blob.bin"), b"abc\0def", &Limits::default());
        assert_eq!(detection.content_kind, ContentKind::Binary);
    }

    #[test]
    fn detects_html_source() {
        let detection = detect_path(
            Path::new("templates/form.htm"),
            br#"<button hx-post="/save">Save</button>"#,
            &Limits::default(),
        );
        assert_eq!(detection.content_kind, ContentKind::Html);
        assert_eq!(detection.language.as_deref(), Some("html"));
    }

    #[test]
    fn detects_csv_source() {
        let detection = detect_path(
            Path::new("data.csv"),
            b"name,score\nalpha,1\n",
            &Limits::default(),
        );
        assert_eq!(detection.content_kind, ContentKind::Csv);
    }
}
