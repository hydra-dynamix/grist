use super::{MediaDocument, MediaReconciledTranscriptSource};
use crate::core::LocatorConfidence;
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentNode, DocumentNodeKind, DocumentRelation,
    GraphIdGenerator, ProjectionAddress, TransformError,
};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn project_transcription_representations(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    root: &str,
    document: &MediaDocument,
) -> Result<(), TransformError> {
    let native = super::transcription::canonical_native_transcript(document).map_err(error)?;
    let reconciled = document.transcription.reconciled.as_ref();
    if let Some(reconciled) = reconciled {
        if reconciled.native_input_identity != native.identity {
            return Err(error(
                "media reconciliation native identity does not match subtitle_tracks",
            ));
        }
        let available = document
            .transcription
            .provider_attempts
            .iter()
            .filter_map(|attempt| attempt.response.metadata.output_identity.as_deref())
            .collect::<Vec<_>>();
        if reconciled
            .provider_output_identities
            .iter()
            .any(|identity| !available.contains(&identity.as_str()))
        {
            return Err(error(
                "media reconciliation references an unavailable transcription output",
            ));
        }
    }

    let mut native_ids = BTreeMap::new();
    for item in &native.value.items {
        let id = ids
            .node_id(&ProjectionAddress::native(
                [
                    "media",
                    "subtitles",
                    item.subtitle_track_id.as_str(),
                    "cues",
                ],
                &item.cue_id,
            ))
            .map_err(error)?;
        native_ids.insert((item.subtitle_track_id.clone(), item.cue_id.clone()), id);
    }

    let provider_primary = reconciled.is_none() && native.value.items.is_empty();
    let mut provider_ids = BTreeMap::new();
    let mut overall_ids = BTreeMap::new();
    for (attempt_index, attempt) in document.transcription.provider_attempts.iter().enumerate() {
        let provider_identity = attempt
            .response
            .metadata
            .output_identity
            .as_deref()
            .unwrap_or(&attempt.response.metadata.request_digest);
        let parent = stream_node_id(ids, document, &attempt.scope.stream_id)?
            .unwrap_or_else(|| root.to_string());
        for segment in &attempt.segments {
            let id = ids
                .node_id(
                    &ProjectionAddress::native(
                        [
                            "media",
                            "transcription",
                            &attempt_index.to_string(),
                            "segments",
                            &segment.index.to_string(),
                        ],
                        &format!(
                            "transcription:{provider_identity}:{}:{}",
                            attempt.scope.stream_id, segment.index
                        ),
                    )
                    .with_locator(segment.locator.clone()),
                )
                .map_err(error)?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Cue)
                .with_text(&segment.text)
                .with_locator(segment.locator.clone())
                .with_ordinal(graph.nodes.len());
            node.attrs
                .insert("text_origin".into(), "provider_transcription".into());
            node.attrs
                .insert("segment_primary".into(), Value::Bool(provider_primary));
            node.attrs
                .insert("start_ms".into(), segment.start_ms.into());
            node.attrs.insert("end_ms".into(), segment.end_ms.into());
            node.attrs.insert(
                "time_locator".into(),
                serde_json::to_value(&segment.locator).map_err(error)?,
            );
            node.extensions.insert(
                "grist.media.transcription".into(),
                serde_json::json!({
                    "origin": "provider_transcription",
                    "scope": attempt.scope,
                    "segment": segment,
                    "provider": attempt.response.metadata.provider,
                    "request_digest": attempt.response.metadata.request_digest,
                    "configuration_digest": attempt.response.metadata.configuration_digest,
                    "output_identity": attempt.response.metadata.output_identity,
                }),
            );
            graph.add_node(node);
            graph.add_contains(&parent, &id);
            provider_ids.insert((attempt_index as u64, segment.index), id);
        }
        if let Some(overall) = super::transcription::distinct_overall_text(attempt) {
            let id = ids
                .node_id(
                    &ProjectionAddress::native(
                        [
                            "media",
                            "transcription",
                            &attempt_index.to_string(),
                            "overall",
                        ],
                        &format!(
                            "transcription-overall:{provider_identity}:{}",
                            attempt.scope.stream_id
                        ),
                    )
                    .with_locator(attempt.scope.locator.clone()),
                )
                .map_err(error)?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Transcript)
                .with_text(overall)
                .with_locator(attempt.scope.locator.clone())
                .with_ordinal(graph.nodes.len());
            node.attrs.insert(
                "text_origin".into(),
                "provider_transcription_overall".into(),
            );
            node.attrs
                .insert("segment_primary".into(), Value::Bool(false));
            node.extensions.insert(
                "grist.media.transcription".into(),
                serde_json::json!({
                    "origin": "provider_transcription_overall",
                    "scope": attempt.scope,
                    "provider": attempt.response.metadata.provider,
                    "request_digest": attempt.response.metadata.request_digest,
                    "configuration_digest": attempt.response.metadata.configuration_digest,
                    "output_identity": attempt.response.metadata.output_identity,
                }),
            );
            graph.add_node(node);
            graph.add_contains(&parent, &id);
            overall_ids.insert(attempt_index as u64, id);
        }
    }

    if let Some(reconciled) = reconciled {
        for item in &reconciled.value.items {
            let id = ids
                .node_id(
                    &ProjectionAddress::native(
                        [
                            "media",
                            "transcription",
                            "reconciled",
                            &item.index.to_string(),
                        ],
                        &format!("reconciled:{}:{}", reconciled.identity, item.index),
                    )
                    .with_locator(item.locator.clone()),
                )
                .map_err(error)?;
            let kind = if item.start_ms.is_some() {
                DocumentNodeKind::Cue
            } else {
                DocumentNodeKind::Transcript
            };
            let mut node = DocumentNode::new(&id, kind)
                .with_text(&item.text)
                .with_locator(item.locator.clone())
                .with_ordinal(graph.nodes.len());
            node.attrs
                .insert("text_origin".into(), "reconciled_transcript".into());
            node.attrs
                .insert("segment_primary".into(), Value::Bool(true));
            if let Some(start_ms) = item.start_ms {
                node.attrs.insert("start_ms".into(), start_ms.into());
            }
            if let Some(end_ms) = item.end_ms {
                node.attrs.insert("end_ms".into(), end_ms.into());
            }
            if item.start_ms.is_some() {
                node.attrs.insert(
                    "time_locator".into(),
                    serde_json::to_value(&item.locator).map_err(error)?,
                );
            }
            node.extensions.insert(
                "grist.media.transcription".into(),
                serde_json::json!({
                    "origin": "reconciled_transcript",
                    "item": item,
                    "algorithm": reconciled.algorithm,
                    "algorithm_version": reconciled.algorithm_version,
                    "configuration_digest": reconciled.configuration_digest,
                    "confidence_evidence": reconciled.value.confidence_evidence,
                }),
            );
            graph.add_node(node);
            graph.add_contains(root, &id);
            let source_id = match &item.source {
                MediaReconciledTranscriptSource::NativeSubtitle {
                    subtitle_track_id,
                    cue_id,
                } => native_ids.get(&(subtitle_track_id.clone(), cue_id.clone())),
                MediaReconciledTranscriptSource::ProviderSegment {
                    attempt_index,
                    segment_index,
                } => provider_ids.get(&(*attempt_index, *segment_index)),
                MediaReconciledTranscriptSource::ProviderOverall { attempt_index } => {
                    overall_ids.get(attempt_index)
                }
            };
            if let Some(source_id) = source_id {
                graph.add_edge(
                    DocumentEdge::inferred(
                        source_id.clone(),
                        DocumentRelation::ReconciledWith,
                        id,
                        "grist.media.native-transcription-append.v1",
                        LocatorConfidence::new(item.confidence.map_or(1.0, |value| value.get()))
                            .map_err(error)?,
                    )
                    .with_inference_evidence(item.locator.clone()),
                );
            }
        }
    }
    Ok(())
}

fn stream_node_id(
    ids: &GraphIdGenerator,
    document: &MediaDocument,
    stream_id: &str,
) -> Result<Option<String>, TransformError> {
    document
        .streams
        .iter()
        .find(|stream| stream.id == stream_id)
        .map(|stream| {
            ids.node_id(&ProjectionAddress::native(["media", "streams"], &stream.id))
                .map_err(error)
        })
        .transpose()
}

fn error(value: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: value.to_string(),
    }
}
