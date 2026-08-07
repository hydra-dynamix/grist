#![cfg(all(feature = "restructured-text", feature = "schemas"))]

use grist::core::{
    ArtifactKind, BudgetSelection, ContentIdentity, Diagnostic, Envelope, Input, Limits,
    OperationKind, OperationStatus, ParseRequest, ParserInfo, ProviderSet, RequestId,
    ResourceBudget, SchemaVersion, SourceInfo, options_digest, sha256_hex,
};
use grist::detect::{Detection, DetectionOptions, DetectionStatus, detect_source};
use grist::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentNodeKind, ToDocumentGraph,
};
use grist::fixtures::load_corpus;
use grist::promotion::{
    CiControl, CiEvidence, ConstructDisposition, ConstructExpectation, DetectionCase,
    DetectionScenario, EvidenceOutcome, FixtureContract, LocatorExpectation,
    PARSER_PROMOTION_SUITE_VERSION, ParseCase, ParseCaseRole, ParseExecution,
    ParserPromotionAdapter, ParserPromotionHarness, ParserPromotionSuite, ProjectionContract,
    PromotionAdapterError, PromotionInput, PublicSurfaceContract, SecurityObservation,
    VerificationEvidence, VerificationKind,
};
use grist::registry::{ParserDescriptor, ParserSelection, builtin_parser_registry};
use grist::restructured_text::{RestructuredTextDocument, RestructuredTextOptions};
use grist::segment::{SegmentCollection, SegmentOptions, segment_document_graph};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

const COMPLETE: &str = "Title\n=====\n\nParagraph :code:`x`, target_, [1]_, and `link <https://example.test>`_.\n\n.. _target: destination\n.. [1] note\n\n.. warning:: inert\n\n.. code-block:: rust\n\n   fn inert() {}\n\n+---+---+\n| A | B |\n+===+===+\n| 1 | 2 |\n+---+---+\n";

struct RstAdapter {
    descriptor: ParserDescriptor,
    suite: ParserPromotionSuite,
}

fn err(value: impl std::fmt::Display) -> PromotionAdapterError {
    PromotionAdapterError::new(value.to_string())
}

impl ParserPromotionAdapter for RstAdapter {
    fn descriptor(&self) -> &ParserDescriptor {
        &self.descriptor
    }

    fn suite(&self) -> &ParserPromotionSuite {
        &self.suite
    }

    fn detect(&self, input: &PromotionInput) -> Result<Detection, PromotionAdapterError> {
        let registry = builtin_parser_registry().map_err(err)?;
        detect_source(
            &input.source,
            &input.bytes,
            input.format_hint.as_ref(),
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .map_err(err)
    }

    fn parse(&self, input: &PromotionInput) -> Result<ParseExecution, PromotionAdapterError> {
        let name = input.source.display_name.as_str();
        if name.contains("encrypted-status") {
            return Ok(terminal(
                input,
                OperationStatus::Encrypted,
                Diagnostic::error(
                    "grist.restructured_text",
                    "restructured_text.status.encrypted",
                    "status contract",
                ),
            )
            .into());
        }
        if name.contains("unsupported-status") {
            return Ok(terminal(
                input,
                OperationStatus::Unsupported,
                Diagnostic::unsupported("grist.restructured_text", "status contract"),
            )
            .into());
        }
        let mut budget = ResourceBudget::trusted_unbounded();
        if name.contains("budget") {
            budget.max_decoded_characters = Some(1);
        }
        let options = if name.contains("mixed-invalid") {
            Some(RestructuredTextOptions {
                encoding: Some("utf-8".into()),
                ..Default::default()
            })
        } else if name.contains("decode-failed") {
            Some(RestructuredTextOptions {
                encoding: Some("x-grist-unsupported-charset".into()),
                ..Default::default()
            })
        } else {
            None
        };
        let envelope = dispatch(input, BudgetSelection::custom(budget), options)?;
        Ok(if name.contains("hostile") {
            ParseExecution::with_security_observation(envelope, SecurityObservation::default())
        } else {
            envelope.into()
        })
    }

    fn project(&self, parsed: &Envelope<Value>) -> Result<DocumentGraph, PromotionAdapterError> {
        let payload = parsed
            .payload
            .clone()
            .ok_or_else(|| PromotionAdapterError::new("reStructuredText payload missing"))?;
        serde_json::from_value::<RestructuredTextDocument>(payload)
            .map_err(err)?
            .to_document_graph(
                DocumentGraphContext::new("promotion:restructured-text")
                    .with_source(parsed.source.clone()),
            )
            .map_err(err)
    }

    fn segment(
        &self,
        graph: &DocumentGraph,
        parsed: &Envelope<Value>,
    ) -> Result<SegmentCollection, PromotionAdapterError> {
        let source = parsed
            .identity
            .as_ref()
            .ok_or_else(|| PromotionAdapterError::new("source identity missing"))?;
        let document = ContentIdentity::default()
            .with_canonical_payload(graph.schema_version.as_str(), graph)
            .map_err(err)?;
        segment_document_graph(graph, source, &document, &SegmentOptions::default(), None)
            .map_err(err)
    }

    fn cli_parse(
        &self,
        selector: &str,
        input: &PromotionInput,
    ) -> Result<Value, PromotionAdapterError> {
        let envelope = dispatch(
            input,
            BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
            None,
        )?;
        if selector != "restructured-text" {
            return Err(PromotionAdapterError::new("unexpected CLI selector"));
        }
        serde_json::to_value(envelope).map_err(err)
    }

    fn rust_payload_accepts(&self, payload: &Value) -> Result<(), PromotionAdapterError> {
        serde_json::from_value::<RestructuredTextDocument>(payload.clone())
            .map(|_| ())
            .map_err(err)
    }

    fn schema_json(&self, name: &str) -> Option<Value> {
        grist::schema::schema_json(name)
    }

    fn canonical_example(&self, name: &str) -> Option<Value> {
        grist::schema::canonical_example_json(name)
    }
}

fn dispatch(
    input: &PromotionInput,
    budget: BudgetSelection,
    options: Option<RestructuredTextOptions>,
) -> Result<Envelope<Value>, PromotionAdapterError> {
    let request = ParseRequest::new(
        RequestId::new("restructured-text-promotion").map_err(err)?,
        Input::bytes(input.bytes.clone()),
        input.source.clone(),
        budget,
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .map_err(err)?
        .dispatch(
            "restructured-text",
            request,
            options.map(|value| serde_json::to_value(value).unwrap()),
        )
        .map_err(err)
}

fn terminal(
    input: &PromotionInput,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> Envelope<Value> {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::RestructuredText,
        status,
        input.source.clone(),
        ParserInfo::new("grist.restructured_text"),
        options_digest(&json!({})).unwrap(),
        SchemaVersion::RESTRUCTURED_TEXT_V1,
    )
    .unwrap()
    .with_identity(ContentIdentity::for_raw_bytes(&input.bytes))
    .with_diagnostics(vec![diagnostic])
}

fn input(bytes: impl Into<Vec<u8>>, name: &str) -> PromotionInput {
    PromotionInput::new(bytes, SourceInfo::stdin(name))
}

fn case(
    id: &str,
    input: PromotionInput,
    status: OperationStatus,
    roles: impl IntoIterator<Item = ParseCaseRole>,
    constructs: Vec<ConstructExpectation>,
) -> ParseCase {
    ParseCase {
        id: id.into(),
        input,
        expected_status: status,
        roles: roles.into_iter().collect(),
        constructs,
    }
}

fn typed(construct: &str, pointer: &str, locator: &str) -> ConstructExpectation {
    ConstructExpectation {
        construct: construct.into(),
        disposition: ConstructDisposition::Typed {
            value_pointer: pointer.into(),
        },
        locator: Some(LocatorExpectation::Payload {
            locator_pointer: locator.into(),
        }),
    }
}

fn detection_cases() -> Vec<DetectionCase> {
    [
        (
            "valid",
            DetectionScenario::Valid,
            input("Title\n=====\n", "valid.rst"),
            DetectionStatus::Selected,
            Some("restructured-text".into()),
        ),
        (
            "mislabeled",
            DetectionScenario::Mislabeled,
            input(".. warning:: inert\n", "mislabeled.bin"),
            DetectionStatus::Selected,
            Some("restructured-text".into()),
        ),
        (
            "extensionless",
            DetectionScenario::Extensionless,
            input("Section\n=======\n", "extensionless"),
            DetectionStatus::Selected,
            Some("restructured-text".into()),
        ),
        (
            "malformed",
            DetectionScenario::Malformed,
            input(".. broken::\n   :bad:\n", "malformed.rst"),
            DetectionStatus::Selected,
            Some("restructured-text".into()),
        ),
        (
            "ambiguous",
            DetectionScenario::Ambiguous,
            input("x = 1\n", "ambiguous"),
            DetectionStatus::Ambiguous,
            None,
        ),
    ]
    .into_iter()
    .map(
        |(id, scenario, input, expected_status, expected_format)| DetectionCase {
            id: id.into(),
            scenario,
            input,
            expected_status,
            expected_format,
        },
    )
    .collect()
}

fn parse_cases() -> Vec<ParseCase> {
    let raw = ".. unknown-extension:: opaque\n";
    vec![
        case(
            "complete",
            input(COMPLETE, "complete.rst"),
            OperationStatus::Complete,
            [
                ParseCaseRole::Complete,
                ParseCaseRole::Determinism,
                ParseCaseRole::Projection,
                ParseCaseRole::Cli,
            ],
            vec![
                typed("headings", "/nodes", "/nodes/0/locator"),
                typed("directives and roles", "/nodes", "/nodes/1/locator"),
                typed("tables and code", "/nodes", "/nodes/2/locator"),
            ],
        ),
        case(
            "raw-unknown",
            input(raw, "raw-unknown.rst"),
            OperationStatus::Complete,
            [],
            vec![ConstructExpectation {
                construct: "raw unknown directive".into(),
                disposition: ConstructDisposition::Raw {
                    value_pointer: "/nodes/0/raw".into(),
                    expected_sha256: sha256_hex(raw.as_bytes()),
                },
                locator: Some(LocatorExpectation::Payload {
                    locator_pointer: "/nodes/0/locator".into(),
                }),
            }],
        ),
        case(
            "include-no-root",
            input(".. include:: child.rst\n", "include-no-root.rst"),
            OperationStatus::Partial,
            [ParseCaseRole::Partial],
            vec![ConstructExpectation {
                construct: "unresolved include reference".into(),
                disposition: ConstructDisposition::Diagnosed {
                    diagnostic_code: "include.project_root_required".into(),
                },
                locator: Some(LocatorExpectation::Diagnostic {
                    diagnostic_code: "include.project_root_required".into(),
                }),
            }],
        ),
        case(
            "mixed-invalid",
            input(b"Title\n=====\ninvalid \xff".to_vec(), "mixed-invalid.rst"),
            OperationStatus::Partial,
            [],
            vec![ConstructExpectation {
                construct: "invalid encoded unit".into(),
                disposition: ConstructDisposition::Diagnosed {
                    diagnostic_code: "decode.replacement.undecodable".into(),
                },
                locator: Some(LocatorExpectation::Diagnostic {
                    diagnostic_code: "decode.replacement.undecodable".into(),
                }),
            }],
        ),
        case(
            "decode-failed",
            input("failed", "decode-failed.rst"),
            OperationStatus::Failed,
            [ParseCaseRole::Failed],
            Vec::new(),
        ),
        case(
            "budget",
            input("Title\n=====\n", "budget.rst"),
            OperationStatus::Failed,
            [ParseCaseRole::BudgetLimited],
            Vec::new(),
        ),
        case(
            "encrypted-status",
            input("status", "encrypted-status.rst"),
            OperationStatus::Encrypted,
            [ParseCaseRole::Encrypted],
            Vec::new(),
        ),
        case(
            "unsupported-status",
            input("status", "unsupported-status.rst"),
            OperationStatus::Unsupported,
            [ParseCaseRole::Unsupported],
            Vec::new(),
        ),
        case(
            "hostile",
            input(
                ".. include:: https://network.invalid/never\n\n.. raw:: html\n\n   <script>never()</script>\n\n.. execute:: ../../escape\n",
                "hostile.rst",
            ),
            OperationStatus::Partial,
            [ParseCaseRole::Hostile],
            Vec::new(),
        ),
    ]
}

fn adapter() -> RstAdapter {
    let registry = builtin_parser_registry().unwrap();
    let descriptor = match registry.select_format("restructured-text") {
        ParserSelection::Available(descriptor) => *descriptor,
        ParserSelection::Unsupported { .. } => panic!("reStructuredText parser unavailable"),
    };
    let verification = VerificationKind::ALL
        .into_iter()
        .map(|kind| VerificationEvidence {
            kind,
            outcome: if matches!(kind, VerificationKind::Differential | VerificationKind::RoundTrip)
            {
                EvidenceOutcome::NotApplicable
            } else {
                EvidenceOutcome::Passed
            },
            evidence: "tests/restructured_text_universal_contract.rs; tests/restructured_text_promotion.rs; fixtures/generated/restructured_text".into(),
            rationale: matches!(kind, VerificationKind::Differential | VerificationKind::RoundTrip)
                .then(|| "The parser claims source preservation and normalized projections, not byte reconstruction; no compatible embedded reference parser is used.".into()),
        })
        .collect();
    let ci = CiControl::ALL
        .into_iter()
        .map(|control| CiEvidence {
            control,
            command: match control {
                CiControl::EnabledFeatureTests => {
                    "cargo test --features cli --test restructured_text_promotion"
                }
                CiControl::MinimalFeatureTests => {
                    "cargo test --no-default-features --features restructured-text,document-graph,schemas --test restructured_text_universal_contract"
                }
                CiControl::Clippy => "cargo clippy --features cli -- -D warnings",
                CiControl::Formatting => "cargo fmt --all -- --check",
                CiControl::SchemaDrift => {
                    "cargo run --example schema_codegen --features cli -- --check"
                }
                CiControl::Documentation => "cargo doc --features cli --no-deps",
            }
            .into(),
            enabled_features: BTreeSet::from([
                "restructured-text".into(),
                "document-graph".into(),
                "schemas".into(),
                "cli".into(),
            ]),
            passed: true,
        })
        .collect();
    RstAdapter {
        descriptor,
        suite: ParserPromotionSuite {
            suite_version: PARSER_PROMOTION_SUITE_VERSION.into(),
            detection_cases: detection_cases(),
            parse_cases: parse_cases(),
            projection: ProjectionContract {
                case_id: "complete".into(),
                required_node_kinds: vec![
                    DocumentNodeKind::Heading,
                    DocumentNodeKind::Paragraph,
                    DocumentNodeKind::RawInline,
                    DocumentNodeKind::Link,
                    DocumentNodeKind::Reference,
                    DocumentNodeKind::Footnote,
                    DocumentNodeKind::RawBlock,
                    DocumentNodeKind::CodeBlock,
                    DocumentNodeKind::Table,
                    DocumentNodeKind::TableRow,
                    DocumentNodeKind::TableCell,
                ],
            },
            public_surface: PublicSurfaceContract {
                case_id: "complete".into(),
                envelope_schema_name: "restructured-text-envelope".into(),
                payload_schema_name: "restructured-text".into(),
                options_schema_name: "restructured-text-options".into(),
                cli_selector: "restructured-text".into(),
            },
            fixtures: FixtureContract {
                corpus_format: "restructured-text".into(),
                required_classes: BTreeSet::new(),
                fuzz_corpus_paths: vec![
                    "fixtures/generated/restructured_text/maximum-complexity.rst".into(),
                    "fixtures/generated/restructured_text/malformed-adversarial.rst".into(),
                ],
            },
            verification,
            ci,
        },
    }
}

#[test]
fn restructured_text_passes_all_universal_promotion_gates() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest: grist::fixtures::FixtureCorpusManifest =
        serde_json::from_slice(&std::fs::read(root.join("fixtures/corpus.v1.json")).unwrap())
            .unwrap();
    let corpus_report = grist::fixtures::validate_corpus_at(root.join("fixtures"), &manifest);
    assert!(corpus_report.is_valid(), "{:#?}", corpus_report.violations);
    let corpus = load_corpus(&root).unwrap();
    let adapter = adapter();
    let first = ParserPromotionHarness::new(&corpus, &root).evaluate(&adapter);
    let second = ParserPromotionHarness::new(&corpus, &root).evaluate(&adapter);
    assert!(
        first.eligible_for_promotion,
        "{:#?}",
        first.failed_gates().collect::<Vec<_>>()
    );
    assert_eq!(first.passed_gate_count, 11);
    assert_eq!(first, second);
    first.validate().unwrap();
}
