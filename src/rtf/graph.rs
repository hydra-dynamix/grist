//! Normalized DocumentGraph projection for Rich Text Format documents.

use super::{RtfDocument, RtfRevisionKind};
use crate::core::SchemaVersion;
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, RawNodeContent,
    ToDocumentGraph, TransformError,
};
use std::collections::HashMap;

impl ToDocumentGraph for RtfDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::RTF_V1, "grist.rtf")
            .map_err(transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Rtf).with_projection(
            "rtf",
            SchemaVersion::RTF_V1,
            "grist.rtf.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.language = context.language;
        graph.dialect = Some(context.dialect.unwrap_or_else(|| "rtf-1.x".into()));
        graph.attrs = context.attrs;
        let root_id = node_id(
            &identities,
            vec!["document".into()],
            Some("rtf-root".into()),
            self.root.locator.clone(),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name("RTF")
            .with_text(&self.views.visible)
            .with_locator(self.root.locator.clone())
            .with_ordinal(0);
        root.extensions.insert(
            "grist.rtf".into(),
            serde_json::json!({
                "rtf_version": self.rtf_version,
                "charset": self.charset,
                "ansi_code_page": self.ansi_code_page,
                "default_font": self.default_font,
                "generator": self.generator,
                "metadata": self.metadata,
                "destinations": self.destinations,
                "fonts": self.fonts,
                "colors": self.colors,
                "styles": self.styles,
                "lists": self.lists,
                "list_overrides": self.list_overrides,
                "views": self.views,
                "embedded_artifacts": self.embedded_artifacts,
            }),
        );
        graph.add_node(root);

        let mut paragraph_ids = HashMap::new();
        let mut run_ids = HashMap::new();
        for (paragraph_index, paragraph) in self.paragraphs.iter().enumerate() {
            let id = node_id(
                &identities,
                vec!["paragraphs".into(), paragraph_index.to_string()],
                Some(paragraph.id.clone()),
                paragraph.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Paragraph)
                .with_text(paragraph_text(paragraph))
                .with_locator(paragraph.locator.clone())
                .with_ordinal(paragraph_index);
            node.extensions.insert(
                "grist.rtf".into(),
                serde_json::json!({
                    "paragraph_style": paragraph.paragraph_style,
                    "list_override": paragraph.list_override,
                    "list_level": paragraph.list_level,
                    "table_position": paragraph.table_position,
                }),
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            paragraph_ids.insert(paragraph.id.clone(), id.clone());
            for (run_index, run) in paragraph.runs.iter().enumerate() {
                let run_id = node_id(
                    &identities,
                    vec![
                        "paragraphs".into(),
                        paragraph_index.to_string(),
                        "runs".into(),
                        run_index.to_string(),
                    ],
                    Some(run.id.clone()),
                    run.locator.clone(),
                )?;
                let mut run_node = DocumentNode::new(&run_id, DocumentNodeKind::TextRun)
                    .with_text(&run.text)
                    .with_locator(run.locator.clone())
                    .with_ordinal(run_index);
                run_node.extensions.insert(
                    "grist.rtf".into(),
                    serde_json::to_value(run).map_err(transform_error)?,
                );
                graph.add_node(run_node);
                graph.add_contains(&id, &run_id);
                run_ids.insert(run.id.clone(), run_id);
            }
        }

        for (index, item) in self.list_items.iter().enumerate() {
            let id = node_id(
                &identities,
                vec!["list_items".into(), index.to_string()],
                Some(format!(
                    "list-item:{}:{}",
                    item.override_id, item.paragraph_id
                )),
                item.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::ListItem)
                .with_text(&item.text)
                .with_locator(item.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.rtf".into(),
                serde_json::to_value(item).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            if let Some(target) = paragraph_ids.get(&item.paragraph_id) {
                graph.add_edge(DocumentEdge::explicit(
                    id,
                    DocumentRelation::References,
                    target,
                    item.locator.clone(),
                ));
            }
        }

        for (table_index, table) in self.tables.iter().enumerate() {
            let table_id = node_id(
                &identities,
                vec!["tables".into(), table_index.to_string()],
                Some(table.id.clone()),
                table.locator.clone(),
            )?;
            let mut table_node = DocumentNode::new(&table_id, DocumentNodeKind::Table)
                .with_locator(table.locator.clone())
                .with_ordinal(table_index);
            table_node.extensions.insert(
                "grist.rtf".into(),
                serde_json::json!({"nesting_level": table.nesting_level}),
            );
            graph.add_node(table_node);
            graph.add_contains(&root_id, &table_id);
            for row in &table.rows {
                let row_id = node_id(
                    &identities,
                    vec![
                        "tables".into(),
                        table_index.to_string(),
                        "rows".into(),
                        row.index.to_string(),
                    ],
                    None,
                    row.locator.clone(),
                )?;
                graph.add_node(
                    DocumentNode::new(&row_id, DocumentNodeKind::TableRow)
                        .with_locator(row.locator.clone())
                        .with_ordinal(row.index),
                );
                graph.add_contains(&table_id, &row_id);
                for cell in &row.cells {
                    let cell_id = node_id(
                        &identities,
                        vec![
                            "tables".into(),
                            table_index.to_string(),
                            "rows".into(),
                            row.index.to_string(),
                            "cells".into(),
                            cell.index.to_string(),
                        ],
                        None,
                        cell.locator.clone(),
                    )?;
                    let mut node = DocumentNode::new(&cell_id, DocumentNodeKind::TableCell)
                        .with_text(&cell.text)
                        .with_locator(cell.locator.clone())
                        .with_ordinal(cell.index);
                    node.extensions.insert(
                        "grist.rtf".into(),
                        serde_json::to_value(cell).map_err(transform_error)?,
                    );
                    graph.add_node(node);
                    graph.add_contains(&row_id, &cell_id);
                }
            }
        }

        project_semantics(self, &root_id, &identities, &mut graph, &run_ids)?;
        graph
            .finalize_projection(&identities)
            .map_err(transform_error)?;
        Ok(graph)
    }
}

fn project_semantics(
    document: &RtfDocument,
    root: &str,
    identities: &GraphIdGenerator,
    graph: &mut DocumentGraph,
    run_ids: &HashMap<String, String>,
) -> Result<(), TransformError> {
    for (index, field) in document.fields.iter().enumerate() {
        let id = semantic_node(
            identities,
            graph,
            root,
            "fields",
            index,
            &field.group_id,
            DocumentNodeKind::FormField,
            &field.result,
            field.locator.clone(),
            field,
        )?;
        if let Some(target) = &field.target {
            graph.add_edge(DocumentEdge::explicit(
                id,
                DocumentRelation::LinksTo,
                target,
                field.locator.clone(),
            ));
        }
    }
    for (index, image) in document.images.iter().enumerate() {
        let id = semantic_node(
            identities,
            graph,
            root,
            "images",
            index,
            &image.group_id,
            DocumentNodeKind::Image,
            "",
            image.locator.clone(),
            image,
        )?;
        if let Some(artifact) = &image.artifact_id {
            graph.add_edge(DocumentEdge::explicit(
                artifact,
                DocumentRelation::EmbeddedIn,
                id,
                image.locator.clone(),
            ));
        }
    }
    for (index, object) in document.objects.iter().enumerate() {
        let id = semantic_node(
            identities,
            graph,
            root,
            "objects",
            index,
            &object.group_id,
            DocumentNodeKind::Attachment,
            &object.result_text,
            object.locator.clone(),
            object,
        )?;
        if let Some(artifact) = &object.artifact_id {
            graph.add_edge(DocumentEdge::explicit(
                artifact,
                DocumentRelation::EmbeddedIn,
                id,
                object.locator.clone(),
            ));
        }
    }
    for (index, comment) in document.comments.iter().enumerate() {
        let id = semantic_node(
            identities,
            graph,
            root,
            "comments",
            index,
            &comment.group_id,
            DocumentNodeKind::Comment,
            &comment.text,
            comment.locator.clone(),
            comment,
        )?;
        graph.add_edge(DocumentEdge::explicit(
            id,
            DocumentRelation::Annotates,
            root,
            comment.locator.clone(),
        ));
    }
    for (index, revision) in document.revisions.iter().enumerate() {
        let native = format!("revision:{}", revision.run_id);
        let id = semantic_node(
            identities,
            graph,
            root,
            "revisions",
            index,
            &native,
            DocumentNodeKind::Revision,
            &revision.text,
            revision.locator.clone(),
            revision,
        )?;
        if let Some(target) = run_ids.get(&revision.run_id) {
            graph.add_edge(DocumentEdge::explicit(
                id,
                DocumentRelation::RevisionOf,
                target,
                revision.locator.clone(),
            ));
        }
    }
    for (index, control) in document.unknown_controls.iter().enumerate() {
        let id = node_id(
            identities,
            vec!["unknown_controls".into(), index.to_string()],
            None,
            control.locator.clone(),
        )?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::Unknown)
            .with_name(&control.name)
            .with_locator(control.locator.clone())
            .with_ordinal(index);
        node.raw = Some(
            RawNodeContent::new(
                "grist.rtf",
                "control",
                serde_json::to_value(control).map_err(transform_error)?,
            )
            .map_err(transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(root, &id);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn semantic_node<T: serde::Serialize>(
    identities: &GraphIdGenerator,
    graph: &mut DocumentGraph,
    root: &str,
    collection: &str,
    index: usize,
    native: &str,
    kind: DocumentNodeKind,
    text: &str,
    locator: crate::core::SourceLocator,
    payload: &T,
) -> Result<String, TransformError> {
    let id = node_id(
        identities,
        vec![collection.into(), index.to_string()],
        Some(native.into()),
        locator.clone(),
    )?;
    let mut node = DocumentNode::new(&id, kind)
        .with_locator(locator)
        .with_ordinal(index);
    if !text.is_empty() {
        node.text = Some(text.into());
    }
    node.extensions.insert(
        "grist.rtf".into(),
        serde_json::to_value(payload).map_err(transform_error)?,
    );
    graph.add_node(node);
    graph.add_contains(root, &id);
    Ok(id)
}

fn paragraph_text(paragraph: &super::RtfParagraph) -> String {
    paragraph
        .runs
        .iter()
        .filter(|run| !run.style.hidden && run.revision != Some(RtfRevisionKind::Deleted))
        .map(|run| run.text.as_str())
        .collect()
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
