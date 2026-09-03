//! Explicit stream-scoped transcription and identity-bearing timed-text reconciliation.

use super::*;
use crate::core::{
    DeclaredLoss, Diagnostic, LossClass, OperationKind, ProvenanceStep, ProviderInvocation,
    ProviderKind, options_digest,
};
use crate::provider::{
    NativeRepresentation, ProviderRequest, ProviderResponse, ProviderResult,
    ReconciledRepresentation, TranscriptionRequest, TranscriptionResult,
};
use crate::registry::ParserContext;
use serde_json::json;
use std::collections::BTreeSet;

const RECONCILIATION_ALGORITHM: &str = "grist.media.native-transcription-append";
const RECONCILIATION_VERSION: &str = "1";
const MAX_TRANSCRIPT_SEGMENTS_PER_SCOPE: u64 = 100_000;
const MAX_TRANSCRIPT_CHARACTERS_PER_SCOPE: u64 = 16 * 1024 * 1024;

pub(crate) struct MediaTranscriptionOutcome {
    pub diagnostics: Vec<Diagnostic>,
    pub invocations: Vec<ProviderInvocation>,
    pub provenance: Vec<ProvenanceStep>,
}

pub(crate) fn native_transcription_content(
    subtitle_tracks: &[EmbeddedSubtitleTrack],
) -> Result<MediaTranscriptionContent, String> {
    let mut items = Vec::new();
    for track in subtitle_tracks {
        let Some(document) = &track.document else {
            continue;
        };
        for entry in &document.transcript.entries {
            items.push(MediaNativeTranscriptItem {
                index: items.len() as u64,
                subtitle_track_id: track.id.clone(),
                cue_id: entry.cue_id.clone(),
                start_ms: entry.start_ms,
                end_ms: entry.end_ms,
                speaker: entry.speaker.clone(),
                text: entry.text.clone(),
                locator: entry.time_locator.clone(),
            });
        }
    }
    let native = NativeRepresentation::new(MediaNativeTranscript {
        text: items
            .iter()
            .map(|item| item.text.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        items,
    })
    .map_err(|error| error.to_string())?;
    Ok(MediaTranscriptionContent {
        native,
        provider_attempts: Vec::new(),
        reconciled: None,
    })
}

pub(super) fn canonical_native_transcript(
    document: &MediaDocument,
) -> Result<NativeRepresentation<MediaNativeTranscript>, String> {
    let canonical = native_transcription_content(&document.subtitle_tracks)?.native;
    if document.transcription.native == canonical {
        return Ok(canonical);
    }
    let legacy_default = document.transcription.native.value == MediaNativeTranscript::default()
        && document.transcription.provider_attempts.is_empty()
        && document.transcription.reconciled.is_none();
    if legacy_default {
        return Ok(canonical);
    }
    Err("media native transcript facts diverge from subtitle_tracks".into())
}

pub(crate) fn apply_selected_transcription(
    context: &mut ParserContext<'_>,
    document: &mut MediaDocument,
    options: &MediaOptions,
) -> MediaTranscriptionOutcome {
    let mut outcome = MediaTranscriptionOutcome {
        diagnostics: Vec::new(),
        invocations: Vec::new(),
        provenance: Vec::new(),
    };
    if matches!(
        options.transcription.selection,
        MediaTranscriptionSelection::Disabled
    ) || context
        .selected_provider_network(ProviderKind::Transcription)
        .is_none()
    {
        return outcome;
    }
    if options.transcription.provider_options.track.is_some() {
        outcome.diagnostics.push(
            Diagnostic::warning(
                "grist.media",
                "media.transcription.track_host_controlled",
                "transcription provider_options.track must be omitted because the media host selects stable stream identities",
            )
            .partial(),
        );
        return outcome;
    }

    let selected_ids = match &options.transcription.selection {
        MediaTranscriptionSelection::SelectedAudioStreams { stream_ids } => {
            Some(stream_ids.iter().cloned().collect::<BTreeSet<_>>())
        }
        MediaTranscriptionSelection::AllAudioStreams => None,
        MediaTranscriptionSelection::Disabled => return outcome,
    };
    if selected_ids.as_ref().is_some_and(BTreeSet::is_empty) {
        outcome.diagnostics.push(
            Diagnostic::warning(
                "grist.media",
                "media.transcription.stream_selection_empty",
                "selected_audio_streams requires at least one stream identity",
            )
            .partial(),
        );
        return outcome;
    }

    let mut scopes = document
        .streams
        .iter()
        .filter(|stream| {
            selected_ids
                .as_ref()
                .is_none_or(|selected| selected.contains(&stream.id))
        })
        .filter(|stream| {
            stream.kind == MediaStreamKind::Audio
                && !stream.encrypted
                && stream.inspection != CodecInspectionStatus::Encrypted
        })
        .map(|stream| MediaTranscriptionScope {
            stream_id: stream.id.clone(),
            stream_index: stream.index,
            locator: stream.locator.clone(),
        })
        .collect::<Vec<_>>();

    if let Some(selected) = &selected_ids {
        for stream_id in selected {
            let eligible = scopes.iter().any(|scope| scope.stream_id == *stream_id);
            if !eligible {
                outcome.diagnostics.push(
                    Diagnostic::warning(
                        "grist.media",
                        "media.transcription.stream_ineligible",
                        format!(
                            "selected stream {stream_id} is absent, non-audio, or encrypted and was not sent to the provider"
                        ),
                    )
                    .partial(),
                );
            }
        }
    }
    if scopes.len() as u64 > options.transcription.max_streams {
        let omitted = scopes.len() as u64 - options.transcription.max_streams;
        scopes.truncate(usize::try_from(options.transcription.max_streams).unwrap_or(usize::MAX));
        outcome.diagnostics.push(
            Diagnostic::budget_exhausted(
                "grist.media",
                format!(
                    "media transcription scope limit {} omitted {omitted} stream request(s)",
                    options.transcription.max_streams
                ),
            )
            .partial(),
        );
    }

    for scope in scopes {
        let configuration = json!({
            "adapter": "grist.media.transcription-stream",
            "version": 1,
            "scope": scope,
        });
        let mut provider_options = options.transcription.provider_options.clone();
        provider_options.track = Some(scope.stream_id.clone());
        let response = match context.run_provider_for_input(
            ProviderKind::Transcription,
            &configuration,
            |request_context| {
                ProviderRequest::Transcription(TranscriptionRequest::new(
                    request_context,
                    provider_options,
                ))
            },
        ) {
            Ok(response) => response,
            Err(diagnostic) => {
                outcome
                    .diagnostics
                    .push((*diagnostic).with_locator(scope.locator.clone()).partial());
                continue;
            }
        };
        let response =
            match account_provider_output(context, &response, options.transcription.reconcile) {
                Ok(()) => response,
                Err(diagnostic) => rejected_response(
                    response,
                    diagnostic.with_locator(scope.locator.clone()).partial(),
                ),
            };
        outcome
            .invocations
            .push(response.metadata.envelope_invocation());
        outcome.diagnostics.extend(
            response
                .metadata
                .diagnostics
                .iter()
                .cloned()
                .map(|diagnostic| diagnostic.with_locator(scope.locator.clone()).partial()),
        );
        document
            .transcription
            .provider_attempts
            .push(build_attempt(scope, response));
    }

    if options.transcription.reconcile {
        match account_reconciliation(context, &document.transcription) {
            Ok(()) => {
                if let Some((reconciled, provenance)) =
                    reconcile(&document.transcription, &options.transcription)
                {
                    document.transcription.reconciled = Some(reconciled);
                    outcome.provenance.push(provenance);
                }
            }
            Err(diagnostic) => outcome.diagnostics.push(diagnostic.partial()),
        }
    }
    outcome
}

fn account_provider_output(
    context: &ParserContext<'_>,
    response: &ProviderResponse,
    reconcile: bool,
) -> Result<(), Diagnostic> {
    context
        .control()
        .checkpoint()
        .map_err(|error| error.diagnostic("grist.media"))?;
    let Some(ProviderResult::Transcription(result)) = response.result() else {
        return Ok(());
    };
    let segment_count = u64::try_from(result.segments.len()).unwrap_or(u64::MAX);
    let character_count = transcription_character_count(result);
    if segment_count > MAX_TRANSCRIPT_SEGMENTS_PER_SCOPE {
        return Err(Diagnostic::budget_exhausted(
            "grist.media",
            format!(
                "transcription provider returned {segment_count} segments; per-scope limit is {MAX_TRANSCRIPT_SEGMENTS_PER_SCOPE}"
            ),
        ));
    }
    if character_count > MAX_TRANSCRIPT_CHARACTERS_PER_SCOPE {
        return Err(Diagnostic::budget_exhausted(
            "grist.media",
            format!(
                "transcription provider returned {character_count} characters; per-scope limit is {MAX_TRANSCRIPT_CHARACTERS_PER_SCOPE}"
            ),
        ));
    }
    let representation_factor = if reconcile { 3 } else { 2 };
    let memory_bytes = transcription_string_bytes(result)
        .saturating_mul(representation_factor)
        .saturating_add(segment_count.saturating_mul(
            u64::try_from(std::mem::size_of::<MediaTranscriptSegment>()).unwrap_or(u64::MAX),
        ));
    context
        .observe_memory_bytes(memory_bytes)
        .map_err(|error| *error)?;
    context
        .consume_decoded_characters(character_count)
        .map_err(|error| *error)?;
    context
        .consume_nodes(segment_count.saturating_add(1))
        .map_err(|error| *error)?;
    context
        .consume_records(segment_count.saturating_add(1))
        .map_err(|error| *error)?;
    context
        .control()
        .checkpoint()
        .map_err(|error| error.diagnostic("grist.media"))
}

fn account_reconciliation(
    context: &ParserContext<'_>,
    content: &MediaTranscriptionContent,
) -> Result<(), Diagnostic> {
    context
        .control()
        .checkpoint()
        .map_err(|error| error.diagnostic("grist.media"))?;
    let successful = content
        .provider_attempts
        .iter()
        .filter(|attempt| attempt.response.is_success())
        .collect::<Vec<_>>();
    if successful.is_empty() {
        return Ok(());
    }
    let provider_items = successful
        .iter()
        .map(|attempt| {
            u64::try_from(attempt.segments.len()).unwrap_or(u64::MAX)
                + u64::from(reconciliation_overall_text(attempt).is_some())
        })
        .fold(0u64, u64::saturating_add);
    let native_items = u64::try_from(content.native.value.items.len()).unwrap_or(u64::MAX);
    let items = native_items.saturating_add(provider_items);
    let native_bytes = content
        .native
        .value
        .items
        .iter()
        .map(|item| u64::try_from(item.text.len()).unwrap_or(u64::MAX))
        .fold(0u64, u64::saturating_add);
    let provider_bytes = successful
        .iter()
        .map(|attempt| {
            attempt
                .segments
                .iter()
                .map(|segment| u64::try_from(segment.text.len()).unwrap_or(u64::MAX))
                .fold(0u64, u64::saturating_add)
                .saturating_add(
                    reconciliation_overall_text(attempt)
                        .map(|text| u64::try_from(text.len()).unwrap_or(u64::MAX))
                        .unwrap_or_default(),
                )
        })
        .fold(0u64, u64::saturating_add);
    let speaker_bytes = content
        .native
        .value
        .items
        .iter()
        .filter_map(|item| item.speaker.as_ref())
        .chain(
            successful
                .iter()
                .flat_map(|attempt| &attempt.segments)
                .filter_map(|segment| segment.speaker.as_ref()),
        )
        .map(|speaker| u64::try_from(speaker.len()).unwrap_or(u64::MAX))
        .fold(0u64, u64::saturating_add);
    // Reconciliation clones selected text/speaker/locator data, creates the joined
    // canonical text, and canonicalizes the value once more to derive its identity.
    // Charge those retained and peak allocations on top of the already observed
    // native/provider representation instead of treating them as a replacement.
    let additional_memory = native_bytes
        .saturating_add(provider_bytes)
        .saturating_mul(3)
        .saturating_add(speaker_bytes.saturating_mul(2))
        .saturating_add(
            items.saturating_mul(
                u64::try_from(std::mem::size_of::<MediaReconciledTranscriptItem>())
                    .unwrap_or(u64::MAX)
                    .saturating_add(512),
            ),
        )
        .saturating_add(
            u64::try_from(successful.len())
                .unwrap_or(u64::MAX)
                .saturating_mul(512),
        );
    let retained_memory = context.control().budget().snapshot().memory_bytes;
    context
        .observe_memory_bytes(retained_memory.saturating_add(additional_memory))
        .map_err(|error| *error)?;
    context.consume_nodes(items).map_err(|error| *error)?;
    context.consume_records(items).map_err(|error| *error)?;
    context
        .control()
        .checkpoint()
        .map_err(|error| error.diagnostic("grist.media"))
}

fn transcription_character_count(result: &TranscriptionResult) -> u64 {
    std::iter::once(result.text.chars().count())
        .chain(
            result
                .segments
                .iter()
                .map(|segment| segment.text.chars().count()),
        )
        .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
        .fold(0u64, u64::saturating_add)
}

fn transcription_string_bytes(result: &TranscriptionResult) -> u64 {
    std::iter::once(result.text.len())
        .chain(result.segments.iter().map(|segment| segment.text.len()))
        .chain(
            result
                .segments
                .iter()
                .filter_map(|segment| segment.speaker.as_ref().map(String::len)),
        )
        .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
        .fold(0u64, u64::saturating_add)
}

fn rejected_response(response: ProviderResponse, diagnostic: Diagnostic) -> ProviderResponse {
    let mut metadata = response.metadata;
    metadata.output_identity = None;
    metadata.diagnostics.push(diagnostic);
    ProviderResponse::failed(metadata).expect("bounded transcription rejection metadata is valid")
}

fn build_attempt(
    scope: MediaTranscriptionScope,
    response: ProviderResponse,
) -> MediaTranscriptionAttempt {
    let segments = match response.result() {
        Some(ProviderResult::Transcription(result)) => result
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| MediaTranscriptSegment {
                index: index as u64,
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                text: segment.text.clone(),
                speaker: segment.speaker.clone(),
                confidence: segment.confidence,
                locator: super::parse::synthesized_media_locator(
                    segment.start_ms,
                    segment.end_ms,
                    Some(scope.stream_index),
                ),
            })
            .collect(),
        _ => Vec::new(),
    };
    MediaTranscriptionAttempt {
        scope,
        response,
        segments,
    }
}

pub(super) fn distinct_overall_text(attempt: &MediaTranscriptionAttempt) -> Option<&str> {
    let ProviderResult::Transcription(result) = attempt.response.result()? else {
        return None;
    };
    if result.text.trim().is_empty() {
        return None;
    }
    let projected = attempt
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (projected != result.text).then_some(result.text.as_str())
}

fn reconciliation_overall_text(attempt: &MediaTranscriptionAttempt) -> Option<&str> {
    if attempt
        .segments
        .iter()
        .any(|segment| !segment.text.trim().is_empty())
    {
        return None;
    }
    let ProviderResult::Transcription(result) = attempt.response.result()? else {
        return None;
    };
    (!result.text.trim().is_empty()).then_some(result.text.as_str())
}

fn reconcile(
    content: &MediaTranscriptionContent,
    options: &MediaTranscriptionOptions,
) -> Option<(
    ReconciledRepresentation<MediaReconciledTranscript>,
    ProvenanceStep,
)> {
    let successful = content
        .provider_attempts
        .iter()
        .enumerate()
        .filter(|(_, attempt)| attempt.response.is_success())
        .collect::<Vec<_>>();
    if successful.is_empty() {
        return None;
    }
    let mut items = content
        .native
        .value
        .items
        .iter()
        .map(|item| MediaReconciledTranscriptItem {
            index: item.index,
            text: item.text.clone(),
            origin: MediaTranscriptOrigin::NativeSubtitle,
            source: MediaReconciledTranscriptSource::NativeSubtitle {
                subtitle_track_id: item.subtitle_track_id.clone(),
                cue_id: item.cue_id.clone(),
            },
            start_ms: Some(item.start_ms),
            end_ms: Some(item.end_ms),
            speaker: item.speaker.clone(),
            confidence: None,
            locator: item.locator.clone(),
        })
        .collect::<Vec<_>>();
    let mut confidence_evidence = Vec::new();
    let mut structure_flattened = false;
    for (attempt_index, attempt) in successful {
        let result_confidence = attempt.response.result().and_then(|result| match result {
            ProviderResult::Transcription(result) => result.confidence,
            _ => None,
        });
        confidence_evidence.push(format!(
            "transcription attempt {attempt_index} provider={} model={} confidence={}",
            attempt.response.metadata.provider.name,
            attempt
                .response
                .metadata
                .provider
                .model_version
                .as_deref()
                .unwrap_or("unspecified"),
            result_confidence
                .map(crate::provider::ProviderConfidence::get)
                .map_or_else(|| "missing".into(), |value| value.to_string()),
        ));
        for segment in &attempt.segments {
            items.push(MediaReconciledTranscriptItem {
                index: items.len() as u64,
                text: segment.text.clone(),
                origin: MediaTranscriptOrigin::ProviderTranscription,
                source: MediaReconciledTranscriptSource::ProviderSegment {
                    attempt_index: attempt_index as u64,
                    segment_index: segment.index,
                },
                start_ms: Some(segment.start_ms),
                end_ms: Some(segment.end_ms),
                speaker: segment.speaker.clone(),
                confidence: segment.confidence.or(result_confidence),
                locator: segment.locator.clone(),
            });
        }
        if let Some(overall) = reconciliation_overall_text(attempt) {
            structure_flattened = true;
            items.push(MediaReconciledTranscriptItem {
                index: items.len() as u64,
                text: overall.to_string(),
                origin: MediaTranscriptOrigin::ProviderTranscription,
                source: MediaReconciledTranscriptSource::ProviderOverall {
                    attempt_index: attempt_index as u64,
                },
                start_ms: None,
                end_ms: None,
                speaker: None,
                confidence: result_confidence,
                locator: attempt.scope.locator.clone(),
            });
        }
    }
    let value = MediaReconciledTranscript {
        text: items
            .iter()
            .map(|item| item.text.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        items,
        confidence_evidence,
    };
    let provider_output_identities = content
        .provider_attempts
        .iter()
        .filter_map(|attempt| attempt.response.metadata.output_identity.clone())
        .collect::<Vec<_>>();
    let configuration_digest = options_digest(&json!({
        "algorithm": RECONCILIATION_ALGORITHM,
        "version": RECONCILIATION_VERSION,
        "reconcile": options.reconcile,
    }))
    .expect("media reconciliation options serialize");
    let representation = ReconciledRepresentation::new(
        value,
        RECONCILIATION_ALGORITHM,
        RECONCILIATION_VERSION,
        configuration_digest.clone(),
        content.native.identity.clone(),
        provider_output_identities,
    )
    .expect("successful media transcription identities satisfy reconciliation invariants");
    let provider = content
        .provider_attempts
        .iter()
        .find(|attempt| attempt.response.is_success())
        .map(|attempt| attempt.response.metadata.provider.name.clone())
        .expect("successful media transcription has a provider");
    let declared_loss = if structure_flattened {
        DeclaredLoss::Lossy(LossClass::from(LossClass::STRUCTURE_FLATTENED))
    } else {
        DeclaredLoss::Lossless
    };
    let provenance = ProvenanceStep::new(
        OperationKind::Parse,
        format!("{RECONCILIATION_ALGORITHM}@{RECONCILIATION_VERSION}"),
        content.native.identity.clone(),
        representation.identity.clone(),
        configuration_digest,
        declared_loss,
    )
    .expect("media transcription reconciliation provenance is valid")
    .with_provider(provider);
    Some((representation, provenance))
}
