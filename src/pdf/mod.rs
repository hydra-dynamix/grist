//! Bounded, inert PDF container and document-structure parser.
//!
//! This module reads PDF syntax and metadata only. It never evaluates document
//! actions, JavaScript, forms, launch targets, or external references.

mod filters;
mod interactive;
mod layout;
mod model;
mod ocr;
mod parse;
mod semantic;
mod syntax;

pub use model::*;
pub use parse::parse_pdf_bytes;

pub(crate) use parse::{parse_registered, parser_info};
