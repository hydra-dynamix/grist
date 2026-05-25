use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::{JsonSchema, schema_for};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaEntry {
    pub name: String,
    pub schema_version: String,
}

pub fn list_schemas() -> Vec<SchemaEntry> {
    vec![
        SchemaEntry {
            name: "envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "repo-ingest".into(),
            schema_version: crate::core::SchemaVersion::REPO_INGEST_V1.into(),
        },
        SchemaEntry {
            name: "model-output".into(),
            schema_version: crate::core::SchemaVersion::MODEL_OUTPUT_V1.into(),
        },
        SchemaEntry {
            name: "serialization".into(),
            schema_version: crate::core::SchemaVersion::SERIALIZATION_V1.into(),
        },
        SchemaEntry {
            name: "markdown".into(),
            schema_version: crate::core::SchemaVersion::MARKDOWN_V1.into(),
        },
        SchemaEntry {
            name: "rust-code".into(),
            schema_version: crate::core::SchemaVersion::RUST_CODE_V1.into(),
        },
        SchemaEntry {
            name: "text".into(),
            schema_version: "grist/text/v1".into(),
        },
        SchemaEntry {
            name: "serialization-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "model-output-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "markdown-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "rust-code-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
    ]
}

#[cfg(feature = "schemas")]
pub fn schema_json(name: &str) -> Option<serde_json::Value> {
    match name {
        "diagnostic" => Some(serde_json::to_value(schema_for!(crate::core::Diagnostic)).ok()?),
        "repo-ingest" => {
            Some(serde_json::to_value(schema_for!(crate::ingest::RepoIngestReport)).ok()?)
        }
        #[cfg(feature = "model-output")]
        "model-output" => {
            Some(serde_json::to_value(schema_for!(crate::model_output::ModelOutputReport)).ok()?)
        }
        #[cfg(feature = "serialization")]
        "serialization" => Some(
            serde_json::to_value(schema_for!(crate::serialization::SerializationPayload)).ok()?,
        ),
        #[cfg(feature = "markdown")]
        "markdown" => {
            Some(serde_json::to_value(schema_for!(crate::markdown::MarkdownDocument)).ok()?)
        }
        #[cfg(feature = "rust")]
        "rust-code" => Some(serde_json::to_value(schema_for!(crate::rust::RustFile)).ok()?),
        "text" => Some(serde_json::to_value(schema_for!(crate::text::TextDocument)).ok()?),
        #[cfg(feature = "serialization")]
        "serialization-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::serialization::SerializationPayload>
            ))
            .ok()?,
        ),
        #[cfg(feature = "model-output")]
        "model-output-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::model_output::ModelOutputReport>
            ))
            .ok()?,
        ),
        #[cfg(feature = "markdown")]
        "markdown-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::markdown::MarkdownDocument>
            ))
            .ok()?,
        ),
        #[cfg(feature = "rust")]
        "rust-code-envelope" => Some(
            serde_json::to_value(schema_for!(crate::core::Envelope<crate::rust::RustFile>)).ok()?,
        ),
        _ => None,
    }
}

#[cfg(not(feature = "schemas"))]
pub fn schema_json(_name: &str) -> Option<serde_json::Value> {
    None
}

#[cfg(feature = "schemas")]
pub fn schema_for_type<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schema_for!(T)).expect("schema serialization should not fail")
}

#[cfg(all(test, feature = "schemas"))]
mod tests {
    use super::schema_json;

    #[test]
    fn checked_in_schemas_match_generated_public_contracts() {
        let fixtures = [
            (
                "diagnostic",
                include_str!("../schemas/grist.diagnostic.v1.schema.json"),
            ),
            (
                "repo-ingest",
                include_str!("../schemas/grist.repo-ingest.v1.schema.json"),
            ),
            (
                "model-output",
                include_str!("../schemas/grist.model-output.v1.schema.json"),
            ),
            (
                "serialization",
                include_str!("../schemas/grist.serialization.v1.schema.json"),
            ),
            (
                "markdown",
                include_str!("../schemas/grist.markdown.v1.schema.json"),
            ),
            (
                "rust-code",
                include_str!("../schemas/grist.rust-code.v1.schema.json"),
            ),
            ("text", include_str!("../schemas/grist.text.v1.schema.json")),
            (
                "serialization-envelope",
                include_str!("../schemas/grist.serialization-envelope.v1.schema.json"),
            ),
            (
                "model-output-envelope",
                include_str!("../schemas/grist.model-output-envelope.v1.schema.json"),
            ),
            (
                "markdown-envelope",
                include_str!("../schemas/grist.markdown-envelope.v1.schema.json"),
            ),
            (
                "rust-code-envelope",
                include_str!("../schemas/grist.rust-code-envelope.v1.schema.json"),
            ),
        ];
        for (name, checked_in) in fixtures {
            let expected: serde_json::Value = serde_json::from_str(checked_in).unwrap();
            let generated =
                schema_json(name).unwrap_or_else(|| panic!("missing generated schema {name}"));
            assert_eq!(generated, expected, "schema drift for {name}");
        }
    }
}
