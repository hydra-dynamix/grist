#![cfg(feature = "schemas")]

#[cfg(feature = "graph")]
use grist::schema::schema_json_version;
use grist::schema::{builtin_migration_registry, schema_catalog, schema_json};
#[cfg(feature = "graph")]
use std::collections::BTreeMap;

const GRAPH_SCHEMA_NAMES: [&str; 6] = [
    "graph",
    "graph-envelope",
    "graph-options",
    "graph-parse-result",
    "graph-source-map",
    "graph-stream-event",
];

#[cfg(feature = "graph")]
#[test]
fn graph_feature_registers_only_the_public_contract_surface() {
    use grist::schema::SchemaKind;

    let descriptors = schema_catalog()
        .schemas
        .into_iter()
        .filter(|descriptor| GRAPH_SCHEMA_NAMES.contains(&descriptor.name.as_str()))
        .map(|descriptor| (descriptor.name.clone(), descriptor))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(descriptors.len(), GRAPH_SCHEMA_NAMES.len());
    for name in GRAPH_SCHEMA_NAMES {
        assert!(
            descriptors.contains_key(name),
            "missing graph schema {name}"
        );
        assert!(
            schema_json(name).is_some(),
            "graph schema {name} cannot emit"
        );
        assert!(
            schema_json_version(name, &descriptors[name].schema_version).is_some(),
            "graph schema {name} cannot emit its registered version"
        );
    }

    let payload = &descriptors["graph"];
    assert_eq!(payload.family, "graph-document");
    assert_eq!(payload.schema_version, "grist/graph-document/v1");
    assert_eq!(payload.file_name, "grist.graph-document.v1.schema.json");
    assert_eq!(
        payload.canonical_example,
        "examples/schema-canonical-examples.v1.json#/examples/graph"
    );

    let envelope = &descriptors["graph-envelope"];
    assert_eq!(envelope.family, "envelope");
    assert_eq!(envelope.schema_version, "grist/envelope/v2");

    let event = &descriptors["graph-stream-event"];
    assert_eq!(event.schema_version, "grist/graph-stream-event/v1");

    for (name, expected_kind) in [
        ("graph", SchemaKind::Graph),
        ("graph-envelope", SchemaKind::Envelope),
        ("graph-options", SchemaKind::Options),
        ("graph-parse-result", SchemaKind::Payload),
        ("graph-source-map", SchemaKind::Contract),
        ("graph-stream-event", SchemaKind::Event),
    ] {
        assert_eq!(
            descriptors[name].kind, expected_kind,
            "wrong kind for {name}"
        );
    }

    // ParseRequest intentionally contains input/provider secrets and therefore
    // has no serialization or schema contract. Do not invent one here.
    assert!(schema_json("graph-parse-request").is_none());
}

#[cfg(not(feature = "graph"))]
#[test]
fn graph_feature_off_omits_every_graph_input_schema() {
    let names = schema_catalog()
        .schemas
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect::<Vec<_>>();

    for name in GRAPH_SCHEMA_NAMES {
        assert!(!names.iter().any(|candidate| candidate == name));
        assert!(schema_json(name).is_none());
    }
    assert!(
        !builtin_migration_registry()
            .manifest()
            .versions
            .iter()
            .any(|version| version.family == "graph-document")
    );
}

#[cfg(feature = "graph")]
#[test]
fn graph_schemas_are_closed_and_canonical_examples_validate() {
    use grist::core::{ArtifactKind, ParserInfo, SourceInfo, StreamEvent, StreamTerminal};
    use grist::graph::{GraphDocument, GraphEdge, GraphOptions, GraphParseResult, GraphSourceMap};
    use grist::schema::{canonical_example_json, canonical_examples, validate_schema};

    let mut document = GraphDocument::new(true);
    document.nodes.push(grist::graph::GraphNode::new("n1"));
    document.nodes.push(grist::graph::GraphNode::new("n2"));
    document.edges.push(GraphEdge::new("e1", "n1", "n2", true));
    let result = GraphParseResult {
        envelope: grist::graph::GraphEnvelope::new(
            ArtifactKind::GraphDocument,
            SourceInfo::stdin("graph.json"),
            ParserInfo::new("grist.graph").with_feature("graph"),
            GraphDocument::SCHEMA_VERSION,
            document,
        ),
        source_map: GraphSourceMap::default(),
    };

    let values = BTreeMap::from([
        (
            "graph",
            serde_json::to_value(result.envelope.payload.as_ref().unwrap()).unwrap(),
        ),
        (
            "graph-envelope",
            serde_json::to_value(&result.envelope).unwrap(),
        ),
        (
            "graph-options",
            serde_json::to_value(GraphOptions::default()).unwrap(),
        ),
        ("graph-parse-result", serde_json::to_value(&result).unwrap()),
        (
            "graph-source-map",
            serde_json::to_value(&result.source_map).unwrap(),
        ),
        (
            "graph-stream-event",
            serde_json::to_value(StreamEvent::<GraphParseResult>::terminal(
                StreamTerminal::complete(0, Default::default()),
            ))
            .unwrap(),
        ),
    ]);
    for (name, value) in values {
        let report = validate_schema(name, &value).expect("registered graph schema");
        assert!(report.valid, "invalid {name}: {:?}", report.issues);
    }

    let payload_schema = schema_json("graph").unwrap();
    assert_eq!(payload_schema["additionalProperties"], false);
    assert_eq!(
        payload_schema["definitions"]["GraphNode"]["additionalProperties"],
        false
    );
    assert_eq!(
        payload_schema["definitions"]["GraphEdge"]["additionalProperties"],
        false
    );
    for name in ["graph-options", "graph-parse-result", "graph-source-map"] {
        assert_eq!(schema_json(name).unwrap()["additionalProperties"], false);
    }

    let manifest = canonical_examples().expect("canonical examples");
    for name in GRAPH_SCHEMA_NAMES {
        assert!(manifest.examples.contains_key(name));
        let example = canonical_example_json(name).expect("canonical graph example");
        let report = validate_schema(name, &example).expect("registered graph schema");
        assert!(report.valid, "invalid {name} example: {:?}", report.issues);
    }
}

#[cfg(feature = "graph")]
#[test]
fn graph_v1_is_registered_without_inventing_a_migration() {
    let manifest = builtin_migration_registry().manifest();
    assert!(manifest.versions.iter().any(|version| {
        version.family == "graph-document" && version.version == "grist/graph-document/v1"
    }));
    assert!(
        manifest
            .migrations
            .iter()
            .all(|migration| migration.family != "graph-document")
    );
}
