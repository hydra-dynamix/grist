//! Shared normalization and canonicalization for expected parser outputs.

use super::ExpectedNormalization;
use crate::core::{canonical_json_bytes, sha256_hex};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExpectedOutputError {
    #[error("expected-output normalization rules are empty or mix exact with transformations")]
    InvalidRules,
    #[error("expected-output JSON pointer does not exist: {0}")]
    PointerMissing(String),
    #[error("expected-output rule cannot remove the JSON root")]
    CannotRemoveRoot,
    #[error("fixture-root replacement requires a string at JSON pointer {0}")]
    PathValueNotString(String),
    #[error("fixture root is empty")]
    EmptyFixtureRoot,
    #[error("expected-output canonicalization failed: {0}")]
    Canonicalization(String),
}

/// Apply only the transformations explicitly listed by one expected output.
pub fn normalize_expected_value(
    mut value: Value,
    rules: &[ExpectedNormalization],
    fixture_root: impl AsRef<Path>,
) -> Result<Value, ExpectedOutputError> {
    if rules.is_empty() || (rules.len() > 1 && rules.contains(&ExpectedNormalization::Exact)) {
        return Err(ExpectedOutputError::InvalidRules);
    }
    for rule in rules {
        match rule {
            ExpectedNormalization::Exact => {}
            ExpectedNormalization::RemoveCallerTimestamp { json_pointer } => {
                remove_pointer(&mut value, json_pointer)?;
            }
            ExpectedNormalization::ReplaceFixtureRoot { json_pointer } => {
                replace_fixture_root(&mut value, json_pointer, fixture_root.as_ref())?;
            }
        }
    }
    Ok(value)
}

/// Grist canonical JSON v1 followed by exactly one LF, ready for a golden file.
pub fn canonical_expected_bytes(
    value: Value,
    rules: &[ExpectedNormalization],
    fixture_root: impl AsRef<Path>,
) -> Result<Vec<u8>, ExpectedOutputError> {
    let normalized = normalize_expected_value(value, rules, fixture_root)?;
    let mut bytes = canonical_json_bytes(&normalized)
        .map_err(|error| ExpectedOutputError::Canonicalization(error.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Hash canonical JSON without the golden file's terminal LF.
pub fn canonical_expected_sha256(
    value: Value,
    rules: &[ExpectedNormalization],
    fixture_root: impl AsRef<Path>,
) -> Result<String, ExpectedOutputError> {
    let mut bytes = canonical_expected_bytes(value, rules, fixture_root)?;
    bytes.pop();
    Ok(sha256_hex(&bytes))
}

fn remove_pointer(value: &mut Value, pointer: &str) -> Result<(), ExpectedOutputError> {
    let (parent_pointer, token) = pointer
        .rsplit_once('/')
        .ok_or(ExpectedOutputError::CannotRemoveRoot)?;
    if token.is_empty() {
        return Err(ExpectedOutputError::PointerMissing(pointer.into()));
    }
    let token = decode_pointer_token(token)
        .ok_or_else(|| ExpectedOutputError::PointerMissing(pointer.into()))?;
    let parent = if parent_pointer.is_empty() {
        value
    } else {
        value
            .pointer_mut(parent_pointer)
            .ok_or_else(|| ExpectedOutputError::PointerMissing(pointer.into()))?
    };
    let removed = match parent {
        Value::Object(object) => object.remove(&token).is_some(),
        Value::Array(array) => token
            .parse::<usize>()
            .ok()
            .filter(|index| *index < array.len())
            .map(|index| {
                array.remove(index);
            })
            .is_some(),
        _ => false,
    };
    if removed {
        Ok(())
    } else {
        Err(ExpectedOutputError::PointerMissing(pointer.into()))
    }
}

fn replace_fixture_root(
    value: &mut Value,
    pointer: &str,
    fixture_root: &Path,
) -> Result<(), ExpectedOutputError> {
    let root = fixture_root.to_string_lossy();
    if root.is_empty() {
        return Err(ExpectedOutputError::EmptyFixtureRoot);
    }
    let target = value
        .pointer_mut(pointer)
        .ok_or_else(|| ExpectedOutputError::PointerMissing(pointer.into()))?;
    let Value::String(target) = target else {
        return Err(ExpectedOutputError::PathValueNotString(pointer.into()));
    };
    let slash_root = root.replace('\\', "/");
    *target = target
        .replace(root.as_ref(), "$FIXTURE_ROOT")
        .replace(&slash_root, "$FIXTURE_ROOT");
    Ok(())
}

fn decode_pointer_token(token: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut chars = token.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match chars.next()? {
            '0' => decoded.push('~'),
            '1' => decoded.push('/'),
            _ => return None,
        }
    }
    Some(decoded)
}
