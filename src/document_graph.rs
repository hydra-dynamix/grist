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

#[cfg(feature = "markdown")]
impl ToDocumentGraph for crate::markdown::MarkdownDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        use crate::markdown::MarkdownNodeKind;

        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Markdown);
        graph.source = context.source;
        graph.language = Some(context.language.unwrap_or_else(|| "markdown".to_string()));
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;

        let root_id = format!("{}:root", graph.id);
        graph.add_node(DocumentNode::new(&root_id, DocumentNodeKind::Document).with_ordinal(0));

        let mut ordinal = 1_usize;
        if let Some(frontmatter) = &self.frontmatter {
            let node_id = format!("{}:frontmatter", graph.id);
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Frontmatter)
                .with_range(frontmatter.range.clone())
                .with_text(frontmatter.raw.clone())
                .with_ordinal(ordinal);
            if let Some(value) = &frontmatter.value {
                node.attrs.insert("value".to_string(), value.clone());
            }
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            ordinal += 1;
        }

        for md_node in &self.nodes {
            let node_id = markdown_node_id(&graph.id, &md_node.id);
            let kind = match md_node.kind {
                MarkdownNodeKind::Heading => DocumentNodeKind::Heading,
                MarkdownNodeKind::Paragraph => DocumentNodeKind::Paragraph,
                MarkdownNodeKind::CodeFence => DocumentNodeKind::CodeBlock,
                MarkdownNodeKind::Link => DocumentNodeKind::Link,
                MarkdownNodeKind::Table => DocumentNodeKind::Table,
                MarkdownNodeKind::Text => DocumentNodeKind::Text,
            };
            let mut node = DocumentNode::new(&node_id, kind).with_ordinal(ordinal);
            node.range = md_node.range.clone();
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
            if let Some(table) = &md_node.table {
                if let Ok(value) = serde_json::to_value(table) {
                    node.attrs.insert("table".to_string(), value);
                }
            }
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);

            if let Some(destination) = &md_node.destination {
                graph.add_edge(
                    DocumentEdge::new(&node_id, DocumentRelation::LinksTo, destination)
                        .with_attr("target_kind", "uri"),
                );
            }

            if let Some(table) = &md_node.table {
                project_markdown_table(&mut graph, &node_id, table);
            }
            ordinal += 1;
        }

        Ok(graph)
    }
}

#[cfg(feature = "markdown")]
fn markdown_node_id(graph_id: &str, node_id: &str) -> String {
    format!("{}:{}", graph_id, node_id)
}

#[cfg(feature = "markdown")]
fn insert_opt_attr(attrs: &mut AttrMap, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        attrs.insert(key.to_string(), value);
    }
}

#[cfg(feature = "markdown")]
fn project_markdown_table(
    graph: &mut DocumentGraph,
    table_node_id: &str,
    table: &crate::markdown::MarkdownTable,
) {
    for (row_idx, row) in table.row_details.iter().enumerate() {
        let row_id = format!("{}:row:{}", table_node_id, row_idx);
        let mut row_node = DocumentNode::new(&row_id, DocumentNodeKind::TableRow)
            .with_ordinal(row_idx)
            .with_attr("header", row.header);
        row_node.range = row.range.clone();
        graph.add_node(row_node);
        graph.add_contains(table_node_id, &row_id);

        for (cell_idx, cell) in row.cells.iter().enumerate() {
            let cell_id = format!("{}:cell:{}", row_id, cell_idx);
            let mut cell_node = DocumentNode::new(&cell_id, DocumentNodeKind::TableCell)
                .with_text(cell.text.clone())
                .with_ordinal(cell_idx);
            cell_node.range = cell.range.clone();
            if let Some(alignment) = table.alignments.get(cell_idx) {
                if let Ok(value) = serde_json::to_value(alignment) {
                    cell_node.attrs.insert("alignment".to_string(), value);
                }
            }
            graph.add_node(cell_node);
            graph.add_contains(&row_id, &cell_id);
        }
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

#[cfg(feature = "markdown")]
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
            .map(|raw| format!("---\n{}\n---\n", raw.trim_matches('\n')))
            .unwrap_or_default()),
        DocumentNodeKind::Heading => {
            let level = node
                .attrs
                .get("level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6);
            Ok(format!(
                "{} {}\n",
                "#".repeat(level as usize),
                node.text.as_deref().unwrap_or("")
            ))
        }
        DocumentNodeKind::Paragraph => Ok(format!("{}\n", node.text.as_deref().unwrap_or(""))),
        DocumentNodeKind::Text => Ok(node.text.clone().unwrap_or_default()),
        DocumentNodeKind::Link => {
            let destination = node
                .attrs
                .get("destination")
                .and_then(Value::as_str)
                .ok_or_else(|| TransformError::MissingRequiredAttribute {
                    target: node.id.clone(),
                    attr: "destination".to_string(),
                })?;
            let text = node.text.as_deref().unwrap_or(destination);
            Ok(format!("[{text}]({destination})\n"))
        }
        DocumentNodeKind::CodeBlock => {
            let language = node
                .attrs
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("");
            Ok(format!(
                "```{language}\n{}\n```\n",
                node.text.as_deref().unwrap_or("")
            ))
        }
        DocumentNodeKind::Table => render_markdown_table_node(node, graph),
        DocumentNodeKind::RawBlock | DocumentNodeKind::RawInline if options.allow_raw_fallback => {
            Ok(node.text.clone().unwrap_or_default())
        }
        _ if options.allow_lossy => Ok(String::new()),
        _ => Err(TransformError::UnsupportedNodeKind {
            node_id: node.id.clone(),
            node_kind: node.kind.clone(),
        }),
    }
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
    out.push_str(&rows[0].join(" | "));
    out.push_str("|\n|");
    out.push_str(&vec!["---"; rows[0].len()].join("|"));
    out.push_str("|\n");
    for row in rows.iter().skip(1) {
        out.push('|');
        out.push_str(&row.join(" | "));
        out.push_str("|\n");
    }
    out
}

#[cfg(feature = "python")]
impl ToDocumentGraph for crate::python::PythonFile {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        use crate::python::PythonSymbolKind;

        let mut graph = code_graph_from_context(context, DocumentKind::Python, "python");
        let root_id = ensure_code_root(&mut graph, "python");
        let symbol_ids = self
            .symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.qualified_name.clone(),
                    code_node_id(&graph.id, "symbol", &symbol.id),
                )
            })
            .collect::<BTreeMap<_, _>>();

        for symbol in &self.symbols {
            let id = code_node_id(&graph.id, "symbol", &symbol.id);
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
            .with_qualified_name(symbol.qualified_name.clone());
            node.range = Some(symbol.range.clone());
            node.parent = symbol_parent_node(&graph.id, &symbol.qualified_name, &symbol_ids);
            insert_json_attr(&mut node.attrs, "visibility", &symbol.visibility);
            insert_json_attr(&mut node.attrs, "decorators", &symbol.decorators);
            insert_json_attr(&mut node.attrs, "superclasses", &symbol.superclasses);
            insert_json_attr(&mut node.attrs, "doc", &symbol.doc);
            graph.add_node(node);
            graph.add_contains(
                node_parent_or_root(
                    &root_id,
                    symbol_parent_node(&graph.id, &symbol.qualified_name, &symbol_ids),
                ),
                &id,
            );

            for superclass in &symbol.superclasses {
                graph.add_edge(DocumentEdge::new(
                    &id,
                    DocumentRelation::Inherits,
                    superclass,
                ));
            }
        }

        for import in &self.imports {
            let id = code_node_id(&graph.id, "import", &import.id);
            let mut node =
                DocumentNode::new(&id, DocumentNodeKind::Import).with_name(import.module.clone());
            node.range = Some(import.range.clone());
            insert_json_attr(&mut node.attrs, "names", &import.names);
            insert_json_attr(&mut node.attrs, "aliases", &import.aliases);
            insert_json_attr(&mut node.attrs, "level", &import.level);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(DocumentEdge::new(&root_id, DocumentRelation::Imports, &id));
        }

        for assignment in &self.assignments {
            add_code_fact_node(
                &mut graph,
                &root_id,
                "assignment",
                &assignment.id,
                DocumentNodeKind::Assignment,
                assignment.parent.as_deref(),
                &symbol_ids,
                Some(assignment.range.clone()),
                Some(assignment.lhs.clone()),
                |attrs| {
                    insert_json_attr(attrs, "rhs", &assignment.rhs);
                    insert_json_attr(attrs, "operator", &assignment.operator);
                },
            );
        }
        for ret in &self.returns {
            add_code_fact_node(
                &mut graph,
                &root_id,
                "return",
                &ret.id,
                DocumentNodeKind::Return,
                ret.parent.as_deref(),
                &symbol_ids,
                Some(ret.range.clone()),
                ret.expression.clone(),
                |_| {},
            );
        }
        for branch in &self.branches {
            add_code_fact_node(
                &mut graph,
                &root_id,
                "branch",
                &branch.id,
                DocumentNodeKind::Branch,
                branch.parent.as_deref(),
                &symbol_ids,
                Some(branch.range.clone()),
                branch.condition.clone(),
                |attrs| insert_json_attr(attrs, "kind", &branch.kind),
            );
        }
        for call in &self.calls {
            let parent = parent_lookup(call.parent.as_deref(), &symbol_ids)
                .unwrap_or_else(|| root_id.clone());
            let id = code_node_id(&graph.id, "call", &call.id);
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Call)
                .with_name(call.target.clone())
                .with_text(call.target.clone());
            node.range = Some(call.range.clone());
            insert_json_attr(&mut node.attrs, "args", &call.args);
            graph.add_node(node);
            graph.add_contains(&parent, &id);
            graph.add_edge(DocumentEdge::new(
                &parent,
                DocumentRelation::Calls,
                call.target.clone(),
            ));
        }

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

        let mut graph = code_graph_from_context(context, DocumentKind::Rust, "rust");
        let root_id = ensure_code_root(&mut graph, "rust");
        let symbol_ids = self
            .symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.name.clone(),
                    code_node_id(&graph.id, "symbol", &symbol.id),
                )
            })
            .collect::<BTreeMap<_, _>>();

        for symbol in &self.symbols {
            let id = code_node_id(&graph.id, "symbol", &symbol.id);
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
                    RustSymbolKind::MacroDefinition | RustSymbolKind::MacroInvocation => {
                        DocumentNodeKind::Symbol
                    }
                    RustSymbolKind::Impl | RustSymbolKind::Union | RustSymbolKind::Unknown => {
                        DocumentNodeKind::Symbol
                    }
                },
            )
            .with_name(symbol.name.clone())
            .with_qualified_name(symbol.name.clone());
            node.range = Some(symbol.range.clone());
            node.parent = parent_lookup(symbol.parent.as_deref(), &symbol_ids);
            insert_json_attr(&mut node.attrs, "visibility", &symbol.visibility);
            insert_json_attr(&mut node.attrs, "attributes", &symbol.attributes);
            insert_json_attr(&mut node.attrs, "doc", &symbol.doc);
            graph.add_node(node);
            graph.add_contains(
                node_parent_or_root(
                    &root_id,
                    parent_lookup(symbol.parent.as_deref(), &symbol_ids),
                ),
                &id,
            );
        }

        for import in &self.imports {
            let id = code_node_id(&graph.id, "import", &import.id);
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Import)
                .with_name(import.alias.clone().unwrap_or_else(|| import.path.clone()))
                .with_text(import.path.clone());
            node.range = Some(import.range.clone());
            insert_json_attr(&mut node.attrs, "path", &import.path);
            insert_json_attr(&mut node.attrs, "alias", &import.alias);
            insert_json_attr(&mut node.attrs, "visibility", &import.visibility);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(DocumentEdge::new(&root_id, DocumentRelation::Imports, &id));
        }

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

        let mut graph = code_graph_from_context(context, DocumentKind::TypeScript, "typescript");
        graph.dialect = Some(format!("{:?}", self.dialect).to_lowercase());
        let root_id = ensure_code_root(&mut graph, "typescript");
        let symbol_ids = self
            .symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.qualified_name.clone(),
                    code_node_id(&graph.id, "symbol", &symbol.id),
                )
            })
            .collect::<BTreeMap<_, _>>();

        for symbol in &self.symbols {
            let id = code_node_id(&graph.id, "symbol", &symbol.id);
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
            .with_qualified_name(symbol.qualified_name.clone());
            node.range = Some(symbol.range.clone());
            node.parent = symbol_parent_node(&graph.id, &symbol.qualified_name, &symbol_ids);
            insert_json_attr(&mut node.attrs, "visibility", &symbol.visibility);
            insert_json_attr(&mut node.attrs, "modifiers", &symbol.modifiers);
            insert_json_attr(&mut node.attrs, "decorators", &symbol.decorators);
            insert_json_attr(&mut node.attrs, "doc", &symbol.doc);
            graph.add_node(node);
            graph.add_contains(
                node_parent_or_root(
                    &root_id,
                    symbol_parent_node(&graph.id, &symbol.qualified_name, &symbol_ids),
                ),
                &id,
            );
        }

        for import in &self.imports {
            let id = code_node_id(&graph.id, "import", &import.id);
            let mut node =
                DocumentNode::new(&id, DocumentNodeKind::Import).with_name(import.module.clone());
            node.range = Some(import.range.clone());
            insert_json_attr(&mut node.attrs, "names", &import.names);
            insert_json_attr(&mut node.attrs, "default", &import.default);
            insert_json_attr(&mut node.attrs, "namespace", &import.namespace);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(DocumentEdge::new(&root_id, DocumentRelation::Imports, &id));
        }
        for export in &self.exports {
            let id = code_node_id(&graph.id, "export", &export.id);
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Export);
            node.range = Some(export.range.clone());
            insert_json_attr(&mut node.attrs, "names", &export.names);
            insert_json_attr(&mut node.attrs, "source", &export.source);
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            graph.add_edge(DocumentEdge::new(&root_id, DocumentRelation::Exports, &id));
        }

        for assignment in &self.assignments {
            add_code_fact_node(
                &mut graph,
                &root_id,
                "assignment",
                &assignment.id,
                DocumentNodeKind::Assignment,
                assignment.parent.as_deref(),
                &symbol_ids,
                Some(assignment.range.clone()),
                Some(assignment.lhs.clone()),
                |attrs| {
                    insert_json_attr(attrs, "rhs", &assignment.rhs);
                    insert_json_attr(attrs, "operator", &assignment.operator);
                },
            );
        }
        for ret in &self.returns {
            add_code_fact_node(
                &mut graph,
                &root_id,
                "return",
                &ret.id,
                DocumentNodeKind::Return,
                ret.parent.as_deref(),
                &symbol_ids,
                Some(ret.range.clone()),
                ret.expression.clone(),
                |_| {},
            );
        }
        for branch in &self.branches {
            add_code_fact_node(
                &mut graph,
                &root_id,
                "branch",
                &branch.id,
                DocumentNodeKind::Branch,
                branch.parent.as_deref(),
                &symbol_ids,
                Some(branch.range.clone()),
                branch.condition.clone(),
                |attrs| insert_json_attr(attrs, "kind", &branch.kind),
            );
        }
        for call in &self.calls {
            let parent = parent_lookup(call.parent.as_deref(), &symbol_ids)
                .unwrap_or_else(|| root_id.clone());
            let id = code_node_id(&graph.id, "call", &call.id);
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Call)
                .with_name(call.target.clone())
                .with_text(call.target.clone());
            node.range = Some(call.range.clone());
            insert_json_attr(&mut node.attrs, "args", &call.args);
            graph.add_node(node);
            graph.add_contains(&parent, &id);
            graph.add_edge(DocumentEdge::new(
                &parent,
                DocumentRelation::Calls,
                call.target.clone(),
            ));
        }

        Ok(graph)
    }
}

#[cfg(any(feature = "python", feature = "rust", feature = "typescript"))]
fn code_graph_from_context(
    context: DocumentGraphContext,
    kind: DocumentKind,
    default_language: &str,
) -> DocumentGraph {
    let mut graph = DocumentGraph::new(context.graph_id, kind);
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

#[cfg(any(feature = "python", feature = "rust", feature = "typescript"))]
fn ensure_code_root(graph: &mut DocumentGraph, language: &str) -> String {
    let root_id = format!("{}:module", graph.id);
    graph.add_node(
        DocumentNode::new(&root_id, DocumentNodeKind::Module)
            .with_name(language)
            .with_qualified_name(language),
    );
    root_id
}

#[cfg(any(feature = "python", feature = "rust", feature = "typescript"))]
fn code_node_id(graph_id: &str, category: &str, id: &str) -> String {
    format!("{}:{}:{}", graph_id, category, id)
}

#[cfg(any(feature = "python", feature = "rust", feature = "typescript"))]
fn insert_json_attr<T: Serialize>(attrs: &mut AttrMap, key: &str, value: &T) {
    if let Ok(value) = serde_json::to_value(value) {
        if !value.is_null() {
            attrs.insert(key.to_string(), value);
        }
    }
}

#[cfg(any(feature = "python", feature = "typescript"))]
fn symbol_parent_node(
    _graph_id: &str,
    qualified_name: &str,
    symbol_ids: &BTreeMap<String, String>,
) -> Option<String> {
    let (parent, _) = qualified_name.rsplit_once('.')?;
    symbol_ids.get(parent).cloned()
}

#[cfg(any(feature = "python", feature = "rust", feature = "typescript"))]
fn parent_lookup(parent: Option<&str>, symbol_ids: &BTreeMap<String, String>) -> Option<String> {
    let parent = parent?;
    symbol_ids.get(parent).cloned().or_else(|| {
        symbol_ids.iter().find_map(|(qualified, id)| {
            qualified
                .ends_with(&format!(".{parent}"))
                .then(|| id.clone())
        })
    })
}

#[cfg(any(feature = "python", feature = "rust", feature = "typescript"))]
fn node_parent_or_root(root_id: &str, parent: Option<String>) -> String {
    parent.unwrap_or_else(|| root_id.to_string())
}

#[cfg(any(feature = "python", feature = "typescript"))]
fn add_code_fact_node<F>(
    graph: &mut DocumentGraph,
    root_id: &str,
    category: &str,
    id: &str,
    kind: DocumentNodeKind,
    parent: Option<&str>,
    symbol_ids: &BTreeMap<String, String>,
    range: Option<SourceRange>,
    text: Option<String>,
    add_attrs: F,
) where
    F: FnOnce(&mut AttrMap),
{
    let parent = parent_lookup(parent, symbol_ids).unwrap_or_else(|| root_id.to_string());
    let node_id = code_node_id(&graph.id, category, id);
    let mut node = DocumentNode::new(&node_id, kind);
    node.range = range;
    node.text = text;
    add_attrs(&mut node.attrs);
    graph.add_node(node);
    graph.add_contains(parent, node_id);
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

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_projection_preserves_structure_links_and_tables() {
        use crate::core::SourceInfo;
        use crate::markdown::parse_markdown;

        let src = "---\ntitle: Demo\n---\n# Intro\n\nSee [site](https://example.com).\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let parsed = parse_markdown(src, SourceInfo::stdin("demo.md"));
        let graph = parsed
            .payload
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
