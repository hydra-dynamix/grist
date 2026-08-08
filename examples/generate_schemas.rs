//! Regenerate every checked public JSON Schema and canonical example manifest.

use grist::schema::{canonical_examples, schema_catalog, schema_json};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for descriptor in schema_catalog().schemas {
        let value = schema_json(&descriptor.name)
            .ok_or_else(|| format!("schema {} has no generator", descriptor.name))?;
        let mut json = serde_json::to_string_pretty(&value)?;
        json.push('\n');
        std::fs::write(root.join("schemas").join(descriptor.file_name), json)?;
    }
    let mut examples = serde_json::to_string_pretty(&canonical_examples()?)?;
    examples.push('\n');
    std::fs::write(
        root.join("examples/schema-canonical-examples.v1.json"),
        examples,
    )?;
    Ok(())
}
