//! Explicit frame-scoped OCR and identity-bearing image text reconciliation.

use super::*;
use crate::core::{
    BoundingBox, CoordinateOrigin, CoordinateUnit, DeclaredLoss, Diagnostic, IndexPosition,
    LocationComponent, LocatorConfidence, LossClass, OperationKind, ProvenanceStep,
    ProviderInvocation, ProviderKind, SourceLocator, options_digest,
};
use crate::provider::{
    NativeRepresentation, OcrOptions, OcrRequest, OcrResult, ProviderRequest, ProviderResponse,
    ProviderResult, ReconciledRepresentation,
};
use crate::registry::ParserContext;
use serde_json::json;

const RECONCILIATION_ALGORITHM: &str = "grist.image.native-ocr-append";
const RECONCILIATION_VERSION: &str = "1";
const MAX_OCR_REGIONS_PER_SCOPE: u64 = 10_000;
const MAX_OCR_CHARACTERS_PER_SCOPE: u64 = 16 * 1024 * 1024;

pub(crate) struct ImageOcrOutcome {
    pub diagnostics: Vec<Diagnostic>,
    pub invocations: Vec<ProviderInvocation>,
    pub provenance: Vec<ProvenanceStep>,
}

pub(crate) fn native_text_content(embedded_text: &[ImageText]) -> Result<ImageTextContent, String> {
    let native = crate::provider::NativeRepresentation::new(ImageNativeText {
        text: embedded_text
            .iter()
            .map(|entry| entry.text.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        entries: embedded_text.to_vec(),
    })
    .map_err(|error| error.to_string())?;
    Ok(ImageTextContent {
        native,
        ocr_attempts: Vec::new(),
        reconciled: None,
    })
}

pub(super) fn canonical_native_text(
    document: &ImageDocument,
) -> Result<NativeRepresentation<ImageNativeText>, String> {
    let canonical = native_text_content(&document.embedded_text)?.native;
    if document.text.native.value == canonical.value
        && document.text.native.identity == canonical.identity
    {
        return Ok(canonical);
    }
    let legacy_default = document.text.native.value == ImageNativeText::default()
        && document.text.ocr_attempts.is_empty()
        && document.text.reconciled.is_none();
    if legacy_default {
        return Ok(canonical);
    }
    Err("image native text facts diverge from embedded_text".into())
}

pub(crate) fn apply_selected_ocr(
    context: &mut ParserContext<'_>,
    document: &mut ImageDocument,
    options: &ImageOptions,
) -> ImageOcrOutcome {
    let mut outcome = ImageOcrOutcome {
        diagnostics: Vec::new(),
        invocations: Vec::new(),
        provenance: Vec::new(),
    };
    if options.ocr.mode == ImageOcrMode::Disabled
        || context
            .selected_provider_network(ProviderKind::Ocr)
            .is_none()
    {
        return outcome;
    }

    let mut frames = document.frames.iter().collect::<Vec<_>>();
    if frames.len() as u64 > options.ocr.max_scopes {
        let omitted = frames.len() as u64 - options.ocr.max_scopes;
        frames.truncate(usize::try_from(options.ocr.max_scopes).unwrap_or(usize::MAX));
        outcome.diagnostics.push(
            Diagnostic::budget_exhausted(
                "grist.image",
                format!(
                    "image OCR scope limit {} omitted {omitted} frame request(s)",
                    options.ocr.max_scopes
                ),
            )
            .partial(),
        );
    }

    for frame in frames {
        let scope = ImageOcrScope {
            frame_index: frame.index,
            locator: frame.locator.clone(),
        };
        let configuration = json!({
            "adapter": "grist.image.ocr-frame",
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
        let response = match account_provider_output(context, &response, options.ocr.reconcile) {
            Ok(()) => response,
            Err(diagnostic) => rejected_response(
                response,
                diagnostic.with_locator(scope.locator.clone()).partial(),
            ),
        };
        outcome.invocations.push(response.envelope_invocation());
        outcome.diagnostics.extend(
            response
                .metadata
                .diagnostics
                .iter()
                .cloned()
                .map(|diagnostic| diagnostic.with_locator(scope.locator.clone()).partial()),
        );
        let attempt = build_attempt(frame, scope, response, &mut outcome.diagnostics);
        document.text.ocr_attempts.push(attempt);
    }

    if options.ocr.reconcile {
        match account_reconciliation(context, &document.text) {
            Ok(()) => {
                if let Some((reconciled, provenance)) = reconcile(&document.text, &options.ocr) {
                    document.text.reconciled = Some(reconciled);
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
        .map_err(|error| error.diagnostic("grist.image"))?;
    let Some(ProviderResult::Ocr(result)) = response.result() else {
        return Ok(());
    };
    let region_count = u64::try_from(result.regions.len()).unwrap_or(u64::MAX);
    let character_count = ocr_character_count(result);
    if region_count > MAX_OCR_REGIONS_PER_SCOPE {
        return Err(Diagnostic::budget_exhausted(
            "grist.image",
            format!(
                "OCR provider returned {region_count} regions; per-scope limit is {MAX_OCR_REGIONS_PER_SCOPE}"
            ),
        ));
    }
    if character_count > MAX_OCR_CHARACTERS_PER_SCOPE {
        return Err(Diagnostic::budget_exhausted(
            "grist.image",
            format!(
                "OCR provider returned {character_count} characters; per-scope limit is {MAX_OCR_CHARACTERS_PER_SCOPE}"
            ),
        ));
    }
    let projected_regions = if result.regions.is_empty() && !result.text.trim().is_empty() {
        1
    } else {
        region_count
    };
    let representation_factor = if reconcile { 3 } else { 2 };
    let memory_bytes = ocr_string_bytes(result)
        .saturating_mul(representation_factor)
        .saturating_add(projected_regions.saturating_mul(
            u64::try_from(std::mem::size_of::<ImageOcrTextRegion>()).unwrap_or(u64::MAX),
        ));
    context
        .observe_memory_bytes(memory_bytes)
        .map_err(|error| *error)?;
    context
        .consume_decoded_characters(character_count)
        .map_err(|error| *error)?;
    context
        .consume_nodes(projected_regions.saturating_add(1))
        .map_err(|error| *error)?;
    context
        .consume_records(projected_regions.saturating_add(1))
        .map_err(|error| *error)?;
    context
        .control()
        .checkpoint()
        .map_err(|error| error.diagnostic("grist.image"))
}

fn account_reconciliation(
    context: &ParserContext<'_>,
    text: &ImageTextContent,
) -> Result<(), Diagnostic> {
    context
        .control()
        .checkpoint()
        .map_err(|error| error.diagnostic("grist.image"))?;
    let successful = text
        .ocr_attempts
        .iter()
        .filter(|attempt| attempt.response.is_success())
        .collect::<Vec<_>>();
    if successful.is_empty() {
        return Ok(());
    }
    let provider_items = successful
        .iter()
        .map(|attempt| {
            if distinct_overall_text(attempt).is_some() {
                1
            } else {
                u64::try_from(attempt.regions.len()).unwrap_or(u64::MAX)
            }
        })
        .fold(0u64, u64::saturating_add);
    let native_items = u64::try_from(text.native.value.entries.len()).unwrap_or(u64::MAX);
    let nodes = native_items.saturating_add(provider_items);
    let native_bytes = text
        .native
        .value
        .entries
        .iter()
        .map(|entry| u64::try_from(entry.text.len()).unwrap_or(u64::MAX))
        .fold(0u64, u64::saturating_add);
    let provider_bytes = successful
        .iter()
        .map(|attempt| {
            distinct_overall_text(attempt).map_or_else(
                || {
                    attempt
                        .regions
                        .iter()
                        .map(|region| u64::try_from(region.text.len()).unwrap_or(u64::MAX))
                        .fold(0u64, u64::saturating_add)
                },
                |overall| u64::try_from(overall.len()).unwrap_or(u64::MAX),
            )
        })
        .fold(0u64, u64::saturating_add);
    let memory_bytes =
        native_bytes
            .saturating_add(provider_bytes)
            .saturating_add(nodes.saturating_mul(
                u64::try_from(std::mem::size_of::<ImageReconciledTextItem>()).unwrap_or(u64::MAX),
            ));
    context
        .observe_memory_bytes(memory_bytes)
        .map_err(|error| *error)?;
    context.consume_nodes(nodes).map_err(|error| *error)?;
    context.consume_records(nodes).map_err(|error| *error)?;
    context
        .control()
        .checkpoint()
        .map_err(|error| error.diagnostic("grist.image"))
}

fn ocr_character_count(result: &OcrResult) -> u64 {
    std::iter::once(result.text.chars().count())
        .chain(
            result
                .regions
                .iter()
                .map(|region| region.text.chars().count()),
        )
        .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
        .fold(0u64, u64::saturating_add)
}

fn ocr_string_bytes(result: &OcrResult) -> u64 {
    std::iter::once(result.text.len())
        .chain(result.regions.iter().map(|region| region.text.len()))
        .chain(
            result
                .regions
                .iter()
                .filter_map(|region| region.language.as_ref().map(String::len)),
        )
        .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
        .fold(0u64, u64::saturating_add)
}

fn rejected_response(response: ProviderResponse, diagnostic: Diagnostic) -> ProviderResponse {
    let mut metadata = response.metadata;
    metadata.output_identity = None;
    metadata.diagnostics.push(diagnostic);
    ProviderResponse::failed(metadata).expect("bounded OCR rejection metadata is valid")
}

fn build_attempt(
    frame: &ImageFrame,
    scope: ImageOcrScope,
    response: crate::provider::ProviderResponse,
    diagnostics: &mut Vec<Diagnostic>,
) -> ImageOcrAttempt {
    let result = match response.result() {
        Some(ProviderResult::Ocr(result)) => Some(result),
        _ => None,
    };
    let synthesized = result
        .as_ref()
        .filter(|result| result.regions.is_empty() && !result.text.trim().is_empty())
        .map(|result| crate::provider::OcrRegion {
            text: result.text.clone(),
            bounding_box: full_frame_bbox(frame),
            confidence: result.confidence,
            language: None,
            reading_order: None,
        });
    let regions = match result.as_ref() {
        Some(result) if !result.regions.is_empty() => result.regions.iter().collect::<Vec<_>>(),
        _ => synthesized.iter().collect(),
    };
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
        let bbox = match resolve_bbox(frame, region.bounding_box) {
            Ok((bbox, warning)) => {
                if let Some(message) = warning {
                    diagnostics.push(
                        Diagnostic::warning(
                            "grist.image",
                            "image.ocr.geometry_unit_unresolved",
                            message,
                        )
                        .with_locator(scope.locator.clone())
                        .partial(),
                    );
                }
                bbox
            }
            Err(message) => {
                diagnostics.push(
                    Diagnostic::warning(
                        "grist.image",
                        "image.ocr.region_geometry_invalid",
                        format!("OCR region {source_index} was not projected: {message}"),
                    )
                    .with_locator(scope.locator.clone())
                    .partial(),
                );
                continue;
            }
        };
        let confidence = region
            .confidence
            .or_else(|| result.as_ref().and_then(|value| value.layout_confidence))
            .or_else(|| result.as_ref().and_then(|value| value.confidence))
            .map_or(0.0, crate::provider::ProviderConfidence::get);
        projected.push(ImageOcrTextRegion {
            index: source_index as u64,
            text: region.text.clone(),
            bbox,
            confidence: region.confidence,
            language: region.language.clone(),
            reading_order: reading_order as u64,
            reading_order_inferred: region.reading_order.is_none(),
            locator: match ocr_locator(frame.index, bbox, confidence) {
                Ok(locator) => locator,
                Err(message) => {
                    diagnostics.push(
                        Diagnostic::warning(
                            "grist.image",
                            "image.ocr.region_locator_invalid",
                            format!("OCR region {source_index} was not projected: {message}"),
                        )
                        .with_locator(scope.locator.clone())
                        .partial(),
                    );
                    continue;
                }
            },
        });
    }
    projected.sort_by_key(|region| region.reading_order);

    if let Some(result) = &result {
        if result.reading_order_confidence.is_none() {
            diagnostics.push(
                Diagnostic::warning(
                    "grist.image",
                    "image.ocr.reading_order_confidence_missing",
                    "OCR provider omitted reading-order confidence",
                )
                .with_locator(scope.locator.clone())
                .partial(),
            );
        }
        if result.layout_confidence.is_none() {
            diagnostics.push(
                Diagnostic::warning(
                    "grist.image",
                    "image.ocr.layout_confidence_missing",
                    "OCR provider omitted layout confidence",
                )
                .with_locator(scope.locator.clone())
                .partial(),
            );
        }
    }
    let reading_order_confidence = result.and_then(|value| value.reading_order_confidence);
    let layout_confidence = result.and_then(|value| value.layout_confidence);
    ImageOcrAttempt {
        scope,
        response,
        regions: projected,
        reading_order_confidence,
        layout_confidence,
    }
}

fn reconcile(
    text: &ImageTextContent,
    options: &ImageOcrOptions,
) -> Option<(
    ReconciledRepresentation<ImageReconciledText>,
    ProvenanceStep,
)> {
    let successful = text
        .ocr_attempts
        .iter()
        .enumerate()
        .filter(|(_, attempt)| attempt.response.is_success())
        .collect::<Vec<_>>();
    if successful.is_empty() {
        return None;
    }
    let mut items = text
        .native
        .value
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| ImageReconciledTextItem {
            index: index as u64,
            text: entry.text.clone(),
            origin: ImageTextOrigin::Native,
            source: ImageReconciledTextSource::Native {
                entry_index: index as u64,
            },
            bbox: None,
            confidence: None,
            locator: entry.locator.clone(),
        })
        .collect::<Vec<_>>();
    let mut confidence_evidence = Vec::new();
    let mut structure_flattened = false;
    for (attempt_index, attempt) in successful {
        confidence_evidence.push(format!(
            "ocr attempt {attempt_index} provider={} model={} reading_order={} layout={}",
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
                .map(crate::provider::ProviderConfidence::get)
                .map_or_else(|| "missing".into(), |value| value.to_string()),
            attempt
                .layout_confidence
                .map(crate::provider::ProviderConfidence::get)
                .map_or_else(|| "missing".into(), |value| value.to_string()),
        ));
        if let Some(overall_text) = distinct_overall_text(attempt) {
            structure_flattened = true;
            let confidence = attempt
                .response
                .result()
                .and_then(|result| match result {
                    ProviderResult::Ocr(result) => result.confidence,
                    _ => None,
                })
                .or(attempt.layout_confidence);
            items.push(ImageReconciledTextItem {
                index: items.len() as u64,
                text: overall_text.to_string(),
                origin: ImageTextOrigin::Ocr,
                source: ImageReconciledTextSource::OcrOverall {
                    attempt_index: attempt_index as u64,
                },
                bbox: None,
                confidence,
                locator: attempt.scope.locator.clone(),
            });
        } else {
            for region in &attempt.regions {
                items.push(ImageReconciledTextItem {
                    index: items.len() as u64,
                    text: region.text.clone(),
                    origin: ImageTextOrigin::Ocr,
                    source: ImageReconciledTextSource::Ocr {
                        attempt_index: attempt_index as u64,
                        region_index: region.index,
                    },
                    bbox: Some(region.bbox),
                    confidence: region.confidence.or(attempt.layout_confidence),
                    locator: region.locator.clone(),
                });
            }
        }
    }
    let value = ImageReconciledText {
        text: items
            .iter()
            .map(|item| item.text.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        items,
        confidence_evidence,
    };
    let provider_output_identities = text
        .ocr_attempts
        .iter()
        .filter_map(|attempt| attempt.response.metadata.output_identity.clone())
        .collect::<Vec<_>>();
    let configuration_digest = options_digest(&json!({
        "algorithm": RECONCILIATION_ALGORITHM,
        "version": RECONCILIATION_VERSION,
        "reconcile": options.reconcile,
    }))
    .expect("image reconciliation options serialize");
    let representation = ReconciledRepresentation::new(
        value,
        RECONCILIATION_ALGORITHM,
        RECONCILIATION_VERSION,
        configuration_digest.clone(),
        text.native.identity.clone(),
        provider_output_identities,
    )
    .expect("successful image OCR identities satisfy reconciliation invariants");
    let provider = text
        .ocr_attempts
        .iter()
        .find(|attempt| attempt.response.is_success())
        .map(|attempt| attempt.response.metadata.provider.name.clone())
        .expect("successful image OCR has a provider");
    let declared_loss = if structure_flattened {
        DeclaredLoss::Lossy(LossClass::from(LossClass::STRUCTURE_FLATTENED))
    } else {
        DeclaredLoss::Lossless
    };
    let provenance = ProvenanceStep::new(
        OperationKind::Parse,
        format!("{RECONCILIATION_ALGORITHM}@{RECONCILIATION_VERSION}"),
        text.native.identity.clone(),
        representation.identity.clone(),
        configuration_digest,
        declared_loss,
    )
    .expect("image OCR reconciliation provenance is valid")
    .with_provider(provider);
    Some((representation, provenance))
}

fn full_frame_bbox(frame: &ImageFrame) -> BoundingBox {
    BoundingBox {
        x: f64::from(frame.x),
        y: f64::from(frame.y),
        width: f64::from(frame.dimensions.width),
        height: f64::from(frame.dimensions.height),
        unit: CoordinateUnit::Pixels,
        origin: CoordinateOrigin::TopLeft,
    }
}

pub(super) fn distinct_overall_text(attempt: &ImageOcrAttempt) -> Option<&str> {
    let ProviderResult::Ocr(result) = attempt.response.result()? else {
        return None;
    };
    if result.text.trim().is_empty() {
        return None;
    }
    (!projected_text_matches(attempt, &result.text)).then_some(result.text.as_str())
}

fn projected_text_matches(attempt: &ImageOcrAttempt, overall: &str) -> bool {
    let mut remaining = overall;
    let mut first = true;
    for text in attempt
        .regions
        .iter()
        .map(|region| region.text.as_str())
        .filter(|text| !text.trim().is_empty())
    {
        if !first {
            let Some(rest) = remaining.strip_prefix('\n') else {
                return false;
            };
            remaining = rest;
        }
        let Some(rest) = remaining.strip_prefix(text) else {
            return false;
        };
        remaining = rest;
        first = false;
    }
    remaining.is_empty()
}

fn resolve_bbox(
    frame: &ImageFrame,
    child: BoundingBox,
) -> Result<(BoundingBox, Option<String>), String> {
    let parent = full_frame_bbox(frame);
    match child.unit {
        CoordinateUnit::Normalized => {
            let child_y = if child.origin == parent.origin {
                child.y
            } else {
                1.0 - child.y - child.height
            };
            Ok((
                BoundingBox {
                    x: parent.x + child.x * parent.width,
                    y: parent.y + child_y * parent.height,
                    width: child.width * parent.width,
                    height: child.height * parent.height,
                    unit: parent.unit,
                    origin: parent.origin,
                },
                None,
            ))
        }
        CoordinateUnit::Pixels => {
            if child.x < 0.0 || child.y < 0.0 {
                return Err("pixel coordinates cannot be negative".into());
            }
            if child.x + child.width > parent.width || child.y + child.height > parent.height {
                return Err("pixel rectangle falls outside the scoped frame".into());
            }
            let child_y = if child.origin == parent.origin {
                child.y
            } else {
                parent.height - child.y - child.height
            };
            Ok((
                BoundingBox {
                    x: parent.x + child.x,
                    y: parent.y + child_y,
                    width: child.width,
                    height: child.height,
                    unit: parent.unit,
                    origin: parent.origin,
                },
                None,
            ))
        }
        CoordinateUnit::Points => {
            if child.x < 0.0 || child.y < 0.0 {
                return Err("point coordinates cannot be negative".into());
            }
            Ok((
                child,
                Some(
                    "OCR provider returned point coordinates; bounds cannot be resolved against a pixel frame without a declared scale"
                        .into(),
                ),
            ))
        }
    }
}

fn ocr_locator(
    frame_index: u64,
    bbox: BoundingBox,
    confidence: f64,
) -> Result<SourceLocator, String> {
    SourceLocator::approximate(
        LocationComponent::ImageRegion {
            frame: IndexPosition::zero_based(frame_index),
            bbox: Some(bbox),
        },
        LocatorConfidence::new(confidence).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}
