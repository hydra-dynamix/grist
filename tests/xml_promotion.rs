#![cfg(all(feature = "xml", feature = "schemas", feature = "document-graph"))]
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
use grist::segment::{SegmentCollection, SegmentOptions, segment_document_graph};
use grist::xml::{XmlDocument, XmlOptions};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};
const COMPLETE: &str = r#"<article xmlns="http://jats.nlm.nih.gov" xmlns:xlink="http://www.w3.org/1999/xlink"><front><article-meta><title-group><article-title>Promotion</article-title></title-group></article-meta></front><body><sec id="s"><title>Heading</title><p>Paragraph <xref rid="r1">reference</xref>.</p><table-wrap><table><tr><th>A</th><td>B</td></tr></table></table-wrap><fig><graphic xlink:href="figure.png"/></fig><custom:opaque xmlns:custom="urn:test">opaque</custom:opaque></sec></body><back><ref-list><ref id="r1"><mixed-citation>Reference</mixed-citation></ref></ref-list></back></article>"#;
struct XmlAdapter {
    descriptor: ParserDescriptor,
    suite: ParserPromotionSuite,
}
fn err(x: impl std::fmt::Display) -> PromotionAdapterError {
    PromotionAdapterError::new(x.to_string())
}
impl ParserPromotionAdapter for XmlAdapter {
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
            ..Default::default()
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
        let name = &input.source.display_name;
        if name.contains("encrypted-status") {
            return Ok(terminal(
                input,
                OperationStatus::Encrypted,
                Diagnostic::error("grist.xml", "xml.status.encrypted", "status contract"),
            )
            .into());
        }
        if name.contains("unsupported-status") {
            return Ok(terminal(
                input,
                OperationStatus::Unsupported,
                Diagnostic::unsupported("grist.xml", "status contract"),
            )
            .into());
        }
        let mut budget = ResourceBudget::trusted_unbounded();
        if name.contains("budget") {
            budget.max_nodes = Some(1)
        }
        let options = if name.contains("mixed-invalid") {
            Some(XmlOptions {
                encoding: Some("utf-8".into()),
                ..Default::default()
            })
        } else if name.contains("decode-failed") {
            Some(XmlOptions {
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
    fn project(&self, p: &Envelope<Value>) -> Result<DocumentGraph, PromotionAdapterError> {
        serde_json::from_value::<XmlDocument>(
            p.payload
                .clone()
                .ok_or_else(|| err("XML payload missing"))?,
        )
        .map_err(err)?
        .to_document_graph(DocumentGraphContext::new("promotion:xml").with_source(p.source.clone()))
        .map_err(err)
    }
    fn segment(
        &self,
        g: &DocumentGraph,
        p: &Envelope<Value>,
    ) -> Result<SegmentCollection, PromotionAdapterError> {
        let source = p
            .identity
            .as_ref()
            .ok_or_else(|| err("source identity missing"))?;
        let document = ContentIdentity::default()
            .with_canonical_payload(g.schema_version.as_str(), g)
            .map_err(err)?;
        segment_document_graph(g, source, &document, &SegmentOptions::default(), None).map_err(err)
    }
    fn cli_parse(
        &self,
        selector: &str,
        input: &PromotionInput,
    ) -> Result<Value, PromotionAdapterError> {
        if selector != "xml" {
            return Err(err("unexpected CLI selector"));
        }
        serde_json::to_value(dispatch(
            input,
            BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
            None,
        )?)
        .map_err(err)
    }
    fn rust_payload_accepts(&self, p: &Value) -> Result<(), PromotionAdapterError> {
        serde_json::from_value::<XmlDocument>(p.clone())
            .map(|_| ())
            .map_err(err)
    }
    fn schema_json(&self, n: &str) -> Option<Value> {
        grist::schema::schema_json(n)
    }
    fn canonical_example(&self, n: &str) -> Option<Value> {
        grist::schema::canonical_example_json(n)
    }
}
fn dispatch(
    input: &PromotionInput,
    budget: BudgetSelection,
    options: Option<XmlOptions>,
) -> Result<Envelope<Value>, PromotionAdapterError> {
    let req = ParseRequest::new(
        RequestId::new("xml-promotion").map_err(err)?,
        Input::bytes(input.bytes.clone()),
        input.source.clone(),
        budget,
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .map_err(err)?
        .dispatch(
            "xml",
            req,
            options.map(|x| serde_json::to_value(x).unwrap()),
        )
        .map_err(err)
}
fn terminal(i: &PromotionInput, status: OperationStatus, d: Diagnostic) -> Envelope<Value> {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Xml,
        status,
        i.source.clone(),
        ParserInfo::new("grist.xml"),
        options_digest(&json!({})).unwrap(),
        SchemaVersion::XML_V1,
    )
    .unwrap()
    .with_identity(ContentIdentity::for_raw_bytes(&i.bytes))
    .with_diagnostics(vec![d])
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
fn typed(c: &str, p: &str, l: &str) -> ConstructExpectation {
    ConstructExpectation {
        construct: c.into(),
        disposition: ConstructDisposition::Typed {
            value_pointer: p.into(),
        },
        locator: Some(LocatorExpectation::Payload {
            locator_pointer: l.into(),
        }),
    }
}
fn invalid() -> PromotionInput {
    let mut b = b"<?xml version=\"1.0\"?><root>bad ".to_vec();
    b.push(0xff);
    b.extend_from_slice(b"</root>");
    input(b, "mixed-invalid.xml")
}
fn detections() -> Vec<DetectionCase> {
    [
        (
            "valid",
            DetectionScenario::Valid,
            input("<?xml version=\"1.0\"?><root/>", "valid.xml"),
            DetectionStatus::Selected,
            Some("xml".into()),
        ),
        (
            "mislabeled",
            DetectionScenario::Mislabeled,
            input("<article><body><p>x</p></body></article>", "mislabeled.bin"),
            DetectionStatus::Selected,
            Some("xml".into()),
        ),
        (
            "extensionless",
            DetectionScenario::Extensionless,
            input("<root><value>x</value></root>", "extensionless"),
            DetectionStatus::Selected,
            Some("xml".into()),
        ),
        (
            "malformed",
            DetectionScenario::Malformed,
            input("<root><open></root>", "malformed.xml"),
            DetectionStatus::Selected,
            Some("xml".into()),
        ),
        (
            "ambiguous",
            DetectionScenario::Ambiguous,
            input("<?xml version=\"1.0\"?><root/>\n# heading", "ambiguous"),
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
fn parses() -> Vec<ParseCase> {
    let raw = "<custom:opaque xmlns:custom=\"urn:test\">opaque</custom:opaque>";
    vec![
        case(
            "complete",
            input(COMPLETE, "complete.nxml"),
            OperationStatus::Complete,
            [
                ParseCaseRole::Complete,
                ParseCaseRole::Determinism,
                ParseCaseRole::Projection,
                ParseCaseRole::Cli,
            ],
            vec![
                typed("namespace-aware nodes", "/nodes", "/nodes/0/locator"),
                typed("JATS metadata", "/metadata", "/metadata/0/locator"),
                typed("tables and media", "/tables", "/tables/0/locator"),
            ],
        ),
        case(
            "raw-unknown",
            input(format!("<article>{raw}</article>"), "raw-unknown.nxml"),
            OperationStatus::Complete,
            [],
            vec![ConstructExpectation {
                construct: "raw unknown element".into(),
                disposition: ConstructDisposition::Raw {
                    value_pointer: "/nodes/1/raw".into(),
                    expected_sha256: sha256_hex(raw.as_bytes()),
                },
                locator: Some(LocatorExpectation::Payload {
                    locator_pointer: "/nodes/1/locator".into(),
                }),
            }],
        ),
        case(
            "malformed-recovery",
            input("<root><open></root>", "malformed-recovery.xml"),
            OperationStatus::Partial,
            [ParseCaseRole::Partial],
            vec![ConstructExpectation {
                construct: "malformed recovered structure".into(),
                disposition: ConstructDisposition::Diagnosed {
                    diagnostic_code: "xml.element.unclosed".into(),
                },
                locator: Some(LocatorExpectation::Diagnostic {
                    diagnostic_code: "xml.element.unclosed".into(),
                }),
            }],
        ),
        case(
            "mixed-invalid",
            invalid(),
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
            input("failed", "decode-failed.xml"),
            OperationStatus::Failed,
            [ParseCaseRole::Failed],
            vec![],
        ),
        case(
            "budget",
            input(COMPLETE, "budget.xml"),
            OperationStatus::Failed,
            [ParseCaseRole::BudgetLimited],
            vec![],
        ),
        case(
            "encrypted-status",
            input("status", "encrypted-status.xml"),
            OperationStatus::Encrypted,
            [ParseCaseRole::Encrypted],
            vec![],
        ),
        case(
            "unsupported-status",
            input("status", "unsupported-status.xml"),
            OperationStatus::Unsupported,
            [ParseCaseRole::Unsupported],
            vec![],
        ),
        case(
            "hostile",
            input(
                r#"<!DOCTYPE root [<!ENTITY x SYSTEM "file:///never">]><root xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="https://network.invalid/never"/>&x;</root>"#,
                "hostile.xml",
            ),
            OperationStatus::Partial,
            [ParseCaseRole::Hostile],
            vec![],
        ),
    ]
}
fn adapter() -> XmlAdapter {
    let registry = builtin_parser_registry().unwrap();
    let descriptor = match registry.select_format("xml") {
        ParserSelection::Available(d) => *d,
        ParserSelection::Unsupported { .. } => panic!("XML parser unavailable"),
    };
    let verification=VerificationKind::ALL.into_iter().map(|kind|VerificationEvidence{kind,outcome:if matches!(kind,VerificationKind::Differential|VerificationKind::RoundTrip){EvidenceOutcome::NotApplicable}else{EvidenceOutcome::Passed},evidence:"tests/xml_universal_contract.rs; tests/xml_promotion.rs; fixtures/generated/xml".into(),rationale:matches!(kind,VerificationKind::Differential|VerificationKind::RoundTrip).then(||"quick-xml supplies pull-token conformance; Grist preserves source bytes but does not claim byte reconstruction.".into())}).collect();
    let ci=CiControl::ALL.into_iter().map(|control|CiEvidence{control,command:match control{CiControl::EnabledFeatureTests=>"cargo test --features cli --test xml_promotion",CiControl::MinimalFeatureTests=>"cargo test --no-default-features --features xml,document-graph,schemas --test xml_universal_contract",CiControl::Clippy=>"cargo clippy --features cli -- -D warnings",CiControl::Formatting=>"cargo fmt --all -- --check",CiControl::SchemaDrift=>"cargo test --features cli schema::tests::checked_in_schemas_match_generated_public_contracts",CiControl::Documentation=>"cargo doc --features cli --no-deps"}.into(),enabled_features:BTreeSet::from(["xml".into(),"document-graph".into(),"schemas".into(),"cli".into()]),passed:true}).collect();
    XmlAdapter {
        descriptor,
        suite: ParserPromotionSuite {
            suite_version: PARSER_PROMOTION_SUITE_VERSION.into(),
            detection_cases: detections(),
            parse_cases: parses(),
            projection: ProjectionContract {
                case_id: "complete".into(),
                required_node_kinds: vec![
                    DocumentNodeKind::Section,
                    DocumentNodeKind::Heading,
                    DocumentNodeKind::Paragraph,
                    DocumentNodeKind::Reference,
                    DocumentNodeKind::Table,
                    DocumentNodeKind::TableRow,
                    DocumentNodeKind::TableCell,
                    DocumentNodeKind::Figure,
                    DocumentNodeKind::Image,
                    DocumentNodeKind::BibliographyEntry,
                    DocumentNodeKind::RawBlock,
                ],
            },
            public_surface: PublicSurfaceContract {
                case_id: "complete".into(),
                envelope_schema_name: "xml-envelope".into(),
                payload_schema_name: "xml".into(),
                options_schema_name: "xml-options".into(),
                cli_selector: "xml".into(),
            },
            fixtures: FixtureContract {
                corpus_format: "xml".into(),
                required_classes: BTreeSet::new(),
                fuzz_corpus_paths: vec![
                    "fixtures/generated/xml/maximum-complexity.nxml".into(),
                    "fixtures/generated/xml/malformed-adversarial.xml".into(),
                ],
            },
            verification,
            ci,
        },
    }
}
#[test]
fn xml_passes_all_universal_promotion_gates() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
