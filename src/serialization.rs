use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, ParserInfo, SchemaVersion, SourceInfo,
};
use crate::detect::ContentKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SerializationPayload {
    pub schema_version: String,
    pub format: SerializationFormat,
    pub value: Option<Value>,
    pub jsonl_records: Vec<JsonlRecord>,
    pub validation: Option<SchemaValidationResult>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SerializationFormat {
    Json,
    Jsonl,
    Yaml,
    Toml,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonlRecord {
    pub line: usize,
    pub value: Option<Value>,
    pub diagnostic: Option<Diagnostic>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchemaValidationResult {
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Default)]
pub struct SerializationOptions {
    pub schema: Option<Value>,
}

pub type SerializationEnvelope = Envelope<SerializationPayload>;

pub fn parse_serialization(
    text: &str,
    format: SerializationFormat,
    source: SourceInfo,
) -> SerializationEnvelope {
    parse_serialization_with_options(text, format, source, &SerializationOptions::default())
}

pub fn parse_serialization_with_options(
    text: &str,
    format: SerializationFormat,
    source: SourceInfo,
    options: &SerializationOptions,
) -> SerializationEnvelope {
    let mut diagnostics = Vec::new();
    let mut payload = SerializationPayload {
        schema_version: SchemaVersion::SERIALIZATION_V1.to_string(),
        format: format.clone(),
        value: None,
        jsonl_records: Vec::new(),
        validation: None,
    };

    match format {
        SerializationFormat::Json => match serde_json::from_str::<Value>(text) {
            Ok(value) => payload.value = Some(value),
            Err(err) => diagnostics.push(Diagnostic::error(
                "grist.serialization.json",
                "json.parse",
                format!("JSON parse failed: {err}"),
            )),
        },
        SerializationFormat::Jsonl => {
            let mut values = Vec::new();
            for (line_idx, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(line) {
                    Ok(value) => {
                        values.push(value.clone());
                        payload.jsonl_records.push(JsonlRecord {
                            line: line_idx + 1,
                            value: Some(value),
                            diagnostic: None,
                        });
                    }
                    Err(err) => {
                        let diagnostic = Diagnostic::error(
                            "grist.serialization.jsonl",
                            "jsonl.line_parse",
                            format!("JSONL line {} parse failed: {err}", line_idx + 1),
                        )
                        .partial();
                        payload.jsonl_records.push(JsonlRecord {
                            line: line_idx + 1,
                            value: None,
                            diagnostic: Some(diagnostic.clone()),
                        });
                        diagnostics.push(diagnostic);
                    }
                }
            }
            payload.value = Some(Value::Array(values));
        }
        SerializationFormat::Yaml => match serde_yaml::from_str::<serde_yaml::Value>(text) {
            Ok(value) => match serde_json::to_value(value) {
                Ok(value) => payload.value = Some(value),
                Err(err) => diagnostics.push(Diagnostic::error(
                    "grist.serialization.yaml",
                    "yaml.to_json",
                    format!("YAML value could not be converted to JSON value: {err}"),
                )),
            },
            Err(err) => diagnostics.push(Diagnostic::error(
                "grist.serialization.yaml",
                "yaml.parse",
                format!("YAML parse failed: {err}"),
            )),
        },
        SerializationFormat::Toml => match text.parse::<toml::Value>() {
            Ok(value) => match serde_json::to_value(value) {
                Ok(value) => payload.value = Some(value),
                Err(err) => diagnostics.push(Diagnostic::error(
                    "grist.serialization.toml",
                    "toml.to_json",
                    format!("TOML value could not be converted to JSON value: {err}"),
                )),
            },
            Err(err) => diagnostics.push(Diagnostic::error(
                "grist.serialization.toml",
                "toml.parse",
                format!("TOML parse failed: {err}"),
            )),
        },
    }

    if let (Some(value), Some(schema)) = (payload.value.as_ref(), options.schema.as_ref()) {
        let validation_diagnostics = validate_json_schema(value, schema);
        payload.validation = Some(SchemaValidationResult {
            valid: validation_diagnostics.is_empty(),
            diagnostics: validation_diagnostics.clone(),
        });
        diagnostics.extend(validation_diagnostics);
    }

    Envelope::new(
        ArtifactKind::Serialization,
        source,
        ParserInfo::new("grist.serialization"),
        SchemaVersion::SERIALIZATION_V1,
        payload,
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(diagnostics)
}

pub fn format_from_content_kind(kind: &ContentKind) -> Option<SerializationFormat> {
    match kind {
        ContentKind::Json => Some(SerializationFormat::Json),
        ContentKind::Jsonl => Some(SerializationFormat::Jsonl),
        ContentKind::Yaml => Some(SerializationFormat::Yaml),
        ContentKind::Toml => Some(SerializationFormat::Toml),
        _ => None,
    }
}

pub fn validate_json_schema(value: &Value, schema: &Value) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    match jsonschema::validator_for(schema) {
        Ok(validator) => {
            for error in validator.iter_errors(value) {
                diagnostics.push(Diagnostic::error(
                    "grist.serialization.schema",
                    "schema.validation",
                    format!(
                        "JSON Schema validation failed at {}: {}",
                        error.instance_path, error
                    ),
                ));
            }
        }
        Err(err) => diagnostics.push(Diagnostic::error(
            "grist.serialization.schema",
            "schema.invalid",
            format!("JSON Schema is invalid: {err}"),
        )),
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jsonl_with_line_diagnostics() {
        let report = parse_serialization(
            "{\"a\":1}\nnot-json\n{\"b\":2}",
            SerializationFormat::Jsonl,
            SourceInfo::stdin("input.jsonl"),
        );
        assert_eq!(report.payload.jsonl_records.len(), 3);
        assert_eq!(report.diagnostics.len(), 1);
    }

    #[test]
    fn validates_json_schema_when_supplied() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["name"],
            "properties": {"name": {"type": "string"}}
        });
        let report = parse_serialization_with_options(
            "{\"name\": 42}",
            SerializationFormat::Json,
            SourceInfo::stdin("input.json"),
            &SerializationOptions {
                schema: Some(schema),
            },
        );
        assert!(!report.payload.validation.unwrap().valid);
        assert!(!report.diagnostics.is_empty());
    }
}
