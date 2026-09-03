//! Serializable schema-catalog, validation, migration, and backend policy records.

use crate::core::SchemaVersion;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SchemaKind {
    Envelope,
    Payload,
    Graph,
    Segment,
    Event,
    Diagnostic,
    Options,
    Manifest,
    #[default]
    Contract,
}

/// Backward-compatible compact schema-list record used by the CLI.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaEntry {
    pub name: String,
    pub schema_version: String,
}

/// Complete registration metadata for one current public wire contract.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaDescriptor {
    pub name: String,
    pub family: String,
    pub schema_version: String,
    pub kind: SchemaKind,
    pub file_name: String,
    pub canonical_example: String,
    pub backend_sensitive: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaCatalog {
    pub schema_version: String,
    pub schemas: Vec<SchemaDescriptor>,
}

impl SchemaCatalog {
    pub fn new(schemas: Vec<SchemaDescriptor>) -> Self {
        Self {
            schema_version: SchemaVersion::SCHEMA_CATALOG_V1.to_string(),
            schemas,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalExample {
    pub schema_name: String,
    pub schema_version: String,
    pub canonical_sha256: String,
    pub value: Value,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalExampleManifest {
    pub schema_version: String,
    pub examples: BTreeMap<String, CanonicalExample>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaValidationIssue {
    pub instance_path: String,
    pub schema_path: String,
    pub message: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaValidationReport {
    pub schema_version: String,
    pub schema_name: String,
    pub target_schema_version: String,
    pub valid: bool,
    pub issues: Vec<SchemaValidationIssue>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationDescriptor {
    pub family: String,
    pub from_version: String,
    pub to_version: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationManifest {
    pub schema_version: String,
    pub versions: Vec<RegisteredSchemaVersion>,
    pub migrations: Vec<MigrationDescriptor>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegisteredSchemaVersion {
    pub family: String,
    pub version: String,
}

/// Checked metadata required whenever a backend upgrade changes canonical output.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputManifest {
    pub schema_version: String,
    pub parser: String,
    pub backend: String,
    pub backend_version: String,
    pub payload_schema_version: String,
    pub canonical_output_sha256: String,
    pub golden_fixture_sha256: String,
}
