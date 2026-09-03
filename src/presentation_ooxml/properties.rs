//! Core, extended, and custom OOXML document-property parsing.

use super::archive::PackageEntry;
use super::model::{
    PresentationCustomProperty, PresentationProperties, PresentationProperty,
    PresentationRelationship, PresentationRelationshipTargetMode,
};
use super::xml::{attribute, descendant_text, local_name, parse_xml_part};
use super::{PARSER, xml_locator};
use crate::core::Diagnostic;
use std::collections::{BTreeMap, BTreeSet};

const CORE_REL: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const EXTENDED_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
const CUSTOM_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";

pub(super) fn parse_properties(
    entries: &[PackageEntry],
    relationships: &[PresentationRelationship],
    diagnostics: &mut Vec<Diagnostic>,
) -> PresentationProperties {
    let core =
        property_part(relationships, CORE_REL).or_else(|| existing(entries, "docProps/core.xml"));
    let extended = property_part(relationships, EXTENDED_REL)
        .or_else(|| existing(entries, "docProps/app.xml"));
    let custom = property_part(relationships, CUSTOM_REL)
        .or_else(|| existing(entries, "docProps/custom.xml"));
    PresentationProperties {
        core: core
            .and_then(|part| entry_bytes(entries, &part).map(|bytes| (part, bytes)))
            .map(|(part, bytes)| parse_simple_properties(&part, bytes, diagnostics))
            .unwrap_or_default(),
        extended: extended
            .and_then(|part| entry_bytes(entries, &part).map(|bytes| (part, bytes)))
            .map(|(part, bytes)| parse_simple_properties(&part, bytes, diagnostics))
            .unwrap_or_default(),
        custom: custom
            .and_then(|part| entry_bytes(entries, &part).map(|bytes| (part, bytes)))
            .map(|(part, bytes)| parse_custom_properties(&part, bytes, diagnostics))
            .unwrap_or_default(),
    }
}

fn property_part(
    relationships: &[PresentationRelationship],
    relationship_type: &str,
) -> Option<String> {
    relationships
        .iter()
        .find(|relationship| {
            relationship.source_part.is_none()
                && relationship.target_mode == PresentationRelationshipTargetMode::Internal
                && relationship.relationship_type == relationship_type
                && relationship.target_exists == Some(true)
        })
        .and_then(|relationship| relationship.resolved_part.clone())
}

fn existing(entries: &[PackageEntry], path: &str) -> Option<String> {
    entries
        .iter()
        .any(|entry| entry.path == path && entry.bytes.is_some() && entry.rejected.is_none())
        .then(|| path.to_string())
}

fn entry_bytes<'a>(entries: &'a [PackageEntry], path: &str) -> Option<&'a [u8]> {
    entries
        .iter()
        .find(|entry| entry.path == path && entry.rejected.is_none())?
        .bytes
        .as_deref()
}

fn parse_simple_properties(
    part: &str,
    bytes: &[u8],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PresentationProperty> {
    let Ok(nodes) = parse_xml_part(part, bytes, diagnostics) else {
        return Vec::new();
    };
    let Some(root) = nodes.first() else {
        return Vec::new();
    };
    let namespaces = namespaces(&root.attributes);
    root.children
        .iter()
        .map(|index| {
            let node = &nodes[*index];
            PresentationProperty {
                part: part.into(),
                namespace: namespace_for(&node.name, &namespaces),
                name: local_name(&node.name).into(),
                value: descendant_text(&nodes, *index).trim().to_string(),
                attributes: node.attributes.clone(),
                locator: xml_locator(part, &node.path),
            }
        })
        .collect()
}

fn parse_custom_properties(
    part: &str,
    bytes: &[u8],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PresentationCustomProperty> {
    let Ok(nodes) = parse_xml_part(part, bytes, diagnostics) else {
        return Vec::new();
    };
    let Some(root) = nodes.first() else {
        return Vec::new();
    };
    let mut properties = Vec::new();
    let mut names = BTreeSet::new();
    for index in &root.children {
        let node = &nodes[*index];
        if local_name(&node.name) != "property" {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "presentation_ooxml.properties.unknown_custom_element",
                    format!("unknown custom-property element {}", node.name),
                )
                .with_locator(xml_locator(part, &node.path))
                .partial(),
            );
            continue;
        }
        let name = attribute(node, "name").map(str::to_string);
        if name.as_deref().is_none_or(str::is_empty) {
            malformed_custom_property(part, &node.path, "missing name", diagnostics);
        }
        if let Some(name) = &name
            && !names.insert(name.clone())
        {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "presentation_ooxml.properties.duplicate_custom_name",
                    format!("duplicate custom property name {name}"),
                )
                .with_locator(xml_locator(part, &node.path))
                .partial(),
            );
        }
        let property_id = match attribute(node, "pid") {
            Some(value) => match value.parse() {
                Ok(value) => Some(value),
                Err(_) => {
                    malformed_custom_property(part, &node.path, "has invalid pid", diagnostics);
                    None
                }
            },
            None => {
                malformed_custom_property(part, &node.path, "missing pid", diagnostics);
                None
            }
        };
        let format_id = attribute(node, "fmtid").map(str::to_string);
        if format_id.as_deref().is_none_or(str::is_empty) {
            malformed_custom_property(part, &node.path, "missing fmtid", diagnostics);
        }
        let value_node = node.children.first().copied();
        if value_node.is_none() {
            malformed_custom_property(part, &node.path, "missing typed value", diagnostics);
        } else if node.children.len() > 1 {
            malformed_custom_property(part, &node.path, "has multiple typed values", diagnostics);
        }
        properties.push(PresentationCustomProperty {
            part: part.into(),
            name,
            property_id,
            format_id,
            link_target: attribute(node, "linkTarget").map(str::to_string),
            value_type: value_node.map(|value| local_name(&nodes[value].name).to_string()),
            value: value_node.map(|value| descendant_text(&nodes, value).trim().to_string()),
            attributes: node.attributes.clone(),
            locator: xml_locator(part, &node.path),
        });
    }
    properties
}

fn malformed_custom_property(
    part: &str,
    path: &str,
    message: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(
        Diagnostic::malformed(PARSER, format!("custom property in {part} {message}"))
            .with_locator(xml_locator(part, path))
            .partial(),
    );
}

fn namespaces(attributes: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    attributes
        .iter()
        .filter_map(|(name, value)| {
            name.strip_prefix("xmlns:")
                .map(|prefix| (prefix.to_string(), value.clone()))
                .or_else(|| (name == "xmlns").then(|| (String::new(), value.clone())))
        })
        .collect()
}

fn namespace_for(name: &str, namespaces: &BTreeMap<String, String>) -> Option<String> {
    let prefix = name.split_once(':').map_or("", |(prefix, _)| prefix);
    namespaces.get(prefix).cloned()
}
