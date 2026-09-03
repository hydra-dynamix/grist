//! Archive-member validation independent of any archive decoder.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveEntryKind {
    #[default]
    RegularFile,
    Directory,
    SymbolicLink,
    HardLink,
    BlockDevice,
    CharacterDevice,
    Fifo,
    Socket,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveSecurityPolicy {
    pub max_path_bytes: u64,
    pub max_path_components: u32,
}

impl Default for ArchiveSecurityPolicy {
    fn default() -> Self {
        Self {
            max_path_bytes: 4_096,
            max_path_components: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveMemberDescriptor<'a> {
    pub path: &'a str,
    pub kind: ArchiveEntryKind,
    pub link_target: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ArchiveRejection {
    pub code: &'static str,
    pub message: String,
    pub normalized_path: Option<String>,
}

impl ArchiveSecurityPolicy {
    /// Validate a member and return a cross-platform collision key. Both slash
    /// forms are separators, regardless of the host operating system.
    pub fn validate_member(
        &self,
        member: &ArchiveMemberDescriptor<'_>,
    ) -> Result<String, ArchiveRejection> {
        let normalized = normalize_member_path(member.path, self)?;
        if let Some(target) = member.link_target {
            normalize_member_path(target, self).map_err(|_| {
                rejection(
                    "grist.security.archive.unsafe_link_target",
                    "archive link target is not a safe relative path",
                    Some(normalized.clone()),
                )
            })?;
        }
        match member.kind {
            ArchiveEntryKind::SymbolicLink | ArchiveEntryKind::HardLink => {
                return Err(rejection(
                    "grist.security.archive.unsafe_link",
                    "archive links are inventoried but not extracted",
                    Some(normalized),
                ));
            }
            ArchiveEntryKind::BlockDevice
            | ArchiveEntryKind::CharacterDevice
            | ArchiveEntryKind::Fifo
            | ArchiveEntryKind::Socket => {
                return Err(rejection(
                    "grist.security.archive.device_file",
                    "archive device and special files are inventoried but not extracted",
                    Some(normalized),
                ));
            }
            _ => {}
        }
        Ok(normalized)
    }

    pub fn duplicate_rejection(&self, normalized_path: String) -> ArchiveRejection {
        rejection(
            "grist.security.archive.duplicate_path",
            "archive path is ambiguous after cross-platform normalization",
            Some(normalized_path),
        )
    }
}

fn normalize_member_path(
    path: &str,
    policy: &ArchiveSecurityPolicy,
) -> Result<String, ArchiveRejection> {
    if path.is_empty() || path.contains('\0') || path.chars().any(char::is_control) {
        return Err(rejection(
            "grist.security.archive.invalid_path",
            "archive member path is empty or contains control characters",
            None,
        ));
    }
    if u64::try_from(path.len()).unwrap_or(u64::MAX) > policy.max_path_bytes {
        return Err(rejection(
            "grist.security.archive.path_limit",
            "archive member path exceeds the configured byte limit",
            None,
        ));
    }
    let portable = path.replace('\\', "/");
    if portable.starts_with('/')
        || portable.as_bytes().get(1) == Some(&b':')
        || Path::new(&portable)
            .components()
            .any(|component| matches!(component, Component::RootDir | Component::Prefix(_)))
    {
        return Err(rejection(
            "grist.security.archive.absolute_path",
            "absolute archive member paths are inventoried but not extracted",
            None,
        ));
    }
    let mut components = Vec::new();
    for component in portable.split('/') {
        match component {
            "" | "." => continue,
            ".." => {
                return Err(rejection(
                    "grist.security.archive.path_traversal",
                    "archive member path contains a parent traversal",
                    None,
                ));
            }
            value => {
                if value.contains(':') || is_windows_device_name(value) {
                    return Err(rejection(
                        "grist.security.archive.device_path",
                        "archive member path names a Windows device or alternate data stream",
                        None,
                    ));
                }
                let collision_component = value.trim_end_matches([' ', '.']).to_ascii_lowercase();
                if collision_component.is_empty() {
                    return Err(rejection(
                        "grist.security.archive.invalid_path",
                        "archive member path contains an empty portable component",
                        None,
                    ));
                }
                components.push(collision_component);
            }
        }
    }
    if components.is_empty() {
        return Err(rejection(
            "grist.security.archive.invalid_path",
            "archive member path has no usable components",
            None,
        ));
    }
    if u32::try_from(components.len()).unwrap_or(u32::MAX) > policy.max_path_components {
        return Err(rejection(
            "grist.security.archive.path_limit",
            "archive member path exceeds the configured component limit",
            None,
        ));
    }
    Ok(components.join("/"))
}

fn is_windows_device_name(component: &str) -> bool {
    let stem = component
        .trim_end_matches([' ', '.'])
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .is_some_and(is_reserved_device_number)
        || stem
            .strip_prefix("lpt")
            .is_some_and(is_reserved_device_number)
}

fn is_reserved_device_number(value: &str) -> bool {
    matches!(value, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
}

fn rejection(
    code: &'static str,
    message: &'static str,
    normalized_path: Option<String>,
) -> ArchiveRejection {
    ArchiveRejection {
        code,
        message: message.into(),
        normalized_path,
    }
}
