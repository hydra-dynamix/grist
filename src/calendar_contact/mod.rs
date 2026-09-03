//! Inert iCalendar and vCard parsing with record-level provenance.

mod graph;
mod model;
mod parser;

pub use model::*;

use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Diagnostic, Envelope, Hashes, OperationControl,
    OperationKind, OperationStatus, ParserInfo, SchemaVersion, SourceInfo,
};

const ICALENDAR_PARSER: &str = "grist.icalendar";
const VCARD_PARSER: &str = "grist.vcard";

pub type ICalendarEnvelope = Envelope<ICalendarDocument>;
pub type VCardEnvelope = Envelope<VCardDocument>;

pub fn icalendar_parser_info() -> ParserInfo {
    ParserInfo::new(ICALENDAR_PARSER)
        .with_implementation("grist-content-lines", env!("CARGO_PKG_VERSION"))
        .with_feature("email-message")
        .with_specification_version("RFC 5545; RFC 7986; RFC 6868")
}

pub fn vcard_parser_info() -> ParserInfo {
    ParserInfo::new(VCARD_PARSER)
        .with_implementation("grist-content-lines", env!("CARGO_PKG_VERSION"))
        .with_feature("email-message")
        .with_specification_version("vCard 2.1; RFC 2426; RFC 6350; RFC 6868")
}

pub fn parse_icalendar(
    bytes: &[u8],
    source: SourceInfo,
    options: &ICalendarOptions,
) -> ICalendarEnvelope {
    let control = trusted_control();
    parse_icalendar_with_operation_control(bytes, source, options, &control)
}

pub fn parse_icalendar_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &ICalendarOptions,
    control: &OperationControl,
) -> ICalendarEnvelope {
    let digest = crate::core::options_digest(options).expect("iCalendar options serialize");
    if let Some(envelope) = preflight::<ICalendarDocument>(
        bytes,
        &source,
        control,
        ArtifactKind::ICalendar,
        &digest,
        SchemaVersion::ICALENDAR_V1,
        icalendar_parser_info(),
    ) {
        return envelope;
    }
    match parser::parse_icalendar_document(bytes, &source, options, control) {
        Ok(document) => finish(
            bytes,
            source,
            digest,
            ArtifactKind::ICalendar,
            SchemaVersion::ICALENDAR_V1,
            icalendar_parser_info(),
            document.complete,
            document.diagnostics.clone(),
            document,
        ),
        Err(diagnostic) => terminal(
            bytes,
            source,
            digest,
            ArtifactKind::ICalendar,
            SchemaVersion::ICALENDAR_V1,
            icalendar_parser_info(),
            OperationStatus::Failed,
            *diagnostic,
        ),
    }
}

pub fn parse_vcard(bytes: &[u8], source: SourceInfo, options: &VCardOptions) -> VCardEnvelope {
    let control = trusted_control();
    parse_vcard_with_operation_control(bytes, source, options, &control)
}

pub fn parse_vcard_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &VCardOptions,
    control: &OperationControl,
) -> VCardEnvelope {
    let digest = crate::core::options_digest(options).expect("vCard options serialize");
    if let Some(envelope) = preflight::<VCardDocument>(
        bytes,
        &source,
        control,
        ArtifactKind::VCard,
        &digest,
        SchemaVersion::VCARD_V1,
        vcard_parser_info(),
    ) {
        return envelope;
    }
    match parser::parse_vcard_document(bytes, &source, options, control) {
        Ok(document) => finish(
            bytes,
            source,
            digest,
            ArtifactKind::VCard,
            SchemaVersion::VCARD_V1,
            vcard_parser_info(),
            document.complete,
            document.diagnostics.clone(),
            document,
        ),
        Err(diagnostic) => terminal(
            bytes,
            source,
            digest,
            ArtifactKind::VCard,
            SchemaVersion::VCARD_V1,
            vcard_parser_info(),
            OperationStatus::Failed,
            *diagnostic,
        ),
    }
}

fn trusted_control() -> OperationControl {
    OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .expect("trusted budget is valid")
}

#[allow(clippy::too_many_arguments)]
fn preflight<T>(
    bytes: &[u8],
    source: &SourceInfo,
    control: &OperationControl,
    kind: ArtifactKind,
    digest: &str,
    schema: &str,
    parser: ParserInfo,
) -> Option<Envelope<T>> {
    if let Err(error) = control.budget().consume_input_bytes(bytes.len() as u64) {
        let parser_name = parser_name_for_kind(&kind);
        return Some(terminal(
            bytes,
            source.clone(),
            digest.to_string(),
            kind.clone(),
            schema,
            parser.clone(),
            error.operation_status(0),
            error.diagnostic(parser_name),
        ));
    }
    if let Err(error) = control.checkpoint() {
        let parser_name = parser_name_for_kind(&kind);
        return Some(terminal(
            bytes,
            source.clone(),
            digest.to_string(),
            kind,
            schema,
            parser,
            error.operation_status(0),
            error.diagnostic(parser_name),
        ));
    }
    None
}

fn parser_name_for_kind(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::ICalendar => ICALENDAR_PARSER,
        ArtifactKind::VCard => VCARD_PARSER,
        _ => "grist.calendar_contact",
    }
}

#[allow(clippy::too_many_arguments)]
fn finish<T: serde::Serialize>(
    bytes: &[u8],
    source: SourceInfo,
    digest: String,
    kind: ArtifactKind,
    schema: &str,
    parser: ParserInfo,
    complete: bool,
    diagnostics: Vec<Diagnostic>,
    document: T,
) -> Envelope<T> {
    let envelope = if complete {
        Envelope::complete(
            OperationKind::Parse,
            kind,
            source,
            parser,
            digest,
            schema,
            document,
        )
    } else {
        Envelope::partial(
            OperationKind::Parse,
            kind,
            source,
            parser,
            digest,
            schema,
            Some(document),
        )
    };
    envelope
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_diagnostics(diagnostics)
        .with_canonical_payload_identity()
        .expect("calendar/contact payload serializes")
}

#[allow(clippy::too_many_arguments)]
fn terminal<T>(
    bytes: &[u8],
    source: SourceInfo,
    digest: String,
    kind: ArtifactKind,
    schema: &str,
    parser: ParserInfo,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> Envelope<T> {
    Envelope::without_payload(
        OperationKind::Parse,
        kind,
        status,
        source,
        parser,
        digest,
        schema,
    )
    .expect("terminal calendar/contact envelope is valid")
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(vec![diagnostic])
}
