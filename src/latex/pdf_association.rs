//! Explicit association of parsed LaTeX source with a caller-supplied PDF.
//!
//! This module only compares already parsed representations. It never invokes
//! a TeX engine, searches for a PDF, reads an implicit path, or performs remote
//! activity.

use super::{LatexDocument, LatexEnvelope, LatexNode, LatexNodeKind};
use crate::core::{
    AggregateMemberIdentity, BudgetExceeded, BudgetSelection, CancellationToken, CitationAnchor,
    CitationAnchorError, CitationAnchorOptions, CitationSourceVersion, CitationVerification,
    CitationVerificationOptions, ContentIdentity, DeclaredLoss, Diagnostic, LocatorConfidence,
    LossClass, MetadataInvariantError, OperationControl, OperationControlError, OperationKind,
    OperationStatus, ParserInfo, ProvenanceStep, ResourceBudget, ResourceBudgetValidationError,
    SourceContentHash, SourceInfo, SourceLocator, canonical_json_bytes, normalize_citation_text,
    options_digest,
};
use crate::pdf::{PdfDocument, PdfEnvelope};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub const LATEX_PDF_ASSOCIATION_SCHEMA_VERSION: &str = "grist/latex-pdf-association/v1";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexPdfRepresentation {
    pub source: SourceInfo,
    pub identity: ContentIdentity,
    pub parser: ParserInfo,
    pub status: OperationStatus,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceStep>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LatexPdfCorrespondenceStatus {
    Exact,
    Partial,
    Ambiguous,
    Changed,
    Stale,
    Unmatched,
}

impl LatexPdfCorrespondenceStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Partial => "partial",
            Self::Ambiguous => "ambiguous",
            Self::Changed => "changed",
            Self::Stale => "stale",
            Self::Unmatched => "unmatched",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexPdfRenderedMatch {
    pub target_id: String,
    pub locator: SourceLocator,
    pub confidence: LocatorConfidence,
    pub citation: CitationAnchor,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexPdfCorrespondence {
    pub source_node_id: String,
    pub source_locator: SourceLocator,
    pub source_citation: CitationAnchor,
    pub status: LatexPdfCorrespondenceStatus,
    pub confidence: LocatorConfidence,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rendered_matches: Vec<LatexPdfRenderedMatch>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

impl LatexPdfCorrespondence {
    pub fn verify_source(
        &self,
        source_version: &CitationSourceVersion,
        options: CitationVerificationOptions,
    ) -> CitationVerification {
        self.source_citation.verify(source_version, options)
    }

    pub fn verify_rendered(
        &self,
        match_index: usize,
        source_version: &CitationSourceVersion,
        options: CitationVerificationOptions,
    ) -> Option<CitationVerification> {
        self.rendered_matches
            .get(match_index)
            .map(|matched| matched.citation.verify(source_version, options))
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexPdfUnmatchedRendered {
    pub target_id: String,
    pub locator: SourceLocator,
    pub citation: CitationAnchor,
}

/// A semantic link between two independently retained payloads.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexPdfAssociation {
    pub schema_version: String,
    pub identity: ContentIdentity,
    pub source_representation: LatexPdfRepresentation,
    pub rendered_representation: LatexPdfRepresentation,
    pub status: LatexPdfCorrespondenceStatus,
    pub caller_supplied_pdf: bool,
    pub correspondences: Vec<LatexPdfCorrespondence>,
    #[serde(default)]
    pub unmatched_rendered: Vec<LatexPdfUnmatchedRendered>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceStep>,
}

#[derive(Serialize)]
struct LatexPdfAssociationIdentityManifest<'a> {
    schema_version: &'static str,
    source_identity: &'a ContentIdentity,
    rendered_identity: &'a ContentIdentity,
    status: LatexPdfCorrespondenceStatus,
    correspondences: &'a [LatexPdfCorrespondence],
    unmatched_rendered: &'a [LatexPdfUnmatchedRendered],
    options_digest: &'a str,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LatexPdfAssociationOptions {
    pub budget: ResourceBudget,
    pub partial_match_threshold: LocatorConfidence,
    pub changed_match_threshold: LocatorConfidence,
    pub citation: CitationAnchorOptions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_source_identity: Option<ContentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_rendered_identity: Option<ContentIdentity>,
}

impl Default for LatexPdfAssociationOptions {
    fn default() -> Self {
        Self {
            budget: ResourceBudget::untrusted_service_v1(),
            partial_match_threshold: LocatorConfidence::new(0.72).expect("bounded threshold"),
            changed_match_threshold: LocatorConfidence::new(0.25).expect("bounded threshold"),
            citation: CitationAnchorOptions::default(),
            expected_source_identity: None,
            expected_rendered_identity: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LatexPdfAssociationError {
    #[error("LaTeX envelope has no parsed payload")]
    MissingLatexPayload,
    #[error("caller-supplied PDF envelope has no parsed payload")]
    MissingPdfPayload,
    #[error("LaTeX envelope has no independently verifiable content identity")]
    MissingLatexIdentity,
    #[error("caller-supplied PDF envelope has no independently verifiable content identity")]
    MissingPdfIdentity,
    #[error("changed-match threshold must not exceed partial-match threshold")]
    InvalidThresholdOrder,
    #[error("expected association identity has no independently verifiable content hash")]
    InvalidExpectedIdentity,
    #[error(transparent)]
    InvalidBudget(#[from] ResourceBudgetValidationError),
    #[error(transparent)]
    BudgetExceeded(#[from] BudgetExceeded),
    #[error(transparent)]
    Control(#[from] OperationControlError),
    #[error(transparent)]
    Citation(#[from] CitationAnchorError),
    #[error(transparent)]
    Provenance(#[from] MetadataInvariantError),
    #[error("association metadata could not be canonicalized: {0}")]
    Canonicalization(String),
}

/// Associates parsed LaTeX with an explicitly supplied, already parsed PDF.
///
/// Visible LaTeX semantic nodes are matched to PDF semantic blocks, falling
/// back to native PDF text blocks when semantic blocks are unavailable.
pub fn associate_latex_pdf(
    latex: &LatexEnvelope,
    pdf: &PdfEnvelope,
    options: &LatexPdfAssociationOptions,
) -> Result<LatexPdfAssociation, LatexPdfAssociationError> {
    options.budget.validate()?;
    let control = OperationControl::new(
        &BudgetSelection::custom(options.budget.clone()),
        CancellationToken::default(),
    )?;
    associate_latex_pdf_with_control(latex, pdf, options, &control)
}

/// Associates parsed representations under caller-owned shared cancellation
/// and budget control.
pub fn associate_latex_pdf_with_control(
    latex: &LatexEnvelope,
    pdf: &PdfEnvelope,
    options: &LatexPdfAssociationOptions,
    control: &OperationControl,
) -> Result<LatexPdfAssociation, LatexPdfAssociationError> {
    options.budget.validate()?;
    if options.changed_match_threshold.get() > options.partial_match_threshold.get() {
        return Err(LatexPdfAssociationError::InvalidThresholdOrder);
    }
    control.checkpoint()?;
    let latex_document = latex
        .payload
        .as_ref()
        .ok_or(LatexPdfAssociationError::MissingLatexPayload)?;
    let pdf_document = pdf
        .payload
        .as_ref()
        .ok_or(LatexPdfAssociationError::MissingPdfPayload)?;
    let latex_identity = latex
        .identity
        .as_ref()
        .filter(|identity| SourceContentHash::from_identity(identity).is_some())
        .ok_or(LatexPdfAssociationError::MissingLatexIdentity)?;
    let pdf_identity = pdf
        .identity
        .as_ref()
        .filter(|identity| SourceContentHash::from_identity(identity).is_some())
        .ok_or(LatexPdfAssociationError::MissingPdfIdentity)?;

    control
        .budget()
        .consume_pages(pdf_document.pages.len() as u64)?;
    let source_candidates = source_candidates(latex_document, &latex.source, latex_identity);
    let rendered_candidates = rendered_candidates(pdf_document, &pdf.source, pdf_identity);
    control.budget().consume_nodes(
        source_candidates
            .len()
            .saturating_add(rendered_candidates.len()) as u64,
    )?;
    let comparison_count = u64::try_from(source_candidates.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(rendered_candidates.len()).unwrap_or(u64::MAX));
    control.budget().consume_records(comparison_count)?;
    let decoded_characters = source_candidates
        .iter()
        .chain(&rendered_candidates)
        .map(|candidate| u64::try_from(candidate.text.chars().count()).unwrap_or(u64::MAX))
        .fold(0u64, u64::saturating_add);
    control
        .budget()
        .consume_decoded_characters(decoded_characters)?;
    let retained_text_bytes = source_candidates
        .iter()
        .chain(&rendered_candidates)
        .map(|candidate| {
            u64::try_from(
                candidate
                    .text
                    .len()
                    .saturating_add(candidate.normalized.len()),
            )
            .unwrap_or(u64::MAX)
        })
        .fold(0u64, u64::saturating_add);
    let candidate_count = u64::try_from(
        source_candidates
            .len()
            .saturating_add(rendered_candidates.len()),
    )
    .unwrap_or(u64::MAX);
    let scored_peak = u64::try_from(rendered_candidates.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(
            u64::try_from(std::mem::size_of::<(usize, f64, bool)>()).unwrap_or(u64::MAX),
        );
    control.budget().observe_memory_bytes(
        retained_text_bytes
            .saturating_mul(3)
            .saturating_add(scored_peak)
            .saturating_add(candidate_count.saturating_mul(1_024)),
    )?;
    let stale_evidence = stale_evidence(latex_identity, pdf_identity, options)?;

    let mut used_rendered = BTreeSet::new();
    let mut correspondences = Vec::with_capacity(source_candidates.len());
    for source in &source_candidates {
        control.checkpoint()?;
        let scored = score_rendered(source, &rendered_candidates, &used_rendered);
        let (mut status, confidence, selected, mut evidence) = classify_matches(scored, options);
        if let Some(stale) = &stale_evidence {
            status = LatexPdfCorrespondenceStatus::Stale;
            evidence.push(stale.clone());
        }
        let source_citation = citation(
            source.source.clone(),
            source.identity.clone(),
            source.id.clone(),
            source.locator.clone(),
            &source.text,
            options.citation,
        )?;
        let mut rendered_matches = Vec::with_capacity(selected.len());
        for (rendered_index, score) in selected {
            used_rendered.insert(rendered_index);
            let rendered = &rendered_candidates[rendered_index];
            rendered_matches.push(LatexPdfRenderedMatch {
                target_id: rendered.id.clone(),
                locator: rendered.locator.clone(),
                confidence: LocatorConfidence::new(score).expect("similarity is bounded"),
                citation: citation(
                    rendered.source.clone(),
                    rendered.identity.clone(),
                    rendered.id.clone(),
                    rendered.locator.clone(),
                    &rendered.text,
                    options.citation,
                )?,
            });
        }
        correspondences.push(LatexPdfCorrespondence {
            source_node_id: source.id.clone(),
            source_locator: source.locator.clone(),
            source_citation,
            status,
            confidence: LocatorConfidence::new(confidence).expect("similarity is bounded"),
            rendered_matches,
            evidence,
        });
    }
    control.checkpoint()?;

    let unmatched_rendered = rendered_candidates
        .iter()
        .enumerate()
        .filter(|(index, _)| !used_rendered.contains(index))
        .map(|(_, rendered)| {
            Ok(LatexPdfUnmatchedRendered {
                target_id: rendered.id.clone(),
                locator: rendered.locator.clone(),
                citation: citation(
                    rendered.source.clone(),
                    rendered.identity.clone(),
                    rendered.id.clone(),
                    rendered.locator.clone(),
                    &rendered.text,
                    options.citation,
                )?,
            })
        })
        .collect::<Result<Vec<_>, LatexPdfAssociationError>>()?;

    let digest = options_digest(options)
        .map_err(|error| LatexPdfAssociationError::Canonicalization(error.to_string()))?;
    let status = aggregate_status(
        &correspondences,
        &unmatched_rendered,
        latex.status,
        pdf.status,
        stale_evidence.is_some(),
    );
    let identity_manifest = LatexPdfAssociationIdentityManifest {
        schema_version: LATEX_PDF_ASSOCIATION_SCHEMA_VERSION,
        source_identity: latex_identity,
        rendered_identity: pdf_identity,
        status,
        correspondences: &correspondences,
        unmatched_rendered: &unmatched_rendered,
        options_digest: &digest,
    };
    let identity = ContentIdentity::default()
        .with_canonical_payload(LATEX_PDF_ASSOCIATION_SCHEMA_VERSION, &identity_manifest)
        .map_err(|error| LatexPdfAssociationError::Canonicalization(error.to_string()))?;
    let peer_identity = ContentIdentity::for_compound(vec![
        AggregateMemberIdentity::new("latex-source", Some(0), latex_identity),
        AggregateMemberIdentity::new("rendered-pdf", Some(1), pdf_identity),
    ])
    .map_err(|error| LatexPdfAssociationError::Canonicalization(error.to_string()))?;
    let peer_hash = peer_identity
        .aggregate
        .as_ref()
        .expect("compound peer identity has aggregate hash")
        .sha256
        .clone();
    let output_hash = identity
        .canonical_payload
        .as_ref()
        .expect("association identity has canonical payload hash")
        .sha256
        .clone();
    let declared_loss = if status == LatexPdfCorrespondenceStatus::Exact
        && latex.status == OperationStatus::Complete
        && pdf.status == OperationStatus::Complete
    {
        DeclaredLoss::Lossless
    } else {
        DeclaredLoss::Lossy(LossClass::from(LossClass::PRECISION_REDUCED))
    };
    let link_provenance = ProvenanceStep::new(
        OperationKind::Validate,
        "grist.latex.pdf-association.v1",
        peer_hash,
        output_hash,
        digest.clone(),
        declared_loss,
    )?
    .with_warning("latex.pdf.caller_supplied_peer_inputs")
    .with_warning(format!("latex.pdf.association_status.{}", status.as_str()));

    let association = LatexPdfAssociation {
        schema_version: LATEX_PDF_ASSOCIATION_SCHEMA_VERSION.to_string(),
        identity,
        source_representation: representation(latex, latex_identity),
        rendered_representation: representation(pdf, pdf_identity),
        status,
        caller_supplied_pdf: true,
        correspondences,
        unmatched_rendered,
        provenance: vec![link_provenance],
    };
    let output_bytes = u64::try_from(
        canonical_json_bytes(&association)
            .map_err(|error| LatexPdfAssociationError::Canonicalization(error.to_string()))?
            .len(),
    )
    .unwrap_or(u64::MAX);
    control.budget().consume_output_bytes(output_bytes)?;
    let retained_memory = control.budget().snapshot().memory_bytes;
    control
        .budget()
        .observe_memory_bytes(retained_memory.saturating_add(output_bytes))?;
    control.checkpoint()?;
    Ok(association)
}

fn representation<T>(
    envelope: &crate::core::Envelope<T>,
    identity: &ContentIdentity,
) -> LatexPdfRepresentation {
    LatexPdfRepresentation {
        source: envelope.source.clone(),
        identity: identity.clone(),
        parser: envelope.parser.clone(),
        status: envelope.status,
        diagnostics: envelope.diagnostics.clone(),
        provenance: resolved_provenance(envelope, identity),
    }
}

fn resolved_provenance<T>(
    envelope: &crate::core::Envelope<T>,
    identity: &ContentIdentity,
) -> Vec<ProvenanceStep> {
    let mut provenance = envelope.provenance.clone();
    if let Some(root) = provenance.first_mut() {
        if root.input_identity.is_none() {
            root.input_identity =
                SourceContentHash::from_identity(identity).map(|hash| hash.sha256().to_string());
        }
        if root.output_identity.is_none() {
            root.output_identity = identity
                .canonical_payload
                .as_ref()
                .map(|payload| payload.sha256.clone())
                .or_else(|| {
                    SourceContentHash::from_identity(identity).map(|hash| hash.sha256().to_string())
                });
        }
    }
    provenance
}

fn citation(
    source: SourceInfo,
    identity: ContentIdentity,
    target_id: String,
    locator: SourceLocator,
    text: &str,
    options: CitationAnchorOptions,
) -> Result<CitationAnchor, CitationAnchorError> {
    CitationAnchor::for_node(source, identity, target_id, locator, text, None, options)
}

#[derive(Clone)]
struct AssociationCandidate {
    id: String,
    source: SourceInfo,
    identity: ContentIdentity,
    locator: SourceLocator,
    text: String,
    normalized: String,
}

fn source_candidates(
    document: &LatexDocument,
    root_source: &SourceInfo,
    root_identity: &ContentIdentity,
) -> Vec<AssociationCandidate> {
    let mut candidates = Vec::new();
    collect_source_nodes(&document.nodes, root_source, root_identity, &mut candidates);
    candidates.sort_by(|left, right| left.id.cmp(&right.id));
    candidates
}

fn collect_source_nodes(
    nodes: &[LatexNode],
    default_source: &SourceInfo,
    source_identity: &ContentIdentity,
    candidates: &mut Vec<AssociationCandidate>,
) {
    for node in nodes {
        let shadowed_paragraph = node.kind == LatexNodeKind::Paragraph
            && nodes.iter().any(|other| {
                other.id != node.id
                    && other.kind != LatexNodeKind::Paragraph
                    && other.range.byte_start >= node.range.byte_start
                    && other.range.byte_end <= node.range.byte_end
            });
        if !shadowed_paragraph
            && is_visible_source_kind(&node.kind)
            && let (Some(locator), Some(text)) = (&node.locator, node.text.as_deref())
        {
            let visible = latex_visible_text(text);
            let normalized = association_normalize(&visible);
            if !normalized.is_empty() {
                candidates.push(AssociationCandidate {
                    id: node.id.clone(),
                    source: node
                        .source
                        .clone()
                        .unwrap_or_else(|| default_source.clone()),
                    identity: source_identity.clone(),
                    locator: locator.clone(),
                    text: visible,
                    normalized,
                });
            }
        }
        if let Some(resolved) = node
            .include
            .as_ref()
            .and_then(|include| include.resolved.as_ref())
        {
            let resolved_identity = ContentIdentity::for_raw_bytes(&resolved.raw_bytes)
                .with_decoded(
                    &resolved.decoded_text,
                    resolved.encoding.label(),
                    resolved.decoding.is_lossy(),
                );
            collect_source_nodes(
                &resolved.nodes,
                &resolved.source,
                &resolved_identity,
                candidates,
            );
        }
    }
}

fn is_visible_source_kind(kind: &LatexNodeKind) -> bool {
    matches!(
        kind,
        LatexNodeKind::Section
            | LatexNodeKind::Paragraph
            | LatexNodeKind::Text
            | LatexNodeKind::MacroUse
            | LatexNodeKind::ListItem
            | LatexNodeKind::TableCell
            | LatexNodeKind::Caption
            | LatexNodeKind::Equation
    )
}

fn rendered_candidates(
    document: &PdfDocument,
    source: &SourceInfo,
    identity: &ContentIdentity,
) -> Vec<AssociationCandidate> {
    let mut candidates = Vec::new();
    for page in &document.semantic_structure.pages {
        for block in &page.blocks {
            let normalized = association_normalize(&block.text);
            if !normalized.is_empty() {
                candidates.push(AssociationCandidate {
                    id: format!(
                        "pdf:semantic:page:{}:block:{}",
                        page.page_index, block.index
                    ),
                    source: source.clone(),
                    identity: identity.clone(),
                    locator: block.locator.clone(),
                    text: block.text.clone(),
                    normalized,
                });
            }
        }
    }
    if candidates.is_empty() {
        for page in &document.native_layout.pages {
            for block in &page.blocks {
                let normalized = association_normalize(&block.text);
                if !normalized.is_empty() {
                    candidates.push(AssociationCandidate {
                        id: format!("pdf:native:page:{}:block:{}", page.page_index, block.index),
                        source: source.clone(),
                        identity: identity.clone(),
                        locator: block.locator.clone(),
                        text: block.text.clone(),
                        normalized,
                    });
                }
            }
        }
    }
    candidates.sort_by(|left, right| left.id.cmp(&right.id));
    candidates
}

fn score_rendered(
    source: &AssociationCandidate,
    rendered: &[AssociationCandidate],
    used_rendered: &BTreeSet<usize>,
) -> Vec<(usize, f64, bool)> {
    let mut scored = rendered
        .iter()
        .enumerate()
        .filter(|(index, _)| !used_rendered.contains(index))
        .map(|(index, candidate)| {
            let exact = source.normalized == candidate.normalized;
            let score = if exact {
                1.0
            } else {
                text_similarity(&source.normalized, &candidate.normalized).min(0.999_999)
            };
            (index, score, exact)
        })
        .collect::<Vec<_>>();
    scored.sort_by(
        |(left_index, left_score, left_exact), (right_index, right_score, right_exact)| {
            right_exact
                .cmp(left_exact)
                .then_with(|| right_score.total_cmp(left_score))
                .then_with(|| rendered[*left_index].id.cmp(&rendered[*right_index].id))
        },
    );
    scored
}

fn classify_matches(
    scored: Vec<(usize, f64, bool)>,
    options: &LatexPdfAssociationOptions,
) -> (
    LatexPdfCorrespondenceStatus,
    f64,
    Vec<(usize, f64)>,
    Vec<String>,
) {
    let Some((_, best, best_exact)) = scored.first().copied() else {
        return (
            LatexPdfCorrespondenceStatus::Unmatched,
            0.0,
            Vec::new(),
            vec!["no rendered text candidates were available".into()],
        );
    };
    if best < options.changed_match_threshold.get() {
        return (
            LatexPdfCorrespondenceStatus::Unmatched,
            0.0,
            Vec::new(),
            vec![format!(
                "best normalized token similarity {best:.6} was below changed threshold {:.6}",
                options.changed_match_threshold.get()
            )],
        );
    }
    let selected = scored
        .into_iter()
        .take_while(|(_, score, exact)| {
            *exact == best_exact && (*score - best).abs() <= f64::EPSILON
        })
        .map(|(index, score, _)| (index, score))
        .collect::<Vec<_>>();
    if selected.len() > 1 {
        let count = selected.len();
        let confidence = (best / count as f64).clamp(0.0, 1.0);
        return (
            LatexPdfCorrespondenceStatus::Ambiguous,
            confidence,
            selected,
            vec![format!(
                "{count} rendered candidates tied at normalized token similarity {best:.6}"
            )],
        );
    }
    let status = if best_exact {
        LatexPdfCorrespondenceStatus::Exact
    } else if best >= options.partial_match_threshold.get() {
        LatexPdfCorrespondenceStatus::Partial
    } else {
        LatexPdfCorrespondenceStatus::Changed
    };
    (
        status,
        best,
        selected,
        vec![format!("normalized token similarity {best:.6}")],
    )
}

fn aggregate_status(
    correspondences: &[LatexPdfCorrespondence],
    unmatched_rendered: &[LatexPdfUnmatchedRendered],
    latex_status: OperationStatus,
    pdf_status: OperationStatus,
    stale: bool,
) -> LatexPdfCorrespondenceStatus {
    if stale
        || correspondences
            .iter()
            .any(|item| item.status == LatexPdfCorrespondenceStatus::Stale)
    {
        return LatexPdfCorrespondenceStatus::Stale;
    }
    if correspondences.is_empty()
        || correspondences
            .iter()
            .all(|item| item.status == LatexPdfCorrespondenceStatus::Unmatched)
    {
        return LatexPdfCorrespondenceStatus::Unmatched;
    }
    if correspondences
        .iter()
        .any(|item| item.status == LatexPdfCorrespondenceStatus::Ambiguous)
    {
        return LatexPdfCorrespondenceStatus::Ambiguous;
    }
    if correspondences
        .iter()
        .any(|item| item.status == LatexPdfCorrespondenceStatus::Changed)
    {
        return LatexPdfCorrespondenceStatus::Changed;
    }
    if !unmatched_rendered.is_empty()
        || latex_status != OperationStatus::Complete
        || pdf_status != OperationStatus::Complete
        || correspondences
            .iter()
            .any(|item| item.status != LatexPdfCorrespondenceStatus::Exact)
    {
        return LatexPdfCorrespondenceStatus::Partial;
    }
    LatexPdfCorrespondenceStatus::Exact
}

fn stale_evidence(
    source: &ContentIdentity,
    rendered: &ContentIdentity,
    options: &LatexPdfAssociationOptions,
) -> Result<Option<String>, LatexPdfAssociationError> {
    let current_source = SourceContentHash::from_identity(source)
        .expect("current source identity checked before stale comparison")
        .sha256()
        .to_string();
    let current_rendered = SourceContentHash::from_identity(rendered)
        .expect("current rendered identity checked before stale comparison")
        .sha256()
        .to_string();
    let expected_source = options
        .expected_source_identity
        .as_ref()
        .map(|identity| {
            SourceContentHash::from_identity(identity)
                .map(|hash| hash.sha256().to_string())
                .ok_or(LatexPdfAssociationError::InvalidExpectedIdentity)
        })
        .transpose()?;
    let expected_rendered = options
        .expected_rendered_identity
        .as_ref()
        .map(|identity| {
            SourceContentHash::from_identity(identity)
                .map(|hash| hash.sha256().to_string())
                .ok_or(LatexPdfAssociationError::InvalidExpectedIdentity)
        })
        .transpose()?;
    let source_changed = expected_source
        .as_deref()
        .is_some_and(|expected| expected != current_source.as_str());
    let rendered_changed = expected_rendered
        .as_deref()
        .is_some_and(|expected| expected != current_rendered.as_str());
    Ok((source_changed || rendered_changed).then(|| {
        format!(
            "association input identity is stale: source_changed={source_changed}, rendered_changed={rendered_changed}"
        )
    }))
}

fn latex_visible_text(text: &str) -> String {
    let mut visible = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '%' => break,
            '\\' => {
                while characters
                    .peek()
                    .is_some_and(|next| next.is_ascii_alphabetic())
                {
                    characters.next();
                }
                visible.push(' ');
            }
            '{' | '}' | '[' | ']' | '~' => visible.push(' '),
            '$' => {}
            other => visible.push(other),
        }
    }
    normalize_citation_text(&visible)
}

fn association_normalize(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_alphanumeric() {
            normalized.extend(character.to_lowercase());
        } else {
            normalized.push(' ');
        }
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_similarity(left: &str, right: &str) -> f64 {
    if left == right {
        return 1.0;
    }
    let counts = |value: &str| {
        let mut counts = BTreeMap::<String, usize>::new();
        for token in value.split_whitespace() {
            *counts.entry(token.to_string()).or_default() += 1;
        }
        counts
    };
    let left_counts = counts(left);
    let right_counts = counts(right);
    let intersection = left_counts
        .iter()
        .map(|(token, count)| count.min(right_counts.get(token).unwrap_or(&0)))
        .sum::<usize>();
    let total = left.split_whitespace().count() + right.split_whitespace().count();
    if total == 0 {
        0.0
    } else {
        (2 * intersection) as f64 / total as f64
    }
}
