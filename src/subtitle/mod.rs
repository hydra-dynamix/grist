//! Loss-retaining, inert SRT, WebVTT, and TTML parsing.

mod graph;
mod model;
mod parse;

pub use model::*;

use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, ContentIdentity, Envelope, FormatIdentity,
    OperationControl, OperationKind, OperationStatus, ParserInfo, ProvenanceStep, SchemaVersion,
    SourceInfo,
};
use crate::decode::{DecodeContext, DecodeOptions, decode_text};

const PARSER: &str = "grist.subtitle";
pub type SubtitleEnvelope = Envelope<SubtitleDocument>;

pub fn parser_info(format: SubtitleFormat) -> ParserInfo {
    ParserInfo::new(format!("grist.{}", format.as_str()))
        .with_implementation("grist-native-subtitle", env!("CARGO_PKG_VERSION"))
        .with_feature("media")
        .with_specification_version(match format {
            SubtitleFormat::Srt => "SubRip",
            SubtitleFormat::WebVtt => "W3C WebVTT",
            SubtitleFormat::Ttml => "W3C TTML2 / IMSC structural subset",
        })
}

pub fn parse_srt(bytes: &[u8], source: SourceInfo, options: &SubtitleOptions) -> SubtitleEnvelope {
    parse_subtitle(bytes, source, SubtitleFormat::Srt, options)
}
pub fn parse_webvtt(
    bytes: &[u8],
    source: SourceInfo,
    options: &SubtitleOptions,
) -> SubtitleEnvelope {
    parse_subtitle(bytes, source, SubtitleFormat::WebVtt, options)
}
pub fn parse_ttml(bytes: &[u8], source: SourceInfo, options: &SubtitleOptions) -> SubtitleEnvelope {
    parse_subtitle(bytes, source, SubtitleFormat::Ttml, options)
}
pub fn parse_subtitle(
    bytes: &[u8],
    source: SourceInfo,
    format: SubtitleFormat,
    options: &SubtitleOptions,
) -> SubtitleEnvelope {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .expect("trusted budget is valid");
    parse_subtitle_with_operation_control(bytes, source, format, options, &control)
}

pub fn parse_subtitle_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    format: SubtitleFormat,
    options: &SubtitleOptions,
    control: &OperationControl,
) -> SubtitleEnvelope {
    let digest = crate::core::options_digest(options).expect("subtitle options serialize");
    if options.max_cues == 0
        || options.max_styles == 0
        || options.max_tracks == 0
        || options.max_regions == 0
        || options.max_nesting_depth == 0
    {
        return terminal(
            bytes,
            source,
            format,
            digest,
            OperationStatus::Failed,
            crate::core::Diagnostic::malformed(
                PARSER,
                "subtitle limits must all be greater than zero",
            ),
        );
    }
    if let Err(error) = control.budget().consume_input_bytes(bytes.len() as u64) {
        return terminal(
            bytes,
            source,
            format,
            digest,
            error.operation_status(0),
            error.diagnostic(PARSER),
        );
    }
    if let Err(error) = control.checkpoint() {
        return terminal(
            bytes,
            source,
            format,
            digest,
            error.operation_status(0),
            error.diagnostic(PARSER),
        );
    }
    let mut decode_options =
        DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some(format.as_str()));
    decode_options.context = if format == SubtitleFormat::Ttml {
        DecodeContext::Xml
    } else {
        DecodeContext::PlainText
    };
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    let decoded = match decode_text(bytes, &decode_options) {
        Ok(decoded) => decoded,
        Err(error) => {
            return terminal(
                bytes,
                source,
                format,
                digest,
                OperationStatus::Failed,
                error.diagnostic().with_parser(PARSER),
            );
        }
    };
    if let Err(error) = control
        .budget()
        .consume_decoded_characters(decoded.text.chars().count() as u64)
    {
        return terminal(
            bytes,
            source,
            format,
            digest,
            error.operation_status(0),
            error.diagnostic(PARSER),
        );
    }
    let document = match parse::parse_decoded(&decoded, format, options, control) {
        Ok(document) => document,
        Err(parse::SubtitleParseError::Control(error)) => {
            return terminal(
                bytes,
                source,
                format,
                digest,
                error.operation_status(0),
                error.diagnostic(PARSER),
            );
        }
    };
    let diagnostics = document.diagnostics.clone();
    let mut envelope = if document.complete {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Subtitle,
            source,
            parser_info(format),
            digest,
            SchemaVersion::SUBTITLE_V1,
            document,
        )
    } else {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Subtitle,
            source,
            parser_info(format),
            digest,
            SchemaVersion::SUBTITLE_V1,
            Some(document),
        )
    };
    envelope.diagnostics = diagnostics;
    envelope.provenance.push(decode_provenance(&decoded.report));
    envelope
        .with_identity(
            ContentIdentity::for_raw_bytes(bytes)
                .with_decoded(
                    &decoded.text,
                    decoded.report.encoding.label(),
                    decoded.report.is_lossy(),
                )
                .with_format(FormatIdentity::new(
                    format.as_str(),
                    Some(format.media_type()),
                )),
        )
        .with_canonical_payload_identity()
        .expect("subtitle payload serializes")
}

fn decode_provenance(report: &crate::decode::DecodeReport) -> ProvenanceStep {
    let loss = if report.is_lossy() {
        crate::core::DeclaredLoss::Lossy(crate::core::LossClass::from(
            crate::core::LossClass::REPAIR_APPLIED,
        ))
    } else {
        crate::core::DeclaredLoss::Lossless
    };
    ProvenanceStep::new(
        OperationKind::Parse,
        format!("grist.decode@{}", env!("CARGO_PKG_VERSION")),
        report.raw_identity.sha256.clone(),
        report.decoded_identity.sha256.clone(),
        crate::core::options_digest(&report.declarations).expect("decode declarations serialize"),
        loss,
    )
    .expect("decode provenance is valid")
}

fn terminal(
    bytes: &[u8],
    source: SourceInfo,
    format: SubtitleFormat,
    digest: String,
    status: OperationStatus,
    diagnostic: crate::core::Diagnostic,
) -> SubtitleEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Subtitle,
        status,
        source,
        parser_info(format),
        digest,
        SchemaVersion::SUBTITLE_V1,
    )
    .expect("terminal subtitle envelope is valid")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes).with_format(FormatIdentity::new(
            format.as_str(),
            Some(format.media_type()),
        )),
    )
    .with_diagnostics(vec![diagnostic])
}
