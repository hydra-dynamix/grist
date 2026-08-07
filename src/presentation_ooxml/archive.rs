//! Bounded, read-only ZIP traversal for PresentationML packages.

use super::{PARSER, fallback_part_locator, part_locator};
use crate::core::Diagnostic;
use crate::registry::{ParserContext, ParserError};
use crate::security::{ArchiveEntryKind, ArchiveMemberDescriptor, ArchiveSecurityPolicy};
use std::collections::HashMap;
use std::io::{Cursor, Read};

#[derive(Debug, Clone)]
pub(super) struct PackageEntry {
    pub index: usize,
    pub path: String,
    pub compression: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    pub kind: ArchiveEntryKind,
    pub encrypted: bool,
    pub rejected: Option<(String, String)>,
    pub bytes: Option<Vec<u8>>,
}

pub(super) struct PackageArchive {
    pub entries: Vec<PackageEntry>,
    pub diagnostics: Vec<Diagnostic>,
    pub encrypted: bool,
}

pub(super) fn read_package(
    bytes: &[u8],
    context: &ParserContext<'_>,
) -> Result<PackageArchive, ParserError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
        Box::new(Diagnostic::malformed(
            PARSER,
            format!("invalid PresentationML ZIP package: {error}"),
        )) as ParserError
    })?;
    context.consume_archive_members(archive.len() as u64)?;
    let policy = ArchiveSecurityPolicy::default();
    let mut entries = Vec::with_capacity(archive.len());
    let mut collision_groups = HashMap::<String, Vec<usize>>::new();
    let mut compressed = 0u64;
    let mut expanded = 0u64;

    for index in 0..archive.len() {
        context.checkpoint()?;
        let file = archive.by_index_raw(index).map_err(|error| {
            Box::new(Diagnostic::malformed(
                PARSER,
                format!("cannot inspect ZIP member {index}: {error}"),
            )) as ParserError
        })?;
        let path = file.name().replace(char::from(92), "/");
        let kind = if file.is_dir() {
            ArchiveEntryKind::Directory
        } else if file.is_symlink() {
            ArchiveEntryKind::SymbolicLink
        } else {
            ArchiveEntryKind::RegularFile
        };
        let validation = policy.validate_member(&ArchiveMemberDescriptor {
            path: &path,
            kind,
            link_target: None,
        });
        let rejected = validation
            .as_ref()
            .err()
            .map(|error| (error.code.to_string(), error.message.clone()));
        if let Ok(collision_key) = validation {
            collision_groups
                .entry(collision_key)
                .or_default()
                .push(index);
        }
        compressed = compressed.saturating_add(file.compressed_size());
        expanded = expanded.saturating_add(file.size());
        entries.push(PackageEntry {
            index,
            path,
            compression: format!("{:?}", file.compression()).to_ascii_lowercase(),
            compressed_size: file.compressed_size(),
            uncompressed_size: file.size(),
            crc32: file.crc32(),
            kind,
            encrypted: file.encrypted(),
            rejected,
            bytes: None,
        });
    }

    context.observe_archive_expansion(compressed, expanded)?;
    context.observe_memory_bytes(expanded)?;
    for indexes in collision_groups
        .values()
        .filter(|indexes| indexes.len() > 1)
    {
        for index in indexes {
            entries[*index].rejected = Some((
                "grist.security.archive.duplicate_path".into(),
                "archive path is ambiguous after cross-platform normalization".into(),
            ));
        }
    }

    let encrypted = entries.iter().any(|entry| entry.encrypted);
    let mut diagnostics = Vec::new();
    for entry in &entries {
        if let Some((code, message)) = &entry.rejected {
            let locator =
                part_locator(&entry.path).unwrap_or_else(|_| fallback_part_locator(entry.index));
            diagnostics.push(
                Diagnostic::warning(PARSER, code, message)
                    .with_locator(locator)
                    .partial(),
            );
        }
    }

    if !encrypted {
        for entry in entries.iter_mut().filter(|entry| {
            entry.rejected.is_none()
                && entry.kind == ArchiveEntryKind::RegularFile
                && !entry.encrypted
        }) {
            context.checkpoint()?;
            let file = archive.by_index(entry.index).map_err(|error| {
                Box::new(Diagnostic::malformed(
                    PARSER,
                    format!("cannot read OOXML part {}: {error}", entry.path),
                )) as ParserError
            })?;
            let declared = entry.uncompressed_size;
            let mut bounded = file.take(declared.saturating_add(1));
            let mut decoded = Vec::with_capacity(
                usize::try_from(declared.min(usize::MAX as u64)).unwrap_or(usize::MAX),
            );
            bounded.read_to_end(&mut decoded).map_err(|error| {
                Box::new(Diagnostic::malformed(
                    PARSER,
                    format!("cannot decompress OOXML part {}: {error}", entry.path),
                )) as ParserError
            })?;
            if u64::try_from(decoded.len()).unwrap_or(u64::MAX) != declared {
                return Err(Box::new(Diagnostic::malformed(
                    PARSER,
                    format!(
                        "OOXML part {} expanded to a length different from its central-directory declaration",
                        entry.path
                    ),
                )));
            }
            entry.bytes = Some(decoded);
        }
    }

    Ok(PackageArchive {
        entries,
        diagnostics,
        encrypted,
    })
}
