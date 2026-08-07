//! Authoritative format parser namespaces.
//!
//! New code should use these paths. The historical crate-root modules remain
//! source-compatible aliases during the 0.x compatibility window.

#[cfg(feature = "asciidoc")]
pub mod asciidoc;
#[cfg(feature = "bibliography")]
pub mod bibliography;
#[cfg(feature = "csv")]
pub mod delimited;
#[cfg(feature = "epub")]
pub mod epub;
#[cfg(feature = "html")]
pub mod html;
#[cfg(feature = "latex")]
pub mod latex;
#[cfg(feature = "ldgr-projection")]
pub mod ldgr_projection;
#[cfg(feature = "markdown")]
pub mod markdown;
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
#[cfg(feature = "serialization")]
pub mod structured_text;
pub mod text;
#[cfg(feature = "typescript")]
pub mod typescript;
#[cfg(feature = "word-ooxml")]
pub mod word_ooxml;
#[cfg(feature = "xml")]
pub mod xml;
