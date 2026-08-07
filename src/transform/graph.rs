//! Deterministic normalized graph operation engine.

use super::{
    GRAPH_TRANSFORM_RESULT_V1, GRAPH_TRANSFORM_SOURCE_MAP_V1, GraphTransformError,
    GraphTransformFidelity, GraphTransformOptions, GraphTransformResult, GraphTransformSourceMap,
    GraphTransformSourceMapEntry, NormalizedGraphOperation, TransformMapStatus,
};
use crate::core::{
    ArtifactKind, CanonicalPayloadIdentity, ContentIdentity, DeclaredLoss, Envelope, OperationKind,
    ParserInfo, ProvenanceStep, SourceInfo, options_digest,
};
use crate::document_graph::{DocumentGraph, DocumentRelation};
use std::collections::{BTreeMap, BTreeSet};

const GRAPH_TRANSFORMER: &str = "grist.transform.graph";

pub fn transform_document_graph(
    input: &DocumentGraph,
    options: &GraphTransformOptions,
) -> Result<Envelope<GraphTransformResult>, GraphTransformError> {
    input
        .validate_contract()
        .map_err(|error| GraphTransformError::InvalidGraph {
            message: error.to_string(),
        })?;
    validate_options(options)?;
    let input_identity = CanonicalPayloadIdentity::new(input.schema_version.as_str(), input)
        .map_err(identity_error)?;
    let mut graph = input.clone();
    for operation in &options.operations {
        match operation {
            NormalizedGraphOperation::Identity => {}
            NormalizedGraphOperation::ExtractConditionalObligations => {
                crate::document_graph::extract_conditional_obligations(&mut graph);
            }
        }
    }
    graph
        .validate_contract()
        .map_err(|error| GraphTransformError::InvalidGraph {
            message: error.to_string(),
        })?;
    let output_identity = CanonicalPayloadIdentity::new(graph.schema_version.as_str(), &graph)
        .map_err(identity_error)?;
    let source_map = build_source_map(
        input,
        &graph,
        &options.operations,
        input_identity.sha256.clone(),
        output_identity.sha256,
    )?;
    let result = GraphTransformResult {
        schema_version: GRAPH_TRANSFORM_RESULT_V1.to_string(),
        graph,
        source_map,
        fidelity: GraphTransformFidelity {
            lossless: true,
            losses: Vec::new(),
        },
    };
    envelope(input, options, input_identity.sha256, result)
}

fn envelope(
    input: &DocumentGraph,
    options: &GraphTransformOptions,
    input_sha256: String,
    result: GraphTransformResult,
) -> Result<Envelope<GraphTransformResult>, GraphTransformError> {
    let digest = options_digest(options).map_err(identity_error)?;
    let output = CanonicalPayloadIdentity::new(GRAPH_TRANSFORM_RESULT_V1, &result)
        .map_err(identity_error)?;
    let provenance = ProvenanceStep::new(
        OperationKind::Transform,
        GRAPH_TRANSFORMER,
        input_sha256,
        output.sha256,
        digest.clone(),
        DeclaredLoss::Lossless,
    )
    .map_err(|error| GraphTransformError::Provenance {
        message: error.to_string(),
    })?;
    let source = input
        .source
        .clone()
        .unwrap_or_else(|| SourceInfo::new(input.id.clone()));
    let identity = ContentIdentity::default()
        .with_canonical_payload(GRAPH_TRANSFORM_RESULT_V1, &result)
        .map_err(identity_error)?;
    Ok(Envelope::complete(
        OperationKind::Transform,
        ArtifactKind::GraphTransformResult,
        source,
        ParserInfo::new(GRAPH_TRANSFORMER).with_feature("document-graph"),
        digest,
        GRAPH_TRANSFORM_RESULT_V1,
        result,
    )
    .with_identity(identity)
    .with_provenance(vec![provenance]))
}

fn validate_options(options: &GraphTransformOptions) -> Result<(), GraphTransformError> {
    if options.operations.is_empty() {
        return Err(GraphTransformError::EmptyPipeline);
    }
    let mut seen = BTreeSet::new();
    for operation in &options.operations {
        if !seen.insert(*operation) {
            return Err(GraphTransformError::DuplicateOperation {
                operation: *operation,
            });
        }
    }
    Ok(())
}

fn build_source_map(
    input: &DocumentGraph,
    output: &DocumentGraph,
    operations: &[NormalizedGraphOperation],
    input_graph_sha256: String,
    output_graph_sha256: String,
) -> Result<GraphTransformSourceMap, GraphTransformError> {
    let input_nodes = input
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let output_ids = output
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    for node_id in input_nodes.keys() {
        if !output_ids.contains(node_id) {
            return Err(GraphTransformError::SilentNodeRemoval {
                node_id: (*node_id).to_string(),
            });
        }
    }
    let entries = output
        .nodes
        .iter()
        .map(|node| map_node(node, output, &input_nodes, operations))
        .collect::<Result<Vec<_>, _>>()?;
    let map = GraphTransformSourceMap {
        schema_version: GRAPH_TRANSFORM_SOURCE_MAP_V1.to_string(),
        input_graph_sha256,
        output_graph_sha256,
        entries,
    };
    validate_source_map(&map, input, output)?;
    Ok(map)
}

fn map_node(
    node: &crate::document_graph::DocumentNode,
    output: &DocumentGraph,
    input_nodes: &BTreeMap<&str, &crate::document_graph::DocumentNode>,
    operations: &[NormalizedGraphOperation],
) -> Result<GraphTransformSourceMapEntry, GraphTransformError> {
    if let Some(original) = input_nodes.get(node.id.as_str()) {
        let status = if *original == node {
            TransformMapStatus::Preserved
        } else {
            TransformMapStatus::Modified
        };
        return Ok(GraphTransformSourceMapEntry {
            output_node_id: node.id.clone(),
            input_node_ids: vec![node.id.clone()],
            status,
            original_locators: original.locator.clone().into_iter().collect(),
            locator_unavailable_reason: original
                .locator
                .is_none()
                .then(|| "input node has no original locator".to_string()),
            derivation_steps: if status == TransformMapStatus::Modified {
                operation_names(operations)
            } else {
                Vec::new()
            },
        });
    }
    derived_entry(node, output, input_nodes, operations)
}

fn derived_entry(
    node: &crate::document_graph::DocumentNode,
    output: &DocumentGraph,
    input_nodes: &BTreeMap<&str, &crate::document_graph::DocumentNode>,
    operations: &[NormalizedGraphOperation],
) -> Result<GraphTransformSourceMapEntry, GraphTransformError> {
    let reference = node
        .locator
        .as_ref()
        .and_then(|locator| locator.precision().derived_from());
    let mut source_ids = reference
        .map(|value| value.source_node_ids.clone())
        .unwrap_or_default();
    if source_ids.is_empty() {
        source_ids.extend(
            output
                .edges
                .iter()
                .filter(|edge| {
                    edge.source == node.id
                        && edge.relation == DocumentRelation::DerivedFrom
                        && input_nodes.contains_key(edge.target.as_str())
                })
                .map(|edge| edge.target.clone()),
        );
    }
    source_ids.sort();
    source_ids.dedup();
    if source_ids.is_empty()
        || source_ids
            .iter()
            .any(|id| !input_nodes.contains_key(id.as_str()))
    {
        return Err(GraphTransformError::UnmappedOutputNode {
            node_id: node.id.clone(),
        });
    }
    let original_locators = source_ids
        .iter()
        .filter_map(|id| input_nodes[id.as_str()].locator.clone())
        .collect::<Vec<_>>();
    let derivation_steps = reference
        .map(|value| value.derivation_steps.clone())
        .unwrap_or_else(|| operation_names(operations));
    Ok(GraphTransformSourceMapEntry {
        output_node_id: node.id.clone(),
        input_node_ids: source_ids,
        status: TransformMapStatus::Derived,
        locator_unavailable_reason: original_locators
            .is_empty()
            .then(|| "source nodes have no original locators".to_string()),
        original_locators,
        derivation_steps,
    })
}

fn validate_source_map(
    map: &GraphTransformSourceMap,
    input: &DocumentGraph,
    output: &DocumentGraph,
) -> Result<(), GraphTransformError> {
    let input_ids = input
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let output_ids = output
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let mapped_ids = map
        .entries
        .iter()
        .map(|entry| entry.output_node_id.as_str())
        .collect::<BTreeSet<_>>();
    if map.schema_version != GRAPH_TRANSFORM_SOURCE_MAP_V1
        || mapped_ids.len() != map.entries.len()
        || mapped_ids != output_ids
    {
        return Err(invalid_map(
            "entries must map every output node exactly once",
        ));
    }
    for entry in &map.entries {
        if entry.input_node_ids.is_empty()
            || entry
                .input_node_ids
                .iter()
                .any(|id| !input_ids.contains(id.as_str()))
        {
            return Err(invalid_map(
                "source-map entry references an unknown input node",
            ));
        }
        let locator_valid = if entry.original_locators.is_empty() {
            entry
                .locator_unavailable_reason
                .as_deref()
                .is_some_and(|reason| !reason.trim().is_empty())
        } else {
            entry.locator_unavailable_reason.is_none()
                && entry
                    .original_locators
                    .iter()
                    .all(|locator| locator.validate().is_ok())
        };
        if !locator_valid {
            return Err(invalid_map(
                "source-map entry has an invalid locator declaration",
            ));
        }
        if entry.status != TransformMapStatus::Preserved && entry.derivation_steps.is_empty() {
            return Err(invalid_map(
                "changed source-map entry does not name its derivation",
            ));
        }
    }
    Ok(())
}

fn operation_names(operations: &[NormalizedGraphOperation]) -> Vec<String> {
    operations
        .iter()
        .filter(|operation| **operation != NormalizedGraphOperation::Identity)
        .map(|operation| match operation {
            NormalizedGraphOperation::Identity => "grist.transform.identity.v1",
            NormalizedGraphOperation::ExtractConditionalObligations => {
                "grist.transform.extract-conditional-obligations.v1"
            }
        })
        .map(str::to_string)
        .collect()
}

fn identity_error(error: impl std::fmt::Display) -> GraphTransformError {
    GraphTransformError::Identity {
        message: error.to_string(),
    }
}

fn invalid_map(message: impl Into<String>) -> GraphTransformError {
    GraphTransformError::InvalidSourceMap {
        message: message.into(),
    }
}
