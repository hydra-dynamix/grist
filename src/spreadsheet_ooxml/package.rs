use super::model::*;
use super::xml::{attr, descendant_text, local_name, parse_xml_part};
use super::{PARSER, part_locator, xml_locator};
use crate::core::{ContentIdentity, Diagnostic, FormatIdentity, OperationStatus};
use crate::registry::{ParserContext, ParserError};
use crate::security::{ArchiveEntryKind, ArchiveMemberDescriptor, ArchiveSecurityPolicy};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Cursor, Read};

#[derive(Debug, Clone)]
pub(super) struct PackageEntry {
    pub index: usize,
    pub path: String,
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
            format!("invalid SpreadsheetML ZIP package: {error}"),
        ))
    })?;
    context.consume_archive_members(archive.len() as u64)?;
    let policy = ArchiveSecurityPolicy::default();
    let mut entries = Vec::with_capacity(archive.len());
    let mut collisions = HashMap::<String, Vec<usize>>::new();
    let mut compressed = 0u64;
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        context.checkpoint()?;
        let file = archive.by_index_raw(index).map_err(|error| {
            Box::new(Diagnostic::malformed(
                PARSER,
                format!("cannot inspect ZIP member {index}: {error}"),
            ))
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
        if let Ok(key) = validation {
            collisions.entry(key).or_default().push(index);
        }
        compressed = compressed.saturating_add(file.compressed_size());
        expanded = expanded.saturating_add(file.size());
        entries.push(PackageEntry {
            index,
            path,
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
    for indexes in collisions.values().filter(|values| values.len() > 1) {
        for index in indexes {
            entries[*index].rejected = Some((
                "grist.security.archive.duplicate_path".into(),
                "archive path is ambiguous after cross-platform normalization".into(),
            ));
        }
    }
    let encrypted = entries.iter().any(|entry| entry.encrypted);
    let diagnostics = entries
        .iter()
        .filter_map(|entry| {
            entry.rejected.as_ref().map(|(code, message)| {
                Diagnostic::warning(PARSER, code, message)
                    .with_locator(part_locator(&entry.path))
                    .partial()
            })
        })
        .collect();
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
                ))
            })?;
            let declared = entry.uncompressed_size;
            let mut data = Vec::with_capacity(usize::try_from(declared).unwrap_or(0));
            file.take(declared.saturating_add(1))
                .read_to_end(&mut data)
                .map_err(|error| {
                    Box::new(Diagnostic::malformed(
                        PARSER,
                        format!("cannot decompress OOXML part {}: {error}", entry.path),
                    ))
                })?;
            if u64::try_from(data.len()).unwrap_or(u64::MAX) != declared {
                return Err(Box::new(Diagnostic::malformed(
                    PARSER,
                    format!(
                        "OOXML part {} length differs from its ZIP declaration",
                        entry.path
                    ),
                )));
            }
            entry.bytes = Some(data);
        }
    }
    Ok(PackageArchive {
        entries,
        diagnostics,
        encrypted,
    })
}

#[derive(Default)]
pub(super) struct ContentTypes {
    pub defaults: BTreeMap<String, String>,
    pub overrides: BTreeMap<String, String>,
}

pub(super) struct Manifest {
    pub kind: SpreadsheetPackageKind,
    pub workbook_part: String,
    pub content_types: ContentTypes,
}

pub(super) fn parse_manifest(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Manifest, ParserError> {
    let entry = available(entries, "[Content_Types].xml").ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "OOXML package has no available [Content_Types].xml",
        ))
    })?;
    let nodes = parse_xml_part(
        &entry.path,
        entry.bytes.as_deref().unwrap_or_default(),
        diagnostics,
    )
    .map_err(|_| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "cannot parse content type manifest",
        ))
    })?;
    let root = nodes
        .first()
        .filter(|node| local_name(&node.name) == "Types")
        .ok_or_else(|| {
            Box::new(Diagnostic::malformed(
                PARSER,
                "content type manifest has no Types root",
            ))
        })?;
    let mut types = ContentTypes::default();
    for child in &root.children {
        let node = &nodes[*child];
        match local_name(&node.name) {
            "Default" => {
                if let (Some(extension), Some(content_type)) =
                    (attr(node, "Extension"), attr(node, "ContentType"))
                {
                    types
                        .defaults
                        .insert(extension.to_ascii_lowercase(), content_type.to_string());
                }
            }
            "Override" => {
                if let (Some(part), Some(content_type)) =
                    (attr(node, "PartName"), attr(node, "ContentType"))
                {
                    types.overrides.insert(
                        part.trim_start_matches('/').to_string(),
                        content_type.to_string(),
                    );
                }
            }
            _ => {}
        }
    }
    let (workbook_part, kind) = types
        .overrides
        .iter()
        .find_map(|(part, content_type)| {
            if content_type
                == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
            {
                Some((part.clone(), SpreadsheetPackageKind::Workbook))
            } else if content_type == "application/vnd.ms-excel.sheet.macroEnabled.main+xml" {
                Some((part.clone(), SpreadsheetPackageKind::MacroEnabledWorkbook))
            } else {
                None
            }
        })
        .ok_or_else(|| {
            Box::new(Diagnostic::malformed(
                PARSER,
                "content type manifest declares no supported SpreadsheetML workbook part",
            ))
        })?;
    Ok(Manifest {
        kind,
        workbook_part,
        content_types: types,
    })
}

pub(super) fn content_type(types: &ContentTypes, path: &str) -> Option<String> {
    types.overrides.get(path).cloned().or_else(|| {
        path.rsplit_once('.')
            .and_then(|(_, extension)| types.defaults.get(&extension.to_ascii_lowercase()).cloned())
    })
}

pub(super) fn available<'a>(entries: &'a [PackageEntry], path: &str) -> Option<&'a PackageEntry> {
    entries.iter().find(|entry| {
        entry.path == path && entry.bytes.is_some() && entry.rejected.is_none() && !entry.encrypted
    })
}

pub(super) fn parse_relationships(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SpreadsheetRelationship> {
    let available_paths = entries
        .iter()
        .filter(|entry| entry.bytes.is_some() && entry.rejected.is_none())
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut output = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in entries.iter().filter(|entry| {
        entry.path.ends_with(".rels") && entry.bytes.is_some() && entry.rejected.is_none()
    }) {
        let Some(source_part) = relationship_source(&entry.path) else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "spreadsheet_ooxml.relationship.invalid_part",
                    format!("relationship part {} has no valid OPC source", entry.path),
                )
                .with_locator(part_locator(&entry.path))
                .partial(),
            );
            continue;
        };
        let Ok(nodes) = parse_xml_part(
            &entry.path,
            entry.bytes.as_deref().unwrap_or_default(),
            diagnostics,
        ) else {
            continue;
        };
        let Some(root) = nodes
            .first()
            .filter(|node| local_name(&node.name) == "Relationships")
        else {
            continue;
        };
        for child in &root.children {
            let node = &nodes[*child];
            if local_name(&node.name) != "Relationship" {
                continue;
            }
            let (Some(id), Some(kind), Some(target)) =
                (attr(node, "Id"), attr(node, "Type"), attr(node, "Target"))
            else {
                diagnostics.push(
                    Diagnostic::malformed(
                        PARSER,
                        format!("incomplete relationship in {}", entry.path),
                    )
                    .with_locator(xml_locator(&entry.path, &node.path))
                    .partial(),
                );
                continue;
            };
            if !seen.insert((source_part.clone(), id.to_string())) {
                diagnostics.push(
                    Diagnostic::malformed(
                        PARSER,
                        format!("duplicate relationship ID {id} in {}", entry.path),
                    )
                    .with_locator(xml_locator(&entry.path, &node.path))
                    .partial(),
                );
                continue;
            }
            let mode = if attr(node, "TargetMode")
                .is_some_and(|value| value.eq_ignore_ascii_case("External"))
            {
                SpreadsheetRelationshipTargetMode::External
            } else {
                SpreadsheetRelationshipTargetMode::Internal
            };
            let resolved_part = (mode == SpreadsheetRelationshipTargetMode::Internal)
                .then(|| resolve_target(source_part.as_deref(), target))
                .flatten();
            let target_exists = resolved_part
                .as_deref()
                .map(|path| available_paths.contains(path));
            if target_exists == Some(false) {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "spreadsheet_ooxml.relationship.missing_target",
                        format!("relationship {id} targets missing part {target}"),
                    )
                    .with_locator(xml_locator(&entry.path, &node.path))
                    .partial(),
                );
            }
            output.push(SpreadsheetRelationship {
                source_part: source_part.clone(),
                id: id.to_string(),
                relationship_type: kind.to_string(),
                target: target.to_string(),
                target_mode: mode,
                resolved_part,
                target_exists,
                locator: xml_locator(&entry.path, &node.path),
            });
        }
    }
    output.sort_by(|left, right| {
        left.source_part
            .cmp(&right.source_part)
            .then(left.id.cmp(&right.id))
    });
    output
}

fn relationship_source(path: &str) -> Option<Option<String>> {
    if path == "_rels/.rels" {
        return Some(None);
    }
    let (directory, file) = path.rsplit_once("/_rels/")?;
    let source = file.strip_suffix(".rels")?;
    Some(Some(if directory.is_empty() {
        source.to_string()
    } else {
        format!("{directory}/{source}")
    }))
}

fn resolve_target(source: Option<&str>, target: &str) -> Option<String> {
    if target.starts_with('/') {
        return normalize_path(target.trim_start_matches('/'));
    }
    let base = source
        .and_then(|path| path.rsplit_once('/').map(|(directory, _)| directory))
        .unwrap_or("");
    let joined = if base.is_empty() {
        target.to_string()
    } else {
        format!("{base}/{target}")
    };
    normalize_path(&joined)
}

fn normalize_path(path: &str) -> Option<String> {
    let mut output = Vec::new();
    let normalized = path.replace(char::from(92), "/");
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                output.pop()?;
            }
            value => output.push(value),
        }
    }
    (!output.is_empty()).then(|| output.join("/"))
}

pub(super) fn relationship<'a>(
    relationships: &'a [SpreadsheetRelationship],
    source: Option<&str>,
    id: &str,
) -> Option<&'a SpreadsheetRelationship> {
    relationships
        .iter()
        .find(|item| item.source_part.as_deref() == source && item.id == id)
}

pub(super) fn package_parts(
    entries: &[PackageEntry],
    types: &ContentTypes,
) -> Vec<SpreadsheetPackagePart> {
    entries
        .iter()
        .map(|entry| SpreadsheetPackagePart {
            package_index: entry.index,
            path: entry.path.clone(),
            content_type: content_type(types, &entry.path),
            compressed_size: entry.compressed_size,
            uncompressed_size: entry.uncompressed_size,
            crc32: entry.crc32,
            status: if entry.rejected.is_some() {
                "rejected"
            } else if entry.encrypted {
                "encrypted"
            } else if entry.kind == ArchiveEntryKind::Directory {
                "directory"
            } else {
                "available"
            }
            .into(),
            identity: entry.bytes.as_deref().map(|bytes| {
                ContentIdentity::for_raw_bytes(bytes).with_format(FormatIdentity::new(
                    "spreadsheet_ooxml_part",
                    content_type(types, &entry.path),
                ))
            }),
            locator: part_locator(&entry.path),
        })
        .collect()
}

pub(super) fn macro_projects(
    entries: &[PackageEntry],
    types: &ContentTypes,
) -> Vec<SpreadsheetMacroProject> {
    entries
        .iter()
        .filter_map(|entry| {
            let content = content_type(types, &entry.path);
            let is_macro = entry.path.to_ascii_lowercase().ends_with("vbaproject.bin")
                || content
                    .as_deref()
                    .is_some_and(|value| value.to_ascii_lowercase().contains("vba"));
            let bytes = entry.bytes.as_deref()?;
            is_macro.then(|| SpreadsheetMacroProject {
                part: entry.path.clone(),
                content_type: content,
                identity: ContentIdentity::for_raw_bytes(bytes).with_format(FormatIdentity::new(
                    "vba_project",
                    Some("application/vnd.ms-office.vbaProject"),
                )),
                byte_length: entry.uncompressed_size,
                classification: "active_content_macro_project".into(),
                quarantined: true,
                executable: false,
                locator: part_locator(&entry.path),
            })
        })
        .collect()
}

pub(super) fn parse_properties(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SpreadsheetProperty> {
    let mut output = Vec::new();
    for entry in entries.iter().filter(|entry| {
        matches!(
            entry.path.as_str(),
            "docProps/core.xml" | "docProps/app.xml" | "docProps/custom.xml"
        )
    }) {
        let Some(bytes) = entry.bytes.as_deref() else {
            continue;
        };
        let Ok(nodes) = parse_xml_part(&entry.path, bytes, diagnostics) else {
            continue;
        };
        let Some(root) = nodes.first() else { continue };
        for child in &root.children {
            let node = &nodes[*child];
            let value = descendant_text(&nodes, *child);
            if value.is_empty() {
                continue;
            }
            let value_type = node
                .children
                .first()
                .map(|index| local_name(&nodes[*index].name).to_string());
            output.push(SpreadsheetProperty {
                part: entry.path.clone(),
                name: attr(node, "name")
                    .unwrap_or(local_name(&node.name))
                    .to_string(),
                value,
                value_type,
                locator: xml_locator(&entry.path, &node.path),
            });
        }
    }
    output
}

pub(super) fn encrypted_output() -> crate::registry::ParserOutput {
    crate::registry::ParserOutput::terminal(
        OperationStatus::Encrypted,
        vec![Diagnostic::error(
            PARSER,
            "spreadsheet_ooxml.encrypted",
            "encrypted workbook requires an explicit decryption provider",
        )],
    )
}
