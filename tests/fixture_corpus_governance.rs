use grist::core::{NetworkAccess, ProviderKind, ProviderSet};
use grist::fixtures::{
    ExpectedNormalization, FixtureClass, FixtureOrigin, FixtureStorage, REQUIRED_FIXTURE_CLASSES,
    Redistribution, canonical_expected_bytes, load_corpus, validate_corpus_at,
};
use grist::provider::{
    OcrOptions, OcrRequest, ProviderRecordingCatalog, ProviderRequest, ProviderRequestContext,
    TranscriptionOptions, TranscriptionRequest,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_recording(path: &str) -> ProviderRecordingCatalog {
    let bytes = std::fs::read(root().join("fixtures").join(path)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn ocr_request() -> ProviderRequest<'static> {
    let context = ProviderRequestContext::new(
        b"synthetic image bytes",
        NetworkAccess::Denied,
        &json!({"dpi": 300, "fixture": true}),
    )
    .unwrap();
    ProviderRequest::Ocr(OcrRequest::new(
        context,
        OcrOptions {
            language_hints: vec!["en".into()],
            recognize_layout: true,
            recognize_tables: false,
            source_locator: None,
        },
    ))
}

fn transcription_request() -> ProviderRequest<'static> {
    let context = ProviderRequestContext::new(
        b"synthetic audio bytes",
        NetworkAccess::Denied,
        &json!({"fixture": true, "sample_rate": 16000}),
    )
    .unwrap();
    ProviderRequest::Transcription(TranscriptionRequest::new(
        context,
        TranscriptionOptions {
            language_hints: vec!["en".into()],
            speaker_diarization: true,
            word_timestamps: false,
            track: Some("main".into()),
        },
    ))
}

#[test]
fn checked_corpus_is_complete_safe_and_identity_verified() {
    let corpus = load_corpus(root()).expect("checked fixture corpus must validate");
    let actual = corpus
        .policy
        .required_classes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual,
        REQUIRED_FIXTURE_CLASSES.into_iter().collect(),
        "Section 16.1 policy coverage drifted"
    );
    assert!(
        corpus
            .formats
            .values()
            .all(|format| !format.cases.is_empty())
    );
}

#[cfg(feature = "schemas")]
#[test]
fn corpus_and_recordings_validate_against_checked_public_schemas() {
    let corpus: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("fixtures/corpus.v1.json")).unwrap())
            .unwrap();
    assert!(
        grist::schema::validate_schema("fixture-corpus-manifest", &corpus)
            .unwrap()
            .valid
    );
    for path in [
        "generated/provider/ocr-baseline.v1.json",
        "generated/provider/transcription-baseline.v1.json",
    ] {
        let recording: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root().join("fixtures").join(path)).unwrap())
                .unwrap();
        assert!(
            grist::schema::validate_schema("provider-recording-catalog", &recording)
                .unwrap()
                .valid
        );
    }
}

#[test]
fn corpus_validation_rejects_unsafe_or_untraceable_registrations() {
    let mut corpus = load_corpus(root()).unwrap();
    let fixtures_root = root().join("fixtures");
    let case = &mut corpus.formats.get_mut("markdown").unwrap().cases[0];
    case.handling.network_allowed = true;
    case.provenance.origin = FixtureOrigin::DownstreamRegression;
    case.classes.push(FixtureClass::DownstreamRegression);
    case.provenance.source_project = None;
    case.provenance.issue_uri = None;
    case.provenance.original_sha256 = None;
    case.input.path = Some("../outside".into());

    let report = validate_corpus_at(fixtures_root, &corpus);
    let codes = report
        .violations
        .iter()
        .map(|violation| violation.code.as_str())
        .collect::<BTreeSet<_>>();
    assert!(codes.contains("grist.fixture.handling.unsafe"));
    assert!(codes.contains("grist.fixture.regression.provenance_incomplete"));
    assert!(codes.contains("grist.fixture.path.invalid_or_duplicate"));
}

#[test]
fn restricted_regressions_are_metadata_only_and_licensed_bytes_need_a_source() {
    let mut corpus = load_corpus(root()).unwrap();
    let mut restricted = corpus.formats["text"].cases[0].clone();
    restricted.id = "downstream-restricted-metadata".into();
    restricted.classes = vec![FixtureClass::DownstreamRegression];
    restricted.input.path = None;
    restricted.provenance.origin = FixtureOrigin::DownstreamRegression;
    restricted.provenance.source = "downstream-private".into();
    restricted.provenance.source_project = Some("downstream-private".into());
    restricted.provenance.issue_uri = Some("https://example.invalid/issues/7".into());
    restricted.provenance.original_sha256 = Some(restricted.input.sha256.clone());
    restricted.provenance.license.redistribution = Redistribution::MetadataOnly;
    restricted.handling.storage = FixtureStorage::ExternalOnly;
    restricted.handling.data_classification = "restricted".into();
    restricted.builder = None;
    restricted.expected.clear();
    corpus
        .formats
        .get_mut("text")
        .unwrap()
        .cases
        .push(restricted);
    assert!(validate_corpus_at(root().join("fixtures"), &corpus).is_valid());

    let licensed = &mut corpus.formats.get_mut("markdown").unwrap().cases[0];
    licensed.provenance.origin = FixtureOrigin::Licensed;
    licensed.provenance.source_uri = None;
    let report = validate_corpus_at(root().join("fixtures"), &corpus);
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "grist.fixture.licensed.source_missing")
    );
}

#[test]
fn checked_provider_recordings_replay_exact_requests_without_network() {
    for (path, kind, request) in [
        (
            "generated/provider/ocr-baseline.v1.json",
            ProviderKind::Ocr,
            ocr_request(),
        ),
        (
            "generated/provider/transcription-baseline.v1.json",
            ProviderKind::Transcription,
            transcription_request(),
        ),
    ] {
        let catalog = load_recording(path);
        catalog.validate().unwrap();
        let provider = catalog.into_provider().unwrap();
        let mut providers = ProviderSet::none();
        providers.select(kind, Arc::new(provider), NetworkAccess::Denied);
        let first = providers.invoke(&request).unwrap();
        let second = providers.invoke(&request).unwrap();
        assert_eq!(first, second);
        assert!(first.metadata.recorded);
        assert_eq!(first.envelope_invocation().deterministic, Some(true));
    }
}

#[test]
fn provider_recording_tampering_is_rejected_before_replay() {
    let mut request_tampered = load_recording("generated/provider/ocr-baseline.v1.json");
    request_tampered.entries[0].request_digest =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000".into();
    assert!(request_tampered.validate().is_err());

    let mut output_tampered = load_recording("generated/provider/transcription-baseline.v1.json");
    output_tampered.entries[0].output_sha256 =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000".into();
    assert!(output_tampered.validate().is_err());
}

#[test]
fn expected_output_normalization_is_explicit_and_canonical() {
    let value = json!({
        "z": 1,
        "source": {"path": "D:/fixture-root/input.md", "caller_time": "volatile"},
        "a": 2
    });
    let rules = [
        ExpectedNormalization::RemoveCallerTimestamp {
            json_pointer: "/source/caller_time".into(),
        },
        ExpectedNormalization::ReplaceFixtureRoot {
            json_pointer: "/source/path".into(),
        },
    ];
    let bytes = canonical_expected_bytes(value, &rules, "D:/fixture-root").unwrap();
    assert_eq!(
        bytes,
        br#"{"a":2,"source":{"path":"$FIXTURE_ROOT/input.md"},"z":1}
"#
    );
    assert!(canonical_expected_bytes(json!({}), &[], ".").is_err());
}
