//! Structural and semantic compatibility policy for public JSON contracts.

use super::BackendOutputManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseClass {
    #[default]
    Patch,
    Minor,
    Major,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityChangeKind {
    OptionalFieldAdded,
    RequiredFieldAdded,
    FieldRemoved,
    TypeChanged,
    EnumValueAdded,
    EnumValueRemoved,
    MeaningChanged,
    CanonicalizationChanged,
    LocatorChanged,
    BehaviorChanged,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatibilityChange {
    pub instance_path: String,
    pub kind: CompatibilityChangeKind,
    pub detail: String,
    pub breaking: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticChange {
    pub instance_path: String,
    pub kind: CompatibilityChangeKind,
    pub detail: String,
}

impl SemanticChange {
    pub fn meaning(path: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            instance_path: path.into(),
            kind: CompatibilityChangeKind::MeaningChanged,
            detail: detail.into(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatibilityPolicy {
    pub release: ReleaseClass,
    pub unknown_enum_recovery: bool,
    #[serde(default)]
    pub semantic_changes: Vec<SemanticChange>,
}

impl Default for CompatibilityPolicy {
    fn default() -> Self {
        Self {
            release: ReleaseClass::Patch,
            unknown_enum_recovery: true,
            semantic_changes: Vec::new(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompatibilityReport {
    pub previous_version: String,
    pub current_version: String,
    pub compatible: bool,
    pub requires_new_schema_version: bool,
    pub changes: Vec<CompatibilityChange>,
}

pub fn check_schema_compatibility(
    previous_schema: &Value,
    current_schema: &Value,
    previous_version: impl Into<String>,
    current_version: impl Into<String>,
    policy: &CompatibilityPolicy,
) -> CompatibilityReport {
    let previous_version = previous_version.into();
    let current_version = current_version.into();
    let mut changes = Vec::new();
    compare_schema("", previous_schema, current_schema, policy, &mut changes);
    changes.extend(
        policy
            .semantic_changes
            .iter()
            .map(|change| CompatibilityChange {
                instance_path: change.instance_path.clone(),
                kind: change.kind,
                detail: change.detail.clone(),
                breaking: true,
            }),
    );
    changes.sort_by(|left, right| {
        (
            &left.instance_path,
            format!("{:?}", left.kind),
            &left.detail,
        )
            .cmp(&(
                &right.instance_path,
                format!("{:?}", right.kind),
                &right.detail,
            ))
    });
    changes.dedup();
    let breaking = changes.iter().any(|change| change.breaking);
    CompatibilityReport {
        requires_new_schema_version: breaking && previous_version == current_version,
        compatible: !breaking,
        previous_version,
        current_version,
        changes,
    }
}

pub fn enforce_schema_compatibility(
    previous_schema: &Value,
    current_schema: &Value,
    previous_version: impl Into<String>,
    current_version: impl Into<String>,
    policy: &CompatibilityPolicy,
) -> Result<CompatibilityReport, CompatibilityPolicyError> {
    let report = check_schema_compatibility(
        previous_schema,
        current_schema,
        previous_version,
        current_version,
        policy,
    );
    if report.requires_new_schema_version {
        return Err(CompatibilityPolicyError::NewSchemaVersionRequired(report));
    }
    if policy.release == ReleaseClass::Patch && !report.compatible {
        return Err(CompatibilityPolicyError::BreakingPatchRelease(report));
    }
    Ok(report)
}

fn compare_schema(
    path: &str,
    previous: &Value,
    current: &Value,
    policy: &CompatibilityPolicy,
    changes: &mut Vec<CompatibilityChange>,
) {
    let (Some(previous_object), Some(current_object)) = (previous.as_object(), current.as_object())
    else {
        return;
    };

    if previous_object.get("type") != current_object.get("type")
        && current_object.contains_key("type")
    {
        changes.push(CompatibilityChange {
            instance_path: path.into(),
            kind: CompatibilityChangeKind::TypeChanged,
            detail: "JSON type constraint changed".into(),
            breaking: true,
        });
    }
    if previous_object.get("const") != current_object.get("const")
        && previous_object.contains_key("const")
    {
        changes.push(CompatibilityChange {
            instance_path: path.into(),
            kind: CompatibilityChangeKind::EnumValueRemoved,
            detail: "constant value constraint changed".into(),
            breaking: true,
        });
    }
    if previous_object.get("additionalProperties") != Some(&Value::Bool(false))
        && current_object.get("additionalProperties") == Some(&Value::Bool(false))
    {
        changes.push(CompatibilityChange {
            instance_path: path.into(),
            kind: CompatibilityChangeKind::FieldRemoved,
            detail: "unknown object fields are no longer accepted".into(),
            breaking: true,
        });
    }

    compare_enums(
        path,
        previous_object.get("enum"),
        current_object.get("enum"),
        policy,
        changes,
    );

    let previous_required = string_set(previous_object.get("required"));
    let current_required = string_set(current_object.get("required"));
    let previous_properties = previous_object.get("properties").and_then(Value::as_object);
    let current_properties = current_object.get("properties").and_then(Value::as_object);
    if let (Some(previous_properties), Some(current_properties)) =
        (previous_properties, current_properties)
    {
        for name in previous_properties.keys() {
            if !current_properties.contains_key(name) {
                changes.push(CompatibilityChange {
                    instance_path: child_path(path, name),
                    kind: CompatibilityChangeKind::FieldRemoved,
                    detail: format!("field `{name}` was removed"),
                    breaking: true,
                });
            }
        }
        for name in current_properties.keys() {
            if !previous_properties.contains_key(name) {
                let required = current_required.contains(name);
                changes.push(CompatibilityChange {
                    instance_path: child_path(path, name),
                    kind: if required {
                        CompatibilityChangeKind::RequiredFieldAdded
                    } else {
                        CompatibilityChangeKind::OptionalFieldAdded
                    },
                    detail: format!(
                        "{} field `{name}` was added",
                        if required { "required" } else { "optional" }
                    ),
                    breaking: required,
                });
            }
        }
        for name in previous_properties.keys() {
            if let Some(current_property) = current_properties.get(name) {
                compare_schema(
                    &child_path(path, name),
                    &previous_properties[name],
                    current_property,
                    policy,
                    changes,
                );
            }
        }
    }
    for name in current_required.difference(&previous_required) {
        if previous_properties.is_some_and(|properties| properties.contains_key(name)) {
            changes.push(CompatibilityChange {
                instance_path: child_path(path, name),
                kind: CompatibilityChangeKind::RequiredFieldAdded,
                detail: format!("existing field `{name}` became required"),
                breaking: true,
            });
        }
    }

    for keyword in ["$defs", "definitions", "items", "allOf", "anyOf", "oneOf"] {
        if let (Some(previous_child), Some(current_child)) =
            (previous_object.get(keyword), current_object.get(keyword))
        {
            compare_nested(path, previous_child, current_child, policy, changes);
        }
    }
}

fn compare_nested(
    path: &str,
    previous: &Value,
    current: &Value,
    policy: &CompatibilityPolicy,
    changes: &mut Vec<CompatibilityChange>,
) {
    match (previous, current) {
        (Value::Object(previous), Value::Object(current)) => {
            for key in previous.keys() {
                if let Some(current_value) = current.get(key) {
                    compare_schema(path, &previous[key], current_value, policy, changes);
                }
            }
        }
        (Value::Array(previous), Value::Array(current)) => {
            for (previous, current) in previous.iter().zip(current) {
                compare_schema(path, previous, current, policy, changes);
            }
        }
        _ => compare_schema(path, previous, current, policy, changes),
    }
}

fn compare_enums(
    path: &str,
    previous: Option<&Value>,
    current: Option<&Value>,
    policy: &CompatibilityPolicy,
    changes: &mut Vec<CompatibilityChange>,
) {
    let (Some(previous), Some(current)) = (
        previous.and_then(Value::as_array),
        current.and_then(Value::as_array),
    ) else {
        return;
    };
    for value in previous {
        if !current.contains(value) {
            changes.push(CompatibilityChange {
                instance_path: path.into(),
                kind: CompatibilityChangeKind::EnumValueRemoved,
                detail: format!("enum value {value} was removed"),
                breaking: true,
            });
        }
    }
    for value in current {
        if !previous.contains(value) {
            changes.push(CompatibilityChange {
                instance_path: path.into(),
                kind: CompatibilityChangeKind::EnumValueAdded,
                detail: format!("enum value {value} was added"),
                breaking: !policy.unknown_enum_recovery,
            });
        }
    }
}

fn string_set(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn child_path(path: &str, child: &str) -> String {
    format!("{path}/{}", child.replace('~', "~0").replace('/', "~1"))
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum CompatibilityPolicyError {
    #[error("incompatible contract changes require a new schema version")]
    NewSchemaVersionRequired(CompatibilityReport),
    #[error("patch releases cannot contain incompatible contract changes")]
    BreakingPatchRelease(CompatibilityReport),
}

/// Enforce metadata and golden updates when a backend materially changes output.
pub fn enforce_backend_output_change(
    previous: &BackendOutputManifest,
    current: &BackendOutputManifest,
) -> Result<(), BackendOutputPolicyError> {
    if previous.canonical_output_sha256 == current.canonical_output_sha256 {
        return Ok(());
    }
    if previous.parser != current.parser || previous.backend != current.backend {
        return Err(BackendOutputPolicyError::BackendIdentityChanged);
    }
    if previous.backend_version == current.backend_version {
        return Err(BackendOutputPolicyError::BackendVersionNotUpdated);
    }
    if previous.golden_fixture_sha256 == current.golden_fixture_sha256 {
        return Err(BackendOutputPolicyError::GoldenFixtureNotUpdated);
    }
    Ok(())
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum BackendOutputPolicyError {
    #[error("backend comparison must retain the same parser and backend identity")]
    BackendIdentityChanged,
    #[error("material output change requires updated backend version metadata")]
    BackendVersionNotUpdated,
    #[error("material output change requires updated golden fixtures")]
    GoldenFixtureNotUpdated,
}
