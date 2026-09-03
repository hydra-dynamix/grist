//! Slide order, master/layout/theme, and inert action inventory.

use super::archive::PackageEntry;
use super::model::{
    PresentationAction, PresentationActionKind, PresentationRelationship,
    PresentationRelationshipTargetMode, PresentationSlideReference, PresentationStructuralPart,
    PresentationStructuralPartKind,
};
use super::xml::{attribute, local_name, parse_xml_part};
use super::{PARSER, xml_locator};
use crate::core::{ContentIdentity, Diagnostic, FormatIdentity};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct PresentationStructure {
    pub slides: Vec<PresentationSlideReference>,
    pub masters: Vec<PresentationStructuralPart>,
    pub layouts: Vec<PresentationStructuralPart>,
    pub themes: Vec<PresentationStructuralPart>,
    pub actions: Vec<PresentationAction>,
}

pub(super) fn parse_structure(
    entries: &[PackageEntry],
    relationships: &[PresentationRelationship],
    main_part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> PresentationStructure {
    let mut slides = Vec::new();
    let mut masters = Vec::new();
    if let Some(bytes) = entry_bytes(entries, main_part)
        && let Ok(nodes) = parse_xml_part(main_part, bytes, diagnostics)
    {
        let mut seen_slide_ids = BTreeSet::new();
        for node in &nodes {
            match local_name(&node.name) {
                "sldId" => {
                    let Some(slide_id) = attribute(node, "id").map(str::to_string) else {
                        malformed(
                            main_part,
                            &node.path,
                            "slide reference has no id",
                            diagnostics,
                        );
                        continue;
                    };
                    let Some(relationship_id) = attribute(node, "r:id")
                        .or_else(|| attribute(node, "id").filter(|value| *value != slide_id))
                        .map(str::to_string)
                    else {
                        malformed(
                            main_part,
                            &node.path,
                            "slide reference has no relationship ID",
                            diagnostics,
                        );
                        continue;
                    };
                    if !seen_slide_ids.insert(slide_id.clone()) {
                        malformed(main_part, &node.path, "duplicate slide ID", diagnostics);
                    }
                    let relationship = relationship(relationships, main_part, &relationship_id);
                    let part = relationship.and_then(|item| item.resolved_part.clone());
                    if relationship.is_none()
                        || relationship.is_some_and(|item| item.target_exists != Some(true))
                    {
                        malformed(
                            main_part,
                            &node.path,
                            "slide relationship is missing or dangling",
                            diagnostics,
                        );
                    }
                    slides.push(PresentationSlideReference {
                        order: slides.len(),
                        slide_id,
                        relationship_id,
                        part_identity: part.as_deref().and_then(|path| identity(entries, path)),
                        part,
                        hidden: attribute(node, "show").is_some_and(is_false),
                        locator: xml_locator(main_part, &node.path),
                    });
                }
                "sldMasterId" => {
                    let relationship_id = attribute(node, "r:id").map(str::to_string);
                    let Some(part) = relationship_id
                        .as_deref()
                        .and_then(|id| relationship(relationships, main_part, id))
                        .and_then(|item| item.resolved_part.clone())
                    else {
                        malformed(
                            main_part,
                            &node.path,
                            "slide master reference is missing or dangling",
                            diagnostics,
                        );
                        continue;
                    };
                    if let Some(part_identity) = identity(entries, &part) {
                        masters.push(PresentationStructuralPart {
                            kind: PresentationStructuralPartKind::Master,
                            native_id: attribute(node, "id").map(str::to_string),
                            relationship_id,
                            source_part: main_part.to_string(),
                            locator: xml_locator(main_part, &node.path),
                            part,
                            part_identity,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    let layouts = relationship_parts(
        entries,
        relationships,
        PresentationStructuralPartKind::Layout,
        "/slideLayout",
    );
    let themes = relationship_parts(
        entries,
        relationships,
        PresentationStructuralPartKind::Theme,
        "/theme",
    );
    add_unlisted_masters(entries, relationships, &mut masters);
    let actions = parse_actions(entries, relationships, diagnostics);
    PresentationStructure {
        slides,
        masters,
        layouts,
        themes,
        actions,
    }
}

fn relationship_parts(
    entries: &[PackageEntry],
    relationships: &[PresentationRelationship],
    kind: PresentationStructuralPartKind,
    suffix: &str,
) -> Vec<PresentationStructuralPart> {
    let mut by_part = BTreeMap::new();
    for item in relationships.iter().filter(|item| {
        item.target_mode == PresentationRelationshipTargetMode::Internal
            && item.relationship_type.ends_with(suffix)
            && item.target_exists == Some(true)
    }) {
        let (Some(source_part), Some(part)) = (&item.source_part, &item.resolved_part) else {
            continue;
        };
        if let Some(part_identity) = identity(entries, part) {
            by_part
                .entry(part.clone())
                .or_insert_with(|| PresentationStructuralPart {
                    kind,
                    native_id: None,
                    relationship_id: Some(item.id.clone()),
                    source_part: source_part.clone(),
                    part: part.clone(),
                    part_identity,
                    locator: item.locator.clone(),
                });
        }
    }
    by_part.into_values().collect()
}

fn add_unlisted_masters(
    entries: &[PackageEntry],
    relationships: &[PresentationRelationship],
    masters: &mut Vec<PresentationStructuralPart>,
) {
    let known = masters
        .iter()
        .map(|item| item.part.clone())
        .collect::<BTreeSet<_>>();
    let extras = relationship_parts(
        entries,
        relationships,
        PresentationStructuralPartKind::Master,
        "/slideMaster",
    );
    masters.extend(
        extras
            .into_iter()
            .filter(|item| !known.contains(&item.part)),
    );
}

fn parse_actions(
    entries: &[PackageEntry],
    relationships: &[PresentationRelationship],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PresentationAction> {
    let mut actions = Vec::new();
    let mut referenced = BTreeSet::new();
    for entry in entries.iter().filter(|entry| {
        entry.path.starts_with("ppt/")
            && entry.path.ends_with(".xml")
            && entry.bytes.is_some()
            && entry.rejected.is_none()
    }) {
        let Some(bytes) = entry.bytes.as_deref() else {
            continue;
        };
        let Ok(nodes) = parse_xml_part(&entry.path, bytes, diagnostics) else {
            continue;
        };
        for node in &nodes {
            let local = local_name(&node.name);
            let action = attribute(node, "action").map(str::to_string);
            if !matches!(local, "hlinkClick" | "hlinkHover") && action.is_none() {
                continue;
            }
            let relationship_id = attribute(node, "r:id").map(str::to_string);
            let related = relationship_id
                .as_deref()
                .and_then(|id| relationship(relationships, &entry.path, id));
            if let Some(id) = &relationship_id {
                referenced.insert((entry.path.clone(), id.clone()));
            }
            actions.push(PresentationAction {
                source_part: entry.path.clone(),
                action_kind: match local {
                    "hlinkClick" => PresentationActionKind::Click,
                    "hlinkHover" => PresentationActionKind::Hover,
                    _ => PresentationActionKind::Action,
                },
                relationship_id,
                action,
                target: related.map(|item| item.target.clone()),
                external: related.is_some_and(|item| {
                    item.target_mode == PresentationRelationshipTargetMode::External
                }),
                locator: xml_locator(&entry.path, &node.path),
            });
        }
    }
    for item in relationships.iter().filter(|item| {
        item.relationship_type
            .to_ascii_lowercase()
            .ends_with("/hyperlink")
            && item
                .source_part
                .as_ref()
                .is_some_and(|source| !referenced.contains(&(source.clone(), item.id.clone())))
    }) {
        actions.push(PresentationAction {
            source_part: item.source_part.clone().unwrap_or_default(),
            action_kind: PresentationActionKind::Hyperlink,
            relationship_id: Some(item.id.clone()),
            action: None,
            target: Some(item.target.clone()),
            external: item.target_mode == PresentationRelationshipTargetMode::External,
            locator: item.locator.clone(),
        });
    }
    actions.sort_by(|left, right| {
        left.source_part
            .cmp(&right.source_part)
            .then_with(|| {
                left.relationship_id
                    .as_deref()
                    .cmp(&right.relationship_id.as_deref())
            })
            .then_with(|| left.action.cmp(&right.action))
    });
    actions
}

fn relationship<'a>(
    relationships: &'a [PresentationRelationship],
    source: &str,
    id: &str,
) -> Option<&'a PresentationRelationship> {
    relationships
        .iter()
        .find(|item| item.source_part.as_deref() == Some(source) && item.id == id)
}

fn entry_bytes<'a>(entries: &'a [PackageEntry], path: &str) -> Option<&'a [u8]> {
    entries
        .iter()
        .find(|entry| entry.path == path && entry.rejected.is_none())?
        .bytes
        .as_deref()
}

fn identity(entries: &[PackageEntry], path: &str) -> Option<ContentIdentity> {
    let bytes = entry_bytes(entries, path)?;
    Some(
        ContentIdentity::for_raw_bytes(bytes).with_format(FormatIdentity::new(
            "presentation_ooxml_part",
            None::<String>,
        )),
    )
}

fn malformed(part: &str, path: &str, message: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(
        Diagnostic::malformed(PARSER, message)
            .with_locator(xml_locator(part, path))
            .partial(),
    );
}

fn is_false(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}
