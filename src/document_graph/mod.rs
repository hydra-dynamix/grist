//! Normalized document graph projection.
//!
//! `DocumentGraph` is a cross-parser intermediate representation for document
//! structure, code facts, semantic facts, and transform pipelines. It is not a
//! replacement for parser-specific payloads such as `MarkdownDocument`,
//! `PythonFile`, or `TypeScriptFile`; those remain the authoritative detailed
//! parser outputs. The graph is the shared projection layer used by downstream
//! transforms and graph-oriented consumers.

use crate::core::{
    CitationAnchor, CitationAnchorError, CitationAnchorOptions, CitationCandidate,
    CitationSourceVersion, CitationTargetKind, ContentIdentity, DerivedNodeReference, Diagnostic,
    LocatorConfidence, LocatorPrecision, ParserInfo, SchemaVersion, SourceInfo, SourceLocator,
    SourceRange, canonical_json_bytes, sha256_hex,
};
#[cfg(any(feature = "markdown", feature = "latex"))]
use crate::security::sanitize_link_destination;
#[cfg(feature = "markdown")]
use crate::security::{escape_active_html, inert_markdown_code};
#[cfg(feature = "latex")]
use crate::security::{escape_latex_text, inert_latex_literal};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

mod identity;
mod projection;

pub use identity::{
    GRAPH_IDENTITY_VERSION, GraphIdGenerator, GraphIdStability, GraphIdentityError,
    ProjectionAddress,
};
pub use projection::{DocumentGraphFragment, GraphProjectionError};

pub type DocumentGraphEnvelope = crate::core::Envelope<DocumentGraph>;
pub type AttrMap = BTreeMap<String, Value>;
/// Namespaced parser-format extensions, keyed by names such as `grist.markdown`.
pub type ExtensionMap = BTreeMap<String, Value>;

const LEGACY_EDGE_INFERENCE_RULE: &str = "grist.document_graph.v1-edge-migration";
const UNSPECIFIED_EDGE_INFERENCE_RULE: &str = "grist.document_graph.caller-unspecified.v1";
const STRUCTURAL_INFERENCE_RULE: &str = "grist.document_graph.structural-containment.v1";

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
    /// Parser-format extensions. Common normalized attributes remain in `attrs`.
    #[serde(default)]
    pub extensions: ExtensionMap,
    /// Describes the authoritative typed payload from which this projection was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<ProjectionMetadata>,
}

impl DocumentGraph {
    pub fn new(id: impl Into<String>, kind: DocumentKind) -> Self {
        Self {
            schema_version: SchemaVersion::DOCUMENT_GRAPH_V2.to_string(),
            id: id.into(),
            kind,
            source: None,
            language: None,
            dialect: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            diagnostics: Vec::new(),
            attrs: AttrMap::new(),
            extensions: ExtensionMap::new(),
            projection: None,
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

    pub fn add_node(&mut self, mut node: DocumentNode) {
        node.hydrate_v2(self.kind.extension_namespace());
        self.nodes.push(node);
    }

    pub fn add_edge(&mut self, mut edge: DocumentEdge) {
        edge.retain_namespaced_attrs(self.kind.extension_namespace());
        projection::ensure_fallback_edge_id(&mut edge);
        self.edges.push(edge);
    }

    pub fn add_contains(&mut self, parent: impl Into<String>, child: impl Into<String>) {
        let parent = parent.into();
        let child = child.into();
        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == child)
            && node.parent.is_none()
        {
            node.parent = Some(parent.clone());
        }
        let edge = self
            .nodes
            .iter()
            .find(|node| node.id == child)
            .and_then(|node| node.locator.clone())
            .map(|locator| {
                DocumentEdge::explicit(
                    parent.clone(),
                    DocumentRelation::Contains,
                    child.clone(),
                    locator,
                )
            })
            .unwrap_or_else(|| {
                DocumentEdge::inferred(
                    parent,
                    DocumentRelation::Contains,
                    child,
                    STRUCTURAL_INFERENCE_RULE,
                    confidence_one(),
                )
            });
        self.add_edge(edge);
    }

    /// Mark this graph as a projection of an authoritative parser-specific payload.
    pub fn with_projection(
        mut self,
        payload_kind: impl Into<String>,
        payload_schema_version: impl Into<String>,
        rule: impl Into<String>,
    ) -> Self {
        self.projection = Some(ProjectionMetadata {
            authoritative_payload_kind: payload_kind.into(),
            authoritative_payload_schema_version: payload_schema_version.into(),
            projection_rule: rule.into(),
        });
        self
    }

    /// Upgrade a graph decoded from the v1 wire shape without discarding fields.
    pub fn migrate_to_v2(mut self) -> Result<Self, DocumentGraphContractError> {
        match self.schema_version.as_str() {
            SchemaVersion::DOCUMENT_GRAPH_V1 | SchemaVersion::DOCUMENT_GRAPH_V2 => {}
            version => {
                return Err(DocumentGraphContractError::UnsupportedSchemaVersion(
                    version.to_string(),
                ));
            }
        }
        let namespace = self.kind.extension_namespace();
        for node in &mut self.nodes {
            node.hydrate_v2(namespace);
        }
        for edge in &mut self.edges {
            edge.hydrate_v2(namespace);
            projection::ensure_fallback_edge_id(edge);
        }
        if !self.attrs.is_empty() && !self.extensions.contains_key(namespace) {
            self.extensions.insert(
                namespace.to_string(),
                Value::Object(attrs_object(&self.attrs)),
            );
        }
        self.schema_version = SchemaVersion::DOCUMENT_GRAPH_V2.to_string();
        self.validate_contract()?;
        Ok(self)
    }

    /// Validate invariants that JSON Schema cannot express across tagged fields.
    pub fn validate_contract(&self) -> Result<(), DocumentGraphContractError> {
        if self.schema_version != SchemaVersion::DOCUMENT_GRAPH_V2 {
            return Err(DocumentGraphContractError::UnsupportedSchemaVersion(
                self.schema_version.clone(),
            ));
        }
        validate_extensions("graph", &self.extensions)?;
        for node in &self.nodes {
            validate_extensions(&format!("node {}", node.id), &node.extensions)?;
            if node.kind.retains_raw_content() && node.raw.is_none() {
                return Err(DocumentGraphContractError::MissingRawContent(
                    node.id.clone(),
                ));
            }
            if let Some(raw) = &node.raw {
                validate_namespace(&raw.namespace).map_err(|_| {
                    DocumentGraphContractError::InvalidExtensionNamespace {
                        target: format!("node {} raw content", node.id),
                        namespace: raw.namespace.clone(),
                    }
                })?;
            }
            if let Some(locator) = &node.locator {
                locator
                    .validate()
                    .map_err(|error| DocumentGraphContractError::InvalidLocator {
                        target: format!("node {}", node.id),
                        message: error.to_string(),
                    })?;
            }
        }
        for edge in &self.edges {
            validate_extensions(
                &format!("edge {} -> {}", edge.source, edge.target),
                &edge.extensions,
            )?;
            edge.evidence.validate(&edge.source, &edge.target)?;
        }
        projection::validate_unique_ids(self).map_err(|error| {
            DocumentGraphContractError::InvalidStableIdentity(error.to_string())
        })?;
        Ok(())
    }

    /// Assign source-scoped edge identities and sort every graph collection.
    pub fn finalize_projection(
        &mut self,
        identities: &GraphIdGenerator,
    ) -> Result<(), GraphProjectionError> {
        projection::finalize_projection(self, identities)
    }

    /// Merge independently produced fragments and restore canonical order.
    pub fn merge_parallel<I>(&mut self, fragments: I) -> Result<(), GraphProjectionError>
    where
        I: IntoIterator<Item = DocumentGraphFragment>,
    {
        projection::merge_parallel(self, fragments)
    }

    /// Sort nodes, edges, and diagnostics into the public canonical order.
    pub fn canonicalize(&mut self) -> Result<(), GraphProjectionError> {
        projection::canonicalize(self)
    }

    /// Create an immutable citation for one locator-addressable node.
    pub fn citation_anchor_for_node(
        &self,
        source_identity: &ContentIdentity,
        node_id: &str,
        options: CitationAnchorOptions,
    ) -> Result<CitationAnchor, DocumentGraphCitationError> {
        let source = self
            .source
            .clone()
            .ok_or(DocumentGraphCitationError::MissingSource)?;
        let node = self
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| DocumentGraphCitationError::UnknownNode(node_id.to_string()))?;
        let locator = node
            .locator
            .clone()
            .ok_or_else(|| DocumentGraphCitationError::UnaddressableNode(node.id.clone()))?;
        CitationAnchor::for_node(
            source,
            source_identity.clone(),
            node.id.clone(),
            locator,
            node.citation_text(),
            node.citation_label(),
            options,
        )
        .map_err(DocumentGraphCitationError::Anchor)
    }

    /// Emit anchors for every node carrying a source locator, in canonical graph order.
    pub fn citation_anchors(
        &self,
        source_identity: &ContentIdentity,
        options: CitationAnchorOptions,
    ) -> Result<Vec<CitationAnchor>, DocumentGraphCitationError> {
        self.nodes
            .iter()
            .filter(|node| node.locator.is_some())
            .map(|node| self.citation_anchor_for_node(source_identity, &node.id, options))
            .collect()
    }

    /// Build the bounded-verifier index for every locator-addressable node.
    pub fn citation_source_version(
        &self,
        source_identity: ContentIdentity,
    ) -> Result<CitationSourceVersion, DocumentGraphCitationError> {
        let candidates = self
            .nodes
            .iter()
            .filter_map(|node| {
                node.locator.as_ref().map(|locator| {
                    CitationCandidate::new(
                        CitationTargetKind::Node,
                        node.id.clone(),
                        vec![node.id.clone()],
                        vec![locator.clone()],
                        node.citation_text(),
                    )
                    .map_err(DocumentGraphCitationError::Anchor)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CitationSourceVersion::new(source_identity, candidates))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentGraphCitationError {
    #[error("document graph has no source metadata")]
    MissingSource,
    #[error("document graph has no requested node: {0}")]
    UnknownNode(String),
    #[error("document graph node has no source locator: {0}")]
    UnaddressableNode(String),
    #[error(transparent)]
    Anchor(#[from] CitationAnchorError),
}

/// Identifies the detailed typed payload that remains authoritative over a graph projection.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectionMetadata {
    pub authoritative_payload_kind: String,
    pub authoritative_payload_schema_version: String,
    pub projection_rule: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Document,
    Markdown,
    RestructuredText,
    AsciiDoc,
    Html,
    Epub,
    Pdf,
    WordOoxml,
    PresentationOoxml,
    SpreadsheetOoxml,
    SpreadsheetOdf,
    PresentationOdf,
    OdfWord,
    Rtf,
    Xml,
    Latex,
    Bibliography,
    Email,
    Mbox,
    Notebook,
    Text,
    Csv,
    Serialization,
    StructuredBinary,
    ModelOutput,
    Repository,
    Code,
    Python,
    Rust,
    JavaScript,
    TypeScript,
    LdgrProjection,
    Other(String),
}

impl DocumentKind {
    fn extension_namespace(&self) -> &'static str {
        match self {
            Self::Markdown => "grist.markdown",
            Self::RestructuredText => "grist.restructured_text",
            Self::AsciiDoc => "grist.asciidoc",
            Self::Html => "grist.html",
            Self::Epub => "grist.epub",
            Self::Pdf => "grist.pdf",
            Self::WordOoxml => "grist.word_ooxml",
            Self::PresentationOoxml => "grist.presentation_ooxml",
            Self::SpreadsheetOoxml => "grist.spreadsheet_ooxml",
            Self::SpreadsheetOdf => "grist.spreadsheet_odf",
            Self::PresentationOdf => "grist.presentation_odf",
            Self::OdfWord => "grist.odf_word",
            Self::Rtf => "grist.rtf",
            Self::Xml => "grist.xml",
            Self::Latex => "grist.latex",
            Self::Bibliography => "grist.bibliography",
            Self::Email => "grist.email",
            Self::Mbox => "grist.mbox",
            Self::Notebook => "grist.ipynb",
            Self::Text => "grist.text",
            Self::Csv => "grist.csv",
            Self::Serialization => "grist.serialization",
            Self::StructuredBinary => "grist.structured_binary",
            Self::ModelOutput => "grist.model_output",
            Self::Repository => "grist.repository",
            Self::Python => "grist.python",
            Self::Rust => "grist.rust",
            Self::JavaScript => "grist.javascript",
            Self::TypeScript => "grist.typescript",
            Self::LdgrProjection => "grist.ldgr_projection",
            Self::Document | Self::Code | Self::Other(_) => "grist.document_graph",
        }
    }
}

/// One normalized node in a `DocumentGraph`.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentNode {
    pub id: String,
    pub kind: DocumentNodeKind,
    pub range: Option<SourceRange>,
    /// Cross-format source locator. `range` is retained for v1 text compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
    pub text: Option<String>,
    pub name: Option<String>,
    pub qualified_name: Option<String>,
    pub parent: Option<String>,
    pub ordinal: Option<usize>,
    pub attrs: AttrMap,
    /// Namespaced parser-format attributes copied from the authoritative payload.
    #[serde(default)]
    pub extensions: ExtensionMap,
    /// Lossless raw representation for raw or unknown constructs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<RawNodeContent>,
}

impl DocumentNode {
    pub fn new(id: impl Into<String>, kind: DocumentNodeKind) -> Self {
        Self {
            id: id.into(),
            kind,
            range: None,
            locator: None,
            text: None,
            name: None,
            qualified_name: None,
            parent: None,
            ordinal: None,
            attrs: AttrMap::new(),
            extensions: ExtensionMap::new(),
            raw: None,
        }
    }

    pub fn with_range(mut self, range: SourceRange) -> Self {
        self.locator = SourceLocator::try_from(range.clone()).ok();
        self.range = Some(range);
        self
    }

    pub fn with_locator(mut self, locator: SourceLocator) -> Self {
        self.locator = Some(locator);
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

    fn citation_text(&self) -> &str {
        self.text
            .as_deref()
            .or(self.name.as_deref())
            .or(self.qualified_name.as_deref())
            .unwrap_or("")
    }

    fn citation_label(&self) -> Option<String> {
        if matches!(
            self.kind,
            DocumentNodeKind::Section | DocumentNodeKind::Heading
        ) {
            let title = self
                .name
                .as_deref()
                .or(self.text.as_deref())
                .filter(|value| !value.trim().is_empty())?;
            Some(format!("§ {title}"))
        } else {
            None
        }
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

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_extension(
        mut self,
        namespace: impl Into<String>,
        value: impl Into<Value>,
    ) -> Result<Self, DocumentGraphContractError> {
        let namespace = namespace.into();
        validate_namespace(&namespace).map_err(|_| {
            DocumentGraphContractError::InvalidExtensionNamespace {
                target: format!("node {}", self.id),
                namespace: namespace.clone(),
            }
        })?;
        self.extensions.insert(namespace, value.into());
        Ok(self)
    }

    pub fn with_raw(mut self, raw: RawNodeContent) -> Self {
        self.raw = Some(raw);
        self
    }

    fn hydrate_v2(&mut self, namespace: &str) {
        if self.locator.is_none() {
            self.locator = self
                .range
                .clone()
                .and_then(|range| SourceLocator::try_from(range).ok());
        }
        if !self.attrs.is_empty() && !self.extensions.contains_key(namespace) {
            self.extensions.insert(
                namespace.to_string(),
                Value::Object(attrs_object(&self.attrs)),
            );
        }
        if self.kind.retains_raw_content()
            && self.raw.is_none()
            && (self.text.is_some() || !self.attrs.is_empty())
        {
            self.raw = Some(RawNodeContent {
                namespace: namespace.to_string(),
                original_kind: self.kind.wire_name().to_string(),
                payload: serde_json::json!({
                    "text": self.text,
                    "attrs": self.attrs,
                }),
            });
        }
    }
}

/// Raw parser content retained when no normalized node kind is sufficient.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawNodeContent {
    pub namespace: String,
    pub original_kind: String,
    pub payload: Value,
}

impl RawNodeContent {
    pub fn new(
        namespace: impl Into<String>,
        original_kind: impl Into<String>,
        payload: impl Into<Value>,
    ) -> Result<Self, DocumentGraphContractError> {
        let namespace = namespace.into();
        validate_namespace(&namespace).map_err(|_| {
            DocumentGraphContractError::InvalidExtensionNamespace {
                target: "raw node content".to_string(),
                namespace: namespace.clone(),
            }
        })?;
        Ok(Self {
            namespace,
            original_kind: original_kind.into(),
            payload: payload.into(),
        })
    }
}

/// Cross-document node vocabulary.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentNodeKind {
    Document,
    Container,
    ArchiveMember,
    Attachment,
    Metadata,
    Page,
    Slide,
    Sheet,
    Section,
    Heading,
    Header,
    Footer,
    Paragraph,
    TextRun,
    Text,
    Span,
    Emphasis,
    Strong,
    Link,
    List,
    ListItem,
    Quote,
    Table,
    TableRow,
    TableCell,
    Row,
    Cell,
    Chart,
    CodeBlock,
    InlineCode,
    MathInline,
    MathBlock,
    Math,
    Equation,
    Label,
    Reference,
    Citation,
    BibliographyEntry,
    Footnote,
    Endnote,
    Figure,
    Image,
    Caption,
    Annotation,
    Comment,
    Revision,
    Bookmark,
    Form,
    FormField,
    ContentControl,
    Email,
    MessageBody,
    MimePart,
    Thread,
    Notebook,
    NotebookCell,
    CellOutput,
    Transcript,
    Cue,
    MediaTrack,
    StructuredValue,
    Record,
    Frontmatter,
    RawBlock,
    RawInline,
    Raw,
    Unknown,

    Module,
    Namespace,
    Package,
    Symbol,
    CodeSymbol,
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

impl DocumentNodeKind {
    fn retains_raw_content(&self) -> bool {
        matches!(
            self,
            Self::Raw | Self::RawBlock | Self::RawInline | Self::Unknown | Self::Other(_)
        )
    }

    fn wire_name(&self) -> &str {
        match self {
            Self::Raw => "raw",
            Self::RawBlock => "raw_block",
            Self::RawInline => "raw_inline",
            Self::Unknown => "unknown",
            Self::Other(value) => value,
            _ => "normalized",
        }
    }
}

/// One normalized relation in a `DocumentGraph`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DocumentEdge {
    /// Stable, source-scoped identity for this relation occurrence.
    #[serde(default)]
    pub id: String,
    pub source: String,
    pub relation: DocumentRelation,
    pub target: String,
    pub range: Option<SourceRange>,
    /// Whether the relation was source-explicit or inferred.
    pub evidence: RelationEvidence,
    pub attrs: AttrMap,
    #[serde(default)]
    pub extensions: ExtensionMap,
}

#[derive(Deserialize)]
struct DocumentEdgeWire {
    #[serde(default)]
    id: String,
    source: String,
    relation: DocumentRelation,
    target: String,
    range: Option<SourceRange>,
    #[serde(default)]
    evidence: Option<RelationEvidence>,
    attrs: AttrMap,
    #[serde(default)]
    extensions: ExtensionMap,
}

impl<'de> Deserialize<'de> for DocumentEdge {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DocumentEdgeWire::deserialize(deserializer)?;
        Ok(Self {
            id: wire.id,
            source: wire.source,
            relation: wire.relation,
            target: wire.target,
            range: wire.range,
            evidence: wire.evidence.unwrap_or_else(legacy_edge_evidence),
            attrs: wire.attrs,
            extensions: wire.extensions,
        })
    }
}

#[cfg(feature = "schemas")]
#[allow(dead_code)]
#[derive(JsonSchema)]
struct DocumentEdgeSchema {
    id: String,
    source: String,
    relation: DocumentRelation,
    target: String,
    range: Option<SourceRange>,
    evidence: RelationEvidence,
    attrs: AttrMap,
    extensions: ExtensionMap,
}

#[cfg(feature = "schemas")]
impl JsonSchema for DocumentEdge {
    fn schema_name() -> String {
        "DocumentEdge".to_string()
    }

    fn json_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        DocumentEdgeSchema::json_schema(generator)
    }
}

impl DocumentEdge {
    pub fn new(
        source: impl Into<String>,
        relation: DocumentRelation,
        target: impl Into<String>,
    ) -> Self {
        Self {
            id: String::new(),
            source: source.into(),
            relation,
            target: target.into(),
            range: None,
            evidence: unspecified_edge_evidence(),
            attrs: AttrMap::new(),
            extensions: ExtensionMap::new(),
        }
    }

    pub fn explicit(
        source: impl Into<String>,
        relation: DocumentRelation,
        target: impl Into<String>,
        locator: SourceLocator,
    ) -> Self {
        Self {
            id: String::new(),
            source: source.into(),
            relation,
            target: target.into(),
            range: locator_text_range(&locator),
            evidence: RelationEvidence::Explicit { locator },
            attrs: AttrMap::new(),
            extensions: ExtensionMap::new(),
        }
    }

    pub fn inferred(
        source: impl Into<String>,
        relation: DocumentRelation,
        target: impl Into<String>,
        rule: impl Into<String>,
        confidence: LocatorConfidence,
    ) -> Self {
        Self {
            id: String::new(),
            source: source.into(),
            relation,
            target: target.into(),
            range: None,
            evidence: RelationEvidence::Inferred {
                inference: InferenceMetadata {
                    rule: rule.into(),
                    confidence,
                    evidence_locators: Vec::new(),
                },
            },
            attrs: AttrMap::new(),
            extensions: ExtensionMap::new(),
        }
    }

    pub fn with_inference_evidence(mut self, locator: SourceLocator) -> Self {
        if let RelationEvidence::Inferred { inference } = &mut self.evidence {
            inference.evidence_locators.push(locator);
        }
        self
    }

    pub fn with_range(mut self, range: SourceRange) -> Self {
        if let Ok(locator) = SourceLocator::try_from(range.clone()) {
            self.evidence = RelationEvidence::Explicit { locator };
        }
        self.range = Some(range);
        self
    }

    pub fn with_locator(mut self, locator: SourceLocator) -> Self {
        self.range = locator_text_range(&locator);
        self.evidence = RelationEvidence::Explicit { locator };
        self
    }

    pub fn with_attr(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.attrs.insert(key.into(), value.into());
        self
    }

    fn retain_namespaced_attrs(&mut self, namespace: &str) {
        if !self.attrs.is_empty() && !self.extensions.contains_key(namespace) {
            self.extensions.insert(
                namespace.to_string(),
                Value::Object(attrs_object(&self.attrs)),
            );
        }
    }

    fn hydrate_v2(&mut self, namespace: &str) {
        let is_legacy = matches!(
            &self.evidence,
            RelationEvidence::Inferred { inference }
                if inference.rule == LEGACY_EDGE_INFERENCE_RULE
        );
        if is_legacy {
            if let Some(range) = self.range.clone() {
                if let Ok(locator) = SourceLocator::try_from(range) {
                    self.evidence = RelationEvidence::Explicit { locator };
                }
            }
        }
        self.retain_namespaced_attrs(namespace);
    }
}

/// Evidence required for every normalized relation.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum RelationEvidence {
    Explicit { locator: SourceLocator },
    Inferred { inference: InferenceMetadata },
}

impl RelationEvidence {
    fn validate(&self, source: &str, target: &str) -> Result<(), DocumentGraphContractError> {
        match self {
            Self::Explicit { locator } => {
                locator
                    .validate()
                    .map_err(|error| DocumentGraphContractError::InvalidLocator {
                        target: format!("edge {source} -> {target}"),
                        message: error.to_string(),
                    })
            }
            Self::Inferred { inference } => {
                if inference.rule.trim().is_empty() {
                    return Err(DocumentGraphContractError::MissingInferenceRule {
                        edge_source: source.to_string(),
                        target: target.to_string(),
                    });
                }
                for locator in &inference.evidence_locators {
                    locator.validate().map_err(|error| {
                        DocumentGraphContractError::InvalidLocator {
                            target: format!("inference evidence for edge {source} -> {target}"),
                            message: error.to_string(),
                        }
                    })?;
                }
                Ok(())
            }
        }
    }
}

/// Rule and bounded confidence for a relation not explicitly present in the source.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InferenceMetadata {
    pub rule: String,
    pub confidence: LocatorConfidence,
    #[serde(default)]
    pub evidence_locators: Vec<SourceLocator>,
}

fn legacy_edge_evidence() -> RelationEvidence {
    RelationEvidence::Inferred {
        inference: InferenceMetadata {
            rule: LEGACY_EDGE_INFERENCE_RULE.to_string(),
            confidence: confidence_one(),
            evidence_locators: Vec::new(),
        },
    }
}

fn unspecified_edge_evidence() -> RelationEvidence {
    RelationEvidence::Inferred {
        inference: InferenceMetadata {
            rule: UNSPECIFIED_EDGE_INFERENCE_RULE.to_string(),
            confidence: confidence_one(),
            evidence_locators: Vec::new(),
        },
    }
}

fn confidence_one() -> LocatorConfidence {
    LocatorConfidence::new(1.0).expect("one is a valid confidence")
}

/// Cross-document relation vocabulary.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentRelation {
    Contains,
    Precedes,
    ParentOf,
    NextSibling,
    PreviousSibling,
    References,
    ResolvesTo,
    Defines,
    LinksTo,
    Cites,
    Annotates,
    CaptionFor,
    FootnoteFor,
    AnnotationFor,
    RevisionOf,
    AttachmentOf,
    EmbeddedIn,
    ReplyTo,
    DerivedFrom,
    SourceOf,
    EvidenceFor,
    AlternativeRepresentationOf,
    ReconciledWith,

    Imports,
    Exports,
    Calls,
    Inherits,
    Implements,
    Assigns,
    Returns,
    FormulaDependsOn,
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

/// A violation of the versioned normalized graph contract.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DocumentGraphContractError {
    #[error("unsupported DocumentGraph schema version {0}")]
    UnsupportedSchemaVersion(String),
    #[error("{target} uses non-namespaced extension key {namespace}")]
    InvalidExtensionNamespace { target: String, namespace: String },
    #[error("raw or unknown node {0} has no retained raw content")]
    MissingRawContent(String),
    #[error("{target} has an invalid source locator: {message}")]
    InvalidLocator { target: String, message: String },
    #[error("inferred edge {edge_source} -> {target} does not name an inference rule")]
    MissingInferenceRule { edge_source: String, target: String },
    #[error("invalid stable graph identity: {0}")]
    InvalidStableIdentity(String),
}

fn validate_extensions(
    target: &str,
    extensions: &ExtensionMap,
) -> Result<(), DocumentGraphContractError> {
    for namespace in extensions.keys() {
        validate_namespace(namespace).map_err(|_| {
            DocumentGraphContractError::InvalidExtensionNamespace {
                target: target.to_string(),
                namespace: namespace.clone(),
            }
        })?;
    }
    Ok(())
}

fn validate_namespace(namespace: &str) -> Result<(), ()> {
    let separator_count = namespace
        .chars()
        .filter(|ch| matches!(ch, '.' | ':'))
        .count();
    if separator_count == 0
        || namespace.starts_with(['.', ':'])
        || namespace.ends_with(['.', ':'])
        || !namespace
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | ':' | '_' | '-'))
    {
        return Err(());
    }
    Ok(())
}

fn attrs_object(attrs: &AttrMap) -> serde_json::Map<String, Value> {
    attrs
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn locator_text_range(locator: &SourceLocator) -> Option<SourceRange> {
    match locator.innermost() {
        crate::core::LocationComponent::TextRange {
            byte_start,
            byte_end,
            start_line,
            start_column,
            end_line,
            end_column,
        } => Some(SourceRange {
            byte_start: *byte_start,
            byte_end: *byte_end,
            start_line: *start_line,
            start_column: *start_column,
            end_line: *end_line,
            end_column: *end_column,
        }),
        _ => None,
    }
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

    /// Build the shared stable-ID scope for one authoritative payload.
    pub fn identity_generator(
        &self,
        payload_schema_version: impl Into<String>,
        parser_name: impl Into<String>,
    ) -> Result<GraphIdGenerator, GraphIdentityError> {
        GraphIdGenerator::new(
            self.graph_id.clone(),
            payload_schema_version,
            ParserInfo::new(parser_name),
        )
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

fn projection_transform_error(error: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: error.to_string(),
    }
}

fn stable_projection_node_id(
    identities: &GraphIdGenerator,
    structural_path: Vec<String>,
    native_id: Option<&str>,
    range: Option<&SourceRange>,
) -> Result<String, TransformError> {
    let locator = range
        .cloned()
        .and_then(|range| SourceLocator::try_from(range).ok());
    let address = ProjectionAddress {
        structural_path,
        native_id: native_id.map(str::to_owned),
        locator,
    };
    identities
        .node_id(&address)
        .map_err(projection_transform_error)
}

impl ToDocumentGraph for crate::text::TextDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::TEXT_V2, "grist.text")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Text).with_projection(
            "text",
            SchemaVersion::TEXT_V2,
            "grist.text.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.language = Some(context.language.unwrap_or_else(|| "text".to_string()));
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "encoding".to_string(),
            Value::String(self.encoding.label().to_string()),
        );
        graph.attrs.insert(
            "newline_fidelity".to_string(),
            serde_json::to_value(&self.decoding.newlines).map_err(projection_transform_error)?,
        );

        let root_id = stable_projection_node_id(
            &identities,
            vec!["document".to_string()],
            Some("root"),
            None,
        )?;
        graph.add_node(DocumentNode::new(&root_id, DocumentNodeKind::Document).with_ordinal(0));

        for (ordinal, block) in self.blocks.iter().enumerate() {
            let node_id = stable_projection_node_id(
                &identities,
                vec![
                    "document".to_string(),
                    "blocks".to_string(),
                    ordinal.to_string(),
                ],
                Some(&block.id),
                Some(&block.range),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Paragraph)
                .with_range(block.range.clone())
                .with_text(block.text.clone())
                .with_ordinal(ordinal.saturating_add(1));
            node.extensions.insert(
                "grist.text".to_string(),
                serde_json::json!({
                    "block_id": block.id,
                    "raw_range": block.raw_range,
                }),
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
        }

        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "restructured-text")]
impl ToDocumentGraph for crate::restructured_text::RestructuredTextDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::RESTRUCTURED_TEXT_V1, "restructured_text")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::RestructuredText)
            .with_projection(
                "restructured_text",
                SchemaVersion::RESTRUCTURED_TEXT_V1,
                "grist.restructured-text.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = Some(
            context
                .language
                .unwrap_or_else(|| "restructured_text".to_string()),
        );
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        let root_id = stable_projection_node_id(
            &identities,
            vec!["document".to_string()],
            Some("root"),
            None,
        )?;
        graph.add_node(DocumentNode::new(&root_id, DocumentNodeKind::Document).with_ordinal(0));
        project_rst_nodes(
            &self.nodes,
            &root_id,
            &["document".to_string(), "nodes".to_string()],
            &identities,
            &mut graph,
        )?;
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "restructured-text")]
fn project_rst_nodes(
    nodes: &[crate::restructured_text::RestructuredTextNode],
    owner: &str,
    path: &[String],
    identities: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let projected = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let mut node_path = path.to_vec();
            node_path.push(index.to_string());
            stable_projection_node_id(
                identities,
                node_path,
                Some(node.id.as_str()),
                Some(&node.range),
            )
            .map(|id| (node.id.clone(), id))
        })
        .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
    let definitions = nodes
        .iter()
        .filter_map(|node| Some((node.name.clone()?, projected.get(&node.id)?.clone())))
        .collect::<std::collections::HashMap<_, _>>();
    for (index, source_node) in nodes.iter().enumerate() {
        let node_id = projected[&source_node.id].clone();
        let mut node = DocumentNode::new(&node_id, rst_document_node_kind(&source_node.kind))
            .with_range(source_node.range.clone())
            .with_ordinal(index.saturating_add(1))
            .with_attr(
                "restructured_text_kind",
                format!("{:?}", source_node.kind).to_ascii_lowercase(),
            );
        node.text = source_node.text.clone();
        insert_rst_opt_attr(&mut node.attrs, "level", source_node.level.map(Value::from));
        insert_rst_opt_attr(
            &mut node.attrs,
            "name",
            source_node.name.clone().map(Value::from),
        );
        insert_rst_opt_attr(
            &mut node.attrs,
            "argument",
            source_node.argument.clone().map(Value::from),
        );
        insert_rst_opt_attr(
            &mut node.attrs,
            "target",
            source_node.target.clone().map(Value::from),
        );
        insert_rst_opt_attr(
            &mut node.attrs,
            "role",
            source_node.role.clone().map(Value::from),
        );
        node.extensions.insert(
            "grist.restructured_text".to_string(),
            serde_json::to_value(source_node).map_err(projection_transform_error)?,
        );
        if rst_retains_raw(&source_node.kind, source_node.known_syntax) {
            node.raw = Some(
                RawNodeContent::new(
                    "grist.restructured_text",
                    format!("{:?}", source_node.kind).to_ascii_lowercase(),
                    serde_json::json!({"syntax": source_node.raw}),
                )
                .map_err(projection_transform_error)?,
            );
        }
        graph.add_node(node);
        let parent = source_node
            .parent_id
            .as_ref()
            .and_then(|id| projected.get(id))
            .map(String::as_str)
            .unwrap_or(owner);
        graph.add_contains(parent, &node_id);
        project_rst_relations(source_node, &node_id, &definitions, graph);
        project_rst_include(source_node, &node_id, path, index, identities, graph)?;
    }
    Ok(())
}

#[cfg(feature = "restructured-text")]
fn project_rst_relations(
    node: &crate::restructured_text::RestructuredTextNode,
    node_id: &str,
    definitions: &std::collections::HashMap<String, String>,
    graph: &mut DocumentGraph,
) {
    use crate::restructured_text::RestructuredTextNodeKind as Rst;
    if let Some(target) = &node.target {
        graph.add_edge(
            DocumentEdge::new(node_id, DocumentRelation::LinksTo, target)
                .with_range(node.range.clone())
                .with_attr("target_kind", "uri_or_label"),
        );
    }
    if matches!(
        node.kind,
        Rst::CrossReference
            | Rst::FootnoteReference
            | Rst::CitationReference
            | Rst::SubstitutionReference
    ) && let Some(label) = &node.name
        && let Some(target) = definitions.get(label)
    {
        graph.add_edge(
            DocumentEdge::new(node_id, DocumentRelation::References, target)
                .with_range(node.range.clone())
                .with_attr("label", label.clone()),
        );
    }
}

#[cfg(feature = "restructured-text")]
fn project_rst_include(
    node: &crate::restructured_text::RestructuredTextNode,
    node_id: &str,
    path: &[String],
    index: usize,
    identities: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let Some(include) = &node.include else {
        return Ok(());
    };
    let Some(resolved) = &include.resolved else {
        graph.add_edge(
            DocumentEdge::new(
                node_id,
                DocumentRelation::References,
                include.target.as_str(),
            )
            .with_range(node.range.clone())
            .with_attr(
                "include_status",
                format!("{:?}", include.status).to_ascii_lowercase(),
            ),
        );
        return Ok(());
    };
    let mut child_path = path.to_vec();
    child_path.extend([index.to_string(), "resolved_include".to_string()]);
    let document_id = stable_projection_node_id(
        identities,
        child_path.clone(),
        include.resolved_path.as_deref(),
        None,
    )?;
    let mut document = DocumentNode::new(&document_id, DocumentNodeKind::Document)
        .with_ordinal(index.saturating_add(1))
        .with_attr(
            "repository_relative_path",
            include.resolved_path.clone().unwrap_or_default(),
        );
    document.extensions.insert(
        "grist.restructured_text".to_string(),
        serde_json::json!({
            "include_target": include.target,
            "source": resolved.source,
            "content_sha256": resolved.content_sha256,
        }),
    );
    graph.add_node(document);
    graph.add_contains(node_id, &document_id);
    graph.add_edge(
        DocumentEdge::new(node_id, DocumentRelation::ResolvesTo, &document_id)
            .with_range(node.range.clone())
            .with_attr("include_status", "resolved"),
    );
    child_path.push("nodes".to_string());
    project_rst_nodes(
        &resolved.nodes,
        &document_id,
        &child_path,
        identities,
        graph,
    )
}

#[cfg(feature = "restructured-text")]
fn rst_retains_raw(
    kind: &crate::restructured_text::RestructuredTextNodeKind,
    _known: bool,
) -> bool {
    use crate::restructured_text::RestructuredTextNodeKind as Rst;
    matches!(
        kind,
        Rst::Directive
            | Rst::Role
            | Rst::SubstitutionDefinition
            | Rst::Comment
            | Rst::RawBlock
            | Rst::RawInline
    )
}

#[cfg(feature = "restructured-text")]
fn rst_document_node_kind(
    kind: &crate::restructured_text::RestructuredTextNodeKind,
) -> DocumentNodeKind {
    use crate::restructured_text::RestructuredTextNodeKind as Rst;
    match kind {
        Rst::Heading => DocumentNodeKind::Heading,
        Rst::Paragraph => DocumentNodeKind::Paragraph,
        Rst::Include => DocumentNodeKind::Reference,
        Rst::CodeBlock | Rst::LiteralBlock | Rst::DoctestBlock => DocumentNodeKind::CodeBlock,
        Rst::Table => DocumentNodeKind::Table,
        Rst::TableRow => DocumentNodeKind::TableRow,
        Rst::TableCell => DocumentNodeKind::TableCell,
        Rst::FootnoteDefinition => DocumentNodeKind::Footnote,
        Rst::FootnoteReference | Rst::CrossReference => DocumentNodeKind::Reference,
        Rst::CitationDefinition => DocumentNodeKind::BibliographyEntry,
        Rst::CitationReference => DocumentNodeKind::Citation,
        Rst::Target => DocumentNodeKind::Label,
        Rst::Hyperlink => DocumentNodeKind::Link,
        Rst::List | Rst::DefinitionList | Rst::FieldList => DocumentNodeKind::List,
        Rst::ListItem | Rst::DefinitionTerm | Rst::DefinitionDescription | Rst::Field => {
            DocumentNodeKind::ListItem
        }
        Rst::Emphasis => DocumentNodeKind::Emphasis,
        Rst::Strong => DocumentNodeKind::Strong,
        Rst::InlineCode => DocumentNodeKind::InlineCode,
        Rst::SubstitutionDefinition => DocumentNodeKind::RawBlock,
        Rst::SubstitutionReference => DocumentNodeKind::Reference,
        Rst::Transition => DocumentNodeKind::Span,
        Rst::Directive | Rst::Comment | Rst::RawBlock => DocumentNodeKind::RawBlock,
        Rst::Role | Rst::RawInline => DocumentNodeKind::RawInline,
    }
}

#[cfg(feature = "restructured-text")]
fn insert_rst_opt_attr(attrs: &mut AttrMap, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        attrs.insert(key.to_string(), value);
    }
}

#[cfg(feature = "html")]
impl ToDocumentGraph for crate::html::HtmlDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::HTML_V2, "html")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Html).with_projection(
            "html",
            SchemaVersion::HTML_V2,
            "grist.html.to-document-graph.v2",
        );
        graph.source = context.source;
        graph.language = Some(context.language.unwrap_or_else(|| "html".to_string()));
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| format!("{:?}", self.syntax).to_ascii_lowercase()),
        );
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "mode".to_string(),
            Value::String(format!("{:?}", self.mode).to_ascii_lowercase()),
        );
        graph.attrs.insert(
            "encoding".to_string(),
            Value::String(self.encoding.label().to_string()),
        );
        let root_id = stable_projection_node_id(
            &identities,
            vec!["document".to_string()],
            Some("root"),
            Some(&self.decoded_range),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_range(self.decoded_range.clone())
            .with_ordinal(0);
        root.extensions.insert(
            "grist.html".to_string(),
            serde_json::json!({
                "syntax": self.syntax,
                "mode": self.mode,
                "has_doctype": self.has_doctype,
                "quirks_mode": self.quirks_mode,
            }),
        );
        graph.add_node(root);

        let projected = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.kind != crate::html::HtmlNodeKind::Document)
            .map(|(index, node)| {
                stable_projection_node_id(
                    &identities,
                    vec![
                        "document".to_string(),
                        "nodes".to_string(),
                        index.to_string(),
                    ],
                    Some(node.id.as_str()),
                    Some(&node.range),
                )
                .map(|id| (node.id.clone(), id))
            })
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;

        for (ordinal, source_node) in self
            .nodes
            .iter()
            .filter(|node| node.kind != crate::html::HtmlNodeKind::Document)
            .enumerate()
        {
            let node_id = projected[&source_node.id].clone();
            let mut node = DocumentNode::new(&node_id, html_document_node_kind(source_node))
                .with_range(source_node.range.clone())
                .with_locator(source_node.locator.clone())
                .with_ordinal(ordinal.saturating_add(1))
                .with_attr("dom_path", source_node.dom_path.clone())
                .with_attr("synthetic", source_node.synthetic);
            node.text = source_node.text.clone();
            node.name = source_node.tag_name.clone();
            node.extensions.insert(
                "grist.html".to_string(),
                serde_json::to_value(source_node).map_err(projection_transform_error)?,
            );
            let link_target = html_link_target(source_node).map(str::to_string);
            if let Some(target) = &link_target {
                node.attrs
                    .insert("destination".to_string(), Value::String(target.clone()));
            }
            if !source_node.known_element
                || matches!(
                    source_node.kind,
                    crate::html::HtmlNodeKind::RawUnknown
                        | crate::html::HtmlNodeKind::ProcessingInstruction
                )
            {
                node.raw = Some(
                    RawNodeContent::new(
                        "grist.html",
                        source_node.tag_name.as_deref().unwrap_or("unknown"),
                        serde_json::json!({"syntax": source_node.raw}),
                    )
                    .map_err(projection_transform_error)?,
                );
            }
            graph.add_node(node);
            let parent = source_node
                .parent_id
                .as_ref()
                .and_then(|id| projected.get(id))
                .map(String::as_str)
                .unwrap_or(root_id.as_str());
            graph.add_contains(parent, &node_id);

            if let Some(target) = link_target {
                graph.add_edge(
                    DocumentEdge::new(&node_id, DocumentRelation::LinksTo, target)
                        .with_locator(source_node.locator.clone()),
                );
            }
        }
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "html")]
fn html_link_target(node: &crate::html::HtmlNode) -> Option<&str> {
    node.attributes
        .iter()
        .find(|attribute| {
            matches!(
                attribute.local_name.to_ascii_lowercase().as_str(),
                "href" | "src" | "action" | "formaction"
            )
        })
        .and_then(|attribute| attribute.value.as_deref())
}

#[cfg(feature = "html")]
fn html_document_node_kind(node: &crate::html::HtmlNode) -> DocumentNodeKind {
    use crate::html::HtmlNodeKind;
    match node.kind {
        HtmlNodeKind::Text => DocumentNodeKind::Text,
        HtmlNodeKind::Comment => DocumentNodeKind::Comment,
        HtmlNodeKind::Doctype => DocumentNodeKind::Metadata,
        HtmlNodeKind::ProcessingInstruction | HtmlNodeKind::RawUnknown => DocumentNodeKind::Raw,
        HtmlNodeKind::Document => DocumentNodeKind::Document,
        HtmlNodeKind::Element => match node.tag_name.as_deref().unwrap_or_default() {
            "section" | "article" | "nav" | "main" | "aside" | "search" => {
                DocumentNodeKind::Section
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => DocumentNodeKind::Heading,
            "header" => DocumentNodeKind::Header,
            "footer" => DocumentNodeKind::Footer,
            "p" => DocumentNodeKind::Paragraph,
            "a" | "area" | "link" => DocumentNodeKind::Link,
            "ul" | "ol" | "menu" => DocumentNodeKind::List,
            "li" => DocumentNodeKind::ListItem,
            "blockquote" | "q" => DocumentNodeKind::Quote,
            "table" => DocumentNodeKind::Table,
            "tr" => DocumentNodeKind::TableRow,
            "td" | "th" => DocumentNodeKind::TableCell,
            "figure" => DocumentNodeKind::Figure,
            "figcaption" | "caption" => DocumentNodeKind::Caption,
            "img" | "picture" | "svg" | "canvas" => DocumentNodeKind::Image,
            "audio" | "video" | "source" | "track" => DocumentNodeKind::MediaTrack,
            "pre" => DocumentNodeKind::CodeBlock,
            "code" => DocumentNodeKind::InlineCode,
            "em" | "i" => DocumentNodeKind::Emphasis,
            "strong" | "b" => DocumentNodeKind::Strong,
            "form" => DocumentNodeKind::Form,
            "input" | "button" | "select" | "textarea" | "option" | "fieldset" => {
                DocumentNodeKind::FormField
            }
            "meta" | "title" | "base" | "head" | "html" => DocumentNodeKind::Metadata,
            _ if !node.known_element => DocumentNodeKind::RawBlock,
            _ => DocumentNodeKind::Span,
        },
    }
}

#[cfg(feature = "pdf")]
impl ToDocumentGraph for crate::pdf::PdfDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::PDF_V1, "pdf")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Pdf).with_projection(
            "pdf",
            SchemaVersion::PDF_V1,
            "grist.pdf.to-document-graph.v2",
        );
        graph.source = context.source;
        graph.language = self.catalog.language.clone().or(context.language);
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| format!("pdf-{}", self.header.version)),
        );
        graph.attrs = context.attrs;
        graph
            .attrs
            .insert("page_count".into(), Value::from(self.pages.len() as u64));
        graph.attrs.insert(
            "object_count".into(),
            Value::from(self.objects.len() as u64),
        );
        graph
            .attrs
            .insert("xref_repaired".into(), Value::Bool(self.xref.repaired));
        graph
            .attrs
            .insert("encrypted".into(), Value::Bool(self.encryption.encrypted));

        let root_id = identities
            .node_id(&ProjectionAddress::native(
                ["document", "catalog"],
                format!(
                    "catalog:{}:{}",
                    self.catalog.object.object_number, self.catalog.object.generation
                ),
            ))
            .map_err(projection_transform_error)?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name(format!("PDF {}", self.header.version))
            .with_ordinal(0);
        root.extensions.insert(
            "grist.pdf".into(),
            serde_json::json!({
                "catalog": self.catalog,
                "trailer": self.trailer,
                "page_tree": self.page_tree,
                "page_labels": self.page_labels,
                "metadata": self.metadata,
                "filters": self.filters,
                "encryption": self.encryption,
                "repairs": self.repairs,
                "active_content": self.active_content,
                "interactive": self.interactive,
                "native_fonts": self.native_layout.fonts,
                "semantic_structure_tree": self.semantic_structure.structure_tree,
                "repeated_regions": self.semantic_structure.repeated_regions,
            }),
        );
        graph.add_node(root);

        for page in &self.pages {
            let semantic_page = self
                .semantic_structure
                .pages
                .iter()
                .find(|semantic| semantic.page_index == page.index);
            let native_layout = self
                .native_layout
                .pages
                .iter()
                .find(|layout| layout.page_index == page.index);
            let page_text = self
                .text
                .pages
                .iter()
                .find(|value| value.page_index == page.index);
            let node_id = identities
                .node_id(&ProjectionAddress {
                    structural_path: vec![
                        "document".into(),
                        "pages".into(),
                        page.index.to_string(),
                    ],
                    native_id: Some(format!(
                        "page:{}:{}",
                        page.object.object_number, page.object.generation
                    )),
                    locator: Some(page.locator.clone()),
                })
                .map_err(projection_transform_error)?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Page)
                .with_name(
                    page.label
                        .clone()
                        .unwrap_or_else(|| format!("page {}", page.index)),
                )
                .with_locator(page.locator.clone())
                .with_ordinal(page.index as usize);
            node.extensions.insert(
                "grist.pdf".into(),
                serde_json::json!({
                    "page": page,
                    "native_layout": native_layout,
                    "text_representations": page_text,
                    "semantic_structure": semantic_page,
                }),
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            if let Some(layout) = native_layout {
                let mut block_ids = Vec::new();
                for block in &layout.blocks {
                    let semantic_block = semantic_page.and_then(|semantic| {
                        semantic
                            .blocks
                            .iter()
                            .find(|candidate| candidate.native_block_index == block.index)
                    });
                    let block_id = identities
                        .node_id(&ProjectionAddress {
                            structural_path: vec![
                                "document".into(),
                                "pages".into(),
                                page.index.to_string(),
                                "blocks".into(),
                                block.index.to_string(),
                            ],
                            native_id: Some(format!("native-block:{}:{}", page.index, block.index)),
                            locator: Some(block.locator.clone()),
                        })
                        .map_err(projection_transform_error)?;
                    let kind =
                        semantic_block.map_or(
                            DocumentNodeKind::Paragraph,
                            |semantic| match semantic.kind {
                                crate::pdf::PdfSemanticBlockKind::Heading => {
                                    DocumentNodeKind::Heading
                                }
                                crate::pdf::PdfSemanticBlockKind::Paragraph => {
                                    DocumentNodeKind::Paragraph
                                }
                                crate::pdf::PdfSemanticBlockKind::ListItem => {
                                    DocumentNodeKind::ListItem
                                }
                                crate::pdf::PdfSemanticBlockKind::Header => {
                                    DocumentNodeKind::Header
                                }
                                crate::pdf::PdfSemanticBlockKind::Footer => {
                                    DocumentNodeKind::Footer
                                }
                                crate::pdf::PdfSemanticBlockKind::PageNumber => {
                                    DocumentNodeKind::Text
                                }
                                crate::pdf::PdfSemanticBlockKind::Caption => {
                                    DocumentNodeKind::Caption
                                }
                            },
                        );
                    let mut block_node = DocumentNode::new(&block_id, kind)
                        .with_text(&block.text)
                        .with_locator(block.locator.clone())
                        .with_ordinal(block.index as usize);
                    block_node
                        .attrs
                        .insert("text_origin".into(), "native".into());
                    block_node.attrs.insert(
                        "segment_primary".into(),
                        Value::Bool(page_text.is_none_or(|value| value.reconciled.is_none())),
                    );
                    block_node.extensions.insert(
                        "grist.pdf".into(),
                        serde_json::json!({ "native": block, "semantic": semantic_block }),
                    );
                    graph.add_node(block_node);
                    graph.add_contains(&node_id, &block_id);
                    block_ids.push((block.index, block_id.clone(), block.locator.clone()));

                    for line in layout.lines.iter().filter(|line| {
                        line.index >= block.line_start && line.index < block.line_end
                    }) {
                        let line_id = identities
                            .node_id(&ProjectionAddress {
                                structural_path: vec![
                                    "document".into(),
                                    "pages".into(),
                                    page.index.to_string(),
                                    "lines".into(),
                                    line.index.to_string(),
                                ],
                                native_id: Some(format!(
                                    "native-line:{}:{}",
                                    page.index, line.index
                                )),
                                locator: Some(line.locator.clone()),
                            })
                            .map_err(projection_transform_error)?;
                        let mut line_node = DocumentNode::new(&line_id, DocumentNodeKind::TextRun)
                            .with_text(&line.text)
                            .with_locator(line.locator.clone())
                            .with_ordinal(line.index as usize);
                        line_node.extensions.insert(
                            "grist.pdf".into(),
                            serde_json::to_value(line).map_err(projection_transform_error)?,
                        );
                        graph.add_node(line_node);
                        graph.add_contains(&block_id, &line_id);

                        for token in layout.tokens.iter().filter(|token| {
                            token.index >= line.token_start && token.index < line.token_end
                        }) {
                            let token_id = identities
                                .node_id(&ProjectionAddress {
                                    structural_path: vec![
                                        "document".into(),
                                        "pages".into(),
                                        page.index.to_string(),
                                        "tokens".into(),
                                        token.index.to_string(),
                                    ],
                                    native_id: Some(format!(
                                        "native-token:{}:{}",
                                        page.index, token.index
                                    )),
                                    locator: Some(token.locator.clone()),
                                })
                                .map_err(projection_transform_error)?;
                            let mut token_node =
                                DocumentNode::new(&token_id, DocumentNodeKind::Span)
                                    .with_text(&token.text)
                                    .with_locator(token.locator.clone())
                                    .with_ordinal(token.index as usize);
                            token_node.extensions.insert(
                                "grist.pdf".into(),
                                serde_json::to_value(token).map_err(projection_transform_error)?,
                            );
                            graph.add_node(token_node);
                            graph.add_contains(&line_id, &token_id);
                        }
                    }
                }
                block_ids.sort_by_key(|(index, _, _)| {
                    layout
                        .reading_order
                        .block_order
                        .iter()
                        .position(|ordered| ordered == index)
                        .unwrap_or(usize::MAX)
                });
                for pair in block_ids.windows(2) {
                    graph.add_edge(
                        DocumentEdge::inferred(
                            pair[0].1.clone(),
                            DocumentRelation::Precedes,
                            pair[1].1.clone(),
                            "grist.pdf.native-reading-order.v1",
                            LocatorConfidence::new(layout.reading_order.confidence)
                                .map_err(projection_transform_error)?,
                        )
                        .with_inference_evidence(pair[0].2.clone())
                        .with_inference_evidence(pair[1].2.clone()),
                    );
                }
            }
            if let Some(semantic) = semantic_page {
                project_pdf_semantic_page(
                    &mut graph,
                    &identities,
                    page.index,
                    &node_id,
                    native_layout,
                    semantic,
                )?;
            }
            if let Some(text) = page_text {
                project_pdf_text_representations(
                    &mut graph,
                    &identities,
                    page.index,
                    &node_id,
                    text,
                )?;
            }
        }
        project_pdf_interactive(
            &mut graph,
            &identities,
            &root_id,
            &self.pages,
            &self.interactive,
        )?;
        graph.diagnostics.extend(self.repairs.iter().map(|repair| {
            Diagnostic::warning("grist.pdf", repair.code.clone(), repair.description.clone())
                .partial()
        }));
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "pdf")]
fn project_pdf_text_representations(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    page_index: u64,
    page_id: &str,
    text: &crate::pdf::PdfPageTextContent,
) -> Result<(), TransformError> {
    let mut native_ids = BTreeMap::new();
    for region in &text.native.value.regions {
        let node_id = identities
            .node_id(&ProjectionAddress {
                structural_path: vec![
                    "document".into(),
                    "pages".into(),
                    page_index.to_string(),
                    "blocks".into(),
                    region.index.to_string(),
                ],
                native_id: Some(format!("native-block:{page_index}:{}", region.index)),
                locator: Some(region.locator.clone()),
            })
            .map_err(projection_transform_error)?;
        native_ids.insert(region.index, node_id);
    }

    let mut ocr_ids = BTreeMap::new();
    for (attempt_index, attempt) in text.ocr_attempts.iter().enumerate() {
        let provider_identity = attempt
            .response
            .metadata
            .output_identity
            .as_deref()
            .unwrap_or(&attempt.response.metadata.request_digest);
        for region in &attempt.regions {
            let node_id = identities
                .node_id(&ProjectionAddress {
                    structural_path: vec![
                        "document".into(),
                        "pages".into(),
                        page_index.to_string(),
                        "text".into(),
                        "ocr".into(),
                        attempt_index.to_string(),
                        region.index.to_string(),
                    ],
                    native_id: Some(format!(
                        "ocr:{provider_identity}:{page_index}:{attempt_index}:{}",
                        region.index
                    )),
                    locator: Some(region.locator.clone()),
                })
                .map_err(projection_transform_error)?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::TextRun)
                .with_text(&region.text)
                .with_locator(region.locator.clone())
                .with_ordinal(region.reading_order as usize);
            node.attrs.insert("text_origin".into(), "ocr".into());
            node.attrs
                .insert("segment_primary".into(), Value::Bool(false));
            node.extensions.insert(
                "grist.pdf".into(),
                serde_json::json!({
                    "origin": "ocr",
                    "scope": attempt.scope,
                    "region": region,
                    "provider": attempt.response.metadata.provider,
                    "request_digest": attempt.response.metadata.request_digest,
                    "output_identity": attempt.response.metadata.output_identity,
                    "reading_order_confidence": attempt.reading_order_confidence,
                    "layout_confidence": attempt.layout_confidence,
                }),
            );
            graph.add_node(node);
            graph.add_contains(page_id, &node_id);
            ocr_ids.insert((attempt_index as u64, region.index), node_id);
        }
    }

    if let Some(reconciled) = &text.reconciled {
        for (index, item) in reconciled.value.items.iter().enumerate() {
            let node_id = identities
                .node_id(&ProjectionAddress {
                    structural_path: vec![
                        "document".into(),
                        "pages".into(),
                        page_index.to_string(),
                        "text".into(),
                        "reconciled".into(),
                        index.to_string(),
                    ],
                    native_id: Some(format!(
                        "reconciled:{}:{page_index}:{}",
                        reconciled.identity, item.index
                    )),
                    locator: Some(item.locator.clone()),
                })
                .map_err(projection_transform_error)?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Paragraph)
                .with_text(&item.text)
                .with_locator(item.locator.clone())
                .with_ordinal(item.index as usize);
            node.attrs.insert("text_origin".into(), "reconciled".into());
            node.attrs
                .insert("segment_primary".into(), Value::Bool(true));
            node.extensions.insert(
                "grist.pdf".into(),
                serde_json::json!({
                    "origin": "reconciled",
                    "item": item,
                    "algorithm": reconciled.algorithm,
                    "algorithm_version": reconciled.algorithm_version,
                    "configuration_digest": reconciled.configuration_digest,
                    "reading_order_confidence": reconciled.value.reading_order_confidence,
                    "layout_confidence": reconciled.value.layout_confidence,
                }),
            );
            graph.add_node(node);
            graph.add_contains(page_id, &node_id);
            let source_id = match item.source {
                crate::pdf::PdfReconciledTextSource::Native { region_index } => {
                    native_ids.get(&region_index)
                }
                crate::pdf::PdfReconciledTextSource::Ocr {
                    attempt_index,
                    region_index,
                } => ocr_ids.get(&(attempt_index, region_index)),
            };
            if let Some(source_id) = source_id {
                graph.add_edge(
                    DocumentEdge::inferred(
                        source_id.clone(),
                        DocumentRelation::ReconciledWith,
                        node_id,
                        "grist.pdf.native-ocr-reconcile.v1",
                        LocatorConfidence::new(item.confidence)
                            .map_err(projection_transform_error)?,
                    )
                    .with_inference_evidence(item.locator.clone()),
                );
            }
        }

        for suppression in &reconciled.value.suppressions {
            let source_id = match suppression.suppressed {
                crate::pdf::PdfReconciledTextSource::Ocr {
                    attempt_index,
                    region_index,
                } => ocr_ids.get(&(attempt_index, region_index)),
                crate::pdf::PdfReconciledTextSource::Native { region_index } => {
                    native_ids.get(&region_index)
                }
            };
            let target_id = match suppression.retained {
                crate::pdf::PdfReconciledTextSource::Native { region_index } => {
                    native_ids.get(&region_index)
                }
                crate::pdf::PdfReconciledTextSource::Ocr {
                    attempt_index,
                    region_index,
                } => ocr_ids.get(&(attempt_index, region_index)),
            };
            if let (Some(source_id), Some(target_id)) = (source_id, target_id) {
                graph.add_edge(
                    DocumentEdge::inferred(
                        source_id.clone(),
                        DocumentRelation::AlternativeRepresentationOf,
                        target_id.clone(),
                        "grist.pdf.duplicate-suppression.v1",
                        LocatorConfidence::new(
                            (suppression.text_similarity + suppression.geometric_overlap) / 2.0,
                        )
                        .map_err(projection_transform_error)?,
                    )
                    .with_inference_evidence(suppression.locator.clone())
                    .with_inference_evidence(suppression.retained_locator.clone()),
                );
            }
        }
    }
    Ok(())
}

#[cfg(feature = "pdf")]
fn project_pdf_interactive(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    root_id: &str,
    pages: &[crate::pdf::PdfPage],
    interactive: &crate::pdf::PdfInteractiveContent,
) -> Result<(), TransformError> {
    let mut ids = BTreeMap::new();
    let mut add = |graph: &mut DocumentGraph,
                   source_id: &str,
                   kind: DocumentNodeKind,
                   name: Option<String>,
                   text: Option<String>,
                   locator: Option<SourceLocator>,
                   extension: Value,
                   ordinal: usize|
     -> Result<String, TransformError> {
        let node_id = identities
            .node_id(&ProjectionAddress {
                structural_path: vec!["document".into(), "interactive".into(), source_id.into()],
                native_id: Some(source_id.into()),
                locator: locator.clone(),
            })
            .map_err(projection_transform_error)?;
        let mut node = DocumentNode::new(&node_id, kind).with_ordinal(ordinal);
        if let Some(name) = name {
            node = node.with_name(name);
        }
        if let Some(text) = text {
            node = node.with_text(text);
        }
        if let Some(locator) = locator {
            node = node.with_locator(locator);
        }
        node.extensions.insert("grist.pdf".into(), extension);
        graph.add_node(node);
        graph.add_contains(root_id, &node_id);
        ids.insert(source_id.to_string(), node_id.clone());
        Ok(node_id)
    };
    for (index, value) in interactive.destinations.iter().enumerate() {
        add(
            graph,
            &value.id,
            DocumentNodeKind::Bookmark,
            Some(value.name.clone()),
            None,
            None,
            serde_json::to_value(value).map_err(projection_transform_error)?,
            index,
        )?;
    }
    for (index, value) in interactive.outlines.iter().enumerate() {
        add(
            graph,
            &value.id,
            DocumentNodeKind::Bookmark,
            value.title.clone(),
            None,
            None,
            serde_json::to_value(value).map_err(projection_transform_error)?,
            index,
        )?;
    }
    for (index, value) in interactive.links.iter().enumerate() {
        add(
            graph,
            &value.id,
            DocumentNodeKind::Link,
            None,
            value.action.as_ref().and_then(|action| action.uri.clone()),
            Some(value.locator.clone()),
            serde_json::to_value(value).map_err(projection_transform_error)?,
            index,
        )?;
    }
    for (index, value) in interactive.annotations.iter().enumerate() {
        add(
            graph,
            &value.id,
            DocumentNodeKind::Annotation,
            value.subject.clone(),
            value.contents.clone(),
            Some(value.locator.clone()),
            serde_json::to_value(value).map_err(projection_transform_error)?,
            index,
        )?;
    }
    for (index, value) in interactive.comments.iter().enumerate() {
        add(
            graph,
            &value.id,
            DocumentNodeKind::Comment,
            value.subject.clone(),
            Some(value.text.clone()),
            Some(value.locator.clone()),
            serde_json::to_value(value).map_err(projection_transform_error)?,
            index,
        )?;
    }
    if let Some(form) = &interactive.form {
        let form_id = add(
            graph,
            "pdf:form",
            DocumentNodeKind::Form,
            None,
            None,
            None,
            serde_json::to_value(form).map_err(projection_transform_error)?,
            0,
        )?;
        for (index, field) in form.fields.iter().enumerate() {
            let field_id = add(
                graph,
                &field.id,
                DocumentNodeKind::FormField,
                field.partial_name.clone(),
                None,
                None,
                serde_json::to_value(field).map_err(projection_transform_error)?,
                index,
            )?;
            graph.add_edge(DocumentEdge::explicit(
                form_id.clone(),
                DocumentRelation::Contains,
                field_id,
                pdf_object_source_locator(&field.locator),
            ));
        }
    }
    for (index, value) in interactive.signatures.iter().enumerate() {
        add(
            graph,
            &value.id,
            DocumentNodeKind::Metadata,
            value.signer_name.clone(),
            None,
            None,
            serde_json::to_value(value).map_err(projection_transform_error)?,
            index,
        )?;
    }
    for (index, value) in interactive.layers.iter().enumerate() {
        add(
            graph,
            &value.id,
            DocumentNodeKind::Metadata,
            value.name.clone(),
            None,
            None,
            serde_json::to_value(value).map_err(projection_transform_error)?,
            index,
        )?;
    }
    project_pdf_attachments(
        graph,
        identities,
        root_id,
        &interactive.embedded_files,
        &mut ids,
    )?;
    for page in pages {
        let node_id = identities
            .node_id(&ProjectionAddress {
                structural_path: vec!["document".into(), "pages".into(), page.index.to_string()],
                native_id: Some(format!(
                    "page:{}:{}",
                    page.object.object_number, page.object.generation
                )),
                locator: Some(page.locator.clone()),
            })
            .map_err(projection_transform_error)?;
        ids.insert(format!("pdf:page:{}", page.index), node_id);
    }
    for relation in &interactive.relationships {
        let (Some(source), Some(target)) =
            (ids.get(&relation.source_id), ids.get(&relation.target_id))
        else {
            continue;
        };
        graph.add_edge(DocumentEdge::explicit(
            source.clone(),
            pdf_interactive_relation(relation.relation),
            target.clone(),
            pdf_object_source_locator(&relation.locator),
        ));
    }
    Ok(())
}

#[cfg(feature = "pdf")]
fn project_pdf_attachments(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    parent_id: &str,
    files: &[crate::pdf::PdfEmbeddedFile],
    ids: &mut BTreeMap<String, String>,
) -> Result<(), TransformError> {
    for (index, file) in files.iter().enumerate() {
        let node_id = identities
            .node_id(&ProjectionAddress {
                structural_path: vec![
                    "document".into(),
                    "attachments".into(),
                    file.artifact.identity.artifact_id.clone(),
                ],
                native_id: Some(file.artifact.identity.artifact_id.clone()),
                locator: Some(file.artifact.locator.clone()),
            })
            .map_err(projection_transform_error)?;
        let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Attachment)
            .with_locator(file.artifact.locator.clone())
            .with_ordinal(index);
        if let Some(name) = &file.artifact.declared_filename {
            node = node.with_name(name);
        }
        node.extensions.insert(
            "grist.pdf".into(),
            serde_json::to_value(file).map_err(projection_transform_error)?,
        );
        graph.add_node(node);
        graph.add_edge(DocumentEdge::explicit(
            parent_id.to_string(),
            DocumentRelation::AttachmentOf,
            node_id.clone(),
            file.artifact.locator.clone(),
        ));
        ids.insert(file.id.clone(), node_id.clone());
        project_pdf_attachments(graph, identities, &node_id, &file.children, ids)?;
    }
    Ok(())
}

#[cfg(feature = "pdf")]
fn pdf_object_source_locator(locator: &crate::pdf::PdfObjectLocator) -> SourceLocator {
    let keys = if locator.key_path.is_empty() {
        String::new()
    } else {
        format!("/{}", locator.key_path.join("/"))
    };
    SourceLocator::exact(crate::core::LocationComponent::JsonPointer {
        pointer: format!(
            "/pdf/objects/{}:{}{}",
            locator.object.object_number, locator.object.generation, keys
        ),
    })
    .expect("PDF object source locator is valid")
}

#[cfg(feature = "pdf")]
fn pdf_interactive_relation(relation: crate::pdf::PdfInteractiveRelation) -> DocumentRelation {
    match relation {
        crate::pdf::PdfInteractiveRelation::ResolvesTo => DocumentRelation::ResolvesTo,
        crate::pdf::PdfInteractiveRelation::ParentOf => DocumentRelation::ParentOf,
        crate::pdf::PdfInteractiveRelation::NextSibling => DocumentRelation::NextSibling,
        crate::pdf::PdfInteractiveRelation::PreviousSibling => DocumentRelation::PreviousSibling,
        crate::pdf::PdfInteractiveRelation::AnnotationFor => DocumentRelation::AnnotationFor,
        crate::pdf::PdfInteractiveRelation::ReplyTo => DocumentRelation::ReplyTo,
        crate::pdf::PdfInteractiveRelation::FieldWidget => {
            DocumentRelation::Other("field_widget".into())
        }
        crate::pdf::PdfInteractiveRelation::AttachmentOf => DocumentRelation::AttachmentOf,
        crate::pdf::PdfInteractiveRelation::EmbeddedIn => DocumentRelation::EmbeddedIn,
    }
}

#[cfg(feature = "pdf")]
fn project_pdf_semantic_page(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    page_index: u64,
    page_id: &str,
    native_layout: Option<&crate::pdf::PdfPageNativeLayout>,
    semantic: &crate::pdf::PdfPageSemanticStructure,
) -> Result<(), TransformError> {
    let id = |category: &str, index: u64, native: String, locator: SourceLocator| {
        identities
            .node_id(&ProjectionAddress {
                structural_path: vec![
                    "document".into(),
                    "pages".into(),
                    page_index.to_string(),
                    "semantic".into(),
                    category.into(),
                    index.to_string(),
                ],
                native_id: Some(native),
                locator: Some(locator),
            })
            .map_err(projection_transform_error)
    };
    let mut block_ids = BTreeMap::new();
    for block in &semantic.blocks {
        let native_locator = native_layout
            .and_then(|layout| {
                layout
                    .blocks
                    .iter()
                    .find(|native| native.index == block.native_block_index)
            })
            .map_or_else(|| block.locator.clone(), |native| native.locator.clone());
        let node_id = identities
            .node_id(&ProjectionAddress {
                structural_path: vec![
                    "document".into(),
                    "pages".into(),
                    page_index.to_string(),
                    "blocks".into(),
                    block.native_block_index.to_string(),
                ],
                native_id: Some(format!(
                    "native-block:{page_index}:{}",
                    block.native_block_index
                )),
                locator: Some(native_locator),
            })
            .map_err(projection_transform_error)?;
        block_ids.insert(block.index, node_id);
    }
    for list in &semantic.lists {
        let node_id = id(
            "lists",
            list.index,
            format!("semantic-list:{page_index}:{}", list.index),
            list.locator.clone(),
        )?;
        let mut node = DocumentNode::new(&node_id, DocumentNodeKind::List)
            .with_locator(list.locator.clone())
            .with_ordinal(list.index as usize);
        node.extensions.insert(
            "grist.pdf".into(),
            serde_json::to_value(list).map_err(projection_transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(page_id, &node_id);
        for block_index in &list.item_block_indices {
            if let Some(block_id) = block_ids.get(block_index) {
                graph.add_edge(
                    DocumentEdge::inferred(
                        node_id.clone(),
                        DocumentRelation::Contains,
                        block_id.clone(),
                        "grist.pdf.list-inference.v1",
                        LocatorConfidence::new(list.confidence)
                            .map_err(projection_transform_error)?,
                    )
                    .with_inference_evidence(list.locator.clone()),
                );
            }
        }
    }
    let mut table_ids = BTreeMap::new();
    for table in &semantic.tables {
        let table_id = id(
            "tables",
            table.index,
            format!("semantic-table:{page_index}:{}", table.index),
            table.locator.clone(),
        )?;
        let mut node = DocumentNode::new(&table_id, DocumentNodeKind::Table)
            .with_locator(table.locator.clone())
            .with_ordinal(table.index as usize);
        node.extensions.insert(
            "grist.pdf".into(),
            serde_json::to_value(table).map_err(projection_transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(page_id, &table_id);
        for row in &table.rows {
            let row_id = id(
                &format!("tables/{}/rows", table.index),
                row.index,
                format!(
                    "semantic-table-row:{page_index}:{}:{}",
                    table.index, row.index
                ),
                row.locator.clone(),
            )?;
            let mut row_node = DocumentNode::new(&row_id, DocumentNodeKind::TableRow)
                .with_locator(row.locator.clone())
                .with_ordinal(row.index as usize);
            row_node.extensions.insert(
                "grist.pdf".into(),
                serde_json::to_value(row).map_err(projection_transform_error)?,
            );
            graph.add_node(row_node);
            graph.add_contains(&table_id, &row_id);
            for cell in &row.cells {
                let cell_ordinal = cell
                    .row
                    .saturating_mul(1_000_000)
                    .saturating_add(cell.column);
                let cell_id = id(
                    &format!("tables/{}/cells", table.index),
                    cell_ordinal,
                    format!(
                        "semantic-table-cell:{page_index}:{}:{}:{}",
                        table.index, cell.row, cell.column
                    ),
                    cell.locator.clone(),
                )?;
                let mut cell_node = DocumentNode::new(&cell_id, DocumentNodeKind::TableCell)
                    .with_text(&cell.text)
                    .with_locator(cell.locator.clone())
                    .with_ordinal(cell.column as usize);
                cell_node.extensions.insert(
                    "grist.pdf".into(),
                    serde_json::to_value(cell).map_err(projection_transform_error)?,
                );
                graph.add_node(cell_node);
                graph.add_contains(&row_id, &cell_id);
            }
        }
        if let Some(caption_id) = table
            .caption_block_index
            .and_then(|index| block_ids.get(&index))
        {
            graph.add_edge(
                DocumentEdge::inferred(
                    caption_id.clone(),
                    DocumentRelation::CaptionFor,
                    table_id.clone(),
                    "grist.pdf.caption-proximity.v1",
                    LocatorConfidence::new(table.confidence).map_err(projection_transform_error)?,
                )
                .with_inference_evidence(table.locator.clone()),
            );
        }
        table_ids.insert(table.index, table_id);
    }
    let mut figure_ids = BTreeMap::new();
    for figure in &semantic.figures {
        let figure_id = id(
            "figures",
            figure.index,
            format!("semantic-figure:{page_index}:{}", figure.index),
            figure.locator.clone(),
        )?;
        let mut node = DocumentNode::new(&figure_id, DocumentNodeKind::Figure)
            .with_locator(figure.locator.clone())
            .with_ordinal(figure.index as usize);
        node.extensions.insert(
            "grist.pdf".into(),
            serde_json::to_value(figure).map_err(projection_transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(page_id, &figure_id);
        for graphic_index in &figure.graphic_indices {
            if let Some(graphic) = semantic
                .graphics
                .iter()
                .find(|value| value.index == *graphic_index)
            {
                let graphic_id = id(
                    &format!("figures/{}/graphics", figure.index),
                    graphic.index,
                    format!("semantic-graphic:{page_index}:{}", graphic.index),
                    graphic.locator.clone(),
                )?;
                let mut graphic_node = DocumentNode::new(&graphic_id, DocumentNodeKind::Image)
                    .with_locator(graphic.locator.clone())
                    .with_ordinal(graphic.index as usize);
                graphic_node.extensions.insert(
                    "grist.pdf".into(),
                    serde_json::to_value(graphic).map_err(projection_transform_error)?,
                );
                graph.add_node(graphic_node);
                graph.add_contains(&figure_id, &graphic_id);
            }
        }
        if let Some(caption_id) = figure
            .caption_block_index
            .and_then(|index| block_ids.get(&index))
        {
            graph.add_edge(
                DocumentEdge::inferred(
                    caption_id.clone(),
                    DocumentRelation::CaptionFor,
                    figure_id.clone(),
                    "grist.pdf.caption-proximity.v1",
                    LocatorConfidence::new(figure.confidence)
                        .map_err(projection_transform_error)?,
                )
                .with_inference_evidence(figure.locator.clone()),
            );
        }
        figure_ids.insert(figure.index, figure_id);
    }
    let reading_id = |item: &crate::pdf::PdfSemanticReadingItem| -> Option<&String> {
        match item.kind {
            crate::pdf::PdfSemanticReadingItemKind::Block => block_ids.get(&item.index),
            crate::pdf::PdfSemanticReadingItemKind::Table => table_ids.get(&item.index),
            crate::pdf::PdfSemanticReadingItemKind::Figure => figure_ids.get(&item.index),
        }
    };
    for pair in semantic.reading_order.windows(2) {
        if let (Some(source), Some(target)) = (reading_id(&pair[0]), reading_id(&pair[1])) {
            let confidence = pair[0].confidence.min(pair[1].confidence);
            graph.add_edge(DocumentEdge::inferred(
                source.clone(),
                DocumentRelation::Precedes,
                target.clone(),
                "grist.pdf.semantic-reading-order.v1",
                LocatorConfidence::new(confidence).map_err(projection_transform_error)?,
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "word-ooxml")]
impl ToDocumentGraph for crate::word_ooxml::WordOoxmlDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::WORD_OOXML_V1, "word_ooxml")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::WordOoxml)
            .with_projection(
                "word_ooxml",
                SchemaVersion::WORD_OOXML_V1,
                "grist.word_ooxml.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = context.language;
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| self.package_kind.format_id().to_string()),
        );
        graph.attrs = context.attrs;
        let id = |path: Vec<String>, native: Option<String>, locator: Option<SourceLocator>| {
            identities
                .node_id(&ProjectionAddress {
                    structural_path: path,
                    native_id: native,
                    locator,
                })
                .map_err(projection_transform_error)
        };
        let root_id = id(
            vec!["package".into()],
            Some(self.main_document_part.clone()),
            Some(self.main_document_locator.clone()),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name(&self.main_document_part)
            .with_locator(self.main_document_locator.clone())
            .with_ordinal(0);
        root.extensions.insert(
            "grist.word_ooxml".into(),
            serde_json::json!({
                "package_kind": self.package_kind,
                "package_media_type": self.package_media_type,
                "main_document_part": self.main_document_part,
                "revision_projections": self.revision_graph.projections,
            }),
        );
        graph.add_node(root);

        let macro_parts = self
            .macro_projects
            .iter()
            .map(|project| project.part.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let child_parts = self
            .child_artifacts
            .iter()
            .map(|artifact| artifact.part.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let image_parts = self
            .child_artifacts
            .iter()
            .filter(|artifact| {
                artifact
                    .content_type
                    .as_deref()
                    .is_some_and(|value| value.starts_with("image/"))
            })
            .map(|artifact| artifact.part.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let mut part_ids = std::collections::HashMap::new();
        for (index, part) in self.parts.iter().enumerate() {
            let node_id = id(
                vec!["package".into(), "parts".into(), index.to_string()],
                Some(format!("part:{}", part.path)),
                Some(part.locator.clone()),
            )?;
            let kind = if image_parts.contains(part.path.as_str()) {
                DocumentNodeKind::Image
            } else if macro_parts.contains(part.path.as_str())
                || child_parts.contains(part.path.as_str())
            {
                DocumentNodeKind::Attachment
            } else if part.path == "[Content_Types].xml" || part.path.ends_with(".rels") {
                DocumentNodeKind::Metadata
            } else {
                DocumentNodeKind::ArchiveMember
            };
            let mut node = DocumentNode::new(&node_id, kind)
                .with_name(&part.path)
                .with_locator(part.locator.clone())
                .with_ordinal(index)
                .with_attr(
                    "content_type",
                    part.content_type.clone().map_or(Value::Null, Value::from),
                )
                .with_attr("uncompressed_size", part.uncompressed_size)
                .with_attr("compressed_size", part.compressed_size);
            node.extensions.insert(
                "grist.word_ooxml".into(),
                serde_json::to_value(part).map_err(projection_transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            part_ids.insert(part.path.as_str(), node_id);
        }

        let mut property_ordinal = self.parts.len();
        for property in self
            .properties
            .core
            .iter()
            .chain(self.properties.extended.iter())
        {
            let node_id = id(
                vec![
                    "package".into(),
                    "properties".into(),
                    property_ordinal.to_string(),
                ],
                None,
                Some(property.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Metadata)
                .with_name(&property.name)
                .with_text(&property.value)
                .with_locator(property.locator.clone())
                .with_ordinal(property_ordinal);
            node.extensions.insert(
                "grist.word_ooxml".into(),
                serde_json::to_value(property).map_err(projection_transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            property_ordinal += 1;
        }
        for property in &self.properties.custom {
            let node_id = id(
                vec![
                    "package".into(),
                    "custom_properties".into(),
                    property_ordinal.to_string(),
                ],
                property.name.as_ref().map(|name| format!("custom:{name}")),
                Some(property.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Metadata)
                .with_name(property.name.as_deref().unwrap_or("custom-property"))
                .with_locator(property.locator.clone())
                .with_ordinal(property_ordinal);
            node.text = property.value.clone();
            node.extensions.insert(
                "grist.word_ooxml".into(),
                serde_json::to_value(property).map_err(projection_transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            property_ordinal += 1;
        }

        for relationship in &self.relationships {
            let source = relationship
                .source_part
                .as_deref()
                .and_then(|part| part_ids.get(part))
                .cloned()
                .unwrap_or_else(|| root_id.clone());
            let target = relationship
                .resolved_part
                .as_deref()
                .and_then(|part| part_ids.get(part))
                .cloned()
                .unwrap_or_else(|| relationship.target.clone());
            let lowered = relationship.relationship_type.to_ascii_lowercase();
            let mut edge = if lowered.ends_with("/image")
                || lowered.ends_with("/oleobject")
                || lowered.ends_with("/package")
                || lowered.ends_with("/embeddedobject")
            {
                DocumentEdge::new(target, DocumentRelation::EmbeddedIn, source)
            } else if lowered.ends_with("/hyperlink") {
                DocumentEdge::new(source, DocumentRelation::LinksTo, target)
            } else {
                DocumentEdge::new(source, DocumentRelation::References, target)
            }
            .with_locator(relationship.locator.clone());
            edge.extensions.insert(
                "grist.word_ooxml".into(),
                serde_json::to_value(relationship).map_err(projection_transform_error)?,
            );
            graph.add_edge(edge);
        }
        crate::word_ooxml::graph::project_content(self, &mut graph, &identities, &root_id)
            .map_err(projection_transform_error)?;
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "epub")]
impl ToDocumentGraph for crate::epub::EpubDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::EPUB_V1, "epub")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Epub).with_projection(
            "epub",
            SchemaVersion::EPUB_V1,
            "grist.epub.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.language = context.language;
        graph.dialect = Some(context.dialect.unwrap_or_else(|| match &self.version {
            crate::epub::EpubVersion::Epub2 => "epub2".to_string(),
            crate::epub::EpubVersion::Epub3 => "epub3".to_string(),
            crate::epub::EpubVersion::Unknown(version) => format!("epub-{version}"),
        }));
        graph.attrs = context.attrs;

        let id = |path: Vec<String>, native: Option<String>, locator: Option<SourceLocator>| {
            identities
                .node_id(&ProjectionAddress {
                    structural_path: path,
                    native_id: native,
                    locator,
                })
                .map_err(projection_transform_error)
        };
        let root_id = id(
            vec!["package".into()],
            Some(self.package_path.clone()),
            Some(self.package_locator.clone()),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name(
                self.metadata
                    .iter()
                    .find(|metadata| metadata.name.ends_with("title"))
                    .map(|metadata| metadata.value.clone())
                    .unwrap_or_else(|| self.package_path.clone()),
            )
            .with_locator(self.package_locator.clone())
            .with_ordinal(0);
        root.extensions.insert(
            "grist.epub".into(),
            serde_json::json!({
                "version": self.version,
                "package_path": self.package_path,
                "unique_identifier_id": self.unique_identifier_id,
            }),
        );
        graph.add_node(root);

        for (index, metadata) in self.metadata.iter().enumerate() {
            let node_id = id(
                vec!["package".into(), "metadata".into(), index.to_string()],
                metadata
                    .id
                    .as_ref()
                    .map(|value| format!("metadata:{value}")),
                Some(metadata.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Metadata)
                .with_name(&metadata.name)
                .with_text(&metadata.value)
                .with_locator(metadata.locator.clone())
                .with_ordinal(index + 1);
            node.extensions.insert(
                "grist.epub".into(),
                serde_json::to_value(metadata).map_err(projection_transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
        }

        let mut resource_ids = std::collections::HashMap::new();
        for (index, item) in self.manifest.iter().enumerate() {
            let kind = if item.media_type.starts_with("image/") {
                DocumentNodeKind::Image
            } else if item.media_type == "text/css" {
                DocumentNodeKind::Metadata
            } else if matches!(
                item.media_type.as_str(),
                "application/xhtml+xml" | "text/html"
            ) {
                DocumentNodeKind::ArchiveMember
            } else {
                DocumentNodeKind::Attachment
            };
            let node_id = id(
                vec!["package".into(), "manifest".into(), index.to_string()],
                Some(format!("manifest:{}", item.id)),
                Some(item.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, kind)
                .with_name(&item.href)
                .with_locator(item.locator.clone())
                .with_ordinal(self.metadata.len() + index + 1)
                .with_attr("media_type", item.media_type.clone())
                .with_attr("encrypted", item.encrypted);
            node.extensions.insert(
                "grist.epub".into(),
                serde_json::to_value(item).map_err(projection_transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            resource_ids.insert(item.id.as_str(), node_id);
        }

        let mut chapter_ids = std::collections::HashMap::new();
        let mut html_ids = std::collections::HashMap::new();
        for (chapter_index, chapter) in self.chapters.iter().enumerate() {
            let chapter_id = id(
                vec![
                    "package".into(),
                    "chapters".into(),
                    chapter_index.to_string(),
                ],
                Some(format!("chapter:{}", chapter.manifest_id)),
                Some(chapter.locator.clone()),
            )?;
            let mut chapter_node = DocumentNode::new(&chapter_id, DocumentNodeKind::Section)
                .with_name(
                    chapter
                        .title
                        .clone()
                        .unwrap_or_else(|| chapter.path.clone()),
                )
                .with_locator(chapter.locator.clone())
                .with_ordinal(chapter_index)
                .with_attr("linear", chapter.linear)
                .with_attr(
                    "spine_position",
                    chapter.spine_position.map_or(Value::Null, Value::from),
                );
            chapter_node.extensions.insert(
                "grist.epub".into(),
                serde_json::json!({
                    "manifest_id": chapter.manifest_id,
                    "path": chapter.path,
                    "identity": chapter.identity,
                }),
            );
            graph.add_node(chapter_node);
            graph.add_contains(&root_id, &chapter_id);
            chapter_ids.insert(chapter.path.as_str(), chapter_id.clone());

            for (node_index, source_node) in chapter
                .document
                .nodes
                .iter()
                .filter(|node| node.kind != crate::html::HtmlNodeKind::Document)
                .enumerate()
            {
                let native = format!("{}#{}", chapter.path, source_node.id);
                let node_id = id(
                    vec![
                        "package".into(),
                        "chapters".into(),
                        chapter_index.to_string(),
                        "nodes".into(),
                        node_index.to_string(),
                    ],
                    Some(native),
                    Some(source_node.locator.clone()),
                )?;
                html_ids.insert(
                    (chapter.path.as_str(), source_node.id.as_str()),
                    node_id.clone(),
                );
                let kind = epub_html_node_kind(source_node);
                let mut node = DocumentNode::new(&node_id, kind.clone())
                    .with_locator(source_node.locator.clone())
                    .with_ordinal(node_index);
                node.text = source_node.text.clone();
                node.name = source_node.tag_name.clone();
                if matches!(kind, DocumentNodeKind::Raw | DocumentNodeKind::RawBlock) {
                    node.raw = Some(
                        RawNodeContent::new(
                            "grist.epub",
                            source_node
                                .tag_name
                                .as_deref()
                                .unwrap_or("unknown-html-node"),
                            serde_json::to_value(source_node)
                                .map_err(projection_transform_error)?,
                        )
                        .map_err(projection_transform_error)?,
                    );
                }
                node.extensions.insert(
                    "grist.epub".into(),
                    serde_json::json!({
                        "chapter_path": chapter.path,
                        "html": source_node,
                    }),
                );
                graph.add_node(node);
                let parent_id = source_node
                    .parent_id
                    .as_deref()
                    .and_then(|parent| html_ids.get(&(chapter.path.as_str(), parent)))
                    .cloned()
                    .unwrap_or_else(|| chapter_id.clone());
                graph.add_contains(parent_id, &node_id);
            }
        }

        let ordered = self
            .spine
            .items
            .iter()
            .filter_map(|item| {
                item.resolved_path
                    .as_deref()
                    .and_then(|path| chapter_ids.get(path))
            })
            .collect::<Vec<_>>();
        for pair in ordered.windows(2) {
            graph.add_edge(DocumentEdge::new(
                pair[0],
                DocumentRelation::Precedes,
                pair[1],
            ));
        }

        for (nav_index, navigation) in self.navigation.iter().enumerate() {
            for (entry_index, entry) in navigation.entries.iter().enumerate() {
                let node_id = id(
                    vec![
                        "package".into(),
                        "navigation".into(),
                        nav_index.to_string(),
                        entry_index.to_string(),
                    ],
                    None,
                    Some(entry.locator.clone()),
                )?;
                let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Link)
                    .with_text(&entry.label)
                    .with_locator(entry.locator.clone())
                    .with_ordinal(entry_index)
                    .with_attr("destination", entry.href.clone());
                node.extensions.insert(
                    "grist.epub".into(),
                    serde_json::to_value(entry).map_err(projection_transform_error)?,
                );
                graph.add_node(node);
                graph.add_contains(&root_id, &node_id);
                let target = entry
                    .resolved_path
                    .as_deref()
                    .and_then(|path| chapter_ids.get(path))
                    .cloned()
                    .unwrap_or_else(|| entry.href.clone());
                graph.add_edge(
                    DocumentEdge::new(&node_id, DocumentRelation::LinksTo, target)
                        .with_locator(entry.locator.clone()),
                );
            }
        }

        for note in &self.footnotes {
            let target = self
                .chapters
                .iter()
                .find(|chapter| chapter.path == note.chapter_path)
                .and_then(|chapter| {
                    chapter.document.nodes.iter().find(|node| {
                        node.attributes.iter().any(|attribute| {
                            attribute.local_name == "id"
                                && attribute.value.as_deref() == Some(note.id.as_str())
                        })
                    })
                })
                .and_then(|node| html_ids.get(&(note.chapter_path.as_str(), node.id.as_str())))
                .cloned();
            if let Some(target) = target {
                for reference in &note.references {
                    let reference_node = self
                        .chapters
                        .iter()
                        .find(|chapter| chapter.path == reference.chapter_path)
                        .and_then(|chapter| {
                            chapter
                                .document
                                .nodes
                                .iter()
                                .find(|node| node.locator == reference.locator)
                        })
                        .and_then(|node| {
                            html_ids.get(&(reference.chapter_path.as_str(), node.id.as_str()))
                        });
                    if let Some(reference_node) = reference_node {
                        graph.add_edge(
                            DocumentEdge::new(
                                &target,
                                DocumentRelation::FootnoteFor,
                                reference_node,
                            )
                            .with_locator(reference.locator.clone()),
                        );
                    }
                }
            }
        }

        for image in &self.images {
            let Some(image_id) = resource_ids.get(image.manifest_id.as_str()) else {
                continue;
            };
            for usage in &image.usages {
                if let Some(chapter_id) = chapter_ids.get(usage.chapter_path.as_str()) {
                    graph.add_edge(
                        DocumentEdge::new(image_id, DocumentRelation::EmbeddedIn, chapter_id)
                            .with_locator(usage.locator.clone()),
                    );
                }
            }
        }
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "epub")]
fn epub_html_node_kind(node: &crate::html::HtmlNode) -> DocumentNodeKind {
    use crate::html::HtmlNodeKind;
    if node.attributes.iter().any(|attribute| {
        matches!(attribute.name.as_str(), "epub:type" | "role")
            && attribute.value.as_deref().is_some_and(|value| {
                value.split_ascii_whitespace().any(|token| {
                    matches!(
                        token,
                        "footnote" | "endnote" | "doc-footnote" | "doc-endnote"
                    )
                })
            })
    }) {
        return DocumentNodeKind::Footnote;
    }
    match node.kind {
        HtmlNodeKind::Text => DocumentNodeKind::Text,
        HtmlNodeKind::Comment => DocumentNodeKind::Comment,
        HtmlNodeKind::Doctype => DocumentNodeKind::Metadata,
        HtmlNodeKind::ProcessingInstruction | HtmlNodeKind::RawUnknown => DocumentNodeKind::Raw,
        HtmlNodeKind::Document => DocumentNodeKind::Document,
        HtmlNodeKind::Element => match node.tag_name.as_deref().unwrap_or_default() {
            "section" | "article" | "nav" | "main" | "aside" => DocumentNodeKind::Section,
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => DocumentNodeKind::Heading,
            "p" => DocumentNodeKind::Paragraph,
            "a" => DocumentNodeKind::Link,
            "ul" | "ol" => DocumentNodeKind::List,
            "li" => DocumentNodeKind::ListItem,
            "table" => DocumentNodeKind::Table,
            "tr" => DocumentNodeKind::TableRow,
            "td" | "th" => DocumentNodeKind::TableCell,
            "figure" => DocumentNodeKind::Figure,
            "figcaption" | "caption" => DocumentNodeKind::Caption,
            "img" | "picture" | "svg" => DocumentNodeKind::Image,
            "pre" => DocumentNodeKind::CodeBlock,
            "code" => DocumentNodeKind::InlineCode,
            _ if !node.known_element => DocumentNodeKind::RawBlock,
            _ => DocumentNodeKind::Span,
        },
    }
}

#[cfg(feature = "xml")]
impl ToDocumentGraph for crate::xml::XmlDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::XML_V1, "xml")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Xml).with_projection(
            "xml",
            SchemaVersion::XML_V1,
            "grist.xml.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.language = Some(context.language.unwrap_or_else(|| "xml".into()));
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| format!("{:?}", self.dialect).to_ascii_lowercase()),
        );
        graph.attrs = context.attrs;
        graph
            .attrs
            .insert("well_formed".into(), Value::Bool(self.well_formed));
        let root_id = stable_projection_node_id(
            &identities,
            vec!["document".into()],
            Some("root"),
            Some(&self.decoded_range),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_range(self.decoded_range.clone())
            .with_locator(self.locator.clone())
            .with_ordinal(0);
        root.extensions.insert("grist.xml".into(), serde_json::json!({"dialect":self.dialect,"declaration":self.declaration,"root_element_ids":self.root_element_ids}));
        graph.add_node(root);
        let native_id_counts = self.nodes.iter().filter_map(xml_native_id).fold(
            std::collections::HashMap::<&str, usize>::new(),
            |mut counts, native_id| {
                *counts.entry(native_id).or_default() += 1;
                counts
            },
        );
        let projected = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let native_id = xml_native_id(n);
                let structural_path = if native_id
                    .and_then(|id| native_id_counts.get(id))
                    .is_some_and(|count| *count == 1)
                {
                    vec![
                        "document".into(),
                        "xml-id".into(),
                        native_id.unwrap().into(),
                    ]
                } else {
                    vec!["document".into(), "nodes".into(), i.to_string()]
                };
                stable_projection_node_id(
                    &identities,
                    structural_path,
                    native_id.or(Some(n.xml_path.as_str())),
                    Some(&n.range),
                )
                .map(|id| (n.id.clone(), id))
            })
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        for (ordinal, source) in self.nodes.iter().enumerate() {
            let id = projected[&source.id].clone();
            let mut node = DocumentNode::new(&id, xml_document_node_kind(source, self.dialect))
                .with_range(source.range.clone())
                .with_locator(source.locator.clone())
                .with_ordinal(ordinal + 1)
                .with_attr("xml_path", source.xml_path.clone());
            node.text = source.text.clone();
            node.name = source.qualified_name.clone();
            if let Some(xml_id) = xml_native_id(source) {
                node.attrs
                    .insert("xml_id".into(), Value::String(xml_id.to_string()));
            }
            if let Some(links) = &self.scholarly_links
                && let Some(target) = links
                    .targets
                    .iter()
                    .find(|target| target.node_id == source.id)
            {
                node.attrs.insert(
                    "jats_target_kind".into(),
                    serde_json::to_value(&target.kind).map_err(projection_transform_error)?,
                );
                if let Some(label) = &target.label {
                    node.attrs
                        .insert("label".into(), Value::String(label.clone()));
                }
            }
            node.extensions.insert(
                "grist.xml".into(),
                serde_json::to_value(source).map_err(projection_transform_error)?,
            );
            if let Some(a) = source.attributes.iter().find(|a| {
                a.local_name == "href"
                    || (self.dialect != crate::xml::XmlDialect::Jats && a.local_name == "rid")
            }) {
                node.attrs
                    .insert("destination".into(), Value::String(a.value.clone()));
                let relation = if a.local_name == "rid" {
                    DocumentRelation::References
                } else {
                    DocumentRelation::LinksTo
                };
                graph.add_edge(
                    DocumentEdge::new(&id, relation, a.value.clone())
                        .with_locator(a.locator.clone()),
                );
            }
            if source.kind == crate::xml::XmlNodeKind::RawUnknown
                || (self.dialect == crate::xml::XmlDialect::Jats
                    && source.kind == crate::xml::XmlNodeKind::Element
                    && !source.known_jats_element)
            {
                node.raw = Some(
                    RawNodeContent::new(
                        "grist.xml",
                        source.qualified_name.as_deref().unwrap_or("unknown"),
                        serde_json::json!({"xml":source.raw}),
                    )
                    .map_err(projection_transform_error)?,
                );
            }
            graph.add_node(node);
            let parent = source
                .parent_id
                .as_ref()
                .and_then(|p| projected.get(p))
                .map(String::as_str)
                .unwrap_or(root_id.as_str());
            graph.add_contains(parent, &id);
        }
        if let Some(links) = &self.scholarly_links {
            for relationship in &links.relationships {
                let source = projected[&relationship.source_node_id].clone();
                let target = relationship
                    .target_node_id
                    .as_ref()
                    .and_then(|node_id| projected.get(node_id))
                    .cloned()
                    .or_else(|| relationship.target_xml_id.clone())
                    .unwrap_or_else(|| relationship.id.clone());
                let relation = if relationship.kind == crate::jats::JatsRelationshipKind::Citation {
                    DocumentRelation::Cites
                } else {
                    DocumentRelation::References
                };
                let mut edge = DocumentEdge::explicit(
                    &source,
                    relation,
                    &target,
                    relationship.locator.clone(),
                )
                .with_attr("jats_relationship_id", relationship.id.clone())
                .with_attr(
                    "resolution",
                    serde_json::to_value(relationship.resolution)
                        .map_err(projection_transform_error)?,
                )
                .with_attr("raw_rid", relationship.raw_rid.clone());
                if let Some(target_xml_id) = &relationship.target_xml_id {
                    edge = edge.with_attr("target_xml_id", target_xml_id.clone());
                }
                if let Some(ref_type) = &relationship.ref_type {
                    edge = edge.with_attr("ref_type", ref_type.clone());
                }
                if let Some(target_kind) = &relationship.target_kind {
                    edge = edge.with_attr(
                        "target_kind",
                        serde_json::to_value(target_kind).map_err(projection_transform_error)?,
                    );
                }
                if !relationship.candidate_node_ids.is_empty() {
                    let candidates = relationship
                        .candidate_node_ids
                        .iter()
                        .filter_map(|node_id| projected.get(node_id))
                        .cloned()
                        .collect::<Vec<_>>();
                    edge = edge.with_attr("candidate_node_ids", serde_json::json!(candidates));
                }
                graph.add_edge(edge);
            }
            for label in &links.labels {
                let Some(owner) = label
                    .owner_node_id
                    .as_ref()
                    .and_then(|node_id| projected.get(node_id))
                else {
                    continue;
                };
                graph.add_edge(
                    DocumentEdge::explicit(
                        &projected[&label.node_id],
                        DocumentRelation::Defines,
                        owner,
                        label.locator.clone(),
                    )
                    .with_attr("label", label.value.clone()),
                );
            }
            for source in self
                .nodes
                .iter()
                .filter(|node| node.local_name.as_deref() == Some("caption"))
            {
                let Some(parent) = source
                    .parent_id
                    .as_ref()
                    .and_then(|node_id| projected.get(node_id))
                else {
                    continue;
                };
                graph.add_edge(DocumentEdge::explicit(
                    &projected[&source.id],
                    DocumentRelation::CaptionFor,
                    parent,
                    source.locator.clone(),
                ));
            }
        }
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}
#[cfg(feature = "xml")]
fn xml_document_node_kind(
    node: &crate::xml::XmlNode,
    dialect: crate::xml::XmlDialect,
) -> DocumentNodeKind {
    use crate::xml::XmlNodeKind as X;
    match node.kind {
        X::Text | X::Cdata => DocumentNodeKind::Text,
        X::Comment => DocumentNodeKind::Comment,
        X::ProcessingInstruction | X::Doctype => DocumentNodeKind::Metadata,
        X::EntityReference => DocumentNodeKind::RawInline,
        X::RawUnknown => DocumentNodeKind::RawBlock,
        X::Element => match node.local_name.as_deref().unwrap_or("") {
            "front" | "body" | "back" | "sec" | "abstract" | "ref-list" => {
                DocumentNodeKind::Section
            }
            "title" | "article-title" | "subtitle" => DocumentNodeKind::Heading,
            "p" => DocumentNodeKind::Paragraph,
            "xref" => {
                if xml_attribute(node, "ref-type")
                    .is_some_and(|value| value.eq_ignore_ascii_case("bibr"))
                {
                    DocumentNodeKind::Citation
                } else {
                    DocumentNodeKind::Reference
                }
            }
            "ext-link" | "self-uri" => DocumentNodeKind::Link,
            "list" => DocumentNodeKind::List,
            "list-item" => DocumentNodeKind::ListItem,
            "table" | "table-wrap" | "table-wrap-group" => DocumentNodeKind::Table,
            "tr" => DocumentNodeKind::TableRow,
            "td" | "th" => DocumentNodeKind::TableCell,
            "fig" | "fig-group" => DocumentNodeKind::Figure,
            "caption" => DocumentNodeKind::Caption,
            "graphic" | "inline-graphic" => DocumentNodeKind::Image,
            "media" => DocumentNodeKind::MediaTrack,
            "supplementary-material" => DocumentNodeKind::Attachment,
            "ref" | "mixed-citation" | "element-citation" => DocumentNodeKind::BibliographyEntry,
            "fn" | "table-wrap-foot" => DocumentNodeKind::Footnote,
            "label" => DocumentNodeKind::Label,
            "article-meta" | "journal-meta" | "article-id" | "contrib" | "aff" | "corresp"
            | "author-notes" | "funding-group" | "award-group" | "pub-date" => {
                DocumentNodeKind::Metadata
            }
            _ if dialect == crate::xml::XmlDialect::Jats && !node.known_jats_element => {
                DocumentNodeKind::RawBlock
            }
            _ => DocumentNodeKind::Span,
        },
    }
}

#[cfg(feature = "xml")]
fn xml_attribute<'a>(node: &'a crate::xml::XmlNode, local_name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|attribute| attribute.local_name == local_name)
        .map(|attribute| attribute.value.as_str())
}

#[cfg(feature = "xml")]
fn xml_native_id(node: &crate::xml::XmlNode) -> Option<&str> {
    node.attributes
        .iter()
        .find(|attribute| {
            attribute.qualified_name == "id"
                || (attribute.prefix.as_deref() == Some("xml") && attribute.local_name == "id")
        })
        .map(|attribute| attribute.value.as_str())
}
#[cfg(feature = "asciidoc")]
impl ToDocumentGraph for crate::asciidoc::AsciiDocDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::ASCIIDOC_V1, "asciidoc")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::AsciiDoc)
            .with_projection(
                "asciidoc",
                SchemaVersion::ASCIIDOC_V1,
                "grist.asciidoc.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = Some(context.language.unwrap_or_else(|| "asciidoc".to_string()));
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        let root_id = stable_projection_node_id(
            &identities,
            vec!["document".to_string()],
            Some("root"),
            None,
        )?;
        graph.add_node(DocumentNode::new(&root_id, DocumentNodeKind::Document).with_ordinal(0));
        project_adoc_nodes(
            &self.nodes,
            &root_id,
            &["document".to_string(), "nodes".to_string()],
            &identities,
            &mut graph,
        )?;
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}
#[cfg(feature = "asciidoc")]
fn project_adoc_nodes(
    nodes: &[crate::asciidoc::AsciiDocNode],
    owner: &str,
    path: &[String],
    identities: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let projected = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let mut node_path = path.to_vec();
            node_path.push(index.to_string());
            stable_projection_node_id(
                identities,
                node_path,
                Some(node.id.as_str()),
                Some(&node.range),
            )
            .map(|id| (node.id.clone(), id))
        })
        .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
    let definitions = nodes
        .iter()
        .filter_map(|node| Some((node.name.clone()?, projected.get(&node.id)?.clone())))
        .collect::<std::collections::HashMap<_, _>>();
    for (index, source_node) in nodes.iter().enumerate() {
        let node_id = projected[&source_node.id].clone();
        let mut node = DocumentNode::new(&node_id, adoc_document_node_kind(&source_node.kind))
            .with_range(source_node.range.clone())
            .with_ordinal(index.saturating_add(1))
            .with_attr(
                "asciidoc_kind",
                format!("{:?}", source_node.kind).to_ascii_lowercase(),
            );
        node.text = source_node.text.clone();
        insert_adoc_opt_attr(&mut node.attrs, "level", source_node.level.map(Value::from));
        insert_adoc_opt_attr(
            &mut node.attrs,
            "name",
            source_node.name.clone().map(Value::from),
        );
        insert_adoc_opt_attr(
            &mut node.attrs,
            "argument",
            source_node.argument.clone().map(Value::from),
        );
        insert_adoc_opt_attr(
            &mut node.attrs,
            "target",
            source_node.target.clone().map(Value::from),
        );
        insert_adoc_opt_attr(
            &mut node.attrs,
            "role",
            source_node.role.clone().map(Value::from),
        );
        node.extensions.insert(
            "grist.asciidoc".to_string(),
            serde_json::to_value(source_node).map_err(projection_transform_error)?,
        );
        if adoc_retains_raw(&source_node.kind, source_node.known_syntax) {
            node.raw = Some(
                RawNodeContent::new(
                    "grist.asciidoc",
                    format!("{:?}", source_node.kind).to_ascii_lowercase(),
                    serde_json::json!({"syntax": source_node.raw}),
                )
                .map_err(projection_transform_error)?,
            );
        }
        graph.add_node(node);
        let parent = source_node
            .parent_id
            .as_ref()
            .and_then(|id| projected.get(id))
            .map(String::as_str)
            .unwrap_or(owner);
        graph.add_contains(parent, &node_id);
        project_adoc_relations(source_node, &node_id, &definitions, graph);
        project_adoc_include(source_node, &node_id, path, index, identities, graph)?;
    }
    Ok(())
}

#[cfg(feature = "asciidoc")]
fn project_adoc_relations(
    node: &crate::asciidoc::AsciiDocNode,
    node_id: &str,
    definitions: &std::collections::HashMap<String, String>,
    graph: &mut DocumentGraph,
) {
    use crate::asciidoc::AsciiDocNodeKind as Adoc;
    if let Some(target) = &node.target {
        graph.add_edge(
            DocumentEdge::new(node_id, DocumentRelation::LinksTo, target)
                .with_range(node.range.clone())
                .with_attr("target_kind", "uri_or_label"),
        );
    }
    if matches!(node.kind, Adoc::CrossReference | Adoc::FootnoteReference)
        && let Some(label) = &node.name
        && let Some(target) = definitions.get(label)
    {
        graph.add_edge(
            DocumentEdge::new(node_id, DocumentRelation::References, target)
                .with_range(node.range.clone())
                .with_attr("label", label.clone()),
        );
    }
}

#[cfg(feature = "asciidoc")]
fn project_adoc_include(
    node: &crate::asciidoc::AsciiDocNode,
    node_id: &str,
    path: &[String],
    index: usize,
    identities: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let Some(include) = &node.include else {
        return Ok(());
    };
    let Some(resolved) = &include.resolved else {
        graph.add_edge(
            DocumentEdge::new(
                node_id,
                DocumentRelation::References,
                include.target.as_str(),
            )
            .with_range(node.range.clone())
            .with_attr(
                "include_status",
                format!("{:?}", include.status).to_ascii_lowercase(),
            ),
        );
        return Ok(());
    };
    let mut child_path = path.to_vec();
    child_path.extend([index.to_string(), "resolved_include".to_string()]);
    let document_id = stable_projection_node_id(
        identities,
        child_path.clone(),
        include.resolved_path.as_deref(),
        None,
    )?;
    let mut document = DocumentNode::new(&document_id, DocumentNodeKind::Document)
        .with_ordinal(index.saturating_add(1))
        .with_attr(
            "repository_relative_path",
            include.resolved_path.clone().unwrap_or_default(),
        );
    document.extensions.insert(
        "grist.asciidoc".to_string(),
        serde_json::json!({
            "include_target": include.target,
            "source": resolved.source,
            "content_sha256": resolved.content_sha256,
        }),
    );
    graph.add_node(document);
    graph.add_contains(node_id, &document_id);
    graph.add_edge(
        DocumentEdge::new(node_id, DocumentRelation::ResolvesTo, &document_id)
            .with_range(node.range.clone())
            .with_attr("include_status", "resolved"),
    );
    child_path.push("nodes".to_string());
    project_adoc_nodes(
        &resolved.nodes,
        &document_id,
        &child_path,
        identities,
        graph,
    )
}

#[cfg(feature = "asciidoc")]
fn adoc_retains_raw(kind: &crate::asciidoc::AsciiDocNodeKind, _known: bool) -> bool {
    use crate::asciidoc::AsciiDocNodeKind as Adoc;
    matches!(
        kind,
        Adoc::Attribute
            | Adoc::BlockAttribute
            | Adoc::Directive
            | Adoc::Role
            | Adoc::Comment
            | Adoc::RawBlock
            | Adoc::RawInline
    )
}

#[cfg(feature = "asciidoc")]
fn adoc_document_node_kind(kind: &crate::asciidoc::AsciiDocNodeKind) -> DocumentNodeKind {
    use crate::asciidoc::AsciiDocNodeKind as Adoc;
    match kind {
        Adoc::Heading => DocumentNodeKind::Heading,
        Adoc::Attribute | Adoc::BlockAttribute => DocumentNodeKind::Metadata,
        Adoc::Paragraph => DocumentNodeKind::Paragraph,
        Adoc::Include => DocumentNodeKind::Reference,
        Adoc::CodeBlock | Adoc::LiteralBlock => DocumentNodeKind::CodeBlock,
        Adoc::Table => DocumentNodeKind::Table,
        Adoc::TableRow => DocumentNodeKind::TableRow,
        Adoc::TableCell => DocumentNodeKind::TableCell,
        Adoc::FootnoteDefinition => DocumentNodeKind::Footnote,
        Adoc::FootnoteReference | Adoc::CrossReference => DocumentNodeKind::Reference,
        Adoc::Target => DocumentNodeKind::Label,
        Adoc::Hyperlink => DocumentNodeKind::Link,
        Adoc::List => DocumentNodeKind::List,
        Adoc::ListItem => DocumentNodeKind::ListItem,
        Adoc::Emphasis => DocumentNodeKind::Emphasis,
        Adoc::Strong => DocumentNodeKind::Strong,
        Adoc::InlineCode => DocumentNodeKind::InlineCode,
        Adoc::Transition => DocumentNodeKind::Span,
        Adoc::Directive | Adoc::Comment | Adoc::RawBlock => DocumentNodeKind::RawBlock,
        Adoc::Role | Adoc::RawInline => DocumentNodeKind::RawInline,
    }
}

#[cfg(feature = "asciidoc")]
fn insert_adoc_opt_attr(attrs: &mut AttrMap, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        attrs.insert(key.to_string(), value);
    }
}

#[cfg(feature = "markdown")]
impl ToDocumentGraph for crate::markdown::MarkdownDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::MARKDOWN_V2, "markdown")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Markdown)
            .with_projection(
                "markdown",
                SchemaVersion::MARKDOWN_V2,
                "grist.markdown.to-document-graph.v3",
            );
        graph.source = context.source;
        graph.language = Some(context.language.unwrap_or_else(|| "markdown".to_string()));
        graph.dialect = context
            .dialect
            .or_else(|| Some(self.dialect.as_str().to_string()));
        graph.attrs = context.attrs;

        let root_id = stable_projection_node_id(
            &identities,
            vec!["document".to_string()],
            Some("root"),
            None,
        )?;
        graph.add_node(DocumentNode::new(&root_id, DocumentNodeKind::Document).with_ordinal(0));

        let mut ordinal = 1_usize;
        if let Some(frontmatter) = &self.frontmatter {
            let node_id = stable_projection_node_id(
                &identities,
                vec!["document".to_string(), "frontmatter".to_string()],
                Some("frontmatter"),
                Some(&frontmatter.range),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Frontmatter)
                .with_range(frontmatter.range.clone())
                .with_text(frontmatter.raw.clone())
                .with_ordinal(ordinal)
                .with_attr(
                    "kind",
                    format!("{:?}", frontmatter.kind).to_ascii_lowercase(),
                )
                .with_attr("delimiter", frontmatter.delimiter.clone());
            if let Some(value) = &frontmatter.value {
                node.attrs.insert("value".to_string(), value.clone());
            }
            node.extensions.insert(
                "grist.markdown".to_string(),
                serde_json::to_value(frontmatter).map_err(projection_transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            ordinal += 1;
        }

        let projected_ids = self
            .nodes
            .iter()
            .enumerate()
            .map(|(node_index, md_node)| {
                stable_projection_node_id(
                    &identities,
                    vec![
                        "document".to_string(),
                        "nodes".to_string(),
                        node_index.to_string(),
                    ],
                    Some(md_node.id.as_str()),
                    md_node.range.as_ref(),
                )
                .map(|id| (md_node.id.clone(), id))
            })
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        let footnotes = self
            .nodes
            .iter()
            .filter(|node| node.kind == crate::markdown::MarkdownNodeKind::FootnoteDefinition)
            .filter_map(|node| Some((node.label.clone()?, projected_ids.get(&node.id)?.clone())))
            .collect::<std::collections::HashMap<_, _>>();

        for md_node in &self.nodes {
            let node_id = projected_ids[&md_node.id].clone();
            let kind = markdown_document_node_kind(&md_node.kind);
            let mut node = DocumentNode::new(&node_id, kind)
                .with_ordinal(ordinal)
                .with_attr(
                    "markdown_kind",
                    format!("{:?}", md_node.kind).to_ascii_lowercase(),
                );
            if let Some(range) = md_node.range.clone() {
                node = node.with_range(range);
            }
            node.text = md_node.text.clone();
            insert_opt_attr(&mut node.attrs, "level", md_node.level.map(Value::from));
            insert_opt_attr(
                &mut node.attrs,
                "language",
                md_node.language.clone().map(Value::from),
            );
            insert_opt_attr(
                &mut node.attrs,
                "info",
                md_node.info.clone().map(Value::from),
            );
            insert_opt_attr(
                &mut node.attrs,
                "destination",
                md_node.destination.clone().map(Value::from),
            );
            insert_opt_attr(
                &mut node.attrs,
                "title",
                md_node.title.clone().map(Value::from),
            );
            insert_opt_attr(
                &mut node.attrs,
                "label",
                md_node.label.clone().map(Value::from),
            );
            insert_opt_attr(
                &mut node.attrs,
                "reference_kind",
                md_node.reference_kind.clone().map(Value::from),
            );
            insert_opt_attr(&mut node.attrs, "ordered", md_node.ordered.map(Value::from));
            insert_opt_attr(
                &mut node.attrs,
                "start_number",
                md_node.start_number.map(Value::from),
            );
            insert_opt_attr(
                &mut node.attrs,
                "item_number",
                md_node.item_number.map(Value::from),
            );
            insert_opt_attr(&mut node.attrs, "checked", md_node.checked.map(Value::from));
            if !md_node.classes.is_empty() {
                node.attrs
                    .insert("classes".to_string(), serde_json::json!(md_node.classes));
            }
            if !md_node.attributes.is_empty() {
                node.attrs.insert(
                    "attributes".to_string(),
                    serde_json::to_value(&md_node.attributes)
                        .map_err(projection_transform_error)?,
                );
            }
            if let Some(table) = &md_node.table {
                if let Ok(value) = serde_json::to_value(table) {
                    node.attrs.insert("table".to_string(), value);
                }
            }
            if let Some(executable) = &md_node.executable {
                node.attrs.insert(
                    "executable".to_string(),
                    serde_json::to_value(executable).map_err(projection_transform_error)?,
                );
                node.attrs
                    .insert("executed".to_string(), Value::Bool(executable.executed));
            }
            if let Some(citation) = &md_node.citation {
                node.attrs.insert(
                    "citation".to_string(),
                    serde_json::to_value(citation).map_err(projection_transform_error)?,
                );
            }
            if let Some(figure) = &md_node.figure {
                node.attrs.insert(
                    "figure".to_string(),
                    serde_json::to_value(figure).map_err(projection_transform_error)?,
                );
            }
            if let Some(reference) = &md_node.local_reference {
                node.attrs.insert(
                    "local_reference".to_string(),
                    serde_json::to_value(reference).map_err(projection_transform_error)?,
                );
            }
            if let Some(output) = &md_node.stored_output {
                node.attrs.insert(
                    "stored_output".to_string(),
                    serde_json::to_value(output).map_err(projection_transform_error)?,
                );
            }
            node.extensions.insert(
                "grist.markdown".to_string(),
                serde_json::to_value(md_node).map_err(projection_transform_error)?,
            );
            if md_node.executable.is_some()
                || matches!(
                    md_node.kind,
                    crate::markdown::MarkdownNodeKind::HtmlBlock
                        | crate::markdown::MarkdownNodeKind::HtmlInline
                        | crate::markdown::MarkdownNodeKind::DirectiveBlock
                        | crate::markdown::MarkdownNodeKind::ExtensionInline
                        | crate::markdown::MarkdownNodeKind::StoredOutput
                        | crate::markdown::MarkdownNodeKind::RawBlock
                        | crate::markdown::MarkdownNodeKind::RawInline
                )
            {
                node.raw = Some(
                    RawNodeContent::new(
                        "grist.markdown",
                        format!("{:?}", md_node.kind).to_ascii_lowercase(),
                        serde_json::json!({"syntax": md_node.raw}),
                    )
                    .map_err(projection_transform_error)?,
                );
            }
            graph.add_node(node);
            let parent = md_node
                .parent_id
                .as_ref()
                .and_then(|id| projected_ids.get(id))
                .unwrap_or(&root_id);
            graph.add_contains(parent, &node_id);

            if let Some(destination) = &md_node.destination {
                let range = md_node.range.clone().ok_or_else(|| {
                    TransformError::MissingRequiredAttribute {
                        target: node_id.clone(),
                        attr: "source locator for explicit link".to_string(),
                    }
                })?;
                graph.add_edge(
                    DocumentEdge::new(&node_id, DocumentRelation::LinksTo, destination)
                        .with_range(range)
                        .with_attr("target_kind", "uri"),
                );
            }
            if md_node.kind == crate::markdown::MarkdownNodeKind::FootnoteReference
                && let Some(label) = &md_node.label
                && let Some(target) = footnotes.get(label)
            {
                let mut edge = DocumentEdge::new(&node_id, DocumentRelation::References, target);
                if let Some(range) = md_node.range.clone() {
                    edge = edge.with_range(range);
                }
                edge.attrs
                    .insert("label".to_string(), Value::from(label.clone()));
                graph.add_edge(edge);
            }
            if let Some(citation) = &md_node.citation {
                for key in &citation.keys {
                    let mut edge = DocumentEdge::new(
                        &node_id,
                        DocumentRelation::References,
                        format!("citation:{key}"),
                    );
                    if let Some(range) = md_node.range.clone() {
                        edge = edge.with_range(range);
                    }
                    edge.attrs
                        .insert("citation_key".to_string(), Value::from(key.clone()));
                    graph.add_edge(edge);
                }
            }
            ordinal += 1;
        }

        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "markdown")]
fn markdown_document_node_kind(kind: &crate::markdown::MarkdownNodeKind) -> DocumentNodeKind {
    use crate::markdown::MarkdownNodeKind as Markdown;
    match kind {
        Markdown::Heading => DocumentNodeKind::Heading,
        Markdown::Paragraph => DocumentNodeKind::Paragraph,
        Markdown::BlockQuote => DocumentNodeKind::Quote,
        Markdown::CodeFence | Markdown::IndentedCode => DocumentNodeKind::CodeBlock,
        Markdown::List | Markdown::DefinitionList => DocumentNodeKind::List,
        Markdown::ListItem | Markdown::DefinitionTerm | Markdown::DefinitionDescription => {
            DocumentNodeKind::ListItem
        }
        Markdown::TaskListMarker
        | Markdown::ThematicBreak
        | Markdown::SoftBreak
        | Markdown::HardBreak => DocumentNodeKind::Span,
        Markdown::Link => DocumentNodeKind::Link,
        Markdown::Image => DocumentNodeKind::Image,
        Markdown::Table => DocumentNodeKind::Table,
        Markdown::TableRow => DocumentNodeKind::TableRow,
        Markdown::TableCell => DocumentNodeKind::TableCell,
        Markdown::FootnoteDefinition => DocumentNodeKind::Footnote,
        Markdown::FootnoteReference | Markdown::Include => DocumentNodeKind::Reference,
        Markdown::Citation => DocumentNodeKind::Citation,
        Markdown::Figure => DocumentNodeKind::Figure,
        Markdown::StoredOutput => DocumentNodeKind::RawBlock,
        Markdown::Emphasis => DocumentNodeKind::Emphasis,
        Markdown::Strong => DocumentNodeKind::Strong,
        Markdown::Strikethrough => DocumentNodeKind::Span,
        Markdown::InlineCode => DocumentNodeKind::InlineCode,
        Markdown::InlineMath => DocumentNodeKind::MathInline,
        Markdown::DisplayMath => DocumentNodeKind::MathBlock,
        Markdown::HtmlBlock | Markdown::DirectiveBlock | Markdown::RawBlock => {
            DocumentNodeKind::RawBlock
        }
        Markdown::HtmlInline | Markdown::ExtensionInline | Markdown::RawInline => {
            DocumentNodeKind::RawInline
        }
        Markdown::Text => DocumentNodeKind::Span,
    }
}

#[cfg(feature = "markdown")]
fn insert_opt_attr(attrs: &mut AttrMap, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        attrs.insert(key.to_string(), value);
    }
}

/// Extract explicit conditional obligations from prose nodes in-place.
///
/// This deterministic pass intentionally handles only clear patterns such as
/// "if/when/unless <condition>, <subject> must/shall/should/may <action>".
/// Ambiguous prose is left untouched rather than guessed.
pub fn extract_conditional_obligations(graph: &mut DocumentGraph) -> usize {
    let candidates = graph
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.kind,
                DocumentNodeKind::Paragraph
                    | DocumentNodeKind::Text
                    | DocumentNodeKind::Requirement
            )
        })
        .filter_map(|node| {
            node.text
                .as_ref()
                .map(|text| (node.id.clone(), text.clone(), node.locator.clone()))
        })
        .collect::<Vec<_>>();

    let mut extracted = 0_usize;
    for (source_id, text, source_locator) in candidates {
        let Some(parsed) = parse_conditional_obligation(&text) else {
            continue;
        };
        let condition_id = format!("{}:condition:{}", source_id, extracted);
        let obligation_id = format!("{}:obligation:{}", source_id, extracted);
        let required_state_id = format!("{}:required-state:{}", source_id, extracted);

        let mut condition_node = DocumentNode::new(&condition_id, DocumentNodeKind::Condition)
            .with_text(parsed.condition.clone())
            .with_attr("connector", parsed.connector.clone());
        condition_node.locator = derived_locator(
            source_locator.as_ref(),
            &source_id,
            "deterministic-if-modal-v1:condition",
        );
        graph.add_node(condition_node);
        let attrs = ObligationAttrs {
            modality: parsed.modality.clone(),
            polarity: parsed.polarity.clone(),
            subject: parsed.subject.clone(),
            predicate: parsed.predicate.clone(),
            action: Some(parsed.action.clone()),
            source_text: Some(text.clone()),
            extraction_method: Some("deterministic-if-modal-v1".to_string()),
            confidence: Some(1.0),
            attrs: AttrMap::new(),
        };
        let mut obligation_node = DocumentNode::new(&obligation_id, DocumentNodeKind::Obligation)
            .with_text(parsed.action.clone())
            .with_attr("obligation", serde_json::to_value(&attrs).unwrap());
        obligation_node.locator = derived_locator(
            source_locator.as_ref(),
            &source_id,
            "deterministic-if-modal-v1:obligation",
        );
        graph.add_node(obligation_node);
        let mut required_state_node =
            DocumentNode::new(&required_state_id, DocumentNodeKind::Requirement)
                .with_text(parsed.action.clone());
        required_state_node.locator = derived_locator(
            source_locator.as_ref(),
            &source_id,
            "deterministic-if-modal-v1:required-state",
        );
        graph.add_node(required_state_node);
        graph.add_edge(inferred_edge_from_source(
            &obligation_id,
            DocumentRelation::ConditionalOn,
            &condition_id,
            "deterministic-if-modal-v1:conditional-on",
            source_locator.as_ref(),
        ));
        graph.add_edge(inferred_edge_from_source(
            &obligation_id,
            relation_for_obligation(&parsed.modality, &parsed.polarity),
            &required_state_id,
            "deterministic-if-modal-v1:modality",
            source_locator.as_ref(),
        ));
        graph.add_edge(inferred_edge_from_source(
            &obligation_id,
            DocumentRelation::DerivedFrom,
            &source_id,
            "deterministic-if-modal-v1:derivation",
            source_locator.as_ref(),
        ));
        graph.add_edge(inferred_edge_from_source(
            &source_id,
            DocumentRelation::EvidenceFor,
            &obligation_id,
            "deterministic-if-modal-v1:evidence",
            source_locator.as_ref(),
        ));
        extracted += 1;
    }
    extracted
}

fn derived_locator(
    source_locator: Option<&SourceLocator>,
    source_node_id: &str,
    derivation_step: &str,
) -> Option<SourceLocator> {
    let source_locator = source_locator?;
    let reference = DerivedNodeReference::new(
        vec![source_node_id.to_string()],
        vec![derivation_step.to_string()],
    )
    .ok()?;
    SourceLocator::new(
        source_locator.components().to_vec(),
        LocatorPrecision::Synthetic {
            derived_from: reference,
            confidence: Some(confidence_one()),
        },
    )
    .ok()
}

fn inferred_edge_from_source(
    source: impl Into<String>,
    relation: DocumentRelation,
    target: impl Into<String>,
    rule: &str,
    source_locator: Option<&SourceLocator>,
) -> DocumentEdge {
    let edge = DocumentEdge::inferred(source, relation, target, rule, confidence_one());
    if let Some(locator) = source_locator {
        edge.with_inference_evidence(locator.clone())
    } else {
        edge
    }
}

#[derive(Debug, Clone)]
struct ParsedConditionalObligation {
    connector: String,
    condition: String,
    modality: ObligationModality,
    polarity: ObligationPolarity,
    subject: Option<String>,
    predicate: Option<String>,
    action: String,
}

fn parse_conditional_obligation(text: &str) -> Option<ParsedConditionalObligation> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = normalized.to_ascii_lowercase();
    let (connector, after_connector) =
        ["if ", "when ", "unless "].into_iter().find_map(|prefix| {
            lower
                .strip_prefix(prefix)
                .map(|_| (prefix.trim(), &normalized[prefix.len()..]))
        })?;
    let (condition, consequent) = after_connector.split_once(',')?;
    let consequent_trimmed = consequent.trim();
    let lower_consequent = consequent_trimmed.to_ascii_lowercase();
    let modal = [
        "must not",
        "shall not",
        "should not",
        "must",
        "shall",
        "should",
        "may",
    ]
    .into_iter()
    .find_map(|modal| lower_consequent.find(modal).map(|idx| (modal, idx)))?;
    let (modal, modal_idx) = modal;
    let subject = consequent_trimmed[..modal_idx]
        .trim()
        .trim_end_matches(',')
        .trim();
    let action_start = modal_idx + modal.len();
    let action = consequent_trimmed[action_start..]
        .trim()
        .trim_end_matches('.')
        .to_string();
    if condition.trim().is_empty() || action.is_empty() {
        return None;
    }
    let modality = match modal {
        "must" => ObligationModality::Must,
        "shall" => ObligationModality::Shall,
        "should" => ObligationModality::Should,
        "may" => ObligationModality::May,
        "must not" => ObligationModality::MustNot,
        "shall not" => ObligationModality::ShallNot,
        "should not" => ObligationModality::ShouldNot,
        _ => ObligationModality::Unknown,
    };
    let polarity = match modal {
        "may" => ObligationPolarity::Permission,
        "must not" | "shall not" | "should not" => ObligationPolarity::Prohibition,
        _ => ObligationPolarity::Positive,
    };
    Some(ParsedConditionalObligation {
        connector: connector.to_string(),
        condition: condition.trim().to_string(),
        modality,
        polarity,
        subject: (!subject.is_empty()).then(|| subject.to_string()),
        predicate: Some(action.clone()),
        action,
    })
}

fn relation_for_obligation(
    modality: &ObligationModality,
    polarity: &ObligationPolarity,
) -> DocumentRelation {
    match (modality, polarity) {
        (ObligationModality::May, _) | (_, ObligationPolarity::Permission) => {
            DocumentRelation::Allows
        }
        (_, ObligationPolarity::Prohibition) => DocumentRelation::Forbids,
        _ => DocumentRelation::Requires,
    }
}

#[cfg(feature = "markdown")]
pub fn render_markdown(
    graph: &DocumentGraph,
    options: TransformOptions,
) -> Result<String, TransformError> {
    let mut out = String::new();
    let roots = graph
        .nodes
        .iter()
        .filter(|node| node.kind == DocumentNodeKind::Document)
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    let mut render_nodes = graph
        .nodes
        .iter()
        .filter(|node| node.kind != DocumentNodeKind::Document)
        .filter(|node| !is_nested_markdown_child(graph, &node.id, &roots))
        .collect::<Vec<_>>();
    render_nodes.sort_by_key(|node| node.ordinal.unwrap_or(usize::MAX));

    for node in render_nodes {
        let rendered = render_markdown_node(node, graph, &options)?;
        if rendered.is_empty() {
            continue;
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&rendered);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }

    Ok(out)
}

#[cfg(any(feature = "markdown", feature = "latex"))]
fn is_nested_markdown_child(graph: &DocumentGraph, node_id: &str, roots: &[&str]) -> bool {
    graph.edges.iter().any(|edge| {
        edge.relation == DocumentRelation::Contains
            && edge.target == node_id
            && !roots.contains(&edge.source.as_str())
    })
}

#[cfg(feature = "markdown")]
fn render_markdown_node(
    node: &DocumentNode,
    graph: &DocumentGraph,
    options: &TransformOptions,
) -> Result<String, TransformError> {
    match node.kind {
        DocumentNodeKind::Frontmatter => Ok(node
            .text
            .as_ref()
            .map(|raw| inert_markdown_code(raw))
            .unwrap_or_default()),
        DocumentNodeKind::Heading => {
            let level = node
                .attrs
                .get("level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6);
            let children = render_markdown_children(node, graph, options)?;
            let text = if children.is_empty() {
                escape_active_html(node.text.as_deref().unwrap_or(""))
            } else {
                children
            };
            Ok(format!("{} {text}\n", "#".repeat(level as usize)))
        }
        DocumentNodeKind::Paragraph => {
            let children = render_markdown_children(node, graph, options)?;
            let text = if children.is_empty() {
                escape_active_html(node.text.as_deref().unwrap_or(""))
            } else {
                children
            };
            Ok(format!("{text}\n"))
        }
        DocumentNodeKind::Text | DocumentNodeKind::Span => {
            Ok(escape_active_html(node.text.as_deref().unwrap_or("")))
        }
        DocumentNodeKind::Link => {
            let destination = node
                .attrs
                .get("destination")
                .and_then(Value::as_str)
                .ok_or_else(|| TransformError::MissingRequiredAttribute {
                    target: node.id.clone(),
                    attr: "destination".to_string(),
                })?;
            let text = escape_active_html(node.text.as_deref().unwrap_or(destination));
            let destination = sanitize_link_destination(destination);
            Ok(format!("[{text}]({destination})\n"))
        }
        DocumentNodeKind::CodeBlock => {
            let language = node
                .attrs
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("");
            let _ = language;
            Ok(inert_markdown_code(node.text.as_deref().unwrap_or("")))
        }
        DocumentNodeKind::Label => Ok(format!(
            "{{#{} }}\n",
            escape_active_html(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Reference => Ok(format!(
            "[{}]\n",
            escape_active_html(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Citation => Ok(format!(
            "[@{}]\n",
            escape_active_html(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::MathInline => Ok(format!(
            "${}$",
            escape_active_html(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::MathBlock => Ok(format!(
            "$$\n{}\n$$\n",
            escape_active_html(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Emphasis => Ok(format!(
            "*{}*",
            escape_active_html(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Strong => Ok(format!(
            "**{}**",
            escape_active_html(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Table => render_markdown_table_node(node, graph),
        DocumentNodeKind::RawBlock | DocumentNodeKind::RawInline if options.allow_raw_fallback => {
            Ok(inert_markdown_code(node.text.as_deref().unwrap_or("")))
        }
        _ if options.allow_lossy => Ok(String::new()),
        _ => Err(TransformError::UnsupportedNodeKind {
            node_id: node.id.clone(),
            node_kind: node.kind.clone(),
        }),
    }
}

#[cfg(feature = "markdown")]
fn render_markdown_children(
    node: &DocumentNode,
    graph: &DocumentGraph,
    options: &TransformOptions,
) -> Result<String, TransformError> {
    let mut children = graph
        .edges
        .iter()
        .filter(|edge| edge.relation == DocumentRelation::Contains && edge.source == node.id)
        .filter_map(|edge| graph.nodes.iter().find(|child| child.id == edge.target))
        .collect::<Vec<_>>();
    children.sort_by_key(|child| child.ordinal.unwrap_or(usize::MAX));
    let mut rendered = String::new();
    for child in children {
        rendered.push_str(&render_markdown_node(child, graph, options)?);
    }
    Ok(rendered)
}

#[cfg(feature = "markdown")]
fn render_markdown_table_node(
    node: &DocumentNode,
    graph: &DocumentGraph,
) -> Result<String, TransformError> {
    if let Some(table_value) = node.attrs.get("table") {
        if let Ok(table) =
            serde_json::from_value::<crate::markdown::MarkdownTable>(table_value.clone())
        {
            return Ok(render_markdown_table_rows(&table.rows));
        }
    }

    let mut rows = graph
        .edges
        .iter()
        .filter(|edge| edge.relation == DocumentRelation::Contains && edge.source == node.id)
        .filter_map(|edge| {
            graph
                .nodes
                .iter()
                .find(|candidate| candidate.id == edge.target)
        })
        .filter(|candidate| candidate.kind == DocumentNodeKind::TableRow)
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.ordinal.unwrap_or(usize::MAX));
    let mut table_rows = Vec::new();
    for row in rows {
        let mut cells = graph
            .edges
            .iter()
            .filter(|edge| edge.relation == DocumentRelation::Contains && edge.source == row.id)
            .filter_map(|edge| {
                graph
                    .nodes
                    .iter()
                    .find(|candidate| candidate.id == edge.target)
            })
            .filter(|candidate| candidate.kind == DocumentNodeKind::TableCell)
            .collect::<Vec<_>>();
        cells.sort_by_key(|cell| cell.ordinal.unwrap_or(usize::MAX));
        table_rows.push(
            cells
                .into_iter()
                .map(|cell| cell.text.clone().unwrap_or_default())
                .collect::<Vec<_>>(),
        );
    }
    Ok(render_markdown_table_rows(&table_rows))
}

#[cfg(feature = "markdown")]
fn render_markdown_table_rows(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push('|');
    out.push_str(
        &rows[0]
            .iter()
            .map(|cell| escape_markdown_table_cell(cell))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    out.push_str("|\n|");
    out.push_str(&vec!["---"; rows[0].len()].join("|"));
    out.push_str("|\n");
    for row in rows.iter().skip(1) {
        out.push('|');
        out.push_str(
            &row.iter()
                .map(|cell| escape_markdown_table_cell(cell))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        out.push_str("|\n");
    }
    out
}

#[cfg(feature = "markdown")]
fn escape_markdown_table_cell(text: &str) -> String {
    escape_active_html(text)
        .replace('|', "\\|")
        .replace(['\r', '\n'], " ")
}

#[cfg(feature = "latex")]
pub fn render_latex(
    graph: &DocumentGraph,
    options: TransformOptions,
) -> Result<String, TransformError> {
    let roots = graph
        .nodes
        .iter()
        .filter(|node| node.kind == DocumentNodeKind::Document)
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    let mut render_nodes = graph
        .nodes
        .iter()
        .filter(|node| node.kind != DocumentNodeKind::Document)
        .filter(|node| !is_nested_markdown_child(graph, &node.id, &roots))
        .collect::<Vec<_>>();
    render_nodes.sort_by_key(|node| node.ordinal.unwrap_or(usize::MAX));

    let mut out = String::new();
    for node in render_nodes {
        let rendered = render_latex_node(node, graph, options.clone())?;
        if rendered.is_empty() {
            continue;
        }
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&rendered);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

#[cfg(feature = "latex")]
fn render_latex_node(
    node: &DocumentNode,
    graph: &DocumentGraph,
    options: TransformOptions,
) -> Result<String, TransformError> {
    match node.kind {
        DocumentNodeKind::Heading | DocumentNodeKind::Section => {
            let level = node.attrs.get("level").and_then(Value::as_u64).unwrap_or(1);
            let cmd = match level {
                1 => "section",
                2 => "subsection",
                3 => "subsubsection",
                _ => "paragraph",
            };
            Ok(format!(
                "\\{}{{{}}}\n",
                cmd,
                escape_latex_text(node.text.as_deref().unwrap_or(""))
            ))
        }
        DocumentNodeKind::Paragraph | DocumentNodeKind::Text | DocumentNodeKind::Span => {
            Ok(format!(
                "{}\n",
                escape_latex_text(node.text.as_deref().unwrap_or(""))
            ))
        }
        DocumentNodeKind::Emphasis => Ok(format!(
            "\\emph{{{}}}",
            escape_latex_text(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Strong => Ok(format!(
            "\\textbf{{{}}}",
            escape_latex_text(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::MathInline => Ok(format!(
            "${}$",
            escape_latex_text(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::MathBlock => Ok(format!(
            "\\[\n{}\n\\]\n",
            escape_latex_text(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::CodeBlock => Ok(format!(
            "{}\n",
            inert_latex_literal(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Table => render_latex_table_node(node, graph),
        DocumentNodeKind::Label => Ok(format!(
            "\\label{{{}}}",
            escape_latex_text(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Reference => Ok(format!(
            "\\ref{{{}}}",
            escape_latex_text(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Citation => Ok(format!(
            "\\cite{{{}}}",
            escape_latex_text(node.text.as_deref().unwrap_or(""))
        )),
        DocumentNodeKind::Link => {
            let destination = node
                .attrs
                .get("destination")
                .and_then(Value::as_str)
                .ok_or_else(|| TransformError::MissingRequiredAttribute {
                    target: node.id.clone(),
                    attr: "destination".to_string(),
                })?;
            Ok(format!(
                "\\href{{{}}}{{{}}}",
                escape_latex_text(&sanitize_link_destination(destination)),
                escape_latex_text(node.text.as_deref().unwrap_or(destination))
            ))
        }
        DocumentNodeKind::RawBlock | DocumentNodeKind::RawInline if options.allow_raw_fallback => {
            let raw = node
                .text
                .as_deref()
                .or_else(|| node.attrs.get("command").and_then(Value::as_str))
                .unwrap_or("");
            Ok(inert_latex_literal(raw))
        }
        _ if options.allow_lossy => Ok(String::new()),
        _ => Err(TransformError::UnsupportedNodeKind {
            node_id: node.id.clone(),
            node_kind: node.kind.clone(),
        }),
    }
}

#[cfg(feature = "latex")]
fn render_latex_table_node(
    node: &DocumentNode,
    graph: &DocumentGraph,
) -> Result<String, TransformError> {
    let mut rows = graph
        .edges
        .iter()
        .filter(|edge| edge.relation == DocumentRelation::Contains && edge.source == node.id)
        .filter_map(|edge| {
            graph
                .nodes
                .iter()
                .find(|candidate| candidate.id == edge.target)
        })
        .filter(|candidate| candidate.kind == DocumentNodeKind::TableRow)
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.ordinal.unwrap_or(usize::MAX));
    if rows.is_empty() {
        return Ok(String::new());
    }

    let mut rendered_rows = Vec::new();
    let mut max_cols = 0_usize;
    for row in rows {
        let mut cells = graph
            .edges
            .iter()
            .filter(|edge| edge.relation == DocumentRelation::Contains && edge.source == row.id)
            .filter_map(|edge| {
                graph
                    .nodes
                    .iter()
                    .find(|candidate| candidate.id == edge.target)
            })
            .filter(|candidate| candidate.kind == DocumentNodeKind::TableCell)
            .collect::<Vec<_>>();
        cells.sort_by_key(|cell| cell.ordinal.unwrap_or(usize::MAX));
        max_cols = max_cols.max(cells.len());
        rendered_rows.push(
            cells
                .into_iter()
                .map(|cell| escape_latex(cell.text.as_deref().unwrap_or("")))
                .collect::<Vec<_>>(),
        );
    }
    let cols = if max_cols == 0 { 1 } else { max_cols };
    let mut out = format!("\\begin{{tabular}}{{{}}}\n", "l".repeat(cols));
    for (idx, row) in rendered_rows.iter().enumerate() {
        out.push_str(&row.join(" & "));
        out.push_str(" \\\\");
        out.push('\n');
        if idx == 0 {
            out.push_str("\\hline\n");
        }
    }
    out.push_str("\\end{tabular}\n");
    Ok(out)
}

#[cfg(feature = "latex")]
fn escape_latex(text: &str) -> String {
    escape_latex_text(text)
}

#[cfg(feature = "latex")]
impl ToDocumentGraph for crate::latex::LatexDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        use crate::latex::LatexNodeKind;

        let identities = context
            .identity_generator(SchemaVersion::LATEX_V1, "latex")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Latex).with_projection(
            "latex",
            SchemaVersion::LATEX_V1,
            "grist.latex.to-document-graph.v2",
        );
        graph.source = context.source;
        graph.language = Some(context.language.unwrap_or_else(|| "latex".to_string()));
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;

        let root_id = stable_projection_node_id(
            &identities,
            vec!["document".to_string()],
            Some("root"),
            None,
        )?;
        graph.add_node(
            DocumentNode::new(&root_id, DocumentNodeKind::Document)
                .with_name("latex")
                .with_ordinal(0),
        );

        for (idx, latex_node) in self.nodes.iter().enumerate() {
            let node_id = stable_projection_node_id(
                &identities,
                vec!["document".to_string(), "nodes".to_string(), idx.to_string()],
                None,
                Some(&latex_node.range),
            )?;
            let kind = match latex_node.kind {
                LatexNodeKind::DocumentClass | LatexNodeKind::Metadata => {
                    DocumentNodeKind::Metadata
                }
                LatexNodeKind::MacroDefinition => DocumentNodeKind::RawBlock,
                LatexNodeKind::MacroUse | LatexNodeKind::Include => DocumentNodeKind::RawInline,
                LatexNodeKind::Section => DocumentNodeKind::Heading,
                LatexNodeKind::Paragraph => DocumentNodeKind::Paragraph,
                LatexNodeKind::Text => DocumentNodeKind::Text,
                LatexNodeKind::Command => match latex_node.command.as_deref() {
                    Some("textbf") => DocumentNodeKind::Strong,
                    Some("emph") => DocumentNodeKind::Emphasis,
                    _ => DocumentNodeKind::RawInline,
                },
                LatexNodeKind::Environment => DocumentNodeKind::RawBlock,
                LatexNodeKind::List => DocumentNodeKind::List,
                LatexNodeKind::ListItem => DocumentNodeKind::ListItem,
                LatexNodeKind::Table => DocumentNodeKind::Table,
                LatexNodeKind::TableRow => DocumentNodeKind::TableRow,
                LatexNodeKind::TableCell => DocumentNodeKind::TableCell,
                LatexNodeKind::Figure => DocumentNodeKind::Figure,
                LatexNodeKind::Caption => DocumentNodeKind::Caption,
                LatexNodeKind::Equation => DocumentNodeKind::Equation,
                LatexNodeKind::MathInline => DocumentNodeKind::MathInline,
                LatexNodeKind::MathBlock => DocumentNodeKind::MathBlock,
                LatexNodeKind::Label => DocumentNodeKind::Label,
                LatexNodeKind::Ref => DocumentNodeKind::Reference,
                LatexNodeKind::Citation => DocumentNodeKind::Citation,
                LatexNodeKind::Comment => DocumentNodeKind::RawBlock,
                LatexNodeKind::RawCommand | LatexNodeKind::RawInline => DocumentNodeKind::RawInline,
            };
            let retains_raw = kind.retains_raw_content();
            let mut node = DocumentNode::new(&node_id, kind).with_ordinal(idx);
            node.range = Some(latex_node.range.clone());
            node.locator = latex_node.locator.clone();
            node.text = latex_node
                .text
                .clone()
                .or_else(|| latex_node.argument.clone())
                .or_else(|| latex_node.name.clone());
            node.name = latex_node.name.clone();
            insert_json_attr(&mut node.attrs, "command", &latex_node.command);
            insert_json_attr(&mut node.attrs, "argument", &latex_node.argument);
            insert_json_attr(&mut node.attrs, "source", &latex_node.source);
            insert_json_attr(&mut node.attrs, "include", &latex_node.include);
            for (key, value) in &latex_node.attrs {
                node.attrs.insert(key.clone(), value.clone());
            }
            if retains_raw {
                node.raw = Some(RawNodeContent {
                    namespace: "grist.latex".to_string(),
                    original_kind: format!("{:?}", latex_node.kind).to_ascii_lowercase(),
                    payload: serde_json::to_value(latex_node).map_err(|error| {
                        TransformError::Other {
                            message: format!("could not retain raw LaTeX node: {error}"),
                        }
                    })?,
                });
            }
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);

            match latex_node.kind {
                LatexNodeKind::Ref => {
                    if let Some(target) = latex_node.argument.as_ref().or(latex_node.name.as_ref())
                    {
                        graph.add_edge(
                            DocumentEdge::new(&node_id, DocumentRelation::References, target)
                                .with_range(latex_node.range.clone()),
                        );
                    }
                }
                LatexNodeKind::Citation => {
                    if let Some(target) = latex_node.argument.as_ref().or(latex_node.name.as_ref())
                    {
                        graph.add_edge(
                            DocumentEdge::new(&node_id, DocumentRelation::Cites, target)
                                .with_range(latex_node.range.clone()),
                        );
                    }
                }
                LatexNodeKind::Label => {
                    if let Some(target) = latex_node.argument.as_ref().or(latex_node.name.as_ref())
                    {
                        graph.add_edge(
                            DocumentEdge::new(&node_id, DocumentRelation::Defines, target)
                                .with_range(latex_node.range.clone()),
                        );
                    }
                }
                _ => {}
            }
        }

        graph
            .diagnostics
            .extend(self.parse_errors.iter().map(|err| {
                Diagnostic::error("grist.latex", "latex.parse", err.message.clone())
                    .with_range(err.range.clone())
            }));

        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "bibliography")]
impl ToDocumentGraph for crate::bibliography::BibliographyDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        use crate::bibliography::BibliographyConstructKind;

        let identities = context
            .identity_generator(SchemaVersion::BIBLIOGRAPHY_V1, "bibliography")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Bibliography)
            .with_projection(
                "bibliography",
                SchemaVersion::BIBLIOGRAPHY_V1,
                "grist.bibliography.to-document-graph.v1",
            );
        graph.source = context.source.or_else(|| Some(self.source.clone()));
        graph.language = Some("bibtex".to_string());
        graph.dialect = Some(format!("{:?}", self.dialect).to_ascii_lowercase());
        graph.attrs = context.attrs;

        let root_id = stable_projection_node_id(
            &identities,
            vec!["bibliography".to_string()],
            Some("root"),
            Some(&self.range),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name("bibliography")
            .with_ordinal(0);
        root.range = Some(self.range.clone());
        root.locator = Some(self.locator.clone());
        graph.add_node(root);

        let mut entry_ids = BTreeMap::new();
        for entry in &self.entries {
            let entry_id = stable_projection_node_id(
                &identities,
                vec!["entries".to_string(), entry.index.to_string()],
                Some(&format!("{}:{}", entry.key, entry.index)),
                Some(&entry.range),
            )?;
            entry_ids.insert(entry.index, entry_id.clone());
            let title = entry
                .effective_field("title")
                .and_then(|field| field.value.resolved.clone())
                .unwrap_or_else(|| entry.key.clone());
            let mut node = DocumentNode::new(&entry_id, DocumentNodeKind::BibliographyEntry)
                .with_name(entry.key.clone())
                .with_ordinal(entry.index);
            node.text = Some(title);
            node.range = Some(entry.range.clone());
            node.locator = Some(entry.locator.clone());
            node.attrs
                .insert("entry_type".into(), entry.entry_type.clone().into());
            node.attrs.insert("key".into(), entry.key.clone().into());
            node.extensions.insert(
                "grist.bibliography".to_string(),
                serde_json::to_value(entry).map_err(|error| TransformError::Other {
                    message: format!("could not retain bibliography entry: {error}"),
                })?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &entry_id);

            for (field_index, field) in entry.fields.iter().enumerate() {
                let field_id = stable_projection_node_id(
                    &identities,
                    vec![
                        "entries".to_string(),
                        entry.index.to_string(),
                        "fields".to_string(),
                        field_index.to_string(),
                    ],
                    Some(&format!("{}:{}", field.name, field_index)),
                    Some(&field.range),
                )?;
                let mut field_node = DocumentNode::new(&field_id, DocumentNodeKind::Field)
                    .with_name(field.name.clone())
                    .with_ordinal(field_index);
                field_node.text = field
                    .value
                    .resolved
                    .clone()
                    .or_else(|| Some(field.value.raw.clone()));
                field_node.range = Some(field.range.clone());
                field_node.locator = Some(field.locator.clone());
                field_node.attrs.insert(
                    "expansion_status".into(),
                    serde_json::to_value(field.value.expansion_status).unwrap_or(Value::Null),
                );
                field_node.extensions.insert(
                    "grist.bibliography".to_string(),
                    serde_json::to_value(field).map_err(|error| TransformError::Other {
                        message: format!("could not retain bibliography field: {error}"),
                    })?,
                );
                graph.add_node(field_node);
                graph.add_contains(&entry_id, &field_id);
            }
        }

        for (construct_index, construct) in self.constructs.iter().enumerate() {
            if construct.entry_index.is_some() {
                continue;
            }
            let kind = match construct.kind {
                BibliographyConstructKind::String | BibliographyConstructKind::Preamble => {
                    DocumentNodeKind::Metadata
                }
                BibliographyConstructKind::Comment | BibliographyConstructKind::LineComment => {
                    DocumentNodeKind::Comment
                }
                BibliographyConstructKind::Raw | BibliographyConstructKind::Malformed => {
                    DocumentNodeKind::RawBlock
                }
                BibliographyConstructKind::Entry => continue,
            };
            let construct_id = stable_projection_node_id(
                &identities,
                vec!["constructs".to_string(), construct_index.to_string()],
                None,
                Some(&construct.range),
            )?;
            let retains_raw = kind.retains_raw_content();
            let mut node = DocumentNode::new(&construct_id, kind).with_ordinal(construct_index);
            node.text = Some(construct.raw.clone());
            node.range = Some(construct.range.clone());
            node.locator = Some(construct.locator.clone());
            node.extensions.insert(
                "grist.bibliography".to_string(),
                serde_json::to_value(construct).map_err(|error| TransformError::Other {
                    message: format!("could not retain bibliography construct: {error}"),
                })?,
            );
            if retains_raw {
                node.raw = Some(RawNodeContent {
                    namespace: "grist.bibliography".to_string(),
                    original_kind: format!("{:?}", construct.kind).to_ascii_lowercase(),
                    payload: serde_json::to_value(construct).map_err(|error| {
                        TransformError::Other {
                            message: format!(
                                "could not retain raw bibliography construct: {error}"
                            ),
                        }
                    })?,
                });
            }
            graph.add_node(node);
            graph.add_contains(&root_id, &construct_id);
        }

        for resolution in &self.crossrefs {
            if resolution.status != crate::bibliography::CrossrefStatus::Resolved {
                continue;
            }
            let Some(source) = entry_ids.get(&resolution.source_entry_index) else {
                continue;
            };
            for target_index in &resolution.target_entry_indices {
                let Some(target) = entry_ids.get(target_index) else {
                    continue;
                };
                graph.add_edge(
                    DocumentEdge::explicit(
                        source,
                        DocumentRelation::References,
                        target,
                        resolution.locator.clone(),
                    )
                    .with_attr("field", resolution.field.clone())
                    .with_attr("target_key", resolution.target_key.clone())
                    .with_attr("resolution", "resolved"),
                );
            }
        }

        graph
            .diagnostics
            .extend(self.parse_errors.iter().map(|error| {
                Diagnostic::error("grist.bibliography", &error.code, &error.message)
                    .with_range(error.range.clone())
            }));
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "csv")]
impl ToDocumentGraph for crate::csv::CsvDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::CSV_V2, "csv")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Csv).with_projection(
            "csv",
            SchemaVersion::CSV_V2,
            "grist.csv.to-document-graph.v2",
        );
        graph.source = context.source;
        graph.language = Some(context.language.unwrap_or_else(|| "csv".into()));
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| self.dialect.delimiter_text.clone()),
        );
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "dialect".into(),
            serde_json::to_value(&self.dialect).map_err(projection_transform_error)?,
        );
        let root_id =
            stable_projection_node_id(&identities, vec!["document".into()], Some("root"), None)?;
        graph.add_node(DocumentNode::new(&root_id, DocumentNodeKind::Document).with_ordinal(0));
        let table_id = stable_projection_node_id(
            &identities,
            vec!["document".into(), "table".into()],
            Some("records"),
            None,
        )?;
        graph.add_node(
            DocumentNode::new(&table_id, DocumentNodeKind::Table)
                .with_ordinal(1)
                .with_attr("column_count", self.column_count),
        );
        graph.add_contains(&root_id, &table_id);
        for (ordinal, row) in self
            .header_record
            .iter()
            .chain(self.rows.iter())
            .enumerate()
        {
            let row_id = stable_projection_node_id(
                &identities,
                vec![
                    "document".into(),
                    "records".into(),
                    row.source_record_index.to_string(),
                ],
                None,
                Some(&row.range),
            )?;
            let mut row_node = DocumentNode::new(&row_id, DocumentNodeKind::TableRow)
                .with_range(row.range.clone())
                .with_locator(row.locator.clone())
                .with_ordinal(ordinal)
                .with_attr("malformed", row.malformed)
                .with_attr(
                    "record_role",
                    serde_json::to_value(row.role).map_err(projection_transform_error)?,
                );
            row_node.extensions.insert(
                "grist.csv".into(),
                serde_json::json!({
                    "source_record_index": row.source_record_index,
                    "raw": row.raw,
                    "terminator": row.terminator,
                    "issues": row.issues,
                }),
            );
            graph.add_node(row_node);
            graph.add_contains(&table_id, &row_id);
            for cell in &row.cells {
                let cell_id = stable_projection_node_id(
                    &identities,
                    vec![
                        "document".into(),
                        "records".into(),
                        row.source_record_index.to_string(),
                        "cells".into(),
                        cell.column_index.to_string(),
                    ],
                    None,
                    Some(&cell.range),
                )?;
                let mut cell_node = DocumentNode::new(&cell_id, DocumentNodeKind::TableCell)
                    .with_range(cell.range.clone())
                    .with_locator(cell.locator.clone())
                    .with_text(cell.text.clone())
                    .with_ordinal(cell.column_index);
                cell_node.name = cell.header.clone();
                cell_node.extensions.insert(
                    "grist.csv".into(),
                    serde_json::to_value(cell).map_err(projection_transform_error)?,
                );
                graph.add_node(cell_node);
                graph.add_contains(&row_id, &cell_id);
            }
        }
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "serialization")]
impl ToDocumentGraph for crate::serialization::StructuredTextDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::STRUCTURED_TEXT_V2, "structured-text")
            .map_err(projection_transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Serialization)
            .with_projection(
                "structured-text",
                SchemaVersion::STRUCTURED_TEXT_V2,
                "grist.structured-text.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = Some(
            context
                .language
                .unwrap_or_else(|| format!("{:?}", self.format).to_ascii_lowercase()),
        );
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "ordering".into(),
            serde_json::to_value(self.ordering).map_err(projection_transform_error)?,
        );
        let root_id =
            stable_projection_node_id(&identities, vec!["document".into()], Some("root"), None)?;
        graph.add_node(DocumentNode::new(&root_id, DocumentNodeKind::Document).with_ordinal(0));

        if self.format == crate::serialization::StructuredTextFormat::Jsonl {
            for record in &self.records {
                let record_id = stable_projection_node_id(
                    &identities,
                    vec![
                        "document".into(),
                        "records".into(),
                        record.index.to_string(),
                    ],
                    None,
                    Some(&record.range),
                )?;
                let mut record_node = DocumentNode::new(&record_id, DocumentNodeKind::Record)
                    .with_range(record.range.clone())
                    .with_locator(record.locator.clone())
                    .with_ordinal(record.index - 1)
                    .with_attr("source_line", record.source_line);
                record_node.extensions.insert(
                    "grist.structured-text".into(),
                    serde_json::json!({"raw": record.raw, "malformed": record.value.is_none()}),
                );
                graph.add_node(record_node);
                graph.add_contains(&root_id, &record_id);
                if let Some(value) = &record.value {
                    add_structured_value(
                        &mut graph,
                        &identities,
                        &record_id,
                        value,
                        vec!["records".into(), record.index.to_string()],
                        0,
                    )?;
                } else {
                    let raw_id = stable_projection_node_id(
                        &identities,
                        vec![
                            "document".into(),
                            "records".into(),
                            record.index.to_string(),
                            "raw".into(),
                        ],
                        None,
                        Some(&record.range),
                    )?;
                    graph.add_node(
                        DocumentNode::new(&raw_id, DocumentNodeKind::Raw)
                            .with_range(record.range.clone())
                            .with_locator(record.locator.clone())
                            .with_text(record.raw.clone()),
                    );
                    graph.add_contains(&record_id, &raw_id);
                }
            }
        } else {
            for (ordinal, value) in self.documents.iter().enumerate() {
                add_structured_value(
                    &mut graph,
                    &identities,
                    &root_id,
                    value,
                    vec!["documents".into(), ordinal.to_string()],
                    ordinal,
                )?;
            }
        }
        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "serialization")]
fn add_structured_value(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    parent_id: &str,
    value: &crate::serialization::StructuredValue,
    structural_path: Vec<String>,
    ordinal: usize,
) -> Result<String, TransformError> {
    use crate::serialization::{StructuredScalar, StructuredValueKind};
    let mut identity_path = vec!["document".into()];
    identity_path.extend(structural_path.iter().cloned());
    let id = stable_projection_node_id(
        identities,
        identity_path,
        Some(&value.id),
        Some(&value.range),
    )?;
    let node_kind = if value.kind == StructuredValueKind::RawUnknown {
        DocumentNodeKind::Raw
    } else {
        DocumentNodeKind::StructuredValue
    };
    let mut node = DocumentNode::new(&id, node_kind)
        .with_range(value.range.clone())
        .with_locator(value.locator.clone())
        .with_ordinal(ordinal)
        .with_attr(
            "value_kind",
            serde_json::to_value(value.kind).map_err(projection_transform_error)?,
        )
        .with_attr("path", value.path.clone())
        .with_attr("recovered", value.recovered);
    node.name = value.tag.clone().or_else(|| value.alias.clone());
    node.text = value.scalar.as_ref().map(|scalar| match scalar {
        StructuredScalar::Null => "null".into(),
        StructuredScalar::Boolean { value } => value.to_string(),
        StructuredScalar::Integer { canonical } | StructuredScalar::Float { canonical, .. } => {
            canonical.clone()
        }
        StructuredScalar::String { value }
        | StructuredScalar::Date { value }
        | StructuredScalar::Time { value }
        | StructuredScalar::DateTime { value } => value.clone(),
    });
    node.extensions.insert(
        "grist.structured-text".into(),
        serde_json::json!({
            "raw": value.raw,
            "anchor": value.anchor,
            "tag": value.tag,
            "alias": value.alias,
            "alias_target_id": value.alias_target_id,
        }),
    );
    graph.add_node(node);
    graph.add_contains(parent_id, &id);

    for entry in &value.entries {
        let mut field_path = structural_path.clone();
        field_path.extend(["entries".into(), entry.index.to_string()]);
        let mut field_identity_path = vec!["document".into()];
        field_identity_path.extend(field_path.iter().cloned());
        let field_id = stable_projection_node_id(
            identities,
            field_identity_path,
            Some(&entry.key.id),
            Some(&entry.key.range),
        )?;
        let mut field = DocumentNode::new(&field_id, DocumentNodeKind::Field)
            .with_range(entry.key.range.clone())
            .with_locator(entry.key.locator.clone())
            .with_ordinal(entry.index)
            .with_attr("duplicate_ordinal", entry.duplicate_ordinal);
        field.name = entry.key_text.clone();
        field.text = entry.key_text.clone();
        graph.add_node(field);
        graph.add_contains(&id, &field_id);
        let mut value_path = field_path;
        value_path.push("value".into());
        add_structured_value(graph, identities, &field_id, &entry.value, value_path, 0)?;
    }
    for (index, item) in value.items.iter().enumerate() {
        let mut item_path = structural_path.clone();
        item_path.extend(["items".into(), index.to_string()]);
        add_structured_value(graph, identities, &id, item, item_path, index)?;
    }
    Ok(id)
}

#[cfg(feature = "python")]
impl ToDocumentGraph for crate::python::PythonFile {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        use crate::python::PythonSymbolKind;

        const NAMESPACE: &str = "grist.python";
        let identities = context
            .identity_generator(SchemaVersion::PYTHON_CODE_V1, "python")
            .map_err(projection_transform_error)?;
        let mut graph = code_graph_from_context(context, DocumentKind::Python, "python");
        let root_id = ensure_code_root(&mut graph, &identities, "python")?;
        if let Some(root) = graph.nodes.iter_mut().find(|node| node.id == root_id) {
            root.extensions.insert(
                NAMESPACE.into(),
                serde_json::json!({"schema_version": self.schema_version, "detail": self.detail}),
            );
        }
        let mut symbol_ids = BTreeMap::new();
        for symbol in &self.symbols {
            let id = code_node_id(&identities, "symbol", Some(&symbol.id), Some(&symbol.range))?;
            for key in [&symbol.id, &symbol.qualified_name] {
                insert_code_symbol_alias(&mut symbol_ids, key.clone(), &id);
            }
        }
        let symbols_by_range = self
            .symbols
            .iter()
            .filter_map(|symbol| {
                symbol_ids
                    .get(&symbol.qualified_name)
                    .cloned()
                    .map(|id| (symbol.range.clone(), id))
            })
            .collect::<Vec<_>>();

        for symbol in &self.symbols {
            let id = symbol_ids[&symbol.qualified_name].clone();
            let mut node = DocumentNode::new(
                &id,
                match symbol.kind {
                    PythonSymbolKind::Class => DocumentNodeKind::Class,
                    PythonSymbolKind::Function => DocumentNodeKind::Function,
                    PythonSymbolKind::Method => DocumentNodeKind::Method,
                    PythonSymbolKind::Unknown => DocumentNodeKind::Symbol,
                },
            )
            .with_name(symbol.name.clone())
            .with_qualified_name(symbol.qualified_name.clone())
            .with_range(symbol.range.clone());
            let parent = symbol_parent_node(&graph.id, &symbol.qualified_name, &symbol_ids)
                .or_else(|| parent_lookup(symbol.parent.as_deref(), &symbol_ids));
            node.parent = parent.clone();
            insert_json_attr(&mut node.attrs, "visibility", &symbol.visibility);
            insert_json_attr(&mut node.attrs, "decorators", &symbol.decorators);
            insert_json_attr(&mut node.attrs, "superclasses", &symbol.superclasses);
            insert_json_attr(&mut node.attrs, "doc", &symbol.doc);
            retain_code_native(&mut node, NAMESPACE, symbol);
            graph.add_node(node);
            graph.add_contains(node_parent_or_root(&root_id, parent), &id);
            for superclass in &symbol.superclasses {
                let target = parent_lookup(Some(superclass), &symbol_ids)
                    .unwrap_or_else(|| superclass.clone());
                graph.add_edge(code_inferred_edge(
                    &id,
                    DocumentRelation::Inherits,
                    target,
                    "grist.code.superclass-name-resolution.v1",
                    0.85,
                    &symbol.range,
                )?);
            }
        }

        for import in &self.imports {
            let id = code_node_id(&identities, "import", Some(&import.id), Some(&import.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Import)
                .with_name(import.module.clone())
                .with_range(import.range.clone());
            insert_json_attr(&mut node.attrs, "names", &import.names);
            insert_json_attr(&mut node.attrs, "aliases", &import.aliases);
            insert_json_attr(&mut node.attrs, "level", &import.level);
            retain_code_native(&mut node, NAMESPACE, import);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(
                DocumentEdge::new(&root_id, DocumentRelation::Imports, &id)
                    .with_range(import.range.clone()),
            );
        }
        for export in &self.exports {
            let id = code_node_id(&identities, "export", Some(&export.id), Some(&export.range))?;
            let mut node =
                DocumentNode::new(&id, DocumentNodeKind::Export).with_range(export.range.clone());
            insert_json_attr(&mut node.attrs, "names", &export.names);
            retain_code_native(&mut node, NAMESPACE, export);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(
                DocumentEdge::new(&root_id, DocumentRelation::Exports, &id)
                    .with_range(export.range.clone()),
            );
        }

        for assignment in &self.assignments {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "assignment",
                &assignment.id,
                DocumentNodeKind::Assignment,
                assignment.parent.as_deref(),
                &symbol_ids,
                Some(assignment.range.clone()),
                Some(assignment.lhs.clone()),
                NAMESPACE,
                assignment,
                |attrs| {
                    insert_json_attr(attrs, "rhs", &assignment.rhs);
                    insert_json_attr(attrs, "operator", &assignment.operator);
                },
            )?;
        }
        for ret in &self.returns {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "return",
                &ret.id,
                DocumentNodeKind::Return,
                ret.parent.as_deref(),
                &symbol_ids,
                Some(ret.range.clone()),
                ret.expression.clone(),
                NAMESPACE,
                ret,
                |_| {},
            )?;
        }
        for branch in &self.branches {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "branch",
                &branch.id,
                DocumentNodeKind::Branch,
                branch.parent.as_deref(),
                &symbol_ids,
                Some(branch.range.clone()),
                branch.condition.clone(),
                NAMESPACE,
                branch,
                |attrs| insert_json_attr(attrs, "kind", &branch.kind),
            )?;
        }
        for call in &self.calls {
            let parent = parent_lookup(call.parent.as_deref(), &symbol_ids)
                .unwrap_or_else(|| root_id.clone());
            let id = code_node_id(&identities, "call", Some(&call.id), Some(&call.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Call)
                .with_name(call.target.clone())
                .with_text(call.target.clone())
                .with_range(call.range.clone());
            insert_json_attr(&mut node.attrs, "args", &call.args);
            retain_code_native(&mut node, NAMESPACE, call);
            graph.add_node(node);
            graph.add_contains(&parent, &id);
            let target = parent_lookup(Some(&call.target), &symbol_ids)
                .unwrap_or_else(|| call.target.clone());
            graph.add_edge(code_inferred_edge(
                &parent,
                DocumentRelation::Calls,
                target,
                "grist.code.call-target-name-resolution.v1",
                0.75,
                &call.range,
            )?);
        }

        for test in &self.tests {
            add_code_test_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                test,
                &test.id,
                &test.name,
                &test.framework,
                Some(&test.symbol_id),
                &symbol_ids,
                &test.range,
            )?;
        }
        for comment in &self.comments {
            add_code_comment_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                comment,
                &comment.id,
                &comment.text,
                false,
                &comment.range,
                &symbols_by_range,
            )?;
        }
        let syntax_ids = self
            .syntax_nodes
            .iter()
            .map(|syntax| {
                Ok((
                    syntax.id.clone(),
                    code_node_id(&identities, "syntax", Some(&syntax.id), Some(&syntax.range))?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, TransformError>>()?;
        for syntax in &self.syntax_nodes {
            add_code_syntax_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                syntax,
                &syntax.id,
                &syntax.kind,
                syntax.named,
                syntax.error,
                syntax.missing,
                syntax.parent.as_deref(),
                &syntax_ids,
                &syntax.raw,
                &syntax.range,
            )?;
        }
        for (ordinal, error) in self.parse_errors.iter().enumerate() {
            add_code_parse_error(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                error,
                ordinal,
                &error.node_kind,
                &error.raw,
                error.missing,
                &error.range,
            )?;
        }

        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(feature = "rust")]
impl ToDocumentGraph for crate::rust::RustFile {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        use crate::rust::RustSymbolKind;

        const NAMESPACE: &str = "grist.rust";
        let identities = context
            .identity_generator(SchemaVersion::RUST_CODE_V1, "rust")
            .map_err(projection_transform_error)?;
        let mut graph = code_graph_from_context(context, DocumentKind::Rust, "rust");
        let root_id = ensure_code_root(&mut graph, &identities, "rust")?;
        if let Some(root) = graph.nodes.iter_mut().find(|node| node.id == root_id) {
            root.extensions.insert(
                NAMESPACE.into(),
                serde_json::json!({"schema_version": self.schema_version, "detail": self.detail}),
            );
        }
        let rust_qualified_names = self
            .symbols
            .iter()
            .map(|symbol| {
                let mut ancestors = self
                    .symbols
                    .iter()
                    .filter(|candidate| {
                        candidate.id != symbol.id
                            && candidate.range.byte_start <= symbol.range.byte_start
                            && candidate.range.byte_end >= symbol.range.byte_end
                    })
                    .collect::<Vec<_>>();
                ancestors.sort_by(|left, right| {
                    left.range
                        .byte_start
                        .cmp(&right.range.byte_start)
                        .then_with(|| right.range.byte_end.cmp(&left.range.byte_end))
                });
                let mut parts = ancestors
                    .into_iter()
                    .map(|ancestor| {
                        if ancestor.kind == RustSymbolKind::Impl {
                            self.inheritances
                                .iter()
                                .find(|inheritance| inheritance.range == ancestor.range)
                                .map(|inheritance| inheritance.implementation.clone())
                                .unwrap_or_else(|| ancestor.name.clone())
                        } else {
                            ancestor.name.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                parts.push(if symbol.kind == RustSymbolKind::Impl {
                    self.inheritances
                        .iter()
                        .find(|inheritance| inheritance.range == symbol.range)
                        .map(|inheritance| inheritance.implementation.clone())
                        .unwrap_or_else(|| symbol.name.clone())
                } else {
                    symbol.name.clone()
                });
                (symbol.id.clone(), parts.join("::"))
            })
            .collect::<BTreeMap<_, _>>();
        let mut symbol_ids = BTreeMap::new();
        for symbol in &self.symbols {
            let id = code_node_id(&identities, "symbol", Some(&symbol.id), Some(&symbol.range))?;
            insert_code_symbol_alias(&mut symbol_ids, symbol.id.clone(), &id);
            insert_code_symbol_alias(
                &mut symbol_ids,
                rust_qualified_names[&symbol.id].clone(),
                &id,
            );
            insert_code_symbol_alias(&mut symbol_ids, id.clone(), &id);
        }
        let symbols_by_range = self
            .symbols
            .iter()
            .filter_map(|symbol| {
                symbol_ids
                    .get(&symbol.id)
                    .cloned()
                    .map(|id| (symbol.range.clone(), id))
            })
            .collect::<Vec<_>>();

        for symbol in &self.symbols {
            let id = symbol_ids[&symbol.id].clone();
            let mut node = DocumentNode::new(
                &id,
                match symbol.kind {
                    RustSymbolKind::Module => DocumentNodeKind::Module,
                    RustSymbolKind::Function => DocumentNodeKind::Function,
                    RustSymbolKind::Method => DocumentNodeKind::Method,
                    RustSymbolKind::Struct => DocumentNodeKind::Class,
                    RustSymbolKind::Enum => DocumentNodeKind::Enum,
                    RustSymbolKind::Trait => DocumentNodeKind::Interface,
                    RustSymbolKind::TypeAlias => DocumentNodeKind::TypeAlias,
                    RustSymbolKind::Const | RustSymbolKind::Static => DocumentNodeKind::Variable,
                    RustSymbolKind::MacroDefinition
                    | RustSymbolKind::MacroInvocation
                    | RustSymbolKind::Impl
                    | RustSymbolKind::Union
                    | RustSymbolKind::Unknown => DocumentNodeKind::Symbol,
                },
            )
            .with_name(symbol.name.clone())
            .with_qualified_name(rust_qualified_names[&symbol.id].clone())
            .with_range(symbol.range.clone());
            let parent = enclosing_code_symbol(&symbol.range, &symbols_by_range)
                .or_else(|| parent_lookup(symbol.parent.as_deref(), &symbol_ids));
            node.parent = parent.clone();
            insert_json_attr(&mut node.attrs, "visibility", &symbol.visibility);
            insert_json_attr(&mut node.attrs, "attributes", &symbol.attributes);
            insert_json_attr(&mut node.attrs, "doc", &symbol.doc);
            retain_code_native(&mut node, NAMESPACE, symbol);
            graph.add_node(node);
            graph.add_contains(node_parent_or_root(&root_id, parent), &id);
        }

        for import in &self.imports {
            let id = code_node_id(&identities, "import", Some(&import.id), Some(&import.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Import)
                .with_name(import.alias.clone().unwrap_or_else(|| import.path.clone()))
                .with_text(import.path.clone())
                .with_range(import.range.clone());
            insert_json_attr(&mut node.attrs, "path", &import.path);
            insert_json_attr(&mut node.attrs, "alias", &import.alias);
            insert_json_attr(&mut node.attrs, "visibility", &import.visibility);
            retain_code_native(&mut node, NAMESPACE, import);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(
                DocumentEdge::new(&root_id, DocumentRelation::Imports, &id)
                    .with_range(import.range.clone()),
            );
        }
        for export in &self.exports {
            let id = code_node_id(&identities, "export", Some(&export.id), Some(&export.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Export)
                .with_name(export.name.clone())
                .with_range(export.range.clone());
            insert_json_attr(&mut node.attrs, "kind", &export.kind);
            retain_code_native(&mut node, NAMESPACE, export);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(
                DocumentEdge::new(&root_id, DocumentRelation::Exports, &id)
                    .with_range(export.range.clone()),
            );
        }

        for assignment in &self.assignments {
            let parent = enclosing_code_symbol(&assignment.range, &symbols_by_range)
                .or_else(|| parent_lookup(assignment.parent.as_deref(), &symbol_ids));
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "assignment",
                &assignment.id,
                DocumentNodeKind::Assignment,
                parent.as_deref(),
                &symbol_ids,
                Some(assignment.range.clone()),
                Some(assignment.lhs.clone()),
                NAMESPACE,
                assignment,
                |attrs| {
                    insert_json_attr(attrs, "rhs", &assignment.rhs);
                    insert_json_attr(attrs, "operator", &assignment.operator);
                },
            )?;
        }
        for ret in &self.returns {
            let parent = enclosing_code_symbol(&ret.range, &symbols_by_range)
                .or_else(|| parent_lookup(ret.parent.as_deref(), &symbol_ids));
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "return",
                &ret.id,
                DocumentNodeKind::Return,
                parent.as_deref(),
                &symbol_ids,
                Some(ret.range.clone()),
                ret.expression.clone(),
                NAMESPACE,
                ret,
                |_| {},
            )?;
        }
        for branch in &self.branches {
            let parent = enclosing_code_symbol(&branch.range, &symbols_by_range)
                .or_else(|| parent_lookup(branch.parent.as_deref(), &symbol_ids));
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "branch",
                &branch.id,
                DocumentNodeKind::Branch,
                parent.as_deref(),
                &symbol_ids,
                Some(branch.range.clone()),
                branch.condition.clone(),
                NAMESPACE,
                branch,
                |attrs| insert_json_attr(attrs, "kind", &branch.kind),
            )?;
        }
        for call in &self.calls {
            let parent = enclosing_code_symbol(&call.range, &symbols_by_range)
                .or_else(|| parent_lookup(call.parent.as_deref(), &symbol_ids))
                .unwrap_or_else(|| root_id.clone());
            let id = code_node_id(&identities, "call", Some(&call.id), Some(&call.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Call)
                .with_name(call.target.clone())
                .with_text(call.target.clone())
                .with_range(call.range.clone());
            insert_json_attr(&mut node.attrs, "arguments", &call.arguments);
            retain_code_native(&mut node, NAMESPACE, call);
            graph.add_node(node);
            graph.add_contains(&parent, &id);
            let target = parent_lookup(Some(&call.target), &symbol_ids)
                .unwrap_or_else(|| call.target.clone());
            graph.add_edge(code_inferred_edge(
                &parent,
                DocumentRelation::Calls,
                target,
                "grist.code.call-target-name-resolution.v1",
                0.75,
                &call.range,
            )?);
        }
        for inheritance in &self.inheritances {
            let node_id = code_node_id(
                &identities,
                "inheritance",
                Some(&inheritance.id),
                Some(&inheritance.range),
            )?;
            let mut node =
                DocumentNode::new(&node_id, DocumentNodeKind::Other("inheritance".into()))
                    .with_range(inheritance.range.clone());
            insert_json_attr(&mut node.attrs, "target", &inheritance.target);
            insert_json_attr(&mut node.attrs, "trait", &inheritance.trait_name);
            retain_code_native(&mut node, NAMESPACE, inheritance);
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);

            if let Some(trait_name) = &inheritance.trait_name {
                let source = parent_lookup(Some(&inheritance.target), &symbol_ids)
                    .unwrap_or_else(|| inheritance.target.clone());
                let target = parent_lookup(Some(trait_name), &symbol_ids)
                    .unwrap_or_else(|| trait_name.clone());
                let mut edge = code_inferred_edge(
                    source,
                    DocumentRelation::Implements,
                    target,
                    "grist.rust.impl-target-resolution.v1",
                    0.8,
                    &inheritance.range,
                )?;
                edge.extensions.insert(
                    NAMESPACE.to_string(),
                    serde_json::to_value(inheritance).unwrap_or_default(),
                );
                graph.add_edge(edge);
            }
        }

        for test in &self.tests {
            add_code_test_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                test,
                &test.id,
                &test.name,
                "rust-test",
                Some(&test.symbol_id),
                &symbol_ids,
                &test.range,
            )?;
        }
        for comment in &self.comments {
            add_code_comment_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                comment,
                &comment.id,
                &comment.text,
                comment.doc,
                &comment.range,
                &symbols_by_range,
            )?;
        }
        let syntax_ids = self
            .syntax_nodes
            .iter()
            .map(|syntax| {
                Ok((
                    syntax.id.clone(),
                    code_node_id(&identities, "syntax", Some(&syntax.id), Some(&syntax.range))?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, TransformError>>()?;
        for syntax in &self.syntax_nodes {
            add_code_syntax_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                syntax,
                &syntax.id,
                &syntax.kind,
                syntax.named,
                syntax.error,
                syntax.missing,
                syntax.parent.as_deref(),
                &syntax_ids,
                &syntax.raw,
                &syntax.range,
            )?;
        }
        for (ordinal, error) in self.parse_errors.iter().enumerate() {
            add_code_parse_error(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                error,
                ordinal,
                &error.node_kind,
                &error.raw,
                error.missing,
                &error.range,
            )?;
        }

        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}
#[cfg(feature = "javascript")]
impl ToDocumentGraph for crate::javascript::JavaScriptFile {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        use crate::javascript::JavaScriptSymbolKind;

        const NAMESPACE: &str = "grist.javascript";
        let identities = context
            .identity_generator(SchemaVersion::JAVASCRIPT_CODE_V1, "javascript")
            .map_err(projection_transform_error)?;
        let language = match self.dialect {
            crate::javascript::JavaScriptDialect::JavaScript => "javascript",
            crate::javascript::JavaScriptDialect::Jsx => "jsx",
        };
        let mut graph = code_graph_from_context(context, DocumentKind::JavaScript, language);
        graph.dialect = Some(language.to_string());
        let root_id = ensure_code_root(&mut graph, &identities, "javascript")?;
        if let Some(root) = graph.nodes.iter_mut().find(|node| node.id == root_id) {
            root.extensions.insert(
                NAMESPACE.into(),
                serde_json::json!({
                    "schema_version": self.schema_version,
                    "dialect": self.dialect,
                    "detail": self.detail,
                }),
            );
        }
        let mut symbol_ids = BTreeMap::new();
        for symbol in &self.symbols {
            let id = code_node_id(&identities, "symbol", Some(&symbol.id), Some(&symbol.range))?;
            for key in [&symbol.id, &symbol.qualified_name] {
                insert_code_symbol_alias(&mut symbol_ids, key.clone(), &id);
            }
        }
        let symbols_by_range = self
            .symbols
            .iter()
            .filter_map(|symbol| {
                symbol_ids
                    .get(&symbol.qualified_name)
                    .cloned()
                    .map(|id| (symbol.range.clone(), id))
            })
            .collect::<Vec<_>>();

        for symbol in &self.symbols {
            let id = symbol_ids[&symbol.qualified_name].clone();
            let mut node = DocumentNode::new(
                &id,
                match symbol.kind {
                    JavaScriptSymbolKind::Class => DocumentNodeKind::Class,
                    JavaScriptSymbolKind::Function => DocumentNodeKind::Function,
                    JavaScriptSymbolKind::Method => DocumentNodeKind::Method,
                    JavaScriptSymbolKind::Constructor => DocumentNodeKind::Constructor,
                    JavaScriptSymbolKind::Variable => DocumentNodeKind::Variable,
                    JavaScriptSymbolKind::Field => DocumentNodeKind::Field,
                    JavaScriptSymbolKind::Unknown => DocumentNodeKind::Symbol,
                },
            )
            .with_name(symbol.name.clone())
            .with_qualified_name(symbol.qualified_name.clone())
            .with_range(symbol.range.clone());
            let parent = symbol_parent_node(&graph.id, &symbol.qualified_name, &symbol_ids)
                .or_else(|| parent_lookup(symbol.parent.as_deref(), &symbol_ids));
            node.parent = parent.clone();
            insert_json_attr(&mut node.attrs, "visibility", &symbol.visibility);
            insert_json_attr(&mut node.attrs, "modifiers", &symbol.modifiers);
            insert_json_attr(&mut node.attrs, "decorators", &symbol.decorators);
            insert_json_attr(&mut node.attrs, "extends", &symbol.extends);
            insert_json_attr(&mut node.attrs, "implements", &symbol.implements);
            insert_json_attr(&mut node.attrs, "doc", &symbol.doc);
            retain_code_native(&mut node, NAMESPACE, symbol);
            graph.add_node(node);
            graph.add_contains(node_parent_or_root(&root_id, parent), &id);
            for (target_name, relation, rule) in symbol
                .extends
                .iter()
                .map(|target| {
                    (
                        target,
                        DocumentRelation::Inherits,
                        "grist.code.inheritance-name-resolution.v1",
                    )
                })
                .chain(symbol.implements.iter().map(|target| {
                    (
                        target,
                        DocumentRelation::Implements,
                        "grist.code.implementation-name-resolution.v1",
                    )
                }))
            {
                let target = parent_lookup(Some(target_name), &symbol_ids)
                    .unwrap_or_else(|| target_name.clone());
                graph.add_edge(code_inferred_edge(
                    &id,
                    relation,
                    target,
                    rule,
                    0.85,
                    &symbol.range,
                )?);
            }
        }

        for import in &self.imports {
            let id = code_node_id(&identities, "import", Some(&import.id), Some(&import.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Import)
                .with_name(import.module.clone())
                .with_range(import.range.clone());
            insert_json_attr(&mut node.attrs, "names", &import.names);
            insert_json_attr(&mut node.attrs, "default", &import.default);
            insert_json_attr(&mut node.attrs, "namespace", &import.namespace);
            retain_code_native(&mut node, NAMESPACE, import);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(
                DocumentEdge::new(&root_id, DocumentRelation::Imports, &id)
                    .with_range(import.range.clone()),
            );
        }
        for export in &self.exports {
            let id = code_node_id(&identities, "export", Some(&export.id), Some(&export.range))?;
            let mut node =
                DocumentNode::new(&id, DocumentNodeKind::Export).with_range(export.range.clone());
            insert_json_attr(&mut node.attrs, "names", &export.names);
            insert_json_attr(&mut node.attrs, "source", &export.source);
            retain_code_native(&mut node, NAMESPACE, export);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(
                DocumentEdge::new(&root_id, DocumentRelation::Exports, &id)
                    .with_range(export.range.clone()),
            );
        }

        for assignment in &self.assignments {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "assignment",
                &assignment.id,
                DocumentNodeKind::Assignment,
                assignment.parent.as_deref(),
                &symbol_ids,
                Some(assignment.range.clone()),
                Some(assignment.lhs.clone()),
                NAMESPACE,
                assignment,
                |attrs| {
                    insert_json_attr(attrs, "rhs", &assignment.rhs);
                    insert_json_attr(attrs, "operator", &assignment.operator);
                },
            )?;
        }
        for ret in &self.returns {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "return",
                &ret.id,
                DocumentNodeKind::Return,
                ret.parent.as_deref(),
                &symbol_ids,
                Some(ret.range.clone()),
                ret.expression.clone(),
                NAMESPACE,
                ret,
                |_| {},
            )?;
        }
        for branch in &self.branches {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "branch",
                &branch.id,
                DocumentNodeKind::Branch,
                branch.parent.as_deref(),
                &symbol_ids,
                Some(branch.range.clone()),
                branch.condition.clone(),
                NAMESPACE,
                branch,
                |attrs| insert_json_attr(attrs, "kind", &branch.kind),
            )?;
        }
        for call in &self.calls {
            let parent = parent_lookup(call.parent.as_deref(), &symbol_ids)
                .unwrap_or_else(|| root_id.clone());
            let id = code_node_id(&identities, "call", Some(&call.id), Some(&call.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Call)
                .with_name(call.target.clone())
                .with_text(call.target.clone())
                .with_range(call.range.clone());
            insert_json_attr(&mut node.attrs, "args", &call.args);
            retain_code_native(&mut node, NAMESPACE, call);
            graph.add_node(node);
            graph.add_contains(&parent, &id);
            let target = parent_lookup(Some(&call.target), &symbol_ids)
                .unwrap_or_else(|| call.target.clone());
            graph.add_edge(code_inferred_edge(
                &parent,
                DocumentRelation::Calls,
                target,
                "grist.code.call-target-name-resolution.v1",
                0.75,
                &call.range,
            )?);
        }

        for test in &self.tests {
            add_code_test_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                test,
                &test.id,
                &test.name,
                &test.framework,
                test.parent.as_deref(),
                &symbol_ids,
                &test.range,
            )?;
        }
        for comment in &self.comments {
            add_code_comment_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                comment,
                &comment.id,
                &comment.text,
                comment.doc,
                &comment.range,
                &symbols_by_range,
            )?;
        }
        let syntax_ids = self
            .syntax_nodes
            .iter()
            .map(|syntax| {
                Ok((
                    syntax.id.clone(),
                    code_node_id(&identities, "syntax", Some(&syntax.id), Some(&syntax.range))?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, TransformError>>()?;
        for syntax in &self.syntax_nodes {
            add_code_syntax_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                syntax,
                &syntax.id,
                &syntax.kind,
                syntax.named,
                syntax.error,
                syntax.missing,
                syntax.parent.as_deref(),
                &syntax_ids,
                &syntax.raw,
                &syntax.range,
            )?;
        }
        for (ordinal, error) in self.parse_errors.iter().enumerate() {
            add_code_parse_error(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                error,
                ordinal,
                &error.node_kind,
                &error.raw,
                error.missing,
                &error.range,
            )?;
        }

        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}
#[cfg(feature = "typescript")]
impl ToDocumentGraph for crate::typescript::TypeScriptFile {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        use crate::typescript::TypeScriptSymbolKind;

        const NAMESPACE: &str = "grist.typescript";
        let identities = context
            .identity_generator(SchemaVersion::TYPESCRIPT_CODE_V1, "typescript")
            .map_err(projection_transform_error)?;
        let language = match self.dialect {
            crate::typescript::TypeScriptDialect::JavaScript => "javascript",
            crate::typescript::TypeScriptDialect::TypeScript => "typescript",
            crate::typescript::TypeScriptDialect::Tsx => "tsx",
            crate::typescript::TypeScriptDialect::Jsx => "jsx",
        };
        let mut graph = code_graph_from_context(context, DocumentKind::TypeScript, language);
        graph.dialect = Some(language.to_string());
        let root_id = ensure_code_root(&mut graph, &identities, "typescript")?;
        if let Some(root) = graph.nodes.iter_mut().find(|node| node.id == root_id) {
            root.extensions.insert(
                NAMESPACE.into(),
                serde_json::json!({
                    "schema_version": self.schema_version,
                    "dialect": self.dialect,
                    "detail": self.detail,
                }),
            );
        }
        let mut symbol_ids = BTreeMap::new();
        for symbol in &self.symbols {
            let id = code_node_id(&identities, "symbol", Some(&symbol.id), Some(&symbol.range))?;
            for key in [&symbol.id, &symbol.qualified_name] {
                insert_code_symbol_alias(&mut symbol_ids, key.clone(), &id);
            }
        }
        let symbols_by_range = self
            .symbols
            .iter()
            .filter_map(|symbol| {
                symbol_ids
                    .get(&symbol.qualified_name)
                    .cloned()
                    .map(|id| (symbol.range.clone(), id))
            })
            .collect::<Vec<_>>();

        for symbol in &self.symbols {
            let id = symbol_ids[&symbol.qualified_name].clone();
            let mut node = DocumentNode::new(
                &id,
                match symbol.kind {
                    TypeScriptSymbolKind::Class => DocumentNodeKind::Class,
                    TypeScriptSymbolKind::Function => DocumentNodeKind::Function,
                    TypeScriptSymbolKind::Method => DocumentNodeKind::Method,
                    TypeScriptSymbolKind::Constructor => DocumentNodeKind::Constructor,
                    TypeScriptSymbolKind::Interface => DocumentNodeKind::Interface,
                    TypeScriptSymbolKind::TypeAlias => DocumentNodeKind::TypeAlias,
                    TypeScriptSymbolKind::Enum => DocumentNodeKind::Enum,
                    TypeScriptSymbolKind::Namespace => DocumentNodeKind::Namespace,
                    TypeScriptSymbolKind::Variable => DocumentNodeKind::Variable,
                    TypeScriptSymbolKind::Field => DocumentNodeKind::Field,
                    TypeScriptSymbolKind::Unknown => DocumentNodeKind::Symbol,
                },
            )
            .with_name(symbol.name.clone())
            .with_qualified_name(symbol.qualified_name.clone())
            .with_range(symbol.range.clone());
            let parent = symbol_parent_node(&graph.id, &symbol.qualified_name, &symbol_ids)
                .or_else(|| parent_lookup(symbol.parent.as_deref(), &symbol_ids));
            node.parent = parent.clone();
            insert_json_attr(&mut node.attrs, "visibility", &symbol.visibility);
            insert_json_attr(&mut node.attrs, "modifiers", &symbol.modifiers);
            insert_json_attr(&mut node.attrs, "decorators", &symbol.decorators);
            insert_json_attr(&mut node.attrs, "extends", &symbol.extends);
            insert_json_attr(&mut node.attrs, "implements", &symbol.implements);
            insert_json_attr(&mut node.attrs, "doc", &symbol.doc);
            retain_code_native(&mut node, NAMESPACE, symbol);
            graph.add_node(node);
            graph.add_contains(node_parent_or_root(&root_id, parent), &id);
            for (target_name, relation, rule) in symbol
                .extends
                .iter()
                .map(|target| {
                    (
                        target,
                        DocumentRelation::Inherits,
                        "grist.code.inheritance-name-resolution.v1",
                    )
                })
                .chain(symbol.implements.iter().map(|target| {
                    (
                        target,
                        DocumentRelation::Implements,
                        "grist.code.implementation-name-resolution.v1",
                    )
                }))
            {
                let target = parent_lookup(Some(target_name), &symbol_ids)
                    .unwrap_or_else(|| target_name.clone());
                graph.add_edge(code_inferred_edge(
                    &id,
                    relation,
                    target,
                    rule,
                    0.85,
                    &symbol.range,
                )?);
            }
        }

        for import in &self.imports {
            let id = code_node_id(&identities, "import", Some(&import.id), Some(&import.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Import)
                .with_name(import.module.clone())
                .with_range(import.range.clone());
            insert_json_attr(&mut node.attrs, "names", &import.names);
            insert_json_attr(&mut node.attrs, "default", &import.default);
            insert_json_attr(&mut node.attrs, "namespace", &import.namespace);
            retain_code_native(&mut node, NAMESPACE, import);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(
                DocumentEdge::new(&root_id, DocumentRelation::Imports, &id)
                    .with_range(import.range.clone()),
            );
        }
        for export in &self.exports {
            let id = code_node_id(&identities, "export", Some(&export.id), Some(&export.range))?;
            let mut node =
                DocumentNode::new(&id, DocumentNodeKind::Export).with_range(export.range.clone());
            insert_json_attr(&mut node.attrs, "names", &export.names);
            insert_json_attr(&mut node.attrs, "source", &export.source);
            retain_code_native(&mut node, NAMESPACE, export);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(
                DocumentEdge::new(&root_id, DocumentRelation::Exports, &id)
                    .with_range(export.range.clone()),
            );
        }

        for assignment in &self.assignments {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "assignment",
                &assignment.id,
                DocumentNodeKind::Assignment,
                assignment.parent.as_deref(),
                &symbol_ids,
                Some(assignment.range.clone()),
                Some(assignment.lhs.clone()),
                NAMESPACE,
                assignment,
                |attrs| {
                    insert_json_attr(attrs, "rhs", &assignment.rhs);
                    insert_json_attr(attrs, "operator", &assignment.operator);
                },
            )?;
        }
        for ret in &self.returns {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "return",
                &ret.id,
                DocumentNodeKind::Return,
                ret.parent.as_deref(),
                &symbol_ids,
                Some(ret.range.clone()),
                ret.expression.clone(),
                NAMESPACE,
                ret,
                |_| {},
            )?;
        }
        for branch in &self.branches {
            add_code_fact_node(
                &mut graph,
                &identities,
                &root_id,
                "branch",
                &branch.id,
                DocumentNodeKind::Branch,
                branch.parent.as_deref(),
                &symbol_ids,
                Some(branch.range.clone()),
                branch.condition.clone(),
                NAMESPACE,
                branch,
                |attrs| insert_json_attr(attrs, "kind", &branch.kind),
            )?;
        }
        for call in &self.calls {
            let parent = parent_lookup(call.parent.as_deref(), &symbol_ids)
                .unwrap_or_else(|| root_id.clone());
            let id = code_node_id(&identities, "call", Some(&call.id), Some(&call.range))?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Call)
                .with_name(call.target.clone())
                .with_text(call.target.clone())
                .with_range(call.range.clone());
            insert_json_attr(&mut node.attrs, "args", &call.args);
            retain_code_native(&mut node, NAMESPACE, call);
            graph.add_node(node);
            graph.add_contains(&parent, &id);
            let target = parent_lookup(Some(&call.target), &symbol_ids)
                .unwrap_or_else(|| call.target.clone());
            graph.add_edge(code_inferred_edge(
                &parent,
                DocumentRelation::Calls,
                target,
                "grist.code.call-target-name-resolution.v1",
                0.75,
                &call.range,
            )?);
        }

        for test in &self.tests {
            add_code_test_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                test,
                &test.id,
                &test.name,
                &test.framework,
                test.parent.as_deref(),
                &symbol_ids,
                &test.range,
            )?;
        }
        for comment in &self.comments {
            add_code_comment_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                comment,
                &comment.id,
                &comment.text,
                comment.doc,
                &comment.range,
                &symbols_by_range,
            )?;
        }
        let syntax_ids = self
            .syntax_nodes
            .iter()
            .map(|syntax| {
                Ok((
                    syntax.id.clone(),
                    code_node_id(&identities, "syntax", Some(&syntax.id), Some(&syntax.range))?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, TransformError>>()?;
        for syntax in &self.syntax_nodes {
            add_code_syntax_node(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                syntax,
                &syntax.id,
                &syntax.kind,
                syntax.named,
                syntax.error,
                syntax.missing,
                syntax.parent.as_deref(),
                &syntax_ids,
                &syntax.raw,
                &syntax.range,
            )?;
        }
        for (ordinal, error) in self.parse_errors.iter().enumerate() {
            add_code_parse_error(
                &mut graph,
                &identities,
                &root_id,
                NAMESPACE,
                error,
                ordinal,
                &error.node_kind,
                &error.raw,
                error.missing,
                &error.range,
            )?;
        }

        graph
            .finalize_projection(&identities)
            .map_err(projection_transform_error)?;
        Ok(graph)
    }
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn code_graph_from_context(
    context: DocumentGraphContext,
    kind: DocumentKind,
    default_language: &str,
) -> DocumentGraph {
    let (payload_kind, payload_schema_version, projection_rule) = match &kind {
        DocumentKind::Python => (
            "python-code",
            SchemaVersion::PYTHON_CODE_V1,
            "grist.python.to-document-graph.v2",
        ),
        DocumentKind::Rust => (
            "rust-code",
            SchemaVersion::RUST_CODE_V1,
            "grist.rust.to-document-graph.v2",
        ),
        DocumentKind::JavaScript => (
            "javascript-code",
            SchemaVersion::JAVASCRIPT_CODE_V1,
            "grist.javascript.to-document-graph.v2",
        ),
        DocumentKind::TypeScript => (
            "typescript-code",
            SchemaVersion::TYPESCRIPT_CODE_V1,
            "grist.typescript.to-document-graph.v2",
        ),
        _ => (
            "code",
            SchemaVersion::DOCUMENT_GRAPH_V2,
            "grist.code.to-document-graph.v2",
        ),
    };
    let mut graph = DocumentGraph::new(context.graph_id, kind).with_projection(
        payload_kind,
        payload_schema_version,
        projection_rule,
    );
    graph.source = context.source;
    graph.language = Some(
        context
            .language
            .unwrap_or_else(|| default_language.to_string()),
    );
    graph.dialect = context.dialect;
    graph.attrs = context.attrs;
    graph
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn ensure_code_root(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    language: &str,
) -> Result<String, TransformError> {
    let root_id =
        stable_projection_node_id(identities, vec!["module".to_string()], Some("root"), None)?;
    graph.add_node(
        DocumentNode::new(&root_id, DocumentNodeKind::Module)
            .with_name(language)
            .with_qualified_name(language)
            .with_ordinal(0),
    );
    Ok(root_id)
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn code_node_id(
    identities: &GraphIdGenerator,
    category: &str,
    native_id: Option<&str>,
    range: Option<&SourceRange>,
) -> Result<String, TransformError> {
    let mut path = vec!["module".to_string(), category.to_string()];
    if native_id.is_none()
        && let Some(range) = range
    {
        path.push(range.byte_start.to_string());
        path.push(range.byte_end.to_string());
    }
    stable_projection_node_id(identities, path, native_id, range)
}
#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn insert_code_symbol_alias(
    symbol_ids: &mut BTreeMap<String, String>,
    alias: impl Into<String>,
    id: &str,
) {
    let id = id.to_string();
    symbol_ids
        .entry(alias.into())
        .and_modify(|existing| {
            if !existing.is_empty() && *existing != id {
                existing.clear();
            }
        })
        .or_insert(id);
}

#[cfg(feature = "rust")]
fn enclosing_code_symbol(range: &SourceRange, symbols: &[(SourceRange, String)]) -> Option<String> {
    symbols
        .iter()
        .filter(|(candidate, _)| {
            candidate.byte_start <= range.byte_start
                && candidate.byte_end >= range.byte_end
                && (candidate.byte_start != range.byte_start
                    || candidate.byte_end != range.byte_end)
        })
        .min_by_key(|(candidate, _)| candidate.byte_end.saturating_sub(candidate.byte_start))
        .map(|(_, id)| id.clone())
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn retain_code_native<T: Serialize>(node: &mut DocumentNode, namespace: &str, native: &T) {
    if let Ok(value) = serde_json::to_value(native) {
        node.extensions.insert(namespace.to_string(), value);
    }
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn code_inferred_edge(
    source: impl Into<String>,
    relation: DocumentRelation,
    target: impl Into<String>,
    rule: &str,
    confidence: f64,
    range: &SourceRange,
) -> Result<DocumentEdge, TransformError> {
    let confidence = LocatorConfidence::new(confidence).map_err(projection_transform_error)?;
    let mut edge = DocumentEdge::inferred(source, relation, target, rule, confidence);
    edge.range = Some(range.clone());
    if let Ok(locator) = SourceLocator::try_from(range.clone()) {
        edge = edge.with_inference_evidence(locator);
    }
    Ok(edge)
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
#[allow(clippy::too_many_arguments)]
fn add_code_test_node<T: Serialize>(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    root_id: &str,
    namespace: &str,
    native: &T,
    native_id: &str,
    name: &str,
    framework: &str,
    parent: Option<&str>,
    symbol_ids: &BTreeMap<String, String>,
    range: &SourceRange,
) -> Result<(), TransformError> {
    let parent = parent_lookup(parent, symbol_ids).unwrap_or_else(|| root_id.to_string());
    let id = code_node_id(identities, "test", Some(native_id), Some(range))?;
    let mut node = DocumentNode::new(&id, DocumentNodeKind::CodeSymbol)
        .with_name(name)
        .with_range(range.clone());
    insert_json_attr(&mut node.attrs, "framework", &framework);
    retain_code_native(&mut node, namespace, native);
    graph.add_node(node);
    graph.add_contains(&parent, &id);
    graph.add_edge(
        DocumentEdge::new(parent, DocumentRelation::Other("tests".into()), id)
            .with_range(range.clone()),
    );
    Ok(())
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
#[allow(clippy::too_many_arguments)]
fn add_code_comment_node<T: Serialize>(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    root_id: &str,
    namespace: &str,
    native: &T,
    native_id: &str,
    text: &str,
    doc: bool,
    range: &SourceRange,
    symbols: &[(SourceRange, String)],
) -> Result<(), TransformError> {
    let id = code_node_id(identities, "comment", Some(native_id), Some(range))?;
    let mut node = DocumentNode::new(&id, DocumentNodeKind::Comment)
        .with_text(text)
        .with_range(range.clone());
    insert_json_attr(&mut node.attrs, "documentation", &doc);
    retain_code_native(&mut node, namespace, native);
    graph.add_node(node);
    graph.add_contains(root_id, &id);

    if doc
        && let Some((_, target)) = symbols
            .iter()
            .filter(|(symbol_range, _)| symbol_range.byte_start >= range.byte_end)
            .min_by_key(|(symbol_range, _)| symbol_range.byte_start)
    {
        graph.add_edge(code_inferred_edge(
            &id,
            DocumentRelation::Annotates,
            target,
            "grist.code.leading-documentation.v1",
            0.85,
            range,
        )?);
    }
    Ok(())
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
#[allow(clippy::too_many_arguments)]
fn add_code_syntax_node<T: Serialize>(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    root_id: &str,
    namespace: &str,
    native: &T,
    native_id: &str,
    grammar_kind: &str,
    named: bool,
    error: bool,
    missing: bool,
    parent: Option<&str>,
    syntax_ids: &BTreeMap<String, String>,
    _raw: &str,
    range: &SourceRange,
) -> Result<(), TransformError> {
    let id = code_node_id(identities, "syntax", Some(native_id), Some(range))?;
    let mut node = DocumentNode::new(&id, DocumentNodeKind::Other("grammar_node".into()))
        .with_range(range.clone());
    insert_json_attr(&mut node.attrs, "grammar_kind", &grammar_kind);
    insert_json_attr(&mut node.attrs, "named", &named);
    insert_json_attr(&mut node.attrs, "error", &error);
    insert_json_attr(&mut node.attrs, "missing", &missing);
    retain_code_native(&mut node, namespace, native);
    if let Ok(payload) = serde_json::to_value(native)
        && let Ok(retained) = RawNodeContent::new(namespace, grammar_kind, payload)
    {
        node.raw = Some(retained);
    }
    graph.add_node(node);
    let parent = parent
        .and_then(|value| syntax_ids.get(value))
        .cloned()
        .unwrap_or_else(|| root_id.to_string());
    graph.add_contains(parent, id);
    Ok(())
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
#[allow(clippy::too_many_arguments)]
fn add_code_parse_error<T: Serialize>(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    root_id: &str,
    namespace: &str,
    native: &T,
    _ordinal: usize,
    grammar_kind: &str,
    _raw: &str,
    missing: bool,
    range: &SourceRange,
) -> Result<(), TransformError> {
    let native_bytes = canonical_json_bytes(native).unwrap_or_default();
    let digest = sha256_hex(&native_bytes);
    let native_id = format!("{}:{}:{}", range.byte_start, range.byte_end, digest);
    let id = code_node_id(identities, "parse_error", Some(&native_id), Some(range))?;
    let mut node = DocumentNode::new(&id, DocumentNodeKind::Diagnostic).with_range(range.clone());
    insert_json_attr(&mut node.attrs, "grammar_kind", &grammar_kind);
    insert_json_attr(&mut node.attrs, "missing", &missing);
    retain_code_native(&mut node, namespace, native);
    graph.add_node(node);
    graph.add_contains(root_id, id);
    Ok(())
}

#[cfg(any(
    feature = "latex",
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn insert_json_attr<T: Serialize>(attrs: &mut AttrMap, key: &str, value: &T) {
    if let Ok(value) = serde_json::to_value(value) {
        if !value.is_null() {
            attrs.insert(key.to_string(), value);
        }
    }
}

#[cfg(any(feature = "javascript", feature = "python", feature = "typescript"))]
fn symbol_parent_node(
    _graph_id: &str,
    qualified_name: &str,
    symbol_ids: &BTreeMap<String, String>,
) -> Option<String> {
    let (parent, _) = qualified_name.rsplit_once('.')?;
    symbol_ids.get(parent).cloned()
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn parent_lookup(parent: Option<&str>, symbol_ids: &BTreeMap<String, String>) -> Option<String> {
    let parent = parent?;
    if let Some(id) = symbol_ids.get(parent).filter(|id| !id.is_empty()) {
        return Some(id.clone());
    }
    let suffixes = [format!(".{parent}"), format!("::{parent}")];
    let matches = symbol_ids
        .iter()
        .filter(|(qualified, id)| {
            !id.is_empty() && suffixes.iter().any(|suffix| qualified.ends_with(suffix))
        })
        .map(|(_, id)| id.clone())
        .collect::<BTreeSet<_>>();
    (matches.len() == 1)
        .then(|| matches.into_iter().next())
        .flatten()
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
fn node_parent_or_root(root_id: &str, parent: Option<String>) -> String {
    parent.unwrap_or_else(|| root_id.to_string())
}

#[cfg(any(
    feature = "javascript",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]
#[allow(clippy::too_many_arguments)]
fn add_code_fact_node<F, T>(
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    root_id: &str,
    category: &str,
    id: &str,
    kind: DocumentNodeKind,
    parent: Option<&str>,
    symbol_ids: &BTreeMap<String, String>,
    range: Option<SourceRange>,
    text: Option<String>,
    namespace: &str,
    native: &T,
    add_attrs: F,
) -> Result<(), TransformError>
where
    F: FnOnce(&mut AttrMap),
    T: Serialize,
{
    let parent = parent_lookup(parent, symbol_ids).unwrap_or_else(|| root_id.to_string());
    let node_id = code_node_id(identities, category, Some(id), range.as_ref())?;
    let mut node = DocumentNode::new(&node_id, kind.clone());
    if let Some(range) = range.clone() {
        node = node.with_range(range);
    }
    node.text = text;
    add_attrs(&mut node.attrs);
    retain_code_native(&mut node, namespace, native);
    graph.add_node(node);
    graph.add_contains(&parent, &node_id);
    let relation = match kind {
        DocumentNodeKind::Assignment => Some(DocumentRelation::Assigns),
        DocumentNodeKind::Return => Some(DocumentRelation::Returns),
        DocumentNodeKind::Branch => Some(DocumentRelation::ConditionalOn),
        _ => None,
    };
    if let (Some(relation), Some(range)) = (relation, range) {
        graph.add_edge(DocumentEdge::new(parent, relation, node_id).with_range(range));
    }
    Ok(())
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

        assert_eq!(graph.schema_version, SchemaVersion::DOCUMENT_GRAPH_V2);
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
    fn conditional_obligation_graph_serializes_with_provenance() {
        let mut graph = DocumentGraph::new("graph:obligation", DocumentKind::Document);
        graph.add_node(DocumentNode::new(
            "condition:file-executable",
            DocumentNodeKind::Condition,
        ));
        let obligation = ObligationAttrs {
            modality: ObligationModality::Must,
            polarity: ObligationPolarity::Positive,
            subject: Some("file".to_string()),
            predicate: Some("has_shebang".to_string()),
            action: Some("include shebang".to_string()),
            source_text: Some("If a file is executable, it must have a shebang.".to_string()),
            extraction_method: Some("test-fixture".to_string()),
            confidence: Some(1.0),
            attrs: AttrMap::new(),
        };
        graph.add_node(
            DocumentNode::new("obligation:has-shebang", DocumentNodeKind::Obligation)
                .with_attr("obligation", serde_json::to_value(&obligation).unwrap()),
        );
        graph.add_edge(DocumentEdge::new(
            "obligation:has-shebang",
            DocumentRelation::ConditionalOn,
            "condition:file-executable",
        ));
        graph.add_edge(DocumentEdge::new(
            "obligation:has-shebang",
            DocumentRelation::Requires,
            "state:has-shebang",
        ));
        graph.add_edge(DocumentEdge::new(
            "obligation:has-shebang",
            DocumentRelation::DerivedFrom,
            "source:line-1",
        ));

        let value = serde_json::to_value(&graph).expect("graph should serialize");
        let decoded: DocumentGraph =
            serde_json::from_value(value).expect("graph should deserialize");
        assert!(
            decoded
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::ConditionalOn)
        );
        let obligation_value = decoded
            .nodes
            .iter()
            .find(|node| node.kind == DocumentNodeKind::Obligation)
            .and_then(|node| node.attrs.get("obligation"))
            .expect("obligation attrs should be preserved");
        let decoded_attrs: ObligationAttrs = serde_json::from_value(obligation_value.clone())
            .expect("obligation attrs should decode");
        assert_eq!(decoded_attrs.modality, ObligationModality::Must);
        assert_eq!(decoded_attrs.predicate.as_deref(), Some("has_shebang"));
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

    #[test]
    fn conditional_obligation_extraction_handles_positive_and_negative_cases() {
        let mut graph = DocumentGraph::new("graph:rules", DocumentKind::Document);
        graph.add_node(
            DocumentNode::new("p:positive", DocumentNodeKind::Paragraph)
                .with_text("If the file is executable, it must have a shebang."),
        );
        graph.add_node(
            DocumentNode::new("p:negative", DocumentNodeKind::Paragraph)
                .with_text("This paragraph mentions quality but has no explicit condition."),
        );

        let count = extract_conditional_obligations(&mut graph);
        assert_eq!(count, 1);
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::ConditionalOn)
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::Requires)
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.kind == DocumentNodeKind::Obligation)
                .count(),
            1
        );
    }

    #[cfg(feature = "latex")]
    #[test]
    fn conditional_obligation_extraction_works_on_latex_projection() {
        use crate::core::SourceInfo;
        use crate::latex::{LatexOptions, parse_latex};

        let parsed = parse_latex(
            "If the file is executable, it must have a shebang.\n",
            SourceInfo::stdin("rules.tex"),
            &LatexOptions::default(),
        );
        let mut graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:latex-rules"))
            .expect("latex projection should succeed");
        assert_eq!(extract_conditional_obligations(&mut graph), 1);
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::ConditionalOn)
        );
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_projection_preserves_structure_links_and_tables() {
        use crate::core::SourceInfo;
        use crate::markdown::parse_markdown;

        let src = "---\ntitle: Demo\n---\n# Intro\n\nSee [site](https://example.com).\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let parsed = parse_markdown(src, SourceInfo::stdin("demo.md"));
        let graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:markdown"))
            .expect("markdown graph projection should succeed");

        assert_eq!(graph.kind, DocumentKind::Markdown);
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::Frontmatter)
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::Heading
                    && node.text.as_deref() == Some("Intro"))
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::LinksTo
                    && edge.target == "https://example.com")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::TableRow)
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::TableCell
                    && node.text.as_deref() == Some("1"))
        );
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_renderer_outputs_supported_prose_graph() {
        use crate::core::SourceInfo;
        use crate::markdown::parse_markdown;

        let src =
            "# Intro\n\nSee [site](https://example.com).\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let parsed = parse_markdown(src, SourceInfo::stdin("demo.md"));
        let graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:markdown"))
            .expect("markdown graph projection should succeed");
        let rendered = render_markdown(&graph, TransformOptions::default())
            .expect("markdown rendering should succeed");

        assert!(rendered.contains("# Intro"));
        assert!(rendered.contains("[site](https://example.com)"));
        assert!(rendered.contains("|A | B|"));
        assert!(rendered.contains("|1 | 2|"));
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_renderer_rejects_unsupported_nodes_without_lossy_mode() {
        let mut graph = DocumentGraph::new("graph:code", DocumentKind::Markdown);
        graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document));
        graph.add_node(DocumentNode::new("fn", DocumentNodeKind::Function));
        graph.add_contains("root", "fn");

        let err = render_markdown(&graph, TransformOptions::default())
            .expect_err("unsupported code node should fail without lossy mode");
        assert_eq!(
            err.diagnostic_code(),
            "document_graph.unsupported_node_kind"
        );

        let rendered = render_markdown(
            &graph,
            TransformOptions {
                allow_lossy: true,
                ..TransformOptions::default()
            },
        )
        .expect("lossy rendering should skip unsupported nodes");
        assert!(rendered.is_empty());
    }

    #[cfg(all(feature = "markdown", feature = "latex"))]
    #[test]
    fn latex_renderer_outputs_markdown_graph_as_latex() {
        use crate::core::SourceInfo;
        use crate::markdown::parse_markdown;

        let parsed = parse_markdown(
            "# Intro\n\nHello **world**.\n",
            SourceInfo::stdin("demo.md"),
        );
        let graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:markdown"))
            .expect("markdown graph projection should succeed");
        let rendered = render_latex(&graph, TransformOptions::default())
            .expect("latex rendering should succeed");
        assert!(rendered.contains("\\section{Intro}"));
        assert!(rendered.contains("Hello world."));
    }

    #[cfg(feature = "latex")]
    #[test]
    fn latex_renderer_outputs_latex_graph_as_latex() {
        use crate::core::SourceInfo;
        use crate::latex::{LatexOptions, parse_latex};

        let src = "\\section{Intro}\nSee \\label{sec:intro} \\ref{sec:intro} \\cite{paper} and $x$. \\unknowncmd{raw}\n";
        let parsed = parse_latex(
            src,
            SourceInfo::stdin("paper.tex"),
            &LatexOptions::default(),
        );
        let graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:latex"))
            .expect("latex graph projection should succeed");
        let rendered = render_latex(&graph, TransformOptions::default())
            .expect("latex rendering should succeed");
        assert!(rendered.contains("\\section{Intro}"));
        assert!(rendered.contains("\\label{sec:intro}"));
        assert!(rendered.contains("\\ref{sec:intro}"));
        assert!(rendered.contains("\\cite{paper}"));
        assert!(rendered.contains("$x$"));
        assert!(!rendered.contains("\\unknowncmd{raw}"));
        assert!(rendered.contains("\\textbackslash{}unknowncmd"));
    }

    #[cfg(feature = "latex")]
    #[test]
    fn latex_projection_preserves_sections_math_refs_citations_and_raw() {
        use crate::core::SourceInfo;
        use crate::latex::{LatexOptions, parse_latex};

        let src = "\\section{Intro}\nSee \\label{sec:intro} \\ref{sec:intro} \\cite{paper} and $x$. \\unknowncmd{raw}\n";
        let parsed = parse_latex(
            src,
            SourceInfo::stdin("paper.tex"),
            &LatexOptions::default(),
        );
        let graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:latex"))
            .expect("latex graph projection should succeed");

        assert_eq!(graph.kind, DocumentKind::Latex);
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::Heading
                    && node.text.as_deref() == Some("Intro"))
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::MathInline
                    && node.text.as_deref() == Some("x"))
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::Label
                    && node.text.as_deref() == Some("sec:intro"))
        );
        assert!(graph.edges.iter().any(
            |edge| edge.relation == DocumentRelation::References && edge.target == "sec:intro"
        ));
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::Cites && edge.target == "paper")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::RawInline
                    && node.attrs.get("command").and_then(Value::as_str) == Some("unknowncmd"))
        );
    }

    #[cfg(feature = "python")]
    #[test]
    fn python_projection_emits_symbols_imports_calls_and_inherits() {
        use crate::core::SourceInfo;
        use crate::python::{PythonDetailMode, PythonIngestOptions, parse_python};

        let src = "from base import Base\nclass Form(Base):\n    def create(self):\n        return helper()\ndef helper():\n    return 1\n";
        let parsed = parse_python(
            src,
            SourceInfo::stdin("forms.py"),
            &PythonIngestOptions {
                detail: PythonDetailMode::Semantic,
            },
        );
        let graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:python"))
            .expect("python graph projection should succeed");
        assert_eq!(graph.kind, DocumentKind::Python);
        assert!(graph.nodes.iter().any(
            |node| node.kind == DocumentNodeKind::Class && node.name.as_deref() == Some("Form")
        ));
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::Inherits && edge.target == "Base")
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::Calls && edge.target == "helper")
        );
    }

    #[cfg(feature = "rust")]
    #[test]
    fn rust_projection_emits_symbols_and_imports() {
        use crate::core::SourceInfo;
        use crate::rust::{RustDetailMode, RustIngestOptions, parse_rust};

        let src = "use std::fmt;\npub struct Form;\nimpl Form { pub fn create(&self) {} }\n";
        let parsed = parse_rust(
            src,
            SourceInfo::stdin("lib.rs"),
            &RustIngestOptions {
                detail: RustDetailMode::Semantic,
            },
        );
        let graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:rust"))
            .expect("rust graph projection should succeed");
        assert_eq!(graph.kind, DocumentKind::Rust);
        assert!(graph.nodes.iter().any(
            |node| node.kind == DocumentNodeKind::Class && node.name.as_deref() == Some("Form")
        ));
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::Imports)
        );
    }

    #[cfg(feature = "typescript")]
    #[test]
    fn typescript_projection_emits_symbols_imports_exports_and_calls() {
        use crate::core::SourceInfo;
        use crate::typescript::{
            TypeScriptDetailMode, TypeScriptDialect, TypeScriptIngestOptions, parse_typescript,
        };

        let src = "import { helper } from './helper';\nexport class Form { create() { return helper(); } }\n";
        let parsed = parse_typescript(
            src,
            SourceInfo::stdin("form.ts"),
            &TypeScriptIngestOptions {
                dialect: TypeScriptDialect::TypeScript,
                detail: TypeScriptDetailMode::Semantic,
            },
        );
        let graph = parsed
            .payload
            .as_ref()
            .expect("complete operation payload")
            .to_document_graph(DocumentGraphContext::new("graph:ts"))
            .expect("typescript graph projection should succeed");
        assert_eq!(graph.kind, DocumentKind::TypeScript);
        assert!(graph.nodes.iter().any(
            |node| node.kind == DocumentNodeKind::Class && node.name.as_deref() == Some("Form")
        ));
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::Imports)
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.relation == DocumentRelation::Exports)
        );
    }
}
