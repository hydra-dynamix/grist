#![cfg(feature = "graph")]

use grist::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, FormatHint, Input, Limits, LocationComponent,
    OperationStatus, ParseRequest, ProviderSet, RequestId, SourceInfo, SourceLocator,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_source};
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::graph::{GraphDocument, GraphOptions, parse_graph_json};
use grist::ingest::Ingestor;
use grist::registry::builtin_parser_registry;
#[cfg(feature = "cli")]
use serde_json::Value;
use serde_json::json;

const GRAPH_JSON: &str = r#"{
  "schema_version":"grist/graph-document/v1",
  "id":"sample",
  "directed":true,
  "nodes":[
    {"id":"a","labels":["claim"],"attrs":{"confidence":0.9}},
    {"id":"b","labels":["vendor_node"],"attrs":{"opaque":{"x":1}}}
  ],
  "edges":[
    {"id":"e1","source":"a","target":"b","directed":true,"label":"evidence_for","attrs":{"weight":2}}
  ],
  "attrs":{"domain":"test"}
}"#;

const GRAPH_YAML: &str = r#"schema_version: grist/graph-document/v1
id: sample
directed: true
nodes:
  - id: a
    labels: [claim]
  - id: b
    labels: [evidence]
edges:
  - id: e1
    source: a
    target: b
    directed: true
attrs: {}
"#;

fn detect(name: &str, bytes: &[u8]) -> grist::detect::Detection {
    let registry = builtin_parser_registry().unwrap();
    detect_source(
        &SourceInfo::stdin(name),
        bytes,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap()
}

#[test]
fn canonical_graph_structure_is_ranked_above_generic_serialization() {
    for (name, bytes) in [
        ("sample.json", GRAPH_JSON.as_bytes()),
        ("sample.yaml", GRAPH_YAML.as_bytes()),
    ] {
        let detection = detect(name, bytes);
        assert_eq!(detection.status, DetectionStatus::Selected);
        assert_eq!(detection.content_kind, ContentKind::Graph);
        assert_eq!(detection.selected_parser.as_deref(), Some("grist.graph"));
        assert_eq!(detection.candidates[0].identity.format, "graph");
    }

    #[cfg(feature = "serialization")]
    {
        let ordinary = detect("ordinary.json", br#"{"hello":"world"}"#);
        assert_eq!(ordinary.content_kind, ContentKind::Json);
        assert_eq!(
            ordinary.selected_parser.as_deref(),
            Some("grist.serialization.json")
        );

        let explicitly_named = detect("broken.graph.json", br#"{"hello":"world"}"#);
        assert_eq!(explicitly_named.content_kind, ContentKind::Graph);
        assert_eq!(
            explicitly_named.selected_parser.as_deref(),
            Some("grist.graph")
        );
    }
}

#[test]
fn automatic_and_explicit_ingest_emit_authoritative_graph_envelopes() {
    let registry = builtin_parser_registry().unwrap();
    let ingestor = Ingestor::new(builtin_parser_registry().unwrap());
    let request = ParseRequest::new(
        RequestId::new("graph-auto").unwrap(),
        Input::bytes(GRAPH_JSON.as_bytes().to_vec()),
        SourceInfo::stdin("sample.graph.json"),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );
    let automatic = ingestor.ingest(request).unwrap();
    assert_eq!(automatic.status, OperationStatus::Complete);
    assert_eq!(automatic.kind, ArtifactKind::GraphDocument);
    assert_eq!(automatic.parser.name, "grist.graph");
    let payload: GraphDocument = serde_json::from_value(automatic.payload.unwrap()).unwrap();
    assert_eq!(payload.id.as_deref(), Some("sample"));
    assert_eq!(payload.nodes.len(), 2);

    let explicit = registry
        .dispatch(
            "graph_yaml",
            ParseRequest::new(
                RequestId::new("graph-explicit").unwrap(),
                Input::bytes(GRAPH_YAML.as_bytes().to_vec()),
                SourceInfo::stdin("stdin"),
                BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
                ProviderSet::none(),
            )
            .with_format_hint(FormatHint::exact("graph_yaml")),
            Some(json!({"encoding":"yaml"})),
        )
        .unwrap();
    assert_eq!(explicit.status, OperationStatus::Complete);
    assert_eq!(explicit.kind, ArtifactKind::GraphDocument);
}

#[test]
fn projection_preserves_identity_unknown_vocabulary_attributes_and_locators() {
    let parsed = parse_graph_json(
        GRAPH_JSON.as_bytes(),
        SourceInfo::stdin("sample.graph.json"),
        &GraphOptions::default(),
    );
    let projected = parsed
        .to_document_graph(DocumentGraphContext::new("normalized"))
        .unwrap();
    assert_eq!(projected.id, "normalized");
    let boundary = projected.projection.as_ref().unwrap();
    assert_eq!(boundary.authoritative_payload_kind, "graph");
    assert_eq!(
        boundary.authoritative_payload_schema_version,
        GraphDocument::SCHEMA_VERSION
    );
    assert_eq!(projected.attrs["domain"], "test");

    let claim = projected.nodes.iter().find(|node| node.id == "a").unwrap();
    assert_eq!(claim.kind, DocumentNodeKind::Claim);
    assert_eq!(claim.attrs["confidence"], 0.9);
    assert!(claim.locator.is_some());

    let unknown = projected.nodes.iter().find(|node| node.id == "b").unwrap();
    assert_eq!(unknown.kind, DocumentNodeKind::Other("vendor_node".into()));
    assert_eq!(unknown.attrs["opaque"]["x"], 1);
    assert!(unknown.raw.is_some());
    assert_eq!(unknown.extensions["grist.graph"]["id"], "b");

    let edge = projected.edges.iter().find(|edge| edge.id == "e1").unwrap();
    assert_eq!(edge.source, "a");
    assert_eq!(edge.target, "b");
    assert_eq!(edge.attrs["weight"], 2);
    assert_eq!(edge.extensions["grist.graph"]["label"], "evidence_for");
}

#[test]
fn malformed_and_budget_limited_graph_ingest_is_terminal_without_panicking() {
    let registry = builtin_parser_registry().unwrap();
    let malformed = registry
        .dispatch(
            "graph",
            ParseRequest::new(
                RequestId::new("bad-graph").unwrap(),
                Input::bytes(br#"{"schema_version":"grist/graph-document/v1","nodes":[}"#.to_vec()),
                SourceInfo::stdin("bad.graph.json"),
                BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
                ProviderSet::none(),
            ),
            None,
        )
        .unwrap();
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(malformed.payload.is_none());
    assert!(!malformed.diagnostics.is_empty());

    let mut budget = grist::core::ResourceBudget::trusted_unbounded();
    budget.max_nodes = Some(1);
    let limited = registry
        .dispatch(
            "graph",
            ParseRequest::new(
                RequestId::new("limited-graph").unwrap(),
                Input::bytes(GRAPH_JSON.as_bytes().to_vec()),
                SourceInfo::stdin("limited.graph.json"),
                BudgetSelection::custom(budget),
                ProviderSet::none(),
            ),
            None,
        )
        .unwrap();
    assert!(matches!(
        limited.status,
        OperationStatus::Failed | OperationStatus::Partial
    ));
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str().starts_with("grist.budget"))
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_memory_surface_agrees_with_registry_and_projection() {
    let envelope = grist::cli::parse_bytes(
        "auto",
        GRAPH_JSON.as_bytes().to_vec(),
        SourceInfo::stdin("sample.graph.json"),
        RequestId::new("cli-graph").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(envelope.kind, ArtifactKind::GraphDocument);
    let projected = grist::cli::project_envelope_to_graph(&envelope, "cli-normalized").unwrap();
    assert_eq!(projected.id, "cli-normalized");
    assert_eq!(
        projected.projection.unwrap().authoritative_payload_kind,
        "graph"
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_executable_parses_and_transforms_graph_fixtures() {
    use std::process::Command;

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/generated/graph/valid/v1-property-multigraph.json");
    let parsed = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "graph"])
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        parsed.status.success(),
        "{}",
        String::from_utf8_lossy(&parsed.stderr)
    );
    let parsed: Value = serde_json::from_slice(&parsed.stdout).unwrap();
    assert_eq!(parsed["kind"], "graph_document");
    assert_eq!(parsed["parser"]["name"], "grist.graph");

    let transformed = Command::new(env!("CARGO_BIN_EXE_grist"))
        .arg("transform")
        .arg(&fixture)
        .args(["--to", "graph"])
        .output()
        .unwrap();
    assert!(
        transformed.status.success(),
        "{}",
        String::from_utf8_lossy(&transformed.stderr)
    );
    let transformed: Value = serde_json::from_slice(&transformed.stdout).unwrap();
    assert_eq!(
        transformed["payload"]["graph"]["projection"]["authoritative_payload_kind"],
        "graph"
    );
}

#[cfg(feature = "ldgr-projection")]
#[test]
fn ldgr_graph_conversion_preserves_dependency_direction_and_uses_shared_analysis() {
    use grist::ldgr_projection::{
        LdgrGraphConversionError, LdgrGraphDocument, LdgrGraphEdge, LdgrGraphNode,
        LdgrProjectionOptions, LdgrRef, parse_ldgr_projection,
    };

    let ldgr = LdgrGraphDocument {
        nodes: vec![
            LdgrGraphNode {
                id: "first".into(),
                artifact: Some(LdgrRef::parse("artifact:1").unwrap()),
                work_item: Some(LdgrRef::parse("work:first").unwrap()),
            },
            LdgrGraphNode {
                id: "second".into(),
                artifact: None,
                work_item: None,
            },
        ],
        edges: vec![LdgrGraphEdge {
            dependency: "first".into(),
            dependent: "second".into(),
            kind: Some("blocks".into()),
        }],
    };
    let shared = GraphDocument::from(&ldgr);
    assert_eq!(shared.edges[0].source, "first");
    assert_eq!(shared.edges[0].target, "second");
    assert_eq!(shared.nodes[0].attrs["artifact"], "artifact:1");
    let round_trip = LdgrGraphDocument::try_from(&shared).unwrap();
    assert_eq!(round_trip, ldgr);

    let mut incompatible = shared.clone();
    incompatible.edges[0].directed = false;
    assert!(matches!(
        LdgrGraphDocument::try_from(&incompatible),
        Err(LdgrGraphConversionError::UndirectedEdge(_))
    ));

    let report = parse_ldgr_projection(
        "---\nldgr_doc: 1\nkind: graph\nid: graph.a\nschema: ldgr.graph.v1\n---\n```ldgr-graph yaml\nnodes:\n  - id: a\n  - id: b\nedges:\n  - dependency: a\n    dependent: b\n  - dependency: b\n    dependent: a\n```\n",
        SourceInfo::stdin("cycle.md"),
        LdgrProjectionOptions::default(),
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|item| item.code == "graph.cycle")
    );
}

#[test]
fn parser_sidecar_uses_json_pointer_locators() {
    let parsed = parse_graph_json(
        GRAPH_JSON.as_bytes(),
        SourceInfo::stdin("sample.graph.json"),
        &GraphOptions::default(),
    );
    let expected = SourceLocator::exact(LocationComponent::JsonPointer {
        pointer: "/nodes/0".into(),
    })
    .unwrap();
    assert_eq!(parsed.source_map.nodes[0], expected);
}
