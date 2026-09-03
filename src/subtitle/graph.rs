use super::{SubtitleDocument, SubtitleFormat};
use crate::core::{SchemaVersion, SourceLocator};
use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    GraphIdGenerator, ProjectionAddress, ToDocumentGraph, TransformError,
};

impl ToDocumentGraph for SubtitleDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::SUBTITLE_V1, "grist.subtitle")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Subtitle)
            .with_projection(
                "subtitle",
                SchemaVersion::SUBTITLE_V1,
                "grist.subtitle.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = context
            .language
            .or_else(|| self.tracks.iter().find_map(|track| track.language.clone()));
        graph.dialect = Some(self.format.as_str().into());
        graph.attrs = context.attrs;
        graph
            .attrs
            .insert("format".into(), self.format.as_str().into());
        graph.attrs.insert("complete".into(), self.complete.into());

        let root = node_id(
            &ids,
            &["transcript"],
            self.format.as_str(),
            Some(self.locator.clone()),
        )?;
        let mut root_node = DocumentNode::new(&root, DocumentNodeKind::Transcript)
            .with_name(match self.format {
                SubtitleFormat::Srt => "SRT transcript",
                SubtitleFormat::WebVtt => "WebVTT transcript",
                SubtitleFormat::Ttml => "TTML transcript",
            })
            .with_locator(self.locator.clone())
            .with_ordinal(0);
        root_node.extensions.insert(
            "grist.subtitle".into(),
            serde_json::json!({
                "format": self.format, "metadata": self.metadata, "regions": self.regions,
                "styles": self.styles,
                "transcript": self.transcript,
            }),
        );
        graph.add_node(root_node);

        let cue_index = self
            .cues
            .iter()
            .map(|cue| {
                (
                    (cue.track_id.as_str(), cue.source_index, cue.id.as_str()),
                    cue,
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        for (track_index, track) in self.tracks.iter().enumerate() {
            let track_id = node_id(
                &ids,
                &["transcript", "tracks", &track_index.to_string()],
                &track.id,
                Some(track.locator.clone()),
            )?;
            let mut track_node = DocumentNode::new(&track_id, DocumentNodeKind::MediaTrack)
                .with_name(track.label.as_deref().unwrap_or(&track.id))
                .with_locator(track.locator.clone())
                .with_ordinal(track_index);
            track_node
                .attrs
                .insert("track_id".into(), track.id.clone().into());
            if let Some(language) = &track.language {
                track_node
                    .attrs
                    .insert("language".into(), language.clone().into());
            }
            track_node.extensions.insert(
                "grist.subtitle".into(),
                serde_json::to_value(track).map_err(error)?,
            );
            graph.add_node(track_node);
            graph.add_contains(&root, &track_id);

            let mut emitted = std::collections::BTreeSet::<(usize, String)>::new();
            let mut ordered_cues = Vec::new();
            for entry in self
                .transcript
                .entries
                .iter()
                .filter(|entry| entry.track_id == track.id)
            {
                if let Some(cue) = cue_index.get(&(
                    entry.track_id.as_str(),
                    entry.source_index,
                    entry.cue_id.as_str(),
                )) && emitted.insert((cue.source_index, cue.id.clone()))
                {
                    ordered_cues.push(*cue);
                }
            }
            for cue in self.cues.iter().filter(|cue| cue.track_id == track.id) {
                if emitted.insert((cue.source_index, cue.id.clone())) {
                    ordered_cues.push(cue);
                }
            }

            for (time_ordinal, cue) in ordered_cues.into_iter().enumerate() {
                let cue_id = node_id(
                    &ids,
                    &[
                        "transcript",
                        "tracks",
                        &track_index.to_string(),
                        "cues",
                        &cue.source_index.to_string(),
                    ],
                    &cue.id,
                    Some(cue.text_locator.clone()),
                )?;
                let mut cue_node = DocumentNode::new(&cue_id, DocumentNodeKind::Cue)
                    .with_text(&cue.text)
                    .with_locator(cue.text_locator.clone())
                    .with_ordinal(time_ordinal);
                cue_node
                    .attrs
                    .insert("cue_id".into(), cue.id.clone().into());
                cue_node
                    .attrs
                    .insert("track_id".into(), cue.track_id.clone().into());
                if let Some(start) = cue.timing.start_ms {
                    cue_node.attrs.insert("start_ms".into(), start.into());
                }
                if let Some(end) = cue.timing.end_ms {
                    cue_node.attrs.insert("end_ms".into(), end.into());
                }
                if let Some(speaker) = &cue.speaker {
                    cue_node
                        .attrs
                        .insert("speaker".into(), speaker.clone().into());
                }
                if let Some(region_id) = &cue.region_id {
                    cue_node
                        .attrs
                        .insert("region_id".into(), region_id.clone().into());
                }
                cue_node.extensions.insert(
                    "grist.subtitle".into(),
                    serde_json::to_value(cue).map_err(error)?,
                );
                graph.add_node(cue_node);
                graph.add_contains(&track_id, &cue_id);
            }
        }
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn node_id(
    ids: &GraphIdGenerator,
    path: &[&str],
    native: &str,
    locator: Option<SourceLocator>,
) -> Result<String, TransformError> {
    let mut address = ProjectionAddress::native(path.iter().copied(), native);
    if let Some(locator) = locator {
        address = address.with_locator(locator);
    }
    ids.node_id(&address).map_err(error)
}
fn error(value: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: value.to_string(),
    }
}
