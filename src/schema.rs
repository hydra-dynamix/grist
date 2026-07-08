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
            name: "ldgr-projection".into(),
            schema_version: crate::core::SchemaVersion::LDGR_PROJECTION_V1.into(),
        },
        SchemaEntry {
            name: "document-graph".into(),
            schema_version: crate::core::SchemaVersion::DOCUMENT_GRAPH_V1.into(),
        },
        SchemaEntry {
            name: "html".into(),
            schema_version: crate::core::SchemaVersion::HTML_V1.into(),
        },
        SchemaEntry {
            name: "csv".into(),
            schema_version: crate::core::SchemaVersion::CSV_V1.into(),
        },
        SchemaEntry {
            name: "rust-code".into(),
            schema_version: crate::core::SchemaVersion::RUST_CODE_V1.into(),
        },
        SchemaEntry {
            name: "python-code".into(),
            schema_version: crate::core::SchemaVersion::PYTHON_CODE_V1.into(),
        },
        SchemaEntry {
            name: "typescript-code".into(),
            schema_version: crate::core::SchemaVersion::TYPESCRIPT_CODE_V1.into(),
        },
        SchemaEntry {
            name: "latex".into(),
            schema_version: crate::core::SchemaVersion::LATEX_V1.into(),
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
            name: "ldgr-projection-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "document-graph-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "html-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "csv-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "rust-code-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "python-code-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "typescript-code-envelope".into(),
            schema_version: crate::core::SchemaVersion::ENVELOPE_V1.into(),
        },
        SchemaEntry {
            name: "latex-envelope".into(),
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
        #[cfg(feature = "ldgr-projection")]
        "ldgr-projection" => Some(
            serde_json::to_value(schema_for!(crate::ldgr_projection::LdgrProjectionDocument))
                .ok()?,
        ),
        #[cfg(feature = "document-graph")]
        "document-graph" => {
            Some(serde_json::to_value(schema_for!(crate::document_graph::DocumentGraph)).ok()?)
        }
        #[cfg(feature = "html")]
        "html" => Some(serde_json::to_value(schema_for!(crate::html::HtmlDocument)).ok()?),
        #[cfg(feature = "csv")]
        "csv" => Some(serde_json::to_value(schema_for!(crate::csv::CsvDocument)).ok()?),
        #[cfg(feature = "rust")]
        "rust-code" => Some(serde_json::to_value(schema_for!(crate::rust::RustFile)).ok()?),
        #[cfg(feature = "python")]
        "python-code" => Some(serde_json::to_value(schema_for!(crate::python::PythonFile)).ok()?),
        #[cfg(feature = "typescript")]
        "typescript-code" => {
            Some(serde_json::to_value(schema_for!(crate::typescript::TypeScriptFile)).ok()?)
        }
        #[cfg(feature = "latex")]
        "latex" => Some(serde_json::to_value(schema_for!(crate::latex::LatexDocument)).ok()?),
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
        #[cfg(feature = "ldgr-projection")]
        "ldgr-projection-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::ldgr_projection::LdgrProjectionDocument>
            ))
            .ok()?,
        ),
        #[cfg(feature = "document-graph")]
        "document-graph-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::document_graph::DocumentGraph>
            ))
            .ok()?,
        ),
        #[cfg(feature = "html")]
        "html-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::html::HtmlDocument>
            ))
            .ok()?,
        ),
        #[cfg(feature = "csv")]
        "csv-envelope" => Some(
            serde_json::to_value(schema_for!(crate::core::Envelope<crate::csv::CsvDocument>))
                .ok()?,
        ),
        #[cfg(feature = "rust")]
        "rust-code-envelope" => Some(
            serde_json::to_value(schema_for!(crate::core::Envelope<crate::rust::RustFile>)).ok()?,
        ),
        #[cfg(feature = "python")]
        "python-code-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::python::PythonFile>
            ))
            .ok()?,
        ),
        #[cfg(feature = "typescript")]
        "typescript-code-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::typescript::TypeScriptFile>
            ))
            .ok()?,
        ),
        #[cfg(feature = "latex")]
        "latex-envelope" => Some(
            serde_json::to_value(schema_for!(
                crate::core::Envelope<crate::latex::LatexDocument>
            ))
            .ok()?,
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
                "ldgr-projection",
                include_str!("../schemas/grist.ldgr-projection.v1.schema.json"),
            ),
            (
                "document-graph",
                include_str!("../schemas/grist.document-graph.v1.schema.json"),
            ),
            ("html", include_str!("../schemas/grist.html.v1.schema.json")),
            ("csv", include_str!("../schemas/grist.csv.v1.schema.json")),
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
                "ldgr-projection-envelope",
                include_str!("../schemas/grist.ldgr-projection-envelope.v1.schema.json"),
            ),
            (
                "document-graph-envelope",
                include_str!("../schemas/grist.document-graph-envelope.v1.schema.json"),
            ),
            (
                "html-envelope",
                include_str!("../schemas/grist.html-envelope.v1.schema.json"),
            ),
            (
                "csv-envelope",
                include_str!("../schemas/grist.csv-envelope.v1.schema.json"),
            ),
            (
                "rust-code-envelope",
                include_str!("../schemas/grist.rust-code-envelope.v1.schema.json"),
            ),
            (
                "python-code",
                include_str!("../schemas/grist.python-code.v1.schema.json"),
            ),
            (
                "python-code-envelope",
                include_str!("../schemas/grist.python-code-envelope.v1.schema.json"),
            ),
            (
                "typescript-code",
                include_str!("../schemas/grist.typescript-code.v1.schema.json"),
            ),
            (
                "typescript-code-envelope",
                include_str!("../schemas/grist.typescript-code-envelope.v1.schema.json"),
            ),
            (
                "latex",
                include_str!("../schemas/grist.latex.v1.schema.json"),
            ),
            (
                "latex-envelope",
                include_str!("../schemas/grist.latex-envelope.v1.schema.json"),
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
