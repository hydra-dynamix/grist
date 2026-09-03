use super::MediaDocument;
use crate::core::{SchemaVersion, SourceLocator};
use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    GraphIdGenerator, ProjectionAddress, ToDocumentGraph, TransformError,
};

impl ToDocumentGraph for MediaDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::MEDIA_V1, "grist.media")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Media).with_projection(
            "media",
            SchemaVersion::MEDIA_V1,
            "grist.media.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.dialect = Some(self.format.as_str().into());
        graph.attrs = context.attrs;
        graph
            .attrs
            .insert("format".into(), self.format.as_str().into());
        graph
            .attrs
            .insert("encrypted".into(), self.encrypted.into());
        graph.attrs.insert("complete".into(), self.complete.into());
        let root_locator = byte_locator(
            0,
            usize::try_from(self.technical.byte_length).map_err(error)?,
        );
        let root = id(
            &ids,
            &["media"],
            self.format.as_str(),
            Some(root_locator.clone()),
        )?;
        let mut root_node = DocumentNode::new(&root, DocumentNodeKind::Document)
            .with_name(format!("{} media container", self.format.as_str()))
            .with_locator(root_locator)
            .with_ordinal(0);
        root_node.extensions.insert("grist.media".into(),serde_json::json!({"technical":self.technical,"metadata":self.metadata,"encrypted":self.encrypted}));
        graph.add_node(root_node);
        for stream in &self.streams {
            let node_id = id(&ids, &["media", "streams"], &stream.id, None)?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::MediaTrack)
                .with_name(stream.name.as_deref().unwrap_or(&stream.id))
                .with_locator(stream.locator.clone())
                .with_ordinal(usize::try_from(stream.index).map_err(error)?);
            node.attrs
                .insert("stream_id".into(), stream.id.clone().into());
            node.attrs
                .insert("codec".into(), stream.codec.clone().into());
            node.attrs.insert(
                "kind".into(),
                serde_json::to_value(stream.kind).map_err(error)?,
            );
            node.extensions.insert(
                "grist.media".into(),
                serde_json::to_value(stream).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &node_id);
        }
        for chapter in &self.chapters {
            let node_id = id(&ids, &["media", "chapters"], &chapter.id, None)?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Section)
                .with_name(chapter.title.as_deref().unwrap_or(&chapter.id))
                .with_locator(chapter.locator.clone())
                .with_ordinal(usize::try_from(chapter.index).map_err(error)?);
            node.attrs
                .insert("start_ms".into(), chapter.start_ms.into());
            if let Some(end) = chapter.end_ms {
                node.attrs.insert("end_ms".into(), end.into());
            }
            node.extensions.insert(
                "grist.media".into(),
                serde_json::to_value(chapter).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &node_id);
        }
        for attachment in &self.attachments {
            let node_id = id(&ids, &["media", "attachments"], &attachment.id, None)?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Attachment)
                .with_name(attachment.filename.as_deref().unwrap_or(&attachment.id))
                .with_locator(attachment.locator.clone())
                .with_ordinal(usize::try_from(attachment.index).map_err(error)?);
            node.extensions.insert(
                "grist.media".into(),
                serde_json::to_value(attachment).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &node_id);
        }
        for artwork in &self.artwork {
            let node_id = id(&ids, &["media", "artwork"], &artwork.id, None)?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Image)
                .with_name(artwork.description.as_deref().unwrap_or("embedded artwork"))
                .with_locator(artwork.locator.clone())
                .with_ordinal(usize::try_from(artwork.index).map_err(error)?);
            node.extensions.insert(
                "grist.media".into(),
                serde_json::to_value(artwork).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &node_id);
        }
        for (index, subtitle) in self.subtitle_tracks.iter().enumerate() {
            let node_id = id(&ids, &["media", "subtitles"], &subtitle.id, None)?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Transcript)
                .with_name(&subtitle.id)
                .with_locator(subtitle.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.media".into(),
                serde_json::to_value(subtitle).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &node_id);
            if let Some(document) = &subtitle.document {
                for cue in &document.cues {
                    let cue_id = id(
                        &ids,
                        &["media", "subtitles", &subtitle.id, "cues"],
                        &cue.id,
                        None,
                    )?;
                    let mut cue_node = DocumentNode::new(&cue_id, DocumentNodeKind::Cue)
                        .with_text(&cue.text)
                        .with_locator(cue.text_locator.clone())
                        .with_ordinal(cue.source_index);
                    cue_node
                        .attrs
                        .insert("cue_id".into(), cue.id.clone().into());
                    cue_node
                        .attrs
                        .insert("text_origin".into(), "native_subtitle".into());
                    cue_node.attrs.insert(
                        "segment_primary".into(),
                        serde_json::Value::Bool(self.transcription.reconciled.is_none()),
                    );
                    if let Some(start) = cue.timing.start_ms {
                        cue_node.attrs.insert("start_ms".into(), start.into());
                    }
                    if let Some(end) = cue.timing.end_ms {
                        cue_node.attrs.insert("end_ms".into(), end.into());
                    }
                    if let Some(locator) = &cue.timing.locator {
                        cue_node.attrs.insert(
                            "time_locator".into(),
                            serde_json::to_value(locator).map_err(error)?,
                        );
                    }
                    cue_node.extensions.insert(
                        "grist.subtitle".into(),
                        serde_json::to_value(cue).map_err(error)?,
                    );
                    graph.add_node(cue_node);
                    graph.add_contains(&node_id, &cue_id);
                }
            }
        }
        super::transcription_graph::project_transcription_representations(
            &mut graph, &ids, &root, self,
        )?;
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}
fn id(
    ids: &GraphIdGenerator,
    path: &[&str],
    native: &str,
    locator: Option<SourceLocator>,
) -> Result<String, TransformError> {
    let mut address = ProjectionAddress::native(path.iter().copied(), native);
    if let Some(locator) = locator {
        address = address.with_locator(locator)
    }
    ids.node_id(&address).map_err(error)
}
fn byte_locator(start: usize, end: usize) -> SourceLocator {
    SourceLocator::exact(crate::core::LocationComponent::ByteRange {
        byte_start: start,
        byte_end: end,
    })
    .expect("ordered range")
}
fn error(value: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: value.to_string(),
    }
}
