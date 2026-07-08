//! Normalized document graph projection.
//!
//! `DocumentGraph` is a cross-parser intermediate representation for document
//! structure, code facts, semantic facts, and transform pipelines. It is not a
//! replacement for parser-specific payloads such as `MarkdownDocument`,
//! `PythonFile`, or `TypeScriptFile`; those remain the authoritative detailed
//! parser outputs. The graph is the shared projection layer used by downstream
//! transforms and graph-oriented consumers.

use crate::core::{Diagnostic, SchemaVersion, SourceInfo, SourceRange};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

pub type DocumentGraphEnvelope = crate::core::Envelope<DocumentGraph>;
pub type AttrMap = BTreeMap<String, Value>;

/// A normalized graph projection of a parsed document or code artifact.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentGraph {
    pub schema_version: String,
    pub id: String,
    pub kind: DocumentKind,
    pub source: Option<SourceInfo>,
    pub language: Option<String>,
    pub dialect: Option<String>,
    pub nodes: Vec<DocumentNode>,
    pub edges: Vec<DocumentEdge>,
    pub diagnostics: Vec<Diagnostic>,
    pub attrs: AttrMap,
}

impl DocumentGraph {
    pub fn new(id: impl Into<String>, kind: DocumentKind) -> Self {
        Self {
            schema_version: SchemaVersion::DOCUMENT_GRAPH_V1.to_string(),
            id: id.into(),
            kind,
            source: None,
            language: None,
            dialect: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            diagnostics: Vec::new(),
            attrs: AttrMap::new(),
        }
    }

    pub fn with_source(mut self, source: SourceInfo) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    pub fn with_dialect(mut self, dialect: impl Into<String>) -> Self {
        self.dialect = Some(dialect.into());
        self
    }

    pub fn add_node(&mut self, node: DocumentNode) {
        self.nodes.push(node);
    }

    pub fn add_edge(&mut self, edge: DocumentEdge) {
        self.edges.push(edge);
    }

    pub fn add_contains(&mut self, parent: impl Into<String>, child: impl Into<String>) {
        self.edges
            .push(DocumentEdge::new(parent, DocumentRelation::Contains, child));
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Document,
    Markdown,
    Html,
    Latex,
    Text,
    Csv,
    Serialization,
    ModelOutput,
    Repository,
    Code,
    Python,
    Rust,
    TypeScript,
    LdgrProjection,
    Other(String),
}

/// One normalized node in a `DocumentGraph`.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentNode {
    pub id: String,
    pub kind: DocumentNodeKind,
    pub range: Option<SourceRange>,
    pub text: Option<String>,
    pub name: Option<String>,
    pub qualified_name: Option<String>,
    pub parent: Option<String>,
    pub ordinal: Option<usize>,
    pub attrs: AttrMap,
}

impl DocumentNode {
    pub fn new(id: impl Into<String>, kind: DocumentNodeKind) -> Self {
        Self {
            id: id.into(),
            kind,
            range: None,
            text: None,
            name: None,
            qualified_name: None,
            parent: None,
            ordinal: None,
            attrs: AttrMap::new(),
        }
    }

    pub fn with_range(mut self, range: SourceRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_qualified_name(mut self, qualified_name: impl Into<String>) -> Self {
        self.qualified_name = Some(qualified_name.into());
        self
    }

    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    pub fn with_ordinal(mut self, ordinal: usize) -> Self {
        self.ordinal = Some(ordinal);
        self
    }

    pub fn with_attr(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.attrs.insert(key.into(), value.into());
        self
    }
}

/// Cross-document node vocabulary.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentNodeKind {
    Document,
    Section,
    Heading,
    Paragraph,
    Text,
    Emphasis,
    Strong,
    Link,
    List,
    ListItem,
    Table,
    TableRow,
    TableCell,
    CodeBlock,
    InlineCode,
    MathInline,
    MathBlock,
    Citation,
    Footnote,
    Figure,
    Image,
    Frontmatter,
    RawBlock,
    RawInline,

    Module,
    Namespace,
    Package,
    Symbol,
    Class,
    Function,
    Method,
    Constructor,
    Interface,
    TypeAlias,
    Enum,
    Variable,
    Field,
    Import,
    Export,
    Call,
    Assignment,
    Return,
    Branch,
    Literal,
    Identifier,

    Obligation,
    Condition,
    Requirement,
    Permission,
    Prohibition,
    Claim,
    Evidence,
    Diagnostic,

    Other(String),
}

/// One normalized relation in a `DocumentGraph`.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentEdge {
    pub source: String,
    pub relation: DocumentRelation,
    pub target: String,
    pub range: Option<SourceRange>,
    pub attrs: AttrMap,
}

impl DocumentEdge {
    pub fn new(
        source: impl Into<String>,
        relation: DocumentRelation,
        target: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            relation,
            target: target.into(),
            range: None,
            attrs: AttrMap::new(),
        }
    }

    pub fn with_range(mut self, range: SourceRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn with_attr(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.attrs.insert(key.into(), value.into());
        self
    }
}

/// Cross-document relation vocabulary.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentRelation {
    Contains,
    NextSibling,
    PreviousSibling,
    References,
    Defines,
    LinksTo,
    Cites,
    Annotates,
    DerivedFrom,
    EvidenceFor,

    Imports,
    Exports,
    Calls,
    Inherits,
    Implements,
    Assigns,
    Returns,
    Reads,
    Writes,
    Decorates,

    ConditionalOn,
    Requires,
    Forbids,
    Allows,
    Satisfies,
    Violates,
    Weakens,
    Strengthens,

    Other(String),
}

/// Structured metadata for conditional or unconditional normative statements.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObligationAttrs {
    pub modality: ObligationModality,
    pub polarity: ObligationPolarity,
    pub subject: Option<String>,
    pub predicate: Option<String>,
    pub action: Option<String>,
    pub source_text: Option<String>,
    pub extraction_method: Option<String>,
    pub confidence: Option<f64>,
    pub attrs: AttrMap,
}

impl ObligationAttrs {
    pub fn new(modality: ObligationModality, polarity: ObligationPolarity) -> Self {
        Self {
            modality,
            polarity,
            subject: None,
            predicate: None,
            action: None,
            source_text: None,
            extraction_method: None,
            confidence: None,
            attrs: AttrMap::new(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObligationModality {
    Must,
    Shall,
    Should,
    May,
    MustNot,
    ShallNot,
    ShouldNot,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObligationPolarity {
    Positive,
    Negative,
    Permission,
    Prohibition,
    Unknown,
}

/// Convert a parser-specific payload into a normalized `DocumentGraph`.
pub trait ToDocumentGraph {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError>;
}

/// Render or project a `DocumentGraph` into a target output type.
pub trait FromDocumentGraph: Sized {
    fn from_document_graph(
        graph: &DocumentGraph,
        options: TransformOptions,
    ) -> Result<Self, TransformError>;
}

/// Shared context for source-to-graph projections.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentGraphContext {
    pub graph_id: String,
    pub source: Option<SourceInfo>,
    pub language: Option<String>,
    pub dialect: Option<String>,
    pub attrs: AttrMap,
}

impl DocumentGraphContext {
    pub fn new(graph_id: impl Into<String>) -> Self {
        Self {
            graph_id: graph_id.into(),
            source: None,
            language: None,
            dialect: None,
            attrs: AttrMap::new(),
        }
    }

    pub fn with_source(mut self, source: SourceInfo) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    pub fn with_dialect(mut self, dialect: impl Into<String>) -> Self {
        self.dialect = Some(dialect.into());
        self
    }
}

/// Options controlling graph rendering/projection behavior.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransformOptions {
    pub allow_lossy: bool,
    pub allow_raw_fallback: bool,
    pub fail_on_warning: bool,
}

impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            allow_lossy: false,
            allow_raw_fallback: true,
            fail_on_warning: false,
        }
    }
}

/// One non-fatal transform warning.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransformWarning {
    pub kind: TransformWarningKind,
    pub message: String,
    pub node_id: Option<String>,
    pub edge_source: Option<String>,
    pub edge_target: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransformWarningKind {
    LossyProjection,
    RawFallback,
    UnsupportedAttribute,
    MissingRange,
    Other(String),
}

/// Structured production errors for graph conversions and renderers.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Error)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TransformError {
    #[error("unsupported node kind {node_kind:?} on node {node_id}")]
    UnsupportedNodeKind {
        node_id: String,
        node_kind: DocumentNodeKind,
    },
    #[error("unsupported relation {relation:?} from {edge_source} to {edge_target}")]
    UnsupportedRelation {
        edge_source: String,
        relation: DocumentRelation,
        edge_target: String,
    },
    #[error("missing required attribute {attr} on {target}")]
    MissingRequiredAttribute { target: String, attr: String },
    #[error("invalid graph shape: {message}")]
    InvalidGraphShape { message: String },
    #[error("lossy transform rejected: {message}")]
    LossyTransformRejected { message: String },
    #[error("feature {feature} is required for {operation}")]
    FeatureUnavailable { feature: String, operation: String },
    #[error("transform failed: {message}")]
    Other { message: String },
}

impl TransformError {
    pub fn diagnostic_code(&self) -> &'static str {
        match self {
            TransformError::UnsupportedNodeKind { .. } => "document_graph.unsupported_node_kind",
            TransformError::UnsupportedRelation { .. } => "document_graph.unsupported_relation",
            TransformError::MissingRequiredAttribute { .. } => {
                "document_graph.missing_required_attribute"
            }
            TransformError::InvalidGraphShape { .. } => "document_graph.invalid_graph_shape",
            TransformError::LossyTransformRejected { .. } => "document_graph.lossy_rejected",
            TransformError::FeatureUnavailable { .. } => "document_graph.feature_unavailable",
            TransformError::Other { .. } => "document_graph.transform_error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_contract_supports_prose_code_and_obligations() {
        let mut graph = DocumentGraph::new("doc:test", DocumentKind::Markdown);
        graph.add_node(
            DocumentNode::new("n:heading", DocumentNodeKind::Heading)
                .with_text("Rules")
                .with_ordinal(0),
        );
        graph.add_node(
            DocumentNode::new("n:function", DocumentNodeKind::Function)
                .with_name("parse")
                .with_qualified_name("crate::parse"),
        );
        graph.add_node(DocumentNode::new(
            "n:obligation",
            DocumentNodeKind::Obligation,
        ));
        graph.add_edge(DocumentEdge::new(
            "n:obligation",
            DocumentRelation::ConditionalOn,
            "n:heading",
        ));
        graph.add_edge(DocumentEdge::new(
            "n:function",
            DocumentRelation::DerivedFrom,
            "n:heading",
        ));

        assert_eq!(graph.schema_version, SchemaVersion::DOCUMENT_GRAPH_V1);
        assert_eq!(graph.nodes.len(), 3);
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::ConditionalOn)
        );
    }

    #[test]
    fn obligation_attrs_preserve_normative_metadata() {
        let attrs = ObligationAttrs {
            modality: ObligationModality::Must,
            polarity: ObligationPolarity::Positive,
            subject: Some("file".to_string()),
            predicate: Some("has_shebang".to_string()),
            action: None,
            source_text: Some("If executable, the file must have a shebang.".to_string()),
            extraction_method: Some("rule".to_string()),
            confidence: Some(1.0),
            attrs: AttrMap::new(),
        };

        assert_eq!(attrs.modality, ObligationModality::Must);
        assert_eq!(attrs.polarity, ObligationPolarity::Positive);
        assert_eq!(attrs.subject.as_deref(), Some("file"));
    }

    #[test]
    fn transform_error_maps_to_stable_diagnostic_codes() {
        let err = TransformError::MissingRequiredAttribute {
            target: "n:link".to_string(),
            attr: "destination".to_string(),
        };
        assert_eq!(
            err.diagnostic_code(),
            "document_graph.missing_required_attribute"
        );
        assert!(err.to_string().contains("destination"));
    }

    struct TinyDoc;

    impl ToDocumentGraph for TinyDoc {
        fn to_document_graph(
            &self,
            context: DocumentGraphContext,
        ) -> Result<DocumentGraph, TransformError> {
            let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Document);
            graph.source = context.source;
            graph.language = context.language;
            graph.dialect = context.dialect;
            graph.attrs = context.attrs;
            graph.add_node(DocumentNode::new("n:root", DocumentNodeKind::Document));
            Ok(graph)
        }
    }

    #[test]
    fn transform_traits_project_to_graph_with_context() {
        let graph = TinyDoc
            .to_document_graph(DocumentGraphContext::new("graph:tiny").with_language("text"))
            .expect("projection should succeed");
        assert_eq!(graph.id, "graph:tiny");
        assert_eq!(graph.language.as_deref(), Some("text"));
        assert_eq!(graph.nodes.len(), 1);
    }
}
