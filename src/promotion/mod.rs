//! Universal parser adapter and promotion-gate harness.
//!
//! Format implementations supply real operations and format-specific cases via
//! [`ParserPromotionAdapter`]. [`ParserPromotionHarness`] owns all acceptance
//! policy and returns a deterministic, schema-backed report. A parser is
//! promotion-eligible only when every one of the eleven gates passes.

mod adapter;
mod harness;
mod model;

pub use adapter::{
    CiControl, CiEvidence, ConstructDisposition, ConstructExpectation, DetectionCase,
    DetectionScenario, EvidenceOutcome, FixtureContract, LocatorExpectation,
    PARSER_PROMOTION_SUITE_VERSION, ParseCase, ParseCaseRole, ParseExecution,
    ParserPromotionAdapter, ParserPromotionSuite, ProjectionContract, PromotionAdapterError,
    PromotionInput, PublicSurfaceContract, SecurityObservation, VerificationEvidence,
    VerificationKind,
};
pub use harness::{PARSER_PROMOTION_HARNESS_VERSION, ParserPromotionHarness};
pub use model::{
    GateStatus, PARSER_CONFORMANCE_REPORT_SCHEMA_VERSION, ParserConformanceReport,
    ParserConformanceReportError, PromotionCheck, PromotionGate, PromotionGateResult,
};
