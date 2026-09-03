#![cfg(feature = "graph")]

use grist::core::{
    BudgetSelection, CancellationToken, LocationComponent, OperationControl, OperationStatus,
    ResourceBudget, SourceInfo,
};
use grist::graph::{
    GraphInputEncoding, GraphOptions, diagnostic_codes, parse_graph,
    parse_graph_with_operation_control, parse_graph_yaml,
};

const VALID_JSON: &[u8] =
    include_bytes!("../fixtures/generated/graph/valid/v1-property-multigraph.json");
const VALID_YAML: &[u8] =
    include_bytes!("../fixtures/generated/graph/valid/v1-property-multigraph.yaml");

fn source(name: &str) -> SourceInfo {
    SourceInfo::stdin(name)
}

fn options(encoding: GraphInputEncoding) -> GraphOptions {
    GraphOptions {
        encoding: Some(encoding),
    }
}

#[test]
fn equivalent_json_and_yaml_have_identical_canonical_payloads_and_distinct_raw_identity() {
    let json = parse_graph(VALID_JSON, source("graph.json"), &GraphOptions::default());
    let yaml = parse_graph(VALID_YAML, source("graph.yaml"), &GraphOptions::default());

    assert_eq!(json.envelope.status, OperationStatus::Complete);
    assert_eq!(yaml.envelope.status, OperationStatus::Complete);
    assert_eq!(json.envelope.payload, yaml.envelope.payload);
    assert_eq!(
        json.envelope
            .identity
            .as_ref()
            .unwrap()
            .canonical_payload
            .as_ref()
            .unwrap()
            .sha256,
        yaml.envelope
            .identity
            .as_ref()
            .unwrap()
            .canonical_payload
            .as_ref()
            .unwrap()
            .sha256
    );
    assert_ne!(
        json.envelope.identity.as_ref().unwrap().raw,
        yaml.envelope.identity.as_ref().unwrap().raw
    );

    let graph = json.envelope.payload.unwrap();
    assert!(graph.edges[0].directed, "document default is materialized");
    assert!(!graph.edges[4].directed, "edge override is retained");
    assert_eq!(json.source_map.nodes.len(), graph.nodes.len());
    assert_eq!(json.source_map.edges.len(), graph.edges.len());
    assert_eq!(yaml.source_map.nodes.len(), graph.nodes.len());
    assert_eq!(yaml.source_map.edges.len(), graph.edges.len());
    assert!(matches!(
        json.source_map.nodes[0].innermost(),
        LocationComponent::JsonPointer { pointer } if pointer == "/nodes/0"
    ));
    assert!(matches!(
        yaml.source_map.nodes[0].innermost(),
        LocationComponent::TextRange { .. }
    ));
    assert_ne!(yaml.source_map.nodes[0], yaml.source_map.nodes[1]);
}

#[test]
fn defaults_and_closed_structural_fields_are_enforced() {
    let minimal = br#"{
        "schema_version":"grist/graph-document/v1",
        "directed":false,
        "nodes":[{"id":"n"}],
        "edges":[{"id":"e","source":"n","target":"n"}]
    }"#;
    let parsed = parse_graph(minimal, source("minimal"), &GraphOptions::default());
    let graph = parsed.envelope.payload.unwrap();
    assert!(graph.attrs.is_empty());
    assert!(graph.nodes[0].labels.is_empty());
    assert!(graph.nodes[0].attrs.is_empty());
    assert!(!graph.edges[0].directed);
    assert!(graph.edges[0].attrs.is_empty());

    let unknown =
        include_bytes!("../fixtures/generated/graph/invalid/unknown-structural-field.yaml");
    let rejected = parse_graph_yaml(
        unknown,
        source("unknown.yaml"),
        &options(GraphInputEncoding::Yaml),
    );
    assert_eq!(rejected.envelope.status, OperationStatus::Failed);
    assert!(rejected.envelope.payload.is_none());
    assert_eq!(
        rejected.envelope.diagnostics[0].code,
        diagnostic_codes::FIELD_UNKNOWN
    );

    let version = include_bytes!("../fixtures/generated/graph/invalid/unsupported-version.json");
    let rejected = parse_graph(version, source("version.json"), &GraphOptions::default());
    assert_eq!(rejected.envelope.status, OperationStatus::Unsupported);
    assert_eq!(
        rejected.envelope.diagnostics[0].code,
        diagnostic_codes::SCHEMA_VERSION_UNSUPPORTED
    );
}

#[test]
fn duplicate_keys_and_unsafe_yaml_are_rejected_without_payloads() {
    let duplicate_json = br#"{
        "schema_version":"grist/graph-document/v1",
        "directed":true,"nodes":[],"edges":[],
        "attrs":{"nested":{"key":1,"key":2}}
    }"#;
    let rejected = parse_graph(
        duplicate_json,
        source("duplicate.json"),
        &GraphOptions::default(),
    );
    assert_eq!(rejected.envelope.status, OperationStatus::Failed);
    assert_eq!(
        rejected.envelope.diagnostics[0].code,
        "grist.input.malformed"
    );

    let cases: &[&[u8]] = &[
        include_bytes!("../fixtures/generated/graph/invalid/yaml-alias.yaml"),
        b"---\nschema_version: grist/graph-document/v1\ndirected: true\nnodes: []\nedges: []\n---\n{}\n",
        b"schema_version: grist/graph-document/v1\ndirected: true\nnodes: &nodes []\nedges: []\n",
        b"schema_version: grist/graph-document/v1\ndirected: true\nnodes: []\nedges: []\nattrs: {value: !custom x}\n",
        b"schema_version: grist/graph-document/v1\ndirected: true\nnodes: []\nedges: []\nattrs: {value: .nan}\n",
    ];
    for yaml in cases {
        let rejected = parse_graph_yaml(
            yaml,
            source("unsafe.yaml"),
            &options(GraphInputEncoding::Yaml),
        );
        assert!(rejected.envelope.payload.is_none());
        assert_eq!(
            rejected.envelope.diagnostics[0].code,
            diagnostic_codes::YAML_FEATURE_UNSUPPORTED
        );
    }
}

#[test]
fn semantic_graph_checks_remain_downstream_of_syntax_decoding() {
    for fixture in [
        include_bytes!("../fixtures/generated/graph/invalid/duplicate-node-id.yaml").as_slice(),
        include_bytes!("../fixtures/generated/graph/invalid/dangling-endpoint.json").as_slice(),
    ] {
        let result = parse_graph(
            fixture,
            source("structural-input"),
            &GraphOptions::default(),
        );
        assert_eq!(result.envelope.status, OperationStatus::Complete);
        assert!(result.envelope.payload.is_some());
    }
}

#[test]
fn cancellation_and_budget_failures_use_shared_terminal_semantics() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let control = OperationControl::new(
        &BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
        cancellation,
    )
    .unwrap();
    let cancelled = parse_graph_with_operation_control(
        VALID_JSON,
        source("cancelled.json"),
        &options(GraphInputEncoding::Json),
        &control,
    );
    assert_eq!(cancelled.envelope.status, OperationStatus::Cancelled);
    assert!(cancelled.envelope.payload.is_none());
    assert_eq!(
        cancelled.envelope.diagnostics[0].code,
        "grist.operation.cancelled"
    );

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nodes = Some(2);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), CancellationToken::new()).unwrap();
    let partial = parse_graph_with_operation_control(
        VALID_JSON,
        source("limited.json"),
        &options(GraphInputEncoding::Json),
        &control,
    );
    assert_eq!(partial.envelope.status, OperationStatus::Partial);
    assert_eq!(partial.envelope.payload.as_ref().unwrap().nodes.len(), 1);
    assert_eq!(partial.source_map.nodes.len(), 1);
    assert_eq!(
        partial.envelope.diagnostics[0].code,
        "grist.budget.nodes.exhausted"
    );
}
