//! Deterministic registration and routing of consumer-supplied JSON migrations.

use super::{MigrationDescriptor, MigrationManifest, RegisteredSchemaVersion};
use crate::core::{SchemaVersion, empty_options_digest};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::Arc;

type MigrationFn = dyn Fn(Value) -> Result<Value, String> + Send + Sync + 'static;

#[derive(Clone)]
struct RegisteredMigration {
    descriptor: MigrationDescriptor,
    apply: Arc<MigrationFn>,
}

/// A migration result that records the exact ordered version path applied.
#[derive(Debug, Clone, PartialEq)]
pub struct MigrationResult {
    pub value: Value,
    pub path: Vec<String>,
}

/// Mutable registry so downstream consumers can register their own supported versions.
#[derive(Clone, Default)]
pub struct MigrationRegistry {
    versions: BTreeSet<RegisteredSchemaVersion>,
    migrations: BTreeMap<(String, String, String), RegisteredMigration>,
}

impl fmt::Debug for MigrationRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MigrationRegistry")
            .field("versions", &self.versions)
            .field("migrations", &self.migrations.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl MigrationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_version(
        &mut self,
        family: impl Into<String>,
        version: impl Into<String>,
    ) -> bool {
        self.versions.insert(RegisteredSchemaVersion {
            family: family.into(),
            version: version.into(),
        })
    }

    pub fn register_migration<F>(
        &mut self,
        family: impl Into<String>,
        from_version: impl Into<String>,
        to_version: impl Into<String>,
        migration: F,
    ) -> Result<(), MigrationRegistryError>
    where
        F: Fn(Value) -> Result<Value, String> + Send + Sync + 'static,
    {
        let descriptor = MigrationDescriptor {
            family: family.into(),
            from_version: from_version.into(),
            to_version: to_version.into(),
        };
        for version in [&descriptor.from_version, &descriptor.to_version] {
            if !self.versions.contains(&RegisteredSchemaVersion {
                family: descriptor.family.clone(),
                version: version.clone(),
            }) {
                return Err(MigrationRegistryError::UnregisteredVersion {
                    family: descriptor.family.clone(),
                    version: version.clone(),
                });
            }
        }
        let key = (
            descriptor.family.clone(),
            descriptor.from_version.clone(),
            descriptor.to_version.clone(),
        );
        if self.migrations.contains_key(&key) {
            return Err(MigrationRegistryError::DuplicateMigration {
                family: descriptor.family,
                from_version: descriptor.from_version,
                to_version: descriptor.to_version,
            });
        }
        self.migrations.insert(
            key,
            RegisteredMigration {
                descriptor,
                apply: Arc::new(migration),
            },
        );
        Ok(())
    }

    pub fn migrate(
        &self,
        family: &str,
        from_version: &str,
        to_version: &str,
        value: Value,
    ) -> Result<MigrationResult, MigrationRegistryError> {
        for version in [from_version, to_version] {
            if !self.versions.contains(&RegisteredSchemaVersion {
                family: family.to_string(),
                version: version.to_string(),
            }) {
                return Err(MigrationRegistryError::UnregisteredVersion {
                    family: family.to_string(),
                    version: version.to_string(),
                });
            }
        }
        if from_version == to_version {
            return Ok(MigrationResult {
                value,
                path: vec![from_version.to_string()],
            });
        }

        let route = self
            .route(family, from_version, to_version)
            .ok_or_else(|| MigrationRegistryError::NoMigrationPath {
                family: family.to_string(),
                from_version: from_version.to_string(),
                to_version: to_version.to_string(),
            })?;
        let mut migrated = value;
        for pair in route.windows(2) {
            let key = (family.to_string(), pair[0].clone(), pair[1].clone());
            let registered = self
                .migrations
                .get(&key)
                .expect("migration route only contains registered edges");
            migrated = (registered.apply)(migrated).map_err(|message| {
                MigrationRegistryError::MigrationFailed {
                    family: family.to_string(),
                    from_version: pair[0].clone(),
                    to_version: pair[1].clone(),
                    message,
                }
            })?;
        }
        Ok(MigrationResult {
            value: migrated,
            path: route,
        })
    }

    pub fn manifest(&self) -> MigrationManifest {
        MigrationManifest {
            schema_version: SchemaVersion::SCHEMA_MIGRATION_MANIFEST_V1.to_string(),
            versions: self.versions.iter().cloned().collect(),
            migrations: self
                .migrations
                .values()
                .map(|migration| migration.descriptor.clone())
                .collect(),
        }
    }

    fn route(&self, family: &str, from_version: &str, to_version: &str) -> Option<Vec<String>> {
        let mut queue = VecDeque::from([from_version.to_string()]);
        let mut previous = BTreeMap::<String, String>::new();
        let mut visited = BTreeSet::from([from_version.to_string()]);
        while let Some(current) = queue.pop_front() {
            let mut next = self
                .migrations
                .keys()
                .filter(|(edge_family, edge_from, _)| {
                    edge_family == family && edge_from == &current
                })
                .map(|(_, _, edge_to)| edge_to.clone())
                .collect::<Vec<_>>();
            next.sort();
            for candidate in next {
                if !visited.insert(candidate.clone()) {
                    continue;
                }
                previous.insert(candidate.clone(), current.clone());
                if candidate == to_version {
                    let mut path = vec![candidate];
                    while path.last().is_some_and(|version| version != from_version) {
                        let parent = previous.get(path.last().expect("path is non-empty"))?;
                        path.push(parent.clone());
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(candidate);
            }
        }
        None
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum MigrationRegistryError {
    #[error("schema family {family} has no registered version {version}")]
    UnregisteredVersion { family: String, version: String },
    #[error("migration {family} {from_version} -> {to_version} is already registered")]
    DuplicateMigration {
        family: String,
        from_version: String,
        to_version: String,
    },
    #[error("no migration path for {family} {from_version} -> {to_version}")]
    NoMigrationPath {
        family: String,
        from_version: String,
        to_version: String,
    },
    #[error("migration {family} {from_version} -> {to_version} failed: {message}")]
    MigrationFailed {
        family: String,
        from_version: String,
        to_version: String,
        message: String,
    },
}

/// Registry containing every migration currently implemented by Grist itself.
pub fn builtin_migration_registry() -> MigrationRegistry {
    let mut registry = MigrationRegistry::new();
    for (family, versions) in [
        (
            "envelope",
            [SchemaVersion::ENVELOPE_V1, SchemaVersion::ENVELOPE_V2],
        ),
        (
            "document-graph",
            [
                SchemaVersion::DOCUMENT_GRAPH_V1,
                SchemaVersion::DOCUMENT_GRAPH_V2,
            ],
        ),
    ] {
        for version in versions {
            registry.register_version(family, version);
        }
    }
    #[cfg(feature = "graph")]
    registry.register_version(
        "graph-document",
        crate::graph::GraphDocument::SCHEMA_VERSION,
    );
    registry
        .register_migration(
            "envelope",
            SchemaVersion::ENVELOPE_V1,
            SchemaVersion::ENVELOPE_V2,
            migrate_envelope_v1_to_v2,
        )
        .expect("built-in envelope migration registration is valid");
    registry
        .register_migration(
            "document-graph",
            SchemaVersion::DOCUMENT_GRAPH_V1,
            SchemaVersion::DOCUMENT_GRAPH_V2,
            migrate_document_graph_v1_to_v2,
        )
        .expect("built-in graph migration registration is valid");
    registry
}

fn migrate_envelope_v1_to_v2(mut value: Value) -> Result<Value, String> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| "envelope must be a JSON object".to_string())?;
    if object.get("schema_version").and_then(Value::as_str) != Some(SchemaVersion::ENVELOPE_V1) {
        return Err("envelope does not declare grist/envelope/v1".into());
    }
    object.insert(
        "schema_version".into(),
        Value::String(SchemaVersion::ENVELOPE_V2.to_string()),
    );
    object
        .entry("operation")
        .or_insert_with(|| Value::String("parse".into()));
    object
        .entry("status")
        .or_insert_with(|| Value::String("complete".into()));
    object.entry("identity").or_insert(Value::Null);
    object
        .entry("options_digest")
        .or_insert_with(|| Value::String(empty_options_digest()));
    object
        .entry("providers")
        .or_insert_with(|| Value::Array(Vec::new()));
    object
        .entry("provenance")
        .or_insert_with(|| Value::Array(Vec::new()));
    Ok(value)
}

fn migrate_document_graph_v1_to_v2(value: Value) -> Result<Value, String> {
    let graph: crate::document_graph::DocumentGraph =
        serde_json::from_value(value).map_err(|error| error.to_string())?;
    let graph = graph.migrate_to_v2().map_err(|error| error.to_string())?;
    serde_json::to_value(graph).map_err(|error| error.to_string())
}
