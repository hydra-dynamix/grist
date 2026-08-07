//! Regenerate or drift-check every registered public schema and canonical example.

use grist::schema::{canonical_examples, schema_catalog, schema_json};
use std::fs;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let check = std::env::args()
        .skip(1)
        .any(|argument| argument == "--check");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let schema_dir = root.join("schemas");
    let example_path = root.join("examples/schema-canonical-examples.v1.json");
    fs::create_dir_all(&schema_dir)?;

    for descriptor in schema_catalog().schemas {
        let schema = schema_json(&descriptor.name).ok_or_else(|| {
            format!(
                "schema {} was registered without a generator",
                descriptor.name
            )
        })?;
        let contents = pretty_json(&schema)?;
        update_or_check(&schema_dir.join(descriptor.file_name), &contents, check)?;
    }

    let examples = canonical_examples()?;
    update_or_check(&example_path, &pretty_json(&examples)?, check)?;
    Ok(())
}

fn pretty_json(value: &impl serde::Serialize) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(value).map(|mut json| {
        json.push('\n');
        json
    })
}

fn update_or_check(
    path: &Path,
    expected: &str,
    check: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if check {
        let actual = fs::read_to_string(path)
            .map_err(|error| format!("missing generated artifact {}: {error}", path.display()))?;
        if actual != expected {
            return Err(format!("generated artifact drift: {}", path.display()).into());
        }
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, expected)?;
    }
    Ok(())
}
