use grist::core::{
    BoundingBox, CoordinateOrigin, CoordinateUnit, NetworkAccess, ProviderKind, ProviderSet,
    SecretBytes, SecretString, canonical_json_bytes, empty_options_digest,
};
use grist::provider::{
    BackendPermission, DecryptionOptions, DecryptionRequest, IsolatedBackendOptions,
    IsolatedBackendRequest, IsolationPolicy, NativeRepresentation, OcrOptions, OcrProvider,
    OcrProviderAdapter, OcrRegion, OcrRequest, OcrResult, ProviderConfidence,
    ProviderContractError, ProviderDeterminism, ProviderError, ProviderMetadata, ProviderRequest,
    ProviderRequestContext, ProviderResponse, ProviderResult, ProviderTiming,
    ReconciledRepresentation, RecordedProvider, RepresentationSet,
};
use serde_json::json;
use std::sync::Arc;

fn metadata(confidence: bool) -> ProviderMetadata {
    let metadata = ProviderMetadata::new(
        "fixture-ocr",
        "fixture",
        "1.2.3",
        ProviderDeterminism::Guaranteed,
    )
    .unwrap()
    .with_model_version("model-7");
    if confidence {
        metadata.with_confidence_model("normalized-probability-v1")
    } else {
        metadata
    }
}

fn box_at(x: f64) -> BoundingBox {
    BoundingBox {
        x,
        y: 0.1,
        width: 0.2,
        height: 0.1,
        unit: CoordinateUnit::Normalized,
        origin: CoordinateOrigin::TopLeft,
    }
}

fn ocr_result(with_confidence: bool) -> OcrResult {
    let confidence = with_confidence.then(|| ProviderConfidence::new(0.875).unwrap());
    OcrResult {
        text: "native-safe provider text".into(),
        regions: vec![OcrRegion {
            text: "provider text".into(),
            bounding_box: box_at(0.1),
            confidence,
            language: Some("en".into()),
            reading_order: Some(0),
        }],
        confidence,
        reading_order_confidence: None,
        layout_confidence: None,
        diagnostics: Vec::new(),
    }
}

struct SuccessfulOcr {
    confidence: bool,
}

impl OcrProvider for SuccessfulOcr {
    fn recognize(&self, _request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError> {
        Ok(ocr_result(self.confidence))
    }
}

struct FailingOcr;

impl OcrProvider for FailingOcr {
    fn recognize(&self, _request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError> {
        Err(ProviderError::failure(
            "fixture-ocr",
            "recorded provider outage",
        ))
    }
}

fn ocr_request<'a>(bytes: &'a [u8], network: NetworkAccess) -> ProviderRequest<'a> {
    let context = ProviderRequestContext::new(bytes, network, &json!({"dpi": 300})).unwrap();
    ProviderRequest::Ocr(OcrRequest::new(context, OcrOptions::default()))
}

#[test]
fn network_permission_and_provider_selection_are_explicit_per_request() {
    let provider = Arc::new(
        OcrProviderAdapter::new(metadata(false), SuccessfulOcr { confidence: false }).unwrap(),
    );
    let mut providers = ProviderSet::none();
    let request = ocr_request(b"image", NetworkAccess::Denied);
    assert_eq!(
        providers.invoke(&request).unwrap_err(),
        ProviderContractError::ProviderNotSelected(ProviderKind::Ocr)
    );

    providers.select(ProviderKind::Ocr, provider, NetworkAccess::Denied);
    let mismatched = ocr_request(b"image", NetworkAccess::Allowed);
    assert_eq!(
        providers.invoke(&mismatched).unwrap_err(),
        ProviderContractError::NetworkPermissionMismatch
    );
    assert!(providers.invoke(&request).unwrap().is_success());
}

#[test]
fn secrets_are_runtime_only_redacted_and_excluded_from_digests() {
    let password_a = SecretString::new("never-print-password-a");
    let password_b = SecretString::new("never-print-password-b");
    let binary = SecretBytes::new(Vec::from(&b"binary-key"[..]).into_boxed_slice());

    let context_a = ProviderRequestContext::new(
        b"encrypted",
        NetworkAccess::Denied,
        &json!({"scheme": "age"}),
    )
    .unwrap()
    .with_text_secret("password", &password_a)
    .unwrap()
    .with_binary_secret("key", &binary)
    .unwrap();
    let request_a = DecryptionRequest::new(
        context_a,
        DecryptionOptions {
            scheme: "age".into(),
            key_id: Some("recipient-1".into()),
            expected_media_type: None,
        },
    )
    .unwrap();

    let context_b = ProviderRequestContext::new(
        b"encrypted",
        NetworkAccess::Denied,
        &json!({"scheme": "age"}),
    )
    .unwrap()
    .with_text_secret("password", &password_b)
    .unwrap()
    .with_binary_secret("key", &binary)
    .unwrap();
    let request_b = DecryptionRequest::new(
        context_b,
        DecryptionOptions {
            scheme: "age".into(),
            key_id: Some("recipient-1".into()),
            expected_media_type: None,
        },
    )
    .unwrap();

    let debug = format!("{request_a:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("never-print"));
    assert!(!debug.contains("binary-key"));
    assert_eq!(
        request_a.manifest().unwrap().request_digest().unwrap(),
        request_b.manifest().unwrap().request_digest().unwrap(),
        "secret values must not influence a hash"
    );
    let manifest_json = serde_json::to_string(&request_a.manifest().unwrap()).unwrap();
    assert!(!manifest_json.contains("password"));
    assert!(!manifest_json.contains("binary-key"));
}

#[test]
fn recorded_provider_replay_is_byte_deterministic_and_attributed() {
    let request = ocr_request(b"recorded-image", NetworkAccess::Denied);
    let digest = request.manifest().unwrap().request_digest().unwrap();
    let recorded = RecordedProvider::new(
        ProviderKind::Ocr,
        metadata(true),
        [(digest, ProviderResult::Ocr(ocr_result(true)))],
    )
    .unwrap();
    let mut providers = ProviderSet::none();
    providers.select(ProviderKind::Ocr, Arc::new(recorded), NetworkAccess::Denied);

    let first = providers.invoke(&request).unwrap();
    let second = providers.invoke(&request).unwrap();
    assert_eq!(
        canonical_json_bytes(&first).unwrap(),
        canonical_json_bytes(&second).unwrap()
    );
    assert!(first.metadata.recorded);
    assert_eq!(first.envelope_invocation().deterministic, Some(true));
    assert_eq!(
        first.metadata.configuration_digest,
        request.context().configuration_digest()
    );
    assert_eq!(first.metadata.provider.implementation_version, "1.2.3");
}

#[test]
fn serialized_response_tampering_is_rejected() {
    let request = ocr_request(b"image", NetworkAccess::Denied);
    let mut providers = ProviderSet::none();
    providers.select(
        ProviderKind::Ocr,
        Arc::new(
            OcrProviderAdapter::new(metadata(false), SuccessfulOcr { confidence: false }).unwrap(),
        ),
        NetworkAccess::Denied,
    );
    let response = providers.invoke(&request).unwrap();
    let mut value = serde_json::to_value(response).unwrap();
    value["metadata"]["output_identity"] =
        json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
    assert!(serde_json::from_value::<ProviderResponse>(value).is_err());
}

#[test]
fn caller_timing_is_copied_exactly_and_no_clock_is_consulted() {
    let timing =
        ProviderTiming::caller("fixture-clock", Some("2026-08-06T17:00:00Z"), Some(125)).unwrap();
    let context =
        ProviderRequestContext::new(b"image", NetworkAccess::Denied, &json!({"dpi": 300}))
            .unwrap()
            .with_timing(timing.clone());
    let request = ProviderRequest::Ocr(OcrRequest::new(context, OcrOptions::default()));
    let mut providers = ProviderSet::none();
    providers.select(
        ProviderKind::Ocr,
        Arc::new(
            OcrProviderAdapter::new(metadata(false), SuccessfulOcr { confidence: false }).unwrap(),
        ),
        NetworkAccess::Denied,
    );
    let response = providers.invoke(&request).unwrap();
    assert_eq!(response.metadata.timing, Some(timing));
}

#[test]
fn confidence_values_require_a_named_model() {
    let request = ocr_request(b"image", NetworkAccess::Denied);
    let mut providers = ProviderSet::none();
    providers.select(
        ProviderKind::Ocr,
        Arc::new(
            OcrProviderAdapter::new(metadata(false), SuccessfulOcr { confidence: true }).unwrap(),
        ),
        NetworkAccess::Denied,
    );
    assert!(matches!(
        providers.invoke(&request),
        Err(ProviderContractError::InvalidResult(_))
    ));

    let mut providers = ProviderSet::none();
    providers.select(
        ProviderKind::Ocr,
        Arc::new(
            OcrProviderAdapter::new(metadata(true), SuccessfulOcr { confidence: true }).unwrap(),
        ),
        NetworkAccess::Denied,
    );
    assert!(providers.invoke(&request).unwrap().is_success());
}

#[test]
fn provider_failure_is_appended_without_mutating_native_facts() {
    let native = NativeRepresentation::new(json!({
        "source": "native",
        "text": "authoritative"
    }))
    .unwrap();
    let native_identity = native.identity.clone();
    let mut representations: RepresentationSet<_, serde_json::Value> =
        RepresentationSet::new(native);
    let request = ocr_request(b"image", NetworkAccess::Denied);
    let mut providers = ProviderSet::none();
    providers.select(
        ProviderKind::Ocr,
        Arc::new(OcrProviderAdapter::new(metadata(false), FailingOcr).unwrap()),
        NetworkAccess::Denied,
    );

    let failure = providers.invoke(&request).unwrap();
    assert!(!failure.is_success());
    representations.record_provider_attempt(failure);
    assert_eq!(representations.native().identity, native_identity);
    assert_eq!(representations.native().value["text"], "authoritative");
    assert_eq!(representations.provider_results().count(), 0);
    assert!(representations.reconciled().is_none());
}

#[test]
fn reconciliation_is_a_third_identity_bearing_representation() {
    let native = NativeRepresentation::new(json!({"text": "native"})).unwrap();
    let native_identity = native.identity.clone();
    let mut representations = RepresentationSet::new(native);
    let request = ocr_request(b"image", NetworkAccess::Denied);
    let mut providers = ProviderSet::none();
    providers.select(
        ProviderKind::Ocr,
        Arc::new(
            OcrProviderAdapter::new(metadata(false), SuccessfulOcr { confidence: false }).unwrap(),
        ),
        NetworkAccess::Denied,
    );
    let response = providers.invoke(&request).unwrap();
    let provider_identity = response.metadata.output_identity.clone().unwrap();
    representations.record_provider_attempt(response);

    let reconciled = ReconciledRepresentation::new(
        json!({"text": "reconciled"}),
        "native-ocr-deduplicate",
        "1",
        empty_options_digest(),
        native_identity,
        vec![provider_identity],
    )
    .unwrap();
    representations.set_reconciled(reconciled).unwrap();

    let value = serde_json::to_value(&representations).unwrap();
    assert_eq!(value["native"]["value"]["text"], "native");
    assert_eq!(value["provider_attempts"].as_array().unwrap().len(), 1);
    assert_eq!(value["reconciled"]["value"]["text"], "reconciled");
}

#[test]
fn isolated_backends_require_permission_and_a_bounded_safe_policy() {
    let context = ProviderRequestContext::new(
        b"legacy-doc",
        NetworkAccess::Denied,
        &json!({"backend": "fixture"}),
    )
    .unwrap();
    let options = IsolatedBackendOptions {
        format: "legacy-word".into(),
        output_schema_version: "example/legacy-word/v1".into(),
        isolation: IsolationPolicy::strict(1_000, 32 * 1024 * 1024, 8 * 1024 * 1024).unwrap(),
        backend_options: json!({}),
    };
    assert_eq!(
        IsolatedBackendRequest::new(BackendPermission::Denied, context, options).unwrap_err(),
        ProviderContractError::BackendPermissionDenied
    );

    let context = ProviderRequestContext::new(
        b"legacy-doc",
        NetworkAccess::Allowed,
        &json!({"backend": "fixture"}),
    )
    .unwrap();
    let options = IsolatedBackendOptions {
        format: "legacy-word".into(),
        output_schema_version: "example/legacy-word/v1".into(),
        isolation: IsolationPolicy::strict(1_000, 32 * 1024 * 1024, 8 * 1024 * 1024).unwrap(),
        backend_options: json!({}),
    };
    assert_eq!(
        IsolatedBackendRequest::new(BackendPermission::ExplicitlyAllowed, context, options,)
            .unwrap_err(),
        ProviderContractError::NetworkPermissionMismatch
    );
}
