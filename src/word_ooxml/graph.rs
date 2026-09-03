//! Normalized graph projection for authoritative WordprocessingML content.

use super::model::*;
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentNode, DocumentNodeKind, DocumentRelation,
    GraphIdGenerator, GraphIdentityError, ProjectionAddress,
};
use serde_json::{Value, json};

pub(crate) fn project_content(
    document: &WordOoxmlDocument,
    graph: &mut DocumentGraph,
    identities: &GraphIdGenerator,
    root_id: &str,
) -> Result<(), GraphIdentityError> {
    let mut projector = Projector {
        graph,
        identities,
        ordinal: 10_000,
    };
    projector.story(&document.body, root_id, vec!["body".into()])?;
    for (index, section) in document.body.sections.iter().enumerate() {
        projector.section(section, root_id, vec!["sections".into(), index.to_string()])?;
    }
    for (index, note) in document.footnotes.iter().enumerate() {
        projector.note(note, false, root_id, index)?;
    }
    for (index, note) in document.endnotes.iter().enumerate() {
        projector.note(note, true, root_id, index)?;
    }
    for (index, item) in document.headers.iter().enumerate() {
        projector.header_footer(item, true, root_id, index)?;
    }
    for (index, item) in document.footers.iter().enumerate() {
        projector.header_footer(item, false, root_id, index)?;
    }
    projector.rich_content(document, root_id)?;
    Ok(())
}

struct Projector<'a> {
    graph: &'a mut DocumentGraph,
    identities: &'a GraphIdGenerator,
    ordinal: usize,
}

impl Projector<'_> {
    fn rich_content(
        &mut self,
        document: &WordOoxmlDocument,
        parent: &str,
    ) -> Result<(), GraphIdentityError> {
        let revision_ids = self.revisions(&document.revision_graph, parent)?;
        self.comments(&document.comments, parent)?;
        let mut object_ids = self.content_controls(&document.content_controls, parent)?;
        object_ids.extend(self.equations(&document.equations, parent)?);
        object_ids.extend(self.drawings(&document.drawings, parent)?);
        object_ids.extend(self.charts(&document.charts, parent)?);
        self.captions(&document.captions, parent, &object_ids)?;
        self.text_boxes(&document.text_boxes, parent, &object_ids)?;
        self.embedded_objects(&document.embedded_objects, parent, &object_ids)?;
        for revision in &document.revision_graph.revisions {
            if let Some(id) = revision_ids.get(&revision.node_id) {
                self.graph.add_edge(DocumentEdge::explicit(
                    id.clone(),
                    DocumentRelation::RevisionOf,
                    revision.target_xml_path.clone(),
                    revision.locator.clone(),
                ));
            }
        }
        Ok(())
    }

    fn revisions(
        &mut self,
        revisions: &WordRevisionGraph,
        parent: &str,
    ) -> Result<std::collections::HashMap<String, String>, GraphIdentityError> {
        let mut ids = std::collections::HashMap::new();
        for (index, revision) in revisions.revisions.iter().enumerate() {
            let id = self.id(
                vec!["revisions".into(), index.to_string()],
                Some(revision.node_id.clone()),
                revision.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Revision)
                .with_text(&revision.text)
                .with_locator(revision.locator.clone())
                .with_attr("revision_kind", json!(revision.revision_kind))
                .with_attr(
                    "revision_id",
                    revision
                        .revision_id
                        .clone()
                        .map_or(Value::Null, Value::from),
                );
            node.extensions
                .insert("grist.word_ooxml".into(), json!(revision));
            self.add(parent, node);
            ids.insert(revision.node_id.clone(), id);
        }
        for edge in &revisions.edges {
            let (Some(source), Some(target)) = (
                ids.get(&edge.source_revision_node_id),
                ids.get(&edge.target_revision_node_id),
            ) else {
                continue;
            };
            let relation = match edge.relation {
                WordRevisionRelation::Contains => DocumentRelation::ParentOf,
                WordRevisionRelation::MovePair => DocumentRelation::AlternativeRepresentationOf,
                WordRevisionRelation::RangePair => DocumentRelation::Defines,
            };
            self.graph.add_edge(DocumentEdge::explicit(
                source.clone(),
                relation,
                target.clone(),
                edge.locator.clone(),
            ));
        }
        Ok(ids)
    }

    fn comments(
        &mut self,
        comments: &[WordComment],
        parent: &str,
    ) -> Result<(), GraphIdentityError> {
        let mut ids = std::collections::HashMap::new();
        for (index, comment) in comments.iter().enumerate() {
            let id = self.id(
                vec!["comments".into(), index.to_string()],
                Some(format!("comment:{}", comment.id)),
                comment.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Comment)
                .with_text(&comment.text)
                .with_locator(comment.locator.clone())
                .with_attr("comment_id", comment.id.clone())
                .with_attr(
                    "author",
                    comment.author.clone().map_or(Value::Null, Value::from),
                );
            node.extensions
                .insert("grist.word_ooxml".into(), json!(comment));
            self.add(parent, node);
            for (anchor_index, anchor) in comment.anchors.iter().enumerate() {
                self.graph.add_edge(DocumentEdge::explicit(
                    id.clone(),
                    DocumentRelation::Annotates,
                    format!("comment-anchor:{}:{anchor_index}", comment.id),
                    anchor.locator.clone(),
                ));
            }
            ids.insert(comment.id.clone(), id);
        }
        for comment in comments {
            if let (Some(source), Some(parent_id)) = (
                ids.get(&comment.id),
                comment
                    .parent_comment_id
                    .as_ref()
                    .and_then(|id| ids.get(id)),
            ) {
                self.graph.add_edge(DocumentEdge::explicit(
                    source.clone(),
                    DocumentRelation::ReplyTo,
                    parent_id.clone(),
                    comment.locator.clone(),
                ));
            }
        }
        Ok(())
    }

    fn content_controls(
        &mut self,
        controls: &[WordContentControl],
        parent: &str,
    ) -> Result<std::collections::HashMap<String, String>, GraphIdentityError> {
        let mut ids = std::collections::HashMap::new();
        for (index, control) in controls.iter().enumerate() {
            let id = self.id(
                vec!["content_controls".into(), index.to_string()],
                Some(control.control_id.clone()),
                control.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::ContentControl)
                .with_text(&control.text)
                .with_locator(control.locator.clone())
                .with_attr("control_type", control.control_type.clone())
                .with_attr("tag", control.tag.clone().map_or(Value::Null, Value::from));
            node.extensions
                .insert("grist.word_ooxml".into(), json!(control));
            self.add(parent, node);
            ids.insert(control.control_id.clone(), id);
        }
        for control in controls {
            if let (Some(source), Some(parent_id)) = (
                ids.get(&control.control_id),
                control
                    .parent_control_id
                    .as_ref()
                    .and_then(|id| ids.get(id)),
            ) {
                self.graph.add_edge(DocumentEdge::explicit(
                    parent_id.clone(),
                    DocumentRelation::ParentOf,
                    source.clone(),
                    control.locator.clone(),
                ));
            }
        }
        Ok(ids)
    }

    fn equations(
        &mut self,
        equations: &[WordEquation],
        parent: &str,
    ) -> Result<std::collections::HashMap<String, String>, GraphIdentityError> {
        let mut ids = std::collections::HashMap::new();
        for (index, equation) in equations.iter().enumerate() {
            let id = self.id(
                vec!["equations".into(), index.to_string()],
                Some(equation.equation_id.clone()),
                equation.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Equation)
                .with_text(&equation.text)
                .with_locator(equation.locator.clone())
                .with_attr("display", equation.display);
            node.extensions
                .insert("grist.word_ooxml".into(), json!(equation));
            self.add(parent, node);
            ids.insert(equation.equation_id.clone(), id);
        }
        Ok(ids)
    }

    fn drawings(
        &mut self,
        drawings: &[WordDrawing],
        parent: &str,
    ) -> Result<std::collections::HashMap<String, String>, GraphIdentityError> {
        let mut ids = std::collections::HashMap::new();
        for (index, drawing) in drawings.iter().enumerate() {
            let id = self.id(
                vec!["drawings".into(), index.to_string()],
                Some(drawing.drawing_id.clone()),
                drawing.locator.clone(),
            )?;
            let kind = if drawing.drawing_kind == WordDrawingKind::Image {
                DocumentNodeKind::Image
            } else {
                DocumentNodeKind::Figure
            };
            let mut node = DocumentNode::new(&id, kind)
                .with_locator(drawing.locator.clone())
                .with_attr("drawing_kind", json!(drawing.drawing_kind))
                .with_attr(
                    "alt_text",
                    drawing.alt_text.clone().map_or(Value::Null, Value::from),
                );
            node.name = drawing.name.clone();
            node.extensions
                .insert("grist.word_ooxml".into(), json!(drawing));
            self.add(parent, node);
            for relationship in &drawing.relationships {
                let relation = if relationship.external {
                    DocumentRelation::LinksTo
                } else {
                    DocumentRelation::References
                };
                self.graph.add_edge(DocumentEdge::explicit(
                    id.clone(),
                    relation,
                    relationship
                        .resolved_part
                        .clone()
                        .unwrap_or_else(|| relationship.target.clone()),
                    relationship.locator.clone(),
                ));
            }
            ids.insert(drawing.drawing_id.clone(), id);
        }
        Ok(ids)
    }

    fn charts(
        &mut self,
        charts: &[WordChart],
        parent: &str,
    ) -> Result<std::collections::HashMap<String, String>, GraphIdentityError> {
        let mut ids = std::collections::HashMap::new();
        for (index, chart) in charts.iter().enumerate() {
            let id = self.id(
                vec!["charts".into(), index.to_string()],
                Some(chart.chart_id.clone()),
                chart.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Chart)
                .with_text(&chart.text)
                .with_locator(chart.locator.clone())
                .with_attr("chart_types", json!(chart.chart_types));
            node.name = chart.title.clone();
            node.extensions
                .insert("grist.word_ooxml".into(), json!(chart));
            self.add(parent, node);
            if let Some(part) = &chart.chart_part {
                self.graph.add_edge(DocumentEdge::explicit(
                    id.clone(),
                    DocumentRelation::SourceOf,
                    part.clone(),
                    chart.locator.clone(),
                ));
            }
            ids.insert(chart.chart_id.clone(), id);
        }
        Ok(ids)
    }

    fn captions(
        &mut self,
        captions: &[WordCaption],
        parent: &str,
        objects: &std::collections::HashMap<String, String>,
    ) -> Result<(), GraphIdentityError> {
        for (index, caption) in captions.iter().enumerate() {
            let id = self.id(
                vec!["captions".into(), index.to_string()],
                Some(caption.caption_id.clone()),
                caption.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Caption)
                .with_text(&caption.text)
                .with_locator(caption.locator.clone())
                .with_attr(
                    "label",
                    caption.label.clone().map_or(Value::Null, Value::from),
                );
            node.extensions
                .insert("grist.word_ooxml".into(), json!(caption));
            self.add(parent, node);
            if let Some(target) = caption
                .target_object_id
                .as_ref()
                .and_then(|target| objects.get(target))
            {
                let edge = DocumentEdge::inferred(
                    id,
                    DocumentRelation::CaptionFor,
                    target.clone(),
                    caption
                        .inference_rule
                        .clone()
                        .unwrap_or_else(|| "grist.word_ooxml.caption-association.v1".to_string()),
                    crate::core::LocatorConfidence::new(0.75)
                        .expect("caption association confidence is valid"),
                )
                .with_inference_evidence(caption.locator.clone());
                self.graph.add_edge(edge);
            }
        }
        Ok(())
    }

    fn text_boxes(
        &mut self,
        text_boxes: &[WordTextBox],
        parent: &str,
        objects: &std::collections::HashMap<String, String>,
    ) -> Result<(), GraphIdentityError> {
        for (index, text_box) in text_boxes.iter().enumerate() {
            let id = self.id(
                vec!["text_boxes".into(), index.to_string()],
                Some(text_box.text_box_id.clone()),
                text_box.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Text)
                .with_text(&text_box.text)
                .with_locator(text_box.locator.clone())
                .with_attr("role", "text_box");
            node.extensions
                .insert("grist.word_ooxml".into(), json!(text_box));
            self.add(parent, node);
            if let Some(drawing) = text_box
                .drawing_id
                .as_ref()
                .and_then(|drawing| objects.get(drawing))
            {
                self.graph.add_edge(DocumentEdge::explicit(
                    drawing.clone(),
                    DocumentRelation::ParentOf,
                    id,
                    text_box.locator.clone(),
                ));
            }
        }
        Ok(())
    }

    fn embedded_objects(
        &mut self,
        embedded: &[WordEmbeddedObject],
        parent: &str,
        objects: &std::collections::HashMap<String, String>,
    ) -> Result<(), GraphIdentityError> {
        for (index, object) in embedded.iter().enumerate() {
            let id = self.id(
                vec!["embedded_objects".into(), index.to_string()],
                Some(object.object_id.clone()),
                object.locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Attachment)
                .with_locator(object.locator.clone())
                .with_attr(
                    "program_id",
                    object.program_id.clone().map_or(Value::Null, Value::from),
                );
            node.extensions
                .insert("grist.word_ooxml".into(), json!(object));
            self.add(parent, node);
            if let Some((_, drawing)) = objects
                .iter()
                .find(|(native, _)| object.object_id.starts_with(native.as_str()))
            {
                self.graph.add_edge(DocumentEdge::explicit(
                    id.clone(),
                    DocumentRelation::EmbeddedIn,
                    drawing.clone(),
                    object.locator.clone(),
                ));
            }
            if let Some(target) = &object.child_artifact_part {
                self.graph.add_edge(DocumentEdge::explicit(
                    id,
                    DocumentRelation::SourceOf,
                    target.clone(),
                    object.locator.clone(),
                ));
            }
        }
        Ok(())
    }
    fn id(
        &self,
        path: Vec<String>,
        native_id: Option<String>,
        locator: crate::core::SourceLocator,
    ) -> Result<String, GraphIdentityError> {
        self.identities.node_id(&ProjectionAddress {
            structural_path: path,
            native_id,
            locator: Some(locator),
        })
    }

    fn add(&mut self, parent: &str, mut node: DocumentNode) -> String {
        self.ordinal += 1;
        node.ordinal = Some(self.ordinal);
        let id = node.id.clone();
        self.graph.add_node(node);
        self.graph.add_contains(parent, &id);
        id
    }

    fn story(
        &mut self,
        story: &WordStory,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        for (index, block) in story.blocks.iter().enumerate() {
            let mut block_path = path.clone();
            block_path.push(index.to_string());
            self.block(block, parent, block_path)?;
        }
        Ok(())
    }

    fn block(
        &mut self,
        block: &WordBlock,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        match block {
            WordBlock::Paragraph(paragraph) => self.paragraph(paragraph, parent, path),
            WordBlock::Table(table) => self.table(table, parent, path),
        }
    }

    fn paragraph(
        &mut self,
        paragraph: &WordParagraph,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        let mut paragraph_parent = parent.to_string();
        if let Some(numbering) = &paragraph.numbering {
            let list_path = [path.clone(), vec!["list".into()]].concat();
            let list_id = self.id(list_path, None, paragraph.locator.clone())?;
            let list = DocumentNode::new(&list_id, DocumentNodeKind::List)
                .with_locator(paragraph.locator.clone())
                .with_attr("ordered", numbering.ordered)
                .with_attr("num_id", numbering.num_id);
            self.add(parent, list);
            let item_path = [path.clone(), vec!["item".into()]].concat();
            let item_id = self.id(item_path, None, paragraph.locator.clone())?;
            let item = DocumentNode::new(&item_id, DocumentNodeKind::ListItem)
                .with_locator(paragraph.locator.clone())
                .with_attr("ordered", numbering.ordered)
                .with_attr("item_number", numbering.ordinal)
                .with_attr("level", numbering.level)
                .with_attr("label", numbering.label.clone());
            paragraph_parent = self.add(&list_id, item);
        }
        let kind = if paragraph.heading_level.is_some() {
            DocumentNodeKind::Heading
        } else {
            DocumentNodeKind::Paragraph
        };
        let paragraph_id = self.id(
            [path.clone(), vec!["paragraph".into()]].concat(),
            None,
            paragraph.locator.clone(),
        )?;
        let mut node = DocumentNode::new(&paragraph_id, kind)
            .with_text(&paragraph.text)
            .with_locator(paragraph.locator.clone());
        if let Some(style_id) = &paragraph.style_id {
            node = node.with_attr("style_id", style_id.clone());
        }
        if let Some(level) = paragraph.heading_level {
            node = node.with_attr("level", level);
        }
        node.extensions.insert(
            "grist.word_ooxml".into(),
            json!({
                "direct_properties": paragraph.direct_properties,
                "effective_properties": paragraph.properties,
                "numbering_reference": paragraph.numbering_reference,
                "numbering": paragraph.numbering,
                "section": paragraph.section,
            }),
        );
        self.add(&paragraph_parent, node);
        for (index, inline) in paragraph.inlines.iter().enumerate() {
            self.inline(
                inline,
                &paragraph_id,
                [path.clone(), vec!["inline".into(), index.to_string()]].concat(),
            )?;
        }
        let summarized_fields = paragraph.fields.iter().filter(|field| {
            !paragraph.inlines.iter().any(|inline| {
                matches!(inline, WordInline::Field(inline_field) if inline_field.locator == field.locator)
            })
        });
        for (index, field) in summarized_fields.enumerate() {
            self.field(
                field,
                &paragraph_id,
                [path.clone(), vec!["field".into(), index.to_string()]].concat(),
            )?;
        }
        if let Some(section) = &paragraph.section {
            self.section(
                section,
                &paragraph_id,
                [path, vec!["section".into()]].concat(),
            )?;
        }
        Ok(())
    }

    fn inline(
        &mut self,
        inline: &WordInline,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        match inline {
            WordInline::Run(run) => self.run(run, parent, path),
            WordInline::Hyperlink(link) => self.hyperlink(link, parent, path),
            WordInline::Bookmark(bookmark) => self.bookmark(bookmark, parent, path),
            WordInline::Field(field) => self.field(field, parent, path),
        }
    }

    fn run(
        &mut self,
        run: &WordRun,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        let id = self.id(path.clone(), None, run.locator.clone())?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::TextRun)
            .with_text(&run.text)
            .with_locator(run.locator.clone());
        if let Some(style_id) = &run.style_id {
            node = node.with_attr("style_id", style_id.clone());
        }
        if let Some(bold) = run.effective_formatting.bold {
            node = node.with_attr("bold", bold);
        }
        if let Some(italic) = run.effective_formatting.italic {
            node = node.with_attr("italic", italic);
        }
        node.extensions.insert(
            "grist.word_ooxml".into(),
            json!({
                "direct_formatting": run.direct_formatting,
                "effective_formatting": run.effective_formatting,
                "contents": run.contents,
            }),
        );
        self.add(parent, node);
        for (index, content) in run.contents.iter().enumerate() {
            let (note_kind, note_id, locator) = match content {
                WordRunContent::FootnoteReference { id, locator } => ("footnote", id, locator),
                WordRunContent::EndnoteReference { id, locator } => ("endnote", id, locator),
                _ => continue,
            };
            let reference_id = self.id(
                [path.clone(), vec!["reference".into(), index.to_string()]].concat(),
                Some(format!("{note_kind}:{note_id}")),
                locator.clone(),
            )?;
            let reference = DocumentNode::new(&reference_id, DocumentNodeKind::Reference)
                .with_locator(locator.clone())
                .with_attr("note_kind", note_kind)
                .with_attr("note_id", note_id.clone());
            self.add(&id, reference);
            self.graph.add_edge(DocumentEdge::explicit(
                reference_id,
                DocumentRelation::References,
                format!("{note_kind}:{note_id}"),
                locator.clone(),
            ));
        }
        Ok(())
    }

    fn hyperlink(
        &mut self,
        link: &WordHyperlink,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        let id = self.id(path, link.relationship_id.clone(), link.locator.clone())?;
        let destination = link
            .target
            .clone()
            .or_else(|| link.anchor.as_ref().map(|anchor| format!("#{anchor}")))
            .unwrap_or_default();
        let node = DocumentNode::new(&id, DocumentNodeKind::Link)
            .with_text(&link.text)
            .with_locator(link.locator.clone())
            .with_attr("destination", destination.clone())
            .with_attr(
                "relationship_id",
                link.relationship_id
                    .clone()
                    .map_or(Value::Null, Value::from),
            );
        self.add(parent, node);
        if !destination.is_empty() {
            self.graph.add_edge(DocumentEdge::explicit(
                id,
                DocumentRelation::LinksTo,
                destination,
                link.locator.clone(),
            ));
        }
        Ok(())
    }

    fn bookmark(
        &mut self,
        bookmark: &WordBookmark,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        let native = bookmark
            .name
            .clone()
            .unwrap_or_else(|| format!("bookmark:{}", bookmark.id));
        let id = self.id(path, Some(native), bookmark.locator.clone())?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::Bookmark)
            .with_locator(bookmark.locator.clone())
            .with_attr("bookmark_id", bookmark.id.clone())
            .with_attr("kind", json!(bookmark.bookmark_kind));
        node.name = bookmark.name.clone();
        self.add(parent, node);
        Ok(())
    }

    fn field(
        &mut self,
        field: &WordField,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        let kind = match field.field_type.as_str() {
            "CITATION" => DocumentNodeKind::Citation,
            "REF" | "PAGEREF" | "NOTEREF" => DocumentNodeKind::Reference,
            _ => DocumentNodeKind::FormField,
        };
        let id = self.id(path.clone(), None, field.locator.clone())?;
        let mut node = DocumentNode::new(&id, kind)
            .with_name(&field.field_type)
            .with_locator(field.locator.clone())
            .with_attr("instruction", field.instruction.clone())
            .with_attr("result_text", field.result_text.clone());
        node.extensions
            .insert("grist.word_ooxml".into(), json!(field));
        self.add(parent, node);
        for (index, run) in field.result_runs.iter().enumerate() {
            self.run(
                run,
                &id,
                [path.clone(), vec!["result_run".into(), index.to_string()]].concat(),
            )?;
        }
        if let Some(target) = field.instruction.split_whitespace().nth(1) {
            let relation = if field.field_type == "CITATION" {
                DocumentRelation::Cites
            } else {
                DocumentRelation::References
            };
            self.graph.add_edge(DocumentEdge::explicit(
                id,
                relation,
                target,
                field.locator.clone(),
            ));
        }
        Ok(())
    }

    fn table(
        &mut self,
        table: &WordTable,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        let id = self.id(path.clone(), None, table.locator.clone())?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::Table)
            .with_locator(table.locator.clone())
            .with_attr(
                "style_id",
                table.style_id.clone().map_or(Value::Null, Value::from),
            );
        node.extensions.insert(
            "grist.word_ooxml".into(),
            json!({
                "width": table.width,
                "layout": table.layout,
                "grid_columns": table.grid_columns,
            }),
        );
        self.add(parent, node);
        for (row_index, row) in table.rows.iter().enumerate() {
            let row_path = [path.clone(), vec!["row".into(), row_index.to_string()]].concat();
            let row_id = self.id(row_path.clone(), None, row.locator.clone())?;
            let row_node = DocumentNode::new(&row_id, DocumentNodeKind::TableRow)
                .with_locator(row.locator.clone())
                .with_attr("header", row.is_header)
                .with_attr("cant_split", row.cant_split);
            self.add(&id, row_node);
            for (cell_index, cell) in row.cells.iter().enumerate() {
                let cell_path = [
                    row_path.clone(),
                    vec!["cell".into(), cell_index.to_string()],
                ]
                .concat();
                let cell_id = self.id(cell_path.clone(), None, cell.locator.clone())?;
                let mut cell_node = DocumentNode::new(&cell_id, DocumentNodeKind::TableCell)
                    .with_text(&cell.text)
                    .with_locator(cell.locator.clone())
                    .with_attr("column_span", cell.column_span)
                    .with_attr("row_span", cell.row_span)
                    .with_attr("grid_column", cell.grid_column);
                cell_node.extensions.insert(
                    "grist.word_ooxml".into(),
                    json!({
                        "vertical_merge": cell.vertical_merge,
                        "horizontal_merge": cell.horizontal_merge,
                        "merged_into": cell.merged_into,
                        "width": cell.width,
                    }),
                );
                self.add(&row_id, cell_node);
                for (block_index, block) in cell.blocks.iter().enumerate() {
                    self.block(
                        block,
                        &cell_id,
                        [
                            cell_path.clone(),
                            vec!["block".into(), block_index.to_string()],
                        ]
                        .concat(),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn section(
        &mut self,
        section: &WordSection,
        parent: &str,
        path: Vec<String>,
    ) -> Result<(), GraphIdentityError> {
        let id = self.id(path, None, section.locator.clone())?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::Section)
            .with_locator(section.locator.clone())
            .with_attr("columns", section.columns.count)
            .with_attr(
                "break_type",
                section.break_type.clone().map_or(Value::Null, Value::from),
            );
        node.extensions
            .insert("grist.word_ooxml".into(), json!(section));
        self.add(parent, node);
        for reference in section
            .header_references
            .iter()
            .chain(section.footer_references.iter())
        {
            if let Some(part) = &reference.part {
                self.graph.add_edge(DocumentEdge::explicit(
                    id.clone(),
                    DocumentRelation::References,
                    part,
                    reference.locator.clone(),
                ));
            }
        }
        Ok(())
    }

    fn note(
        &mut self,
        note: &WordNote,
        endnote: bool,
        parent: &str,
        index: usize,
    ) -> Result<(), GraphIdentityError> {
        let kind = if endnote {
            DocumentNodeKind::Endnote
        } else {
            DocumentNodeKind::Footnote
        };
        let path = vec![
            if endnote { "endnotes" } else { "footnotes" }.into(),
            index.to_string(),
        ];
        let id = self.id(path.clone(), Some(note.id.clone()), note.locator.clone())?;
        let mut node = DocumentNode::new(&id, kind)
            .with_text(&note.story.visible_text)
            .with_locator(note.locator.clone())
            .with_attr("label", note.id.clone());
        node.extensions.insert(
            "grist.word_ooxml".into(),
            json!({"note_type": note.note_type, "part": note.story.part}),
        );
        self.add(parent, node);
        self.story(&note.story, &id, [path, vec!["story".into()]].concat())
    }

    fn header_footer(
        &mut self,
        item: &WordHeaderFooter,
        header: bool,
        parent: &str,
        index: usize,
    ) -> Result<(), GraphIdentityError> {
        let kind = if header {
            DocumentNodeKind::Header
        } else {
            DocumentNodeKind::Footer
        };
        let path = vec![
            if header { "headers" } else { "footers" }.into(),
            index.to_string(),
        ];
        let id = self.id(path.clone(), Some(item.part.clone()), item.locator.clone())?;
        let node = DocumentNode::new(&id, kind)
            .with_name(&item.part)
            .with_locator(item.locator.clone())
            .with_attr("relationship_ids", json!(item.relationship_ids));
        self.add(parent, node);
        self.story(&item.story, &id, [path, vec!["story".into()]].concat())
    }
}
