//! Machine-readable parser-promotion report contracts.

use crate::registry::ParserDescriptor;
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub const PARSER_CONFORMANCE_REPORT_SCHEMA_VERSION: &str = "grist/parser-conformance-report/v1";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PromotionGate {
    Detection,
    ConstructPreservation,
    SourceLocators,
    OperationStatuses,
    Determinism,
    DocumentGraphProjection,
    Segmentation,
    PublicSurfaces,
    HostileInputSafety,
    FixtureCoverage,
    EnabledFeatureCi,
}

impl PromotionGate {
    pub const ALL: [Self; 11] = [
        Self::Detection,
        Self::ConstructPreservation,
        Self::SourceLocators,
        Self::OperationStatuses,
        Self::Determinism,
        Self::DocumentGraphProjection,
        Self::Segmentation,
        Self::PublicSurfaces,
        Self::HostileInputSafety,
        Self::FixtureCoverage,
        Self::EnabledFeatureCi,
    ];

    pub const fn number(self) -> u8 {
        match self {
            Self::Detection => 1,
            Self::ConstructPreservation => 2,
            Self::SourceLocators => 3,
            Self::OperationStatuses => 4,
            Self::Determinism => 5,
            Self::DocumentGraphProjection => 6,
            Self::Segmentation => 7,
            Self::PublicSurfaces => 8,
            Self::HostileInputSafety => 9,
            Self::FixtureCoverage => 10,
            Self::EnabledFeatureCi => 11,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Passed,
    Failed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionCheck {
    pub code: String,
    pub passed: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_id: Option<String>,
}

impl PromotionCheck {
    pub(crate) fn new(
        code: impl Into<String>,
        passed: bool,
        message: impl Into<String>,
        case_id: Option<&str>,
    ) -> Self {
        Self {
            code: code.into(),
            passed,
            message: message.into(),
            case_id: case_id.map(str::to_string),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionGateResult {
    pub gate: PromotionGate,
    pub gate_number: u8,
    pub status: GateStatus,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub checks: Vec<PromotionCheck>,
}

impl PromotionGateResult {
    pub(crate) fn from_checks(gate: PromotionGate, mut checks: Vec<PromotionCheck>) -> Self {
        if checks.is_empty() {
            checks.push(PromotionCheck::new(
                "grist.promotion.gate.no_evidence",
                false,
                "the adapter supplied no evidence for this gate",
                None,
            ));
        }
        let status = if checks.iter().all(|check| check.passed) {
            GateStatus::Passed
        } else {
            GateStatus::Failed
        };
        Self {
            gate,
            gate_number: gate.number(),
            status,
            checks,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParserConformanceReport {
    pub schema_version: String,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub harness_version: String,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub suite_version: String,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub suite_digest: String,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub format: String,
    pub parser: ParserDescriptor,
    #[cfg_attr(feature = "schemas", schemars(length(min = 11, max = 11)))]
    pub gates: Vec<PromotionGateResult>,
    #[cfg_attr(feature = "schemas", schemars(range(max = 11)))]
    pub passed_gate_count: u8,
    pub eligible_for_promotion: bool,
}

impl ParserConformanceReport {
    pub fn gate(&self, gate: PromotionGate) -> Option<&PromotionGateResult> {
        self.gates.iter().find(|result| result.gate == gate)
    }

    pub fn failed_gates(&self) -> impl Iterator<Item = &PromotionGateResult> {
        self.gates
            .iter()
            .filter(|result| result.status == GateStatus::Failed)
    }

    /// Validate semantic invariants that JSON Schema cannot express.
    pub fn validate(&self) -> Result<(), ParserConformanceReportError> {
        if self.schema_version != PARSER_CONFORMANCE_REPORT_SCHEMA_VERSION {
            return Err(ParserConformanceReportError::SchemaVersion);
        }
        if self.gates.len() != PromotionGate::ALL.len() {
            return Err(ParserConformanceReportError::GateCount);
        }
        for (expected, result) in PromotionGate::ALL.iter().zip(&self.gates) {
            if result.gate != *expected || result.gate_number != expected.number() {
                return Err(ParserConformanceReportError::GateOrder);
            }
            if result.checks.is_empty()
                || (result.status == GateStatus::Passed
                    && result.checks.iter().any(|check| !check.passed))
                || (result.status == GateStatus::Failed
                    && result.checks.iter().all(|check| check.passed))
            {
                return Err(ParserConformanceReportError::GateStatus(result.gate));
            }
        }
        let passed = self
            .gates
            .iter()
            .filter(|gate| gate.status == GateStatus::Passed)
            .count() as u8;
        if passed != self.passed_gate_count {
            return Err(ParserConformanceReportError::PassedGateCount);
        }
        if self.eligible_for_promotion != (passed == PromotionGate::ALL.len() as u8) {
            return Err(ParserConformanceReportError::Eligibility);
        }
        if self.format != self.parser.format.id
            || self.harness_version.trim().is_empty()
            || self.suite_version.trim().is_empty()
            || !self.suite_digest.starts_with("sha256:")
        {
            return Err(ParserConformanceReportError::Metadata);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ParserConformanceReportError {
    #[error("unsupported parser-conformance report schema version")]
    SchemaVersion,
    #[error("parser-conformance report must contain exactly eleven gates")]
    GateCount,
    #[error("parser-conformance gates are missing, duplicated, or out of order")]
    GateOrder,
    #[error("gate {0:?} status disagrees with its checks")]
    GateStatus(PromotionGate),
    #[error("passed_gate_count disagrees with gate results")]
    PassedGateCount,
    #[error("promotion eligibility requires all eleven gates to pass")]
    Eligibility,
    #[error("parser-conformance report metadata is incomplete or inconsistent")]
    Metadata,
}
