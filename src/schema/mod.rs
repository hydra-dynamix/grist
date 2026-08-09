//! Public schema discovery, generation, validation, migration, and compatibility.

mod catalog;
mod compatibility;
mod contracts;
mod examples;
mod migration;
mod recovery;
mod validation;

#[cfg(feature = "schemas")]
pub use catalog::schema_for_type;
pub use catalog::{
    list_schemas, schema_catalog, schema_descriptor, schema_json, schema_json_version,
};
pub use compatibility::{
    BackendOutputPolicyError, CompatibilityChange, CompatibilityChangeKind, CompatibilityPolicy,
    CompatibilityPolicyError, CompatibilityReport, ReleaseClass, SemanticChange,
    check_schema_compatibility, enforce_backend_output_change, enforce_schema_compatibility,
};
pub use contracts::{
    BackendOutputManifest, CanonicalExample, CanonicalExampleManifest, MigrationDescriptor,
    MigrationManifest, RegisteredSchemaVersion, SchemaCatalog, SchemaDescriptor, SchemaEntry,
    SchemaKind, SchemaValidationIssue, SchemaValidationReport,
};
pub use examples::{canonical_example_json, canonical_examples};
pub use migration::{
    MigrationRegistry, MigrationRegistryError, MigrationResult, builtin_migration_registry,
};
pub use recovery::{
    ForwardCompatibilityError, ForwardCompatible, UnknownEnumDocument, UnknownEnumValue,
    deserialize_forward_compatible, find_unknown_enum_values,
};
pub use validation::{
    SchemaValidationError, validate_against_schema, validate_schema, validate_schema_version,
};

#[cfg(all(test, feature = "schemas"))]
mod tests {
    use super::{canonical_examples, schema_catalog, schema_json};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn checked_in_schemas_match_generated_public_contracts() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for descriptor in schema_catalog().schemas {
            let generated = schema_json(&descriptor.name).expect("registered schema generator");
            let expected: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(root.join("schemas").join(&descriptor.file_name))
                    .unwrap_or_else(|error| panic!("missing {}: {error}", descriptor.file_name)),
            )
            .expect("checked-in schema JSON");
            assert_eq!(generated, expected, "schema drift for {}", descriptor.name);
        }
    }

    #[test]
    fn checked_in_canonical_examples_match_generated_values() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let generated = canonical_examples().expect("generated canonical examples");
        let checked_in =
            fs::read_to_string(root.join("examples/schema-canonical-examples.v1.json"))
                .expect("checked-in canonical examples");
        let checked_in: super::CanonicalExampleManifest =
            serde_json::from_str(&checked_in).expect("canonical example JSON");
        assert_eq!(generated.schema_version, checked_in.schema_version);
        for (name, example) in &generated.examples {
            assert_eq!(
                Some(example),
                checked_in.examples.get(name),
                "canonical example drift for {name}"
            );
        }
        if cfg!(feature = "full") {
            assert_eq!(
                generated.examples.len(),
                checked_in.examples.len(),
                "all-feature canonical example inventory drift"
            );
        }
    }
}
