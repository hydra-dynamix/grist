//! Stable repository-ingestion policy and report model.

use crate::core::{ArtifactKind, Limits, OperationStatus};
use crate::detect::{ContentKind, FileKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SymlinkPolicy {
    #[default]
    Skip,
    FollowFilesWithinRoot,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubmodulePolicy {
    #[default]
    Skip,
    Traverse,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RepoIngestOptions {
    pub limits: Limits,
    pub honor_ignore: bool,
    pub include_ignored: bool,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub symlink_policy: SymlinkPolicy,
    pub submodule_policy: SubmodulePolicy,
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
            symlink_policy: SymlinkPolicy::Skip,
            submodule_policy: SubmodulePolicy::Skip,
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
    #[serde(default)]
    pub symlink_policy: SymlinkPolicy,
    #[serde(default)]
    pub submodule_policy: SubmodulePolicy,
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
    #[serde(default)]
    pub entries: Vec<RepositoryEntry>,
    #[serde(default)]
    pub aggregate_hashes: RepoAggregateHashes,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoAggregateHashes {
    pub canonicalization: String,
    pub inventory_sha256: String,
    pub content_sha256: String,
    pub parsed_artifacts_sha256: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_payload_hash: Option<String>,
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

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryEntryKind {
    File,
    Directory,
    Symlink,
    Submodule,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryDisposition {
    Parsed,
    Inventoried,
    Binary,
    Unsupported,
    Ignored,
    Skipped,
    BudgetLimited,
    Failed,
    Traversed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RepositorySkipReason {
    IgnoreRule,
    IncludeGlob,
    ExcludeGlob,
    DefaultExcluded,
    RepositoryMetadata,
    ExternalArtifactOutput,
    SymlinkPolicy,
    SymlinkEscapesRoot,
    SymlinkDirectory,
    PathEscapesRoot,
    SubmodulePolicy,
    RepositoryFileLimit,
    TraversalDepthLimit,
    FileSizeLimit,
    ReadError,
    UnsupportedFormat,
    BinaryFile,
    ParseFailed,
    ParseAmbiguous,
    Encrypted,
    Cancelled,
}

impl RepositorySkipReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IgnoreRule => "ignore_rule",
            Self::IncludeGlob => "include_glob",
            Self::ExcludeGlob => "exclude_glob",
            Self::DefaultExcluded => "default_excluded",
            Self::RepositoryMetadata => "repository_metadata",
            Self::ExternalArtifactOutput => "external_artifact_output",
            Self::SymlinkPolicy => "symlink_policy",
            Self::SymlinkEscapesRoot => "symlink_escapes_root",
            Self::SymlinkDirectory => "symlink_directory",
            Self::PathEscapesRoot => "path_escapes_root",
            Self::SubmodulePolicy => "submodule_policy",
            Self::RepositoryFileLimit => "repository_file_limit",
            Self::TraversalDepthLimit => "traversal_depth_limit",
            Self::FileSizeLimit => "file_size_limit",
            Self::ReadError => "read_error",
            Self::UnsupportedFormat => "unsupported_format",
            Self::BinaryFile => "binary_file",
            Self::ParseFailed => "parse_failed",
            Self::ParseAmbiguous => "parse_ambiguous",
            Self::Encrypted => "encrypted",
            Self::Cancelled => "cancelled",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepositoryEntry {
    pub path: String,
    pub entry_kind: RepositoryEntryKind,
    pub disposition: RepositoryDisposition,
    #[serde(default)]
    pub skip_reasons: Vec<RepositorySkipReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_kind: Option<FileKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_kind: Option<ContentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser_status: Option<OperationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
}

impl RepositoryEntry {
    pub(crate) fn skipped(
        path: String,
        entry_kind: RepositoryEntryKind,
        disposition: RepositoryDisposition,
        skip_reasons: Vec<RepositorySkipReason>,
        size_bytes: Option<u64>,
    ) -> Self {
        Self {
            path,
            entry_kind,
            disposition,
            skip_reasons,
            size_bytes,
            raw_sha256: None,
            file_kind: None,
            content_kind: None,
            language: None,
            parser_status: None,
            symlink_target: None,
        }
    }
}
