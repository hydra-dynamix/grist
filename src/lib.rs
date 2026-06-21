//! Grist interpretation utility library.

#[cfg(all(feature = "model-output", feature = "rust"))]
pub mod compatibility;
pub mod core;
#[cfg(feature = "csv")]
pub mod csv;
pub mod detect;
pub mod ingest;
pub mod text;

#[cfg(feature = "html")]
pub mod html;
#[cfg(feature = "ldgr-projection")]
pub mod ldgr_projection;
#[cfg(feature = "markdown")]
pub mod markdown;
#[cfg(feature = "model-output")]
pub mod model_output;
#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "rust")]
pub mod rust;
pub mod schema;
#[cfg(feature = "serialization")]
pub mod serialization;
#[cfg(feature = "typescript")]
pub mod typescript;

/// Returns the crate version from Cargo metadata.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_matches_manifest() {
        assert_eq!(version(), "0.1.0");
    }
}
