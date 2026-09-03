//! Normalized DocumentGraph projection for OpenDocument text packages.

use super::{OdfContent, OdfNode, OdfNodeKind, OdfWordDocument};
use crate::core::SchemaVersion;
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, RawNodeContent,
    ToDocumentGraph, TransformError,
};
use serde_json::Value;
use std::collections::HashMap;

impl ToDocumentGraph for OdfWordDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::ODF_WORD_V1, "grist.odf_word")
            .map_err(transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::OdfWord)
            .with_projection(
                "odf_word",
                SchemaVersion::ODF_WORD_V1,
                "grist.odf-word.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = context.language;
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| format!("{}-opendocument", self.package_kind.format_id())),
        );
        graph.attrs = context.attrs;

        let root_id = identities
            .node_id(&ProjectionAddress {
                structural_path: vec!["document".into()],
                native_id: Some("odf-root".into()),
                locator: Some(self.body.locator.clone()),
            })
            .map_err(transform_error)?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name(self.package_kind.format_id())
            .with_text(&self.views.visible)
            .with_locator(self.body.locator.clone())
            .with_ordinal(0);
        root.extensions.insert(
            "grist.odf_word".into(),
            serde_json::json!({
                "package_kind": self.package_kind,
                "package_media_type": self.package_media_type,
                "version": self.version,
                "manifest": self.manifest,
                "metadata": self.metadata,
                "settings": self.settings,
                "styles": self.styles,
                "list_styles": self.list_styles,
                "master_pages": self.master_pages,
                "links": self.links,
                "notes": self.notes,
                "annotations": self.annotations,
                "drawings": self.drawings,
                "equations": self.equations,
                "embedded_objects": self.embedded_objects,
                "embedded_artifacts": self.embedded_artifacts,
                "views": self.views,
            }),
        );
        graph.add_node(root);

        for (index, part) in self.parts.iter().enumerate() {
            let id = node_id(
                &identities,
                vec!["package".into(), "parts".into(), index.to_string()],
                Some(format!("part:{}", part.path)),
                part.locator.clone(),
            )?;
            let attachment = self
                .embedded_artifacts
                .iter()
                .any(|artifact| artifact.declared_filename.as_deref() == Some(part.path.as_str()));
            let kind = if attachment {
                DocumentNodeKind::Attachment
            } else if matches!(
                part.path.as_str(),
                "META-INF/manifest.xml" | "meta.xml" | "settings.xml" | "styles.xml"
            ) {
                DocumentNodeKind::Metadata
            } else {
                DocumentNodeKind::ArchiveMember
            };
            let mut node = DocumentNode::new(&id, kind)
                .with_name(&part.path)
                .with_locator(part.locator.clone())
                .with_ordinal(index)
                .with_attr("compressed_size", part.compressed_size)
                .with_attr("uncompressed_size", part.uncompressed_size);
            node.extensions.insert(
                "grist.odf_word".into(),
                serde_json::to_value(part).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
        }

        for (index, property) in self.metadata.iter().enumerate() {
            let id = node_id(
                &identities,
                vec!["metadata".into(), index.to_string()],
                None,
                property.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Metadata)
                .with_name(&property.qualified_name)
                .with_text(&property.value)
                .with_locator(property.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.odf_word".into(),
                serde_json::to_value(property).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
        }

        let mut source_ids = HashMap::new();
        project_node(
            &self.body,
            &root_id,
            vec!["body".into()],
            &identities,
            &mut graph,
            &mut source_ids,
        )?;
        project_revisions(self, &root_id, &identities, &mut graph, &source_ids)?;
        project_relations(self, &root_id, &mut graph, &source_ids);
        graph
            .finalize_projection(&identities)
            .map_err(transform_error)?;
        Ok(graph)
    }
}

fn project_node(
    source: &OdfNode,
    parent: &str,
    path: Vec<String>,
    identities: &GraphIdGenerator,
    graph: &mut DocumentGraph,
    source_ids: &mut HashMap<String, String>,
) -> Result<(), TransformError> {
    let id = node_id(
        identities,
        path.clone(),
        Some(source.id.clone()),
        source.locator.clone(),
    )?;
    let mut node = DocumentNode::new(&id, graph_kind(&source.kind))
        .with_locator(source.locator.clone())
        .with_ordinal(graph.nodes.len())
        .with_attr("qualified_name", source.qualified_name.clone());
    let text = super::parse::node_text(source);
    if !text.is_empty() {
        node.text = Some(text);
    }
    if let Some(style) = &source.style_name {
        node.attrs
            .insert("style_name".into(), Value::String(style.clone()));
    }
    if let Some(change) = &source.change_id {
        node.attrs
            .insert("change_id".into(), Value::String(change.clone()));
    }
    if source.kind == OdfNodeKind::Link
        && let Some(destination) = source
            .attributes
            .iter()
            .find(|(key, _)| key.rsplit(':').next() == Some("href"))
            .map(|(_, value)| value.clone())
    {
        node.attrs
            .insert("destination".into(), Value::String(destination));
    }
    node.extensions.insert(
        "grist.odf_word".into(),
        serde_json::json!({
            "attributes": source.attributes,
            "raw_xml": source.raw_xml,
        }),
    );
    if source.kind == OdfNodeKind::Unknown {
        node.raw = Some(
            RawNodeContent::new(
                "grist.odf_word",
                &source.qualified_name,
                serde_json::json!({
                    "xml": source.raw_xml,
                    "attributes": source.attributes,
                }),
            )
            .map_err(transform_error)?,
        );
    }
    graph.add_node(node);
    graph.add_contains(parent, &id);
    source_ids.insert(source.id.clone(), id.clone());
    let mut child_index = 0usize;
    for content in &source.content {
        if let OdfContent::Element { node } = content {
            let mut child_path = path.clone();
            child_path.push(child_index.to_string());
            project_node(node, &id, child_path, identities, graph, source_ids)?;
            child_index += 1;
        }
    }
    Ok(())
}

fn project_revisions(
    document: &OdfWordDocument,
    root: &str,
    identities: &GraphIdGenerator,
    graph: &mut DocumentGraph,
    source_ids: &HashMap<String, String>,
) -> Result<(), TransformError> {
    for (index, revision) in document.revisions.iter().enumerate() {
        let id = node_id(
            identities,
            vec!["revisions".into(), index.to_string()],
            Some(format!("revision:{}", revision.id)),
            revision.locator.clone(),
        )?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::Revision)
            .with_text(&revision.deleted_text)
            .with_locator(revision.locator.clone())
            .with_ordinal(index)
            .with_attr("revision_id", revision.id.clone())
            .with_attr(
                "revision_kind",
                serde_json::to_value(revision.kind).map_err(transform_error)?,
            );
        node.extensions.insert(
            "grist.odf_word".into(),
            serde_json::to_value(revision).map_err(transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(root, &id);
        for source in affected_nodes(&document.body, &revision.id) {
            if let Some(target) = source_ids.get(&source.id) {
                graph.add_edge(DocumentEdge::explicit(
                    id.clone(),
                    DocumentRelation::RevisionOf,
                    target.clone(),
                    revision.locator.clone(),
                ));
            }
        }
    }
    Ok(())
}

fn project_relations(
    document: &OdfWordDocument,
    root: &str,
    graph: &mut DocumentGraph,
    source_ids: &HashMap<String, String>,
) {
    for link in &document.links {
        let source = source_ids
            .get(&link.source_node_id)
            .map(String::as_str)
            .unwrap_or(root);
        let target = link
            .resolved_member
            .clone()
            .unwrap_or_else(|| link.href.clone());
        graph.add_edge(DocumentEdge::explicit(
            source,
            DocumentRelation::LinksTo,
            target,
            link.locator.clone(),
        ));
    }
    for annotation in &document.annotations {
        let source = source_ids
            .get(&annotation.source_node_id)
            .map(String::as_str)
            .unwrap_or(root);
        graph.add_edge(DocumentEdge::explicit(
            source,
            DocumentRelation::Annotates,
            root,
            annotation.locator.clone(),
        ));
    }
    for object in &document.embedded_objects {
        let source = source_ids
            .get(&object.source_node_id)
            .map(String::as_str)
            .unwrap_or(root);
        for artifact in &object.artifact_ids {
            graph.add_edge(DocumentEdge::explicit(
                artifact,
                DocumentRelation::EmbeddedIn,
                source,
                object.locator.clone(),
            ));
        }
    }
}

fn affected_nodes<'a>(node: &'a OdfNode, revision_id: &str) -> Vec<&'a OdfNode> {
    let mut output = Vec::new();
    if node.change_id.as_deref() == Some(revision_id)
        && !matches!(
            node.kind,
            OdfNodeKind::Change | OdfNodeKind::ChangeStart | OdfNodeKind::ChangeEnd
        )
    {
        output.push(node);
    }
    for content in &node.content {
        if let OdfContent::Element { node } = content {
            output.extend(affected_nodes(node, revision_id));
        }
    }
    output
}

fn graph_kind(kind: &OdfNodeKind) -> DocumentNodeKind {
    match kind {
        OdfNodeKind::Document => DocumentNodeKind::Section,
        OdfNodeKind::Section => DocumentNodeKind::Section,
        OdfNodeKind::Heading => DocumentNodeKind::Heading,
        OdfNodeKind::Paragraph => DocumentNodeKind::Paragraph,
        OdfNodeKind::Span => DocumentNodeKind::Span,
        OdfNodeKind::List => DocumentNodeKind::List,
        OdfNodeKind::ListItem => DocumentNodeKind::ListItem,
        OdfNodeKind::Table => DocumentNodeKind::Table,
        OdfNodeKind::TableRow => DocumentNodeKind::TableRow,
        OdfNodeKind::TableCell | OdfNodeKind::CoveredTableCell => DocumentNodeKind::TableCell,
        OdfNodeKind::Link => DocumentNodeKind::Link,
        OdfNodeKind::Bookmark => DocumentNodeKind::Bookmark,
        OdfNodeKind::Reference => DocumentNodeKind::Reference,
        OdfNodeKind::Field => DocumentNodeKind::FormField,
        OdfNodeKind::Footnote => DocumentNodeKind::Footnote,
        OdfNodeKind::Endnote => DocumentNodeKind::Endnote,
        OdfNodeKind::NoteCitation => DocumentNodeKind::Reference,
        OdfNodeKind::NoteBody => DocumentNodeKind::Paragraph,
        OdfNodeKind::Header => DocumentNodeKind::Header,
        OdfNodeKind::Footer => DocumentNodeKind::Footer,
        OdfNodeKind::Annotation | OdfNodeKind::AnnotationEnd => DocumentNodeKind::Annotation,
        OdfNodeKind::RevisionContainer
        | OdfNodeKind::RevisionRegion
        | OdfNodeKind::ChangeStart
        | OdfNodeKind::ChangeEnd
        | OdfNodeKind::Change => DocumentNodeKind::Revision,
        OdfNodeKind::Drawing => DocumentNodeKind::Figure,
        OdfNodeKind::Image => DocumentNodeKind::Image,
        OdfNodeKind::TextBox => DocumentNodeKind::Span,
        OdfNodeKind::EmbeddedObject => DocumentNodeKind::Attachment,
        OdfNodeKind::Equation => DocumentNodeKind::Equation,
        OdfNodeKind::Space | OdfNodeKind::Tab | OdfNodeKind::LineBreak | OdfNodeKind::PageBreak => {
            DocumentNodeKind::TextRun
        }
        OdfNodeKind::Unknown => DocumentNodeKind::Unknown,
    }
}

fn node_id(
    identities: &GraphIdGenerator,
    structural_path: Vec<String>,
    native_id: Option<String>,
    locator: crate::core::SourceLocator,
) -> Result<String, TransformError> {
    identities
        .node_id(&ProjectionAddress {
            structural_path,
            native_id,
            locator: Some(locator),
        })
        .map_err(transform_error)
}

fn transform_error(error: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: error.to_string(),
    }
}
