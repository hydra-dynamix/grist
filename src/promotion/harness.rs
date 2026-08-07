//! Shared executable policy for the eleven parser-promotion gates.

use super::adapter::{
    CiControl, ConstructDisposition, DetectionScenario, EvidenceOutcome, LocatorExpectation,
    PARSER_PROMOTION_SUITE_VERSION, ParseCaseRole, ParseExecution, ParserPromotionAdapter,
    ParserPromotionSuite, VerificationKind,
};
use super::model::{
    GateStatus, PARSER_CONFORMANCE_REPORT_SCHEMA_VERSION, ParserConformanceReport, PromotionCheck,
    PromotionGate, PromotionGateResult,
};
use crate::core::{
    DiagnosticClass, LocatorPrecision, OperationStatus, SourceLocator, canonical_json_bytes,
    canonical_json_sha256, sha256_hex,
};
use crate::detect::{Detection, DetectionStatus};
use crate::document_graph::{DocumentGraph, DocumentNodeKind};
use crate::fixtures::{FixtureClass, FixtureCorpusManifest};
use crate::segment::SegmentCollection;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component, Path, PathBuf};

pub const PARSER_PROMOTION_HARNESS_VERSION: &str = "grist/parser-promotion-harness/v1";

pub struct ParserPromotionHarness<'a> {
    corpus: &'a FixtureCorpusManifest,
    repository_root: PathBuf,
}

impl<'a> ParserPromotionHarness<'a> {
    pub fn new(corpus: &'a FixtureCorpusManifest, repository_root: impl Into<PathBuf>) -> Self {
        Self {
            corpus,
            repository_root: repository_root.into(),
        }
    }

    pub fn evaluate(&self, adapter: &dyn ParserPromotionAdapter) -> ParserConformanceReport {
        let suite = adapter.suite();
        let detections = execute_detections(adapter, suite);
        let parses = execute_parses(adapter, suite);
        let projection = projection_result(adapter, suite, &parses);
        let segments = segment_result(adapter, suite, &parses, &projection);

        let gates = vec![
            PromotionGateResult::from_checks(
                PromotionGate::Detection,
                detection_checks(adapter, suite, &detections),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::ConstructPreservation,
                construct_checks(suite, &parses),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::SourceLocators,
                locator_checks(suite, &parses),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::OperationStatuses,
                status_checks(suite, &parses),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::Determinism,
                determinism_checks(adapter, suite, &parses, &projection, &segments),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::DocumentGraphProjection,
                graph_checks(suite, &projection),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::Segmentation,
                segmentation_checks(suite, &parses, &segments),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::PublicSurfaces,
                surface_checks(adapter, suite, &parses),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::HostileInputSafety,
                hostile_checks(suite, &parses),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::FixtureCoverage,
                fixture_checks(self.corpus, &self.repository_root, adapter, suite),
            ),
            PromotionGateResult::from_checks(
                PromotionGate::EnabledFeatureCi,
                ci_checks(adapter, suite),
            ),
        ];
        let passed_gate_count = gates
            .iter()
            .filter(|gate| gate.status == GateStatus::Passed)
            .count() as u8;
        let eligible_for_promotion = passed_gate_count == PromotionGate::ALL.len() as u8;
        let suite_digest = canonical_json_sha256(suite)
            .expect("promotion suites contain only canonicalizable finite values");
        let report = ParserConformanceReport {
            schema_version: PARSER_CONFORMANCE_REPORT_SCHEMA_VERSION.to_string(),
            harness_version: PARSER_PROMOTION_HARNESS_VERSION.to_string(),
            suite_version: suite.suite_version.clone(),
            suite_digest,
            format: adapter.descriptor().format.id.clone(),
            parser: adapter.descriptor().clone(),
            gates,
            passed_gate_count,
            eligible_for_promotion,
        };
        debug_assert!(report.validate().is_ok());
        report
    }
}

type Evaluation<T> = Result<T, String>;

fn invoke<T>(
    operation: &str,
    call: impl FnOnce() -> Result<T, super::PromotionAdapterError>,
) -> Evaluation<T> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(format!("{operation}: {error}")),
        Err(_) => Err(format!(
            "{operation}: adapter panicked across the promotion boundary"
        )),
    }
}

fn execute_detections(
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
) -> BTreeMap<String, Evaluation<Detection>> {
    suite
        .detection_cases
        .iter()
        .map(|case| {
            (
                case.id.clone(),
                invoke("detect", || adapter.detect(&case.input)),
            )
        })
        .collect()
}

fn execute_parses(
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
) -> BTreeMap<String, Evaluation<ParseExecution>> {
    suite
        .parse_cases
        .iter()
        .map(|case| {
            (
                case.id.clone(),
                invoke("parse", || adapter.parse(&case.input)),
            )
        })
        .collect()
}

fn projection_result(
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
) -> Evaluation<DocumentGraph> {
    let parsed = successful_parse(parses, &suite.projection.case_id)?;
    invoke("project", || adapter.project(&parsed.envelope))
}

fn segment_result(
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
    projection: &Evaluation<DocumentGraph>,
) -> Evaluation<SegmentCollection> {
    let parsed = successful_parse(parses, &suite.projection.case_id)?;
    let graph = projection.as_ref().map_err(Clone::clone)?;
    invoke("segment", || adapter.segment(graph, &parsed.envelope))
}

fn successful_parse<'a>(
    parses: &'a BTreeMap<String, Evaluation<ParseExecution>>,
    case_id: &str,
) -> Evaluation<&'a ParseExecution> {
    parses
        .get(case_id)
        .ok_or_else(|| format!("unknown parse case `{case_id}`"))?
        .as_ref()
        .map_err(Clone::clone)
}

fn check(
    code: &str,
    passed: bool,
    message: impl Into<String>,
    case_id: Option<&str>,
) -> PromotionCheck {
    PromotionCheck::new(code, passed, message, case_id)
}

fn evaluation_check<T>(
    code: &str,
    evaluation: &Evaluation<T>,
    case_id: Option<&str>,
) -> PromotionCheck {
    match evaluation {
        Ok(_) => check(
            code,
            true,
            "operation completed without an adapter error",
            case_id,
        ),
        Err(error) => check(code, false, error, case_id),
    }
}

fn detection_checks(
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
    detections: &BTreeMap<String, Evaluation<Detection>>,
) -> Vec<PromotionCheck> {
    let scenarios = suite
        .detection_cases
        .iter()
        .map(|case| case.scenario)
        .collect::<BTreeSet<_>>();
    let mut checks = DetectionScenario::ALL
        .into_iter()
        .map(|scenario| {
            check(
                "grist.promotion.detection.scenario",
                scenarios.contains(&scenario),
                format!("detection scenario {scenario:?} is covered"),
                None,
            )
        })
        .collect::<Vec<_>>();
    for case in &suite.detection_cases {
        let Some(result) = detections.get(&case.id) else {
            checks.push(check(
                "grist.promotion.detection.missing_result",
                false,
                "detection case has no result",
                Some(&case.id),
            ));
            continue;
        };
        checks.push(evaluation_check(
            "grist.promotion.detection.adapter",
            result,
            Some(&case.id),
        ));
        let Ok(detection) = result else { continue };
        checks.push(check(
            "grist.promotion.detection.status",
            detection.status == case.expected_status,
            format!(
                "expected {:?}, observed {:?}",
                case.expected_status, detection.status
            ),
            Some(&case.id),
        ));
        let observed_format = detection
            .selected_format_identity()
            .map(|identity| identity.format);
        checks.push(check(
            "grist.promotion.detection.format",
            observed_format == case.expected_format,
            format!(
                "expected format {:?}, observed {:?}",
                case.expected_format, observed_format
            ),
            Some(&case.id),
        ));
        if case.scenario == DetectionScenario::Ambiguous {
            checks.push(check(
                "grist.promotion.detection.ambiguity_preserved",
                detection.status == DetectionStatus::Ambiguous
                    && detection.selected_parser.is_none()
                    && detection.candidates.len() >= 2,
                "ambiguous detection retains at least two candidates and selects no parser",
                Some(&case.id),
            ));
        } else if case.expected_status == DetectionStatus::Selected {
            checks.push(check(
                "grist.promotion.detection.parser_selected",
                detection.selected_parser.as_deref() == Some(adapter.descriptor().id.as_str()),
                "selected detection routes to the adapter parser",
                Some(&case.id),
            ));
        }
        checks.push(check(
            "grist.promotion.detection.evidence",
            !detection.candidates.is_empty()
                && detection
                    .candidates
                    .iter()
                    .all(|candidate| !candidate.evidence.is_empty()),
            "ranked candidates retain typed detection evidence",
            Some(&case.id),
        ));
    }
    checks
}

fn construct_checks(
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
) -> Vec<PromotionCheck> {
    let mut checks = vec![check(
        "grist.promotion.construct.accounting_present",
        suite
            .parse_cases
            .iter()
            .any(|case| !case.constructs.is_empty()),
        "at least one meaningful construct is explicitly accounted for",
        None,
    )];
    for case in &suite.parse_cases {
        let Ok(parsed) = successful_parse(parses, &case.id) else {
            continue;
        };
        for construct in &case.constructs {
            let (passed, message) = match &construct.disposition {
                ConstructDisposition::Typed { value_pointer } => {
                    let present = parsed
                        .envelope
                        .payload
                        .as_ref()
                        .and_then(|payload| payload.pointer(value_pointer))
                        .is_some_and(|value| !value.is_null());
                    (present, format!("typed value exists at {value_pointer}"))
                }
                ConstructDisposition::Raw {
                    value_pointer,
                    expected_sha256,
                } => {
                    let observed = parsed
                        .envelope
                        .payload
                        .as_ref()
                        .and_then(|payload| payload.pointer(value_pointer));
                    let present = observed.is_some_and(|value| {
                        !value.is_null()
                            && raw_value_sha256(value).as_ref() == Some(expected_sha256)
                    });
                    (
                        present,
                        format!(
                            "raw/unknown value at {value_pointer} has digest {expected_sha256}"
                        ),
                    )
                }
                ConstructDisposition::Diagnosed { diagnostic_code } => {
                    let present = parsed
                        .envelope
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code.as_str() == diagnostic_code);
                    (
                        present,
                        format!("specific diagnostic {diagnostic_code} exists"),
                    )
                }
            };
            checks.push(check(
                "grist.promotion.construct.disposition",
                passed,
                format!("{}: {message}", construct.construct),
                Some(&case.id),
            ));
        }
    }
    checks
}

fn raw_value_sha256(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(sha256_hex(value.as_bytes())),
        value => canonical_json_bytes(value)
            .ok()
            .map(|bytes| sha256_hex(&bytes)),
    }
}

fn locator_checks(
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
) -> Vec<PromotionCheck> {
    let mut checks = Vec::new();
    for case in &suite.parse_cases {
        let Ok(parsed) = successful_parse(parses, &case.id) else {
            continue;
        };
        for construct in &case.constructs {
            if matches!(
                construct.disposition,
                ConstructDisposition::Diagnosed { .. }
            ) && construct.locator.is_none()
            {
                continue;
            }
            let Some(expectation) = &construct.locator else {
                checks.push(check(
                    "grist.promotion.locator.missing_expectation",
                    false,
                    format!(
                        "{} emits a fact but declares no locator",
                        construct.construct
                    ),
                    Some(&case.id),
                ));
                continue;
            };
            let locator = locate_expected(parsed, expectation);
            match locator {
                Ok(locator) => checks.push(check(
                    "grist.promotion.locator.valid",
                    locator.validate().is_ok()
                        && matches!(
                            locator.precision(),
                            LocatorPrecision::Exact { .. }
                                | LocatorPrecision::Approximate { .. }
                                | LocatorPrecision::Synthetic { .. }
                        ),
                    format!(
                        "{} has a validated exact, approximate, or derived locator",
                        construct.construct
                    ),
                    Some(&case.id),
                )),
                Err(error) => checks.push(check(
                    "grist.promotion.locator.invalid",
                    false,
                    format!("{}: {error}", construct.construct),
                    Some(&case.id),
                )),
            }
        }
    }
    checks
}

fn locate_expected(
    parsed: &ParseExecution,
    expectation: &LocatorExpectation,
) -> Evaluation<SourceLocator> {
    match expectation {
        LocatorExpectation::Payload { locator_pointer } => {
            let value = parsed
                .envelope
                .payload
                .as_ref()
                .and_then(|payload| payload.pointer(locator_pointer))
                .ok_or_else(|| format!("locator pointer `{locator_pointer}` is missing"))?;
            serde_json::from_value(value.clone())
                .map_err(|error| format!("locator pointer `{locator_pointer}` is invalid: {error}"))
        }
        LocatorExpectation::Diagnostic { diagnostic_code } => parsed
            .envelope
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code.as_str() == diagnostic_code)
            .ok_or_else(|| format!("diagnostic `{diagnostic_code}` is missing"))?
            .locator
            .clone()
            .map(|locator| *locator)
            .ok_or_else(|| format!("diagnostic `{diagnostic_code}` has no locator")),
    }
}

fn status_checks(
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
) -> Vec<PromotionCheck> {
    let required = [
        ParseCaseRole::Complete,
        ParseCaseRole::Partial,
        ParseCaseRole::Failed,
        ParseCaseRole::Encrypted,
        ParseCaseRole::Unsupported,
        ParseCaseRole::BudgetLimited,
    ];
    let roles = suite
        .parse_cases
        .iter()
        .flat_map(|case| case.roles.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut checks = required
        .into_iter()
        .map(|role| {
            check(
                "grist.promotion.status.case_present",
                roles.contains(&role),
                format!("status role {role:?} is covered"),
                None,
            )
        })
        .collect::<Vec<_>>();
    for case in &suite.parse_cases {
        let Some(result) = parses.get(&case.id) else {
            continue;
        };
        checks.push(evaluation_check(
            "grist.promotion.status.adapter",
            result,
            Some(&case.id),
        ));
        let Ok(parsed) = result else { continue };
        checks.push(check(
            "grist.promotion.status.expected",
            parsed.envelope.status == case.expected_status,
            format!(
                "expected {:?}, observed {:?}",
                case.expected_status, parsed.envelope.status
            ),
            Some(&case.id),
        ));
        checks.push(check(
            "grist.promotion.status.envelope_invariants",
            parsed.envelope.validate().is_ok(),
            "envelope payload/status/diagnostic invariants hold",
            Some(&case.id),
        ));
        for role in &case.roles {
            let expected = match role {
                ParseCaseRole::Complete => Some(OperationStatus::Complete),
                ParseCaseRole::Partial => Some(OperationStatus::Partial),
                ParseCaseRole::Failed => Some(OperationStatus::Failed),
                ParseCaseRole::Encrypted => Some(OperationStatus::Encrypted),
                ParseCaseRole::Unsupported => Some(OperationStatus::Unsupported),
                _ => None,
            };
            if let Some(expected) = expected {
                checks.push(check(
                    "grist.promotion.status.role_agreement",
                    parsed.envelope.status == expected,
                    format!("role {role:?} agrees with envelope status"),
                    Some(&case.id),
                ));
            }
            if *role == ParseCaseRole::BudgetLimited {
                let budget_status = matches!(
                    parsed.envelope.status,
                    OperationStatus::Partial | OperationStatus::Failed
                );
                let diagnostic = parsed.envelope.diagnostics.iter().any(|diagnostic| {
                    diagnostic.class == DiagnosticClass::ResourceBudgetExhaustion
                });
                checks.push(check(
                    "grist.promotion.status.budget_distinguished",
                    budget_status && diagnostic,
                    "budget-limited output is partial/failed with a resource-budget diagnostic",
                    Some(&case.id),
                ));
            }
        }
    }
    checks
}

fn determinism_checks(
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
    projection: &Evaluation<DocumentGraph>,
    segments: &Evaluation<SegmentCollection>,
) -> Vec<PromotionCheck> {
    let cases = suite
        .parse_cases
        .iter()
        .filter(|case| case.roles.contains(&ParseCaseRole::Determinism))
        .collect::<Vec<_>>();
    let mut checks = vec![check(
        "grist.promotion.determinism.case_present",
        !cases.is_empty(),
        "at least one parse case is designated for deterministic replay",
        None,
    )];
    for case in cases {
        let first = successful_parse(parses, &case.id).map(|parsed| &parsed.envelope);
        let second = invoke("deterministic parse replay", || adapter.parse(&case.input))
            .map(|parsed| parsed.envelope);
        checks.push(canonical_equality_check(
            "grist.promotion.determinism.parse",
            first,
            second.as_ref().map_err(Clone::clone),
            Some(&case.id),
        ));
    }
    let projection_case = suite
        .parse_cases
        .iter()
        .find(|case| case.id == suite.projection.case_id);
    if let (Some(case), Ok(parsed), Ok(first_graph), Ok(first_segments)) = (
        projection_case,
        successful_parse(parses, &suite.projection.case_id),
        projection,
        segments,
    ) {
        let second_graph = invoke("deterministic graph replay", || {
            adapter.project(&parsed.envelope)
        });
        checks.push(canonical_equality_check(
            "grist.promotion.determinism.graph",
            Ok(first_graph),
            second_graph.as_ref().map_err(Clone::clone),
            Some(&case.id),
        ));
        if let Ok(second_graph) = &second_graph {
            let second_segments = invoke("deterministic segment replay", || {
                adapter.segment(second_graph, &parsed.envelope)
            });
            checks.push(canonical_equality_check(
                "grist.promotion.determinism.segments",
                Ok(first_segments),
                second_segments.as_ref().map_err(Clone::clone),
                Some(&case.id),
            ));
        }
    } else {
        checks.push(check(
            "grist.promotion.determinism.projection_unavailable",
            false,
            "projection and segmentation must be available for deterministic replay",
            Some(&suite.projection.case_id),
        ));
    }
    checks
}

fn canonical_equality_check<T: Serialize>(
    code: &str,
    first: Evaluation<&T>,
    second: Evaluation<&T>,
    case_id: Option<&str>,
) -> PromotionCheck {
    let comparison = first.and_then(|first| {
        let first = canonical_json_bytes(first).map_err(|error| error.to_string())?;
        let second = second?;
        let second = canonical_json_bytes(second).map_err(|error| error.to_string())?;
        Ok(first == second)
    });
    match comparison {
        Ok(equal) => check(
            code,
            equal,
            "canonical JSON replay is byte-identical",
            case_id,
        ),
        Err(error) => check(code, false, error, case_id),
    }
}

fn graph_checks(
    suite: &ParserPromotionSuite,
    projection: &Evaluation<DocumentGraph>,
) -> Vec<PromotionCheck> {
    let mut checks = vec![evaluation_check(
        "grist.promotion.graph.adapter",
        projection,
        Some(&suite.projection.case_id),
    )];
    let Ok(graph) = projection else { return checks };
    checks.push(check(
        "grist.promotion.graph.nodes_present",
        !graph.nodes.is_empty(),
        "DocumentGraph contains projected structure",
        Some(&suite.projection.case_id),
    ));
    checks.push(check(
        "grist.promotion.graph.authoritative_payload",
        graph.projection.as_ref().is_some_and(|metadata| {
            !metadata.authoritative_payload_kind.is_empty()
                && !metadata.authoritative_payload_schema_version.is_empty()
                && !metadata.projection_rule.is_empty()
        }),
        "projection identifies its authoritative payload and projection rule",
        Some(&suite.projection.case_id),
    ));
    checks.push(check(
        "grist.promotion.graph.contract",
        graph.validate_contract().is_ok(),
        "DocumentGraph cross-node, raw-content, locator, and identity invariants hold",
        Some(&suite.projection.case_id),
    ));
    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    checks.push(check(
        "grist.promotion.graph.unique_node_ids",
        node_ids.len() == graph.nodes.len() && !node_ids.contains(""),
        "all graph node IDs are non-empty and unique",
        Some(&suite.projection.case_id),
    ));
    checks.push(check(
        "grist.promotion.graph.locators",
        graph.nodes.iter().all(|node| {
            node.kind == DocumentNodeKind::Document
                || node
                    .locator
                    .as_ref()
                    .is_some_and(|locator| locator.validate().is_ok())
        }),
        "every non-root projected node has a validated source locator",
        Some(&suite.projection.case_id),
    ));
    for kind in &suite.projection.required_node_kinds {
        checks.push(check(
            "grist.promotion.graph.required_kind",
            graph.nodes.iter().any(|node| &node.kind == kind),
            format!("required node kind {kind:?} is projected"),
            Some(&suite.projection.case_id),
        ));
    }
    checks
}

fn segmentation_checks(
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
    segments: &Evaluation<SegmentCollection>,
) -> Vec<PromotionCheck> {
    let mut checks = vec![evaluation_check(
        "grist.promotion.segment.adapter",
        segments,
        Some(&suite.projection.case_id),
    )];
    let Ok(collection) = segments else {
        return checks;
    };
    checks.push(check(
        "grist.promotion.segment.present",
        !collection.segments.is_empty(),
        "projection produces at least one deterministic segment",
        Some(&suite.projection.case_id),
    ));
    checks.push(check(
        "grist.promotion.segment.traceable",
        collection.segments.iter().all(|segment| {
            !segment.id.is_empty()
                && !segment.node_ids.is_empty()
                && segment.node_ids.len() == segment.locators.len()
                && segment.node_references.iter().all(|reference| {
                    reference.locator.validate().is_ok()
                        && reference.segment_byte_start <= reference.segment_byte_end
                        && reference.segment_byte_end <= segment.text.len()
                        && segment.node_ids.contains(&reference.node_id)
                })
                && segment.node_references.len() >= segment.node_ids.len()
        }),
        "every segment retains source nodes, locators, and rendered-span mappings",
        Some(&suite.projection.case_id),
    ));
    let identity_matches = successful_parse(parses, &suite.projection.case_id)
        .ok()
        .and_then(|parsed| parsed.envelope.identity.as_ref())
        .is_some_and(|identity| {
            collection
                .segments
                .iter()
                .all(|segment| &segment.source_identity == identity)
        });
    checks.push(check(
        "grist.promotion.segment.source_identity",
        identity_matches,
        "every segment preserves the parse source identity",
        Some(&suite.projection.case_id),
    ));
    checks
}

fn surface_checks(
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
) -> Vec<PromotionCheck> {
    let surface = &suite.public_surface;
    let parsed = successful_parse(parses, &surface.case_id);
    let mut checks = vec![check(
        "grist.promotion.surface.cli_case",
        suite
            .parse_cases
            .iter()
            .any(|case| case.id == surface.case_id && case.roles.contains(&ParseCaseRole::Cli)),
        "public-surface case is explicitly designated for CLI parity",
        Some(&surface.case_id),
    )];
    let Ok(parsed) = parsed else {
        checks.push(check(
            "grist.promotion.surface.parse_missing",
            false,
            "public-surface parse case did not complete",
            Some(&surface.case_id),
        ));
        return checks;
    };
    let payload = parsed.envelope.payload.as_ref();
    checks.push(check(
        "grist.promotion.surface.rust_payload",
        payload.is_some_and(|payload| adapter.rust_payload_accepts(payload).is_ok()),
        "authoritative payload deserializes through the public Rust type",
        Some(&surface.case_id),
    ));
    checks.push(check(
        "grist.promotion.surface.payload_version",
        parsed.envelope.payload_schema_version.0.as_str()
            == adapter.descriptor().payload_schema.version,
        "Rust descriptor and envelope agree on payload schema version",
        Some(&surface.case_id),
    ));
    checks.push(check(
        "grist.promotion.surface.schema_names",
        surface.payload_schema_name == adapter.descriptor().payload_schema.name
            && surface.options_schema_name == adapter.descriptor().options.schema.name,
        "public-surface schema names agree with parser metadata",
        Some(&surface.case_id),
    ));

    let envelope_value = serde_json::to_value(&parsed.envelope).expect("envelope serializes");
    let schema_inputs = [
        (
            surface.envelope_schema_name.as_str(),
            parsed.envelope.schema_version.0.as_str(),
            &envelope_value,
        ),
        (
            surface.payload_schema_name.as_str(),
            adapter.descriptor().payload_schema.version.as_str(),
            payload.unwrap_or(&Value::Null),
        ),
        (
            surface.options_schema_name.as_str(),
            adapter.descriptor().options.schema.version.as_str(),
            &adapter.descriptor().options.default,
        ),
    ];
    for (name, version, instance) in schema_inputs {
        let schema = adapter.schema_json(name);
        let valid = schema.as_ref().is_some_and(|schema| {
            crate::schema::validate_against_schema(name, version, instance, schema)
                .is_ok_and(|report| report.valid)
        });
        checks.push(check(
            "grist.promotion.surface.schema",
            valid,
            format!("{name}@{version} exists and validates its Rust/API value"),
            Some(&surface.case_id),
        ));
        let canonical_valid = adapter.canonical_example(name).is_some_and(|example| {
            schema.as_ref().is_some_and(|schema| {
                crate::schema::validate_against_schema(name, version, &example, schema)
                    .is_ok_and(|report| report.valid)
            })
        });
        checks.push(check(
            "grist.promotion.surface.canonical_example",
            canonical_valid,
            format!("{name}@{version} has a schema-valid canonical example"),
            Some(&surface.case_id),
        ));
    }

    let case = suite
        .parse_cases
        .iter()
        .find(|case| case.id == surface.case_id)
        .expect("successful parse implies declared case");
    let first_cli = invoke("CLI parse", || {
        adapter.cli_parse(&surface.cli_selector, &case.input)
    });
    let second_cli = invoke("CLI parse replay", || {
        adapter.cli_parse(&surface.cli_selector, &case.input)
    });
    checks.push(canonical_equality_check(
        "grist.promotion.surface.cli_library_parity",
        Ok(&envelope_value),
        first_cli.as_ref().map_err(Clone::clone),
        Some(&surface.case_id),
    ));
    checks.push(canonical_equality_check(
        "grist.promotion.surface.cli_deterministic",
        first_cli.as_ref().map_err(Clone::clone),
        second_cli.as_ref().map_err(Clone::clone),
        Some(&surface.case_id),
    ));
    let selector_valid = surface.cli_selector == adapter.descriptor().format.id
        || adapter
            .descriptor()
            .format
            .aliases
            .contains(&surface.cli_selector);
    checks.push(check(
        "grist.promotion.surface.cli_selector",
        selector_valid,
        "CLI selector is the canonical format or a registered alias",
        Some(&surface.case_id),
    ));
    checks
}

fn hostile_checks(
    suite: &ParserPromotionSuite,
    parses: &BTreeMap<String, Evaluation<ParseExecution>>,
) -> Vec<PromotionCheck> {
    let cases = suite
        .parse_cases
        .iter()
        .filter(|case| case.roles.contains(&ParseCaseRole::Hostile))
        .collect::<Vec<_>>();
    let mut checks = vec![check(
        "grist.promotion.security.hostile_case_present",
        !cases.is_empty(),
        "at least one hostile byte-facing case crosses the public adapter boundary",
        None,
    )];
    for case in cases {
        let Some(result) = parses.get(&case.id) else {
            continue;
        };
        checks.push(evaluation_check(
            "grist.promotion.security.no_panic",
            result,
            Some(&case.id),
        ));
        let Ok(parsed) = result else { continue };
        let Some(observation) = &parsed.security else {
            checks.push(check(
                "grist.promotion.security.observation_missing",
                false,
                "hostile cases require explicit side-effect instrumentation",
                Some(&case.id),
            ));
            continue;
        };
        checks.push(check(
            "grist.promotion.security.no_execution",
            observation.active_content_executions == 0,
            "active content execution counter remained zero",
            Some(&case.id),
        ));
        checks.push(check(
            "grist.promotion.security.no_network",
            observation.network_requests == 0,
            "network request counter remained zero",
            Some(&case.id),
        ));
        checks.push(check(
            "grist.promotion.security.no_path_escape",
            observation.path_escape_writes == 0,
            "path-escape write counter remained zero",
            Some(&case.id),
        ));
        checks.push(check(
            "grist.promotion.security.bounded_expansion",
            !observation.expansion_limit_exceeded,
            "hostile input remained within the declared expansion budget",
            Some(&case.id),
        ));
        checks.push(check(
            "grist.promotion.security.no_temp_leak",
            observation.leaked_temporary_artifacts == 0,
            "private temporary artifacts were cleaned up",
            Some(&case.id),
        ));
    }
    checks
}

fn fixture_checks(
    corpus: &FixtureCorpusManifest,
    repository_root: &Path,
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
) -> Vec<PromotionCheck> {
    let fixture = &suite.fixtures;
    let mut checks = vec![check(
        "grist.promotion.fixture.format_agreement",
        fixture.corpus_format == adapter.descriptor().format.id,
        "fixture-corpus format agrees with parser descriptor",
        None,
    )];
    let Some(format) = corpus.formats.get(&fixture.corpus_format) else {
        checks.push(check(
            "grist.promotion.fixture.format_missing",
            false,
            "fixture corpus has no entry for this format",
            None,
        ));
        return checks;
    };
    let mandatory = [
        FixtureClass::MinimalValid,
        FixtureClass::RepresentativeRealWorld,
        FixtureClass::Malformed,
        FixtureClass::Adversarial,
        FixtureClass::UnsupportedConstruct,
        FixtureClass::MaliciousActiveContent,
        FixtureClass::DownstreamRegression,
    ];
    let declared = mandatory
        .into_iter()
        .chain(fixture.required_classes.iter().copied())
        .collect::<BTreeSet<_>>();
    let observed = format
        .cases
        .iter()
        .flat_map(|case| case.classes.iter().copied())
        .collect::<BTreeSet<_>>();
    for class in declared {
        checks.push(check(
            "grist.promotion.fixture.class",
            observed.contains(&class),
            format!("fixture class {class:?} is registered for the format"),
            None,
        ));
    }
    checks.push(check(
        "grist.promotion.fixture.provenance",
        format.cases.iter().all(|case| {
            !case.id.trim().is_empty()
                && !case.provenance.source.trim().is_empty()
                && !case.provenance.license.expression.trim().is_empty()
        }),
        "every format fixture has source and license provenance",
        None,
    ));
    checks.push(check(
        "grist.promotion.fixture.fuzz_corpus_present",
        !fixture.fuzz_corpus_paths.is_empty(),
        "at least one byte-facing fuzz corpus path is declared",
        None,
    ));
    for path in &fixture.fuzz_corpus_paths {
        let relative = Path::new(path);
        let safe = !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        let populated = safe && path_has_content(&repository_root.join(relative));
        checks.push(check(
            "grist.promotion.fixture.fuzz_corpus_path",
            populated,
            format!("fuzz corpus path `{path}` is safe, present, and non-empty"),
            None,
        ));
    }
    checks
}

fn path_has_content(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() {
        return false;
    }
    if metadata.is_file() {
        return metadata.len() > 0;
    }
    metadata.is_dir()
        && std::fs::read_dir(path)
            .ok()
            .is_some_and(|mut entries| entries.any(|entry| entry.is_ok()))
}

fn ci_checks(
    adapter: &dyn ParserPromotionAdapter,
    suite: &ParserPromotionSuite,
) -> Vec<PromotionCheck> {
    let mut checks = vec![check(
        "grist.promotion.ci.suite_version",
        suite.suite_version == PARSER_PROMOTION_SUITE_VERSION,
        format!(
            "suite version is {}, expected {}",
            suite.suite_version, PARSER_PROMOTION_SUITE_VERSION
        ),
        None,
    )];
    let detection_ids = suite
        .detection_cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<Vec<_>>();
    let parse_ids = suite
        .parse_cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<Vec<_>>();
    checks.push(check(
        "grist.promotion.ci.unique_case_ids",
        all_unique(&detection_ids)
            && all_unique(&parse_ids)
            && detection_ids.iter().all(|id| !id.trim().is_empty())
            && parse_ids.iter().all(|id| !id.trim().is_empty()),
        "detection and parse case IDs are independently unique and non-empty",
        None,
    ));

    for kind in VerificationKind::ALL {
        let evidence = suite
            .verification
            .iter()
            .filter(|entry| entry.kind == kind)
            .collect::<Vec<_>>();
        let valid = evidence.len() == 1
            && evidence[0].outcome != EvidenceOutcome::Failed
            && !evidence[0].evidence.trim().is_empty()
            && match evidence[0].outcome {
                EvidenceOutcome::Passed => true,
                EvidenceOutcome::NotApplicable => {
                    kind.permits_not_applicable()
                        && evidence[0]
                            .rationale
                            .as_deref()
                            .is_some_and(|rationale| !rationale.trim().is_empty())
                }
                EvidenceOutcome::Failed => false,
            };
        checks.push(check(
            "grist.promotion.ci.verification_kind",
            valid,
            format!("verification kind {kind:?} has one acceptable evidence record"),
            None,
        ));
    }

    for control in CiControl::ALL {
        let evidence = suite
            .ci
            .iter()
            .filter(|entry| entry.control == control)
            .collect::<Vec<_>>();
        let valid =
            evidence.len() == 1 && evidence[0].passed && !evidence[0].command.trim().is_empty();
        checks.push(check(
            "grist.promotion.ci.control",
            valid,
            format!("CI control {control:?} has one passing command"),
            None,
        ));
        if let Some(evidence) = evidence.first()
            && matches!(
                control,
                CiControl::EnabledFeatureTests | CiControl::MinimalFeatureTests
            )
        {
            checks.push(check(
                "grist.promotion.ci.features",
                adapter
                    .descriptor()
                    .required_features
                    .is_subset(&evidence.enabled_features),
                format!("{control:?} enables every parser-required feature"),
                None,
            ));
            if control == CiControl::MinimalFeatureTests {
                checks.push(check(
                    "grist.promotion.ci.no_default_features",
                    evidence.command.contains("--no-default-features"),
                    "minimal feature test explicitly disables default features",
                    None,
                ));
            }
        }
    }
    checks
}

fn all_unique(values: &[&str]) -> bool {
    values.iter().copied().collect::<BTreeSet<_>>().len() == values.len()
}
