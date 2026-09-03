//! Explicit, collision-safe embedded-artifact materialization.

use super::{
    ArtifactContent, ArtifactSafetyClassification, ContentAddressedArtifactResolver,
    EmbeddedArtifact,
};
use crate::core::{BudgetExceeded, BudgetTracker, sha256_hex};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// No filesystem destination is selected by default.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MaterializationRequest {
    #[default]
    Disabled,
    Directory {
        directory: PathBuf,
        #[serde(default)]
        allow_unsafe: bool,
        /// Includes the unsuffixed name attempt and prevents unbounded collision loops.
        max_name_attempts: u32,
    },
}

impl MaterializationRequest {
    pub fn directory(directory: impl Into<PathBuf>) -> Self {
        Self::Directory {
            directory: directory.into(),
            allow_unsafe: false,
            max_name_attempts: 1_024,
        }
    }

    pub fn allowing_unsafe(mut self) -> Self {
        if let Self::Directory { allow_unsafe, .. } = &mut self {
            *allow_unsafe = true;
        }
        self
    }

    pub fn with_max_name_attempts(mut self, attempts: u32) -> Self {
        if let Self::Directory {
            max_name_attempts, ..
        } = &mut self
        {
            *max_name_attempts = attempts;
        }
        self
    }
}

/// Auditable result. A blocked or disabled request never writes.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ArtifactMaterializationOutcome {
    Disabled,
    BlockedBySafety {
        classification: ArtifactSafetyClassification,
    },
    Materialized {
        path: PathBuf,
        generated_filename: String,
        byte_length: u64,
        sha256: String,
        quarantined: bool,
    },
}

impl EmbeddedArtifact {
    /// Materialize only after an explicit directory request.
    ///
    /// Original filenames are never used for filesystem naming. External bytes
    /// must be supplied by an explicit resolver and are hash-checked before I/O.
    pub fn materialize(
        &self,
        request: &MaterializationRequest,
        resolver: Option<&dyn ContentAddressedArtifactResolver>,
        budget: Option<&BudgetTracker>,
    ) -> Result<ArtifactMaterializationOutcome, ArtifactMaterializationError> {
        self.validate()
            .map_err(|error| ArtifactMaterializationError::InvalidArtifact(error.to_string()))?;
        let MaterializationRequest::Directory {
            directory,
            allow_unsafe,
            max_name_attempts,
        } = request
        else {
            return Ok(ArtifactMaterializationOutcome::Disabled);
        };
        if *max_name_attempts == 0 {
            return Err(ArtifactMaterializationError::InvalidNameAttemptLimit);
        }
        let quarantined = self.safety.requires_explicit_unsafe_opt_in();
        if quarantined && !allow_unsafe {
            return Ok(ArtifactMaterializationOutcome::BlockedBySafety {
                classification: self.safety.classification,
            });
        }
        let bytes = resolve_bytes(self, resolver)?;
        let raw = self
            .identity
            .content
            .raw
            .as_ref()
            .ok_or(ArtifactMaterializationError::ContentUnavailable)?;
        let byte_length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let digest = sha256_hex(&bytes);
        if raw.byte_length != byte_length || raw.sha256 != digest {
            return Err(ArtifactMaterializationError::ContentAddressMismatch);
        }
        if let Some(budget) = budget {
            budget
                .observe_temporary_storage_bytes(byte_length)
                .map_err(ArtifactMaterializationError::Budget)?;
        }
        let root = prepare_directory(directory)?;
        let extension = generated_extension(self.media_type.as_deref(), quarantined);
        let digest_hex = digest
            .strip_prefix("sha256:")
            .ok_or(ArtifactMaterializationError::ContentAddressMismatch)?;
        for attempt in 0..*max_name_attempts {
            let generated_filename = if attempt == 0 {
                format!("artifact-{digest_hex}.{extension}")
            } else {
                format!("artifact-{digest_hex}-{attempt}.{extension}")
            };
            let path = root.join(&generated_filename);
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(ArtifactMaterializationError::Io {
                        operation: "create artifact",
                        path,
                        source,
                    });
                }
            };
            if let Err(source) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(ArtifactMaterializationError::Io {
                    operation: "write artifact",
                    path,
                    source,
                });
            }
            return Ok(ArtifactMaterializationOutcome::Materialized {
                path,
                generated_filename,
                byte_length,
                sha256: digest,
                quarantined,
            });
        }
        Err(ArtifactMaterializationError::NameAttemptsExhausted {
            directory: root,
            attempts: *max_name_attempts,
        })
    }
}

fn resolve_bytes<'a>(
    artifact: &'a EmbeddedArtifact,
    resolver: Option<&dyn ContentAddressedArtifactResolver>,
) -> Result<Cow<'a, [u8]>, ArtifactMaterializationError> {
    match artifact.content.as_ref() {
        Some(ArtifactContent::Inline(inline)) => Ok(Cow::Borrowed(&inline.bytes)),
        Some(ArtifactContent::ContentAddressed { reference, .. }) => {
            let bytes = resolver
                .ok_or(ArtifactMaterializationError::ResolverRequired)?
                .resolve(reference)
                .map_err(|error| ArtifactMaterializationError::Resolver(error.to_string()))?;
            reference
                .validate_bytes(&bytes)
                .map_err(|_| ArtifactMaterializationError::ContentAddressMismatch)?;
            Ok(Cow::Owned(bytes))
        }
        None => Err(ArtifactMaterializationError::ContentUnavailable),
    }
}

fn prepare_directory(directory: &Path) -> Result<PathBuf, ArtifactMaterializationError> {
    if directory.as_os_str().is_empty() {
        return Err(ArtifactMaterializationError::InvalidDirectory);
    }
    if directory.exists() {
        let metadata =
            fs::symlink_metadata(directory).map_err(|source| ArtifactMaterializationError::Io {
                operation: "inspect materialization directory",
                path: directory.to_path_buf(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(ArtifactMaterializationError::DirectoryIsSymlink(
                directory.to_path_buf(),
            ));
        }
        if !metadata.is_dir() {
            return Err(ArtifactMaterializationError::NotDirectory(
                directory.to_path_buf(),
            ));
        }
    } else {
        fs::create_dir_all(directory).map_err(|source| ArtifactMaterializationError::Io {
            operation: "create materialization directory",
            path: directory.to_path_buf(),
            source,
        })?;
    }
    directory
        .canonicalize()
        .map_err(|source| ArtifactMaterializationError::Io {
            operation: "canonicalize materialization directory",
            path: directory.to_path_buf(),
            source,
        })
}

fn generated_extension(media_type: Option<&str>, quarantined: bool) -> &'static str {
    if quarantined {
        return "quarantine";
    }
    match media_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("text/plain") => "txt",
        Some("application/json") => "json",
        Some("application/xml") | Some("text/xml") => "xml",
        Some("application/pdf") => "pdf",
        Some("application/zip") => "zip",
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/gif") => "gif",
        Some("audio/mpeg") => "mp3",
        Some("video/mp4") => "mp4",
        _ => "bin",
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactMaterializationError {
    #[error("embedded artifact is invalid: {0}")]
    InvalidArtifact(String),
    #[error("materialization directory cannot be empty")]
    InvalidDirectory,
    #[error("max_name_attempts must be greater than zero")]
    InvalidNameAttemptLimit,
    #[error("artifact has no extracted content to materialize")]
    ContentUnavailable,
    #[error("content-addressed artifact materialization requires a resolver")]
    ResolverRequired,
    #[error("content-addressed resolver failed: {0}")]
    Resolver(String),
    #[error("resolved or inline bytes do not match the artifact identity")]
    ContentAddressMismatch,
    #[error("materialization directory is a symbolic link: {0}")]
    DirectoryIsSymlink(PathBuf),
    #[error("materialization target is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("resource budget prevented materialization: {0}")]
    Budget(BudgetExceeded),
    #[error("{operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not allocate a collision-free name in {directory} after {attempts} attempts")]
    NameAttemptsExhausted { directory: PathBuf, attempts: u32 },
}
