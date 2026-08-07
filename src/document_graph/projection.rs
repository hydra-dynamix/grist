//! Canonical projection finalization and order-independent fragment merging.

use crate::core::{Diagnostic, LocationComponent, SourceLocator, canonical_json_bytes, sha256_hex};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use super::{
    DocumentEdge, DocumentGraph, DocumentNode, GraphIdGenerator, GraphIdentityError,
    ProjectionAddress, RelationEvidence,
};

const FALLBACK_EDGE_PREFIX: &str = "grist:edge:fallback:";

/// Independently produced graph data, for example one parsed page or sheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocumentGraphFragment {
    pub nodes: Vec<DocumentNode>,
    pub edges: Vec<DocumentEdge>,
    pub diagnostics: Vec<Diagnostic>,
}

impl DocumentGraphFragment {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_node(mut self, node: DocumentNode) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn with_edge(mut self, edge: DocumentEdge) -> Self {
        self.edges.push(edge);
        self
    }

    pub fn with_diagnostic(mut self, diagnostic: Diagnostic) -> Self {
        self.diagnostics.push(diagnostic);
        self
    }
}

pub(crate) fn finalize_projection(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
) -> Result<(), GraphProjectionError> {
    for edge in &mut graph.edges {
        if !edge.id.is_empty() && !edge.id.starts_with(FALLBACK_EDGE_PREFIX) {
            continue;
        }
        let relation = relation_name(edge);
        let locator = edge_locator(edge).cloned();
        let address = ProjectionAddress {
            structural_path: vec![
                "edges".to_string(),
                edge.source.clone(),
                relation,
                edge.target.clone(),
            ],
            native_id: None,
            locator,
        };
        edge.id = identities
            .edge_id(&address, &edge.source, &edge.relation, &edge.target)
            .map_err(GraphProjectionError::Identity)?;
    }
    canonicalize(graph)
}

pub(crate) fn merge_parallel<I>(
    graph: &mut DocumentGraph,
    fragments: I,
) -> Result<(), GraphProjectionError>
where
    I: IntoIterator<Item = DocumentGraphFragment>,
{
    for fragment in fragments {
        graph.nodes.extend(fragment.nodes);
        graph
            .edges
            .extend(fragment.edges.into_iter().map(|mut edge| {
                ensure_fallback_edge_id(&mut edge);
                edge
            }));
        graph.diagnostics.extend(fragment.diagnostics);
    }
    canonicalize(graph)
}

pub(crate) fn canonicalize(graph: &mut DocumentGraph) -> Result<(), GraphProjectionError> {
    validate_unique_ids(graph)?;
    let order = node_order_keys(&graph.nodes);
    graph.nodes.sort_by(|left, right| {
        order
            .get(&left.id)
            .cmp(&order.get(&right.id))
            .then_with(|| left.id.cmp(&right.id))
    });
    let ranks = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(rank, node)| (node.id.clone(), rank))
        .collect::<BTreeMap<_, _>>();
    graph.edges.sort_by(|left, right| {
        ranks
            .get(&left.source)
            .cmp(&ranks.get(&right.source))
            .then_with(|| relation_name(left).cmp(&relation_name(right)))
            .then_with(|| ranks.get(&left.target).cmp(&ranks.get(&right.target)))
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| canonical_bytes(&left.evidence).cmp(&canonical_bytes(&right.evidence)))
            .then_with(|| left.id.cmp(&right.id))
    });
    graph.diagnostics.sort_by_cached_key(canonical_bytes);
    Ok(())
}

pub(crate) fn validate_unique_ids(graph: &DocumentGraph) -> Result<(), GraphProjectionError> {
    let mut node_ids = BTreeSet::new();
    for node in &graph.nodes {
        if node.id.is_empty() {
            return Err(GraphProjectionError::EmptyNodeId);
        }
        if !node_ids.insert(&node.id) {
            return Err(GraphProjectionError::DuplicateNodeId(node.id.clone()));
        }
    }
    let mut edge_ids = BTreeSet::new();
    for edge in &graph.edges {
        if edge.id.is_empty() {
            return Err(GraphProjectionError::EmptyEdgeId);
        }
        if !edge_ids.insert(&edge.id) {
            return Err(GraphProjectionError::DuplicateEdgeId(edge.id.clone()));
        }
    }
    Ok(())
}

pub(crate) fn ensure_fallback_edge_id(edge: &mut DocumentEdge) {
    if !edge.id.is_empty() {
        return;
    }
    #[derive(Serialize)]
    struct FallbackEdge<'a> {
        identity_version: &'static str,
        source: &'a str,
        relation: &'a super::DocumentRelation,
        target: &'a str,
        evidence: &'a RelationEvidence,
        attrs: &'a super::AttrMap,
        extensions: &'a super::ExtensionMap,
    }
    let material = FallbackEdge {
        identity_version: "grist/document-graph-fallback-edge/v1",
        source: &edge.source,
        relation: &edge.relation,
        target: &edge.target,
        evidence: &edge.evidence,
        attrs: &edge.attrs,
        extensions: &edge.extensions,
    };
    let bytes = canonical_json_bytes(&material).unwrap_or_default();
    let digest = sha256_hex(&bytes);
    edge.id = format!(
        "{FALLBACK_EDGE_PREFIX}{}",
        digest.strip_prefix("sha256:").unwrap_or(&digest)
    );
}

type NodeOrderKey = Vec<(usize, Vec<u8>, String)>;

fn node_order_keys(nodes: &[DocumentNode]) -> BTreeMap<String, NodeOrderKey> {
    let nodes = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    nodes
        .values()
        .map(|node| (node.id.clone(), node_order_key(node, &nodes)))
        .collect()
}

fn node_order_key(node: &DocumentNode, nodes: &BTreeMap<&str, &DocumentNode>) -> NodeOrderKey {
    let mut chain = Vec::new();
    let mut current = Some(node);
    let mut seen = BTreeSet::new();
    while let Some(item) = current {
        if !seen.insert(item.id.as_str()) {
            break;
        }
        chain.push((
            item.ordinal.unwrap_or(usize::MAX),
            item.locator
                .as_ref()
                .map(source_order_bytes)
                .unwrap_or_default(),
            item.id.clone(),
        ));
        current = item
            .parent
            .as_deref()
            .and_then(|parent| nodes.get(parent).copied());
    }
    chain.reverse();
    chain
}

fn source_order_bytes(locator: &SourceLocator) -> Vec<u8> {
    let mut output = Vec::new();
    for component in locator.components() {
        match component {
            LocationComponent::TextRange {
                byte_start,
                byte_end,
                ..
            } => {
                output.push(0);
                push_u64(&mut output, *byte_start as u64);
                push_u64(&mut output, *byte_end as u64);
            }
            LocationComponent::PdfRegion {
                page,
                bbox,
                rotation_degrees,
                tokens,
            } => {
                output.push(1);
                push_u64(&mut output, page.value);
                if let Some(bbox) = bbox {
                    push_f64(&mut output, bbox.y);
                    push_f64(&mut output, bbox.x);
                    push_f64(&mut output, bbox.height);
                    push_f64(&mut output, bbox.width);
                }
                push_option_i16(&mut output, *rotation_degrees);
                if let Some(tokens) = tokens {
                    push_u64(&mut output, tokens.start);
                    push_u64(&mut output, tokens.end);
                }
            }
            LocationComponent::OoxmlPart {
                part,
                paragraph,
                run,
                table,
                row,
                column,
                object_id,
            } => {
                output.push(2);
                push_string(&mut output, part);
                for position in [paragraph, run, table, row, column] {
                    push_option_u64(&mut output, position.as_ref().map(|value| value.value));
                }
                push_option_string(&mut output, object_id.as_deref());
            }
            LocationComponent::SlideRegion {
                slide,
                shape_id,
                bbox,
            } => {
                output.push(3);
                push_u64(&mut output, slide.value);
                push_option_string(&mut output, shape_id.as_deref());
                if let Some(bbox) = bbox {
                    push_f64(&mut output, bbox.y);
                    push_f64(&mut output, bbox.x);
                }
            }
            LocationComponent::SheetRange {
                sheet,
                start_cell,
                end_cell,
            } => {
                output.push(4);
                push_string(&mut output, sheet);
                for value in [
                    start_cell.row,
                    start_cell.column,
                    end_cell.row,
                    end_cell.column,
                ] {
                    push_u64(&mut output, value);
                }
            }
            LocationComponent::NotebookCell {
                index,
                cell_id,
                output_index,
            } => {
                output.push(5);
                push_u64(&mut output, index.value);
                push_option_string(&mut output, cell_id.as_deref());
                push_option_u64(&mut output, output_index.as_ref().map(|value| value.value));
            }
            LocationComponent::EmailPart {
                message_id,
                mime_path,
                header,
            } => {
                output.push(6);
                push_option_string(&mut output, message_id.as_deref());
                for position in mime_path {
                    push_u64(&mut output, position.value);
                }
                push_option_string(&mut output, header.as_deref());
            }
            LocationComponent::ArchiveMember {
                member_path,
                member_index,
            } => {
                output.push(7);
                push_u64(&mut output, member_index.value);
                push_string(&mut output, member_path);
            }
            LocationComponent::ImageRegion { frame, bbox } => {
                output.push(8);
                push_u64(&mut output, frame.value);
                if let Some(bbox) = bbox {
                    push_f64(&mut output, bbox.y);
                    push_f64(&mut output, bbox.x);
                }
            }
            LocationComponent::MediaTime {
                start_ms,
                end_ms,
                track,
            } => {
                output.push(9);
                push_option_u64(&mut output, track.as_ref().map(|value| value.value));
                push_u64(&mut output, *start_ms);
                push_u64(&mut output, *end_ms);
            }
            LocationComponent::RecordRange {
                collection,
                records,
                field,
            } => {
                output.push(10);
                push_string(&mut output, collection);
                push_u64(&mut output, records.start);
                push_u64(&mut output, records.end);
                push_option_string(&mut output, field.as_deref());
            }
            LocationComponent::JsonPointer { pointer } => {
                output.push(11);
                push_string(&mut output, pointer);
            }
            LocationComponent::XmlPath { path } => {
                output.push(12);
                push_string(&mut output, path);
            }
        }
    }
    output.extend(canonical_bytes(locator));
    output
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend(value.to_be_bytes());
}

fn push_f64(output: &mut Vec<u8>, value: f64) {
    let bits = value.to_bits();
    let sortable = if bits >> 63 == 0 {
        bits ^ (1_u64 << 63)
    } else {
        !bits
    };
    push_u64(output, sortable);
}

fn push_string(output: &mut Vec<u8>, value: &str) {
    for byte in value.bytes() {
        if byte == 0 {
            output.extend([0, u8::MAX]);
        } else {
            output.push(byte);
        }
    }
    output.extend([0, 0]);
}

fn push_option_u64(output: &mut Vec<u8>, value: Option<u64>) {
    output.push(u8::from(value.is_some()));
    if let Some(value) = value {
        push_u64(output, value);
    }
}

fn push_option_i16(output: &mut Vec<u8>, value: Option<i16>) {
    output.push(u8::from(value.is_some()));
    if let Some(value) = value {
        output.extend(value.to_be_bytes());
    }
}

fn push_option_string(output: &mut Vec<u8>, value: Option<&str>) {
    output.push(u8::from(value.is_some()));
    if let Some(value) = value {
        push_string(output, value);
    }
}

fn edge_locator(edge: &DocumentEdge) -> Option<&SourceLocator> {
    match &edge.evidence {
        RelationEvidence::Explicit { locator } => Some(locator),
        RelationEvidence::Inferred { inference } => inference.evidence_locators.first(),
    }
}

fn relation_name(edge: &DocumentEdge) -> String {
    serde_json::to_value(&edge.relation)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| canonical_bytes(&edge.relation).escape_ascii().to_string())
}

fn canonical_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    canonical_json_bytes(value).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphProjectionError {
    Identity(GraphIdentityError),
    EmptyNodeId,
    EmptyEdgeId,
    DuplicateNodeId(String),
    DuplicateEdgeId(String),
}

impl std::fmt::Display for GraphProjectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identity(error) => error.fmt(formatter),
            Self::EmptyNodeId => formatter.write_str("graph contains an empty node ID"),
            Self::EmptyEdgeId => formatter.write_str("graph contains an empty edge ID"),
            Self::DuplicateNodeId(id) => {
                formatter.write_str("duplicate graph node ID: ")?;
                formatter.write_str(id)
            }
            Self::DuplicateEdgeId(id) => {
                formatter.write_str("duplicate graph edge ID: ")?;
                formatter.write_str(id)
            }
        }
    }
}

impl std::error::Error for GraphProjectionError {}
