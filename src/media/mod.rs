//! Bounded, inert audio/video container metadata and embedded-track inspection.
//!
//! Codec payloads are never decoded or executed. Only container headers, passive metadata,
//! and explicitly bounded subtitle/artwork children are inspected.

mod graph;
mod model;
mod parse;
mod parse_audio;
mod parse_ebml;
mod parse_iso;
mod transcription;
mod transcription_graph;

pub use model::*;

use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Diagnostic, Envelope, Hashes, OperationControl,
    OperationKind, OperationStatus, ParserInfo, SchemaVersion, SourceInfo,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};

pub type MediaEnvelope = Envelope<MediaDocument>;

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(parse::PARSER)
        .with_implementation(
            "grist-safe-native-media-metadata",
            env!("CARGO_PKG_VERSION"),
        )
        .with_specification_version("ID3v2/MP3; RIFF WAVE; FLAC; ISO BMFF/QuickTime; Matroska/WebM")
        .with_feature("media")
}

pub fn parse_media_bytes(
    bytes: &[u8],
    source: SourceInfo,
    format: MediaFormat,
    options: &MediaOptions,
) -> MediaEnvelope {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .expect("trusted budget is valid");
    parse_media_with_operation_control(bytes, source, format, options, &control)
}

pub fn parse_media_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    format: MediaFormat,
    options: &MediaOptions,
    control: &OperationControl,
) -> MediaEnvelope {
    envelope_from_result(
        bytes,
        source,
        format,
        options,
        parse::parse_document(bytes, format, options, control, true),
    )
}

fn envelope_from_result(
    bytes: &[u8],
    source: SourceInfo,
    _format: MediaFormat,
    options: &MediaOptions,
    result: Result<MediaDocument, parse::MediaParseError>,
) -> MediaEnvelope {
    let digest = crate::core::options_digest(options).expect("media options serialize");
    match result {
        Ok(document) => {
            let diagnostics = document.diagnostics.clone();
            let mut envelope = if document.complete {
                Envelope::complete(
                    OperationKind::Parse,
                    ArtifactKind::Media,
                    source,
                    parser_info(),
                    digest,
                    SchemaVersion::MEDIA_V1,
                    document,
                )
            } else {
                Envelope::partial(
                    OperationKind::Parse,
                    ArtifactKind::Media,
                    source,
                    parser_info(),
                    digest,
                    SchemaVersion::MEDIA_V1,
                    Some(document),
                )
            };
            envelope.diagnostics = diagnostics;
            envelope
                .with_hashes(Hashes::for_bytes(bytes, None))
                .with_canonical_payload_identity()
                .expect("media payload serializes")
        }
        Err(error) => {
            let (status, diagnostic) = match error {
                parse::MediaParseError::Malformed(message) => (
                    OperationStatus::Failed,
                    Diagnostic::malformed(parse::PARSER, message),
                ),
                parse::MediaParseError::Control(error) => {
                    (error.operation_status(0), error.diagnostic(parse::PARSER))
                }
            };
            Envelope::without_payload(
                OperationKind::Parse,
                ArtifactKind::Media,
                status,
                source,
                parser_info(),
                digest,
                SchemaVersion::MEDIA_V1,
            )
            .expect("terminal media envelope is valid")
            .with_hashes(Hashes::for_bytes(bytes, None))
            .with_diagnostics(vec![diagnostic])
        }
    }
}

pub(crate) fn parse_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    let options: MediaOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(parse::PARSER, error.to_string())))?;
    let format = match context.format_id() {
        "mp3" => MediaFormat::Mp3,
        "mp4" => MediaFormat::Mp4,
        "quicktime" => MediaFormat::QuickTime,
        "wav" => MediaFormat::Wav,
        "flac" => MediaFormat::Flac,
        "matroska" => MediaFormat::Matroska,
        value => {
            return Err(Box::new(Diagnostic::parser_defect(
                parse::PARSER,
                format!("unknown registered media format {value}"),
            )));
        }
    };
    let document =
        match parse::parse_document(context.bytes(), format, &options, context.control(), false) {
            Ok(document) => document,
            Err(parse::MediaParseError::Malformed(message)) => {
                return Err(Box::new(Diagnostic::malformed(parse::PARSER, message)));
            }
            Err(parse::MediaParseError::Control(error)) => {
                return Ok(ParserOutput::terminal(
                    error.operation_status(0),
                    vec![error.diagnostic(parse::PARSER)],
                ));
            }
        };
    let mut document = document;
    let transcription =
        transcription::apply_selected_transcription(context, &mut document, &options);
    let mut diagnostics = document.diagnostics.clone();
    diagnostics.extend(transcription.diagnostics);
    let payload = serde_json::to_value(document)
        .map_err(|error| Box::new(Diagnostic::parser_defect(parse::PARSER, error.to_string())))?;
    let mut output = if diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        ParserOutput::partial(Some(payload), diagnostics)
    } else {
        let mut output = ParserOutput::complete(payload);
        output.diagnostics = diagnostics;
        output
    };
    output.providers = transcription.invocations;
    output.provenance = transcription.provenance;
    Ok(output)
}
