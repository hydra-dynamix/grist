//! OPC relationship parsing with package-root-bounded target resolution.

use super::archive::PackageEntry;
use super::model::{WordRelationship, WordRelationshipTargetMode};
use super::xml_util::{attribute, local_name, parse_xml_part};
use super::{PARSER, xml_locator};
use crate::core::Diagnostic;
use crate::security::{ArchiveEntryKind, ArchiveMemberDescriptor, ArchiveSecurityPolicy};
use std::collections::{BTreeSet, HashMap};

pub(super) const OFFICE_DOCUMENT_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

pub(super) fn parse_relationships(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<WordRelationship> {
    let available = entries
        .iter()
        .filter(|entry| entry.bytes.is_some() && entry.rejected.is_none())
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut relationships = Vec::new();
    let mut seen = HashMap::<(Option<String>, String), String>::new();
    for entry in entries.iter().filter(|entry| {
        entry.path.ends_with(".rels") && entry.bytes.is_some() && entry.rejected.is_none()
    }) {
        let Some(source_part) = relationship_source(&entry.path) else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.relationship.invalid_part_name",
                    format!(
                        "relationship part {} has no valid OPC source part",
                        entry.path
                    ),
                )
                .with_locator(super::part_locator(&entry.path).expect("validated package path"))
                .partial(),
            );
            continue;
        };
        if let Some(source) = source_part.as_deref()
            && !available.contains(source)
        {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.relationship.orphan_source",
                    format!(
                        "relationship part {} belongs to missing source part {source}",
                        entry.path
                    ),
                )
                .with_locator(super::part_locator(&entry.path).expect("validated package path"))
                .partial(),
            );
        }
        let Some(bytes) = entry.bytes.as_deref() else {
            continue;
        };
        let Ok(nodes) = parse_xml_part(&entry.path, bytes, diagnostics) else {
            continue;
        };
        let Some(root) = nodes
            .first()
            .filter(|node| local_name(&node.name) == "Relationships")
        else {
            diagnostics.push(
                Diagnostic::malformed(
                    PARSER,
                    format!("relationship part {} has no Relationships root", entry.path),
                )
                .with_locator(super::part_locator(&entry.path).expect("validated package path"))
                .partial(),
            );
            continue;
        };
        for child in &root.children {
            let node = &nodes[*child];
            if local_name(&node.name) != "Relationship" {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "word_ooxml.relationship.unknown_element",
                        format!("unknown relationship element {}", node.name),
                    )
                    .with_locator(xml_locator(&entry.path, &node.path))
                    .partial(),
                );
                continue;
            }
            let Some(id) = attribute(node, "Id").filter(|value| !value.is_empty()) else {
                malformed_relationship(&entry.path, &node.path, "missing Id", diagnostics);
                continue;
            };
            let Some(relationship_type) = attribute(node, "Type").filter(|value| !value.is_empty())
            else {
                malformed_relationship(&entry.path, &node.path, "missing Type", diagnostics);
                continue;
            };
            let Some(target) = attribute(node, "Target").filter(|value| !value.is_empty()) else {
                malformed_relationship(&entry.path, &node.path, "missing Target", diagnostics);
                continue;
            };
            let mode = match attribute(node, "TargetMode") {
                None => WordRelationshipTargetMode::Internal,
                Some(value) if value.eq_ignore_ascii_case("Internal") => {
                    WordRelationshipTargetMode::Internal
                }
                Some(value) if value.eq_ignore_ascii_case("External") => {
                    WordRelationshipTargetMode::External
                }
                Some(value) => {
                    diagnostics.push(
                        Diagnostic::warning(
                            PARSER,
                            "word_ooxml.relationship.unknown_target_mode",
                            format!(
                                "relationship {id} has unsupported TargetMode {value:?}; treating it as external"
                            ),
                        )
                        .with_locator(xml_locator(&entry.path, &node.path))
                        .partial(),
                    );
                    // Conservative classification: an unknown mode is never resolved or fetched.
                    WordRelationshipTargetMode::External
                }
            };
            let locator = xml_locator(&entry.path, &node.path);
            let (resolved_part, target_exists) = match mode {
                WordRelationshipTargetMode::External => (None, None),
                WordRelationshipTargetMode::Internal => {
                    match resolve_internal_target(source_part.as_deref(), target) {
                        Ok(part) => {
                            let exists = available.contains(part.as_str());
                            if !exists {
                                diagnostics.push(
                                    Diagnostic::warning(
                                        PARSER,
                                        "word_ooxml.relationship.dangling_target",
                                        format!("relationship {id} targets missing part {part}"),
                                    )
                                    .with_locator(locator.clone())
                                    .partial(),
                                );
                            }
                            (Some(part), Some(exists))
                        }
                        Err(message) => {
                            diagnostics.push(
                                Diagnostic::warning(
                                    PARSER,
                                    "word_ooxml.relationship.unsafe_target",
                                    message,
                                )
                                .with_locator(locator.clone())
                                .partial(),
                            );
                            (None, Some(false))
                        }
                    }
                }
            };
            let key = (source_part.clone(), id.to_string());
            if let Some(previous_part) = seen.insert(key, entry.path.clone()) {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "word_ooxml.relationship.duplicate_id",
                        format!(
                            "duplicate relationship ID {id} for the same source (first in {previous_part})"
                        ),
                    )
                    .with_locator(locator.clone())
                    .partial(),
                );
            }
            relationships.push(WordRelationship {
                relationship_part: entry.path.clone(),
                source_part: source_part.clone(),
                id: id.to_string(),
                relationship_type: relationship_type.to_string(),
                target: target.to_string(),
                target_mode: mode,
                resolved_part,
                target_exists,
                locator,
            });
        }
    }
    relationships.sort_by(|left, right| {
        left.relationship_part
            .cmp(&right.relationship_part)
            .then_with(|| left.id.cmp(&right.id))
    });
    relationships
}

/// `Some(None)` is the package root. `None` is not a relationship part name.
fn relationship_source(path: &str) -> Option<Option<String>> {
    if path == "_rels/.rels" {
        return Some(None);
    }
    let (prefix, filename) = path.rsplit_once("/_rels/")?;
    let source_name = filename.strip_suffix(".rels")?;
    (!source_name.is_empty()).then(|| Some(format!("{prefix}/{source_name}")))
}

fn malformed_relationship(
    part: &str,
    path: &str,
    message: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(
        Diagnostic::malformed(PARSER, format!("relationship in {part} is {message}"))
            .with_locator(xml_locator(part, path))
            .partial(),
    );
}

pub(super) fn resolve_internal_target(
    source_part: Option<&str>,
    target: &str,
) -> Result<String, String> {
    let target = target
        .split(['#', '?'])
        .next()
        .ok_or_else(|| "relationship target is empty".to_string())?;
    let decoded = percent_decode(target)?;
    let portable = decoded.replace(char::from(92), "/");
    let mut components = Vec::<String>::new();
    if !portable.starts_with('/')
        && let Some(source) = source_part
        && let Some((directory, _)) = source.rsplit_once('/')
    {
        components.extend(directory.split('/').map(str::to_string));
    }
    for component in portable.trim_start_matches('/').split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err(format!(
                        "relationship target {target:?} escapes the package root"
                    ));
                }
            }
            value => components.push(value.to_string()),
        }
    }
    let resolved = components.join("/");
    if resolved.is_empty() {
        return Err("relationship target resolves to an empty package part".into());
    }
    ArchiveSecurityPolicy::default()
        .validate_member(&ArchiveMemberDescriptor {
            path: &resolved,
            kind: ArchiveEntryKind::RegularFile,
            link_target: None,
        })
        .map_err(|error| format!("unsafe relationship target {target:?}: {}", error.message))?;
    Ok(resolved)
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let encoded = bytes.get(index + 1..index + 3).ok_or_else(|| {
                format!("relationship target {value:?} has an incomplete percent escape")
            })?;
            let text = std::str::from_utf8(encoded).map_err(|_| {
                format!("relationship target {value:?} has an invalid percent escape")
            })?;
            let byte = u8::from_str_radix(text, 16).map_err(|_| {
                format!("relationship target {value:?} has an invalid percent escape")
            })?;
            output.push(byte);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output)
        .map_err(|_| format!("relationship target {value:?} is not valid UTF-8 after decoding"))
}
