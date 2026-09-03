#![cfg(all(feature = "media", feature = "document-graph"))]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, LocationComponent, NetworkAccess, OperationStatus,
    ParseRequest, Provider, ProviderKind, ProviderSet, RequestId, ResourceBudget, SourceInfo,
    canonical_json_bytes,
};
use grist::document_graph::{DocumentGraphContext, DocumentRelation, ToDocumentGraph};
use grist::media::{
    MediaDocument, MediaFormat, MediaOptions, MediaTranscriptOrigin, MediaTranscriptionSelection,
    parse_media_bytes,
};
use grist::provider::{
    ProviderConfidence, ProviderDeterminism, ProviderError, ProviderMetadata,
    ProviderRequestManifest, ProviderResult, RecordedProvider, TranscriptSegment,
    TranscriptionProvider, TranscriptionProviderAdapter, TranscriptionRequest, TranscriptionResult,
};
use grist::registry::{Capability, ParserSelection, builtin_parser_registry};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn wav() -> Vec<u8> {
    let mut format = Vec::new();
    format.extend_from_slice(&1u16.to_le_bytes());
    format.extend_from_slice(&1u16.to_le_bytes());
    format.extend_from_slice(&16_000u32.to_le_bytes());
    format.extend_from_slice(&32_000u32.to_le_bytes());
    format.extend_from_slice(&2u16.to_le_bytes());
    format.extend_from_slice(&16u16.to_le_bytes());
    let chunks = [riff_chunk(b"fmt ", &format), riff_chunk(b"data", &[0; 64])].concat();
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend(chunks);
    out
}

fn riff_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = kind.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn elem(id: &[u8], data: &[u8]) -> Vec<u8> {
    assert!(data.len() < 16_383);
    let mut out = id.to_vec();
    if data.len() < 127 {
        out.push(0x80 | data.len() as u8);
    } else {
        out.push(0x40 | ((data.len() >> 8) as u8));
        out.push(data.len() as u8);
    }
    out.extend_from_slice(data);
    out
}

fn track(number: u8, uid: u8, kind: u8, codec: &str) -> Vec<u8> {
    let mut data = elem(&[0xd7], &[number]);
    data.extend(elem(&[0x73, 0xc5], &[uid]));
    data.extend(elem(&[0x83], &[kind]));
    data.extend(elem(&[0x86], codec.as_bytes()));
    elem(&[0xae], &data)
}

fn matroska_with_native_subtitle_and_audio() -> Vec<u8> {
    let info = elem(
        &[0x15, 0x49, 0xa9, 0x66],
        &elem(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]),
    );
    let tracks = elem(
        &[0x16, 0x54, 0xae, 0x6b],
        &[
            track(1, 42, 0x11, "S_TEXT/UTF8"),
            track(2, 99, 2, "A_PCM/INT/LIT"),
        ]
        .concat(),
    );
    let mut cluster = elem(&[0xe7], &[0]);
    let mut block = vec![0x81, 0, 0, 0];
    block.extend_from_slice(b"Native caption");
    cluster.extend(elem(&[0xa3], &block));
    let segment = [info, tracks, elem(&[0x1f, 0x43, 0xb6, 0x75], &cluster)].concat();
    [
        elem(&[0x1a, 0x45, 0xdf, 0xa3], &elem(&[0x42, 0x82], b"matroska")),
        elem(&[0x18, 0x53, 0x80, 0x67], &segment),
    ]
    .concat()
}

fn confidence(value: f64) -> ProviderConfidence {
    ProviderConfidence::new(value).unwrap()
}

fn transcript_result() -> TranscriptionResult {
    TranscriptionResult {
        // Deliberately differs from the timed segment join. The overall result is
        // retained as a non-primary provider representation, but reconciliation
        // must select the timed form once rather than duplicate the transcript.
        text: "Hello, world.".into(),
        segments: vec![
            TranscriptSegment {
                start_ms: 125,
                end_ms: 875,
                text: "Hello".into(),
                speaker: Some("speaker-1".into()),
                confidence: Some(confidence(0.96)),
            },
            TranscriptSegment {
                start_ms: 900,
                end_ms: 1_500,
                text: "world".into(),
                speaker: Some("speaker-2".into()),
                confidence: Some(confidence(0.91)),
            },
        ],
        language: Some("en".into()),
        confidence: Some(confidence(0.94)),
        diagnostics: Vec::new(),
    }
}

fn metadata(name: &str, determinism: ProviderDeterminism) -> ProviderMetadata {
    ProviderMetadata::new(name, "fixture-media-transcription", "1", determinism)
        .unwrap()
        .with_model_version("speech-v2")
        .with_confidence_model("fixture-normalized-v1")
}

#[derive(Clone)]
struct FixedTranscription {
    manifests: Option<Arc<Mutex<Vec<ProviderRequestManifest>>>>,
}

impl TranscriptionProvider for FixedTranscription {
    fn transcribe(
        &self,
        request: &TranscriptionRequest<'_>,
    ) -> Result<TranscriptionResult, ProviderError> {
        assert_eq!(request.context.network_access(), NetworkAccess::Denied);
        assert_eq!(request.options.track.as_deref(), Some("stream:audio:0"));
        assert!(request.options.word_timestamps);
        if let Some(manifests) = &self.manifests {
            manifests.lock().unwrap().push(request.manifest().unwrap());
        }
        Ok(transcript_result())
    }
}

struct FailingTranscription;

impl TranscriptionProvider for FailingTranscription {
    fn transcribe(
        &self,
        _request: &TranscriptionRequest<'_>,
    ) -> Result<TranscriptionResult, ProviderError> {
        Err(ProviderError::failure(
            "fixture-media-transcription",
            "transcription backend unavailable",
        ))
    }
}

fn provider(name: &str, implementation: impl TranscriptionProvider) -> Arc<dyn Provider> {
    Arc::new(
        TranscriptionProviderAdapter::new(
            metadata(name, ProviderDeterminism::Guaranteed),
            implementation,
        )
        .unwrap(),
    )
}

fn selected_options(stream_id: &str) -> MediaOptions {
    let mut options = MediaOptions::default();
    options.transcription.selection = MediaTranscriptionSelection::SelectedAudioStreams {
        stream_ids: vec![stream_id.into()],
    };
    options.transcription.provider_options.language_hints = vec!["en".into()];
    options.transcription.provider_options.speaker_diarization = true;
    options.transcription.provider_options.word_timestamps = true;
    options.transcription.reconcile = true;
    options
}

fn parse_registered(
    bytes: Vec<u8>,
    format: &str,
    source: SourceInfo,
    selected: Option<Arc<dyn Provider>>,
    options: MediaOptions,
) -> grist::core::Envelope<serde_json::Value> {
    parse_registered_with_budget(
        bytes,
        format,
        source,
        selected,
        options,
        ResourceBudget::trusted_unbounded(),
    )
}

fn parse_registered_with_budget(
    bytes: Vec<u8>,
    format: &str,
    source: SourceInfo,
    selected: Option<Arc<dyn Provider>>,
    options: MediaOptions,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let mut providers = ProviderSet::none();
    if let Some(provider) = selected {
        providers.select(ProviderKind::Transcription, provider, NetworkAccess::Denied);
    }
    let request = ParseRequest::new(
        RequestId::new("media-transcription-test").unwrap(),
        Input::bytes(bytes),
        source,
        BudgetSelection::custom(budget),
        providers,
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch(
            format,
            request,
            Some(serde_json::to_value(options).unwrap()),
        )
        .unwrap()
}

#[test]
fn explicit_stream_transcription_preserves_timing_confidence_identity_and_projections() {
    let bytes = wav();
    let options = selected_options("stream:audio:0");
    let envelope = parse_registered(
        bytes.clone(),
        "wav",
        SourceInfo::stdin("speech.wav").with_declared_mime_type("audio/wav"),
        Some(provider(
            "fixture-media-transcription",
            FixedTranscription { manifests: None },
        )),
        options.clone(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    assert_eq!(envelope.providers.len(), 1);
    assert_eq!(
        envelope.providers[0].provider,
        "fixture-media-transcription"
    );
    assert_eq!(
        envelope.providers[0].model_version.as_deref(),
        Some("speech-v2")
    );
    assert!(envelope.provenance.iter().any(|step| {
        step.implementation == "grist.media.native-transcription-append@1"
            && step.provider.as_deref() == Some("fixture-media-transcription")
    }));

    let payload = envelope.payload.unwrap();
    #[cfg(feature = "schemas")]
    {
        assert!(
            grist::schema::validate_schema("media", &payload)
                .unwrap()
                .valid
        );
        assert!(
            grist::schema::validate_schema(
                "media-options",
                &serde_json::to_value(&options).unwrap(),
            )
            .unwrap()
            .valid
        );
    }
    let document: MediaDocument = serde_json::from_value(payload).unwrap();
    assert!(document.subtitle_tracks.is_empty());
    assert!(document.transcription.native.value.items.is_empty());
    assert_eq!(document.transcription.provider_attempts.len(), 1);
    let attempt = &document.transcription.provider_attempts[0];
    assert_eq!(attempt.scope.stream_id, "stream:audio:0");
    assert_eq!(
        attempt.response.metadata.provider.name,
        "fixture-media-transcription"
    );
    assert!(
        attempt
            .response
            .metadata
            .configuration_digest
            .starts_with("sha256:")
    );
    assert_eq!(attempt.segments.len(), 2);
    assert_eq!(attempt.segments[0].start_ms, 125);
    assert_eq!(attempt.segments[0].end_ms, 875);
    assert_eq!(attempt.segments[0].confidence, Some(confidence(0.96)));
    assert!(attempt.segments.iter().all(|segment| {
        segment.locator.components().iter().any(|component| {
            matches!(
                component,
                LocationComponent::MediaTime {
                    start_ms: _,
                    end_ms: _,
                    track: Some(_)
                }
            )
        })
    }));
    let reconciled = document.transcription.reconciled.as_ref().unwrap();
    assert_eq!(
        reconciled.algorithm,
        "grist.media.native-transcription-append"
    );
    assert_eq!(reconciled.algorithm_version, "1");
    assert_eq!(reconciled.value.items.len(), 2);
    assert_eq!(reconciled.value.text, "Hello\nworld");
    assert!(reconciled.value.items.iter().all(|item| {
        item.origin == MediaTranscriptOrigin::ProviderTranscription
            && item.start_ms.is_some()
            && item.confidence.is_some()
    }));

    let graph = document
        .to_document_graph(
            DocumentGraphContext::new("media-transcription")
                .with_source(SourceInfo::stdin("speech.wav")),
        )
        .unwrap();
    graph.validate_contract().unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.attrs
            .get("text_origin")
            .and_then(serde_json::Value::as_str)
            == Some("provider_transcription")
            && node.attrs.contains_key("time_locator")
    }));
    assert!(graph.nodes.iter().any(|node| {
        node.attrs
            .get("text_origin")
            .and_then(serde_json::Value::as_str)
            == Some("provider_transcription_overall")
            && node
                .attrs
                .get("segment_primary")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
    }));
    assert!(graph.nodes.iter().any(|node| {
        node.attrs
            .get("text_origin")
            .and_then(serde_json::Value::as_str)
            == Some("reconciled_transcript")
            && node.attrs.contains_key("time_locator")
    }));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ReconciledWith)
    );

    let source_identity = ContentIdentity::for_raw_bytes(&bytes);
    let document_identity = ContentIdentity::for_raw_bytes(&canonical_json_bytes(&graph).unwrap());
    let segments = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(
        segments
            .segments
            .iter()
            .map(|segment| segment.text.matches("Hello").count())
            .sum::<usize>(),
        1,
        "default segmentation must use the reconciled timed representation once"
    );
    assert!(
        segments
            .segments
            .iter()
            .flat_map(|segment| &segment.locators)
            .all(|locator| locator.validate().is_ok())
    );
}

#[test]
fn recorded_transcription_replay_is_deterministic_network_denied_and_timestamped() {
    let manifests = Arc::new(Mutex::new(Vec::new()));
    let options = selected_options("stream:audio:0");
    let captured = parse_registered(
        wav(),
        "wav",
        SourceInfo::stdin("capture.wav"),
        Some(provider(
            "recorded-media-transcription",
            FixedTranscription {
                manifests: Some(manifests.clone()),
            },
        )),
        options.clone(),
    );
    assert_eq!(captured.status, OperationStatus::Complete);
    let entries = || {
        manifests
            .lock()
            .unwrap()
            .iter()
            .map(|manifest| {
                assert_eq!(manifest.network_access, NetworkAccess::Denied);
                (
                    manifest.request_digest().unwrap(),
                    ProviderResult::Transcription(transcript_result()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let replay = || {
        Arc::new(
            RecordedProvider::new(
                ProviderKind::Transcription,
                metadata(
                    "recorded-media-transcription",
                    ProviderDeterminism::GuaranteedWithRecording,
                ),
                entries(),
            )
            .unwrap(),
        ) as Arc<dyn Provider>
    };
    let first = parse_registered(
        wav(),
        "wav",
        SourceInfo::stdin("replay.wav"),
        Some(replay()),
        options.clone(),
    );
    let second = parse_registered(
        wav(),
        "wav",
        SourceInfo::stdin("replay.wav"),
        Some(replay()),
        options,
    );
    assert_eq!(
        canonical_json_bytes(&first).unwrap(),
        canonical_json_bytes(&second).unwrap()
    );
    assert_eq!(first.providers[0].deterministic, Some(true));
    let document: MediaDocument = serde_json::from_value(first.payload.unwrap()).unwrap();
    let attempt = &document.transcription.provider_attempts[0];
    assert!(attempt.response.metadata.recorded);
    assert_eq!(
        attempt
            .segments
            .iter()
            .map(|segment| (segment.start_ms, segment.end_ms))
            .collect::<Vec<_>>(),
        vec![(125, 875), (900, 1_500)]
    );
}

#[test]
fn transcription_output_is_budgeted_before_projection_or_reconciliation() {
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_decoded_characters = Some(5);
    let envelope = parse_registered_with_budget(
        wav(),
        "wav",
        SourceInfo::stdin("budget.wav"),
        Some(provider(
            "fixture-media-transcription",
            FixedTranscription { manifests: None },
        )),
        selected_options("stream:audio:0"),
        budget,
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(envelope.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.decoded_characters.exhausted"
            && diagnostic.partial
    }));
    let document: MediaDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert_eq!(document.transcription.provider_attempts.len(), 1);
    assert!(
        !document.transcription.provider_attempts[0]
            .response
            .is_success()
    );
    assert!(
        document.transcription.provider_attempts[0]
            .segments
            .is_empty()
    );
    assert!(document.transcription.reconciled.is_none());
}

#[test]
fn reconciliation_peak_memory_is_budgeted_before_allocation() {
    let mut budget = ResourceBudget::trusted_unbounded();
    // The native parse and provider projection fit; the additional cloned,
    // joined, and identity-canonicalized reconciliation representation does not.
    budget.max_memory_bytes = Some(1_000);
    let envelope = parse_registered_with_budget(
        wav(),
        "wav",
        SourceInfo::stdin("memory-budget.wav"),
        Some(provider(
            "fixture-media-transcription",
            FixedTranscription { manifests: None },
        )),
        selected_options("stream:audio:0"),
        budget,
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(envelope.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.memory_bytes.exhausted" && diagnostic.partial
    }));
    let document: MediaDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert_eq!(document.transcription.provider_attempts.len(), 1);
    assert!(
        document.transcription.provider_attempts[0]
            .response
            .is_success()
    );
    assert!(document.transcription.reconciled.is_none());
}

#[test]
fn provider_failure_preserves_nonempty_native_subtitles_and_partial_output() {
    let bytes = matroska_with_native_subtitle_and_audio();
    let native = parse_registered(
        bytes.clone(),
        "matroska",
        SourceInfo::stdin("native.mkv"),
        None,
        MediaOptions::default(),
    );
    assert_eq!(native.status, OperationStatus::Complete);
    let native_document: MediaDocument = serde_json::from_value(native.payload.unwrap()).unwrap();
    assert_eq!(
        native_document.transcription.native.value.items[0].text,
        "Native caption"
    );
    assert!(native_document.transcription.provider_attempts.is_empty());

    let failed = parse_registered(
        bytes,
        "matroska",
        SourceInfo::stdin("failed.mkv"),
        Some(provider(
            "fixture-media-transcription",
            FailingTranscription,
        )),
        MediaOptions {
            transcription: grist::media::MediaTranscriptionOptions {
                selection: MediaTranscriptionSelection::AllAudioStreams,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(failed.status, OperationStatus::Partial);
    let failed_document: MediaDocument = serde_json::from_value(failed.payload.unwrap()).unwrap();
    assert_eq!(
        failed_document.subtitle_tracks,
        native_document.subtitle_tracks
    );
    assert_eq!(
        failed_document.transcription.native,
        native_document.transcription.native
    );
    assert_eq!(failed_document.transcription.provider_attempts.len(), 1);
    assert!(
        !failed_document.transcription.provider_attempts[0]
            .response
            .is_success()
    );
    assert!(failed_document.transcription.reconciled.is_none());
    assert!(failed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.provider.failed" && diagnostic.partial
    }));
}

#[test]
fn native_apis_and_default_disabled_registry_dispatch_are_provider_inert() {
    let bytes = wav();
    let direct = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("direct.wav"),
        MediaFormat::Wav,
        &MediaOptions::default(),
    );
    assert_eq!(direct.status, OperationStatus::Complete);
    assert!(
        direct
            .payload()
            .unwrap()
            .transcription
            .provider_attempts
            .is_empty()
    );
    assert!(direct.providers.is_empty());

    let manifests = Arc::new(Mutex::new(Vec::new()));
    let registered = parse_registered(
        bytes,
        "wav",
        SourceInfo::stdin("registered.wav"),
        Some(provider(
            "fixture-media-transcription",
            FixedTranscription {
                manifests: Some(Arc::clone(&manifests)),
            },
        )),
        MediaOptions::default(),
    );
    assert_eq!(registered.status, OperationStatus::Complete);
    assert!(registered.providers.is_empty());
    let document: MediaDocument = serde_json::from_value(registered.payload.unwrap()).unwrap();
    assert!(document.transcription.provider_attempts.is_empty());
    assert!(manifests.lock().unwrap().is_empty());

    let registry = builtin_parser_registry().unwrap();
    let ParserSelection::Available(descriptor) = registry.select_format("wav") else {
        panic!("wav unavailable")
    };
    assert!(
        descriptor
            .allowed_providers
            .contains(&ProviderKind::Transcription)
    );
    assert!(
        descriptor
            .capabilities
            .contains(&Capability::ProviderDerivedContent)
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_native_projection_never_invents_provider_transcription() {
    let envelope = grist::cli::parse_bytes(
        "wav",
        wav(),
        SourceInfo::stdin("cli-native.wav"),
        RequestId::new("cli-native-media").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert!(envelope.providers.is_empty());
    let document: MediaDocument =
        serde_json::from_value(envelope.payload.clone().unwrap()).unwrap();
    assert!(document.transcription.provider_attempts.is_empty());
    assert!(document.transcription.reconciled.is_none());
    let graph = grist::cli::project_envelope_to_graph(&envelope, "media:cli-native").unwrap();
    assert!(graph.nodes.iter().all(|node| {
        node.attrs
            .get("text_origin")
            .and_then(serde_json::Value::as_str)
            != Some("provider_transcription")
    }));
}
