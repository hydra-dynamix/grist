# Schema compatibility and migrations

Grist registers every current public envelope, payload, graph, segment, event,
diagnostic, serializable options file, and manifest in one catalog. The compact
`schema::list_schemas()` API remains the CLI list surface. The detailed
`schema::schema_catalog()` surface adds family, contract class, checked-in file,
canonical-example location, and backend-sensitivity metadata.

Use `schema::schema_json(name)` for the current version,
`schema::schema_json_version(name, version)` for an explicitly registered
version, and `schema::validate_schema(name, value)` to validate a JSON value.
The validation result is itself a versioned public payload. Historical envelope
v1 and DocumentGraph v1 schemas remain registered while v2 is current.

## Checked artifacts

Run the following configured generator to update artifacts:

```text
cargo run --example schema_codegen --features "ldgr-projection latex basin"
```

Use `--check` in CI. It compares every catalog entry with its file under
`schemas/` and also checks `examples/schema-canonical-examples.v1.json`.
Canonical examples are generated deterministically, hashed with Grist canonical
JSON v1, and validated against their named schemas in the compatibility tests.

## Compatibility policy

`check_schema_compatibility` reports optional and required field additions,
field removals, type changes, enum additions/removals, and caller-declared
meaning, canonicalization, locator, or behavior changes. Optional fields are
patch-compatible. A breaking change with an unchanged schema version is
rejected by `enforce_schema_compatibility`; patch policy also rejects breaking
changes even when a new version is proposed.

Enum additions are compatible only when the caller enables the explicit
unknown-enum recovery contract. `deserialize_forward_compatible` first attempts
typed decoding. If decoding fails and the schema identifies unknown enum
values, it returns `ForwardCompatible::UnknownEnums` with the complete raw
JSON document, JSON pointers, received values, and known values. It never drops
or silently maps the new value.

## Migrations

`MigrationRegistry` lets consumers register schema-family versions and pure JSON
migration functions. Routing is deterministic and records the complete applied
version path. `builtin_migration_registry()` includes envelope v1 to v2 and
DocumentGraph v1 to v2. Missing versions, duplicate edges, absent paths, and
migration failures are explicit errors. The registry's versioned migration
manifest is serializable and has its own checked schema.

## Backend output drift

Backend-sensitive catalog entries must carry parser/backend metadata in their
golden workflow. `enforce_backend_output_change` compares two
`BackendOutputManifest` values. A material canonical-output change is rejected
unless both backend version metadata and the golden-fixture digest changed.
Schema-shape or semantic incompatibilities additionally require the normal
schema-version compatibility check.
