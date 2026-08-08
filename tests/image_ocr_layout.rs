#![cfg(all(feature = "media", feature = "document-graph"))]

use grist::core::{
    BoundingBox, BudgetSelection, ContentIdentity, CoordinateOrigin, CoordinateUnit, Input,
    NetworkAccess, OperationStatus, ParseRequest, Provider, ProviderKind, ProviderSet, RequestId,
    ResourceBudget, SourceInfo, canonical_json_bytes,
};
use grist::document_graph::{DocumentGraphContext, DocumentRelation, ToDocumentGraph};
use grist::image::{ImageDocument, ImageOptions, ImageReconciledTextSource, ImageTextOrigin};
use grist::provider::{
    OcrProvider, OcrProviderAdapter, OcrRegion, OcrRequest, OcrResult, ProviderConfidence,
    ProviderDeterminism, ProviderError, ProviderMetadata, ProviderRequestManifest, ProviderResult,
    RecordedProvider,
};
use grist::registry::builtin_parser_registry;
use grist::segment::{SegmentOptions, segment_document_graph};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn source() -> SourceInfo {
    SourceInfo::new("ocr.png").with_declared_mime_type("image/png")
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let mut value = index as u32;
        for _ in 0..8 {
            let mask = (value & 1).wrapping_neg();
            value = (value >> 1) ^ (0xedb8_8320 & mask);
        }
        *entry = value;
    }
    let mut crc = u32::MAX;
    for byte in bytes {
        crc = table[((crc as u8) ^ *byte) as usize] ^ (crc >> 8);
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&png_crc32(&out[4..]).to_be_bytes());
    out
}

fn fixture() -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&8u32.to_be_bytes());
    header.extend_from_slice(&4u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    out.extend(png_chunk(b"IHDR", &header));
    out.extend(png_chunk(b"tEXt", b"Label\0Native metadata text"));
    out.extend(png_chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1],
    ));
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn fixture_without_native_text() -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&8u32.to_be_bytes());
    header.extend_from_slice(&4u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    out.extend(png_chunk(b"IHDR", &header));
    out.extend(png_chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1],
    ));
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn confidence(value: f64) -> ProviderConfidence {
    ProviderConfidence::new(value).unwrap()
}

fn ocr_result() -> OcrResult {
    OcrResult {
        text: "Visible OCR".into(),
        regions: vec![OcrRegion {
            text: "Visible OCR".into(),
            bounding_box: BoundingBox {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
                unit: CoordinateUnit::Normalized,
                origin: CoordinateOrigin::TopLeft,
            },
            confidence: Some(confidence(0.94)),
            language: Some("en".into()),
            reading_order: Some(0),
        }],
        confidence: Some(confidence(0.93)),
        reading_order_confidence: Some(confidence(0.92)),
        layout_confidence: Some(confidence(0.91)),
        diagnostics: Vec::new(),
    }
}

fn metadata(name: &str, determinism: ProviderDeterminism) -> ProviderMetadata {
    ProviderMetadata::new(name, "fixture-image-ocr", "1", determinism)
        .unwrap()
        .with_model_version("layout-v1")
        .with_confidence_model("fixture-normalized-v1")
}

#[derive(Clone)]
struct FixedOcr {
    output: OcrResult,
    manifests: Option<Arc<Mutex<Vec<ProviderRequestManifest>>>>,
}

impl OcrProvider for FixedOcr {
    fn recognize(&self, request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError> {
        assert_eq!(request.context.network_access(), NetworkAccess::Denied);
        assert!(request.options.recognize_layout);
        assert!(request.options.source_locator.is_some());
        if let Some(manifests) = &self.manifests {
            manifests.lock().unwrap().push(request.manifest().unwrap());
        }
        Ok(self.output.clone())
    }
}

struct FailingOcr;

impl OcrProvider for FailingOcr {
    fn recognize(&self, _request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError> {
        Err(ProviderError::failure(
            "fixture-image-ocr",
            "OCR backend unavailable",
        ))
    }
}

#[derive(Clone)]
struct CountingOcr {
    calls: Arc<AtomicUsize>,
}

impl OcrProvider for CountingOcr {
    fn recognize(&self, _request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ocr_result())
    }
}

fn provider(implementation: impl OcrProvider, confidence_model: bool) -> Arc<dyn Provider> {
    let mut metadata = ProviderMetadata::new(
        "fixture-image-ocr",
        "fixture-image-ocr",
        "1",
        ProviderDeterminism::Guaranteed,
    )
    .unwrap()
    .with_model_version("layout-v1");
    if confidence_model {
        metadata = metadata.with_confidence_model("fixture-normalized-v1");
    }
    Arc::new(OcrProviderAdapter::new(metadata, implementation).unwrap())
}

fn parse_with(
    selected: Option<Arc<dyn Provider>>,
    options: ImageOptions,
) -> grist::core::Envelope<serde_json::Value> {
    parse_bytes_with(
        fixture(),
        selected,
        options,
        ResourceBudget::trusted_unbounded(),
    )
}

fn parse_bytes_with(
    bytes: Vec<u8>,
    selected: Option<Arc<dyn Provider>>,
    options: ImageOptions,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let mut providers = ProviderSet::none();
    if let Some(provider) = selected {
        providers.select(ProviderKind::Ocr, provider, NetworkAccess::Denied);
    }
    let request = ParseRequest::new(
        RequestId::new("image-ocr-test").unwrap(),
        Input::bytes(bytes),
        source(),
        BudgetSelection::custom(budget),
        providers,
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch("png", request, Some(serde_json::to_value(options).unwrap()))
        .unwrap()
}

#[test]
fn explicit_ocr_preserves_geometry_attribution_and_separate_text_facts() {
    let envelope = parse_with(
        Some(provider(
            FixedOcr {
                output: ocr_result(),
                manifests: None,
            },
            true,
        )),
        ImageOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    assert_eq!(envelope.providers.len(), 1);
    assert_eq!(envelope.providers[0].provider, "fixture-image-ocr");
    assert!(envelope.provenance.iter().any(|step| {
        step.implementation == "grist.image.native-ocr-append@1"
            && step.provider.as_deref() == Some("fixture-image-ocr")
    }));

    let payload = envelope.payload.unwrap();
    #[cfg(feature = "schemas")]
    assert!(
        grist::schema::validate_schema("image", &payload)
            .unwrap()
            .valid
    );
    let document: ImageDocument = serde_json::from_value(payload).unwrap();
    assert_eq!(document.embedded_text[0].text, "Native metadata text");
    assert_eq!(document.text.native.value.text, "Native metadata text");
    assert_eq!(document.text.native.value.entries, document.embedded_text);
    let attempt = &document.text.ocr_attempts[0];
    assert_eq!(attempt.scope.frame_index, 0);
    assert_eq!(attempt.response.metadata.provider.name, "fixture-image-ocr");
    assert_eq!(attempt.regions[0].text, "Visible OCR");
    assert_eq!(attempt.regions[0].bbox.x, 2.0);
    assert_eq!(attempt.regions[0].bbox.y, 1.0);
    assert_eq!(attempt.regions[0].bbox.width, 4.0);
    assert_eq!(attempt.regions[0].bbox.height, 2.0);
    assert_eq!(attempt.regions[0].bbox.unit, CoordinateUnit::Pixels);
    attempt.regions[0].locator.validate().unwrap();
    let reconciled = document.text.reconciled.as_ref().unwrap();
    assert!(reconciled.value.text.contains("Native metadata text"));
    assert!(reconciled.value.text.contains("Visible OCR"));
    assert!(
        reconciled
            .value
            .items
            .iter()
            .any(|item| item.origin == ImageTextOrigin::Native)
    );
    assert!(
        reconciled
            .value
            .items
            .iter()
            .any(|item| item.origin == ImageTextOrigin::Ocr)
    );

    let graph = document
        .to_document_graph(DocumentGraphContext::new("image-ocr").with_source(source()))
        .unwrap();
    graph.validate_contract().unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.attrs
            .get("text_origin")
            .and_then(serde_json::Value::as_str)
            == Some("ocr")
            && node.text.as_deref() == Some("Visible OCR")
    }));
    assert!(graph.nodes.iter().any(|node| {
        node.attrs
            .get("text_origin")
            .and_then(serde_json::Value::as_str)
            == Some("reconciled")
    }));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ReconciledWith)
    );

    let mut segment_options = SegmentOptions::default();
    segment_options
        .selection
        .required_metadata
        .insert("text_origin".into(), "reconciled".into());
    let source_identity = ContentIdentity::for_raw_bytes(&fixture());
    let document_identity = ContentIdentity::for_raw_bytes(&canonical_json_bytes(&graph).unwrap());
    let segments = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &segment_options,
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| segment.text.contains("Visible OCR"))
    );
    assert!(
        segments
            .segments
            .iter()
            .flat_map(|segment| &segment.locators)
            .all(|locator| locator.validate().is_ok())
    );

    let default_segments = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(
        default_segments
            .segments
            .iter()
            .map(|segment| segment.text.matches("Visible OCR").count())
            .sum::<usize>(),
        1,
        "default segmentation must use the reconciled representation once"
    );

    let mut raw_ocr_options = SegmentOptions::default();
    raw_ocr_options
        .selection
        .required_metadata
        .insert("text_origin".into(), "ocr".into());
    let raw_ocr_segments = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &raw_ocr_options,
        None,
    )
    .unwrap();
    assert_eq!(
        raw_ocr_segments
            .segments
            .iter()
            .map(|segment| segment.text.matches("Visible OCR").count())
            .sum::<usize>(),
        1,
        "explicit origin selection must retain raw OCR regions"
    );
}

#[test]
fn provider_failure_preserves_native_metadata_and_text() {
    let native = parse_with(None, ImageOptions::default());
    assert_eq!(native.status, OperationStatus::Complete);
    assert!(native.providers.is_empty());
    let native_document: ImageDocument = serde_json::from_value(native.payload.unwrap()).unwrap();
    assert!(native_document.text.ocr_attempts.is_empty());

    let failed = parse_with(Some(provider(FailingOcr, false)), ImageOptions::default());
    assert_eq!(failed.status, OperationStatus::Partial);
    let failed_document: ImageDocument = serde_json::from_value(failed.payload.unwrap()).unwrap();
    assert_eq!(failed_document.metadata, native_document.metadata);
    assert_eq!(failed_document.embedded_text, native_document.embedded_text);
    assert_eq!(failed_document.text.native, native_document.text.native);
    assert_eq!(failed_document.text.ocr_attempts.len(), 1);
    assert!(!failed_document.text.ocr_attempts[0].response.is_success());
    assert!(failed_document.text.reconciled.is_none());
}

#[test]
fn recorded_ocr_replay_is_deterministic_and_network_denied() {
    let manifests = Arc::new(Mutex::new(Vec::new()));
    let captured = parse_with(
        Some(Arc::new(
            OcrProviderAdapter::new(
                metadata("recorded-image-ocr", ProviderDeterminism::Guaranteed),
                FixedOcr {
                    output: ocr_result(),
                    manifests: Some(manifests.clone()),
                },
            )
            .unwrap(),
        )),
        ImageOptions::default(),
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
                    ProviderResult::Ocr(ocr_result()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let replay = || {
        Arc::new(
            RecordedProvider::new(
                ProviderKind::Ocr,
                metadata(
                    "recorded-image-ocr",
                    ProviderDeterminism::GuaranteedWithRecording,
                ),
                entries(),
            )
            .unwrap(),
        ) as Arc<dyn Provider>
    };
    let first = parse_with(Some(replay()), ImageOptions::default());
    let second = parse_with(Some(replay()), ImageOptions::default());
    assert_eq!(
        canonical_json_bytes(&first).unwrap(),
        canonical_json_bytes(&second).unwrap()
    );
    assert_eq!(first.providers[0].deterministic, Some(true));
}

#[test]
fn reconciliation_can_be_disabled_without_discarding_ocr() {
    let mut options = ImageOptions::default();
    options.ocr.reconcile = false;
    let envelope = parse_with(
        Some(provider(
            FixedOcr {
                output: ocr_result(),
                manifests: None,
            },
            true,
        )),
        options,
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert!(
        !envelope
            .provenance
            .iter()
            .any(|step| { step.implementation == "grist.image.native-ocr-append@1" })
    );
    let document: ImageDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert_eq!(document.text.ocr_attempts.len(), 1);
    assert!(document.text.reconciled.is_none());
    assert_eq!(document.text.native.value.text, "Native metadata text");
}

#[test]
fn disabled_mode_never_calls_an_explicitly_selected_provider() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut options = ImageOptions::default();
    options.ocr.mode = grist::image::ImageOcrMode::Disabled;
    let envelope = parse_with(
        Some(provider(
            CountingOcr {
                calls: calls.clone(),
            },
            true,
        )),
        options,
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert!(envelope.providers.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn oversized_and_over_budget_provider_outputs_are_rejected_before_hosting() {
    let region = OcrRegion {
        text: "x".into(),
        bounding_box: BoundingBox {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            unit: CoordinateUnit::Normalized,
            origin: CoordinateOrigin::TopLeft,
        },
        confidence: Some(confidence(0.9)),
        language: None,
        reading_order: None,
    };
    let oversized = OcrResult {
        text: String::new(),
        regions: vec![region; 10_001],
        confidence: Some(confidence(0.9)),
        reading_order_confidence: Some(confidence(0.9)),
        layout_confidence: Some(confidence(0.9)),
        diagnostics: Vec::new(),
    };
    let limited = parse_bytes_with(
        fixture_without_native_text(),
        Some(provider(
            FixedOcr {
                output: oversized,
                manifests: None,
            },
            true,
        )),
        ImageOptions::default(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(limited.status, OperationStatus::Partial);
    let limited_document: ImageDocument = serde_json::from_value(limited.payload.unwrap()).unwrap();
    assert_eq!(limited_document.text.ocr_attempts.len(), 1);
    assert!(!limited_document.text.ocr_attempts[0].response.is_success());
    assert!(limited_document.text.ocr_attempts[0].regions.is_empty());
    assert!(limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.exhausted"
            || diagnostic.message.contains("per-scope limit")
    }));

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_decoded_characters = Some(8);
    let over_budget = parse_bytes_with(
        fixture_without_native_text(),
        Some(provider(
            FixedOcr {
                output: ocr_result(),
                manifests: None,
            },
            true,
        )),
        ImageOptions::default(),
        budget,
    );
    assert_eq!(over_budget.status, OperationStatus::Partial);
    let over_budget_document: ImageDocument =
        serde_json::from_value(over_budget.payload.unwrap()).unwrap();
    assert!(
        !over_budget_document.text.ocr_attempts[0]
            .response
            .is_success()
    );
    assert!(over_budget_document.text.reconciled.is_none());
    assert!(over_budget.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.decoded_characters.exhausted"
    }));
}

#[test]
fn distinct_provider_full_text_is_projected_and_declared_structure_flattened() {
    let mut output = ocr_result();
    output.text = "Complete OCR text with an unboxed tail".into();
    let envelope = parse_with(
        Some(provider(
            FixedOcr {
                output,
                manifests: None,
            },
            true,
        )),
        ImageOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let reconciliation_step = envelope
        .provenance
        .iter()
        .find(|step| step.implementation == "grist.image.native-ocr-append@1")
        .unwrap();
    assert_eq!(
        reconciliation_step
            .loss_class
            .as_ref()
            .map(|loss| loss.as_str()),
        Some("structure_flattened")
    );
    let document: ImageDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert!(
        document
            .text
            .reconciled
            .as_ref()
            .unwrap()
            .value
            .items
            .iter()
            .any(|item| matches!(
                &item.source,
                ImageReconciledTextSource::OcrOverall { attempt_index: 0 }
            ))
    );
    let graph = document
        .to_document_graph(DocumentGraphContext::new("image-ocr-overall").with_source(source()))
        .unwrap();
    graph.validate_contract().unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.attrs
            .get("text_origin")
            .and_then(serde_json::Value::as_str)
            == Some("ocr_overall")
            && node.text.as_deref() == Some("Complete OCR text with an unboxed tail")
    }));
    let source_identity = ContentIdentity::for_raw_bytes(&fixture());
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
            .map(|segment| {
                segment
                    .text
                    .matches("Complete OCR text with an unboxed tail")
                    .count()
            })
            .sum::<usize>(),
        1
    );
}

#[test]
fn legacy_native_text_defaults_are_canonicalized_and_divergence_is_rejected() {
    let envelope = parse_with(None, ImageOptions::default());
    let mut payload = envelope.payload.unwrap();
    payload.as_object_mut().unwrap().remove("text");
    let document: ImageDocument = serde_json::from_value(payload).unwrap();
    assert!(document.text.native.value.entries.is_empty());
    let graph = document
        .to_document_graph(DocumentGraphContext::new("legacy-image").with_source(source()))
        .unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.text.as_deref() == Some("Native metadata text")
            && node
                .attrs
                .get("text_origin")
                .and_then(serde_json::Value::as_str)
                == Some("native")
    }));

    let mut divergent: ImageDocument =
        serde_json::from_value(parse_with(None, ImageOptions::default()).payload.unwrap()).unwrap();
    divergent.text.native.value.text = "tampered".into();
    assert!(
        divergent
            .to_document_graph(DocumentGraphContext::new("divergent-image"))
            .is_err()
    );
}

#[test]
fn frame_relative_geometry_is_normalized_or_diagnosed_without_panics() {
    let regions = vec![
        OcrRegion {
            text: "pixel".into(),
            bounding_box: BoundingBox {
                x: 1.0,
                y: 1.0,
                width: 2.0,
                height: 1.0,
                unit: CoordinateUnit::Pixels,
                origin: CoordinateOrigin::BottomLeft,
            },
            confidence: Some(confidence(0.9)),
            language: None,
            reading_order: Some(0),
        },
        OcrRegion {
            text: "outside".into(),
            bounding_box: BoundingBox {
                x: 7.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
                unit: CoordinateUnit::Pixels,
                origin: CoordinateOrigin::TopLeft,
            },
            confidence: Some(confidence(0.8)),
            language: None,
            reading_order: Some(1),
        },
        OcrRegion {
            text: "points".into(),
            bounding_box: BoundingBox {
                x: 1.0,
                y: 1.0,
                width: 2.0,
                height: 1.0,
                unit: CoordinateUnit::Points,
                origin: CoordinateOrigin::TopLeft,
            },
            confidence: Some(confidence(0.7)),
            language: None,
            reading_order: Some(2),
        },
    ];
    let output = OcrResult {
        text: "pixel\noutside\npoints".into(),
        regions,
        confidence: Some(confidence(0.9)),
        reading_order_confidence: Some(confidence(0.9)),
        layout_confidence: Some(confidence(0.9)),
        diagnostics: Vec::new(),
    };
    let envelope = parse_with(
        Some(provider(
            FixedOcr {
                output,
                manifests: None,
            },
            true,
        )),
        ImageOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "image.ocr.region_geometry_invalid" })
    );
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "image.ocr.geometry_unit_unresolved" })
    );
    let document: ImageDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    let attempt = &document.text.ocr_attempts[0];
    assert_eq!(attempt.regions.len(), 2);
    let pixel = attempt
        .regions
        .iter()
        .find(|region| region.text == "pixel")
        .unwrap();
    assert_eq!(pixel.bbox.x, 1.0);
    assert_eq!(pixel.bbox.y, 2.0);
    assert_eq!(pixel.bbox.origin, CoordinateOrigin::TopLeft);
    assert!(
        attempt
            .regions
            .iter()
            .all(|region| region.locator.validate().is_ok())
    );
}

#[cfg(feature = "schemas")]
#[test]
fn checked_image_schemas_match_generated_contracts() {
    for (name, file) in [
        ("image", "grist.image.v1.schema.json"),
        ("image-envelope", "grist.image-envelope.v2.schema.json"),
        ("image-options", "grist.image-options.v1.schema.json"),
    ] {
        let mut expected = serde_json::to_string_pretty(
            &grist::schema::schema_json(name).expect("registered image schema"),
        )
        .unwrap();
        expected.push('\n');
        let actual = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("schemas")
                .join(file),
        )
        .unwrap();
        assert_eq!(actual, expected, "schema drift for {name}");
    }
}
