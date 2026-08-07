//! Explicit PDF OCR scope selection and deterministic native/OCR reconciliation.

use super::*;
use crate::core::{
    BoundingBox, CoordinateOrigin, CoordinateUnit, DeclaredLoss, Diagnostic, DiagnosticDetails,
    IndexPosition, LocationComponent, LocatorConfidence, LossClass, OperationKind, ProvenanceStep,
    ProviderInvocation, ProviderKind, SourceLocator, options_digest,
};
use crate::provider::{
    NativeRepresentation, OcrOptions, OcrRequest, ProviderConfidence, ProviderRequest,
    ProviderResponse, ProviderResult, ReconciledRepresentation,
};
use crate::registry::{ParserContext, ParserError};
use serde_json::json;
use std::collections::BTreeSet;

const RECONCILIATION_ALGORITHM: &str = "grist.pdf.native-ocr-reconcile";
const RECONCILIATION_VERSION: &str = "1";

pub(crate) struct PdfOcrOutcome {
    pub diagnostics: Vec<Diagnostic>,
    pub invocations: Vec<ProviderInvocation>,
    pub provenance: Vec<ProvenanceStep>,
}

pub(crate) fn native_text_content(layout: &PdfNativeLayout) -> Result<PdfTextContent, ParserError> {
    let mut pages = Vec::with_capacity(layout.pages.len());
    for page in &layout.pages {
        let mut ordered = page.blocks.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|block| {
            page.reading_order
                .block_order
                .iter()
                .position(|index| *index == block.index)
                .unwrap_or(usize::MAX)
        });
        let regions = ordered
            .iter()
            .enumerate()
            .map(|(order, block)| PdfNativeTextRegion {
                index: block.index,
                text: block.text.clone(),
                bbox: block.bbox,
                reading_order: order as u64,
                layout_confidence: block.confidence,
                locator: block.locator.clone(),
            })
            .collect::<Vec<_>>();
        let text = regions
            .iter()
            .map(|region| region.text.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let layout_confidence = mean(regions.iter().map(|region| region.layout_confidence));
        let native = NativeRepresentation::new(PdfNativePageText {
            page_index: page.page_index,
            text,
            status: page.status,
            regions,
            reading_order_confidence: page.reading_order.confidence,
            layout_confidence,
        })
        .map_err(|error| {
            Box::new(Diagnostic::parser_defect("grist.pdf", error.to_string())) as ParserError
        })?;
        pages.push(PdfPageTextContent {
            page_index: page.page_index,
            native,
            ocr_attempts: Vec::new(),
            reconciled: None,
        });
    }
    Ok(PdfTextContent { pages })
}

pub(crate) fn apply_selected_ocr(
    context: &mut ParserContext<'_>,
    document: &mut PdfDocument,
    options: &PdfOptions,
) -> PdfOcrOutcome {
    let mut outcome = PdfOcrOutcome {
        diagnostics: Vec::new(),
        invocations: Vec::new(),
        provenance: Vec::new(),
    };
    if options.ocr.mode == PdfOcrMode::Disabled {
        return outcome;
    }
    if context
        .selected_provider_network(ProviderKind::Ocr)
        .is_none()
    {
        return outcome;
    }

    let scopes = select_scopes(document, options, &mut outcome.diagnostics);
    if scopes.is_empty() {
        return outcome;
    }

    for scope in scopes {
        let configuration = json!({
            "adapter": "grist.pdf.ocr-scope",
            "version": 1,
            "scope": scope,
        });
        let ocr_options = OcrOptions {
            language_hints: options.ocr.language_hints.clone(),
            recognize_layout: true,
            recognize_tables: options.ocr.recognize_tables,
            source_locator: Some(scope.locator.clone()),
        };
        let response = match context.run_provider_for_input(
            ProviderKind::Ocr,
            &configuration,
            |request_context| ProviderRequest::Ocr(OcrRequest::new(request_context, ocr_options)),
        ) {
            Ok(response) => response,
            Err(diagnostic) => {
                outcome
                    .diagnostics
                    .push((*diagnostic).with_locator(scope.locator.clone()).partial());
                continue;
            }
        };
        outcome.invocations.push(response.envelope_invocation());
        for diagnostic in &response.metadata.diagnostics {
            outcome.diagnostics.push(
                diagnostic
                    .clone()
                    .with_locator(scope.locator.clone())
                    .partial(),
            );
        }
        let page_index = scope.page_index;
        let attempt = build_attempt(document, scope, response, &mut outcome.diagnostics);
        if let Some(page) = document
            .text
            .pages
            .iter_mut()
            .find(|page| page.page_index == page_index)
        {
            page.ocr_attempts.push(attempt);
        }
    }

    for page in &mut document.text.pages {
        if let Some((reconciled, mut diagnostics, provenance)) = reconcile_page(
            page,
            document
                .pages
                .iter()
                .find(|value| value.index == page.page_index),
            &options.ocr,
        ) {
            page.reconciled = Some(reconciled);
            outcome.diagnostics.append(&mut diagnostics);
            outcome.provenance.push(provenance);
        }
    }
    outcome
}

fn select_scopes(
    document: &PdfDocument,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PdfOcrScope> {
    let mut scopes = Vec::new();
    for page in &document.pages {
        let native = document
            .text
            .pages
            .iter()
            .find(|value| value.page_index == page.index)
            .map(|value| &value.native.value);
        let page_reason = match options.ocr.mode {
            PdfOcrMode::Disabled => None,
            PdfOcrMode::AllPages => Some(PdfOcrScopeReason::AllPagesRequested),
            PdfOcrMode::Auto => match native.map(|value| value.status) {
                Some(PdfNativeTextStatus::NoNativeText) | None => {
                    Some(PdfOcrScopeReason::NoNativeText)
                }
                Some(PdfNativeTextStatus::Unusable) => Some(PdfOcrScopeReason::NativeTextUnusable),
                Some(PdfNativeTextStatus::BudgetExceeded) => {
                    Some(PdfOcrScopeReason::NativeTextBudgetExceeded)
                }
                Some(PdfNativeTextStatus::Extracted)
                    if native.is_some_and(|value| value.text.trim().is_empty()) =>
                {
                    Some(PdfOcrScopeReason::NoNativeText)
                }
                Some(PdfNativeTextStatus::Extracted) => None,
            },
        };
        if let Some(reason) = page_reason {
            scopes.push(PdfOcrScope {
                kind: PdfOcrScopeKind::Page,
                reason,
                page_index: page.index,
                graphic_index: None,
                locator: page.locator.clone(),
            });
            continue;
        }
        if options.ocr.mode == PdfOcrMode::Auto {
            if let Some(semantic) = document
                .semantic_structure
                .pages
                .iter()
                .find(|value| value.page_index == page.index)
            {
                scopes.extend(
                    semantic
                        .graphics
                        .iter()
                        .filter(|graphic| graphic.kind == PdfGraphicObjectKind::RasterImage)
                        .map(|graphic| PdfOcrScope {
                            kind: PdfOcrScopeKind::Region,
                            reason: PdfOcrScopeReason::RasterImageOnHybridPage,
                            page_index: page.index,
                            graphic_index: Some(graphic.index),
                            locator: graphic.locator.clone(),
                        }),
                );
            }
        }
    }
    if scopes.len() as u64 > options.ocr.max_scopes {
        let omitted = scopes.len() as u64 - options.ocr.max_scopes;
        scopes.truncate(usize::try_from(options.ocr.max_scopes).unwrap_or(usize::MAX));
        diagnostics.push(
            Diagnostic::budget_exhausted(
                "grist.pdf",
                format!(
                    "PDF OCR scope limit {} omitted {omitted} page/region request(s)",
                    options.ocr.max_scopes
                ),
            )
            .with_details(
                DiagnosticDetails::from_value(json!({
                    "limit": options.ocr.max_scopes,
                    "omitted": omitted,
                }))
                .expect("OCR budget details are safe"),
            ),
        );
    }
    scopes
}

fn build_attempt(
    document: &PdfDocument,
    scope: PdfOcrScope,
    response: ProviderResponse,
    diagnostics: &mut Vec<Diagnostic>,
) -> PdfOcrAttempt {
    let result = match response.result() {
        Some(ProviderResult::Ocr(result)) => Some(result.clone()),
        _ => None,
    };
    let page = document
        .pages
        .iter()
        .find(|page| page.index == scope.page_index);
    let mut regions = result
        .as_ref()
        .map(|result| result.regions.clone())
        .unwrap_or_default();
    if regions.is_empty() {
        if let Some(result) = result
            .as_ref()
            .filter(|result| !result.text.trim().is_empty())
        {
            regions.push(crate::provider::OcrRegion {
                text: result.text.clone(),
                bounding_box: scope_bbox(&scope, page),
                confidence: result.confidence,
                language: None,
                reading_order: None,
            });
        }
    }
    let mut order = (0..regions.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        let a = &regions[*left];
        let b = &regions[*right];
        a.reading_order
            .unwrap_or(u64::MAX)
            .cmp(&b.reading_order.unwrap_or(u64::MAX))
            .then_with(|| a.bounding_box.y.total_cmp(&b.bounding_box.y))
            .then_with(|| a.bounding_box.x.total_cmp(&b.bounding_box.x))
            .then_with(|| left.cmp(right))
    });
    let mut projected = Vec::with_capacity(regions.len());
    for (reading_order, source_index) in order.into_iter().enumerate() {
        let region = &regions[source_index];
        let bbox = resolve_bbox(&scope, region.bounding_box, page);
        let confidence = region
            .confidence
            .or_else(|| result.as_ref().and_then(|value| value.layout_confidence))
            .or_else(|| result.as_ref().and_then(|value| value.confidence))
            .map_or(0.0, ProviderConfidence::get);
        let locator = ocr_locator(scope.page_index, bbox, page, confidence);
        projected.push(PdfOcrTextRegion {
            index: source_index as u64,
            text: region.text.clone(),
            bbox,
            confidence: region.confidence,
            language: region.language.clone(),
            reading_order: reading_order as u64,
            reading_order_inferred: region.reading_order.is_none(),
            locator,
        });
    }
    projected.sort_by_key(|region| region.reading_order);

    if let Some(result) = &result {
        if result.reading_order_confidence.is_none() {
            diagnostics.push(
                Diagnostic::warning(
                    "grist.pdf",
                    "pdf.ocr.reading_order_confidence_missing",
                    "OCR provider omitted reading-order confidence",
                )
                .with_locator(scope.locator.clone())
                .partial(),
            );
        }
        if result.layout_confidence.is_none() {
            diagnostics.push(
                Diagnostic::warning(
                    "grist.pdf",
                    "pdf.ocr.layout_confidence_missing",
                    "OCR provider omitted layout confidence",
                )
                .with_locator(scope.locator.clone())
                .partial(),
            );
        }
    }
    PdfOcrAttempt {
        scope,
        response,
        regions: projected,
        reading_order_confidence: result
            .as_ref()
            .and_then(|value| value.reading_order_confidence),
        layout_confidence: result.as_ref().and_then(|value| value.layout_confidence),
    }
}

fn reconcile_page(
    page: &PdfPageTextContent,
    page_geometry: Option<&PdfPage>,
    options: &PdfOcrOptions,
) -> Option<(
    ReconciledRepresentation<PdfReconciledPageText>,
    Vec<Diagnostic>,
    ProvenanceStep,
)> {
    let successful = page
        .ocr_attempts
        .iter()
        .enumerate()
        .filter(|(_, attempt)| attempt.response.is_success())
        .collect::<Vec<_>>();
    if successful.is_empty() {
        return None;
    }

    let mut items = page
        .native
        .value
        .regions
        .iter()
        .map(|region| PdfReconciledTextItem {
            index: 0,
            text: region.text.clone(),
            origin: PdfTextOrigin::Native,
            source: PdfReconciledTextSource::Native {
                region_index: region.index,
            },
            bbox: region.bbox,
            confidence: region.layout_confidence,
            locator: region.locator.clone(),
        })
        .collect::<Vec<_>>();
    let mut suppressions = Vec::new();
    let mut diagnostics = Vec::new();
    for (attempt_index, attempt) in successful {
        for region in &attempt.regions {
            let duplicate = page
                .native
                .value
                .regions
                .iter()
                .filter_map(|native| {
                    let similarity = text_similarity(&native.text, &region.text);
                    let overlap = geometric_overlap(native.bbox, region.bbox, page_geometry)?;
                    (similarity >= options.duplicate_text_similarity
                        && overlap >= options.duplicate_overlap)
                        .then_some((native, similarity, overlap))
                })
                .max_by(|left, right| {
                    left.1
                        .total_cmp(&right.1)
                        .then_with(|| left.2.total_cmp(&right.2))
                });
            if let Some((native, similarity, overlap)) = duplicate {
                let suppression = PdfTextSuppression {
                    reason: PdfTextSuppressionReason::DuplicateNativePreferred,
                    suppressed: PdfReconciledTextSource::Ocr {
                        attempt_index: attempt_index as u64,
                        region_index: region.index,
                    },
                    retained: PdfReconciledTextSource::Native {
                        region_index: native.index,
                    },
                    suppressed_text: region.text.clone(),
                    text_similarity: similarity,
                    geometric_overlap: overlap,
                    locator: region.locator.clone(),
                    retained_locator: native.locator.clone(),
                    loss_class: "duplicate_suppression".into(),
                };
                diagnostics.push(
                    Diagnostic::info(
                        "grist.pdf",
                        "pdf.ocr.duplicate_suppressed",
                        format!(
                            "suppressed OCR attempt {attempt_index} region {} as a duplicate of native region {}",
                            region.index, native.index
                        ),
                    )
                    .with_locator(region.locator.clone())
                    .with_details(
                        DiagnosticDetails::from_value(json!({
                            "attempt_index": attempt_index,
                            "ocr_region_index": region.index,
                            "native_region_index": native.index,
                            "text_similarity": similarity,
                            "geometric_overlap": overlap,
                            "loss_class": "duplicate_suppression",
                        }))
                        .expect("suppression details are safe"),
                    ),
                );
                suppressions.push(suppression);
                continue;
            }
            let confidence = region
                .confidence
                .or(attempt.layout_confidence)
                .map_or(0.0, ProviderConfidence::get);
            items.push(PdfReconciledTextItem {
                index: 0,
                text: region.text.clone(),
                origin: PdfTextOrigin::Ocr,
                source: PdfReconciledTextSource::Ocr {
                    attempt_index: attempt_index as u64,
                    region_index: region.index,
                },
                bbox: region.bbox,
                confidence,
                locator: region.locator.clone(),
            });
        }
    }
    items.sort_by(|left, right| {
        normalized_box(left.bbox, page_geometry)
            .map(|bbox| (bbox[1], bbox[0]))
            .partial_cmp(&normalized_box(right.bbox, page_geometry).map(|bbox| (bbox[1], bbox[0])))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| origin_rank(left.origin).cmp(&origin_rank(right.origin)))
            .then_with(|| source_rank(&left.source).cmp(&source_rank(&right.source)))
    });
    for (index, item) in items.iter_mut().enumerate() {
        item.index = index as u64;
    }
    let text = items
        .iter()
        .map(|item| item.text.as_str())
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let reading_order_confidence = mean(
        std::iter::once(page.native.value.reading_order_confidence).chain(
            page.ocr_attempts
                .iter()
                .filter_map(|attempt| attempt.reading_order_confidence)
                .map(ProviderConfidence::get),
        ),
    );
    let layout_confidence = mean(items.iter().map(|item| item.confidence));
    let mut confidence_evidence = vec![format!(
        "native reading_order={} layout={}",
        page.native.value.reading_order_confidence, page.native.value.layout_confidence
    )];
    confidence_evidence.extend(
        page.ocr_attempts
            .iter()
            .enumerate()
            .map(|(index, attempt)| {
                format!(
                    "ocr attempt {index} provider={} model={} reading_order={} layout={}",
                    attempt.response.metadata.provider.name,
                    attempt
                        .response
                        .metadata
                        .provider
                        .model_version
                        .as_deref()
                        .unwrap_or("unspecified"),
                    attempt
                        .reading_order_confidence
                        .map(ProviderConfidence::get)
                        .map_or_else(|| "missing".into(), |value| value.to_string()),
                    attempt
                        .layout_confidence
                        .map(ProviderConfidence::get)
                        .map_or_else(|| "missing".into(), |value| value.to_string()),
                )
            }),
    );
    let value = PdfReconciledPageText {
        text,
        items,
        suppressions,
        reading_order_confidence,
        layout_confidence,
        confidence_evidence,
    };
    let provider_output_identities = page
        .ocr_attempts
        .iter()
        .filter_map(|attempt| attempt.response.metadata.output_identity.clone())
        .collect::<Vec<_>>();
    let configuration_digest = options_digest(&json!({
        "algorithm": RECONCILIATION_ALGORITHM,
        "version": RECONCILIATION_VERSION,
        "duplicate_text_similarity": options.duplicate_text_similarity,
        "duplicate_overlap": options.duplicate_overlap,
    }))
    .expect("PDF reconciliation options serialize");
    let mut representation = ReconciledRepresentation::new(
        value,
        RECONCILIATION_ALGORITHM,
        RECONCILIATION_VERSION,
        configuration_digest.clone(),
        page.native.identity.clone(),
        provider_output_identities,
    )
    .expect("successful OCR identities satisfy reconciliation invariants");
    representation.confidence = ProviderConfidence::new(reading_order_confidence).ok();
    representation.diagnostics = diagnostics.clone();
    let provider = page
        .ocr_attempts
        .iter()
        .find(|attempt| attempt.response.is_success())
        .map(|attempt| attempt.response.metadata.provider.name.clone())
        .expect("successful OCR page has a provider");
    let loss = if representation.value.suppressions.is_empty() {
        DeclaredLoss::Lossless
    } else {
        DeclaredLoss::Lossy(LossClass::new("duplicate_suppression").expect("valid loss class"))
    };
    let mut provenance = ProvenanceStep::new(
        OperationKind::Parse,
        format!("{RECONCILIATION_ALGORITHM}@{RECONCILIATION_VERSION}"),
        page.native.identity.clone(),
        representation.identity.clone(),
        configuration_digest,
        loss,
    )
    .expect("PDF OCR reconciliation provenance is valid")
    .with_provider(provider);
    if !representation.value.suppressions.is_empty() {
        provenance = provenance.with_warning("pdf.ocr.duplicate_suppressed");
    }
    Some((representation, diagnostics, provenance))
}

fn scope_bbox(scope: &PdfOcrScope, page: Option<&PdfPage>) -> BoundingBox {
    match scope.locator.innermost() {
        LocationComponent::PdfRegion {
            bbox: Some(bbox), ..
        } => *bbox,
        _ => page.map_or(
            BoundingBox {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                unit: CoordinateUnit::Normalized,
                origin: CoordinateOrigin::TopLeft,
            },
            |page| BoundingBox {
                x: 0.0,
                y: 0.0,
                width: page.width_points,
                height: page.height_points,
                unit: CoordinateUnit::Points,
                origin: CoordinateOrigin::BottomLeft,
            },
        ),
    }
}

fn resolve_bbox(scope: &PdfOcrScope, child: BoundingBox, page: Option<&PdfPage>) -> BoundingBox {
    if scope.kind != PdfOcrScopeKind::Region || child.unit != CoordinateUnit::Normalized {
        return child;
    }
    let parent = scope_bbox(scope, page);
    let child_y = if child.origin == parent.origin {
        child.y
    } else {
        1.0 - child.y - child.height
    };
    BoundingBox {
        x: parent.x + child.x * parent.width,
        y: parent.y + child_y * parent.height,
        width: child.width * parent.width,
        height: child.height * parent.height,
        unit: parent.unit,
        origin: parent.origin,
    }
}

fn ocr_locator(
    page_index: u64,
    bbox: BoundingBox,
    page: Option<&PdfPage>,
    confidence: f64,
) -> SourceLocator {
    SourceLocator::approximate(
        LocationComponent::PdfRegion {
            page: IndexPosition::one_based(page_index).expect("PDF pages are one based"),
            bbox: Some(bbox),
            rotation_degrees: page.map(|value| value.rotation_degrees),
            tokens: None,
        },
        LocatorConfidence::new(confidence).expect("OCR confidence is normalized"),
    )
    .expect("OCR PDF region locator is valid")
}

fn geometric_overlap(left: BoundingBox, right: BoundingBox, page: Option<&PdfPage>) -> Option<f64> {
    let left = normalized_box(left, page)?;
    let right = normalized_box(right, page)?;
    let x1 = left[0].max(right[0]);
    let y1 = left[1].max(right[1]);
    let x2 = (left[0] + left[2]).min(right[0] + right[2]);
    let y2 = (left[1] + left[3]).min(right[1] + right[3]);
    let intersection = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let smallest = (left[2] * left[3]).min(right[2] * right[3]);
    Some(if smallest > 0.0 {
        (intersection / smallest).clamp(0.0, 1.0)
    } else {
        0.0
    })
}

fn normalized_box(bbox: BoundingBox, page: Option<&PdfPage>) -> Option<[f64; 4]> {
    let (mut x, mut y, width, height) = match bbox.unit {
        CoordinateUnit::Normalized => (bbox.x, bbox.y, bbox.width, bbox.height),
        CoordinateUnit::Points => {
            let page = page?;
            if page.width_points <= 0.0 || page.height_points <= 0.0 {
                return None;
            }
            (
                bbox.x / page.width_points,
                bbox.y / page.height_points,
                bbox.width / page.width_points,
                bbox.height / page.height_points,
            )
        }
        CoordinateUnit::Pixels => return None,
    };
    if bbox.origin == CoordinateOrigin::BottomLeft {
        y = 1.0 - y - height;
    }
    x = x.clamp(0.0, 1.0);
    y = y.clamp(0.0, 1.0);
    Some([x, y, width.clamp(0.0, 1.0), height.clamp(0.0, 1.0)])
}

fn text_similarity(left: &str, right: &str) -> f64 {
    let left_normalized = normalize_text(left);
    let right_normalized = normalize_text(right);
    if left_normalized == right_normalized && !left_normalized.is_empty() {
        return 1.0;
    }
    let left_words = left_normalized.split_whitespace().collect::<BTreeSet<_>>();
    let right_words = right_normalized.split_whitespace().collect::<BTreeSet<_>>();
    let union = left_words.union(&right_words).count();
    if union == 0 {
        0.0
    } else {
        left_words.intersection(&right_words).count() as f64 / union as f64
    }
}

fn normalize_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn origin_rank(origin: PdfTextOrigin) -> u8 {
    match origin {
        PdfTextOrigin::Native => 0,
        PdfTextOrigin::Ocr => 1,
        PdfTextOrigin::Reconciled => 2,
    }
}

fn source_rank(source: &PdfReconciledTextSource) -> (u64, u64) {
    match source {
        PdfReconciledTextSource::Native { region_index } => (0, *region_index),
        PdfReconciledTextSource::Ocr {
            attempt_index,
            region_index,
        } => (attempt_index.saturating_add(1), *region_index),
    }
}

fn mean(values: impl IntoIterator<Item = f64>) -> f64 {
    let values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        0.0
    } else {
        (values.iter().sum::<f64>() / values.len() as f64).clamp(0.0, 1.0)
    }
}
