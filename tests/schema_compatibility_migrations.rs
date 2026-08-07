use grist::core::SchemaVersion;
use grist::schema::{
    BackendOutputManifest, BackendOutputPolicyError, CompatibilityChangeKind, CompatibilityPolicy,
    ForwardCompatible, MigrationRegistry, SchemaKind, builtin_migration_registry,
    canonical_examples, check_schema_compatibility, deserialize_forward_compatible,
    enforce_backend_output_change, enforce_schema_compatibility, schema_catalog, schema_json,
    schema_json_version, validate_schema,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

#[test]
fn catalog_covers_every_public_contract_class_and_emits_registered_versions() {
    let catalog = schema_catalog();
    let names = catalog
        .schemas
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), catalog.schemas.len());
    for required in [
        "envelope",
        "file-ingest-envelope",
        "document-graph",
        "segment",
        "segment-event",
        "diagnostic",
        "parse-options",
        "segment-options",
        "provider-request-manifest",
        "registry-snapshot",
        "schema-catalog",
        "migration-manifest",
        "backend-output-manifest",
    ] {
        assert!(names.contains(required), "missing public schema {required}");
    }
    let kinds = catalog
        .schemas
        .iter()
        .map(|entry| entry.kind)
        .collect::<BTreeSet<_>>();
    for kind in [
        SchemaKind::Envelope,
        SchemaKind::Payload,
        SchemaKind::Graph,
        SchemaKind::Segment,
        SchemaKind::Event,
        SchemaKind::Diagnostic,
        SchemaKind::Options,
        SchemaKind::Manifest,
    ] {
        assert!(kinds.contains(&kind), "missing schema class {kind:?}");
    }
    for entry in catalog.schemas {
        assert!(
            schema_json(&entry.name).is_some(),
            "{} cannot emit",
            entry.name
        );
        assert!(
            schema_json_version(&entry.name, &entry.schema_version).is_some(),
            "{} version is not registered",
            entry.name
        );
    }
    assert!(schema_json_version("document-graph", SchemaVersion::DOCUMENT_GRAPH_V1).is_some());
    assert!(schema_json_version("serialization-envelope", SchemaVersion::ENVELOPE_V1).is_some());
}

#[test]
fn generated_schemas_and_canonical_examples_have_no_drift() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for descriptor in schema_catalog().schemas {
        let schema = schema_json(&descriptor.name).expect("registered schema");
        let expected = pretty_json(&schema);
        let actual = fs::read_to_string(root.join("schemas").join(&descriptor.file_name))
            .unwrap_or_else(|error| panic!("missing {}: {error}", descriptor.file_name));
        assert_eq!(actual, expected, "schema drift for {}", descriptor.name);
    }
    let expected = pretty_json(&canonical_examples().expect("canonical examples"));
    let actual = fs::read_to_string(root.join("examples/schema-canonical-examples.v1.json"))
        .expect("checked-in canonical examples");
    assert_eq!(actual, expected, "canonical example drift");
}

#[test]
fn every_canonical_example_validates_against_its_registered_schema() {
    let manifest = canonical_examples().expect("canonical examples");
    assert_eq!(manifest.examples.len(), schema_catalog().schemas.len());
    for (name, example) in manifest.examples {
        let report = validate_schema(&name, &example.value).expect("valid registered schema");
        assert!(report.valid, "invalid {name} example: {:?}", report.issues);
    }
}

#[test]
fn builtin_and_consumer_migrations_use_deterministic_registered_paths() {
    let registry = builtin_migration_registry();
    let legacy = json!({
        "schema_version": SchemaVersion::ENVELOPE_V1,
        "kind": "text",
        "source": {"display_name": "input.txt"},
        "hashes": null,
        "parser": {"name": "text", "version": "0.1.0"},
        "diagnostics": [],
        "payload_schema_version": SchemaVersion::TEXT_V1,
        "payload": {"schema_version": SchemaVersion::TEXT_V1, "blocks": []}
    });
    let migrated = registry
        .migrate(
            "envelope",
            SchemaVersion::ENVELOPE_V1,
            SchemaVersion::ENVELOPE_V2,
            legacy,
        )
        .expect("built-in envelope migration");
    assert_eq!(
        migrated.path,
        [SchemaVersion::ENVELOPE_V1, SchemaVersion::ENVELOPE_V2]
    );
    assert_eq!(migrated.value["operation"], "parse");
    assert_eq!(migrated.value["status"], "complete");
    assert!(validate_schema("envelope", &migrated.value).unwrap().valid);

    let mut graph = grist::document_graph::DocumentGraph::new(
        "document",
        grist::document_graph::DocumentKind::Text,
    );
    graph.schema_version = SchemaVersion::DOCUMENT_GRAPH_V1.into();
    let migrated_graph = registry
        .migrate(
            "document-graph",
            SchemaVersion::DOCUMENT_GRAPH_V1,
            SchemaVersion::DOCUMENT_GRAPH_V2,
            serde_json::to_value(graph).unwrap(),
        )
        .expect("built-in graph migration");
    assert_eq!(
        migrated_graph.value["schema_version"],
        SchemaVersion::DOCUMENT_GRAPH_V2
    );

    let mut custom = MigrationRegistry::new();
    for version in ["example/v1", "example/v2", "example/v3"] {
        custom.register_version("example", version);
    }
    custom
        .register_migration("example", "example/v1", "example/v2", |mut value| {
            value["step"] = json!(1);
            Ok(value)
        })
        .unwrap();
    custom
        .register_migration("example", "example/v2", "example/v3", |mut value| {
            value["step"] = json!(2);
            Ok(value)
        })
        .unwrap();
    let migrated = custom
        .migrate("example", "example/v1", "example/v3", json!({}))
        .unwrap();
    assert_eq!(migrated.path, ["example/v1", "example/v2", "example/v3"]);
    assert_eq!(migrated.value["step"], 2);
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum KnownMode {
    Stable,
}

#[test]
fn unknown_enum_values_are_recovered_without_losing_the_document() {
    let schema = json!({"type":"string", "enum":["stable"]});
    let value = json!("future_mode");
    let recovered = deserialize_forward_compatible::<KnownMode>(value.clone(), &schema).unwrap();
    let ForwardCompatible::UnknownEnums(document) = recovered else {
        panic!("unknown value must use explicit recovery representation");
    };
    assert_eq!(document.value, value);
    assert_eq!(document.unknown_values[0].value, "future_mode");
    assert_eq!(document.unknown_values[0].known_values, [json!("stable")]);
}

#[test]
fn compatibility_policy_requires_versions_for_breaking_changes() {
    let previous = json!({
        "type":"object",
        "properties":{"mode":{"type":"string","enum":["stable"]}},
        "required":["mode"]
    });
    let optional = json!({
        "type":"object",
        "properties":{
            "mode":{"type":"string","enum":["stable","future"]},
            "note":{"type":"string"}
        },
        "required":["mode"]
    });
    let policy = CompatibilityPolicy::default();
    let report = check_schema_compatibility(&previous, &optional, "shape/v1", "shape/v1", &policy);
    assert!(report.compatible);
    assert!(report.changes.iter().any(|change| {
        change.kind == CompatibilityChangeKind::EnumValueAdded && !change.breaking
    }));
    assert!(
        enforce_schema_compatibility(&previous, &optional, "shape/v1", "shape/v1", &policy).is_ok()
    );

    let required = json!({
        "type":"object",
        "properties":{
            "mode":{"type":"string","enum":["stable"]},
            "note":{"type":"string"}
        },
        "required":["mode","note"]
    });
    let report = check_schema_compatibility(&previous, &required, "shape/v1", "shape/v1", &policy);
    assert!(report.requires_new_schema_version);
    assert!(
        enforce_schema_compatibility(&previous, &required, "shape/v1", "shape/v1", &policy)
            .is_err()
    );
}

#[test]
fn material_backend_output_changes_require_metadata_and_golden_updates() {
    let previous = backend_manifest("1.0", "sha256:old", "sha256:gold-old");
    let stale_metadata = backend_manifest("1.0", "sha256:new", "sha256:gold-new");
    assert_eq!(
        enforce_backend_output_change(&previous, &stale_metadata),
        Err(BackendOutputPolicyError::BackendVersionNotUpdated)
    );
    let stale_golden = backend_manifest("2.0", "sha256:new", "sha256:gold-old");
    assert_eq!(
        enforce_backend_output_change(&previous, &stale_golden),
        Err(BackendOutputPolicyError::GoldenFixtureNotUpdated)
    );
    let current = backend_manifest("2.0", "sha256:new", "sha256:gold-new");
    enforce_backend_output_change(&previous, &current).unwrap();
}

fn backend_manifest(
    backend_version: &str,
    canonical_output_sha256: &str,
    golden_fixture_sha256: &str,
) -> BackendOutputManifest {
    BackendOutputManifest {
        schema_version: SchemaVersion::BACKEND_OUTPUT_MANIFEST_V1.into(),
        parser: "example".into(),
        backend: "example-backend".into(),
        backend_version: backend_version.into(),
        payload_schema_version: "example/payload/v1".into(),
        canonical_output_sha256: canonical_output_sha256.into(),
        golden_fixture_sha256: golden_fixture_sha256.into(),
    }
}

fn pretty_json(value: &impl serde::Serialize) -> String {
    let mut json = serde_json::to_string_pretty(value).unwrap();
    json.push('\n');
    json
}
