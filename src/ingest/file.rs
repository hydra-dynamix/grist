//! Compatibility file-ingestion reports over the built-in format parsers.

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, Envelope, GristError, Hashes, Limits, ParserInfo,
    SchemaVersion, SourceInfo,
};
use crate::detect::{ContentKind, Detection, detect_path};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileIngestReport {
    pub schema_version: String,
    pub detection: Detection,
    pub artifact: Option<Value>,
    pub skipped: bool,
    pub skip_reason: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct FileIngestOptions {
    pub limits: Limits,
    pub filename_hint: Option<PathBuf>,
}

pub type FileIngestEnvelope = Envelope<FileIngestReport>;

pub fn ingest_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &FileIngestOptions,
) -> FileIngestEnvelope {
    let path = options
        .filename_hint
        .as_deref()
        .unwrap_or_else(|| Path::new(&source.display_name));
    let detection = detect_path(path, bytes, &options.limits);
    let hashes = Hashes::for_bytes(bytes, std::str::from_utf8(bytes).ok());
    let identity = detection.apply_to_identity(ContentIdentity::from(hashes.clone()));
    let mut diagnostics = detection.diagnostics.clone();
    let mut skipped = false;
    let mut skip_reason = None;
    let artifact = match detection.content_kind {
        ContentKind::Binary => {
            skipped = true;
            skip_reason = Some("binary files are inventory-only".to_string());
            None
        }
        _ if bytes.len() > options.limits.max_file_bytes => {
            skipped = true;
            skip_reason = Some(format!(
                "file exceeds max_file_bytes {}",
                options.limits.max_file_bytes
            ));
            None
        }
        #[cfg(feature = "html")]
        ContentKind::Html => serde_json::to_value(crate::html::parse_html_bytes(
            bytes,
            source.clone(),
            &crate::html::HtmlOptions::default(),
        ))
        .ok(),
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => parse_detected_text(text, source.clone(), &detection),
            Err(err) => {
                diagnostics.push(Diagnostic::error(
                    "grist.ingest",
                    "ingest.utf8",
                    format!("input is not valid UTF-8: {err}"),
                ));
                skipped = true;
                skip_reason = Some("invalid UTF-8".into());
                None
            }
        },
    };

    let report = FileIngestReport {
        schema_version: SchemaVersion::ENVELOPE_V2.to_string(),
        detection,
        artifact,
        skipped,
        skip_reason,
    };
    let options_digest = file_ingest_options_digest(options);
    let envelope = if report.skipped {
        Envelope::partial(
            crate::core::OperationKind::Ingest,
            ArtifactKind::FileIngest,
            source,
            ParserInfo::new("grist.ingest.file"),
            options_digest,
            SchemaVersion::ENVELOPE_V2,
            Some(report),
        )
    } else {
        Envelope::complete(
            crate::core::OperationKind::Ingest,
            ArtifactKind::FileIngest,
            source,
            ParserInfo::new("grist.ingest.file"),
            options_digest,
            SchemaVersion::ENVELOPE_V2,
            report,
        )
    };
    envelope
        .with_hashes(hashes)
        .with_identity(identity)
        .with_diagnostics(diagnostics)
        .with_canonical_payload_identity()
        .expect("file-ingest reports must serialize canonically")
}

pub fn ingest_path(
    path: &Path,
    options: &FileIngestOptions,
) -> Result<FileIngestEnvelope, GristError> {
    let bytes = fs::read(path)?;
    let mut options = options.clone();
    options.filename_hint = Some(path.to_path_buf());
    Ok(ingest_bytes(&bytes, SourceInfo::from_path(path), &options))
}

fn file_ingest_options_digest(options: &FileIngestOptions) -> String {
    let value = serde_json::json!({
        "limits": &options.limits,
        "filename_hint": options
            .filename_hint
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
    });
    crate::core::options_digest(&value).expect("file ingest options must serialize")
}

fn parse_detected_text(text: &str, source: SourceInfo, detection: &Detection) -> Option<Value> {
    let _ = (text, &source);
    match detection.content_kind {
        #[cfg(feature = "markdown")]
        ContentKind::Markdown => {
            serde_json::to_value(crate::markdown::parse_markdown(text, source)).ok()
        }
        #[cfg(feature = "csv")]
        ContentKind::Csv => serde_json::to_value(crate::csv::parse_csv(
            text,
            source,
            &crate::csv::CsvOptions::default(),
        ))
        .ok(),
        #[cfg(feature = "rust")]
        ContentKind::Rust => serde_json::to_value(crate::rust::parse_rust(
            text,
            source,
            &crate::rust::RustIngestOptions::default(),
        ))
        .ok(),
        #[cfg(feature = "python")]
        ContentKind::Python => serde_json::to_value(crate::python::parse_python(
            text,
            source,
            &crate::python::PythonIngestOptions::default(),
        ))
        .ok(),
        #[cfg(feature = "typescript")]
        ContentKind::TypeScript | ContentKind::Tsx | ContentKind::Jsx => {
            serde_json::to_value(crate::typescript::parse_typescript(
                text,
                source,
                &crate::typescript::TypeScriptIngestOptions {
                    dialect: match detection.content_kind {
                        ContentKind::Tsx => crate::typescript::TypeScriptDialect::Tsx,
                        ContentKind::Jsx => crate::typescript::TypeScriptDialect::Jsx,
                        _ => crate::typescript::TypeScriptDialect::TypeScript,
                    },
                    ..Default::default()
                },
            ))
            .ok()
        }
        #[cfg(feature = "latex")]
        ContentKind::Latex => serde_json::to_value(crate::latex::parse_latex(
            text,
            source,
            &crate::latex::LatexOptions::default(),
        ))
        .ok(),
        #[cfg(feature = "bibliography")]
        ContentKind::Bibliography => serde_json::to_value(crate::bibliography::parse_bibliography(
            text,
            source,
            &crate::bibliography::BibliographyOptions::default(),
        ))
        .ok(),
        #[cfg(feature = "serialization")]
        ContentKind::Json | ContentKind::Jsonl | ContentKind::Yaml | ContentKind::Toml => {
            crate::serialization::format_from_content_kind(&detection.content_kind).and_then(
                |format| {
                    serde_json::to_value(crate::serialization::parse_serialization(
                        text, format, source,
                    ))
                    .ok()
                },
            )
        }
        ContentKind::Text => serde_json::to_value(crate::text::parse_text(text, source)).ok(),
        _ => None,
    }
}

#[cfg(all(test, feature = "markdown"))]
mod tests {
    use super::*;

    #[test]
    fn ingests_markdown_bytes() {
        let report = ingest_bytes(
            b"# Hello",
            SourceInfo::stdin("README.md"),
            &FileIngestOptions {
                filename_hint: Some(PathBuf::from("README.md")),
                ..Default::default()
            },
        );
        assert!(!report.payload.as_ref().expect("operation payload").skipped);
        assert!(
            report
                .payload
                .as_ref()
                .expect("operation payload")
                .artifact
                .is_some()
        );
    }
}
