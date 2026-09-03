//! Grist interpretation utility library.
#[cfg(feature = "archives")]
pub mod archive;

#[cfg(feature = "basin")]
pub mod basin;
#[cfg(feature = "email-message")]
pub mod calendar_contact;
pub mod capabilities;
#[cfg(feature = "cli")]
pub mod cli;
pub mod code;
#[cfg(feature = "columnar")]
pub mod columnar;
#[cfg(all(feature = "model-output", feature = "rust"))]
pub mod compatibility;
pub mod container;
pub mod core;
#[cfg(feature = "csv")]
pub mod csv;
pub mod decode;
pub mod detect;
pub mod document_graph;
#[cfg(feature = "email-message")]
pub mod email;
#[cfg(feature = "epub")]
pub mod epub;
pub mod fixtures;
pub mod formats;
#[cfg(feature = "graph")]
pub mod graph;
pub mod ingest;
#[cfg(feature = "manifests")]
pub mod manifests;
#[cfg(feature = "email-message")]
pub mod mbox;
#[cfg(feature = "email-message")]
pub mod outlook;
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
#[cfg(feature = "media")]
pub mod image;
#[cfg(feature = "xml")]
pub mod jats;
#[cfg(feature = "javascript")]
pub mod javascript;
#[cfg(feature = "latex")]
pub mod latex;
#[cfg(feature = "ldgr-projection")]
pub mod ldgr_projection;
#[cfg(feature = "markdown")]
pub mod markdown;
#[cfg(feature = "media")]
pub mod media;
#[cfg(feature = "model-output")]
pub mod model_output;
#[cfg(feature = "notebooks")]
pub mod notebook;
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
#[cfg(feature = "notebooks")]
pub mod rmarkdown_quarto;
#[cfg(feature = "rtf")]
pub mod rtf;
#[cfg(feature = "rust")]
pub mod rust;
pub mod schema;
pub mod security;
#[cfg(feature = "serialization")]
pub mod serialization;
#[cfg(feature = "spreadsheet-odf")]
pub mod spreadsheet_odf;
#[cfg(feature = "spreadsheet-ooxml")]
pub mod spreadsheet_ooxml;
#[cfg(feature = "sqlite")]
pub mod sqlite;
#[cfg(feature = "structured-binary")]
pub mod structured_binary;
#[cfg(feature = "media")]
pub mod subtitle;
pub mod summary;
#[cfg(any(feature = "javascript", feature = "typescript"))]
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
        assert_eq!(version(), "0.2.0");
    }
}
