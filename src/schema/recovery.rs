//! Lossless fallback for values containing enum members unknown to this build.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnknownEnumValue {
    pub instance_path: String,
    pub schema_path: String,
    pub value: Value,
    pub known_values: Vec<Value>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnknownEnumDocument {
    pub value: Value,
    pub unknown_values: Vec<UnknownEnumValue>,
}

/// Typed decoding when possible, otherwise an explicit lossless unknown-enum document.
#[derive(Debug, Clone, PartialEq)]
pub enum ForwardCompatible<T> {
    Known(T),
    UnknownEnums(UnknownEnumDocument),
}

pub fn deserialize_forward_compatible<T: DeserializeOwned>(
    value: Value,
    schema: &Value,
) -> Result<ForwardCompatible<T>, ForwardCompatibilityError> {
    match serde_json::from_value(value.clone()) {
        Ok(typed) => Ok(ForwardCompatible::Known(typed)),
        Err(source) => {
            let unknown_values = find_unknown_enum_values(&value, schema);
            if unknown_values.is_empty() {
                Err(ForwardCompatibilityError::Deserialize(source.to_string()))
            } else {
                Ok(ForwardCompatible::UnknownEnums(UnknownEnumDocument {
                    value,
                    unknown_values,
                }))
            }
        }
    }
}

pub fn find_unknown_enum_values(instance: &Value, schema: &Value) -> Vec<UnknownEnumValue> {
    let mut unknown = Vec::new();
    inspect(instance, schema, schema, "", "#", &mut unknown);
    unknown.sort_by(|left, right| {
        (&left.instance_path, &left.schema_path).cmp(&(&right.instance_path, &right.schema_path))
    });
    unknown.dedup();
    unknown
}

fn inspect(
    instance: &Value,
    schema: &Value,
    root: &Value,
    instance_path: &str,
    schema_path: &str,
    unknown: &mut Vec<UnknownEnumValue>,
) {
    let Some(schema) = resolve(schema, root) else {
        return;
    };
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(values) = object.get("enum").and_then(Value::as_array)
        && !values.contains(instance)
    {
        unknown.push(UnknownEnumValue {
            instance_path: instance_path.to_string(),
            schema_path: format!("{schema_path}/enum"),
            value: instance.clone(),
            known_values: values.clone(),
        });
        return;
    }
    if let Some(expected) = object.get("const")
        && expected != instance
    {
        unknown.push(UnknownEnumValue {
            instance_path: instance_path.to_string(),
            schema_path: format!("{schema_path}/const"),
            value: instance.clone(),
            known_values: vec![expected.clone()],
        });
        return;
    }

    if let (Some(instance), Some(properties)) = (
        instance.as_object(),
        object.get("properties").and_then(Value::as_object),
    ) {
        for (name, value) in instance {
            if let Some(property_schema) = properties.get(name) {
                inspect(
                    value,
                    property_schema,
                    root,
                    &pointer_child(instance_path, name),
                    &format!("{schema_path}/properties/{}", escape(name)),
                    unknown,
                );
            }
        }
    }
    if let (Some(instance), Some(item_schema)) = (instance.as_array(), object.get("items")) {
        for (index, value) in instance.iter().enumerate() {
            inspect(
                value,
                item_schema,
                root,
                &format!("{instance_path}/{index}"),
                &format!("{schema_path}/items"),
                unknown,
            );
        }
    }
    if let Some(all_of) = object.get("allOf").and_then(Value::as_array) {
        for (index, branch) in all_of.iter().enumerate() {
            inspect(
                instance,
                branch,
                root,
                instance_path,
                &format!("{schema_path}/allOf/{index}"),
                unknown,
            );
        }
    }
    for keyword in ["oneOf", "anyOf"] {
        let Some(branches) = object.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        if let Some(known_values) = scalar_variant_values(branches, root)
            && !known_values.contains(instance)
            && !known_values.is_empty()
        {
            unknown.push(UnknownEnumValue {
                instance_path: instance_path.to_string(),
                schema_path: format!("{schema_path}/{keyword}"),
                value: instance.clone(),
                known_values,
            });
            continue;
        }
        if let Some((index, branch)) = branches
            .iter()
            .enumerate()
            .find(|(_, branch)| shape_matches(instance, branch, root))
        {
            inspect(
                instance,
                branch,
                root,
                instance_path,
                &format!("{schema_path}/{keyword}/{index}"),
                unknown,
            );
        }
    }
}

fn scalar_variant_values(branches: &[Value], root: &Value) -> Option<Vec<Value>> {
    let mut values = Vec::new();
    for branch in branches {
        let branch = resolve(branch, root)?;
        let object = branch.as_object()?;
        if let Some(value) = object.get("const") {
            values.push(value.clone());
        } else if let Some(variants) = object.get("enum").and_then(Value::as_array) {
            values.extend(variants.iter().cloned());
        } else if object.get("type").and_then(Value::as_str) == Some("null") {
            values.push(Value::Null);
        } else {
            return None;
        }
    }
    Some(values)
}

fn shape_matches(instance: &Value, schema: &Value, root: &Value) -> bool {
    let Some(schema) = resolve(schema, root).and_then(Value::as_object) else {
        return false;
    };
    if let Some(expected) = schema.get("const") {
        return expected == instance;
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let Some(instance) = instance.as_object() else {
                return false;
            };
            schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .all(|name| instance.contains_key(name))
        }
        Some("array") => instance.is_array(),
        Some("string") => instance.is_string(),
        Some("number") => instance.is_number(),
        Some("integer") => instance.as_i64().is_some() || instance.as_u64().is_some(),
        Some("boolean") => instance.is_boolean(),
        Some("null") => instance.is_null(),
        _ => true,
    }
}

fn resolve<'a>(schema: &'a Value, root: &'a Value) -> Option<&'a Value> {
    let reference = schema.get("$ref").and_then(Value::as_str);
    let Some(reference) = reference else {
        return Some(schema);
    };
    let pointer = reference.strip_prefix('#')?;
    root.pointer(pointer)
}

fn pointer_child(path: &str, child: &str) -> String {
    format!("{path}/{}", escape(child))
}

fn escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ForwardCompatibilityError {
    #[error("value could not be decoded and no unknown enum member explained the failure: {0}")]
    Deserialize(String),
}
