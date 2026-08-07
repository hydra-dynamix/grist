//! Grist interpretation utility library.

#[cfg(feature = "basin")]
pub mod basin;
pub mod capabilities;
#[cfg(feature = "cli")]
pub mod cli;
#[cfg(all(feature = "model-output", feature = "rust"))]
pub mod compatibility;
pub mod container;
pub mod core;
#[cfg(feature = "csv")]
pub mod csv;
pub mod decode;
pub mod detect;
pub mod document_graph;
#[cfg(feature = "epub")]
pub mod epub;
pub mod fixtures;
pub mod formats;
pub mod ingest;
pub mod promotion;
pub mod provider;
pub mod registry;
pub mod render;
pub mod runtime;
pub mod segment;
pub mod text;
pub mod transform;

#[cfg(feature = "asciidoc")]
pub mod asciidoc;
#[cfg(feature = "bibliography")]
pub mod bibliography;
#[cfg(feature = "html")]
pub mod html;
#[cfg(feature = "xml")]
pub mod jats;
#[cfg(feature = "latex")]
pub mod latex;
#[cfg(feature = "ldgr-projection")]
pub mod ldgr_projection;
#[cfg(feature = "markdown")]
pub mod markdown;
#[cfg(feature = "model-output")]
pub mod model_output;
#[cfg(feature = "odf-word")]
pub mod odf_word;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "presentation-odf")]
pub mod presentation_odf;
#[cfg(feature = "presentation-ooxml")]
pub mod presentation_ooxml;
#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "restructured-text")]
pub mod restructured_text;
#[cfg(feature = "rtf")]
pub mod rtf;
#[cfg(feature = "rust")]
pub mod rust;
pub mod schema;
pub mod security;
#[cfg(feature = "serialization")]
pub mod serialization;
pub mod summary;
#[cfg(feature = "typescript")]
pub mod typescript;
#[cfg(feature = "word-ooxml")]
pub mod word_ooxml;
#[cfg(feature = "xml")]
pub mod xml;

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
