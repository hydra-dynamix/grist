//! Immutable citation anchors and bounded source-version verification.

use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

use super::{
    CellAddress, ContentIdentity, IndexBase, IndexPosition, LocationComponent, SourceInfo,
    SourceLocator, sha256_hex,
};

pub const CITATION_TEXT_NORMALIZATION_VERSION: &str = "grist/citation-text/v1";
pub const MAX_CITATION_EXCERPT_CHARS: usize = 512;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CitationTargetKind {
    Node,
    Segment,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceContentHash {
    RawSha256 { sha256: String },
    AggregateSha256 { sha256: String },
    DecodedSha256 { sha256: String },
}

impl SourceContentHash {
    pub fn from_identity(identity: &ContentIdentity) -> Option<Self> {
        identity
            .raw
            .as_ref()
            .map(|raw| Self::RawSha256 {
                sha256: raw.sha256.clone(),
            })
            .or_else(|| {
                identity
                    .aggregate
                    .as_ref()
                    .map(|aggregate| Self::AggregateSha256 {
                        sha256: aggregate.sha256.clone(),
                    })
            })
            .or_else(|| {
                identity
                    .decoded
                    .as_ref()
                    .map(|decoded| Self::DecodedSha256 {
                        sha256: decoded.sha256.clone(),
                    })
            })
    }

    pub fn sha256(&self) -> &str {
        match self {
            Self::RawSha256 { sha256 }
            | Self::AggregateSha256 { sha256 }
            | Self::DecodedSha256 { sha256 } => sha256,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CitationAnchorOptions {
    excerpt_max_chars: usize,
}

impl CitationAnchorOptions {
    pub fn new(excerpt_max_chars: usize) -> Result<Self, CitationAnchorError> {
        if excerpt_max_chars > MAX_CITATION_EXCERPT_CHARS {
            return Err(CitationAnchorError::ExcerptLimitExceeded {
                requested: excerpt_max_chars,
                maximum: MAX_CITATION_EXCERPT_CHARS,
            });
        }
        Ok(Self { excerpt_max_chars })
    }

    pub const fn excerpt_max_chars(self) -> usize {
        self.excerpt_max_chars
    }
}

impl Default for CitationAnchorOptions {
    fn default() -> Self {
        Self {
            excerpt_max_chars: 160,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CitationAnchor {
    pub schema_version: String,
    pub target_kind: CitationTargetKind,
    pub target_id: String,
    pub node_ids: Vec<String>,
    pub source: SourceInfo,
    pub source_identity: ContentIdentity,
    pub source_content_hash: SourceContentHash,
    pub locators: Vec<SourceLocator>,
    pub text_normalization: String,
    pub normalized_text_hash: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

impl CitationAnchor {
    #[allow(clippy::too_many_arguments)]
    pub fn for_node(
        source: SourceInfo,
        source_identity: ContentIdentity,
        node_id: impl Into<String>,
        locator: SourceLocator,
        text: &str,
        label: Option<String>,
        options: CitationAnchorOptions,
    ) -> Result<Self, CitationAnchorError> {
        let node_id = node_id.into();
        Self::new(
            CitationTargetKind::Node,
            node_id.clone(),
            vec![node_id],
            source,
            source_identity,
            vec![locator],
            text,
            label,
            options,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn for_segment(
        source: SourceInfo,
        source_identity: ContentIdentity,
        segment_id: impl Into<String>,
        ordered_node_ids: Vec<String>,
        locators: Vec<SourceLocator>,
        rendered_text: &str,
        label: Option<String>,
        options: CitationAnchorOptions,
    ) -> Result<Self, CitationAnchorError> {
        Self::new(
            CitationTargetKind::Segment,
            segment_id.into(),
            ordered_node_ids,
            source,
            source_identity,
            locators,
            rendered_text,
            label,
            options,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        target_kind: CitationTargetKind,
        target_id: String,
        node_ids: Vec<String>,
        source: SourceInfo,
        source_identity: ContentIdentity,
        locators: Vec<SourceLocator>,
        text: &str,
        label: Option<String>,
        options: CitationAnchorOptions,
    ) -> Result<Self, CitationAnchorError> {
        validate_target(&target_id, &node_ids, &locators)?;
        let source_content_hash = SourceContentHash::from_identity(&source_identity)
            .ok_or(CitationAnchorError::MissingSourceContentHash)?;
        let normalized = normalize_citation_text(text);
        let label = label
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| citation_label(&locators[0]));
        Ok(Self {
            schema_version: super::SchemaVersion::CITATION_ANCHOR_V1.to_string(),
            target_kind,
            target_id,
            node_ids,
            source,
            source_identity,
            source_content_hash,
            locators,
            text_normalization: CITATION_TEXT_NORMALIZATION_VERSION.to_string(),
            normalized_text_hash: sha256_hex(normalized.as_bytes()),
            label,
            excerpt: bounded_excerpt(&normalized, options.excerpt_max_chars),
        })
    }

    pub fn verify(
        &self,
        source_version: &CitationSourceVersion,
        options: CitationVerificationOptions,
    ) -> CitationVerification {
        verify_anchor(self, source_version, options)
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CitationCandidate {
    pub target_kind: CitationTargetKind,
    pub target_id: String,
    pub node_ids: Vec<String>,
    pub locators: Vec<SourceLocator>,
    pub normalized_text_hash: String,
}

impl CitationCandidate {
    pub fn from_anchor(anchor: &CitationAnchor) -> Self {
        Self {
            target_kind: anchor.target_kind,
            target_id: anchor.target_id.clone(),
            node_ids: anchor.node_ids.clone(),
            locators: anchor.locators.clone(),
            normalized_text_hash: anchor.normalized_text_hash.clone(),
        }
    }

    pub fn new(
        target_kind: CitationTargetKind,
        target_id: impl Into<String>,
        node_ids: Vec<String>,
        locators: Vec<SourceLocator>,
        text: &str,
    ) -> Result<Self, CitationAnchorError> {
        let target_id = target_id.into();
        validate_target(&target_id, &node_ids, &locators)?;
        Ok(Self {
            target_kind,
            target_id,
            node_ids,
            locators,
            normalized_text_hash: normalized_text_hash(text),
        })
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CitationSourceVersion {
    pub schema_version: String,
    pub source_identity: ContentIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_content_hash: Option<SourceContentHash>,
    pub candidates: Vec<CitationCandidate>,
}

impl CitationSourceVersion {
    pub fn new(source_identity: ContentIdentity, candidates: Vec<CitationCandidate>) -> Self {
        let source_content_hash = SourceContentHash::from_identity(&source_identity);
        Self {
            schema_version: super::SchemaVersion::CITATION_SOURCE_VERSION_V1.to_string(),
            source_identity,
            source_content_hash,
            candidates,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CitationVerificationOptions {
    pub max_candidates: usize,
}

impl Default for CitationVerificationOptions {
    fn default() -> Self {
        Self {
            max_candidates: 10_000,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CitationVerificationOutcome {
    Exact,
    Relocated,
    Changed,
    Missing,
    Unverifiable,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CitationVerificationMethod {
    SourceHash,
    StableTargetIdentity,
    OriginalLocator,
    BoundedContentMatch,
    NoMatch,
    SourceHashUnavailable,
    IdentityMismatch,
    CandidateBudgetExceeded,
    AmbiguousContentMatch,
    AmbiguousLocatorMatch,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CitationVerification {
    pub schema_version: String,
    pub outcome: CitationVerificationOutcome,
    pub method: CitationVerificationMethod,
    pub original_locators: Vec<SourceLocator>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub current_locators: Vec<SourceLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_target_id: Option<String>,
    pub original_source_content_hash: SourceContentHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_source_content_hash: Option<SourceContentHash>,
}

impl CitationVerification {
    fn result(
        anchor: &CitationAnchor,
        source_version: &CitationSourceVersion,
        outcome: CitationVerificationOutcome,
        method: CitationVerificationMethod,
        candidate: Option<&CitationCandidate>,
    ) -> Self {
        Self {
            schema_version: super::SchemaVersion::CITATION_VERIFICATION_V1.to_string(),
            outcome,
            method,
            original_locators: anchor.locators.clone(),
            current_locators: candidate.map_or_else(Vec::new, |value| value.locators.clone()),
            matched_target_id: candidate.map(|value| value.target_id.clone()),
            original_source_content_hash: anchor.source_content_hash.clone(),
            verified_source_content_hash: source_version.source_content_hash.clone(),
        }
    }
}

fn verify_anchor(
    anchor: &CitationAnchor,
    source_version: &CitationSourceVersion,
    options: CitationVerificationOptions,
) -> CitationVerification {
    if SourceContentHash::from_identity(&anchor.source_identity).as_ref()
        != Some(&anchor.source_content_hash)
        || SourceContentHash::from_identity(&source_version.source_identity)
            != source_version.source_content_hash
    {
        return CitationVerification::result(
            anchor,
            source_version,
            CitationVerificationOutcome::Unverifiable,
            CitationVerificationMethod::IdentityMismatch,
            None,
        );
    }
    let Some(version_hash) = source_version.source_content_hash.as_ref() else {
        return CitationVerification::result(
            anchor,
            source_version,
            CitationVerificationOutcome::Unverifiable,
            CitationVerificationMethod::SourceHashUnavailable,
            None,
        );
    };
    if version_hash == &anchor.source_content_hash {
        return CitationVerification::result(
            anchor,
            source_version,
            CitationVerificationOutcome::Exact,
            CitationVerificationMethod::SourceHash,
            None,
        );
    }
    if options.max_candidates == 0 || source_version.candidates.len() > options.max_candidates {
        return CitationVerification::result(
            anchor,
            source_version,
            CitationVerificationOutcome::Unverifiable,
            CitationVerificationMethod::CandidateBudgetExceeded,
            None,
        );
    }

    let relevant = source_version
        .candidates
        .iter()
        .filter(|candidate| candidate.target_kind == anchor.target_kind)
        .collect::<Vec<_>>();
    let stable = relevant
        .iter()
        .copied()
        .filter(|candidate| {
            candidate.target_id == anchor.target_id || candidate.node_ids == anchor.node_ids
        })
        .collect::<Vec<_>>();
    if stable.len() == 1 {
        return compare_candidate(
            anchor,
            source_version,
            stable[0],
            CitationVerificationMethod::StableTargetIdentity,
        );
    }

    let at_original = relevant
        .iter()
        .copied()
        .filter(|candidate| candidate.locators == anchor.locators)
        .collect::<Vec<_>>();
    if at_original.len() == 1 {
        return compare_candidate(
            anchor,
            source_version,
            at_original[0],
            CitationVerificationMethod::OriginalLocator,
        );
    }
    if at_original.len() > 1 {
        return CitationVerification::result(
            anchor,
            source_version,
            CitationVerificationOutcome::Unverifiable,
            CitationVerificationMethod::AmbiguousLocatorMatch,
            None,
        );
    }

    let content_matches = relevant
        .into_iter()
        .filter(|candidate| candidate.normalized_text_hash == anchor.normalized_text_hash)
        .collect::<Vec<_>>();
    match content_matches.as_slice() {
        [candidate] => CitationVerification::result(
            anchor,
            source_version,
            CitationVerificationOutcome::Relocated,
            CitationVerificationMethod::BoundedContentMatch,
            Some(candidate),
        ),
        [] => CitationVerification::result(
            anchor,
            source_version,
            CitationVerificationOutcome::Missing,
            CitationVerificationMethod::NoMatch,
            None,
        ),
        _ => CitationVerification::result(
            anchor,
            source_version,
            CitationVerificationOutcome::Unverifiable,
            CitationVerificationMethod::AmbiguousContentMatch,
            None,
        ),
    }
}

fn compare_candidate(
    anchor: &CitationAnchor,
    source_version: &CitationSourceVersion,
    candidate: &CitationCandidate,
    method: CitationVerificationMethod,
) -> CitationVerification {
    let outcome = if candidate.normalized_text_hash != anchor.normalized_text_hash {
        CitationVerificationOutcome::Changed
    } else if candidate.locators == anchor.locators {
        CitationVerificationOutcome::Exact
    } else {
        CitationVerificationOutcome::Relocated
    };
    CitationVerification::result(anchor, source_version, outcome, method, Some(candidate))
}

fn validate_target(
    target_id: &str,
    node_ids: &[String],
    locators: &[SourceLocator],
) -> Result<(), CitationAnchorError> {
    if target_id.trim().is_empty() {
        return Err(CitationAnchorError::EmptyTargetId);
    }
    if node_ids.is_empty() || node_ids.iter().any(|id| id.trim().is_empty()) {
        return Err(CitationAnchorError::InvalidNodeIds);
    }
    if locators.is_empty() {
        return Err(CitationAnchorError::MissingLocator);
    }
    for locator in locators {
        locator
            .validate()
            .map_err(|error| CitationAnchorError::InvalidLocator(error.to_string()))?;
    }
    Ok(())
}

/// Collapse every Unicode whitespace run to one ASCII space and trim both ends.
pub fn normalize_citation_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
        } else {
            if pending_space {
                normalized.push(' ');
                pending_space = false;
            }
            normalized.push(character);
        }
    }
    normalized
}

pub fn normalized_text_hash(text: &str) -> String {
    sha256_hex(normalize_citation_text(text).as_bytes())
}

fn bounded_excerpt(text: &str, maximum: usize) -> Option<String> {
    if maximum == 0 || text.is_empty() {
        return None;
    }
    if text.chars().count() <= maximum {
        return Some(text.to_string());
    }
    if maximum == 1 {
        return Some("\u{2026}".to_string());
    }
    let mut excerpt = text.chars().take(maximum - 1).collect::<String>();
    excerpt.push('\u{2026}');
    Some(excerpt)
}

pub fn citation_label(locator: &SourceLocator) -> String {
    let label = match locator.innermost() {
        LocationComponent::ByteRange {
            byte_start,
            byte_end,
        } => format!("bytes {byte_start}..{byte_end}"),
        LocationComponent::TextRange {
            start_line,
            end_line,
            ..
        } => {
            if start_line == end_line {
                format!("line {start_line}")
            } else {
                format!("lines {start_line}-{end_line}")
            }
        }
        LocationComponent::PdfRegion { page, .. } => format!("page {}", human_index(*page)),
        LocationComponent::OoxmlPart {
            part, paragraph, ..
        } => paragraph.map_or_else(
            || part.clone(),
            |position| format!("{part} paragraph {}", human_index(position)),
        ),
        LocationComponent::SlideRegion { slide, .. } => {
            format!("slide {}", human_index(*slide))
        }
        LocationComponent::SheetRange {
            sheet,
            start_cell,
            end_cell,
        } => format!(
            "{sheet}!{}:{}",
            cell_label(*start_cell),
            cell_label(*end_cell)
        ),
        LocationComponent::NotebookCell { index, .. } => {
            format!("cell {}", human_index(*index))
        }
        LocationComponent::EmailPart {
            header: Some(header),
            ..
        } => format!("header {header}"),
        LocationComponent::EmailPart { mime_path, .. } => {
            let path = mime_path
                .iter()
                .map(|position| human_index(*position).to_string())
                .collect::<Vec<_>>()
                .join(".");
            format!("MIME part {path}")
        }
        LocationComponent::ArchiveMember { member_path, .. } => member_path.clone(),
        LocationComponent::ImageRegion { frame, .. } => {
            format!("frame {}", human_index(*frame))
        }
        LocationComponent::MediaTime {
            start_ms, end_ms, ..
        } => format!("{start_ms}-{end_ms} ms"),
        LocationComponent::RecordRange {
            collection,
            records,
            field,
        } => field.as_ref().map_or_else(
            || format!("{collection}[{}..{}]", records.start, records.end),
            |field| format!("{collection}[{}..{}].{field}", records.start, records.end),
        ),
        LocationComponent::JsonPointer { pointer } => {
            if pointer.is_empty() {
                "/".to_string()
            } else {
                pointer.clone()
            }
        }
        LocationComponent::XmlPath { path } => path.clone(),
    };
    if locator.precision().name() == "approximate" {
        format!("{label} (approximate)")
    } else {
        label
    }
}

const fn human_index(position: IndexPosition) -> u64 {
    match position.base {
        IndexBase::Zero => position.value.saturating_add(1),
        IndexBase::One => position.value,
    }
}

fn cell_label(cell: CellAddress) -> String {
    let row = match cell.base {
        IndexBase::Zero => cell.row.saturating_add(1),
        IndexBase::One => cell.row,
    };
    let mut column = match cell.base {
        IndexBase::Zero => cell.column.saturating_add(1),
        IndexBase::One => cell.column,
    };
    let mut letters = Vec::new();
    while column > 0 {
        let remainder = ((column - 1) % 26) as u8;
        letters.push(char::from(b'A' + remainder));
        column = (column - 1) / 26;
    }
    letters.reverse();
    format!("{}{row}", letters.into_iter().collect::<String>())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CitationAnchorError {
    #[error("citation target ID must not be empty")]
    EmptyTargetId,
    #[error("citation node IDs must contain at least one non-empty ID")]
    InvalidNodeIds,
    #[error("a citation anchor must contain at least one source locator")]
    MissingLocator,
    #[error("citation source identity has no raw, aggregate, or decoded content hash")]
    MissingSourceContentHash,
    #[error("citation locator is invalid: {0}")]
    InvalidLocator(String),
    #[error("citation excerpt limit {requested} exceeds hard maximum {maximum}")]
    ExcerptLimitExceeded { requested: usize, maximum: usize },
}
