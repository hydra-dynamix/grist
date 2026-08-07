//! Deterministic graph projection for the PresentationML package layer.

use super::*;
use crate::core::{LocatorConfidence, SchemaVersion};
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, ToDocumentGraph,
    TransformError,
};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

impl ToDocumentGraph for PresentationOoxmlDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::PRESENTATION_OOXML_V1, "presentation_ooxml")
            .map_err(transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::PresentationOoxml)
            .with_projection(
                "presentation_ooxml",
                SchemaVersion::PRESENTATION_OOXML_V1,
                "grist.presentation_ooxml.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = context.language;
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| self.package_kind.format_id().to_string()),
        );
        graph.attrs = context.attrs;
        let id = |path: Vec<String>,
                  native: Option<String>,
                  locator: Option<crate::core::SourceLocator>| {
            identities
                .node_id(&ProjectionAddress {
                    structural_path: path,
                    native_id: native,
                    locator,
                })
                .map_err(transform_error)
        };
        let root_id = id(
            vec!["package".into()],
            Some(self.main_presentation_part.clone()),
            Some(self.main_presentation_locator.clone()),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name(&self.main_presentation_part)
            .with_locator(self.main_presentation_locator.clone())
            .with_ordinal(0);
        root.extensions.insert(
            "grist.presentation_ooxml".into(),
            json!({
                "package_kind": self.package_kind,
                "package_media_type": self.package_media_type,
                "main_presentation_part": self.main_presentation_part,
            }),
        );
        graph.add_node(root);

        let attachment_parts = self
            .macro_projects
            .iter()
            .map(|item| item.part.as_str())
            .chain(self.child_artifacts.iter().map(|item| item.part.as_str()))
            .collect::<BTreeSet<_>>();
        let slide_parts = self
            .slides
            .iter()
            .filter_map(|item| item.part.as_deref())
            .collect::<BTreeSet<_>>();
        let mut part_ids = HashMap::new();
        for (index, part) in self.parts.iter().enumerate() {
            let node_id = id(
                vec!["package".into(), "parts".into(), index.to_string()],
                Some(format!("part:{}", part.path)),
                Some(part.locator.clone()),
            )?;
            let kind = if attachment_parts.contains(part.path.as_str()) {
                DocumentNodeKind::Attachment
            } else if slide_parts.contains(part.path.as_str()) {
                DocumentNodeKind::ArchiveMember
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
                "grist.presentation_ooxml".into(),
                serde_json::to_value(part).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            part_ids.insert(part.path.as_str(), node_id);
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
            let relation = if lowered.ends_with("/hyperlink") {
                DocumentRelation::LinksTo
            } else if ["/image", "/oleobject", "/package", "/embeddedobject"]
                .iter()
                .any(|suffix| lowered.ends_with(suffix))
            {
                DocumentRelation::EmbeddedIn
            } else {
                DocumentRelation::References
            };
            let mut edge = DocumentEdge::new(source, relation, target)
                .with_locator(relationship.locator.clone());
            edge.extensions.insert(
                "grist.presentation_ooxml".into(),
                serde_json::to_value(relationship).map_err(transform_error)?,
            );
            graph.add_edge(edge);
        }

        let mut previous_slide = None;
        let mut slide_node_ids = HashMap::new();
        for slide in &self.slides {
            let node_id = id(
                vec!["slides".into(), slide.order.to_string()],
                Some(format!("slide:{}", slide.slide_id)),
                Some(slide.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Slide)
                .with_name(
                    slide
                        .part
                        .as_deref()
                        .unwrap_or("unresolved-presentation-slide"),
                )
                .with_locator(slide.locator.clone())
                .with_ordinal(slide.order)
                .with_attr("slide_id", slide.slide_id.clone())
                .with_attr("hidden", slide.hidden);
            node.extensions.insert(
                "grist.presentation_ooxml".into(),
                serde_json::to_value(slide).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            if let Some(part_id) = slide.part.as_deref().and_then(|part| part_ids.get(part)) {
                graph.add_edge(DocumentEdge::explicit(
                    node_id.clone(),
                    DocumentRelation::References,
                    part_id.clone(),
                    slide.locator.clone(),
                ));
            }
            if let Some(previous) = previous_slide {
                graph.add_edge(DocumentEdge::explicit(
                    previous,
                    DocumentRelation::Precedes,
                    node_id.clone(),
                    slide.locator.clone(),
                ));
            }
            previous_slide = Some(node_id);
            slide_node_ids.insert(slide.order, previous_slide.clone().expect("slide node ID"));
        }

        project_slide_content(self, &identities, &mut graph, &slide_node_ids)?;

        let mut ordinal = self.parts.len().saturating_add(self.slides.len());
        for (collection, name) in [
            (&self.masters, "masters"),
            (&self.layouts, "layouts"),
            (&self.themes, "themes"),
        ] {
            for item in collection {
                let node_id = id(
                    vec![name.into(), item.part.clone()],
                    item.native_id
                        .clone()
                        .or_else(|| Some(format!("{name}:{}", item.part))),
                    Some(item.locator.clone()),
                )?;
                let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Metadata)
                    .with_name(&item.part)
                    .with_locator(item.locator.clone())
                    .with_ordinal(ordinal)
                    .with_attr("structural_kind", json!(item.kind));
                node.extensions.insert(
                    "grist.presentation_ooxml".into(),
                    serde_json::to_value(item).map_err(transform_error)?,
                );
                graph.add_node(node);
                graph.add_contains(&root_id, &node_id);
                if let Some(part_id) = part_ids.get(item.part.as_str()) {
                    graph.add_edge(DocumentEdge::explicit(
                        node_id,
                        DocumentRelation::References,
                        part_id.clone(),
                        item.locator.clone(),
                    ));
                }
                ordinal += 1;
            }
        }

        for (index, action) in self.actions.iter().enumerate() {
            let node_id = id(
                vec!["actions".into(), index.to_string()],
                action
                    .relationship_id
                    .as_ref()
                    .map(|value| format!("action:{}:{value}", action.source_part)),
                Some(action.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Link)
                .with_name(format!("{:?}", action.action_kind).to_ascii_lowercase())
                .with_locator(action.locator.clone())
                .with_ordinal(ordinal)
                .with_attr("inert", true)
                .with_attr("external", action.external)
                .with_attr(
                    "destination",
                    action
                        .target
                        .clone()
                        .or_else(|| action.action.clone())
                        .unwrap_or_default(),
                );
            node.text = action.action.clone().or_else(|| action.target.clone());
            node.extensions.insert(
                "grist.presentation_ooxml".into(),
                serde_json::to_value(action).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            if let Some(target) = &action.target {
                graph.add_edge(DocumentEdge::explicit(
                    node_id,
                    DocumentRelation::LinksTo,
                    target.clone(),
                    action.locator.clone(),
                ));
            }
            ordinal += 1;
        }

        for property in self
            .properties
            .core
            .iter()
            .chain(self.properties.extended.iter())
        {
            let node_id = id(
                vec!["properties".into(), ordinal.to_string()],
                None,
                Some(property.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Metadata)
                .with_name(&property.name)
                .with_text(&property.value)
                .with_locator(property.locator.clone())
                .with_ordinal(ordinal);
            node.extensions.insert(
                "grist.presentation_ooxml".into(),
                serde_json::to_value(property).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            ordinal += 1;
        }
        for property in &self.properties.custom {
            let node_id = id(
                vec!["custom_properties".into(), ordinal.to_string()],
                property.name.as_ref().map(|name| format!("custom:{name}")),
                Some(property.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Metadata)
                .with_name(property.name.as_deref().unwrap_or("custom-property"))
                .with_locator(property.locator.clone())
                .with_ordinal(ordinal);
            node.text = property.value.clone();
            node.extensions.insert(
                "grist.presentation_ooxml".into(),
                serde_json::to_value(property).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
            ordinal += 1;
        }
        graph
            .finalize_projection(&identities)
            .map_err(transform_error)?;
        Ok(graph)
    }
}

fn content_id(
    ids: &GraphIdGenerator,
    path: Vec<String>,
    native: Option<String>,
    locator: crate::core::SourceLocator,
) -> Result<String, TransformError> {
    ids.node_id(&ProjectionAddress {
        structural_path: path,
        native_id: native,
        locator: Some(locator),
    })
    .map_err(transform_error)
}

fn project_slide_content(
    document: &PresentationOoxmlDocument,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
    slide_ids: &HashMap<usize, String>,
) -> Result<(), TransformError> {
    for slide in &document.slide_contents {
        let Some(slide_node) = slide_ids.get(&slide.order) else {
            continue;
        };
        let mut shape_ids = HashMap::new();
        for shape in &slide.shapes {
            let node_id = content_id(
                ids,
                vec![
                    "slides".into(),
                    slide.order.to_string(),
                    "shapes".into(),
                    shape.shape_id.clone(),
                ],
                Some(format!("shape:{}:{}", slide.slide_id, shape.shape_id)),
                shape.locator.clone(),
            )?;
            let kind = match shape.kind {
                PresentationShapeKind::Picture => DocumentNodeKind::Image,
                PresentationShapeKind::Group => DocumentNodeKind::Container,
                PresentationShapeKind::Connector => DocumentNodeKind::Figure,
                _ => DocumentNodeKind::Container,
            };
            let mut node = DocumentNode::new(&node_id, kind)
                .with_locator(shape.locator.clone())
                .with_ordinal(shape.z_order)
                .with_attr("shape_id", shape.shape_id.clone())
                .with_attr(
                    "reading_order_rank",
                    slide
                        .reading_order
                        .entries
                        .iter()
                        .find(|entry| entry.shape_id == shape.shape_id)
                        .map_or(Value::Null, |entry| {
                            Value::from(u64::try_from(entry.rank).unwrap_or(u64::MAX))
                        }),
                );
            node.name = shape.name.clone();
            node.extensions.insert(
                "grist.presentation_ooxml".into(),
                serde_json::to_value(shape).map_err(transform_error)?,
            );
            graph.add_node(node);
            let parent = shape
                .parent_shape_id
                .as_ref()
                .and_then(|id| shape_ids.get(id))
                .unwrap_or(slide_node);
            graph.add_contains(parent, &node_id);
            shape_ids.insert(shape.shape_id.clone(), node_id.clone());
            project_shape_text(slide, shape, &node_id, ids, graph)?;
        }
        for pair in slide.reading_order.entries.windows(2) {
            let (Some(source), Some(target)) = (
                shape_ids.get(&pair[0].shape_id),
                shape_ids.get(&pair[1].shape_id),
            ) else {
                continue;
            };
            let confidence = LocatorConfidence::new(pair[0].confidence.min(pair[1].confidence))
                .map_err(transform_error)?;
            graph.add_edge(
                DocumentEdge::inferred(
                    source.clone(),
                    DocumentRelation::Precedes,
                    target.clone(),
                    "grist.presentation_ooxml.reading-order.v1",
                    confidence,
                )
                .with_inference_evidence(pair[0].locator.clone())
                .with_inference_evidence(pair[1].locator.clone()),
            );
        }
        project_slide_objects(slide, slide_node, &shape_ids, ids, graph)?;
    }
    Ok(())
}

fn project_shape_text(
    slide: &PresentationSlideContent,
    shape: &PresentationShape,
    shape_node: &str,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let Some(body) = &shape.text_body else {
        return Ok(());
    };
    for paragraph in &body.paragraphs {
        let paragraph_id = content_id(
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
            paragraph.locator.clone(),
        )?;
        let mut node = DocumentNode::new(&paragraph_id, DocumentNodeKind::Paragraph)
            .with_text(&paragraph.text)
            .with_locator(paragraph.locator.clone())
            .with_ordinal(paragraph.index);
        node.extensions.insert(
            "grist.presentation_ooxml".into(),
            serde_json::to_value(paragraph).map_err(transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(shape_node, &paragraph_id);
        for run in &paragraph.runs {
            let run_id = content_id(
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
                run.field_id.clone(),
                run.locator.clone(),
            )?;
            let mut run_node = DocumentNode::new(&run_id, DocumentNodeKind::TextRun)
                .with_text(&run.text)
                .with_locator(run.locator.clone())
                .with_ordinal(run.index);
            run_node.extensions.insert(
                "grist.presentation_ooxml".into(),
                serde_json::to_value(run).map_err(transform_error)?,
            );
            graph.add_node(run_node);
            graph.add_contains(&paragraph_id, &run_id);
        }
    }
    Ok(())
}

fn project_slide_objects(
    slide: &PresentationSlideContent,
    slide_node: &str,
    shapes: &HashMap<String, String>,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    for table in &slide.tables {
        project_table(slide, table, slide_node, shapes, ids, graph)?;
    }
    for (index, chart) in slide.charts.iter().enumerate() {
        add_semantic_node(
            slide,
            "charts",
            index,
            DocumentNodeKind::Chart,
            chart.title.as_deref(),
            &chart.locator,
            &chart.shape_id,
            serde_json::to_value(chart).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    for (index, equation) in slide.equations.iter().enumerate() {
        add_semantic_node(
            slide,
            "equations",
            index,
            DocumentNodeKind::Equation,
            Some(&equation.text),
            &equation.locator,
            &equation.shape_id,
            serde_json::to_value(equation).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    for (index, image) in slide.images.iter().enumerate() {
        add_semantic_node(
            slide,
            "images",
            index,
            DocumentNodeKind::Image,
            image.alt_text.as_deref().or(image.alt_title.as_deref()),
            &image.locator,
            &image.shape_id,
            serde_json::to_value(image).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    for (index, comment) in slide.comments.iter().enumerate() {
        add_semantic_node(
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
        add_semantic_node(
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
    project_metadata_objects(slide, slide_node, shapes, ids, graph)
}

#[allow(clippy::too_many_arguments)]
fn add_semantic_node(
    slide: &PresentationSlideContent,
    collection: &str,
    index: usize,
    kind: DocumentNodeKind,
    text: Option<&str>,
    locator: &crate::core::SourceLocator,
    shape_id: &str,
    value: Value,
    slide_node: &str,
    shapes: &HashMap<String, String>,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let node_id = content_id(
        ids,
        vec![
            "slides".into(),
            slide.order.to_string(),
            collection.into(),
            index.to_string(),
        ],
        None,
        locator.clone(),
    )?;
    let mut node = DocumentNode::new(&node_id, kind)
        .with_locator(locator.clone())
        .with_ordinal(index);
    node.text = text.filter(|text| !text.is_empty()).map(str::to_string);
    node.extensions
        .insert("grist.presentation_ooxml".into(), value);
    graph.add_node(node);
    let parent = shapes
        .get(shape_id)
        .map(String::as_str)
        .unwrap_or(slide_node);
    graph.add_contains(parent, &node_id);
    Ok(())
}

fn project_table(
    slide: &PresentationSlideContent,
    table: &PresentationTable,
    slide_node: &str,
    shapes: &HashMap<String, String>,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    let table_id = content_id(
        ids,
        vec![
            "slides".into(),
            slide.order.to_string(),
            "tables".into(),
            table.shape_id.clone(),
        ],
        Some(format!("table:{}", table.shape_id)),
        table.locator.clone(),
    )?;
    let mut node = DocumentNode::new(&table_id, DocumentNodeKind::Table)
        .with_locator(table.locator.clone())
        .with_ordinal(0);
    node.extensions.insert(
        "grist.presentation_ooxml".into(),
        serde_json::to_value(table).map_err(transform_error)?,
    );
    graph.add_node(node);
    graph.add_contains(
        shapes
            .get(&table.shape_id)
            .map(String::as_str)
            .unwrap_or(slide_node),
        &table_id,
    );
    for row in &table.rows {
        let row_id = content_id(
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
            row.locator.clone(),
        )?;
        graph.add_node(
            DocumentNode::new(&row_id, DocumentNodeKind::TableRow)
                .with_locator(row.locator.clone())
                .with_ordinal(row.index),
        );
        graph.add_contains(&table_id, &row_id);
        for cell in &row.cells {
            let cell_id = content_id(
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
                cell.locator.clone(),
            )?;
            let text = cell
                .text_body
                .as_ref()
                .map(|body| body.text.as_str())
                .unwrap_or("");
            let mut cell_node = DocumentNode::new(&cell_id, DocumentNodeKind::TableCell)
                .with_text(text)
                .with_locator(cell.locator.clone())
                .with_ordinal(cell.column);
            cell_node.extensions.insert(
                "grist.presentation_ooxml".into(),
                serde_json::to_value(cell).map_err(transform_error)?,
            );
            graph.add_node(cell_node);
            graph.add_contains(&row_id, &cell_id);
        }
    }
    Ok(())
}

fn project_metadata_objects(
    slide: &PresentationSlideContent,
    slide_node: &str,
    shapes: &HashMap<String, String>,
    ids: &GraphIdGenerator,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError> {
    for (index, link) in slide.links.iter().enumerate() {
        let node_id = content_id(
            ids,
            vec![
                "slides".into(),
                slide.order.to_string(),
                "links".into(),
                index.to_string(),
            ],
            link.relationship_id.clone(),
            link.locator.clone(),
        )?;
        let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Link)
            .with_locator(link.locator.clone())
            .with_ordinal(index)
            .with_attr("external", link.external)
            .with_attr("inert", true)
            .with_attr(
                "destination",
                link.target
                    .clone()
                    .or_else(|| link.action.clone())
                    .unwrap_or_default(),
            );
        node.text = link.target.clone().or_else(|| link.action.clone());
        node.extensions.insert(
            "grist.presentation_ooxml".into(),
            serde_json::to_value(link).map_err(transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(
            shapes
                .get(&link.source_shape_id)
                .map(String::as_str)
                .unwrap_or(slide_node),
            &node_id,
        );
        if let Some(target) = &link.target {
            graph.add_edge(DocumentEdge::explicit(
                node_id,
                DocumentRelation::LinksTo,
                target.clone(),
                link.locator.clone(),
            ));
        }
    }
    if let Some(transition) = &slide.transition {
        add_semantic_node(
            slide,
            "transition",
            0,
            DocumentNodeKind::Metadata,
            transition.kind.as_deref(),
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
        add_semantic_node(
            slide,
            "animations",
            index,
            DocumentNodeKind::Metadata,
            animation.text.as_deref(),
            &animation.locator,
            animation
                .target_shape_ids
                .first()
                .map(String::as_str)
                .unwrap_or(""),
            serde_json::to_value(animation).map_err(transform_error)?,
            slide_node,
            shapes,
            ids,
            graph,
        )?;
    }
    for (index, object) in slide.embedded_objects.iter().enumerate() {
        add_semantic_node(
            slide,
            "embedded_objects",
            index,
            DocumentNodeKind::Attachment,
            object.name.as_deref().or(object.program_id.as_deref()),
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

fn transform_error(error: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: error.to_string(),
    }
}
