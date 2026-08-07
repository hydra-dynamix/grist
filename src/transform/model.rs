//! Public contracts for normalized graph-to-graph operations.

use crate::core::{SchemaVersion, SourceLocator};
use crate::document_graph::DocumentGraph;
#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const GRAPH_TRANSFORM_RESULT_V1: &str = SchemaVersion::GRAPH_TRANSFORM_RESULT_V1;
pub const GRAPH_TRANSFORM_SOURCE_MAP_V1: &str = SchemaVersion::GRAPH_TRANSFORM_SOURCE_MAP_V1;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NormalizedGraphOperation {
    Identity,
    ExtractConditionalObligations,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphTransformOptions {
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub operations: Vec<NormalizedGraphOperation>,
}

impl Default for GraphTransformOptions {
    fn default() -> Self {
        Self {
            operations: vec![NormalizedGraphOperation::Identity],
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransformMapStatus {
    Preserved,
    Modified,
    Derived,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphTransformSourceMapEntry {
    pub output_node_id: String,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub input_node_ids: Vec<String>,
    pub status: TransformMapStatus,
    #[serde(default)]
    pub original_locators: Vec<SourceLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator_unavailable_reason: Option<String>,
    #[serde(default)]
    pub derivation_steps: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphTransformSourceMap {
    pub schema_version: String,
    pub input_graph_sha256: String,
    pub output_graph_sha256: String,
    pub entries: Vec<GraphTransformSourceMapEntry>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphTransformLossKind {
    NodeRemoved,
    ContentChanged,
    LocatorPrecisionReduced,
    Other,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphTransformLoss {
    pub kind: GraphTransformLossKind,
    pub message: String,
    #[serde(default)]
    pub input_node_ids: Vec<String>,
    #[serde(default)]
    pub output_node_ids: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphTransformFidelity {
    pub lossless: bool,
    #[serde(default)]
    pub losses: Vec<GraphTransformLoss>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphTransformResult {
    pub schema_version: String,
    pub graph: DocumentGraph,
    pub source_map: GraphTransformSourceMap,
    pub fidelity: GraphTransformFidelity,
}

pub type GraphTransformEnvelope = crate::core::Envelope<GraphTransformResult>;

#[derive(Debug, Error)]
pub enum GraphTransformError {
    #[error("invalid input graph: {message}")]
    InvalidGraph { message: String },
    #[error("graph transform pipeline must contain at least one operation")]
    EmptyPipeline,
    #[error("graph transform pipeline repeats operation {operation:?}")]
    DuplicateOperation { operation: NormalizedGraphOperation },
    #[error("output node {node_id} cannot be attributed to any input node")]
    UnmappedOutputNode { node_id: String },
    #[error("input node {node_id} was removed without a declared loss")]
    SilentNodeRemoval { node_id: String },
    #[error("graph transform source map is invalid: {message}")]
    InvalidSourceMap { message: String },
    #[error("canonical graph identity failed: {message}")]
    Identity { message: String },
    #[error("graph transform provenance failed: {message}")]
    Provenance { message: String },
}
