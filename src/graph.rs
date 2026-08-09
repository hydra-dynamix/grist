//! Input-side generic graph contracts.
//!
//! [`GraphDocument`] preserves the declarations and generic graph semantics of
//! an input graph. It is intentionally separate from
//! [`crate::document_graph::DocumentGraph`], Grist's normalized cross-format
//! projection.

mod analysis;
mod parse;

pub use analysis::{
    GraphAnalysis, GraphAnalysisError, GraphAnalysisOptions, GraphCycleWitness, GraphIndexes,
    GraphValidationOptions, GraphValidationResult, analyze_graph,
    analyze_graph_with_operation_control, validate_graph, validate_graph_with_operation_control,
};

pub use parse::{
    parse_graph, parse_graph_json, parse_graph_request, parse_graph_with_operation_control,
    parse_graph_yaml, parser_info,
};

use crate::core::{
    Diagnostic, Envelope, FormatOptions, ParseRequest, ResolvedParseRequest, SchemaVersion,
    SourceLocator,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Deterministically ordered application attributes.
pub type GraphAttributeMap = BTreeMap<String, Value>;

/// The encoding selected for a generic graph input.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphInputEncoding {
    Json,
    Yaml,
}

/// Format-specific graph input options.
///
/// Resource limits, cancellation, recovery, security, and providers remain on
/// the shared [`ParseRequest`] boundary. `None` leaves encoding selection to
/// the request's shared format hint and the downstream graph parser.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GraphOptions {
    pub encoding: Option<GraphInputEncoding>,
}

impl FormatOptions for GraphOptions {
    const FORMAT: &'static str = "graph";
}

/// An input-side generic graph, preserving source declaration order.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphDocument {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub directed: bool,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: GraphAttributeMap,
}

impl GraphDocument {
    pub const SCHEMA_VERSION: &'static str = SchemaVersion::GRAPH_DOCUMENT_V1;

    pub fn new(directed: bool) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION.into(),
            id: None,
            directed,
            nodes: Vec::new(),
            edges: Vec::new(),
            attrs: GraphAttributeMap::new(),
        }
    }
}

/// One source node declaration.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: GraphAttributeMap,
}

impl GraphNode {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            labels: Vec::new(),
            attrs: GraphAttributeMap::new(),
        }
    }
}

/// One edge occurrence. Distinct IDs preserve parallel edge occurrences.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    /// Effective direction after the input document default is materialized.
    pub directed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: GraphAttributeMap,
}

impl GraphEdge {
    pub fn new(
        id: impl Into<String>,
        source: impl Into<String>,
        target: impl Into<String>,
        directed: bool,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            target: target.into(),
            directed,
            label: None,
            attrs: GraphAttributeMap::new(),
        }
    }
}

/// Trusted source evidence kept outside the closed canonical v1 graph value.
///
/// Node and edge vectors align by occurrence with `GraphDocument::nodes` and
/// `GraphDocument::edges`. This preserves declaration order and parallel edge
/// occurrences without letting caller-authored attributes masquerade as
/// provenance or alter canonical payload identity.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GraphSourceMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<SourceLocator>,
    pub nodes: Vec<SourceLocator>,
    pub edges: Vec<SourceLocator>,
}

/// Shared operation envelope specialized to the input-side graph payload.
pub type GraphEnvelope = Envelope<GraphDocument>;

/// Parser result retaining canonical graph semantics and source evidence as
/// distinct representations.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphParseResult {
    pub envelope: GraphEnvelope,
    pub source_map: GraphSourceMap,
}

/// Shared typed request specialized to graph options and their explicit budget.
pub type GraphParseRequest = ParseRequest<GraphOptions>;

/// Resolved shared request specialized to graph options.
pub type ResolvedGraphParseRequest = ResolvedParseRequest<GraphOptions>;

/// Graph parser failure represented by the shared diagnostic contract.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct GraphError {
    pub diagnostic: Diagnostic,
}

impl GraphError {
    pub fn new(diagnostic: Diagnostic) -> Self {
        Self { diagnostic }
    }

    pub fn as_diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    pub fn into_diagnostic(self) -> Diagnostic {
        self.diagnostic
    }
}

impl From<Diagnostic> for GraphError {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::new(diagnostic)
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}",
            self.diagnostic.code, self.diagnostic.message
        )
    }
}

impl std::error::Error for GraphError {}

/// Stable graph-specific codes carried by [`Diagnostic::code`].
pub mod diagnostic_codes {
    pub const SCHEMA_VERSION_UNSUPPORTED: &str = "grist.graph.schema_version.unsupported";
    pub const NODE_ID_DUPLICATE: &str = "grist.graph.node.id.duplicate";
    pub const EDGE_ID_DUPLICATE: &str = "grist.graph.edge.id.duplicate";
    pub const EDGE_ENDPOINT_UNKNOWN: &str = "grist.graph.edge.endpoint.unknown";
    pub const FIELD_UNKNOWN: &str = "grist.graph.field.unknown";
    pub const YAML_FEATURE_UNSUPPORTED: &str = "grist.graph.yaml.feature.unsupported";
    pub const CONSTRUCT_UNSUPPORTED: &str = "grist.graph.construct.unsupported";
    pub const ID_EMPTY: &str = "grist.graph.id.empty";
    pub const SELF_LOOP_FORBIDDEN: &str = "grist.graph.edge.self_loop.forbidden";
    pub const PARALLEL_EDGE_FORBIDDEN: &str = "grist.graph.edge.parallel.forbidden";
    pub const UNDIRECTED_EDGE_FORBIDDEN: &str = "grist.graph.edge.undirected.forbidden";
    pub const ATTRIBUTE_DEPTH_EXCEEDED: &str = "grist.graph.attribute.depth.exceeded";
    pub const SOURCE_MAP_MISMATCH: &str = "grist.graph.source_map.mismatch";
    pub const CYCLE: &str = "grist.graph.cycle";
}
