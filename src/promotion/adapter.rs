//! Format-owned inputs and the shared parser-promotion adapter boundary.

use crate::core::{Envelope, FormatHint, OperationStatus, SourceInfo};
use crate::detect::{Detection, DetectionStatus};
use crate::document_graph::{DocumentGraph, DocumentNodeKind};
use crate::fixtures::FixtureClass;
use crate::registry::ParserDescriptor;
use crate::segment::SegmentCollection;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

pub const PARSER_PROMOTION_SUITE_VERSION: &str = "grist/parser-promotion-suite/v1";

#[derive(Debug, Clone, Serialize)]
pub struct PromotionInput {
    pub bytes: Vec<u8>,
    pub source: SourceInfo,
    pub format_hint: Option<FormatHint>,
}

impl PromotionInput {
    pub fn new(bytes: impl Into<Vec<u8>>, source: SourceInfo) -> Self {
        Self {
            bytes: bytes.into(),
            source,
            format_hint: None,
        }
    }

    pub fn with_format_hint(mut self, hint: FormatHint) -> Self {
        self.format_hint = Some(hint);
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DetectionScenario {
    Valid,
    Mislabeled,
    Extensionless,
    Malformed,
    Ambiguous,
}

impl DetectionScenario {
    pub const ALL: [Self; 5] = [
        Self::Valid,
        Self::Mislabeled,
        Self::Extensionless,
        Self::Malformed,
        Self::Ambiguous,
    ];
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectionCase {
    pub id: String,
    pub scenario: DetectionScenario,
    pub input: PromotionInput,
    pub expected_status: DetectionStatus,
    pub expected_format: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ParseCaseRole {
    Complete,
    Partial,
    Failed,
    Encrypted,
    Unsupported,
    BudgetLimited,
    Determinism,
    Projection,
    Cli,
    Hostile,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum ConstructDisposition {
    Typed {
        value_pointer: String,
    },
    Raw {
        value_pointer: String,
        expected_sha256: String,
    },
    Diagnosed {
        diagnostic_code: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum LocatorExpectation {
    Payload { locator_pointer: String },
    Diagnostic { diagnostic_code: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct ConstructExpectation {
    pub construct: String,
    pub disposition: ConstructDisposition,
    pub locator: Option<LocatorExpectation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParseCase {
    pub id: String,
    pub input: PromotionInput,
    pub expected_status: OperationStatus,
    pub roles: BTreeSet<ParseCaseRole>,
    pub constructs: Vec<ConstructExpectation>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecurityObservation {
    pub active_content_executions: u64,
    pub network_requests: u64,
    pub path_escape_writes: u64,
    pub expansion_limit_exceeded: bool,
    pub leaked_temporary_artifacts: u64,
}

#[derive(Debug, Clone)]
pub struct ParseExecution {
    pub envelope: Envelope<Value>,
    pub security: Option<SecurityObservation>,
}

impl From<Envelope<Value>> for ParseExecution {
    fn from(envelope: Envelope<Value>) -> Self {
        Self {
            envelope,
            security: None,
        }
    }
}

impl ParseExecution {
    pub fn with_security_observation(
        envelope: Envelope<Value>,
        security: SecurityObservation,
    ) -> Self {
        Self {
            envelope,
            security: Some(security),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectionContract {
    pub case_id: String,
    pub required_node_kinds: Vec<DocumentNodeKind>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicSurfaceContract {
    pub case_id: String,
    pub envelope_schema_name: String,
    pub payload_schema_name: String,
    pub options_schema_name: String,
    pub cli_selector: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FixtureContract {
    pub corpus_format: String,
    pub required_classes: BTreeSet<FixtureClass>,
    pub fuzz_corpus_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    Unit,
    Golden,
    Property,
    Fuzz,
    Differential,
    RoundTrip,
    SourceMap,
    ConcurrencyCancellation,
    SchemaCompatibility,
    ResourceBudget,
    Security,
    PerformanceBenchmark,
}

impl VerificationKind {
    pub const ALL: [Self; 12] = [
        Self::Unit,
        Self::Golden,
        Self::Property,
        Self::Fuzz,
        Self::Differential,
        Self::RoundTrip,
        Self::SourceMap,
        Self::ConcurrencyCancellation,
        Self::SchemaCompatibility,
        Self::ResourceBudget,
        Self::Security,
        Self::PerformanceBenchmark,
    ];

    pub const fn permits_not_applicable(self) -> bool {
        matches!(self, Self::Differential | Self::RoundTrip)
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOutcome {
    Passed,
    NotApplicable,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerificationEvidence {
    pub kind: VerificationKind,
    pub outcome: EvidenceOutcome,
    pub evidence: String,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CiControl {
    EnabledFeatureTests,
    MinimalFeatureTests,
    Clippy,
    Formatting,
    SchemaDrift,
    Documentation,
}

impl CiControl {
    pub const ALL: [Self; 6] = [
        Self::EnabledFeatureTests,
        Self::MinimalFeatureTests,
        Self::Clippy,
        Self::Formatting,
        Self::SchemaDrift,
        Self::Documentation,
    ];
}

#[derive(Debug, Clone, Serialize)]
pub struct CiEvidence {
    pub control: CiControl,
    pub command: String,
    pub enabled_features: BTreeSet<String>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParserPromotionSuite {
    pub suite_version: String,
    pub detection_cases: Vec<DetectionCase>,
    pub parse_cases: Vec<ParseCase>,
    pub projection: ProjectionContract,
    pub public_surface: PublicSurfaceContract,
    pub fixtures: FixtureContract,
    pub verification: Vec<VerificationEvidence>,
    pub ci: Vec<CiEvidence>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct PromotionAdapterError {
    pub message: String,
}

impl PromotionAdapterError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// One reusable adapter per format. Implementations call the real public
/// parser/graph/segment/CLI surfaces; they do not duplicate promotion policy.
pub trait ParserPromotionAdapter {
    fn descriptor(&self) -> &ParserDescriptor;
    fn suite(&self) -> &ParserPromotionSuite;
    fn detect(&self, input: &PromotionInput) -> Result<Detection, PromotionAdapterError>;
    fn parse(&self, input: &PromotionInput) -> Result<ParseExecution, PromotionAdapterError>;
    fn project(&self, parsed: &Envelope<Value>) -> Result<DocumentGraph, PromotionAdapterError>;
    fn segment(
        &self,
        graph: &DocumentGraph,
        parsed: &Envelope<Value>,
    ) -> Result<SegmentCollection, PromotionAdapterError>;
    fn cli_parse(
        &self,
        selector: &str,
        input: &PromotionInput,
    ) -> Result<Value, PromotionAdapterError>;
    fn rust_payload_accepts(&self, payload: &Value) -> Result<(), PromotionAdapterError>;
    fn schema_json(&self, name: &str) -> Option<Value>;
    fn canonical_example(&self, name: &str) -> Option<Value>;
}
