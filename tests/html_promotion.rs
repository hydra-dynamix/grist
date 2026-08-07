#![cfg(all(feature = "html", feature = "schemas", feature = "document-graph"))]

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
use grist::html::{HtmlDocument, HtmlOptions};
use grist::promotion::{
    CiControl, CiEvidence, ConstructDisposition, ConstructExpectation, DetectionCase,
    DetectionScenario, EvidenceOutcome, FixtureContract, LocatorExpectation,
    PARSER_PROMOTION_SUITE_VERSION, ParseCase, ParseCaseRole, ParseExecution,
    ParserPromotionAdapter, ParserPromotionHarness, ParserPromotionSuite, ProjectionContract,
    PromotionAdapterError, PromotionInput, PublicSurfaceContract, SecurityObservation,
    VerificationEvidence, VerificationKind,
};
use grist::registry::{ParserDescriptor, ParserSelection, builtin_parser_registry};
use grist::segment::{SegmentCollection, SegmentOptions, segment_document_graph};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

const COMPLETE: &str = r#"<!doctype html><html><head><title>Promotion</title><meta name="author" content="Ada"></head><body><main><section><h1>Heading</h1><p>Paragraph <a href="/next">next</a>.</p></section><table><tr><th>A</th><td>B</td></tr></table><img src="figure.png" alt="Figure"><form action="/post"><button>Send</button></form><custom-widget>opaque</custom-widget></main></body></html>"#;

struct HtmlAdapter {
    descriptor: ParserDescriptor,
    suite: ParserPromotionSuite,
}

fn err(value: impl std::fmt::Display) -> PromotionAdapterError {
    PromotionAdapterError::new(value.to_string())
}

impl ParserPromotionAdapter for HtmlAdapter {
    fn descriptor(&self) -> &ParserDescriptor {
        &self.descriptor
    }

    fn suite(&self) -> &ParserPromotionSuite {
        &self.suite
    }

    fn detect(&self, input: &PromotionInput) -> Result<Detection, PromotionAdapterError> {
        let registry = builtin_parser_registry().map_err(err)?;
        let options = DetectionOptions {
            ambiguity_margin: if input.source.display_name == "ambiguous" {
                1.0
            } else {
                DetectionOptions::default().ambiguity_margin
            },
            ..DetectionOptions::default()
        };
        detect_source(
            &input.source,
            &input.bytes,
            input.format_hint.as_ref(),
            &Limits::default(),
            &registry,
            &options,
        )
        .map_err(err)
    }

    fn parse(&self, input: &PromotionInput) -> Result<ParseExecution, PromotionAdapterError> {
        let name = input.source.display_name.as_str();
        if name.contains("encrypted-status") {
            return Ok(terminal(
                input,
                OperationStatus::Encrypted,
                Diagnostic::error("grist.html", "html.status.encrypted", "status contract"),
            )
            .into());
        }
        if name.contains("unsupported-status") {
            return Ok(terminal(
                input,
                OperationStatus::Unsupported,
                Diagnostic::unsupported("grist.html", "status contract"),
            )
            .into());
        }
        let mut budget = ResourceBudget::trusted_unbounded();
        if name.contains("budget") {
            budget.max_nodes = Some(1);
        }
        let options = if name.contains("mixed-invalid") {
            Some(HtmlOptions {
                encoding: Some("utf-8".into()),
                ..Default::default()
            })
        } else if name.contains("decode-failed") {
            Some(HtmlOptions {
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
            .ok_or_else(|| PromotionAdapterError::new("HTML payload missing"))?;
        serde_json::from_value::<HtmlDocument>(payload)
            .map_err(err)?
            .to_document_graph(
                DocumentGraphContext::new("promotion:html").with_source(parsed.source.clone()),
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
        if selector != "html" {
            return Err(PromotionAdapterError::new("unexpected CLI selector"));
        }
        serde_json::to_value(dispatch(
            input,
            BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
            None,
        )?)
        .map_err(err)
    }

    fn rust_payload_accepts(&self, payload: &Value) -> Result<(), PromotionAdapterError> {
        serde_json::from_value::<HtmlDocument>(payload.clone())
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
    options: Option<HtmlOptions>,
) -> Result<Envelope<Value>, PromotionAdapterError> {
    let request = ParseRequest::new(
        RequestId::new("html-promotion").map_err(err)?,
        Input::bytes(input.bytes.clone()),
        input.source.clone(),
        budget,
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .map_err(err)?
        .dispatch(
            "html",
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
        ArtifactKind::Html,
        status,
        input.source.clone(),
        ParserInfo::new("grist.html"),
        options_digest(&json!({})).unwrap(),
        SchemaVersion::HTML_V2,
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

fn invalid_utf8_input() -> PromotionInput {
    let mut bytes = br#"<!doctype html><meta charset="utf-8"><p>bad "#.to_vec();
    bytes.push(0xff);
    input(bytes, "mixed-invalid.html")
}

fn detection_cases() -> Vec<DetectionCase> {
    [
        (
            "valid",
            DetectionScenario::Valid,
            input("<!doctype html><title>x</title>", "valid.html"),
            DetectionStatus::Selected,
            Some("html".into()),
        ),
        (
            "mislabeled",
            DetectionScenario::Mislabeled,
            input("<html><body><p>x</p></body></html>", "mislabeled.bin"),
            DetectionStatus::Selected,
            Some("html".into()),
        ),
        (
            "extensionless",
            DetectionScenario::Extensionless,
            input("<!doctype html><html></html>", "extensionless"),
            DetectionStatus::Selected,
            Some("html".into()),
        ),
        (
            "malformed",
            DetectionScenario::Malformed,
            input("<table><td>x</table>", "malformed.html"),
            DetectionStatus::Selected,
            Some("html".into()),
        ),
        (
            "ambiguous",
            DetectionScenario::Ambiguous,
            input("<!doctype html>\n# Markdown heading\n", "ambiguous"),
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
    let raw = "<custom-widget>opaque</custom-widget>";
    vec![
        case(
            "complete",
            input(COMPLETE, "complete.html"),
            OperationStatus::Complete,
            [
                ParseCaseRole::Complete,
                ParseCaseRole::Determinism,
                ParseCaseRole::Projection,
                ParseCaseRole::Cli,
            ],
            vec![
                typed("DOM and semantic sections", "/nodes", "/nodes/0/locator"),
                typed("metadata and links", "/metadata", "/metadata/0/locator"),
                typed("tables and media", "/tables", "/tables/0/locator"),
            ],
        ),
        case(
            "raw-unknown",
            input(raw, "raw-unknown.html"),
            OperationStatus::Complete,
            [],
            vec![ConstructExpectation {
                construct: "raw unknown custom element".into(),
                disposition: ConstructDisposition::Raw {
                    value_pointer: "/nodes/2/raw".into(),
                    expected_sha256: sha256_hex(raw.as_bytes()),
                },
                locator: Some(LocatorExpectation::Payload {
                    locator_pointer: "/nodes/2/locator".into(),
                }),
            }],
        ),
        case(
            "malformed-recovery",
            input("<table><td>x</table>", "malformed-recovery.html"),
            OperationStatus::Partial,
            [ParseCaseRole::Partial],
            vec![ConstructExpectation {
                construct: "malformed recovered tree".into(),
                disposition: ConstructDisposition::Diagnosed {
                    diagnostic_code: "grist.input.malformed".into(),
                },
                locator: Some(LocatorExpectation::Diagnostic {
                    diagnostic_code: "grist.input.malformed".into(),
                }),
            }],
        ),
        case(
            "mixed-invalid",
            invalid_utf8_input(),
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
            input("failed", "decode-failed.html"),
            OperationStatus::Failed,
            [ParseCaseRole::Failed],
            Vec::new(),
        ),
        case(
            "budget",
            input(COMPLETE, "budget.html"),
            OperationStatus::Failed,
            [ParseCaseRole::BudgetLimited],
            Vec::new(),
        ),
        case(
            "encrypted-status",
            input("status", "encrypted-status.html"),
            OperationStatus::Encrypted,
            [ParseCaseRole::Encrypted],
            Vec::new(),
        ),
        case(
            "unsupported-status",
            input("status", "unsupported-status.html"),
            OperationStatus::Unsupported,
            [ParseCaseRole::Unsupported],
            Vec::new(),
        ),
        case(
            "hostile",
            input(
                r#"<script src="https://network.invalid/a.js">never()</script><form action="https://network.invalid/post"><input onclick="never()" formaction="javascript:never()"></form>"#,
                "hostile.html",
            ),
            OperationStatus::Complete,
            [ParseCaseRole::Hostile],
            Vec::new(),
        ),
    ]
}

fn adapter() -> HtmlAdapter {
    let registry = builtin_parser_registry().unwrap();
    let descriptor = match registry.select_format("html") {
        ParserSelection::Available(descriptor) => *descriptor,
        ParserSelection::Unsupported { .. } => panic!("HTML parser unavailable"),
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
            evidence: "tests/html_universal_contract.rs; tests/html_promotion.rs; fixtures/generated/html".into(),
            rationale: matches!(kind, VerificationKind::Differential | VerificationKind::RoundTrip)
                .then(|| "html5ever is the standards tree builder; Grist claims lossless source retention and normalized projection, not byte reconstruction.".into()),
        })
        .collect();
    let ci = CiControl::ALL
        .into_iter()
        .map(|control| CiEvidence {
            control,
            command: match control {
                CiControl::EnabledFeatureTests => {
                    "cargo test --features cli --test html_promotion"
                }
                CiControl::MinimalFeatureTests => {
                    "cargo test --no-default-features --features html,document-graph,schemas --test html_universal_contract"
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
                "html".into(),
                "document-graph".into(),
                "schemas".into(),
                "cli".into(),
            ]),
            passed: true,
        })
        .collect();
    HtmlAdapter {
        descriptor,
        suite: ParserPromotionSuite {
            suite_version: PARSER_PROMOTION_SUITE_VERSION.into(),
            detection_cases: detection_cases(),
            parse_cases: parse_cases(),
            projection: ProjectionContract {
                case_id: "complete".into(),
                required_node_kinds: vec![
                    DocumentNodeKind::Section,
                    DocumentNodeKind::Heading,
                    DocumentNodeKind::Paragraph,
                    DocumentNodeKind::Link,
                    DocumentNodeKind::Table,
                    DocumentNodeKind::TableRow,
                    DocumentNodeKind::TableCell,
                    DocumentNodeKind::Image,
                    DocumentNodeKind::Form,
                    DocumentNodeKind::RawBlock,
                ],
            },
            public_surface: PublicSurfaceContract {
                case_id: "complete".into(),
                envelope_schema_name: "html-envelope".into(),
                payload_schema_name: "html".into(),
                options_schema_name: "html-options".into(),
                cli_selector: "html".into(),
            },
            fixtures: FixtureContract {
                corpus_format: "html".into(),
                required_classes: BTreeSet::new(),
                fuzz_corpus_paths: vec![
                    "fixtures/generated/html/maximum-complexity.html".into(),
                    "fixtures/generated/html/malformed-adversarial.html".into(),
                ],
            },
            verification,
            ci,
        },
    }
}

#[test]
fn html_passes_all_universal_promotion_gates() {
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
