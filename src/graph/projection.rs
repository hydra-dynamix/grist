use super::{GraphDocument, GraphParseResult, GraphSourceMap, validate_graph};
use crate::core::Diagnostic;
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, RawNodeContent, ToDocumentGraph, TransformError,
};
use serde_json::{Value, json};

const NAMESPACE: &str = "grist.graph";

/// Explicitly project the authoritative input graph into the normalized IR.
/// Original IDs, vocabulary, attributes, direction flags, and locators remain
/// available in the projection; the input payload remains authoritative.
pub fn project_graph_to_document_graph(
    document: &GraphDocument,
    source_map: &GraphSourceMap,
    context: DocumentGraphContext,
) -> Result<DocumentGraph, TransformError> {
    let validation =
        validate_graph(document, source_map, &Default::default()).map_err(|error| {
            TransformError::Other {
                message: error.to_string(),
            }
        })?;
    if !validation.valid {
        return Err(TransformError::InvalidGraphShape {
            message: validation
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        });
    }

    let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Other("graph".into()))
        .with_projection(
            "graph",
            GraphDocument::SCHEMA_VERSION,
            "grist.graph.to-document-graph.v1",
        );
    graph.source = context.source;
    graph.language = Some(context.language.unwrap_or_else(|| "graph".into()));
    graph.dialect = context.dialect;
    graph.attrs = context.attrs;
    graph.attrs.extend(document.attrs.clone());
    graph.extensions.insert(
        NAMESPACE.into(),
        json!({
            "id": document.id,
            "directed": document.directed,
            "attrs": document.attrs,
        }),
    );

    for (position, source_node) in document.nodes.iter().enumerate() {
        let kind = source_node
            .labels
            .iter()
            .find_map(|label| known_node_kind(label))
            .unwrap_or_else(|| {
                DocumentNodeKind::Other(
                    source_node
                        .labels
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "graph_node".into()),
                )
            });
        let retains_raw = matches!(kind, DocumentNodeKind::Unknown | DocumentNodeKind::Other(_));
        let mut node = DocumentNode::new(&source_node.id, kind);
        node.attrs = source_node.attrs.clone();
        let original_node = serde_json::to_value(source_node).map_err(projection_error)?;
        node.extensions
            .insert(NAMESPACE.into(), original_node.clone());
        if let Some(locator) = source_map.nodes.get(position).cloned() {
            node = node.with_locator(locator);
        }
        if retains_raw {
            node.raw = Some(
                RawNodeContent::new(NAMESPACE, "graph_node", original_node)
                    .map_err(projection_error)?,
            );
        }
        graph.nodes.push(node);
    }

    for (position, source_edge) in document.edges.iter().enumerate() {
        let relation = source_edge
            .label
            .as_deref()
            .and_then(known_relation)
            .unwrap_or_else(|| {
                DocumentRelation::Other(source_edge.label.clone().unwrap_or_else(|| "edge".into()))
            });
        let mut edge = DocumentEdge::new(&source_edge.source, relation, &source_edge.target);
        edge.id.clone_from(&source_edge.id);
        edge.attrs = source_edge.attrs.clone();
        edge.attrs
            .insert("directed".into(), Value::Bool(source_edge.directed));
        edge.extensions.insert(
            NAMESPACE.into(),
            serde_json::to_value(source_edge).map_err(projection_error)?,
        );
        if let Some(locator) = source_map.edges.get(position).cloned() {
            edge = edge.with_locator(locator);
        }
        graph.edges.push(edge);
        if !source_edge.directed {
            graph.diagnostics.push(Diagnostic::warning(
                "grist.graph.projection",
                "grist.graph.projection.undirected_authoritative",
                format!(
                    "edge `{}` is represented once in the normalized directed IR; the authoritative graph payload retains its undirected semantics",
                    source_edge.id
                ),
            ));
        }
    }
    graph.canonicalize().map_err(projection_error)?;
    graph.validate_contract().map_err(projection_error)?;
    Ok(graph)
}

impl ToDocumentGraph for GraphDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        project_graph_to_document_graph(self, &GraphSourceMap::default(), context)
    }
}

impl ToDocumentGraph for GraphParseResult {
    fn to_document_graph(
        &self,
        mut context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let document =
            self.envelope
                .payload
                .as_ref()
                .ok_or_else(|| TransformError::InvalidGraphShape {
                    message: "graph parse result has no payload".into(),
                })?;
        if context.source.is_none() {
            context.source = Some(self.envelope.source.clone());
        }
        let mut graph = project_graph_to_document_graph(document, &self.source_map, context)?;
        graph.diagnostics.extend(self.envelope.diagnostics.clone());
        graph.canonicalize().map_err(projection_error)?;
        Ok(graph)
    }
}

fn known_node_kind(label: &str) -> Option<DocumentNodeKind> {
    Some(match normalize(label).as_str() {
        "document" => DocumentNodeKind::Document,
        "container" => DocumentNodeKind::Container,
        "section" => DocumentNodeKind::Section,
        "heading" => DocumentNodeKind::Heading,
        "paragraph" => DocumentNodeKind::Paragraph,
        "text" => DocumentNodeKind::Text,
        "link" => DocumentNodeKind::Link,
        "list" => DocumentNodeKind::List,
        "list_item" => DocumentNodeKind::ListItem,
        "table" => DocumentNodeKind::Table,
        "row" => DocumentNodeKind::Row,
        "cell" => DocumentNodeKind::Cell,
        "record" => DocumentNodeKind::Record,
        "module" => DocumentNodeKind::Module,
        "namespace" => DocumentNodeKind::Namespace,
        "package" => DocumentNodeKind::Package,
        "symbol" => DocumentNodeKind::Symbol,
        "class" => DocumentNodeKind::Class,
        "function" => DocumentNodeKind::Function,
        "method" => DocumentNodeKind::Method,
        "interface" => DocumentNodeKind::Interface,
        "variable" => DocumentNodeKind::Variable,
        "field" => DocumentNodeKind::Field,
        "claim" => DocumentNodeKind::Claim,
        "evidence" => DocumentNodeKind::Evidence,
        "requirement" => DocumentNodeKind::Requirement,
        "condition" => DocumentNodeKind::Condition,
        "diagnostic" => DocumentNodeKind::Diagnostic,
        _ => return None,
    })
}

fn known_relation(label: &str) -> Option<DocumentRelation> {
    Some(match normalize(label).as_str() {
        "contains" => DocumentRelation::Contains,
        "precedes" => DocumentRelation::Precedes,
        "parent_of" => DocumentRelation::ParentOf,
        "references" => DocumentRelation::References,
        "resolves_to" => DocumentRelation::ResolvesTo,
        "defines" => DocumentRelation::Defines,
        "links_to" => DocumentRelation::LinksTo,
        "cites" => DocumentRelation::Cites,
        "derived_from" => DocumentRelation::DerivedFrom,
        "source_of" => DocumentRelation::SourceOf,
        "evidence_for" => DocumentRelation::EvidenceFor,
        "imports" => DocumentRelation::Imports,
        "exports" => DocumentRelation::Exports,
        "calls" => DocumentRelation::Calls,
        "inherits" => DocumentRelation::Inherits,
        "implements" => DocumentRelation::Implements,
        "requires" | "depends_on" => DocumentRelation::Requires,
        "satisfies" => DocumentRelation::Satisfies,
        "violates" => DocumentRelation::Violates,
        "conditional_on" => DocumentRelation::ConditionalOn,
        _ => return None,
    })
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn projection_error(error: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: error.to_string(),
    }
}
