use crate::core::{
    ArtifactKind, Diagnostic, Envelope, GristError, Hashes, Limits, ParserInfo, SchemaVersion,
    SourceInfo,
};
use crate::detect::{ContentKind, Detection, FileKind, detect_path};
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

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoIngestReport {
    pub schema_version: String,
    pub root: String,
    pub options: RepoIngestOptionsSummary,
    pub files: Vec<FileInventoryEntry>,
    pub artifacts: Vec<FileArtifact>,
    pub ignored: Vec<String>,
    pub unsupported: Vec<String>,
    pub skipped: Vec<SkippedFile>,
    pub detected_languages: Vec<String>,
    pub manifest_paths: Vec<String>,
    pub lockfile_paths: Vec<String>,
    pub test_hints: Vec<TestHint>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileArtifact {
    pub path: String,
    pub kind: ArtifactKind,
    pub schema_version: String,
    pub content_hash: String,
    pub artifact: Option<Value>,
    pub artifact_ref: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileInventoryEntry {
    pub path: String,
    pub kind: FileKind,
    pub content_kind: ContentKind,
    pub language: Option<String>,
    pub size_bytes: usize,
    pub content_hash: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestHint {
    pub path: String,
    pub kind: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FileIngestOptions {
    pub limits: Limits,
    pub filename_hint: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RepoIngestOptions {
    pub limits: Limits,
    pub honor_ignore: bool,
    pub include_ignored: bool,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub inline_artifacts: bool,
    pub external_artifact_dir: Option<PathBuf>,
}

impl Default for RepoIngestOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            honor_ignore: true,
            include_ignored: false,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            inline_artifacts: true,
            external_artifact_dir: None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoIngestOptionsSummary {
    pub honor_ignore: bool,
    pub include_ignored: bool,
    pub inline_artifacts: bool,
    pub max_file_bytes: usize,
    pub max_repo_files: usize,
}

pub type FileIngestEnvelope = Envelope<FileIngestReport>;
pub type RepoIngestEnvelope = Envelope<RepoIngestReport>;

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
    let mut diagnostics = detection.diagnostics.clone();
    let mut skipped = false;
    let mut skip_reason = None;
    let artifact = match detection.content_kind {
        ContentKind::Binary => {
            skipped = true;
            skip_reason = Some("binary files are not ingested by Grist v1".to_string());
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

    Envelope::new(
        ArtifactKind::FileIngest,
        source,
        ParserInfo::new("grist.ingest.file"),
        SchemaVersion::ENVELOPE_V1,
        FileIngestReport {
            schema_version: SchemaVersion::ENVELOPE_V1.to_string(),
            detection,
            artifact,
            skipped,
            skip_reason,
        },
    )
    .with_hashes(hashes)
    .with_diagnostics(diagnostics)
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

pub fn ingest_repo(
    root: &Path,
    options: &RepoIngestOptions,
) -> Result<RepoIngestEnvelope, GristError> {
    let mut diagnostics = Vec::new();
    let mut files = Vec::new();
    let mut artifacts = Vec::new();
    let ignored = Vec::new();
    let mut unsupported = Vec::new();
    let mut skipped = Vec::new();
    let mut detected_languages = std::collections::BTreeSet::new();
    let mut manifest_paths = Vec::new();
    let mut lockfile_paths = Vec::new();
    let mut test_hints = Vec::new();

    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(false);
    builder.git_ignore(options.honor_ignore && !options.include_ignored);
    builder.git_exclude(options.honor_ignore && !options.include_ignored);
    builder.ignore(options.honor_ignore && !options.include_ignored);
    let walker = builder.build();
    let include_set = build_glob_set(&options.include_globs)?;
    let exclude_set = build_glob_set(&options.exclude_globs)?;
    if let Some(dir) = &options.external_artifact_dir {
        fs::create_dir_all(dir)?;
    }

    for (idx, entry_result) in walker.enumerate() {
        if idx > options.limits.max_repo_files {
            diagnostics.push(
                Diagnostic::error(
                    "grist.ingest.repo",
                    "limit.max_repo_files",
                    format!(
                        "repo traversal exceeded max_repo_files {}",
                        options.limits.max_repo_files
                    ),
                )
                .partial(),
            );
            break;
        }
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(err) => {
                diagnostics.push(Diagnostic::warning(
                    "grist.ingest.repo",
                    "walk.error",
                    format!("repo walk error: {err}"),
                ));
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if is_default_noisy_path(&rel) && !options.include_ignored {
            skipped.push(SkippedFile {
                path: rel,
                reason: "default noisy/generated path".into(),
            });
            continue;
        }
        if !matches_glob_set(&rel, include_set.as_ref(), true)
            || matches_glob_set(&rel, exclude_set.as_ref(), false)
        {
            skipped.push(SkippedFile {
                path: rel,
                reason: "filtered by include/exclude globs".into(),
            });
            continue;
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                diagnostics.push(Diagnostic::warning(
                    "grist.ingest.repo",
                    "file.read",
                    format!("could not read {}: {err}", rel),
                ));
                continue;
            }
        };
        let detection = detect_path(path, &bytes, &options.limits);
        let hashes = Hashes::for_bytes(&bytes, std::str::from_utf8(&bytes).ok());
        if let Some(language) = &detection.language {
            detected_languages.insert(language.clone());
        }
        if detection.file_kind == FileKind::Manifest {
            manifest_paths.push(rel.clone());
        }
        if detection.file_kind == FileKind::Lockfile {
            lockfile_paths.push(rel.clone());
        }
        if detection.file_kind == FileKind::Test {
            test_hints.push(TestHint {
                path: rel.clone(),
                kind: "test_file".into(),
                name: None,
            });
        }
        files.push(FileInventoryEntry {
            path: rel.clone(),
            kind: detection.file_kind.clone(),
            content_kind: detection.content_kind.clone(),
            language: detection.language.clone(),
            size_bytes: bytes.len(),
            content_hash: hashes.sha256.clone(),
        });
        if detection.content_kind == ContentKind::Binary {
            skipped.push(SkippedFile {
                path: rel.clone(),
                reason: "binary".into(),
            });
            continue;
        }
        let file_options = FileIngestOptions {
            limits: options.limits.clone(),
            filename_hint: Some(path.to_path_buf()),
        };
        let report = ingest_bytes(
            &bytes,
            SourceInfo::from_path(Path::new(&rel)),
            &file_options,
        );
        diagnostics.extend(
            report
                .diagnostics
                .clone()
                .into_iter()
                .map(|diagnostic| ensure_diagnostic_source(diagnostic, &rel)),
        );
        collect_artifact_test_hints(&rel, &report.payload.artifact, &mut test_hints);
        if report.payload.skipped {
            skipped.push(SkippedFile {
                path: rel.clone(),
                reason: report
                    .payload
                    .skip_reason
                    .clone()
                    .unwrap_or_else(|| "skipped".into()),
            });
        } else if let Some(artifact) = report.payload.artifact {
            let kind = artifact
                .get("kind")
                .and_then(Value::as_str)
                .map(kind_from_str)
                .unwrap_or(ArtifactKind::Unsupported);
            let schema_version = artifact
                .get("payload_schema_version")
                .and_then(Value::as_str)
                .unwrap_or(SchemaVersion::ENVELOPE_V1)
                .to_string();
            let (artifact_value, artifact_ref) = if options.inline_artifacts {
                (Some(artifact), None)
            } else if let Some(dir) = &options.external_artifact_dir {
                let file_name = format!("{}.json", hashes.sha256.trim_start_matches("sha256:"));
                let artifact_path = dir.join(file_name);
                fs::write(&artifact_path, serde_json::to_vec(&artifact)?)?;
                (None, Some(artifact_path.to_string_lossy().to_string()))
            } else {
                (None, None)
            };
            artifacts.push(FileArtifact {
                path: rel,
                kind,
                schema_version,
                content_hash: hashes.sha256,
                artifact: artifact_value,
                artifact_ref,
            });
        } else {
            unsupported.push(rel);
        }
    }

    let report = RepoIngestReport {
        schema_version: SchemaVersion::REPO_INGEST_V1.to_string(),
        root: root.to_string_lossy().to_string(),
        options: RepoIngestOptionsSummary {
            honor_ignore: options.honor_ignore,
            include_ignored: options.include_ignored,
            inline_artifacts: options.inline_artifacts,
            max_file_bytes: options.limits.max_file_bytes,
            max_repo_files: options.limits.max_repo_files,
        },
        files,
        artifacts,
        ignored,
        unsupported,
        skipped,
        detected_languages: detected_languages.into_iter().collect(),
        manifest_paths,
        lockfile_paths,
        test_hints,
    };
    Ok(Envelope::new(
        ArtifactKind::RepoIngest,
        SourceInfo::from_path(root),
        ParserInfo::new("grist.ingest.repo"),
        SchemaVersion::REPO_INGEST_V1,
        report,
    )
    .with_diagnostics(diagnostics))
}

fn parse_detected_text(text: &str, source: SourceInfo, detection: &Detection) -> Option<Value> {
    let _ = (text, &source);
    match detection.content_kind {
        #[cfg(feature = "markdown")]
        ContentKind::Markdown => {
            serde_json::to_value(crate::markdown::parse_markdown(text, source)).ok()
        }
        #[cfg(feature = "html")]
        ContentKind::Html => serde_json::to_value(crate::html::parse_html(
            text,
            source,
            &crate::html::HtmlOptions::default(),
        ))
        .ok(),
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

fn kind_from_str(value: &str) -> ArtifactKind {
    match value {
        "markdown" => ArtifactKind::Markdown,
        "html" => ArtifactKind::Html,
        "csv" => ArtifactKind::Csv,
        "rust_code" => ArtifactKind::RustCode,
        "python_code" => ArtifactKind::PythonCode,
        "typescript_code" => ArtifactKind::TypeScriptCode,
        "serialization" => ArtifactKind::Serialization,
        "model_output" => ArtifactKind::ModelOutput,
        "repo_ingest" => ArtifactKind::RepoIngest,
        "file_ingest" => ArtifactKind::FileIngest,
        "text" => ArtifactKind::Text,
        _ => ArtifactKind::Unsupported,
    }
}

fn ensure_diagnostic_source(mut diagnostic: Diagnostic, source: &str) -> Diagnostic {
    if diagnostic.source.is_none() {
        diagnostic.source = Some(source.to_string());
    }
    diagnostic
}

fn collect_artifact_test_hints(
    path: &str,
    artifact: &Option<Value>,
    test_hints: &mut Vec<TestHint>,
) {
    let Some(artifact) = artifact else {
        return;
    };
    let kind = artifact.get("kind").and_then(Value::as_str);
    if kind != Some("rust_code") && kind != Some("python_code") && kind != Some("typescript_code") {
        return;
    }
    let symbols = artifact
        .pointer("/payload/symbols")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for symbol in symbols {
        let attrs = symbol
            .get("attributes")
            .or_else(|| symbol.get("decorators"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let name = symbol.get("name").and_then(Value::as_str).unwrap_or("");
        if attrs.iter().any(|attr| attr.contains("test")) || name.starts_with("test_") {
            test_hints.push(TestHint {
                path: path.to_string(),
                kind: "test_function".into(),
                name: symbol
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }
    if kind == Some("typescript_code") {
        let calls = artifact
            .pointer("/payload/calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for call in calls {
            let target = call.get("target").and_then(Value::as_str).unwrap_or("");
            if matches!(target, "test" | "it" | "describe") || target.ends_with(".test") {
                test_hints.push(TestHint {
                    path: path.to_string(),
                    kind: "test_call".into(),
                    name: call
                        .get("args")
                        .and_then(Value::as_array)
                        .and_then(|args| args.first())
                        .and_then(Value::as_str)
                        .map(|name| name.trim_matches(['"', '\'', '`']).to_string()),
                });
            }
        }
    }
}

fn is_default_noisy_path(path: &str) -> bool {
    path == ".git"
        || path.starts_with(".git/")
        || path == "target"
        || path.starts_with("target/")
        || path == ".bathysphere"
        || path.starts_with(".bathysphere/")
}

fn build_glob_set(globs: &[String]) -> Result<Option<globset::GlobSet>, GristError> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in globs {
        let glob = globset::Glob::new(pattern)
            .map_err(|err| GristError::Message(format!("invalid glob `{pattern}`: {err}")))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|err| GristError::Message(format!("invalid glob set: {err}")))
}

fn matches_glob_set(path: &str, set: Option<&globset::GlobSet>, empty_default: bool) -> bool {
    set.map(|set| set.is_match(path)).unwrap_or(empty_default)
}

#[cfg(test)]
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
        assert!(!report.payload.skipped);
        assert!(report.payload.artifact.is_some());
    }
}
