//! Deterministic canonical examples generated from the registered schemas.

use super::{CanonicalExample, CanonicalExampleManifest, schema_catalog, schema_json};
use crate::core::{SchemaVersion, canonical_json_sha256};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn canonical_example_json(name: &str) -> Option<Value> {
    let descriptor = super::schema_descriptor(name)?;
    let schema = schema_json(name)?;
    let mut active_refs = BTreeSet::new();
    let mut value = example_from_schema(&schema, &schema, &mut active_refs);
    if let Some(object) = value.as_object_mut()
        && object.contains_key("schema_version")
    {
        object.insert(
            "schema_version".into(),
            Value::String(descriptor.schema_version),
        );
    }
    Some(value)
}

pub fn canonical_examples() -> Result<CanonicalExampleManifest, serde_json::Error> {
    let examples = schema_catalog()
        .schemas
        .into_iter()
        .filter_map(|descriptor| {
            let value = canonical_example_json(&descriptor.name)?;
            let canonical_sha256 = canonical_json_sha256(&value).ok()?;
            let name = descriptor.name;
            Some((
                name.clone(),
                CanonicalExample {
                    schema_name: name,
                    schema_version: descriptor.schema_version,
                    canonical_sha256,
                    value,
                },
            ))
        })
        .collect::<BTreeMap<_, _>>();
    Ok(CanonicalExampleManifest {
        schema_version: SchemaVersion::CANONICAL_EXAMPLES_V1.to_string(),
        examples,
    })
}

fn example_from_schema(schema: &Value, root: &Value, active_refs: &mut BTreeSet<String>) -> Value {
    let Some(object) = schema.as_object() else {
        return Value::Null;
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        if !active_refs.insert(reference.to_string()) {
            return Value::Null;
        }
        let value = reference
            .strip_prefix('#')
            .and_then(|pointer| root.pointer(pointer))
            .map(|resolved| example_from_schema(resolved, root, active_refs))
            .unwrap_or(Value::Null);
        active_refs.remove(reference);
        return value;
    }
    if let Some(value) = object.get("const") {
        return value.clone();
    }
    if let Some(value) = object.get("default") {
        return value.clone();
    }
    if let Some(value) = object
        .get("examples")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return value.clone();
    }
    if let Some(value) = object
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return value.clone();
    }
    if !object.contains_key("properties") {
        for keyword in ["oneOf", "anyOf"] {
            if let Some(branch) = object
                .get(keyword)
                .and_then(Value::as_array)
                .and_then(|branches| branches.first())
            {
                return example_from_schema(branch, root, active_refs);
            }
        }
    }
    if let Some(branches) = object.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        for branch in branches {
            if let Value::Object(value) = example_from_schema(branch, root, active_refs) {
                merged.extend(value);
            }
        }
        return Value::Object(merged);
    }
    match schema_type(object.get("type")) {
        Some("object") => object_example(object, root, active_refs),
        None if object.contains_key("properties") => object_example(object, root, active_refs),
        Some("array") => {
            let count = object.get("minItems").and_then(Value::as_u64).unwrap_or(0);
            let item = object.get("items").unwrap_or(&Value::Null);
            Value::Array(
                (0..count)
                    .map(|_| example_from_schema(item, root, active_refs))
                    .collect(),
            )
        }
        Some("string") => Value::String(example_string(object)),
        Some("integer") => Value::Number(Number::from(
            object.get("minimum").and_then(Value::as_i64).unwrap_or(0),
        )),
        Some("number") => {
            Number::from_f64(object.get("minimum").and_then(Value::as_f64).unwrap_or(0.0))
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        Some("boolean") => Value::Bool(false),
        Some("null") => Value::Null,
        _ => Value::Null,
    }
}

fn object_example(
    object: &Map<String, Value>,
    root: &Value,
    active_refs: &mut BTreeSet<String>,
) -> Value {
    let mut properties = object
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut required = object
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branch) = object
            .get(keyword)
            .and_then(Value::as_array)
            .and_then(|branches| branches.first())
        {
            if let Some(branch_properties) = branch.get("properties").and_then(Value::as_object) {
                properties.extend(branch_properties.clone());
            }
            required.extend(
                branch
                    .get("required")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str),
            );
        }
    }
    let mut value = Map::new();
    for name in required {
        if let Some(property) = properties.get(name) {
            value.insert(
                name.to_string(),
                example_from_schema(property, root, active_refs),
            );
        }
    }
    Value::Object(value)
}

fn schema_type(value: Option<&Value>) -> Option<&str> {
    match value {
        Some(Value::String(value)) => Some(value),
        Some(Value::Array(values)) => values.iter().find_map(Value::as_str),
        _ => None,
    }
}

fn example_string(schema: &Map<String, Value>) -> String {
    let minimum = schema.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
    let base = match schema.get("format").and_then(Value::as_str) {
        Some("uri") | Some("uri-reference") => "https://example.invalid/",
        Some("date-time") => "2000-01-01T00:00:00Z",
        Some("date") => "2000-01-01",
        Some("time") => "00:00:00Z",
        Some("uuid") => "00000000-0000-0000-0000-000000000000",
        _ => "",
    };
    if base.len() >= minimum {
        base.to_string()
    } else {
        "x".repeat(minimum)
    }
}
