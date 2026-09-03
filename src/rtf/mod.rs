//! Bounded, inert Rich Text Format parsing.

mod graph;
mod lexer;
mod model;
mod semantics;

pub use model::*;

use crate::core::{Diagnostic, ParserInfo, SchemaVersion};
use crate::registry::{ParserContext, ParserError, ParserOutput};

pub(crate) const PARSER: &str = "grist.rtf";

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("grist byte-oriented RTF parser + encoding_rs", "1/0.8")
        .with_specification_version("Rich Text Format 1.9.1")
        .with_feature("rtf")
}

pub(crate) fn parse_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    let options: RtfOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(PARSER, error.to_string())))?;
    let bytes = context.bytes();
    let lexed = lexer::lex(bytes, context)?;
    let mut root = lexed.root;
    let mut diagnostics = lexed.diagnostics;
    let rtf_version = root
        .contents
        .iter()
        .find_map(|element| match element {
            RtfElement::Control { control } if control.name == "rtf" => control.parameter,
            _ => None,
        })
        .ok_or_else(|| {
            Box::new(Diagnostic::malformed(
                PARSER,
                "top-level RTF group has no numeric \\rtf version control",
            )) as ParserError
        })?;
    let semantic = semantics::interpret(&mut root, bytes, &options, &mut diagnostics)?;
    context.consume_decoded_characters(semantic.views.visible.chars().count() as u64)?;
    context.consume_child_artifacts(semantic.embedded_artifacts.len() as u64)?;
    let node_count = count_groups(&root)
        .saturating_add(semantic.paragraphs.len())
        .saturating_add(semantic.fields.len())
        .saturating_add(semantic.images.len())
        .saturating_add(semantic.objects.len())
        .saturating_add(semantic.revisions.len())
        .saturating_add(semantic.comments.len())
        .saturating_add(semantic.tables.len())
        .saturating_add(semantic.list_items.len());
    context.consume_nodes(node_count as u64)?;
    let document = RtfDocument {
        schema_version: SchemaVersion::RTF_V1.into(),
        rtf_version,
        charset: semantic.charset,
        ansi_code_page: semantic.ansi_code_page,
        default_font: semantic.default_font,
        generator: semantic.generator,
        metadata: semantic.metadata,
        root,
        destinations: semantic.destinations,
        fonts: semantic.fonts,
        colors: semantic.colors,
        styles: semantic.styles,
        lists: semantic.lists,
        list_overrides: semantic.list_overrides,
        paragraphs: semantic.paragraphs,
        list_items: semantic.list_items,
        tables: semantic.tables,
        fields: semantic.fields,
        images: semantic.images,
        objects: semantic.objects,
        revisions: semantic.revisions,
        comments: semantic.comments,
        unknown_controls: semantic.unknown_controls,
        embedded_artifacts: semantic.embedded_artifacts,
        views: semantic.views,
        raw_source_bytes: options.retain_raw_source_bytes.then(|| bytes.to_vec()),
    };
    let payload = serde_json::to_value(document).map_err(|error| {
        Box::new(Diagnostic::parser_defect(PARSER, error.to_string())) as ParserError
    })?;
    if diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        Ok(ParserOutput::partial(Some(payload), diagnostics))
    } else {
        let mut output = ParserOutput::complete(payload);
        output.diagnostics = diagnostics;
        Ok(output)
    }
}

fn count_groups(group: &RtfGroup) -> usize {
    1usize.saturating_add(
        group
            .contents
            .iter()
            .map(|element| match element {
                RtfElement::Group { group } => count_groups(group),
                _ => 1,
            })
            .sum::<usize>(),
    )
}
