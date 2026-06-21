use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaVersion(pub String);

impl SchemaVersion {
    pub const ENVELOPE_V1: &'static str = "grist/envelope/v1";
    pub const MARKDOWN_V1: &'static str = "grist/markdown/v1";
    pub const HTML_V1: &'static str = "grist/html/v1";
    pub const CSV_V1: &'static str = "grist/csv/v1";
    pub const RUST_CODE_V1: &'static str = "grist/rust-code/v1";
    pub const PYTHON_CODE_V1: &'static str = "grist/python-code/v1";
    pub const TYPESCRIPT_CODE_V1: &'static str = "grist/typescript-code/v1";
    pub const SERIALIZATION_V1: &'static str = "grist/serialization/v1";
    pub const MODEL_OUTPUT_V1: &'static str = "grist/model-output/v1";
    pub const REPO_INGEST_V1: &'static str = "grist/repo-ingest/v1";
    pub const LDGR_PROJECTION_V1: &'static str = "grist.ldgr_projection.v1";
}

impl From<&str> for SchemaVersion {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Markdown,
    Html,
    Csv,
    RustCode,
    PythonCode,
    #[serde(rename = "typescript_code")]
    TypeScriptCode,
    Serialization,
    ModelOutput,
    RepoIngest,
    FileIngest,
    Text,
    LdgrProjection,
    Unsupported,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRange {
    pub byte_start: usize,
    pub byte_end: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl SourceRange {
    pub fn new(byte_start: usize, byte_end: usize, index: &LineIndex) -> Self {
        let start = index.line_column(byte_start);
        let end = index.line_column(byte_end);
        Self {
            byte_start,
            byte_end,
            start_line: start.line,
            start_column: start.column,
            end_line: end.line,
            end_column: end.column,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone)]
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (idx, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }
        Self { line_starts }
    }

    pub fn line_column(&self, byte_offset: usize) -> LineColumn {
        let line_idx = match self.line_starts.binary_search(&byte_offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        LineColumn {
            line: line_idx + 1,
            column: byte_offset.saturating_sub(self.line_starts[line_idx]) + 1,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceInfo {
    pub path: Option<String>,
    pub display_name: String,
}

impl SourceInfo {
    pub fn from_path(path: &Path) -> Self {
        Self {
            path: Some(path.to_string_lossy().to_string()),
            display_name: path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string()),
        }
    }

    pub fn stdin(display_name: impl Into<String>) -> Self {
        Self {
            path: None,
            display_name: display_name.into(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hashes {
    pub sha256: String,
    pub text_sha256: Option<String>,
    pub size_bytes: usize,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

impl Hashes {
    pub fn for_bytes(bytes: &[u8], text: Option<&str>) -> Self {
        Self {
            sha256: sha256_hex(bytes),
            text_sha256: text.map(|value| sha256_hex(value.as_bytes())),
            size_bytes: bytes.len(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub parser: String,
    pub source: Option<String>,
    pub range: Option<SourceRange>,
    pub partial: bool,
    pub cause: Vec<String>,
    pub details: Option<Value>,
}

impl Diagnostic {
    pub fn error(
        parser: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Severity::Error, parser, code, message)
    }

    pub fn warning(
        parser: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Severity::Warning, parser, code, message)
    }

    pub fn info(
        parser: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Severity::Info, parser, code, message)
    }

    pub fn new(
        severity: Severity,
        parser: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            parser: parser.into(),
            source: None,
            range: None,
            partial: false,
            cause: Vec::new(),
            details: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_range(mut self, range: SourceRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn partial(mut self) -> Self {
        self.partial = true;
        self
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParserInfo {
    pub name: String,
    pub version: String,
}

impl ParserInfo {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope<T> {
    pub schema_version: SchemaVersion,
    pub kind: ArtifactKind,
    pub source: SourceInfo,
    pub hashes: Option<Hashes>,
    pub parser: ParserInfo,
    pub diagnostics: Vec<Diagnostic>,
    pub payload_schema_version: SchemaVersion,
    pub payload: T,
}

impl<T> Envelope<T> {
    pub fn new(
        kind: ArtifactKind,
        source: SourceInfo,
        parser: ParserInfo,
        payload_schema_version: impl Into<SchemaVersion>,
        payload: T,
    ) -> Self {
        Self {
            schema_version: SchemaVersion::from(SchemaVersion::ENVELOPE_V1),
            kind,
            source,
            hashes: None,
            parser,
            diagnostics: Vec::new(),
            payload_schema_version: payload_schema_version.into(),
            payload,
        }
    }

    pub fn with_hashes(mut self, hashes: Hashes) -> Self {
        self.hashes = Some(hashes);
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Vec<Diagnostic>) -> Self {
        self.diagnostics = diagnostics;
        self
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: usize,
    pub max_repo_files: usize,
    pub max_model_output_bytes: usize,
    pub max_parse_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 64 * 1024 * 1024,
            max_repo_files: 250_000,
            max_model_output_bytes: 128 * 1024 * 1024,
            max_parse_depth: 4096,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SourceOptions {
    pub display_name: Option<String>,
    pub filename_hint: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticOptions {
    pub include_info: bool,
    pub include_details: bool,
}

impl Default for DiagnosticOptions {
    fn default() -> Self {
        Self {
            include_info: true,
            include_details: true,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SchemaOptions {
    pub schema_path: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum GristError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("UTF-8 decode error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Message(String),
}

impl From<String> for GristError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for GristError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
