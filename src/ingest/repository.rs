//! Root-confined deterministic repository traversal and parsing.

mod model;

pub use model::*;

use super::Ingestor;
use crate::core::{
    AggregateMemberIdentity, ArtifactKind, BudgetSelection, CanonicalJsonVersion, ContentIdentity,
    Diagnostic, Envelope, GristError, Input, OperationKind, OperationStatus, ParseRequest,
    ParserInfo, ProviderSet, RequestId, ResourceBudget, SchemaVersion, SourceInfo,
    canonical_json_bytes, canonical_json_sha256, sha256_hex,
};
use crate::detect::{ContentKind, Detection, FileKind, detect_path};
use globset::GlobSet;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub type RepoIngestEnvelope = Envelope<RepoIngestReport>;

pub fn ingest_repo(
    root: &Path,
    options: &RepoIngestOptions,
) -> Result<RepoIngestEnvelope, GristError> {
    let root_metadata = fs::metadata(root).map_err(|error| {
        GristError::Message(format!(
            "repository root `{}` is not accessible: {error}",
            root.display()
        ))
    })?;
    if !root_metadata.is_dir() {
        return Err(GristError::Message(format!(
            "repository root `{}` is not a directory",
            root.display()
        )));
    }
    let canonical_root = fs::canonicalize(root)?;
    let include_set = build_glob_set(&options.include_globs)?;
    let exclude_set = build_glob_set(&options.exclude_globs)?;
    let (visible_paths, mut diagnostics) = visible_paths(&canonical_root, options);
    let external_artifact_dir = prepare_external_artifact_dir(options)?;
    let root_source = SourceInfo::from_path(root);
    let mut state = RepositoryState {
        root: canonical_root.clone(),
        root_source: root_source.clone(),
        options,
        include_set,
        exclude_set,
        visible_paths,
        external_artifact_dir,
        ingestor: Ingestor::builtin().map_err(|error| GristError::Message(error.to_string()))?,
        diagnostics: Vec::new(),
        files: Vec::new(),
        artifacts: Vec::new(),
        ignored: Vec::new(),
        unsupported: Vec::new(),
        skipped: Vec::new(),
        entries: Vec::new(),
        detected_languages: BTreeSet::new(),
        manifest_paths: Vec::new(),
        lockfile_paths: Vec::new(),
        test_hints: Vec::new(),
        members: Vec::new(),
        eligible_file_count: 0,
        partial: false,
    };
    state.visit_directory(&canonical_root, 0)?;
    diagnostics.append(&mut state.diagnostics);
    state.sort_output();

    let identity = ContentIdentity::for_compound(state.members)?;
    let content_sha256 = identity
        .aggregate
        .as_ref()
        .map(|aggregate| aggregate.sha256.clone())
        .unwrap_or_else(|| sha256_hex(b""));
    let inventory_sha256 = canonical_json_sha256(&state.entries)?;
    let artifact_manifest = state
        .artifacts
        .iter()
        .map(ArtifactDigestMember::from)
        .collect::<Vec<_>>();
    let parsed_artifacts_sha256 = canonical_json_sha256(&artifact_manifest)?;
    let report = RepoIngestReport {
        schema_version: SchemaVersion::REPO_INGEST_V1.to_string(),
        root: path_label(root),
        options: RepoIngestOptionsSummary {
            honor_ignore: options.honor_ignore,
            include_ignored: options.include_ignored,
            inline_artifacts: options.inline_artifacts,
            max_file_bytes: options.limits.max_file_bytes,
            max_repo_files: options.limits.max_repo_files,
            symlink_policy: options.symlink_policy,
            submodule_policy: options.submodule_policy,
        },
        files: state.files,
        artifacts: state.artifacts,
        ignored: state.ignored,
        unsupported: state.unsupported,
        skipped: state.skipped,
        detected_languages: state.detected_languages.into_iter().collect(),
        manifest_paths: state.manifest_paths,
        lockfile_paths: state.lockfile_paths,
        test_hints: state.test_hints,
        entries: state.entries,
        aggregate_hashes: RepoAggregateHashes {
            canonicalization: CanonicalJsonVersion::CURRENT.as_str().to_string(),
            inventory_sha256,
            content_sha256,
            parsed_artifacts_sha256,
        },
    };
    let options_digest = repo_ingest_options_digest(options)?;
    let envelope = if state.partial {
        Envelope::partial(
            OperationKind::Ingest,
            ArtifactKind::RepoIngest,
            root_source,
            ParserInfo::new("grist.ingest.repo"),
            options_digest,
            SchemaVersion::REPO_INGEST_V1,
            Some(report),
        )
    } else {
        Envelope::complete(
            OperationKind::Ingest,
            ArtifactKind::RepoIngest,
            root_source,
            ParserInfo::new("grist.ingest.repo"),
            options_digest,
            SchemaVersion::REPO_INGEST_V1,
            report,
        )
    };
    Ok(envelope
        .with_identity(identity)
        .with_diagnostics(diagnostics)
        .with_canonical_payload_identity()?)
}

struct RepositoryState<'a> {
    root: PathBuf,
    root_source: SourceInfo,
    options: &'a RepoIngestOptions,
    include_set: Option<GlobSet>,
    exclude_set: Option<GlobSet>,
    visible_paths: Option<HashSet<String>>,
    external_artifact_dir: Option<PathBuf>,
    ingestor: Ingestor,
    diagnostics: Vec<Diagnostic>,
    files: Vec<FileInventoryEntry>,
    artifacts: Vec<FileArtifact>,
    ignored: Vec<String>,
    unsupported: Vec<String>,
    skipped: Vec<SkippedFile>,
    entries: Vec<RepositoryEntry>,
    detected_languages: BTreeSet<String>,
    manifest_paths: Vec<String>,
    lockfile_paths: Vec<String>,
    test_hints: Vec<TestHint>,
    members: Vec<AggregateMemberIdentity>,
    eligible_file_count: usize,
    partial: bool,
}

#[derive(Serialize)]
struct ArtifactDigestMember<'a> {
    path: &'a str,
    kind: &'a ArtifactKind,
    schema_version: &'a str,
    content_hash: &'a str,
    canonical_payload_hash: &'a Option<String>,
}

impl<'a> From<&'a FileArtifact> for ArtifactDigestMember<'a> {
    fn from(artifact: &'a FileArtifact) -> Self {
        Self {
            path: &artifact.path,
            kind: &artifact.kind,
            schema_version: &artifact.schema_version,
            content_hash: &artifact.content_hash,
            canonical_payload_hash: &artifact.canonical_payload_hash,
        }
    }
}

impl RepositoryState<'_> {
    fn visit_directory(&mut self, directory: &Path, depth: usize) -> Result<(), GristError> {
        if depth > self.options.limits.max_parse_depth {
            self.record_budget_skip(
                self.relative_label(directory),
                RepositoryEntryKind::Directory,
                RepositorySkipReason::TraversalDepthLimit,
                None,
                format!(
                    "repository depth {depth} exceeds max_parse_depth {}",
                    self.options.limits.max_parse_depth
                ),
            );
            return Ok(());
        }
        let read_dir = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) => {
                let path = self.relative_label(directory);
                self.record_failure(
                    path,
                    RepositoryEntryKind::Directory,
                    None,
                    format!("could not enumerate directory: {error}"),
                );
                return Ok(());
            }
        };
        let mut children = Vec::new();
        for result in read_dir {
            match result {
                Ok(entry) => children.push(entry),
                Err(error) => {
                    self.partial = true;
                    self.diagnostics.push(Diagnostic::warning(
                        "grist.ingest.repo",
                        "repo.directory_entry.read",
                        format!("could not read a directory entry: {error}"),
                    ));
                }
            }
        }
        children.sort_by(|left, right| {
            os_component_label(&left.file_name())
                .as_bytes()
                .cmp(os_component_label(&right.file_name()).as_bytes())
        });

        for child in children {
            let path = child.path();
            let relative = self.relative_label(&path);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.record_failure(
                        relative,
                        RepositoryEntryKind::File,
                        None,
                        format!("could not inspect entry: {error}"),
                    );
                    continue;
                }
            };
            let file_type = metadata.file_type();
            if child.file_name() == OsStr::new(".git") {
                self.record_policy_skip(
                    relative,
                    if file_type.is_dir() {
                        RepositoryEntryKind::Directory
                    } else {
                        RepositoryEntryKind::File
                    },
                    vec![RepositorySkipReason::RepositoryMetadata],
                    (!file_type.is_dir()).then_some(metadata.len()),
                );
                continue;
            }
            if self.is_external_artifact_path(&path) {
                self.record_policy_skip(
                    relative,
                    if file_type.is_dir() {
                        RepositoryEntryKind::Directory
                    } else {
                        RepositoryEntryKind::File
                    },
                    vec![RepositorySkipReason::ExternalArtifactOutput],
                    (!file_type.is_dir()).then_some(metadata.len()),
                );
                continue;
            }
            if file_type.is_symlink() {
                self.visit_symlink(&path, relative, metadata.len());
                continue;
            }
            if file_type.is_dir() {
                let canonical = match fs::canonicalize(&path) {
                    Ok(canonical) => canonical,
                    Err(error) => {
                        self.record_failure(
                            relative,
                            RepositoryEntryKind::Directory,
                            None,
                            format!("could not resolve directory: {error}"),
                        );
                        continue;
                    }
                };
                if !canonical.starts_with(&self.root) {
                    self.record_path_escape(relative, RepositoryEntryKind::Directory, None);
                    continue;
                }
                if is_default_excluded(&relative) && !self.options.include_ignored {
                    self.record_policy_skip(
                        relative,
                        RepositoryEntryKind::Directory,
                        vec![RepositorySkipReason::DefaultExcluded],
                        None,
                    );
                    continue;
                }
                if is_nested_repository(&path) {
                    match self.options.submodule_policy {
                        SubmodulePolicy::Skip => {
                            self.record_policy_skip(
                                relative,
                                RepositoryEntryKind::Submodule,
                                vec![RepositorySkipReason::SubmodulePolicy],
                                None,
                            );
                            continue;
                        }
                        SubmodulePolicy::Traverse => self.entries.push(RepositoryEntry {
                            path: relative,
                            entry_kind: RepositoryEntryKind::Submodule,
                            disposition: RepositoryDisposition::Traversed,
                            skip_reasons: Vec::new(),
                            size_bytes: None,
                            raw_sha256: None,
                            file_kind: None,
                            content_kind: None,
                            language: None,
                            parser_status: None,
                            symlink_target: None,
                        }),
                    }
                }
                self.visit_directory(&path, depth.saturating_add(1))?;
                continue;
            }
            if file_type.is_file() {
                self.visit_file(&path, &path, relative, RepositoryEntryKind::File, None);
                continue;
            }
            self.record_failure(
                relative,
                RepositoryEntryKind::File,
                Some(metadata.len()),
                "entry is not a regular file, directory, or symbolic link".to_string(),
            );
        }
        Ok(())
    }

    fn visit_symlink(&mut self, path: &Path, relative: String, size_bytes: u64) {
        let target_label = fs::read_link(path).ok().map(|target| path_label(&target));
        let reasons = self.filter_reasons(&relative);
        if !reasons.is_empty() {
            self.record_skipped_entry(
                relative,
                RepositoryEntryKind::Symlink,
                reasons,
                Some(size_bytes),
                target_label,
            );
            return;
        }
        if self.options.symlink_policy == SymlinkPolicy::Skip {
            self.record_skipped_entry(
                relative,
                RepositoryEntryKind::Symlink,
                vec![RepositorySkipReason::SymlinkPolicy],
                Some(size_bytes),
                target_label,
            );
            return;
        }
        let target = match fs::canonicalize(path) {
            Ok(target) => target,
            Err(error) => {
                self.record_failure(
                    relative,
                    RepositoryEntryKind::Symlink,
                    Some(size_bytes),
                    format!("could not resolve symbolic link: {error}"),
                );
                return;
            }
        };
        if !target.starts_with(&self.root) {
            let mut entry = RepositoryEntry::skipped(
                relative.clone(),
                RepositoryEntryKind::Symlink,
                RepositoryDisposition::Skipped,
                vec![RepositorySkipReason::SymlinkEscapesRoot],
                Some(size_bytes),
            );
            entry.symlink_target = target_label;
            self.entries.push(entry);
            self.skipped.push(SkippedFile {
                path: relative.clone(),
                reason: RepositorySkipReason::SymlinkEscapesRoot.as_str().into(),
            });
            self.partial = true;
            self.diagnostics.push(Diagnostic::security_rejection(
                "grist.ingest.repo",
                format!("symbolic link `{relative}` resolves outside the explicit root"),
            ));
            return;
        }
        let metadata = match fs::metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.record_failure(
                    relative,
                    RepositoryEntryKind::Symlink,
                    Some(size_bytes),
                    format!("could not inspect symbolic-link target: {error}"),
                );
                return;
            }
        };
        if !metadata.is_file() {
            self.record_skipped_entry(
                relative,
                RepositoryEntryKind::Symlink,
                vec![RepositorySkipReason::SymlinkDirectory],
                Some(size_bytes),
                target_label,
            );
            return;
        }
        self.visit_file(
            path,
            &target,
            relative,
            RepositoryEntryKind::Symlink,
            target_label,
        );
    }

    fn visit_file(
        &mut self,
        logical_path: &Path,
        read_path: &Path,
        relative: String,
        entry_kind: RepositoryEntryKind,
        symlink_target: Option<String>,
    ) {
        let canonical_read_path = match fs::canonicalize(read_path) {
            Ok(path) => path,
            Err(error) => {
                self.record_failure(
                    relative,
                    entry_kind,
                    None,
                    format!("could not resolve file: {error}"),
                );
                return;
            }
        };
        if !canonical_read_path.starts_with(&self.root) {
            self.record_path_escape(relative, entry_kind, None);
            return;
        }
        let metadata = match fs::metadata(read_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.record_failure(
                    relative,
                    entry_kind,
                    None,
                    format!("could not inspect file: {error}"),
                );
                return;
            }
        };
        let reasons = self.filter_reasons(&relative);
        if !reasons.is_empty() {
            self.record_skipped_entry(
                relative,
                entry_kind,
                reasons,
                Some(metadata.len()),
                symlink_target,
            );
            return;
        }
        self.eligible_file_count = self.eligible_file_count.saturating_add(1);
        if self.eligible_file_count > self.options.limits.max_repo_files {
            self.record_budget_skip(
                relative,
                entry_kind,
                RepositorySkipReason::RepositoryFileLimit,
                Some(metadata.len()),
                format!(
                    "repository file limit {} was exceeded",
                    self.options.limits.max_repo_files
                ),
            );
            return;
        }
        if metadata.len() > self.options.limits.max_file_bytes as u64 {
            self.record_budget_skip(
                relative,
                entry_kind,
                RepositorySkipReason::FileSizeLimit,
                Some(metadata.len()),
                format!(
                    "file has {} bytes, exceeding max_file_bytes {}",
                    metadata.len(),
                    self.options.limits.max_file_bytes
                ),
            );
            return;
        }
        let bytes = match read_bounded(read_path, self.options.limits.max_file_bytes) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.record_failure(
                    relative,
                    entry_kind,
                    Some(metadata.len()),
                    format!("could not read file: {error}"),
                );
                return;
            }
        };
        if bytes.len() > self.options.limits.max_file_bytes {
            self.record_budget_skip(
                relative,
                entry_kind,
                RepositorySkipReason::FileSizeLimit,
                Some(bytes.len() as u64),
                format!(
                    "file grew beyond max_file_bytes {} while it was read",
                    self.options.limits.max_file_bytes
                ),
            );
            return;
        }
        let detection = detect_path(logical_path, &bytes, &self.options.limits);
        let raw_identity = detection.apply_to_identity(ContentIdentity::for_raw_bytes(&bytes));
        let raw_sha256 = raw_identity
            .raw
            .as_ref()
            .map(|identity| identity.sha256.clone())
            .unwrap_or_else(|| sha256_hex(&bytes));
        self.record_classification(&relative, &detection);
        self.files.push(FileInventoryEntry {
            path: relative.clone(),
            kind: detection.file_kind.clone(),
            content_kind: detection.content_kind.clone(),
            language: detection.language.clone(),
            size_bytes: bytes.len(),
            content_hash: raw_sha256.clone(),
        });

        if detection.content_kind == ContentKind::Binary {
            self.members
                .push(AggregateMemberIdentity::new(&relative, None, &raw_identity));
            self.entries.push(RepositoryEntry {
                path: relative.clone(),
                entry_kind,
                disposition: RepositoryDisposition::Binary,
                skip_reasons: vec![RepositorySkipReason::BinaryFile],
                size_bytes: Some(bytes.len() as u64),
                raw_sha256: Some(raw_sha256),
                file_kind: Some(detection.file_kind),
                content_kind: Some(detection.content_kind),
                language: detection.language,
                parser_status: Some(OperationStatus::Unsupported),
                symlink_target,
            });
            self.skipped.push(SkippedFile {
                path: relative,
                reason: RepositorySkipReason::BinaryFile.as_str().into(),
            });
            return;
        }

        let source = SourceInfo::new(&relative)
            .with_path(Path::new(&relative))
            .with_repository_relative_path(&relative)
            .with_parent(self.root_source.clone());
        let mut budget = ResourceBudget::trusted_unbounded();
        budget.max_input_bytes = Some(self.options.limits.max_file_bytes as u64);
        let request_id = RequestId::new(format!("repo-file-{}", self.eligible_file_count))
            .expect("bounded numeric request IDs are valid");
        let request = ParseRequest::new(
            request_id,
            Input::bytes(bytes),
            source,
            BudgetSelection::custom(budget),
            ProviderSet::none(),
        );
        let envelope = match self.ingestor.ingest(request) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.members
                    .push(AggregateMemberIdentity::new(&relative, None, &raw_identity));
                self.record_failure_with_reason(
                    relative,
                    entry_kind,
                    Some(metadata.len()),
                    RepositorySkipReason::ParseFailed,
                    format!("parser dispatch failed: {error}"),
                );
                return;
            }
        };
        let identity = envelope
            .identity
            .clone()
            .unwrap_or_else(|| raw_identity.clone());
        self.members
            .push(AggregateMemberIdentity::new(&relative, None, &identity));
        self.diagnostics.extend(
            envelope
                .diagnostics
                .iter()
                .cloned()
                .map(|diagnostic| ensure_diagnostic_source(diagnostic, &relative)),
        );
        let status = envelope.status;
        let mut entry = RepositoryEntry {
            path: relative.clone(),
            entry_kind,
            disposition: RepositoryDisposition::Inventoried,
            skip_reasons: Vec::new(),
            size_bytes: Some(identity.byte_length()),
            raw_sha256: Some(raw_sha256.clone()),
            file_kind: Some(detection.file_kind),
            content_kind: Some(detection.content_kind),
            language: detection.language,
            parser_status: Some(status),
            symlink_target,
        };
        match status {
            OperationStatus::Complete | OperationStatus::Partial if envelope.payload.is_some() => {
                entry.disposition = RepositoryDisposition::Parsed;
                if status == OperationStatus::Partial {
                    self.partial = true;
                }
                if let Ok(artifact_value) = serde_json::to_value(&envelope) {
                    collect_artifact_test_hints(
                        &relative,
                        Some(&artifact_value),
                        &mut self.test_hints,
                    );
                }
                match self.store_artifact(&relative, &raw_sha256, &envelope) {
                    Ok(artifact) => {
                        self.artifacts.push(artifact);
                    }
                    Err(error) => {
                        self.record_failure_with_reason(
                            relative.clone(),
                            entry_kind,
                            Some(identity.byte_length()),
                            RepositorySkipReason::ParseFailed,
                            format!("could not store parsed artifact: {error}"),
                        );
                        return;
                    }
                }
            }
            OperationStatus::Unsupported => {
                entry.disposition = RepositoryDisposition::Unsupported;
                entry
                    .skip_reasons
                    .push(RepositorySkipReason::UnsupportedFormat);
                self.unsupported.push(relative.clone());
                self.skipped.push(SkippedFile {
                    path: relative.clone(),
                    reason: RepositorySkipReason::UnsupportedFormat.as_str().into(),
                });
            }
            OperationStatus::Ambiguous => {
                entry.disposition = RepositoryDisposition::Failed;
                entry
                    .skip_reasons
                    .push(RepositorySkipReason::ParseAmbiguous);
                self.record_terminal_skip(&relative, RepositorySkipReason::ParseAmbiguous);
            }
            OperationStatus::Encrypted => {
                entry.disposition = RepositoryDisposition::Unsupported;
                entry.skip_reasons.push(RepositorySkipReason::Encrypted);
                self.record_terminal_skip(&relative, RepositorySkipReason::Encrypted);
            }
            OperationStatus::Cancelled => {
                entry.disposition = RepositoryDisposition::Failed;
                entry.skip_reasons.push(RepositorySkipReason::Cancelled);
                self.record_terminal_skip(&relative, RepositorySkipReason::Cancelled);
            }
            OperationStatus::Failed | OperationStatus::Complete | OperationStatus::Partial => {
                entry.disposition = RepositoryDisposition::Failed;
                entry.skip_reasons.push(RepositorySkipReason::ParseFailed);
                self.record_terminal_skip(&relative, RepositorySkipReason::ParseFailed);
            }
        }
        self.entries.push(entry);
    }

    fn filter_reasons(&self, relative: &str) -> Vec<RepositorySkipReason> {
        let mut reasons = Vec::new();
        if let Some(visible) = &self.visible_paths {
            if !visible.contains(relative) {
                reasons.push(RepositorySkipReason::IgnoreRule);
            }
        }
        if self
            .include_set
            .as_ref()
            .is_some_and(|set| !set.is_match(relative))
        {
            reasons.push(RepositorySkipReason::IncludeGlob);
        }
        if self
            .exclude_set
            .as_ref()
            .is_some_and(|set| set.is_match(relative))
        {
            reasons.push(RepositorySkipReason::ExcludeGlob);
        }
        reasons.sort();
        reasons.dedup();
        reasons
    }

    fn record_skipped_entry(
        &mut self,
        path: String,
        entry_kind: RepositoryEntryKind,
        reasons: Vec<RepositorySkipReason>,
        size_bytes: Option<u64>,
        symlink_target: Option<String>,
    ) {
        let disposition = if reasons.contains(&RepositorySkipReason::IgnoreRule) {
            self.ignored.push(path.clone());
            RepositoryDisposition::Ignored
        } else {
            RepositoryDisposition::Skipped
        };
        let reason = reasons
            .iter()
            .map(|reason| reason.as_str())
            .collect::<Vec<_>>()
            .join(",");
        self.skipped.push(SkippedFile {
            path: path.clone(),
            reason,
        });
        let mut entry =
            RepositoryEntry::skipped(path, entry_kind, disposition, reasons, size_bytes);
        entry.symlink_target = symlink_target;
        self.entries.push(entry);
    }

    fn record_policy_skip(
        &mut self,
        path: String,
        entry_kind: RepositoryEntryKind,
        reasons: Vec<RepositorySkipReason>,
        size_bytes: Option<u64>,
    ) {
        self.record_skipped_entry(path, entry_kind, reasons, size_bytes, None);
    }

    fn record_budget_skip(
        &mut self,
        path: String,
        entry_kind: RepositoryEntryKind,
        reason: RepositorySkipReason,
        size_bytes: Option<u64>,
        message: String,
    ) {
        self.entries.push(RepositoryEntry::skipped(
            path.clone(),
            entry_kind,
            RepositoryDisposition::BudgetLimited,
            vec![reason],
            size_bytes,
        ));
        self.skipped.push(SkippedFile {
            path: path.clone(),
            reason: reason.as_str().into(),
        });
        self.partial = true;
        self.diagnostics.push(
            Diagnostic::budget_exhausted("grist.ingest.repo", message)
                .with_source(path)
                .partial(),
        );
    }

    fn record_failure(
        &mut self,
        path: String,
        entry_kind: RepositoryEntryKind,
        size_bytes: Option<u64>,
        message: String,
    ) {
        self.record_failure_with_reason(
            path,
            entry_kind,
            size_bytes,
            RepositorySkipReason::ReadError,
            message,
        );
    }

    fn record_failure_with_reason(
        &mut self,
        path: String,
        entry_kind: RepositoryEntryKind,
        size_bytes: Option<u64>,
        reason: RepositorySkipReason,
        message: String,
    ) {
        self.entries.push(RepositoryEntry::skipped(
            path.clone(),
            entry_kind,
            RepositoryDisposition::Failed,
            vec![reason],
            size_bytes,
        ));
        self.skipped.push(SkippedFile {
            path: path.clone(),
            reason: reason.as_str().into(),
        });
        self.partial = true;
        self.diagnostics.push(
            Diagnostic::warning("grist.ingest.repo", "repo.entry.read", message)
                .with_source(path)
                .partial(),
        );
    }

    fn record_path_escape(
        &mut self,
        path: String,
        entry_kind: RepositoryEntryKind,
        size_bytes: Option<u64>,
    ) {
        self.entries.push(RepositoryEntry::skipped(
            path.clone(),
            entry_kind,
            RepositoryDisposition::Skipped,
            vec![RepositorySkipReason::PathEscapesRoot],
            size_bytes,
        ));
        self.skipped.push(SkippedFile {
            path: path.clone(),
            reason: RepositorySkipReason::PathEscapesRoot.as_str().into(),
        });
        self.partial = true;
        self.diagnostics.push(
            Diagnostic::security_rejection(
                "grist.ingest.repo",
                format!("entry `{path}` resolves outside the explicit root"),
            )
            .with_source(path)
            .partial(),
        );
    }

    fn record_terminal_skip(&mut self, path: &str, reason: RepositorySkipReason) {
        self.skipped.push(SkippedFile {
            path: path.to_string(),
            reason: reason.as_str().into(),
        });
        self.partial = true;
    }

    fn record_classification(&mut self, path: &str, detection: &Detection) {
        if let Some(language) = &detection.language {
            self.detected_languages.insert(language.clone());
        }
        match detection.file_kind {
            FileKind::Manifest => self.manifest_paths.push(path.to_string()),
            FileKind::Lockfile => self.lockfile_paths.push(path.to_string()),
            FileKind::Test => self.test_hints.push(TestHint {
                path: path.to_string(),
                kind: "test_file".into(),
                name: None,
            }),
            _ => {}
        }
    }

    fn store_artifact(
        &self,
        path: &str,
        content_hash: &str,
        envelope: &Envelope<Value>,
    ) -> Result<FileArtifact, GristError> {
        let artifact_value = serde_json::to_value(envelope)?;
        let canonical_payload_hash = envelope
            .identity
            .as_ref()
            .and_then(|identity| identity.canonical_payload.as_ref())
            .map(|identity| identity.sha256.clone());
        let (artifact, artifact_ref) = if self.options.inline_artifacts {
            (Some(artifact_value), None)
        } else if let Some(directory) = &self.external_artifact_dir {
            let canonical = canonical_json_bytes(&artifact_value)?;
            let artifact_hash = sha256_hex(&canonical);
            let file_name = format!("{}.json", artifact_hash.trim_start_matches("sha256:"));
            let artifact_path = directory.join(file_name);
            fs::write(&artifact_path, canonical)?;
            (None, Some(path_label(&artifact_path)))
        } else {
            (None, None)
        };
        Ok(FileArtifact {
            path: path.to_string(),
            kind: envelope.kind.clone(),
            schema_version: envelope.payload_schema_version.to_string(),
            content_hash: content_hash.to_string(),
            artifact,
            artifact_ref,
            canonical_payload_hash,
        })
    }

    fn is_external_artifact_path(&self, path: &Path) -> bool {
        self.external_artifact_dir
            .as_ref()
            .is_some_and(|directory| path.starts_with(directory))
    }

    fn relative_label(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .ok()
            .filter(|relative| !relative.as_os_str().is_empty())
            .map(relative_path_label)
            .unwrap_or_else(|| ".".into())
    }

    fn sort_output(&mut self) {
        self.files.sort_by(|left, right| left.path.cmp(&right.path));
        self.artifacts
            .sort_by(|left, right| left.path.cmp(&right.path));
        self.ignored.sort();
        self.ignored.dedup();
        self.unsupported.sort();
        self.unsupported.dedup();
        self.skipped.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.reason.cmp(&right.reason))
        });
        self.skipped.dedup();
        self.entries.sort_by(|left, right| {
            left.path.cmp(&right.path).then_with(|| {
                entry_kind_rank(left.entry_kind).cmp(&entry_kind_rank(right.entry_kind))
            })
        });
        self.manifest_paths.sort();
        self.manifest_paths.dedup();
        self.lockfile_paths.sort();
        self.lockfile_paths.dedup();
        self.test_hints.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.name.cmp(&right.name))
        });
        self.test_hints.dedup();
    }
}

fn visible_paths(
    root: &Path,
    options: &RepoIngestOptions,
) -> (Option<HashSet<String>>, Vec<Diagnostic>) {
    if !options.honor_ignore || options.include_ignored {
        return (None, Vec::new());
    }
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(false)
        .parents(false)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        // Repository-local ignore rules apply to any ingested root, including
        // directories that are not inside a Git worktree.
        .require_git(false)
        .follow_links(false);
    let mut visible = HashSet::new();
    let mut diagnostics = Vec::new();
    for result in builder.build() {
        match result {
            Ok(entry) => {
                if let Ok(relative) = entry.path().strip_prefix(root) {
                    if !relative.as_os_str().is_empty() {
                        visible.insert(relative_path_label(relative));
                    }
                }
            }
            Err(error) => {
                let diagnostic = Diagnostic::warning(
                    "grist.ingest.repo",
                    "repo.ignore.walk",
                    format!("ignore-aware traversal could not inspect an entry: {error}"),
                );
                diagnostics.push(diagnostic.partial());
            }
        }
    }
    (Some(visible), diagnostics)
}

fn prepare_external_artifact_dir(
    options: &RepoIngestOptions,
) -> Result<Option<PathBuf>, GristError> {
    let Some(directory) = &options.external_artifact_dir else {
        return Ok(None);
    };
    fs::create_dir_all(directory)?;
    Ok(Some(fs::canonicalize(directory)?))
}

fn read_bounded(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut reader = file.take((limit as u64).saturating_add(1));
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn build_glob_set(globs: &[String]) -> Result<Option<GlobSet>, GristError> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in globs {
        builder.add(
            globset::Glob::new(pattern).map_err(|error| {
                GristError::Message(format!("invalid glob `{pattern}`: {error}"))
            })?,
        );
    }
    builder
        .build()
        .map(Some)
        .map_err(|error| GristError::Message(format!("invalid glob set: {error}")))
}

fn repo_ingest_options_digest(options: &RepoIngestOptions) -> Result<String, GristError> {
    Ok(crate::core::options_digest(&serde_json::json!({
        "limits": &options.limits,
        "honor_ignore": options.honor_ignore,
        "include_ignored": options.include_ignored,
        "include_globs": &options.include_globs,
        "exclude_globs": &options.exclude_globs,
        "symlink_policy": options.symlink_policy,
        "submodule_policy": options.submodule_policy,
        "inline_artifacts": options.inline_artifacts,
        "external_artifact_dir": options.external_artifact_dir.as_ref().map(|path| path_label(path)),
    }))?)
}

fn ensure_diagnostic_source(mut diagnostic: Diagnostic, source: &str) -> Diagnostic {
    if diagnostic.source.is_none() {
        diagnostic.source = Some(source.to_string());
    }
    diagnostic
}

fn is_nested_repository(path: &Path) -> bool {
    path.join(".git").exists()
}

fn is_default_excluded(path: &str) -> bool {
    matches!(
        path.split('/').next().unwrap_or(path),
        ".git" | "target" | ".bathysphere"
    )
}

fn entry_kind_rank(kind: RepositoryEntryKind) -> u8 {
    match kind {
        RepositoryEntryKind::Directory => 0,
        RepositoryEntryKind::Submodule => 1,
        RepositoryEntryKind::Symlink => 2,
        RepositoryEntryKind::File => 3,
    }
}

fn path_label(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn relative_path_label(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(os_component_label(value)),
            std::path::Component::CurDir => None,
            std::path::Component::ParentDir => Some("..".into()),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(unix)]
fn os_component_label(value: &OsStr) -> String {
    use std::os::unix::ffi::OsStrExt;
    let bytes = value.as_bytes();
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.replace('%', "%25").replace('\\', "%5C");
    }
    let mut label = String::new();
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'-' | b'_') {
            label.push(*byte as char);
        } else {
            label.push_str(&format!("%{byte:02X}"));
        }
    }
    label
}

#[cfg(windows)]
fn os_component_label(value: &OsStr) -> String {
    value
        .to_string_lossy()
        .replace('%', "%25")
        .replace('\\', "%5C")
}

#[cfg(not(any(unix, windows)))]
fn os_component_label(value: &OsStr) -> String {
    value
        .to_string_lossy()
        .replace('%', "%25")
        .replace('\\', "%5C")
}

fn collect_artifact_test_hints(
    path: &str,
    artifact: Option<&Value>,
    test_hints: &mut Vec<TestHint>,
) {
    let Some(artifact) = artifact else {
        return;
    };
    let kind = artifact.get("kind").and_then(Value::as_str);
    if kind != Some("rust_code")
        && kind != Some("python_code")
        && kind != Some("javascript_code")
        && kind != Some("typescript_code")
    {
        return;
    }
    for symbol in artifact
        .pointer("/payload/symbols")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let attributes = symbol
            .get("attributes")
            .or_else(|| symbol.get("decorators"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let name = symbol.get("name").and_then(Value::as_str).unwrap_or("");
        if attributes
            .iter()
            .any(|attribute| attribute.contains("test"))
            || name.starts_with("test_")
        {
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
    if matches!(kind, Some("javascript_code" | "typescript_code")) {
        for call in artifact
            .pointer("/payload/calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
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
                        .map(|name| name.trim_matches(['\"', '\'', '`']).to_string()),
                });
            }
        }
    }
}
