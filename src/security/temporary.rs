//! Private generated temporary storage with explicit retention.

use crate::core::{BudgetExceeded, BudgetTracker, sha256_hex};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);
const MARKER: &str = ".grist-private-temporary-storage";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemporaryRetentionPolicy {
    #[default]
    DeleteOnDrop,
    RetainUntilExplicitCleanup,
}

pub struct PrivateTemporaryStorage {
    path: PathBuf,
    retention: TemporaryRetentionPolicy,
    live: bool,
    max_bytes: Option<u64>,
    used_bytes: AtomicU64,
}

impl PrivateTemporaryStorage {
    pub fn create(retention: TemporaryRetentionPolicy) -> Result<Self, TemporaryStorageError> {
        Self::create_in_with_limit(std::env::temp_dir(), retention, None)
    }

    pub fn create_limited(
        retention: TemporaryRetentionPolicy,
        max_bytes: u64,
    ) -> Result<Self, TemporaryStorageError> {
        Self::create_in_with_limit(std::env::temp_dir(), retention, Some(max_bytes))
    }

    pub fn create_in(
        base: impl AsRef<Path>,
        retention: TemporaryRetentionPolicy,
    ) -> Result<Self, TemporaryStorageError> {
        Self::create_in_with_limit(base, retention, None)
    }

    fn create_in_with_limit(
        base: impl AsRef<Path>,
        retention: TemporaryRetentionPolicy,
        max_bytes: Option<u64>,
    ) -> Result<Self, TemporaryStorageError> {
        let base = prepare_base(base.as_ref())?;
        for _ in 0..128 {
            let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let material = format!(
                "{}:{timestamp}:{nonce}:{}",
                std::process::id(),
                base.display()
            );
            let digest = sha256_hex(material.as_bytes());
            let path = base.join(format!("grist-private-{}", &digest[7..39]));
            match fs::create_dir(&path) {
                Ok(()) => {
                    let initialized = set_private_permissions(&path).and_then(|()| {
                        OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(path.join(MARKER))?
                            .sync_all()
                            .map_err(TemporaryStorageError::Io)
                    });
                    if let Err(error) = initialized {
                        let _ = fs::remove_dir_all(&path);
                        return Err(error);
                    }
                    return Ok(Self {
                        path,
                        retention,
                        live: true,
                        max_bytes,
                        used_bytes: AtomicU64::new(0),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(TemporaryStorageError::Io(error)),
            }
        }
        Err(TemporaryStorageError::NameAttemptsExhausted)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn retention(&self) -> TemporaryRetentionPolicy {
        self.retention
    }

    pub fn write_generated(
        &self,
        name: &str,
        bytes: &[u8],
        budget: Option<&BudgetTracker>,
    ) -> Result<PathBuf, TemporaryStorageError> {
        if !valid_generated_name(name) {
            return Err(TemporaryStorageError::InvalidGeneratedName);
        }
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if let Some(budget) = budget {
            budget
                .observe_temporary_storage_bytes(length)
                .map_err(TemporaryStorageError::Budget)?;
        }
        self.reserve_bytes(length)?;
        let path = self.path.join(name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) => {
                self.used_bytes.fetch_sub(length, Ordering::Relaxed);
                return Err(TemporaryStorageError::Io(error));
            }
        };
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&path);
            self.used_bytes.fetch_sub(length, Ordering::Relaxed);
            return Err(TemporaryStorageError::Io(error));
        }
        Ok(path)
    }

    pub fn cleanup(&mut self) -> Result<(), TemporaryStorageError> {
        if !self.live {
            return Ok(());
        }
        if !self.path.join(MARKER).is_file() {
            return Err(TemporaryStorageError::MissingOwnershipMarker);
        }
        fs::remove_dir_all(&self.path)?;
        self.live = false;
        Ok(())
    }

    fn reserve_bytes(&self, length: u64) -> Result<(), TemporaryStorageError> {
        let Some(limit) = self.max_bytes else {
            self.used_bytes.fetch_add(length, Ordering::Relaxed);
            return Ok(());
        };
        let mut current = self.used_bytes.load(Ordering::Relaxed);
        loop {
            let next =
                current
                    .checked_add(length)
                    .ok_or(TemporaryStorageError::StorageLimitExceeded {
                        limit,
                        attempted: u64::MAX,
                    })?;
            if next > limit {
                return Err(TemporaryStorageError::StorageLimitExceeded {
                    limit,
                    attempted: next,
                });
            }
            match self.used_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }
}

impl std::fmt::Debug for PrivateTemporaryStorage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrivateTemporaryStorage")
            .field("retention", &self.retention)
            .field("live", &self.live)
            .finish_non_exhaustive()
    }
}

impl Drop for PrivateTemporaryStorage {
    fn drop(&mut self) {
        if self.retention == TemporaryRetentionPolicy::DeleteOnDrop {
            let _ = self.cleanup();
        }
    }
}

fn prepare_base(base: &Path) -> Result<PathBuf, TemporaryStorageError> {
    if base.as_os_str().is_empty() {
        return Err(TemporaryStorageError::InvalidBase);
    }
    if !base.exists() {
        fs::create_dir_all(base)?;
    }
    let metadata = fs::symlink_metadata(base)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(TemporaryStorageError::InvalidBase);
    }
    Ok(base.canonicalize()?)
}

fn valid_generated_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.chars().any(char::is_control)
        && !name.contains('/')
        && !name.contains('\\')
        && Path::new(name).file_name().and_then(|value| value.to_str()) == Some(name)
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), TemporaryStorageError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), TemporaryStorageError> {
    // Windows inherits the current user's ACL; creation never broadens it.
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum TemporaryStorageError {
    #[error("temporary storage base must be a real directory, not a link")]
    InvalidBase,
    #[error("temporary storage generated name is invalid")]
    InvalidGeneratedName,
    #[error("could not allocate private temporary storage")]
    NameAttemptsExhausted,
    #[error("private temporary storage ownership marker is missing")]
    MissingOwnershipMarker,
    #[error("temporary storage budget exceeded: {0}")]
    Budget(BudgetExceeded),
    #[error("temporary storage limit {limit} bytes would be exceeded by usage {attempted}")]
    StorageLimitExceeded { limit: u64, attempted: u64 },
    #[error("temporary storage I/O failed: {0}")]
    Io(#[from] io::Error),
}
