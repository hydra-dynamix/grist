use crate::core::{Diagnostic, SourceInfo};
use crate::model_output::{CandidateGrammar, ModelOutputOptions, parse_model_output};
use crate::rust::{RustIngestOptions, RustSymbol, parse_rust};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedModelOutput {
    pub ok: bool,
    pub grammar: String,
    pub command_name: Option<String>,
    pub argument_name: Option<String>,
    pub value: Option<Value>,
    pub normalizations: Vec<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

pub fn parse_strict_normalized_model_output(
    text: &str,
    accepted_commands: &[&str],
    accepted_arguments: &[&str],
) -> NormalizedModelOutput {
    if accepted_commands.is_empty() || accepted_arguments.is_empty() {
        return normalized_error(
            "model output parse failed: explicit command and argument names are required",
            Vec::new(),
        );
    }
    let options = ModelOutputOptions {
        accepted_commands: accepted_commands
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        accepted_arguments: accepted_arguments
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        ..Default::default()
    };
    let envelope = parse_model_output(text, SourceInfo::stdin("model-output"), &options);
    let warnings = flatten_diagnostics(&envelope.diagnostics);
    let matching = envelope
        .payload
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.grammar == CandidateGrammar::PythonStyleCommand
                && candidate
                    .command_name
                    .as_deref()
                    .is_some_and(|name| accepted_commands.contains(&name))
                && candidate
                    .argument_name
                    .as_deref()
                    .is_some_and(|name| accepted_arguments.contains(&name))
        })
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [candidate] => NormalizedModelOutput {
            ok: true,
            grammar: "python_style_command".into(),
            command_name: candidate.command_name.clone(),
            argument_name: candidate.argument_name.clone(),
            value: candidate.value.clone(),
            normalizations: candidate.normalizations.clone(),
            warnings: [warnings, flatten_diagnostics(&candidate.diagnostics)].concat(),
            error: None,
        },
        [] => normalized_error(
            "model output parse failed: expected explicit Python-style command grammar",
            warnings,
        ),
        _ => normalized_error(
            "model output parse failed: ambiguous matching candidates",
            warnings,
        ),
    }
}

pub fn parse_research_chain_model_output(
    text: &str,
    accepted_commands: &[&str],
    accepted_arguments: &[&str],
) -> NormalizedModelOutput {
    parse_strict_normalized_model_output(text, accepted_commands, accepted_arguments)
}

pub fn normalize_work_graph_value(mut value: Value) -> Value {
    if let Value::Object(map) = &mut value {
        unwrap_graph(map);
    }
    normalize_graph_aliases(&mut value);
    value
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeFacts {
    pub path: String,
    pub language: String,
    pub symbols: Vec<CodeSymbol>,
    pub imports: Vec<String>,
    pub tests: Vec<String>,
    pub generated: bool,
    pub vendor: bool,
    pub metadata: BTreeMap<String, Value>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: String,
    pub range: crate::core::SourceRange,
    pub visibility: String,
}

pub fn rust_code_facts(path: &str, source: &str, max_bytes: usize) -> CodeFacts {
    let truncated = source.len() > max_bytes;
    let bounded = if truncated {
        &source[..max_bytes]
    } else {
        source
    };
    let envelope = parse_rust(
        bounded,
        SourceInfo {
            path: Some(path.to_string()),
            display_name: path.to_string(),
        },
        &RustIngestOptions::default(),
    );
    let tests = envelope
        .payload
        .symbols
        .iter()
        .filter(|symbol| symbol.attributes.iter().any(|attr| attr.contains("test")))
        .map(|symbol| symbol.name.clone())
        .collect::<Vec<_>>();
    let mut metadata = BTreeMap::new();
    metadata.insert("input_truncated".into(), Value::Bool(truncated));
    metadata.insert(
        "parse_diagnostics".into(),
        serde_json::to_value(&envelope.diagnostics).unwrap_or(Value::Null),
    );
    CodeFacts {
        path: path.to_string(),
        language: "rust".into(),
        symbols: envelope.payload.symbols.iter().map(code_symbol).collect(),
        imports: envelope
            .payload
            .imports
            .iter()
            .map(|import| import.path.clone())
            .collect(),
        tests,
        generated: is_generated_path(path),
        vendor: is_vendor_path(path),
        metadata,
    }
}

fn normalized_error(error: impl Into<String>, warnings: Vec<String>) -> NormalizedModelOutput {
    NormalizedModelOutput {
        ok: false,
        grammar: "unparsed".into(),
        command_name: None,
        argument_name: None,
        value: None,
        normalizations: Vec::new(),
        warnings,
        error: Some(error.into()),
    }
}

fn flatten_diagnostics(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{}:{}: {}",
                diagnostic.parser, diagnostic.code, diagnostic.message
            )
        })
        .collect()
}

fn unwrap_graph(map: &mut Map<String, Value>) {
    for key in ["graph", "work_graph", "workGraph"] {
        if let Some(Value::Object(inner)) = map.remove(key) {
            *map = inner;
            return;
        }
    }
}

fn normalize_graph_aliases(value: &mut Value) {
    match value {
        Value::Object(map) => {
            rename(map, &["nodes", "tasks"], "leaves");
            rename(map, &["edges"], "edges");
            rename(map, &["source"], "from");
            rename(map, &["target"], "to");
            rename(map, &["type", "label"], "kind");
            rename(
                map,
                &["output_artifact", "outputs", "output_refs"],
                "expected_outputs",
            );
            rename(map, &["id", "node_id"], "leaf_id");
            if map.contains_key("leaf_id") {
                default_string(map, "kind", "task");
                let leaf_id = map
                    .get("leaf_id")
                    .and_then(Value::as_str)
                    .unwrap_or("task")
                    .to_string();
                default_string(map, "title", &leaf_id);
                default_string(map, "instructions", &leaf_id);
                default_string(map, "task_ref", &leaf_id);
                map.entry("expected_outputs")
                    .or_insert_with(|| Value::Array(Vec::new()));
                map.entry("inputs")
                    .or_insert_with(|| Value::Array(Vec::new()));
            }
            for value in map.values_mut() {
                normalize_graph_aliases(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_graph_aliases(value);
            }
        }
        _ => {}
    }
}

fn rename(map: &mut Map<String, Value>, aliases: &[&str], canonical: &str) {
    if map.contains_key(canonical) {
        return;
    }
    for alias in aliases {
        if let Some(value) = map.remove(*alias) {
            map.insert(canonical.to_string(), value);
            return;
        }
    }
}

fn default_string(map: &mut Map<String, Value>, key: &str, value: &str) {
    map.entry(key.to_string())
        .or_insert_with(|| Value::String(value.to_string()));
}

fn code_symbol(symbol: &RustSymbol) -> CodeSymbol {
    CodeSymbol {
        name: symbol.name.clone(),
        kind: serde_json::to_value(&symbol.kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".into()),
        range: symbol.range.clone(),
        visibility: serde_json::to_value(&symbol.visibility)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".into()),
    }
}

fn is_generated_path(path: &str) -> bool {
    path.contains("/target/") || path.contains("/generated/") || path.ends_with(".generated.rs")
}

fn is_vendor_path(path: &str) -> bool {
    path.contains("/vendor/") || path.contains("/.cargo/registry/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_model_output_fails_closed_without_allowlists() {
        let parsed = parse_strict_normalized_model_output("Tool.Run(arg={})", &[], &["arg"]);
        assert!(!parsed.ok);
    }

    #[test]
    fn strict_model_output_matches_graph_composer_shape() {
        let parsed = parse_strict_normalized_model_output(
            "Tool.Run(arg={\"ok\":true})",
            &["Tool.Run"],
            &["arg"],
        );
        assert!(parsed.ok);
        assert_eq!(parsed.grammar, "python_style_command");
        assert_eq!(parsed.value.unwrap()["ok"], true);
    }

    #[test]
    fn work_graph_repair_normalizes_aliases_and_defaults() {
        let value = serde_json::json!({"graph":{"nodes":[{"id":"a"}],"edges":[{"source":"a","target":"b","type":"depends"}]}});
        let repaired = normalize_work_graph_value(value);
        assert!(repaired.get("leaves").is_some());
        assert_eq!(repaired["leaves"][0]["leaf_id"], "a");
        assert_eq!(repaired["leaves"][0]["kind"], "task");
        assert_eq!(repaired["edges"][0]["from"], "a");
    }

    #[test]
    fn code_facts_projection_contains_graph_composer_fields() {
        let facts = rust_code_facts(
            "src/lib.rs",
            "use std::fmt;\n#[test]\nfn it_works() {}\npub struct Thing;",
            1024,
        );
        assert_eq!(facts.language, "rust");
        assert!(
            facts
                .imports
                .iter()
                .any(|import| import.contains("std::fmt"))
        );
        assert!(facts.tests.contains(&"it_works".to_string()));
        assert!(facts.symbols.iter().any(|symbol| symbol.name == "Thing"));
    }
}
