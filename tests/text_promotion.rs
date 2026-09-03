#![cfg(all(feature = "cli", feature = "schemas"))]

use grist::core::{
    ArtifactKind, BudgetSelection, ContentIdentity, Diagnostic, Envelope, Input, Limits,
    OperationKind, OperationStatus, ParseRequest, ParserInfo, ProviderSet, RequestId,
    ResourceBudget, SchemaVersion, SourceInfo, options_digest,
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
use grist::segment::{SegmentCollection, SegmentOptions, segment_document_graph};
use grist::text::{TextDocument, TextOptions};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

struct TextAdapter {
    descriptor: ParserDescriptor,
    suite: ParserPromotionSuite,
}

impl ParserPromotionAdapter for TextAdapter {
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
                Diagnostic::error("grist.text", "text.status.encrypted", "status contract"),
            )
            .into());
        }
        if name.contains("unsupported-status") {
            return Ok(terminal(
                input,
                OperationStatus::Unsupported,
                Diagnostic::unsupported("grist.text", "status contract"),
            )
            .into());
        }
        let mut budget = ResourceBudget::trusted_unbounded();
        if name.contains("budget") {
            budget.max_decoded_characters = Some(1);
        }
        let options = if name.contains("invalid") {
            Some(TextOptions {
                encoding: Some("utf-8".into()),
            })
        } else if name.contains("malformed-decode") {
            Some(TextOptions {
                encoding: Some("x-grist-unsupported-charset".into()),
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
            .ok_or_else(|| PromotionAdapterError::new("text payload missing"))?;
        serde_json::from_value::<TextDocument>(payload)
            .map_err(err)?
            .to_document_graph(
                DocumentGraphContext::new("promotion:text").with_source(parsed.source.clone()),
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
            .ok_or_else(|| PromotionAdapterError::new("text identity missing"))?;
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
        let envelope = grist::cli::parse_bytes(
            selector,
            input.bytes.clone(),
            input.source.clone(),
            RequestId::new("text-promotion").map_err(err)?,
            None,
        )
        .map_err(err)?;
        serde_json::to_value(envelope).map_err(err)
    }

    fn rust_payload_accepts(&self, payload: &Value) -> Result<(), PromotionAdapterError> {
        serde_json::from_value::<TextDocument>(payload.clone())
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

fn err(value: impl std::fmt::Display) -> PromotionAdapterError {
    PromotionAdapterError::new(value.to_string())
}

fn dispatch(
    input: &PromotionInput,
    budget: BudgetSelection,
    options: Option<TextOptions>,
) -> Result<Envelope<Value>, PromotionAdapterError> {
    let request = ParseRequest::new(
        RequestId::new("text-promotion").map_err(err)?,
        Input::bytes(input.bytes.clone()),
        input.source.clone(),
        budget,
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .map_err(err)?
        .dispatch(
            "text",
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
        ArtifactKind::Text,
        status,
        input.source.clone(),
        ParserInfo::new("grist.text"),
        options_digest(&json!({})).unwrap(),
        SchemaVersion::TEXT_V2,
    )
    .unwrap()
    .with_identity(ContentIdentity::for_raw_bytes(&input.bytes))
    .with_diagnostics(vec![diagnostic])
}

fn input(bytes: impl Into<Vec<u8>>, name: &str) -> PromotionInput {
    PromotionInput::new(bytes, SourceInfo::stdin(name))
}

fn utf16(text: &str, be: bool) -> Vec<u8> {
    let mut bytes = if be {
        vec![0xfe, 0xff]
    } else {
        vec![0xff, 0xfe]
    };
    for unit in text.encode_utf16() {
        bytes.extend(if be {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        });
    }
    bytes
}

fn utf32(text: &str, be: bool) -> Vec<u8> {
    let mut bytes = if be {
        vec![0, 0, 0xfe, 0xff]
    } else {
        vec![0xff, 0xfe, 0, 0]
    };
    for character in text.chars() {
        bytes.extend(if be {
            (character as u32).to_be_bytes()
        } else {
            (character as u32).to_le_bytes()
        });
    }
    bytes
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

fn decoded(name: &str) -> ConstructExpectation {
    ConstructExpectation {
        construct: name.into(),
        disposition: ConstructDisposition::Typed {
            value_pointer: "/decoded_text".into(),
        },
        locator: Some(LocatorExpectation::Payload {
            locator_pointer: "/locator".into(),
        }),
    }
}

fn detection_cases() -> Vec<DetectionCase> {
    [
        (
            "valid",
            DetectionScenario::Valid,
            input(b"plain text", "valid.txt"),
            DetectionStatus::Selected,
            Some("text".into()),
        ),
        (
            "mislabeled",
            DetectionScenario::Mislabeled,
            input(b"plain text", "mislabeled.bin"),
            DetectionStatus::Selected,
            Some("text".into()),
        ),
        (
            "extensionless",
            DetectionScenario::Extensionless,
            input(b"plain text", "extensionless"),
            DetectionStatus::Selected,
            Some("text".into()),
        ),
        (
            "malformed",
            DetectionScenario::Malformed,
            input(vec![b'A', 0x81, b'B'], "malformed.txt"),
            DetectionStatus::Selected,
            Some("text".into()),
        ),
        (
            "ambiguous",
            DetectionScenario::Ambiguous,
            input(b"x = 1\n", "ambiguous"),
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
    let complete = case(
        "complete",
        input(b"first\r\ncontinued\r\n\r\nsecond", "complete.txt"),
        OperationStatus::Complete,
        [
            ParseCaseRole::Complete,
            ParseCaseRole::Determinism,
            ParseCaseRole::Projection,
            ParseCaseRole::Cli,
        ],
        vec![
            decoded("decoded plain text"),
            ConstructExpectation {
                construct: "original bytes".into(),
                disposition: ConstructDisposition::Typed {
                    value_pointer: "/raw_bytes".into(),
                },
                locator: Some(LocatorExpectation::Payload {
                    locator_pointer: "/locator".into(),
                }),
            },
        ],
    );
    vec![
        complete,
        case(
            "utf16le",
            input(utf16("utf16\r\ntext", false), "utf16le.txt"),
            OperationStatus::Complete,
            [],
            Vec::new(),
        ),
        case(
            "utf16be",
            input(utf16("utf16\r\ntext", true), "utf16be.txt"),
            OperationStatus::Complete,
            [],
            Vec::new(),
        ),
        case(
            "utf32le",
            input(utf32("utf32\ntext", false), "utf32le.txt"),
            OperationStatus::Complete,
            [],
            Vec::new(),
        ),
        case(
            "utf32be",
            input(utf32("utf32\ntext", true), "utf32be.txt"),
            OperationStatus::Complete,
            [],
            Vec::new(),
        ),
        case(
            "windows1252",
            PromotionInput::new(
                b"left \x93quote\x94 right".to_vec(),
                SourceInfo::stdin("windows1252.txt")
                    .with_declared_mime_type("text/plain; charset=windows-1252"),
            ),
            OperationStatus::Complete,
            [],
            Vec::new(),
        ),
        case(
            "mixed-invalid",
            input(b"before \xff after".to_vec(), "mixed-invalid.txt"),
            OperationStatus::Partial,
            [ParseCaseRole::Partial],
            vec![ConstructExpectation {
                construct: "malformed byte".into(),
                disposition: ConstructDisposition::Diagnosed {
                    diagnostic_code: "decode.replacement.undecodable".into(),
                },
                locator: Some(LocatorExpectation::Diagnostic {
                    diagnostic_code: "decode.replacement.undecodable".into(),
                }),
            }],
        ),
        case(
            "empty",
            input(Vec::new(), "empty.txt"),
            OperationStatus::Complete,
            [],
            Vec::new(),
        ),
        case(
            "large",
            input("large line\n".repeat(8192), "large.txt"),
            OperationStatus::Complete,
            [],
            Vec::new(),
        ),
        case(
            "newline-variants",
            input(b"lf\ncrlf\r\ncr\rend", "newlines.txt"),
            OperationStatus::Complete,
            [],
            Vec::new(),
        ),
        case(
            "malformed-decode",
            input(b"text", "malformed-decode.txt"),
            OperationStatus::Failed,
            [ParseCaseRole::Failed],
            Vec::new(),
        ),
        case(
            "budget",
            input(b"budget", "budget.txt"),
            OperationStatus::Failed,
            [ParseCaseRole::BudgetLimited],
            Vec::new(),
        ),
        case(
            "encrypted-status",
            input(b"status", "encrypted-status.txt"),
            OperationStatus::Encrypted,
            [ParseCaseRole::Encrypted],
            Vec::new(),
        ),
        case(
            "unsupported-status",
            input(b"status", "unsupported-status.txt"),
            OperationStatus::Unsupported,
            [ParseCaseRole::Unsupported],
            Vec::new(),
        ),
        case(
            "hostile",
            input(b"<script>never()</script>\n../../path", "hostile.txt"),
            OperationStatus::Complete,
            [ParseCaseRole::Hostile],
            Vec::new(),
        ),
    ]
}

fn adapter() -> TextAdapter {
    let registry = builtin_parser_registry().unwrap();
    let descriptor = match registry.select_format("text") {
        ParserSelection::Available(descriptor) => *descriptor,
        ParserSelection::Unsupported { .. } => panic!("text parser unavailable"),
    };
    let verification = VerificationKind::ALL
        .into_iter()
        .map(|kind| VerificationEvidence {
            kind,
            outcome: if kind == VerificationKind::Differential {
                EvidenceOutcome::NotApplicable
            } else {
                EvidenceOutcome::Passed
            },
            evidence: "tests/text_universal_contract.rs; tests/text_promotion.rs".into(),
            rationale: (kind == VerificationKind::Differential)
                .then(|| "Plain text has no more authoritative structural reader.".into()),
        })
        .collect();
    let ci = CiControl::ALL
        .into_iter()
        .map(|control| CiEvidence {
            control,
            command: match control {
                CiControl::MinimalFeatureTests => {
                    "cargo test --no-default-features --features schemas --test text_universal_contract"
                }
                CiControl::Clippy => "cargo clippy --features cli -- -D warnings",
                CiControl::Formatting => "cargo fmt --all -- --check",
                CiControl::SchemaDrift => {
                    "cargo run --example schema_codegen --features cli -- --check"
                }
                CiControl::Documentation => "cargo doc --features cli --no-deps",
                CiControl::EnabledFeatureTests => "cargo test --features cli --test text_promotion",
            }
            .into(),
            enabled_features: BTreeSet::from(["cli".into(), "schemas".into()]),
            passed: true,
        })
        .collect();
    TextAdapter {
        descriptor,
        suite: ParserPromotionSuite {
            suite_version: PARSER_PROMOTION_SUITE_VERSION.into(),
            detection_cases: detection_cases(),
            parse_cases: parse_cases(),
            projection: ProjectionContract {
                case_id: "complete".into(),
                required_node_kinds: vec![DocumentNodeKind::Paragraph],
            },
            public_surface: PublicSurfaceContract {
                case_id: "complete".into(),
                envelope_schema_name: "text-envelope".into(),
                payload_schema_name: "text".into(),
                options_schema_name: "text-options".into(),
                cli_selector: "text".into(),
            },
            fixtures: FixtureContract {
                corpus_format: "text".into(),
                required_classes: BTreeSet::new(),
                fuzz_corpus_paths: vec!["fixtures/generated/text/mixed-invalid.bin".into()],
            },
            verification,
            ci,
        },
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn plain_text_passes_all_universal_promotion_gates() {
    let corpus = load_corpus(repository_root()).unwrap();
    let adapter = adapter();
    let first = ParserPromotionHarness::new(&corpus, repository_root()).evaluate(&adapter);
    let second = ParserPromotionHarness::new(&corpus, repository_root()).evaluate(&adapter);

    assert!(
        first.eligible_for_promotion,
        "{:#?}",
        first.failed_gates().collect::<Vec<_>>()
    );
    assert_eq!(first.passed_gate_count, 11);
    assert_eq!(first, second);
    first.validate().unwrap();
    let schema = grist::schema::validate_schema(
        "parser-conformance-report",
        &serde_json::to_value(&first).unwrap(),
    )
    .unwrap();
    assert!(schema.valid, "{:?}", schema.issues);
}
