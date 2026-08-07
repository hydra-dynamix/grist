use grist::core::{
    ArtifactKind, ContentIdentity, DetectionCandidate, DetectionEvidence, DetectionEvidenceKind,
    Diagnostic, DiagnosticClass, Envelope, FormatIdentity, Hashes, OperationKind, OperationStatus,
    ParserAvailability, ParserInfo, SourceInfo, SourceLocator, SourceRange, options_digest,
};
use grist::detect::{ContentKind, Detection, DetectionStatus, FileKind};
use grist::document_graph::{DocumentGraph, DocumentKind, DocumentNode, DocumentNodeKind};
use grist::fixtures::{FixtureClass, FixtureCorpusManifest, load_corpus};
use grist::promotion::{
    CiControl, CiEvidence, ConstructDisposition, ConstructExpectation, DetectionCase,
    DetectionScenario, EvidenceOutcome, FixtureContract, LocatorExpectation,
    PARSER_PROMOTION_SUITE_VERSION, ParseCase, ParseCaseRole, ParseExecution,
    ParserPromotionAdapter, ParserPromotionHarness, ParserPromotionSuite, ProjectionContract,
    PromotionAdapterError, PromotionGate, PromotionInput, PublicSurfaceContract,
    SecurityObservation, VerificationEvidence, VerificationKind,
};
use grist::registry::{FormatMetadata, OptionsMetadata, ParserDescriptor, SchemaMetadata};
use grist::segment::{SegmentCollection, SegmentOptions, segment_document_graph};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

struct ReferenceAdapter {
    descriptor: ParserDescriptor,
    suite: ParserPromotionSuite,
    panic_on_hostile: bool,
}

impl ParserPromotionAdapter for ReferenceAdapter {
    fn descriptor(&self) -> &ParserDescriptor {
        &self.descriptor
    }

    fn suite(&self) -> &ParserPromotionSuite {
        &self.suite
    }

    fn detect(&self, input: &PromotionInput) -> Result<Detection, PromotionAdapterError> {
        let ambiguous = input.source.display_name.contains("ambiguous");
        let mut candidates = vec![candidate(1, "text", 0.9, Some(&self.descriptor.id))];
        if ambiguous {
            candidates.push(candidate(2, "markdown", 0.88, None));
        }
        Ok(Detection {
            file_kind: FileKind::Documentation,
            content_kind: ContentKind::Text,
            language: None,
            confidence: 0.9,
            reasons: vec!["reference structural evidence".into()],
            candidates,
            status: if ambiguous {
                DetectionStatus::Ambiguous
            } else {
                DetectionStatus::Selected
            },
            selected_parser: (!ambiguous).then(|| self.descriptor.id.clone()),
            diagnostics: Vec::new(),
        })
    }

    fn parse(&self, input: &PromotionInput) -> Result<ParseExecution, PromotionAdapterError> {
        let marker = std::str::from_utf8(&input.bytes).unwrap_or_default();
        if self.panic_on_hostile && marker == "hostile" {
            panic!("reference hostile panic");
        }
        let envelope = parse_marker(marker, input);
        Ok(if marker == "hostile" {
            ParseExecution::with_security_observation(envelope, SecurityObservation::default())
        } else {
            envelope.into()
        })
    }

    fn project(&self, parsed: &Envelope<Value>) -> Result<DocumentGraph, PromotionAdapterError> {
        Ok(reference_graph(parsed.source.clone()))
    }

    fn segment(
        &self,
        graph: &DocumentGraph,
        parsed: &Envelope<Value>,
    ) -> Result<SegmentCollection, PromotionAdapterError> {
        let identity = parsed
            .identity
            .as_ref()
            .ok_or_else(|| PromotionAdapterError::new("parse identity missing"))?;
        segment_document_graph(graph, identity, identity, &SegmentOptions::default(), None)
            .map_err(|error| PromotionAdapterError::new(error.to_string()))
    }

    fn cli_parse(
        &self,
        _selector: &str,
        input: &PromotionInput,
    ) -> Result<Value, PromotionAdapterError> {
        serde_json::to_value(parse_marker(
            std::str::from_utf8(&input.bytes).unwrap_or_default(),
            input,
        ))
        .map_err(|error| PromotionAdapterError::new(error.to_string()))
    }

    fn rust_payload_accepts(&self, payload: &Value) -> Result<(), PromotionAdapterError> {
        payload
            .get("typed")
            .and_then(|value| value.get("value"))
            .ok_or_else(|| PromotionAdapterError::new("typed payload field missing"))?;
        Ok(())
    }

    fn schema_json(&self, _name: &str) -> Option<Value> {
        Some(json!({"type": "object"}))
    }

    fn canonical_example(&self, _name: &str) -> Option<Value> {
        Some(json!({}))
    }
}

fn candidate(
    rank: u32,
    format: &str,
    confidence: f32,
    parser_id: Option<&str>,
) -> DetectionCandidate {
    let mut candidate = DetectionCandidate::new(
        rank,
        FormatIdentity::new(format, Some("text/plain")),
        confidence,
        vec![DetectionEvidence::new(
            DetectionEvidenceKind::Structure,
            "reference structural probe",
        )],
    );
    candidate.parser_availability = if parser_id.is_some() {
        ParserAvailability::Available
    } else {
        ParserAvailability::Unregistered
    };
    candidate.parser_id = parser_id.map(str::to_string);
    candidate
}

fn locator() -> SourceLocator {
    SourceLocator::try_from(SourceRange {
        byte_start: 0,
        byte_end: 5,
        start_line: 1,
        start_column: 1,
        end_line: 1,
        end_column: 6,
    })
    .unwrap()
}

fn parse_marker(marker: &str, input: &PromotionInput) -> Envelope<Value> {
    let parser = ParserInfo::new("reference.text");
    let digest = options_digest(&json!({})).unwrap();
    let identity = ContentIdentity::from(Hashes::for_bytes(
        &input.bytes,
        std::str::from_utf8(&input.bytes).ok(),
    ));
    let payload = json!({
        "typed": {"value": "hello", "locator": locator()},
        "raw": {"value": "<unknown>", "locator": locator()}
    });
    let envelope = match marker {
        "complete" | "hostile" => Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Text,
            input.source.clone(),
            parser,
            digest,
            "reference/payload/v1",
            payload,
        ),
        "partial" => Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Text,
            input.source.clone(),
            parser,
            digest,
            "reference/payload/v1",
            Some(payload),
        )
        .with_diagnostics(vec![
            Diagnostic::lossy("reference.text", "one construct was normalized")
                .with_locator(locator()),
        ]),
        "budget" => {
            let mut diagnostic = Diagnostic::error(
                "reference.text",
                "grist.budget.input_bytes.exceeded",
                "input budget exceeded",
            )
            .partial();
            diagnostic.class = DiagnosticClass::ResourceBudgetExhaustion;
            Envelope::partial(
                OperationKind::Parse,
                ArtifactKind::Text,
                input.source.clone(),
                parser,
                digest,
                "reference/payload/v1",
                Some(payload),
            )
            .with_diagnostics(vec![diagnostic])
        }
        "encrypted" => terminal(
            OperationStatus::Encrypted,
            input.source.clone(),
            parser,
            digest,
            Diagnostic::unsupported("reference.text", "decryption key required"),
        ),
        "unsupported" => terminal(
            OperationStatus::Unsupported,
            input.source.clone(),
            parser,
            digest,
            Diagnostic::unsupported("reference.text", "construct is unsupported"),
        ),
        _ => terminal(
            OperationStatus::Failed,
            input.source.clone(),
            parser,
            digest,
            Diagnostic::malformed("reference.text", "malformed reference input"),
        ),
    };
    envelope
        .with_identity(identity)
        .with_canonical_payload_identity()
        .unwrap()
}

fn terminal(
    status: OperationStatus,
    source: SourceInfo,
    parser: ParserInfo,
    digest: String,
    diagnostic: Diagnostic,
) -> Envelope<Value> {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Text,
        status,
        source,
        parser,
        digest,
        "reference/payload/v1",
    )
    .unwrap()
    .with_diagnostics(vec![diagnostic])
}

fn reference_graph(source: SourceInfo) -> DocumentGraph {
    let mut graph = DocumentGraph::new("reference-graph", DocumentKind::Text)
        .with_source(source)
        .with_projection("text", "reference/payload/v1", "reference.projection/v1");
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document));
    graph.add_node(
        DocumentNode::new("paragraph", DocumentNodeKind::Paragraph)
            .with_locator(locator())
            .with_text("hello"),
    );
    graph.add_contains("root", "paragraph");
    graph.canonicalize().unwrap();
    graph.validate_contract().unwrap();
    graph
}

fn reference_adapter() -> ReferenceAdapter {
    let format = FormatMetadata::new("text", ArtifactKind::Text)
        .with_aliases(["txt"])
        .with_media_types(["text/plain"])
        .with_extensions(["txt"]);
    let mut descriptor = ParserDescriptor::caller(
        "reference.text",
        format,
        ParserInfo::new("reference.text").with_feature("text-publishing"),
        SchemaMetadata::new("reference-payload", "reference/payload/v1"),
        OptionsMetadata::new(
            SchemaMetadata::new("reference-options", "reference/options/v1"),
            json!({}),
        ),
    );
    descriptor
        .required_features
        .insert("text-publishing".into());

    let detection_cases = [
        (
            DetectionScenario::Valid,
            "valid.txt",
            DetectionStatus::Selected,
        ),
        (
            DetectionScenario::Mislabeled,
            "mislabeled.bin",
            DetectionStatus::Selected,
        ),
        (
            DetectionScenario::Extensionless,
            "extensionless",
            DetectionStatus::Selected,
        ),
        (
            DetectionScenario::Malformed,
            "malformed.txt",
            DetectionStatus::Selected,
        ),
        (
            DetectionScenario::Ambiguous,
            "ambiguous",
            DetectionStatus::Ambiguous,
        ),
    ]
    .into_iter()
    .map(|(scenario, name, status)| DetectionCase {
        id: format!("detect-{scenario:?}"),
        scenario,
        input: input("complete", name),
        expected_status: status,
        expected_format: (status != DetectionStatus::Ambiguous).then(|| "text".into()),
    })
    .collect();

    let parse_cases = vec![
        parse_case(
            "complete",
            OperationStatus::Complete,
            [
                ParseCaseRole::Complete,
                ParseCaseRole::Determinism,
                ParseCaseRole::Projection,
                ParseCaseRole::Cli,
            ],
            vec![
                ConstructExpectation {
                    construct: "typed text".into(),
                    disposition: ConstructDisposition::Typed {
                        value_pointer: "/typed/value".into(),
                    },
                    locator: Some(LocatorExpectation::Payload {
                        locator_pointer: "/typed/locator".into(),
                    }),
                },
                ConstructExpectation {
                    construct: "unknown syntax".into(),
                    disposition: ConstructDisposition::Raw {
                        value_pointer: "/raw/value".into(),
                        expected_sha256: grist::core::sha256_hex(b"<unknown>"),
                    },
                    locator: Some(LocatorExpectation::Payload {
                        locator_pointer: "/raw/locator".into(),
                    }),
                },
            ],
        ),
        parse_case(
            "partial",
            OperationStatus::Partial,
            [ParseCaseRole::Partial],
            Vec::new(),
        ),
        parse_case(
            "failed",
            OperationStatus::Failed,
            [ParseCaseRole::Failed],
            Vec::new(),
        ),
        parse_case(
            "encrypted",
            OperationStatus::Encrypted,
            [ParseCaseRole::Encrypted],
            Vec::new(),
        ),
        parse_case(
            "unsupported",
            OperationStatus::Unsupported,
            [ParseCaseRole::Unsupported],
            Vec::new(),
        ),
        parse_case(
            "budget",
            OperationStatus::Partial,
            [ParseCaseRole::BudgetLimited],
            Vec::new(),
        ),
        parse_case(
            "hostile",
            OperationStatus::Complete,
            [ParseCaseRole::Hostile],
            Vec::new(),
        ),
    ];

    let verification = VerificationKind::ALL
        .into_iter()
        .map(|kind| VerificationEvidence {
            kind,
            outcome: EvidenceOutcome::Passed,
            evidence: format!("tests/reference-{kind:?}"),
            rationale: None,
        })
        .collect();
    let ci = CiControl::ALL
        .into_iter()
        .map(|control| CiEvidence {
            control,
            command: if control == CiControl::MinimalFeatureTests {
                "cargo test --no-default-features --features text-publishing,schemas".into()
            } else {
                format!("cargo test --features text-publishing,schemas # {control:?}")
            },
            enabled_features: BTreeSet::from(["text-publishing".into(), "schemas".into()]),
            passed: true,
        })
        .collect();
    ReferenceAdapter {
        descriptor,
        suite: ParserPromotionSuite {
            suite_version: PARSER_PROMOTION_SUITE_VERSION.into(),
            detection_cases,
            parse_cases,
            projection: ProjectionContract {
                case_id: "complete".into(),
                required_node_kinds: vec![DocumentNodeKind::Paragraph],
            },
            public_surface: PublicSurfaceContract {
                case_id: "complete".into(),
                envelope_schema_name: "reference-envelope".into(),
                payload_schema_name: "reference-payload".into(),
                options_schema_name: "reference-options".into(),
                cli_selector: "text".into(),
            },
            fixtures: FixtureContract {
                corpus_format: "text".into(),
                required_classes: mandatory_fixture_classes(),
                fuzz_corpus_paths: vec!["fixtures/generated/markdown/maximum-complexity.md".into()],
            },
            verification,
            ci,
        },
        panic_on_hostile: false,
    }
}

fn input(marker: &str, display_name: &str) -> PromotionInput {
    PromotionInput::new(marker.as_bytes(), SourceInfo::new(display_name))
}

fn parse_case(
    marker: &str,
    expected_status: OperationStatus,
    roles: impl IntoIterator<Item = ParseCaseRole>,
    constructs: Vec<ConstructExpectation>,
) -> ParseCase {
    ParseCase {
        id: marker.into(),
        input: input(marker, &format!("{marker}.txt")),
        expected_status,
        roles: roles.into_iter().collect(),
        constructs,
    }
}

fn mandatory_fixture_classes() -> BTreeSet<FixtureClass> {
    BTreeSet::from([
        FixtureClass::MinimalValid,
        FixtureClass::RepresentativeRealWorld,
        FixtureClass::Malformed,
        FixtureClass::Adversarial,
        FixtureClass::UnsupportedConstruct,
        FixtureClass::MaliciousActiveContent,
        FixtureClass::DownstreamRegression,
    ])
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn reference_corpus() -> FixtureCorpusManifest {
    let mut corpus = load_corpus(repository_root()).unwrap();
    let format = corpus.formats.get_mut("text").unwrap();
    format.cases[0].classes = mandatory_fixture_classes().into_iter().collect();
    corpus
}

#[cfg(feature = "schemas")]
#[test]
fn all_eleven_gates_are_required_for_promotion() {
    let corpus = reference_corpus();
    let adapter = reference_adapter();
    let harness = ParserPromotionHarness::new(&corpus, repository_root());
    let first = harness.evaluate(&adapter);
    let second = harness.evaluate(&adapter);

    assert!(first.eligible_for_promotion);
    assert!(first.validate().is_ok());
    assert_eq!(first.passed_gate_count, 11);
    assert_eq!(first.gates.len(), 11);
    assert_eq!(
        first
            .gates
            .iter()
            .map(|gate| gate.gate_number)
            .collect::<Vec<_>>(),
        (1_u8..=11).collect::<Vec<_>>()
    );
    assert!(first.gates.iter().all(|gate| !gate.checks.is_empty()));
    assert_eq!(first, second, "reports must not contain wall-clock state");
    assert_eq!(
        grist::core::canonical_json_bytes(&first).unwrap(),
        grist::core::canonical_json_bytes(&second).unwrap()
    );
}

#[cfg(feature = "schemas")]
#[test]
fn report_invariants_reject_fabricated_eligibility() {
    let corpus = reference_corpus();
    let mut report =
        ParserPromotionHarness::new(&corpus, repository_root()).evaluate(&reference_adapter());
    report.gates[0].checks[0].passed = false;
    assert!(report.validate().is_err());

    report.gates[0].checks[0].passed = true;
    report.eligible_for_promotion = false;
    assert_eq!(
        report.validate().unwrap_err(),
        grist::promotion::ParserConformanceReportError::Eligibility
    );
}

#[cfg(feature = "schemas")]
#[test]
fn report_is_machine_readable_and_schema_valid() {
    let corpus = reference_corpus();
    let report =
        ParserPromotionHarness::new(&corpus, repository_root()).evaluate(&reference_adapter());
    let value = serde_json::to_value(&report).unwrap();
    let validation = grist::schema::validate_schema("parser-conformance-report", &value).unwrap();
    assert!(validation.valid, "{:?}", validation.issues);
    assert_eq!(value["eligible_for_promotion"], true);
    assert!(value.get("detection_cases").is_none());
}

#[test]
fn absent_or_failed_evidence_fails_closed() {
    let corpus = reference_corpus();
    let mut adapter = reference_adapter();
    adapter
        .suite
        .ci
        .retain(|evidence| evidence.control != CiControl::Documentation);
    adapter.suite.parse_cases[0].constructs[0].locator = None;
    let report = ParserPromotionHarness::new(&corpus, repository_root()).evaluate(&adapter);

    assert!(!report.eligible_for_promotion);
    assert_eq!(report.gates.len(), 11);
    assert_eq!(
        report.gate(PromotionGate::SourceLocators).unwrap().status,
        grist::promotion::GateStatus::Failed
    );
    assert_eq!(
        report.gate(PromotionGate::EnabledFeatureCi).unwrap().status,
        grist::promotion::GateStatus::Failed
    );
}

#[test]
fn adapter_panics_are_contained_and_fail_the_security_gate() {
    let corpus = reference_corpus();
    let mut adapter = reference_adapter();
    adapter.panic_on_hostile = true;
    let report = ParserPromotionHarness::new(&corpus, repository_root()).evaluate(&adapter);

    assert!(!report.eligible_for_promotion);
    let security = report.gate(PromotionGate::HostileInputSafety).unwrap();
    assert_eq!(security.status, grist::promotion::GateStatus::Failed);
    assert!(security.checks.iter().any(|check| {
        !check.passed
            && check
                .message
                .contains("panicked across the promotion boundary")
    }));
}
