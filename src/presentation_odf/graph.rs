//! Normalized DocumentGraph projection for OpenDocument presentations.

use super::*;
use crate::core::{LocatorConfidence, SchemaVersion};
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, RawNodeContent,
    ToDocumentGraph, TransformError,
};
use std::collections::HashMap;

impl ToDocumentGraph for OdfPresentationDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::PRESENTATION_ODF_V1, "grist.presentation_odf")
            .map_err(transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::PresentationOdf)
            .with_projection(
                "presentation_odf",
                SchemaVersion::PRESENTATION_ODF_V1,
                "grist.presentation-odf.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = context.language;
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| format!("{}-opendocument", self.package_kind.format_id())),
        );
        graph.attrs = context.attrs;
        let root_id = node_id(
            &ids,
            vec!["document".into()],
            Some("presentation-root".into()),
            self.slides
                .first()
                .map(|slide| slide.locator.clone())
                .or_else(|| self.parts.first().map(|part| part.locator.clone())),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name(self.package_kind.format_id())
            .with_ordinal(0);
        root.extensions.insert(
            "grist.presentation_odf".into(),
            serde_json::json!({
                "package_kind": self.package_kind,
                "package_media_type": self.package_media_type,
                "version": self.version,
                "manifest": self.manifest,
                "metadata": self.metadata,
                "settings": self.settings,
                "styles": self.styles,
                "page_layouts": self.page_layouts,
                "master_pages": self.master_pages,
                "embedded_artifacts": self.embedded_artifacts,
                "raw_elements": self.raw_elements,
            }),
        );
        graph.add_node(root);

        for (index, part) in self.parts.iter().enumerate() {
            let id = node_id(
                &ids,
                vec!["parts".into(), index.to_string()],
                Some(format!("part:{}", part.path)),
                Some(part.locator.clone()),
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
                .with_ordinal(index);
            node.extensions.insert(
                "grist.presentation_odf".into(),
                serde_json::to_value(part).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
        }

        for (index, master) in self.master_pages.iter().enumerate() {
            let id = node_id(
                &ids,
                vec!["masters".into(), index.to_string()],
                master.name.as_ref().map(|name| format!("master:{name}")),
                Some(master.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Section)
                .with_name(master.name.as_deref().unwrap_or("master-page"))
                .with_locator(master.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.presentation_odf".into(),
                serde_json::to_value(master).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
        }

        for property in self.metadata.iter().chain(self.settings.iter()) {
            let id = node_id(
                &ids,
                vec!["properties".into(), graph.nodes.len().to_string()],
                None,
                Some(property.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Metadata)
                .with_name(&property.qualified_name)
                .with_text(&property.value)
                .with_locator(property.locator.clone())
                .with_ordinal(graph.nodes.len());
            node.extensions.insert(
                "grist.presentation_odf".into(),
                serde_json::to_value(property).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
        }

        let mut slide_ids = HashMap::new();
        for slide in &self.slides {
            let id = node_id(
                &ids,
                vec!["slides".into(), slide.order.to_string()],
                Some(format!("slide:{}", slide.slide_id)),
                Some(slide.locator.clone()),
            )?;
            let title = slide
                .shapes
                .iter()
                .find(|shape| shape.presentation_class.as_deref() == Some("title"))
                .and_then(|shape| shape.text.as_ref())
                .map(|body| body.text.as_str())
                .or(slide.name.as_deref())
                .unwrap_or("slide");
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Slide)
                .with_name(title)
                .with_locator(slide.locator.clone())
                .with_ordinal(slide.order)
                .with_attr("slide_id", slide.slide_id.clone())
                .with_attr("visible", slide.visible);
            node.extensions.insert(
                "grist.presentation_odf".into(),
                serde_json::to_value(slide).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
            slide_ids.insert(slide.order, id);
        }
        for pair in self.slides.windows(2) {
            graph.add_edge(DocumentEdge::explicit(
                slide_ids[&pair[0].order].clone(),
                DocumentRelation::Precedes,
                slide_ids[&pair[1].order].clone(),
                pair[0].locator.clone(),
            ));
        }
        project_slides(self, &ids, &mut graph, &slide_ids)?;
        graph.finalize_projection(&ids).map_err(transform_error)?;
        Ok(graph)
    }
}

fn project_slides(
    document: &OdfPresentationDocument,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
    slide_ids: &HashMap<usize, String>,
) -> Result<(), TransformError> {
    for slide in &document.slides {
        let slide_node = &slide_ids[&slide.order];
        let mut shapes = HashMap::new();
        for shape in &slide.shapes {
            let id = node_id(
                ids,
                vec![
                    "slides".into(),
                    slide.order.to_string(),
                    "shapes".into(),
                    shape.shape_id.clone(),
                ],
                Some(format!("shape:{}:{}", slide.slide_id, shape.shape_id)),
                Some(shape.locator.clone()),
            )?;
            let kind = match shape.kind {
                OdfPresentationShapeKind::Group => DocumentNodeKind::Container,
                OdfPresentationShapeKind::Connector | OdfPresentationShapeKind::Line => {
                    DocumentNodeKind::Figure
                }
                _ => DocumentNodeKind::Container,
            };
            let mut node = DocumentNode::new(&id, kind)
                .with_locator(shape.locator.clone())
                .with_ordinal(shape.z_order)
                .with_attr("shape_id", shape.shape_id.clone());
            node.name = shape.name.clone();
            node.extensions.insert(
                "grist.presentation_odf".into(),
                serde_json::to_value(shape).map_err(transform_error)?,
            );
            if shape.kind == OdfPresentationShapeKind::Unknown {
                node.raw = Some(
                    RawNodeContent::new(
                        "grist.presentation_odf",
                        &shape.qualified_name,
                        serde_json::json!({"xml": shape.raw_xml, "attributes": shape.attributes}),
                    )
                    .map_err(transform_error)?,
                );
            }
            graph.add_node(node);
            let parent = shape
                .parent_shape_id
                .as_ref()
                .and_then(|parent| shapes.get(parent))
                .unwrap_or(slide_node);
            graph.add_contains(parent, &id);
            shapes.insert(shape.shape_id.clone(), id.clone());
            project_text(slide, shape, &id, ids, graph)?;
        }
        for pair in slide.reading_order.entries.windows(2) {
            if let (Some(source), Some(target)) =
                (shapes.get(&pair[0].shape_id), shapes.get(&pair[1].shape_id))
            {
                let confidence = LocatorConfidence::new(pair[0].confidence.min(pair[1].confidence))
                    .map_err(transform_error)?;
                graph.add_edge(
                    DocumentEdge::inferred(
                        source.clone(),
                        DocumentRelation::Precedes,
                        target.clone(),
                        "grist.presentation_odf.reading-order.v1",
                        confidence,
                    )
                    .with_inference_evidence(pair[0].locator.clone())
                    .with_inference_evidence(pair[1].locator.clone()),
                );
            }
        }
        project_objects(slide, slide_node, &shapes, ids, graph)?;
    }
    Ok(())
}

fn project_text(
    slide: &OdfPresentationSlide,
    shape: &OdfPresentationShape,
    parent: &str,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let Some(body) = &shape.text else {
        return Ok(());
    };
    for paragraph in &body.paragraphs {
        let id = node_id(
            ids,
            vec![
                "slides".into(),
                slide.order.to_string(),
                "shapes".into(),
                shape.shape_id.clone(),
                "paragraphs".into(),
                paragraph.index.to_string(),
            ],
            None,
            Some(paragraph.locator.clone()),
        )?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::Paragraph)
            .with_text(&paragraph.text)
            .with_locator(paragraph.locator.clone())
            .with_ordinal(paragraph.index);
        node.extensions.insert(
            "grist.presentation_odf".into(),
            serde_json::to_value(paragraph).map_err(transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(parent, &id);
        for run in &paragraph.runs {
            let run_id = node_id(
                ids,
                vec![
                    "slides".into(),
                    slide.order.to_string(),
                    "shapes".into(),
                    shape.shape_id.clone(),
                    "paragraphs".into(),
                    paragraph.index.to_string(),
                    "runs".into(),
                    run.index.to_string(),
                ],
                None,
                Some(run.locator.clone()),
            )?;
            let mut run_node = DocumentNode::new(&run_id, DocumentNodeKind::TextRun)
                .with_text(&run.text)
                .with_locator(run.locator.clone())
                .with_ordinal(run.index);
            run_node.extensions.insert(
                "grist.presentation_odf".into(),
                serde_json::to_value(run).map_err(transform_error)?,
            );
            graph.add_node(run_node);
            graph.add_contains(&id, &run_id);
            if let Some(target) = &run.href {
                graph.add_edge(DocumentEdge::explicit(
                    run_id,
                    DocumentRelation::LinksTo,
                    target.clone(),
                    run.locator.clone(),
                ));
            }
        }
    }
    Ok(())
}

fn project_objects(
    slide: &OdfPresentationSlide,
    slide_node: &str,
    shapes: &HashMap<String, String>,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    for table in &slide.tables {
        project_table(slide, table, slide_node, shapes, ids, graph)?;
    }
    for (collection, kind, values) in [
        (
            "charts",
            DocumentNodeKind::Chart,
            slide
                .charts
                .iter()
                .map(|value| {
                    (
                        value.shape_id.as_str(),
                        value.title.as_deref(),
                        &value.locator,
                        serde_json::to_value(value).map_err(transform_error),
                    )
                })
                .collect::<Vec<_>>(),
        ),
        (
            "images",
            DocumentNodeKind::Image,
            slide
                .images
                .iter()
                .map(|value| {
                    (
                        value.shape_id.as_str(),
                        value
                            .alt_description
                            .as_deref()
                            .or(value.alt_title.as_deref()),
                        &value.locator,
                        serde_json::to_value(value).map_err(transform_error),
                    )
                })
                .collect::<Vec<_>>(),
        ),
    ] {
        for (index, (shape_id, text, locator, value)) in values.into_iter().enumerate() {
            add_node(
                slide,
                collection,
                index,
                kind.clone(),
                text,
                locator,
                shape_id,
                value?,
                slide_node,
                shapes,
                ids,
                graph,
            )?;
        }
    }
    for (index, comment) in slide.comments.iter().enumerate() {
        add_node(
            slide,
            "comments",
            index,
            DocumentNodeKind::Comment,
            Some(&comment.text),
            &comment.locator,
            "",
            serde_json::to_value(comment).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    for (index, note) in slide.notes.iter().enumerate() {
        add_node(
            slide,
            "notes",
            index,
            DocumentNodeKind::Annotation,
            Some(&note.text),
            &note.locator,
            "",
            serde_json::to_value(note).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    for (index, link) in slide.links.iter().enumerate() {
        let id = add_node(
            slide,
            "links",
            index,
            DocumentNodeKind::Link,
            Some(&link.text),
            &link.locator,
            link.source_shape_id.as_deref().unwrap_or(""),
            serde_json::to_value(link).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
        if let Some(node) = graph.nodes.iter_mut().find(|node| node.id == id) {
            node.attrs
                .insert("destination".into(), link.href.clone().into());
            node.attrs.insert("external".into(), link.external.into());
            node.attrs.insert("inert".into(), true.into());
        }
        graph.add_edge(DocumentEdge::explicit(
            id,
            DocumentRelation::LinksTo,
            link.href.clone(),
            link.locator.clone(),
        ));
    }
    if let Some(transition) = &slide.transition {
        add_node(
            slide,
            "transition",
            0,
            DocumentNodeKind::Metadata,
            transition
                .style
                .as_deref()
                .or(transition.type_name.as_deref()),
            &transition.locator,
            "",
            serde_json::to_value(transition).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    for (index, animation) in slide.animations.iter().enumerate() {
        add_node(
            slide,
            "animations",
            index,
            DocumentNodeKind::Metadata,
            Some(&animation.text),
            &animation.locator,
            animation.target_element.as_deref().unwrap_or(""),
            serde_json::to_value(animation).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    for (index, object) in slide.embedded_objects.iter().enumerate() {
        add_node(
            slide,
            "objects",
            index,
            DocumentNodeKind::Attachment,
            object.href.as_deref().or(Some(&object.kind)),
            &object.locator,
            &object.shape_id,
            serde_json::to_value(object).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_node(
    slide: &OdfPresentationSlide,
    collection: &str,
    index: usize,
    kind: DocumentNodeKind,
    text: Option<&str>,
    locator: &crate::core::SourceLocator,
    shape_id: &str,
    value: serde_json::Value,
    slide_node: &str,
    shapes: &HashMap<String, String>,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<String, TransformError> {
    let id = node_id(
        ids,
        vec![
            "slides".into(),
            slide.order.to_string(),
            collection.into(),
            index.to_string(),
        ],
        None,
        Some(locator.clone()),
    )?;
    let mut node = DocumentNode::new(&id, kind)
        .with_locator(locator.clone())
        .with_ordinal(index);
    node.text = text.filter(|value| !value.is_empty()).map(str::to_string);
    node.extensions
        .insert("grist.presentation_odf".into(), value);
    graph.add_node(node);
    let parent = shapes
        .get(shape_id)
        .map(String::as_str)
        .unwrap_or(slide_node);
    graph.add_contains(parent, &id);
    Ok(id)
}

fn project_table(
    slide: &OdfPresentationSlide,
    table: &OdfPresentationTable,
    slide_node: &str,
    shapes: &HashMap<String, String>,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let id = node_id(
        ids,
        vec![
            "slides".into(),
            slide.order.to_string(),
            "tables".into(),
            table.shape_id.clone(),
        ],
        Some(format!("table:{}", table.shape_id)),
        Some(table.locator.clone()),
    )?;
    let mut node = DocumentNode::new(&id, DocumentNodeKind::Table)
        .with_locator(table.locator.clone())
        .with_ordinal(0);
    node.extensions.insert(
        "grist.presentation_odf".into(),
        serde_json::to_value(table).map_err(transform_error)?,
    );
    graph.add_node(node);
    graph.add_contains(
        shapes
            .get(&table.shape_id)
            .map(String::as_str)
            .unwrap_or(slide_node),
        &id,
    );
    for row in &table.rows {
        let row_id = node_id(
            ids,
            vec![
                "slides".into(),
                slide.order.to_string(),
                "tables".into(),
                table.shape_id.clone(),
                "rows".into(),
                row.index.to_string(),
            ],
            None,
            Some(row.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(&row_id, DocumentNodeKind::TableRow)
                .with_locator(row.locator.clone())
                .with_ordinal(row.index),
        );
        graph.add_contains(&id, &row_id);
        for cell in &row.cells {
            let cell_id = node_id(
                ids,
                vec![
                    "slides".into(),
                    slide.order.to_string(),
                    "tables".into(),
                    table.shape_id.clone(),
                    "rows".into(),
                    row.index.to_string(),
                    "cells".into(),
                    cell.column.to_string(),
                ],
                None,
                Some(cell.locator.clone()),
            )?;
            let mut cell_node = DocumentNode::new(&cell_id, DocumentNodeKind::TableCell)
                .with_text(&cell.text)
                .with_locator(cell.locator.clone())
                .with_ordinal(cell.column);
            cell_node.extensions.insert(
                "grist.presentation_odf".into(),
                serde_json::to_value(cell).map_err(transform_error)?,
            );
            graph.add_node(cell_node);
            graph.add_contains(&row_id, &cell_id);
        }
    }
    Ok(())
}

fn node_id(
    ids: &GraphIdGenerator,
    structural_path: Vec<String>,
    native_id: Option<String>,
    locator: Option<crate::core::SourceLocator>,
) -> Result<String, TransformError> {
    ids.node_id(&ProjectionAddress {
        structural_path,
        native_id,
        locator,
    })
    .map_err(transform_error)
}

fn transform_error(error: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: error.to_string(),
    }
}
