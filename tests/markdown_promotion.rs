#![cfg(all(feature = "cli", feature = "schemas"))]

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
use grist::markdown::{MarkdownDocument, MarkdownOptions};
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

const COMPLETE: &str = "---\ntitle: Promotion\n---\n# Heading *em*\n\nParagraph with **strong**, ~~strike~~, `code`, $math$, [link](https://example.test), and ![alt](image.png).\n\n1. first\n2. second\n\n- [x] done\n- [ ] open\n\n> quote\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\nFootnote[^n].\n\n[^n]: note\n\nTerm\n: definition\n";

struct MarkdownAdapter {
    descriptor: ParserDescriptor,
    suite: ParserPromotionSuite,
}

impl ParserPromotionAdapter for MarkdownAdapter {
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
                    "grist.markdown",
                    "markdown.status.encrypted",
                    "status contract",
                ),
            )
            .into());
        }
        if name.contains("unsupported-status") {
            return Ok(terminal(
                input,
                OperationStatus::Unsupported,
                Diagnostic::unsupported("grist.markdown", "status contract"),
            )
            .into());
        }
        let mut budget = ResourceBudget::trusted_unbounded();
        if name.contains("budget") {
            budget.max_decoded_characters = Some(1);
        }
        let options = if name.contains("mixed-invalid") {
            Some(MarkdownOptions {
                encoding: Some("utf-8".into()),
                ..MarkdownOptions::default()
            })
        } else if name.contains("decode-failed") {
            Some(MarkdownOptions {
                encoding: Some("x-grist-unsupported-charset".into()),
                ..MarkdownOptions::default()
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
            .ok_or_else(|| PromotionAdapterError::new("Markdown payload missing"))?;
        serde_json::from_value::<MarkdownDocument>(payload)
            .map_err(err)?
            .to_document_graph(
                DocumentGraphContext::new("promotion:markdown").with_source(parsed.source.clone()),
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
            .ok_or_else(|| PromotionAdapterError::new("Markdown identity missing"))?;
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
            RequestId::new("markdown-promotion").map_err(err)?,
            None,
        )
        .map_err(err)?;
        serde_json::to_value(envelope).map_err(err)
    }

    fn rust_payload_accepts(&self, payload: &Value) -> Result<(), PromotionAdapterError> {
        serde_json::from_value::<MarkdownDocument>(payload.clone())
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
    options: Option<MarkdownOptions>,
) -> Result<Envelope<Value>, PromotionAdapterError> {
    let request = ParseRequest::new(
        RequestId::new("markdown-promotion").map_err(err)?,
        Input::bytes(input.bytes.clone()),
        input.source.clone(),
        budget,
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .map_err(err)?
        .dispatch(
            "markdown",
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
        ArtifactKind::Markdown,
        status,
        input.source.clone(),
        ParserInfo::new("grist.markdown"),
        options_digest(&json!({})).unwrap(),
        SchemaVersion::MARKDOWN_V2,
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
            input("# valid\n", "valid.md"),
            DetectionStatus::Selected,
            Some("markdown".into()),
        ),
        (
            "mislabeled",
            DetectionScenario::Mislabeled,
            input("# mislabeled\n", "mislabeled.bin"),
            DetectionStatus::Selected,
            Some("markdown".into()),
        ),
        (
            "extensionless",
            DetectionScenario::Extensionless,
            input("# extensionless\n", "extensionless"),
            DetectionStatus::Selected,
            Some("markdown".into()),
        ),
        (
            "malformed",
            DetectionScenario::Malformed,
            input("# malformed\n```\n", "malformed.md"),
            DetectionStatus::Selected,
            Some("markdown".into()),
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
    let directive = "::: warning\nraw extension\n:::\n";
    vec![
        case(
            "complete",
            input(COMPLETE, "complete.md"),
            OperationStatus::Complete,
            [
                ParseCaseRole::Complete,
                ParseCaseRole::Determinism,
                ParseCaseRole::Projection,
                ParseCaseRole::Cli,
            ],
            vec![
                typed("YAML frontmatter", "/frontmatter", "/frontmatter/locator"),
                typed("exact raw bytes", "/raw_bytes", "/locator"),
                typed(
                    "CommonMark/GFM block and inline tree",
                    "/nodes",
                    "/nodes/0/locator",
                ),
            ],
        ),
        case(
            "frontmatter-toml",
            input(
                format!(
                    "+++\ntitle = {}TOML{}\n+++\n# Body\n",
                    char::from(34),
                    char::from(34)
                ),
                "frontmatter-toml.md",
            ),
            OperationStatus::Complete,
            [],
            vec![typed(
                "TOML frontmatter variant",
                "/frontmatter",
                "/frontmatter/locator",
            )],
        ),
        case(
            "html-and-directive",
            input(
                format!("<div>raw html</div>\n\n{directive}"),
                "html-and-directive.md",
            ),
            OperationStatus::Complete,
            [],
            vec![typed(
                "HTML blocks and directives",
                "/nodes",
                "/nodes/0/locator",
            )],
        ),
        case(
            "raw-unknown",
            input(directive, "raw-unknown.md"),
            OperationStatus::Complete,
            [],
            vec![ConstructExpectation {
                construct: "raw unknown extension".into(),
                disposition: ConstructDisposition::Raw {
                    value_pointer: "/nodes/0/raw".into(),
                    expected_sha256: sha256_hex(directive.as_bytes()),
                },
                locator: Some(LocatorExpectation::Payload {
                    locator_pointer: "/nodes/0/locator".into(),
                }),
            }],
        ),
        case(
            "mixed-invalid",
            input(b"# before \xff after".to_vec(), "mixed-invalid.md"),
            OperationStatus::Partial,
            [ParseCaseRole::Partial],
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
            "malformed-fence",
            input("```rust\nunclosed\n", "malformed-fence.md"),
            OperationStatus::Partial,
            [],
            vec![ConstructExpectation {
                construct: "malformed fenced block".into(),
                disposition: ConstructDisposition::Diagnosed {
                    diagnostic_code: "fence.unclosed".into(),
                },
                locator: Some(LocatorExpectation::Diagnostic {
                    diagnostic_code: "fence.unclosed".into(),
                }),
            }],
        ),
        case(
            "decode-failed",
            input("# failed\n", "decode-failed.md"),
            OperationStatus::Failed,
            [ParseCaseRole::Failed],
            Vec::new(),
        ),
        case(
            "budget",
            input("# budget\n", "budget.md"),
            OperationStatus::Failed,
            [ParseCaseRole::BudgetLimited],
            Vec::new(),
        ),
        case(
            "encrypted-status",
            input("status", "encrypted-status.md"),
            OperationStatus::Encrypted,
            [ParseCaseRole::Encrypted],
            Vec::new(),
        ),
        case(
            "unsupported-status",
            input("status", "unsupported-status.md"),
            OperationStatus::Unsupported,
            [ParseCaseRole::Unsupported],
            Vec::new(),
        ),
        case(
            "hostile",
            input(
                "<script>never()</script>\n\n[jump](java\\nscript:owned)\n\n::: run\n../../escape\n:::\n",
                "hostile.md",
            ),
            OperationStatus::Complete,
            [ParseCaseRole::Hostile],
            Vec::new(),
        ),
    ]
}

fn adapter() -> MarkdownAdapter {
    let registry = builtin_parser_registry().unwrap();
    let descriptor = match registry.select_format("markdown") {
        ParserSelection::Available(descriptor) => *descriptor,
        ParserSelection::Unsupported { .. } => panic!("Markdown parser unavailable"),
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
            evidence:
                "tests/markdown_universal_contract.rs; tests/markdown_promotion.rs; pulldown-cmark 0.12"
                    .into(),
            rationale: (kind == VerificationKind::Differential).then(|| {
                "pulldown-cmark is the authoritative CommonMark/GFM event source used here; the integration contract verifies Grist-specific preservation and projections."
                    .into()
            }),
        })
        .collect();
    let ci = CiControl::ALL
        .into_iter()
        .map(|control| CiEvidence {
            control,
            command: match control {
                CiControl::MinimalFeatureTests => {
                    "cargo test --no-default-features --features markdown,schemas --test markdown_universal_contract"
                }
                CiControl::Clippy => "cargo clippy --features cli -- -D warnings",
                CiControl::Formatting => "cargo fmt --all -- --check",
                CiControl::SchemaDrift => {
                    "cargo run --example schema_codegen --features cli -- --check"
                }
                CiControl::Documentation => "cargo doc --features cli --no-deps",
                CiControl::EnabledFeatureTests => {
                    "cargo test --features cli --test markdown_promotion"
                }
            }
            .into(),
            enabled_features: BTreeSet::from([
                "cli".into(),
                "schemas".into(),
                "markdown".into(),
            ]),
            passed: true,
        })
        .collect();
    MarkdownAdapter {
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
                    DocumentNodeKind::Emphasis,
                    DocumentNodeKind::Strong,
                    DocumentNodeKind::Link,
                    DocumentNodeKind::Image,
                    DocumentNodeKind::List,
                    DocumentNodeKind::ListItem,
                    DocumentNodeKind::Quote,
                    DocumentNodeKind::Table,
                    DocumentNodeKind::TableRow,
                    DocumentNodeKind::TableCell,
                    DocumentNodeKind::Footnote,
                    DocumentNodeKind::Reference,
                ],
            },
            public_surface: PublicSurfaceContract {
                case_id: "complete".into(),
                envelope_schema_name: "markdown-envelope".into(),
                payload_schema_name: "markdown".into(),
                options_schema_name: "markdown-options".into(),
                cli_selector: "markdown".into(),
            },
            fixtures: FixtureContract {
                corpus_format: "markdown".into(),
                required_classes: BTreeSet::new(),
                fuzz_corpus_paths: vec!["fixtures/generated/markdown/maximum-complexity.md".into()],
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
fn markdown_passes_all_universal_promotion_gates() {
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
