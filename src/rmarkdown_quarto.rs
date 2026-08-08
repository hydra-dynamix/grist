//! R Markdown and Quarto are inert dialects of the Markdown v2 payload.

use crate::core::SourceInfo;
pub use crate::markdown::{
    ExecutableBlockMetadata, Frontmatter, FrontmatterKind, LocalReferenceKind,
    LocalReferenceStatus, MarkdownCitation, MarkdownDialect, MarkdownDocument, MarkdownEnvelope,
    MarkdownFigure, MarkdownLocalReference, MarkdownNode, MarkdownNodeKind, MarkdownOptions,
    MarkdownStoredOutput,
};

pub fn parse_r_markdown(text: &str, source: SourceInfo) -> MarkdownEnvelope {
    parse_r_markdown_with_options(text, source, &MarkdownOptions::default())
}

pub fn parse_r_markdown_with_options(
    text: &str,
    source: SourceInfo,
    options: &MarkdownOptions,
) -> MarkdownEnvelope {
    let mut options = options.clone();
    options.dialect = Some(MarkdownDialect::RMarkdown);
    crate::markdown::parse_markdown_with_options(text, source, &options)
}

pub fn parse_r_markdown_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &MarkdownOptions,
) -> MarkdownEnvelope {
    let mut options = options.clone();
    options.dialect = Some(MarkdownDialect::RMarkdown);
    crate::markdown::parse_markdown_bytes(bytes, source, &options)
}

pub fn parse_quarto(text: &str, source: SourceInfo) -> MarkdownEnvelope {
    parse_quarto_with_options(text, source, &MarkdownOptions::default())
}

pub fn parse_quarto_with_options(
    text: &str,
    source: SourceInfo,
    options: &MarkdownOptions,
) -> MarkdownEnvelope {
    let mut options = options.clone();
    options.dialect = Some(MarkdownDialect::Quarto);
    crate::markdown::parse_markdown_with_options(text, source, &options)
}

pub fn parse_quarto_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &MarkdownOptions,
) -> MarkdownEnvelope {
    let mut options = options.clone();
    options.dialect = Some(MarkdownDialect::Quarto);
    crate::markdown::parse_markdown_bytes(bytes, source, &options)
}
