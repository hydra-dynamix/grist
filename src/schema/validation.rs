//! Validation of values against named, versioned public schemas.

#[cfg(feature = "schemas")]
use super::SchemaValidationIssue;
use super::{SchemaValidationReport, schema_descriptor, schema_json_version};
#[cfg(feature = "schemas")]
use crate::core::SchemaVersion;
use serde_json::Value;

pub fn validate_schema(
    name: &str,
    instance: &Value,
) -> Result<SchemaValidationReport, SchemaValidationError> {
    let descriptor =
        schema_descriptor(name).ok_or_else(|| SchemaValidationError::UnknownSchema {
            name: name.to_string(),
        })?;
    validate_schema_version(name, &descriptor.schema_version, instance)
}

pub fn validate_schema_version(
    name: &str,
    version: &str,
    instance: &Value,
) -> Result<SchemaValidationReport, SchemaValidationError> {
    let schema = schema_json_version(name, version).ok_or_else(|| {
        SchemaValidationError::UnknownSchemaVersion {
            name: name.to_string(),
            version: version.to_string(),
        }
    })?;
    validate_against_schema(name, version, instance, &schema)
}

pub fn validate_against_schema(
    name: &str,
    version: &str,
    instance: &Value,
    schema: &Value,
) -> Result<SchemaValidationReport, SchemaValidationError> {
    #[cfg(feature = "schemas")]
    {
        let validator = jsonschema::validator_for(schema)
            .map_err(|error| SchemaValidationError::InvalidSchema(error.to_string()))?;
        let issues = validator
            .iter_errors(instance)
            .map(|error| SchemaValidationIssue {
                instance_path: error.instance_path.to_string(),
                schema_path: error.schema_path.to_string(),
                message: error.to_string(),
            })
            .collect::<Vec<_>>();
        Ok(SchemaValidationReport {
            schema_version: SchemaVersion::SCHEMA_VALIDATION_V1.to_string(),
            schema_name: name.to_string(),
            target_schema_version: version.to_string(),
            valid: issues.is_empty(),
            issues,
        })
    }
    #[cfg(not(feature = "schemas"))]
    {
        let _ = (name, version, instance, schema);
        Err(SchemaValidationError::SchemasFeatureDisabled)
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum SchemaValidationError {
    #[error("unknown schema `{name}`")]
    UnknownSchema { name: String },
    #[error("unknown schema `{name}` version `{version}`")]
    UnknownSchemaVersion { name: String, version: String },
    #[error("invalid JSON Schema: {0}")]
    InvalidSchema(String),
    #[error("schema generation and validation require the `schemas` feature")]
    SchemasFeatureDisabled,
}
