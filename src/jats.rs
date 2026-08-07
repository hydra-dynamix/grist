//! Typed, deterministic JATS scholarly cross-link resolution.
//!
//! The XML tree remains authoritative for syntax. This module adds the JATS
//! semantic view without erasing malformed, duplicate, or unresolved links.

use crate::core::{Diagnostic, DiagnosticClass, SourceLocator};
use crate::xml::{XmlAttribute, XmlNode, XmlNodeKind};
#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const JATS_SCHOLARLY_LINKS_SCHEMA_VERSION: &str = "grist/jats-scholarly-links/v1";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JatsScholarlyLinks {
    pub schema_version: String,
    pub targets: Vec<JatsTarget>,
    pub relationships: Vec<JatsRelationship>,
    pub labels: Vec<JatsLabel>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JatsTarget {
    pub node_id: String,
    pub xml_id: String,
    pub kind: JatsTargetKind,
    pub label: Option<String>,
    pub title: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JatsTargetKind {
    BibliographyEntry,
    Figure,
    Table,
    Supplement,
    Section,
    Footnote,
    Metadata,
    Label,
    Other(String),
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JatsRelationship {
    pub id: String,
    pub source_node_id: String,
    pub scope: JatsReferenceScope,
    pub kind: JatsRelationshipKind,
    pub ref_type: Option<String>,
    /// The complete source attribute value, retained even when it is empty or malformed.
    pub raw_rid: String,
    /// One whitespace-separated IDREF token. `None` represents an empty `rid`.
    pub target_xml_id: Option<String>,
    pub target_node_id: Option<String>,
    pub target_kind: Option<JatsTargetKind>,
    pub resolution: JatsResolution,
    /// All candidates are retained when duplicate XML IDs make resolution ambiguous.
    pub candidate_node_ids: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JatsReferenceScope {
    Body,
    FrontMetadata,
    BackMatter,
    Other,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JatsRelationshipKind {
    Citation,
    FigureReference,
    TableReference,
    SupplementReference,
    SectionReference,
    FootnoteReference,
    MetadataReference,
    OtherReference,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JatsResolution {
    Resolved,
    Unresolved,
    Ambiguous,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JatsLabel {
    pub node_id: String,
    pub owner_node_id: Option<String>,
    pub owner_xml_id: Option<String>,
    pub value: String,
    pub locator: SourceLocator,
}

pub(crate) fn resolve_scholarly_links(nodes: &[XmlNode]) -> (JatsScholarlyLinks, Vec<Diagnostic>) {
    let by_node_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let labels = nodes
        .iter()
        .filter(|node| is_element(node, "label"))
        .map(|node| {
            let owner = node
                .parent_id
                .as_deref()
                .and_then(|id| by_node_id.get(id).copied());
            JatsLabel {
                node_id: node.id.clone(),
                owner_node_id: owner.map(|node| node.id.clone()),
                owner_xml_id: owner.and_then(xml_id).map(ToOwned::to_owned),
                value: descendant_text(nodes, node),
                locator: node.locator.clone(),
            }
        })
        .collect::<Vec<_>>();

    let mut targets = Vec::new();
    for node in nodes
        .iter()
        .filter(|node| node.kind == XmlNodeKind::Element)
    {
        for id in node.attributes.iter().filter(|attribute| is_id(attribute)) {
            targets.push(JatsTarget {
                node_id: node.id.clone(),
                xml_id: id.value.clone(),
                kind: target_kind(node),
                label: direct_child_text(nodes, node, "label"),
                title: direct_child_text(nodes, node, "title")
                    .or_else(|| direct_child_text(nodes, node, "article-title")),
                locator: id.locator.clone(),
            });
        }
    }

    let mut targets_by_xml_id = BTreeMap::<String, Vec<usize>>::new();
    for (index, target) in targets.iter().enumerate() {
        targets_by_xml_id
            .entry(target.xml_id.clone())
            .or_default()
            .push(index);
    }

    let mut diagnostics = Vec::new();
    for (xml_id, candidates) in targets_by_xml_id
        .iter()
        .filter(|(_, values)| values.len() > 1)
    {
        let affected_ids = candidates
            .iter()
            .map(|index| targets[*index].node_id.clone())
            .collect::<Vec<_>>();
        let first = &targets[candidates[0]];
        diagnostics.push(
            semantic_diagnostic(
                "jats.target.duplicate_id",
                format!(
                    "JATS target ID `{xml_id}` occurs {} times; references remain ambiguous",
                    candidates.len()
                ),
                first.locator.clone(),
                affected_ids,
            )
            .partial(),
        );
    }

    let mut relationships = Vec::new();
    for node in nodes
        .iter()
        .filter(|node| node.kind == XmlNodeKind::Element)
    {
        let ref_type = attribute(node, "ref-type").map(|value| value.value.clone());
        for (rid_index, rid) in node
            .attributes
            .iter()
            .filter(|attribute| is_rid(attribute))
            .enumerate()
        {
            let ids = rid.value.split_whitespace().collect::<Vec<_>>();
            let ids = if ids.is_empty() {
                vec![None]
            } else {
                ids.into_iter().map(Some).collect()
            };
            for (token_index, target_xml_id) in ids.into_iter().enumerate() {
                let candidates = target_xml_id
                    .and_then(|id| targets_by_xml_id.get(id))
                    .cloned()
                    .unwrap_or_default();
                let (resolution, target_node_id, target_kind, candidate_node_ids) =
                    match candidates.as_slice() {
                        [index] => (
                            JatsResolution::Resolved,
                            Some(targets[*index].node_id.clone()),
                            Some(targets[*index].kind.clone()),
                            Vec::new(),
                        ),
                        [] => (
                            JatsResolution::Unresolved,
                            None,
                            expected_target_kind(ref_type.as_deref()),
                            Vec::new(),
                        ),
                        many => (
                            JatsResolution::Ambiguous,
                            None,
                            expected_target_kind(ref_type.as_deref()),
                            many.iter()
                                .map(|index| targets[*index].node_id.clone())
                                .collect(),
                        ),
                    };
                let relationship_kind =
                    relationship_kind(ref_type.as_deref(), target_kind.as_ref());
                let relationship = JatsRelationship {
                    id: format!(
                        "{}/@{}[{}]/idref[{}]",
                        node.id,
                        rid.qualified_name,
                        rid_index + 1,
                        token_index + 1
                    ),
                    source_node_id: node.id.clone(),
                    scope: reference_scope(node, &by_node_id),
                    kind: relationship_kind,
                    ref_type: ref_type.clone(),
                    raw_rid: rid.value.clone(),
                    target_xml_id: target_xml_id.map(ToOwned::to_owned),
                    target_node_id,
                    target_kind,
                    resolution,
                    candidate_node_ids,
                    locator: rid.locator.clone(),
                };
                if relationship.resolution != JatsResolution::Resolved {
                    let (code, message) = if relationship.resolution == JatsResolution::Ambiguous {
                        (
                            "jats.xref.ambiguous",
                            format!(
                                "JATS reference `{}` matches duplicate targets",
                                relationship.target_xml_id.as_deref().unwrap_or("")
                            ),
                        )
                    } else if relationship.target_xml_id.is_none() {
                        (
                            "jats.xref.empty_rid",
                            "JATS reference has an empty `rid`; the source reference was retained"
                                .to_string(),
                        )
                    } else {
                        (
                            "jats.xref.unresolved",
                            format!(
                                "JATS reference `{}` has no matching target; the unresolved reference was retained",
                                relationship.target_xml_id.as_deref().unwrap_or("")
                            ),
                        )
                    };
                    let mut affected = vec![relationship.source_node_id.clone()];
                    affected.extend(relationship.candidate_node_ids.iter().cloned());
                    diagnostics.push(
                        semantic_diagnostic(code, message, relationship.locator.clone(), affected)
                            .partial(),
                    );
                }
                relationships.push(relationship);
            }
        }
    }

    (
        JatsScholarlyLinks {
            schema_version: JATS_SCHOLARLY_LINKS_SCHEMA_VERSION.to_string(),
            targets,
            relationships,
            labels,
        },
        diagnostics,
    )
}

fn semantic_diagnostic(
    code: &str,
    message: String,
    locator: SourceLocator,
    affected_ids: Vec<String>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::warning("grist.jats", code, message)
        .with_module("grist.jats")
        .with_locator(locator)
        .with_affected_ids(affected_ids)
        .with_explanation_key(code);
    diagnostic.class = DiagnosticClass::MalformedInput;
    diagnostic
}

fn is_element(node: &XmlNode, name: &str) -> bool {
    node.kind == XmlNodeKind::Element && node.local_name.as_deref() == Some(name)
}

fn attribute<'a>(node: &'a XmlNode, local_name: &str) -> Option<&'a XmlAttribute> {
    node.attributes
        .iter()
        .find(|attribute| attribute.local_name == local_name)
}

fn is_id(attribute: &XmlAttribute) -> bool {
    attribute.qualified_name == "id"
        || (attribute.prefix.as_deref() == Some("xml") && attribute.local_name == "id")
}

fn is_rid(attribute: &XmlAttribute) -> bool {
    attribute.qualified_name == "rid" || attribute.local_name == "rid"
}

fn xml_id(node: &XmlNode) -> Option<&str> {
    node.attributes
        .iter()
        .find(|attribute| is_id(attribute))
        .map(|attribute| attribute.value.as_str())
}

fn descendant_text(nodes: &[XmlNode], root: &XmlNode) -> String {
    let prefix = format!("{}/", root.xml_path);
    nodes
        .iter()
        .filter(|node| node.xml_path.starts_with(&prefix))
        .filter_map(|node| node.text.as_deref())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn direct_child_text(nodes: &[XmlNode], parent: &XmlNode, name: &str) -> Option<String> {
    nodes
        .iter()
        .find(|node| {
            node.parent_id.as_deref() == Some(parent.id.as_str())
                && node.local_name.as_deref() == Some(name)
        })
        .map(|node| descendant_text(nodes, node))
}

fn target_kind(node: &XmlNode) -> JatsTargetKind {
    match node.local_name.as_deref().unwrap_or("") {
        "ref" | "mixed-citation" | "element-citation" => JatsTargetKind::BibliographyEntry,
        "fig" | "fig-group" => JatsTargetKind::Figure,
        "table" | "table-wrap" | "table-wrap-group" => JatsTargetKind::Table,
        "supplementary-material" | "media" => JatsTargetKind::Supplement,
        "sec" | "abstract" | "ref-list" => JatsTargetKind::Section,
        "fn" | "table-wrap-foot" => JatsTargetKind::Footnote,
        "label" => JatsTargetKind::Label,
        "article" | "front" | "journal-meta" | "article-meta" | "contrib" | "aff" | "corresp"
        | "author-notes" | "award-group" | "funding-group" | "article-id" => {
            JatsTargetKind::Metadata
        }
        other => JatsTargetKind::Other(other.to_string()),
    }
}

fn expected_target_kind(ref_type: Option<&str>) -> Option<JatsTargetKind> {
    match ref_type.unwrap_or("").to_ascii_lowercase().as_str() {
        "bibr" => Some(JatsTargetKind::BibliographyEntry),
        "fig" => Some(JatsTargetKind::Figure),
        "table" => Some(JatsTargetKind::Table),
        "supplement" | "supplementary-material" => Some(JatsTargetKind::Supplement),
        "sec" => Some(JatsTargetKind::Section),
        "fn" | "table-fn" => Some(JatsTargetKind::Footnote),
        "aff" | "author-notes" | "corresp" | "contrib" | "award" => Some(JatsTargetKind::Metadata),
        "" => None,
        other => Some(JatsTargetKind::Other(other.to_string())),
    }
}

fn relationship_kind(
    ref_type: Option<&str>,
    target_kind: Option<&JatsTargetKind>,
) -> JatsRelationshipKind {
    match expected_target_kind(ref_type).as_ref().or(target_kind) {
        Some(JatsTargetKind::BibliographyEntry) => JatsRelationshipKind::Citation,
        Some(JatsTargetKind::Figure) => JatsRelationshipKind::FigureReference,
        Some(JatsTargetKind::Table) => JatsRelationshipKind::TableReference,
        Some(JatsTargetKind::Supplement) => JatsRelationshipKind::SupplementReference,
        Some(JatsTargetKind::Section) => JatsRelationshipKind::SectionReference,
        Some(JatsTargetKind::Footnote) => JatsRelationshipKind::FootnoteReference,
        Some(JatsTargetKind::Metadata) | Some(JatsTargetKind::Label) => {
            JatsRelationshipKind::MetadataReference
        }
        Some(JatsTargetKind::Other(_)) | None => JatsRelationshipKind::OtherReference,
    }
}

fn reference_scope(node: &XmlNode, by_node_id: &HashMap<&str, &XmlNode>) -> JatsReferenceScope {
    let mut current = Some(node);
    while let Some(candidate) = current {
        match candidate.local_name.as_deref() {
            Some("body") => return JatsReferenceScope::Body,
            Some("article-meta" | "journal-meta" | "front") => {
                return JatsReferenceScope::FrontMetadata;
            }
            Some("back" | "ref-list") => return JatsReferenceScope::BackMatter,
            _ => {}
        }
        current = candidate
            .parent_id
            .as_deref()
            .and_then(|id| by_node_id.get(id).copied());
    }
    JatsReferenceScope::Other
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SourceInfo;
    use crate::xml::{XmlOptions, parse_xml};

    #[test]
    fn resolves_multiple_and_retains_unresolved_and_ambiguous_references() {
        let xml = r#"<article><body><p><xref ref-type="bibr" rid="r1 missing r2"/></p></body><back><ref-list><ref id="r1"><label>1</label></ref><ref id="r2"/><ref id="r2"/></ref-list></back></article>"#;
        let envelope = parse_xml(xml, SourceInfo::stdin("links.nxml"), &XmlOptions::default());
        let links = envelope.payload.unwrap().scholarly_links.unwrap();
        assert_eq!(links.relationships.len(), 3);
        assert_eq!(links.relationships[0].resolution, JatsResolution::Resolved);
        assert_eq!(
            links.relationships[1].resolution,
            JatsResolution::Unresolved
        );
        assert_eq!(links.relationships[2].resolution, JatsResolution::Ambiguous);
        assert_eq!(
            links
                .targets
                .iter()
                .filter(|target| target.xml_id == "r2")
                .count(),
            2
        );
        assert_eq!(links.labels[0].value, "1");
    }
}
