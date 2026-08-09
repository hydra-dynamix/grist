#![cfg(feature = "graph")]

use grist::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, ContentIdentity, Diagnostic, FormatOptions,
    Input, LocationComponent, ParseRequest, ParserInfo, ProviderSet, RequestId, SourceInfo,
    SourceLocator,
};
use grist::graph::{
    GraphDocument, GraphEdge, GraphError, GraphInputEncoding, GraphNode, GraphOptions,
    GraphParseResult, GraphSourceMap, diagnostic_codes,
};
use serde_json::{Value, json};

fn pointer(value: &str) -> SourceLocator {
    SourceLocator::exact(LocationComponent::JsonPointer {
        pointer: value.into(),
    })
    .unwrap()
}

fn sample_document() -> GraphDocument {
    let mut document = GraphDocument::new(true);
    document.id = Some("source-graph".into());
    document.attrs.insert("zeta".into(), json!({"b": 2}));
    document.attrs.insert("alpha".into(), json!([true, null]));

    let mut first = GraphNode::new("n:1");
    first.labels = vec!["Person".into(), "Reviewer".into()];
    first.attrs.insert("name".into(), json!("Ada"));
    let second = GraphNode::new("n:2");
    document.nodes = vec![first, second];

    let mut edge = GraphEdge::new("e:1", "n:1", "n:2", false);
    edge.label = Some("reviews".into());
    edge.attrs.insert("weight".into(), json!(1.5));
    document.edges.push(edge);
    document
}

#[test]
fn graph_payload_round_trips_with_deterministic_attribute_order() {
    let document = sample_document();
    let bytes = grist::core::canonical_json_bytes(&document).unwrap();
    let bytes_again = grist::core::canonical_json_bytes(&document).unwrap();
    assert_eq!(bytes, bytes_again);

    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schema_version"], GraphDocument::SCHEMA_VERSION);
    assert_eq!(value["nodes"][0]["id"], "n:1");
    assert_eq!(value["edges"][0]["directed"], false);
    assert!(!document.edges[0].directed);

    let round_trip: GraphDocument = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(round_trip, document);
    let encoded = String::from_utf8(bytes).unwrap();
    assert!(encoded.find("\"alpha\"").unwrap() < encoded.find("\"zeta\"").unwrap());
}

#[test]
fn graph_direction_defaults_are_materialized_and_source_locations_are_sidecar_evidence() {
    let document = sample_document();
    let canonical = serde_json::to_value(&document).unwrap();
    assert_eq!(canonical["edges"][0]["directed"], false);
    assert!(canonical.get("locator").is_none());
    assert!(canonical["nodes"][0].get("locator").is_none());
    assert!(canonical["edges"][0].get("locator").is_none());

    let source_map = GraphSourceMap {
        graph: Some(pointer("")),
        nodes: vec![pointer("/nodes/0"), pointer("/nodes/1")],
        edges: vec![pointer("/edges/0")],
    };
    let result = GraphParseResult {
        envelope: grist::graph::GraphEnvelope::new(
            ArtifactKind::GraphDocument,
            SourceInfo::stdin("graph.json"),
            ParserInfo::new("grist.graph").with_feature("graph"),
            GraphDocument::SCHEMA_VERSION,
            document.clone(),
        ),
        source_map: source_map.clone(),
    };
    let round_trip: GraphParseResult =
        serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
    assert_eq!(round_trip.source_map, source_map);
    assert_eq!(round_trip.envelope.payload, Some(document));

    let omitted = serde_json::from_value::<GraphEdge>(json!({
        "id": "e:omitted",
        "source": "n:1",
        "target": "n:2"
    }));
    assert!(
        omitted.is_err(),
        "canonical GraphEdge requires materialized direction"
    );
}

#[test]
fn graph_payload_uses_shared_envelope_identity_and_schema_metadata() {
    let document = sample_document();
    let envelope = grist::graph::GraphEnvelope::new(
        ArtifactKind::GraphDocument,
        SourceInfo::stdin("graph.json"),
        ParserInfo::new("grist.graph").with_feature("graph"),
        GraphDocument::SCHEMA_VERSION,
        document,
    )
    .with_canonical_payload_identity()
    .unwrap();
    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(
        value["payload_schema_version"],
        GraphDocument::SCHEMA_VERSION
    );
    assert_eq!(value["kind"], "graph_document");
    let identity: ContentIdentity = serde_json::from_value(value["identity"].clone()).unwrap();
    assert_eq!(
        identity
            .canonical_payload
            .as_ref()
            .unwrap()
            .payload_schema_version
            .0,
        GraphDocument::SCHEMA_VERSION
    );
}

#[test]
fn graph_options_reuse_the_shared_request_budget_contract() {
    assert_eq!(GraphOptions::FORMAT, "graph");
    let request: grist::graph::GraphParseRequest = ParseRequest::new(
        RequestId::new("graph/1").unwrap(),
        Input::utf8("{}"),
        SourceInfo::stdin("graph.json"),
        BudgetSelection::Profile(BudgetProfile::UntrustedServiceV1),
        ProviderSet::none(),
    )
    .with_format_options(GraphOptions {
        encoding: Some(GraphInputEncoding::Json),
    });
    assert_eq!(
        request.budget,
        BudgetSelection::Profile(BudgetProfile::UntrustedServiceV1)
    );
    assert_eq!(
        request.format_hint.and_then(|hint| hint.format),
        Some("graph".into())
    );
}

#[test]
fn graph_errors_are_shared_diagnostics_with_stable_codes_and_locators() {
    let diagnostic = Diagnostic::malformed("grist.graph", "duplicate node ID")
        .with_locator(pointer("/nodes/1/id"));
    let mut diagnostic = diagnostic;
    diagnostic.code = diagnostic_codes::NODE_ID_DUPLICATE.into();
    let error = GraphError::from(diagnostic.clone());
    assert_eq!(error.as_diagnostic(), &diagnostic);
    assert_eq!(
        error.to_string(),
        "grist.graph.node.id.duplicate: duplicate node ID"
    );
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        serde_json::to_value(&diagnostic).unwrap()
    );
}

#[test]
fn graph_structural_types_reject_unknown_fields() {
    let error = serde_json::from_value::<GraphNode>(json!({
        "id": "n1",
        "unknown": true
    }))
    .unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[cfg(feature = "schemas")]
#[test]
fn graph_public_types_participate_in_shared_schema_generation() {
    let payload = grist::schema::schema_for_type::<GraphDocument>();
    let options = grist::schema::schema_for_type::<GraphOptions>();
    let error = grist::schema::schema_for_type::<GraphError>();
    assert_eq!(payload["title"], "GraphDocument");
    assert_eq!(options["title"], "GraphOptions");
    assert_eq!(
        error["title"], "Diagnostic",
        "the transparent graph error must retain the shared diagnostic schema"
    );
}
