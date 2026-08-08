use super::{ImageDocument, ImageReconciledTextSource};
use crate::core::LocatorConfidence;
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentNode, DocumentNodeKind, DocumentRelation,
    GraphIdGenerator, ProjectionAddress, TransformError,
};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn project_text_representations(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    root: &str,
    document: &ImageDocument,
) -> Result<(), TransformError> {
    let native = super::ocr::canonical_native_text(document).map_err(error)?;
    let reconciled = document.text.reconciled.as_ref();
    if let Some(reconciled) = reconciled {
        if reconciled.native_input_identity != native.identity {
            return Err(error(
                "image reconciliation native identity does not match embedded_text",
            ));
        }
        let available = document
            .text
            .ocr_attempts
            .iter()
            .filter_map(|attempt| attempt.response.metadata.output_identity.as_deref())
            .collect::<Vec<_>>();
        if reconciled
            .provider_output_identities
            .iter()
            .any(|identity| !available.contains(&identity.as_str()))
        {
            return Err(error(
                "image reconciliation references an unavailable OCR output",
            ));
        }
    }
    let mut native_ids = BTreeMap::new();
    for (index, entry) in native.value.entries.iter().enumerate() {
        let id = ids
            .node_id(
                &ProjectionAddress::native(
                    ["image", "text", &index.to_string()],
                    &format!("{:?}-{index}", entry.kind),
                )
                .with_locator(entry.locator.clone()),
            )
            .map_err(error)?;
        if let Some(node) = graph.nodes.iter_mut().find(|node| node.id == id) {
            node.attrs.insert("text_origin".into(), "native".into());
            node.attrs
                .insert("segment_primary".into(), Value::Bool(reconciled.is_none()));
        }
        native_ids.insert(index as u64, id);
    }

    let mut ocr_ids = BTreeMap::new();
    let mut overall_ids = BTreeMap::new();
    for (attempt_index, attempt) in document.text.ocr_attempts.iter().enumerate() {
        let provider_identity = attempt
            .response
            .metadata
            .output_identity
            .as_deref()
            .unwrap_or(&attempt.response.metadata.request_digest);
        for region in &attempt.regions {
            let id = ids
                .node_id(
                    &ProjectionAddress::native(
                        [
                            "image",
                            "text",
                            "ocr",
                            &attempt_index.to_string(),
                            &region.index.to_string(),
                        ],
                        &format!(
                            "ocr:{provider_identity}:{}:{attempt_index}:{}",
                            attempt.scope.frame_index, region.index
                        ),
                    )
                    .with_locator(region.locator.clone()),
                )
                .map_err(error)?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::TextRun)
                .with_text(&region.text)
                .with_locator(region.locator.clone())
                .with_ordinal(graph.nodes.len());
            node.attrs.insert("text_origin".into(), "ocr".into());
            node.attrs
                .insert("segment_primary".into(), Value::Bool(false));
            node.extensions.insert(
                "grist.image".into(),
                serde_json::json!({
                    "origin": "ocr",
                    "scope": attempt.scope,
                    "region": region,
                    "provider": attempt.response.metadata.provider,
                    "request_digest": attempt.response.metadata.request_digest,
                    "output_identity": attempt.response.metadata.output_identity,
                    "reading_order_confidence": attempt.reading_order_confidence,
                    "layout_confidence": attempt.layout_confidence,
                }),
            );
            graph.add_node(node);
            let parent = frame_node_id(ids, document, attempt.scope.frame_index)?
                .unwrap_or_else(|| root.to_string());
            graph.add_contains(parent, &id);
            ocr_ids.insert((attempt_index as u64, region.index), id);
        }
        if let Some(overall_text) = super::ocr::distinct_overall_text(attempt) {
            let id = ids
                .node_id(
                    &ProjectionAddress::native(
                        [
                            "image",
                            "text",
                            "ocr",
                            &attempt_index.to_string(),
                            "overall",
                        ],
                        &format!(
                            "ocr-overall:{provider_identity}:{}:{attempt_index}",
                            attempt.scope.frame_index
                        ),
                    )
                    .with_locator(attempt.scope.locator.clone()),
                )
                .map_err(error)?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::TextRun)
                .with_text(overall_text)
                .with_locator(attempt.scope.locator.clone())
                .with_ordinal(graph.nodes.len());
            node.attrs
                .insert("text_origin".into(), "ocr_overall".into());
            node.attrs
                .insert("segment_primary".into(), Value::Bool(false));
            node.extensions.insert(
                "grist.image".into(),
                serde_json::json!({
                    "origin": "ocr_overall",
                    "scope": attempt.scope,
                    "provider": attempt.response.metadata.provider,
                    "request_digest": attempt.response.metadata.request_digest,
                    "output_identity": attempt.response.metadata.output_identity,
                }),
            );
            graph.add_node(node);
            let parent = frame_node_id(ids, document, attempt.scope.frame_index)?
                .unwrap_or_else(|| root.to_string());
            graph.add_contains(parent, &id);
            overall_ids.insert(attempt_index as u64, id);
        }
    }

    if let Some(reconciled) = reconciled {
        for item in &reconciled.value.items {
            let id = ids
                .node_id(
                    &ProjectionAddress::native(
                        ["image", "text", "reconciled", &item.index.to_string()],
                        &format!("reconciled:{}:{}", reconciled.identity, item.index),
                    )
                    .with_locator(item.locator.clone()),
                )
                .map_err(error)?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Paragraph)
                .with_text(&item.text)
                .with_locator(item.locator.clone())
                .with_ordinal(graph.nodes.len());
            node.attrs.insert("text_origin".into(), "reconciled".into());
            node.attrs
                .insert("segment_primary".into(), Value::Bool(true));
            node.extensions.insert(
                "grist.image".into(),
                serde_json::json!({
                    "origin": "reconciled",
                    "item": item,
                    "algorithm": reconciled.algorithm,
                    "algorithm_version": reconciled.algorithm_version,
                    "configuration_digest": reconciled.configuration_digest,
                    "confidence_evidence": reconciled.value.confidence_evidence,
                }),
            );
            graph.add_node(node);
            graph.add_contains(root, &id);
            let source_id = match item.source {
                ImageReconciledTextSource::Native { entry_index } => native_ids.get(&entry_index),
                ImageReconciledTextSource::OcrOverall { attempt_index } => {
                    overall_ids.get(&attempt_index)
                }
                ImageReconciledTextSource::Ocr {
                    attempt_index,
                    region_index,
                } => ocr_ids.get(&(attempt_index, region_index)),
            };
            if let Some(source_id) = source_id {
                graph.add_edge(
                    DocumentEdge::inferred(
                        source_id.clone(),
                        DocumentRelation::ReconciledWith,
                        id,
                        "grist.image.native-ocr-append.v1",
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

fn frame_node_id(
    ids: &GraphIdGenerator,
    document: &ImageDocument,
    frame_index: u64,
) -> Result<Option<String>, TransformError> {
    document
        .frames
        .iter()
        .find(|frame| frame.index == frame_index)
        .map(|frame| {
            ids.node_id(
                &ProjectionAddress::native(
                    ["image", "frames", &frame.index.to_string()],
                    &frame.index.to_string(),
                )
                .with_locator(frame.locator.clone()),
            )
            .map_err(error)
        })
        .transpose()
}

fn error(value: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: value.to_string(),
    }
}
