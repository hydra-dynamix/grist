#![cfg(all(feature = "pdf", feature = "document-graph"))]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, NetworkAccess, OperationStatus, ParseRequest,
    Provider, ProviderSet, RequestId, ResourceBudget, SourceInfo, canonical_json_bytes,
};
use grist::document_graph::{DocumentGraphContext, DocumentRelation, ToDocumentGraph};
use grist::pdf::{PdfDocument, PdfOcrMode, PdfOptions, PdfTextSuppressionReason};
use grist::provider::{
    OcrProvider, OcrProviderAdapter, OcrRegion, OcrRequest, OcrResult, ProviderConfidence,
    ProviderDeterminism, ProviderError, ProviderMetadata, ProviderRequestManifest, ProviderResult,
    RecordedProvider,
};
use grist::registry::builtin_parser_registry;
use grist::segment::{SegmentOptions, segment_document_graph};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn source() -> SourceInfo {
    SourceInfo::new("ocr.pdf").with_declared_mime_type("application/pdf")
}

fn stream(dictionary: &str, bytes: &[u8]) -> Vec<u8> {
    let mut value = format!("<< {dictionary} /Length {} >>\nstream\n", bytes.len()).into_bytes();
    value.extend_from_slice(bytes);
    value.extend_from_slice(b"\nendstream");
    value
}

fn pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    bytes.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.into_iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn image() -> Vec<u8> {
    stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8",
        b"abc",
    )
}

fn simple_font() -> Vec<u8> {
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec()
}

fn scanned_fixture() -> Vec<u8> {
    pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", b"q 612 0 0 792 0 0 cm /Im1 Do Q"),
        image(),
    ])
}

fn hybrid_fixture() -> Vec<u8> {
    pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 6 0 R >> /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(
            "",
            b"q 200 0 0 80 40 650 cm /Im1 Do Q BT /F1 12 Tf 1 0 0 1 50 700 Tm (Native overlay) Tj ET",
        ),
        image(),
        simple_font(),
    ])
}

fn confidence(value: f64) -> ProviderConfidence {
    ProviderConfidence::new(value).unwrap()
}

fn result(text: &str, regions: Vec<(&str, f64)>) -> OcrResult {
    OcrResult {
        text: text.into(),
        regions: regions
            .into_iter()
            .enumerate()
            .map(|(index, (text, y))| OcrRegion {
                text: text.into(),
                bounding_box: grist::core::BoundingBox {
                    x: 0.0,
                    y,
                    width: 1.0,
                    height: if y == 0.0 { 1.0 } else { 0.25 },
                    unit: grist::core::CoordinateUnit::Normalized,
                    origin: grist::core::CoordinateOrigin::TopLeft,
                },
                confidence: Some(confidence(0.94)),
                language: Some("en".into()),
                reading_order: Some(index as u64),
            })
            .collect(),
        confidence: Some(confidence(0.93)),
        reading_order_confidence: Some(confidence(0.91)),
        layout_confidence: Some(confidence(0.92)),
        diagnostics: Vec::new(),
    }
}

fn metadata(name: &str, determinism: ProviderDeterminism) -> ProviderMetadata {
    ProviderMetadata::new(name, "fixture-ocr", "1", determinism)
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
            "fixture-pdf-ocr",
            "OCR backend unavailable",
        ))
    }
}

fn provider(implementation: impl OcrProvider, with_confidence: bool) -> Arc<dyn Provider> {
    let mut metadata = ProviderMetadata::new(
        "fixture-pdf-ocr",
        "fixture-ocr",
        "1",
        ProviderDeterminism::Guaranteed,
    )
    .unwrap()
    .with_model_version("layout-v1");
    if with_confidence {
        metadata = metadata.with_confidence_model("fixture-normalized-v1");
    }
    Arc::new(OcrProviderAdapter::new(metadata, implementation).unwrap())
}

fn parse_with(
    bytes: Vec<u8>,
    provider: Arc<dyn Provider>,
    options: PdfOptions,
) -> grist::core::Envelope<serde_json::Value> {
    let mut providers = ProviderSet::none();
    providers.select(ProviderKind::Ocr, provider, NetworkAccess::Denied);
    let request = ParseRequest::new(
        RequestId::new("pdf-ocr-test").unwrap(),
        Input::bytes(bytes),
        source(),
        BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
        providers,
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch("pdf", request, Some(serde_json::to_value(options).unwrap()))
        .unwrap()
}

use grist::core::ProviderKind;

#[test]
fn scanned_page_ocr_is_separate_attributed_and_segmentable() {
    let bytes = scanned_fixture();
    let output = result("Scanned page", vec![("Scanned page", 0.0)]);
    let envelope = parse_with(
        bytes.clone(),
        provider(
            FixedOcr {
                output,
                manifests: None,
            },
            true,
        ),
        PdfOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    assert_eq!(envelope.providers.len(), 1);
    assert_eq!(envelope.providers[0].provider, "fixture-pdf-ocr");
    let document: PdfDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    let page = &document.text.pages[0];
    assert!(page.native.value.text.is_empty());
    assert_eq!(page.ocr_attempts[0].regions[0].text, "Scanned page");
    assert_eq!(page.reconciled.as_ref().unwrap().value.text, "Scanned page");
    assert!(page.ocr_attempts[0].regions[0].locator.validate().is_ok());

    let graph = document
        .to_document_graph(DocumentGraphContext::new("pdf-ocr").with_source(source()))
        .unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.attrs
            .get("text_origin")
            .and_then(serde_json::Value::as_str)
            == Some("reconciled")
            && node.text.as_deref() == Some("Scanned page")
    }));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ReconciledWith)
    );
    graph.validate_contract().unwrap();

    let mut options = SegmentOptions::default();
    options
        .selection
        .required_metadata
        .insert("text_origin".into(), "reconciled".into());
    let source_identity = ContentIdentity::for_raw_bytes(&bytes);
    let document_identity = ContentIdentity::for_raw_bytes(&canonical_json_bytes(&graph).unwrap());
    let segments =
        segment_document_graph(&graph, &source_identity, &document_identity, &options, None)
            .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| segment.text.contains("Scanned page"))
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
fn hybrid_reconciliation_names_duplicate_suppression_and_keeps_all_origins() {
    let output = result(
        "Native overlay\nPhoto label",
        vec![("Native overlay", 0.0), ("Photo label", 0.7)],
    );
    let envelope = parse_with(
        hybrid_fixture(),
        provider(
            FixedOcr {
                output,
                manifests: None,
            },
            true,
        ),
        PdfOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document: PdfDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    let page = &document.text.pages[0];
    assert!(page.native.value.text.contains("Native overlay"));
    assert_eq!(
        page.ocr_attempts.len(),
        1,
        "hybrid raster region is OCR scoped once"
    );
    let reconciled = &page.reconciled.as_ref().unwrap().value;
    assert_eq!(reconciled.text.matches("Native overlay").count(), 1);
    assert!(reconciled.text.contains("Photo label"));
    assert_eq!(reconciled.suppressions.len(), 1);
    assert_eq!(
        reconciled.suppressions[0].reason,
        PdfTextSuppressionReason::DuplicateNativePreferred
    );
    assert_eq!(
        reconciled.suppressions[0].loss_class,
        "duplicate_suppression"
    );
    assert!(reconciled.reading_order_confidence > 0.0);
    assert!(reconciled.layout_confidence > 0.0);
    assert!(
        page.reconciled
            .as_ref()
            .unwrap()
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "pdf.ocr.duplicate_suppressed" })
    );

    let graph = document
        .to_document_graph(DocumentGraphContext::new("pdf-hybrid-ocr").with_source(source()))
        .unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| { edge.relation == DocumentRelation::AlternativeRepresentationOf })
    );
    graph.validate_contract().unwrap();
}

#[test]
fn provider_failure_preserves_native_text_without_reconciliation() {
    let envelope = parse_with(
        hybrid_fixture(),
        provider(FailingOcr, false),
        PdfOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let document: PdfDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    let page = &document.text.pages[0];
    assert!(page.native.value.text.contains("Native overlay"));
    assert_eq!(page.ocr_attempts.len(), 1);
    assert!(!page.ocr_attempts[0].response.is_success());
    assert!(page.reconciled.is_none());
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.class == grist::core::DiagnosticClass::ProviderFailure)
    );
}

#[test]
fn recorded_page_ocr_replay_is_deterministic() {
    let bytes = scanned_fixture();
    let output = result("Recorded scan", vec![("Recorded scan", 0.0)]);
    let manifests = Arc::new(Mutex::new(Vec::new()));
    let capture = Arc::new(
        OcrProviderAdapter::new(
            metadata("recorded-pdf-ocr", ProviderDeterminism::Guaranteed),
            FixedOcr {
                output: output.clone(),
                manifests: Some(manifests.clone()),
            },
        )
        .unwrap(),
    );
    let first = parse_with(bytes.clone(), capture, PdfOptions::default());
    assert_eq!(first.status, OperationStatus::Complete);
    let entries = manifests
        .lock()
        .unwrap()
        .iter()
        .map(|manifest| {
            (
                manifest.request_digest().unwrap(),
                ProviderResult::Ocr(output.clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let recorded = RecordedProvider::new(
        ProviderKind::Ocr,
        metadata(
            "recorded-pdf-ocr",
            ProviderDeterminism::GuaranteedWithRecording,
        ),
        entries,
    )
    .unwrap();
    let replay_a = parse_with(bytes.clone(), Arc::new(recorded), PdfOptions::default());

    let entries = manifests
        .lock()
        .unwrap()
        .iter()
        .map(|manifest| {
            (
                manifest.request_digest().unwrap(),
                ProviderResult::Ocr(output.clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let recorded = RecordedProvider::new(
        ProviderKind::Ocr,
        metadata(
            "recorded-pdf-ocr",
            ProviderDeterminism::GuaranteedWithRecording,
        ),
        entries,
    )
    .unwrap();
    let replay_b = parse_with(bytes, Arc::new(recorded), PdfOptions::default());
    assert_eq!(
        canonical_json_bytes(&replay_a).unwrap(),
        canonical_json_bytes(&replay_b).unwrap()
    );
    assert_eq!(replay_a.providers[0].deterministic, Some(true));
}

#[test]
fn ocr_can_be_disabled_even_when_a_provider_is_selected() {
    let mut options = PdfOptions::default();
    options.ocr.mode = PdfOcrMode::Disabled;
    let envelope = parse_with(
        scanned_fixture(),
        provider(
            FixedOcr {
                output: result("unused", vec![("unused", 0.0)]),
                manifests: None,
            },
            true,
        ),
        options,
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert!(envelope.providers.is_empty());
    let document: PdfDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert!(document.text.pages[0].ocr_attempts.is_empty());
}

#[test]
fn invalid_reconciliation_threshold_is_rejected_before_provider_use() {
    let mut options = PdfOptions::default();
    options.ocr.duplicate_overlap = 1.5;
    let envelope = parse_with(
        scanned_fixture(),
        provider(
            FixedOcr {
                output: result("unused", vec![("unused", 0.0)]),
                manifests: None,
            },
            true,
        ),
        options,
    );
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert!(envelope.payload.is_none());
    assert!(envelope.providers.is_empty());
    assert!(
        envelope.diagnostics[0]
            .message
            .contains("ocr.duplicate_overlap")
    );
}
