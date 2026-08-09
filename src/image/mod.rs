//! Bounded, inert native parsing for raster images, camera metadata, and SVG.
//!
//! The parser inventories SVG links, scripts, event handlers, and active elements.
//! It never evaluates script, dereferences links, decodes pixels, or performs network I/O.

mod graph;
pub(crate) mod metadata;
mod model;
mod ocr;
mod ocr_graph;
mod parse;
mod svg;

pub use model::*;

use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Diagnostic, Envelope, Hashes, OperationControl,
    OperationKind, OperationStatus, ParserInfo, SchemaVersion, SourceInfo,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};

const PARSER: &str = "grist.image";
pub type ImageEnvelope = Envelope<ImageDocument>;

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation(
            "grist-safe-native-image-metadata",
            env!("CARGO_PKG_VERSION"),
        )
        .with_specification_version(
            "PNG/APNG; JPEG; TIFF 6; WebP; GIF89a; BMP; ISO BMFF HEIF; SVG 2 structural subset",
        )
        .with_feature("media")
}

pub fn parse_image_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &ImageOptions,
) -> ImageEnvelope {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .expect("trusted budget is valid");
    parse_image_with_operation_control(bytes, source, options, &control)
}

/// Parse an image while sharing the caller's cancellation token and resource budgets.
pub fn parse_image_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &ImageOptions,
    control: &OperationControl,
) -> ImageEnvelope {
    let digest = crate::core::options_digest(options).expect("image options serialize");
    let result = control
        .budget()
        .consume_input_bytes(bytes.len() as u64)
        .map_err(crate::core::OperationControlError::from)
        .and_then(|_| {
            control
                .budget()
                .observe_memory_bytes(bytes.len() as u64)
                .map_err(crate::core::OperationControlError::from)
        })
        .and_then(|_| control.checkpoint());
    if let Err(error) = result {
        return Envelope::without_payload(
            OperationKind::Parse,
            ArtifactKind::Image,
            error.operation_status(0),
            source,
            parser_info(),
            digest,
            SchemaVersion::IMAGE_V1,
        )
        .expect("failed image envelope is valid")
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_diagnostics(vec![error.diagnostic(PARSER)]);
    }
    match parse::parse_document_controlled(bytes, options, Some(control)) {
        Ok(document) => Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Image,
            source,
            parser_info(),
            digest,
            SchemaVersion::IMAGE_V1,
            document,
        )
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_canonical_payload_identity()
        .expect("image payload serializes"),
        Err(parse::ImageParseError::Malformed(message)) => Envelope::without_payload(
            OperationKind::Parse,
            ArtifactKind::Image,
            OperationStatus::Failed,
            source,
            parser_info(),
            digest,
            SchemaVersion::IMAGE_V1,
        )
        .expect("failed image envelope is valid")
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_diagnostics(vec![Diagnostic::malformed(PARSER, message)]),
        Err(parse::ImageParseError::Control(error)) => Envelope::without_payload(
            OperationKind::Parse,
            ArtifactKind::Image,
            error.operation_status(0),
            source,
            parser_info(),
            digest,
            SchemaVersion::IMAGE_V1,
        )
        .expect("failed image envelope is valid")
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_diagnostics(vec![error.diagnostic(PARSER)]),
    }
}

pub(crate) fn parse_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    if let Err(error) = context.control().checkpoint() {
        return Ok(ParserOutput::terminal(
            error.operation_status(0),
            vec![error.diagnostic(PARSER)],
        ));
    }
    let options: ImageOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(PARSER, error.to_string())))?;
    let mut document = match parse::parse_document_controlled(
        context.bytes(),
        &options,
        Some(context.control()),
    ) {
        Ok(document) => document,
        Err(parse::ImageParseError::Malformed(message)) => {
            return Err(Box::new(Diagnostic::malformed(PARSER, message)));
        }
        Err(parse::ImageParseError::Control(error)) => {
            return Ok(ParserOutput::terminal(
                error.operation_status(0),
                vec![error.diagnostic(PARSER)],
            ));
        }
    };
    if let Err(error) = context.control().checkpoint() {
        return Ok(ParserOutput::terminal(
            error.operation_status(0),
            vec![error.diagnostic(PARSER)],
        ));
    }
    if document.format.as_str() != context.format_id() {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            format!(
                "selected {} parser does not match {} input signature",
                context.format_id(),
                document.format.as_str()
            ),
        )));
    }
    let ocr = ocr::apply_selected_ocr(context, &mut document, &options);
    let payload = serde_json::to_value(document)
        .map_err(|error| Box::new(Diagnostic::parser_defect(PARSER, error.to_string())))?;
    let mut output = if ocr.diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        ParserOutput::partial(Some(payload), ocr.diagnostics)
    } else {
        let mut output = ParserOutput::complete(payload);
        output.diagnostics = ocr.diagnostics;
        output
    };
    output.providers = ocr.invocations;
    output.provenance = ocr.provenance;
    Ok(output)
}
