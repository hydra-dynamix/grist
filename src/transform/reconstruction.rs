//! Format-specific faithful package reconstruction boundary.

use crate::core::{
    ArtifactKind, CanonicalPayloadIdentity, ContentIdentity, DeclaredLoss, Diagnostic, Envelope,
    LossClass, OperationKind, ParserInfo, ProvenanceStep, SchemaVersion, SourceInfo,
    options_digest, sha256_hex,
};
use crate::document_graph::DocumentGraph;
#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub const FORMAT_RECONSTRUCTION_RESULT_V1: &str = SchemaVersion::FORMAT_RECONSTRUCTION_RESULT_V1;
pub const RECONSTRUCTION_FIDELITY_REPORT_V1: &str =
    SchemaVersion::RECONSTRUCTION_FIDELITY_REPORT_V1;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ReconstructionFidelity {
    Lossy,
    SemanticEquivalent,
    PackageEquivalent,
    ByteIdentical,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionFixtureEvidence {
    pub fixture_id: String,
    pub input_sha256: String,
    pub expected_package_sha256: String,
    pub verified_fidelity: ReconstructionFidelity,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FormatReconstructionClaim {
    pub format: String,
    pub media_type: String,
    pub package_profile: String,
    pub implementation: String,
    pub implementation_version: String,
    pub maximum_fidelity: ReconstructionFidelity,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub fixture_evidence: Vec<ReconstructionFixtureEvidence>,
}

impl FormatReconstructionClaim {
    pub fn validate(&self) -> Result<(), ReconstructionError> {
        for (field, value) in [
            ("format", self.format.as_str()),
            ("media_type", self.media_type.as_str()),
            ("package_profile", self.package_profile.as_str()),
            ("implementation", self.implementation.as_str()),
            (
                "implementation_version",
                self.implementation_version.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                return Err(ReconstructionError::InvalidClaim {
                    message: format!("{field} must not be empty"),
                });
            }
        }
        if self.fixture_evidence.is_empty() {
            return Err(ReconstructionError::InvalidClaim {
                message: "at least one fixture evidence record is required".to_string(),
            });
        }
        for fixture in &self.fixture_evidence {
            if fixture.fixture_id.trim().is_empty()
                || !is_sha256(&fixture.input_sha256)
                || !is_sha256(&fixture.expected_package_sha256)
                || fixture.verified_fidelity < self.maximum_fidelity
            {
                return Err(ReconstructionError::InvalidClaim {
                    message: format!("fixture evidence {} is incomplete", fixture.fixture_id),
                });
            }
        }
        Ok(())
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionOptions {
    pub required_fidelity: ReconstructionFidelity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_package_sha256: Option<String>,
}

impl Default for ReconstructionOptions {
    fn default() -> Self {
        Self {
            required_fidelity: ReconstructionFidelity::PackageEquivalent,
            expected_package_sha256: None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageByteRange {
    pub byte_start: usize,
    pub byte_end: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionSourceMapEntry {
    pub package_part: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<PackageByteRange>,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub source_node_ids: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconstructionDifferenceKind {
    Content,
    Structure,
    Formatting,
    Metadata,
    Ordering,
    Packaging,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionDifference {
    pub kind: ReconstructionDifferenceKind,
    pub message: String,
    #[serde(default)]
    pub affected_node_ids: Vec<String>,
    #[serde(default)]
    pub affected_package_parts: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionFidelityReport {
    pub schema_version: String,
    pub format: String,
    pub media_type: String,
    pub package_profile: String,
    pub implementation: String,
    pub implementation_version: String,
    pub requested_fidelity: ReconstructionFidelity,
    pub achieved_fidelity: ReconstructionFidelity,
    pub verification: String,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub fixture_evidence: Vec<ReconstructionFixtureEvidence>,
    #[serde(default)]
    pub differences: Vec<ReconstructionDifference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconstructionProduct {
    pub package_bytes: Vec<u8>,
    pub achieved_fidelity: ReconstructionFidelity,
    pub differences: Vec<ReconstructionDifference>,
    pub source_map: Vec<ReconstructionSourceMapEntry>,
}

pub trait FormatReconstructor {
    type Error: std::error::Error + Send + Sync + 'static;

    fn claim(&self) -> &FormatReconstructionClaim;
    fn reconstruct(
        &self,
        graph: &DocumentGraph,
        options: &ReconstructionOptions,
    ) -> Result<ReconstructionProduct, Self::Error>;
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionResult {
    pub schema_version: String,
    pub format: String,
    pub media_type: String,
    pub package_profile: String,
    pub package_sha256: String,
    pub package_bytes: Vec<u8>,
    #[serde(default)]
    pub source_map: Vec<ReconstructionSourceMapEntry>,
    pub fidelity_report: ReconstructionFidelityReport,
}

pub type ReconstructionEnvelope = crate::core::Envelope<ReconstructionResult>;

pub fn reconstruct_package<R: FormatReconstructor>(
    reconstructor: &R,
    graph: &DocumentGraph,
    options: &ReconstructionOptions,
) -> Result<Envelope<ReconstructionResult>, ReconstructionError> {
    graph
        .validate_contract()
        .map_err(|error| ReconstructionError::InvalidGraph {
            message: error.to_string(),
        })?;
    let claim = reconstructor.claim();
    claim.validate()?;
    validate_options(options, claim)?;
    let product = reconstructor.reconstruct(graph, options).map_err(|error| {
        ReconstructionError::Adapter {
            format: claim.format.clone(),
            message: error.to_string(),
        }
    })?;
    validate_product(graph, claim, options, &product)?;
    let package_sha256 = sha256_hex(&product.package_bytes);
    let verification = if product.achieved_fidelity == ReconstructionFidelity::ByteIdentical {
        format!("exact_raw_sha256_match:{package_sha256}")
    } else {
        "format_scoped_fixture_claim".to_string()
    };
    let report = ReconstructionFidelityReport {
        schema_version: RECONSTRUCTION_FIDELITY_REPORT_V1.to_string(),
        format: claim.format.clone(),
        media_type: claim.media_type.clone(),
        package_profile: claim.package_profile.clone(),
        implementation: claim.implementation.clone(),
        implementation_version: claim.implementation_version.clone(),
        requested_fidelity: options.required_fidelity,
        achieved_fidelity: product.achieved_fidelity,
        verification,
        fixture_evidence: claim.fixture_evidence.clone(),
        differences: product.differences,
    };
    let result = ReconstructionResult {
        schema_version: FORMAT_RECONSTRUCTION_RESULT_V1.to_string(),
        format: claim.format.clone(),
        media_type: claim.media_type.clone(),
        package_profile: claim.package_profile.clone(),
        package_sha256,
        package_bytes: product.package_bytes,
        source_map: product.source_map,
        fidelity_report: report,
    };
    reconstruction_envelope(graph, claim, options, result)
}

fn reconstruction_envelope(
    graph: &DocumentGraph,
    claim: &FormatReconstructionClaim,
    options: &ReconstructionOptions,
    result: ReconstructionResult,
) -> Result<Envelope<ReconstructionResult>, ReconstructionError> {
    let digest = options_digest(options).map_err(identity_error)?;
    let input = CanonicalPayloadIdentity::new(graph.schema_version.as_str(), graph)
        .map_err(identity_error)?;
    let output = CanonicalPayloadIdentity::new(FORMAT_RECONSTRUCTION_RESULT_V1, &result)
        .map_err(identity_error)?;
    let lossy = result.fidelity_report.achieved_fidelity == ReconstructionFidelity::Lossy;
    let loss = if lossy {
        DeclaredLoss::Lossy(LossClass::from(LossClass::CONTENT_OMITTED))
    } else {
        DeclaredLoss::Lossless
    };
    let provenance = ProvenanceStep::new(
        OperationKind::Transform,
        format!("{}@{}", claim.implementation, claim.implementation_version),
        input.sha256,
        output.sha256,
        digest.clone(),
        loss,
    )
    .map_err(|error| ReconstructionError::Provenance {
        message: error.to_string(),
    })?;
    let source = graph
        .source
        .clone()
        .unwrap_or_else(|| SourceInfo::new(graph.id.clone()));
    let identity = ContentIdentity::default()
        .with_canonical_payload(FORMAT_RECONSTRUCTION_RESULT_V1, &result)
        .map_err(identity_error)?;
    let parser = ParserInfo::new(&claim.implementation)
        .with_implementation(&claim.implementation, &claim.implementation_version);
    let mut envelope = if lossy {
        let affected_ids = result
            .fidelity_report
            .differences
            .iter()
            .flat_map(|difference| difference.affected_node_ids.iter().cloned())
            .collect();
        Envelope::partial(
            OperationKind::Transform,
            ArtifactKind::ReconstructionResult,
            source,
            parser,
            digest,
            FORMAT_RECONSTRUCTION_RESULT_V1,
            Some(result),
        )
        .with_diagnostics(vec![
            Diagnostic::lossy(
                &claim.implementation,
                "format reconstruction completed with declared losses",
            )
            .with_affected_ids(affected_ids),
        ])
    } else {
        Envelope::complete(
            OperationKind::Transform,
            ArtifactKind::ReconstructionResult,
            source,
            parser,
            digest,
            FORMAT_RECONSTRUCTION_RESULT_V1,
            result,
        )
    };
    envelope = envelope
        .with_identity(identity)
        .with_provenance(vec![provenance]);
    Ok(envelope)
}

fn validate_options(
    options: &ReconstructionOptions,
    claim: &FormatReconstructionClaim,
) -> Result<(), ReconstructionError> {
    if options.required_fidelity > claim.maximum_fidelity {
        return Err(ReconstructionError::UnsupportedFidelity {
            requested: options.required_fidelity,
            maximum: claim.maximum_fidelity,
        });
    }
    if let Some(hash) = options.expected_package_sha256.as_deref()
        && !is_sha256(hash)
    {
        return Err(ReconstructionError::InvalidExpectedIdentity);
    }
    if options.required_fidelity == ReconstructionFidelity::ByteIdentical
        && options.expected_package_sha256.is_none()
    {
        return Err(ReconstructionError::MissingExactIdentity);
    }
    Ok(())
}

fn validate_product(
    graph: &DocumentGraph,
    claim: &FormatReconstructionClaim,
    options: &ReconstructionOptions,
    product: &ReconstructionProduct,
) -> Result<(), ReconstructionError> {
    if product.achieved_fidelity > claim.maximum_fidelity {
        return Err(ReconstructionError::OverclaimedFidelity);
    }
    if product.achieved_fidelity < options.required_fidelity {
        return Err(ReconstructionError::FidelityNotMet {
            requested: options.required_fidelity,
            achieved: product.achieved_fidelity,
        });
    }
    if product.achieved_fidelity == ReconstructionFidelity::ByteIdentical {
        if !product.differences.is_empty() {
            return Err(ReconstructionError::ExactWithDifferences);
        }
        let expected = options
            .expected_package_sha256
            .as_deref()
            .ok_or(ReconstructionError::MissingExactIdentity)?;
        let actual = sha256_hex(&product.package_bytes);
        if actual != expected {
            return Err(ReconstructionError::ExactHashMismatch {
                expected: expected.to_string(),
                actual,
            });
        }
    }
    if product.achieved_fidelity == ReconstructionFidelity::Lossy && product.differences.is_empty()
    {
        return Err(ReconstructionError::LossyWithoutDifferences);
    }
    validate_reconstruction_source_map(graph, product)
}

fn validate_reconstruction_source_map(
    graph: &DocumentGraph,
    product: &ReconstructionProduct,
) -> Result<(), ReconstructionError> {
    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    for entry in &product.source_map {
        let invalid_range = entry.generated.is_some_and(|range| {
            range.byte_end <= range.byte_start || range.byte_end > product.package_bytes.len()
        });
        if entry.package_part.trim().is_empty()
            || entry.source_node_ids.is_empty()
            || entry
                .source_node_ids
                .iter()
                .any(|id| !node_ids.contains(id.as_str()))
            || invalid_range
        {
            return Err(ReconstructionError::InvalidSourceMap {
                package_part: entry.package_part.clone(),
            });
        }
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn identity_error(error: impl std::fmt::Display) -> ReconstructionError {
    ReconstructionError::Identity {
        message: error.to_string(),
    }
}

#[derive(Debug, Error)]
pub enum ReconstructionError {
    #[error("invalid document graph: {message}")]
    InvalidGraph { message: String },
    #[error("invalid format reconstruction claim: {message}")]
    InvalidClaim { message: String },
    #[error("requested fidelity {requested:?} exceeds claimed maximum {maximum:?}")]
    UnsupportedFidelity {
        requested: ReconstructionFidelity,
        maximum: ReconstructionFidelity,
    },
    #[error("byte-identical reconstruction requires an expected raw package identity")]
    MissingExactIdentity,
    #[error("expected package identity must be a lowercase SHA-256 digest")]
    InvalidExpectedIdentity,
    #[error("reconstructor reported fidelity above its fixture-backed claim")]
    OverclaimedFidelity,
    #[error("requested fidelity {requested:?} was not met; achieved {achieved:?}")]
    FidelityNotMet {
        requested: ReconstructionFidelity,
        achieved: ReconstructionFidelity,
    },
    #[error("byte-identical reconstruction cannot report differences")]
    ExactWithDifferences,
    #[error("byte-identical package hash mismatch: expected {expected}, actual {actual}")]
    ExactHashMismatch { expected: String, actual: String },
    #[error("lossy reconstruction must name at least one difference")]
    LossyWithoutDifferences,
    #[error("invalid reconstruction source map entry for package part {package_part}")]
    InvalidSourceMap { package_part: String },
    #[error("{format} reconstruction adapter failed: {message}")]
    Adapter { format: String, message: String },
    #[error("reconstruction identity failed: {message}")]
    Identity { message: String },
    #[error("reconstruction provenance failed: {message}")]
    Provenance { message: String },
}
