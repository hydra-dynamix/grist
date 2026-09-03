//! Inert Microsoft Outlook MSG parsing over bounded Compound Binary File traversal.

mod cfb;
mod graph;
mod mapi;
mod model;
mod parser;
mod rtf;

pub use model::*;

use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Diagnostic, Envelope, Hashes, OperationControl,
    OperationKind, OperationStatus, ParserInfo, SchemaVersion, SourceInfo,
};

const PARSER: &str = "grist.outlook.msg";
pub type OutlookMsgEnvelope = Envelope<OutlookMsgDocument>;

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("grist-cfb-mapi-msg", crate::version())
        .with_feature("email-message")
        .with_specification_version("MS-CFB; MS-OXMSG; MS-OXPROPS; MS-OXRTFCP")
}

fn trusted_control() -> OperationControl {
    let selection = BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1);
    OperationControl::new(&selection, Default::default()).expect("trusted budget is valid")
}

pub fn parse_outlook_msg_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &OutlookMsgOptions,
    control: &OperationControl,
) -> OutlookMsgEnvelope {
    let digest = crate::core::options_digest(options).expect("MSG options serialize");
    if let Err(error) = control.budget().consume_input_bytes(bytes.len() as u64) {
        return terminal(
            bytes,
            source,
            digest,
            error.operation_status(0),
            error.diagnostic(PARSER),
        );
    }
    if let Err(error) = control.checkpoint() {
        return terminal(
            bytes,
            source,
            digest,
            error.operation_status(0),
            error.diagnostic(PARSER),
        );
    }
    let document = match parser::parse_document(bytes, &source, options, control) {
        Ok(document) => document,
        Err(diagnostic) => {
            return terminal(bytes, source, digest, OperationStatus::Failed, *diagnostic);
        }
    };
    finish_document(bytes, source, digest, document)
}

fn finish_document(
    bytes: &[u8],
    source: SourceInfo,
    digest: String,
    document: OutlookMsgDocument,
) -> OutlookMsgEnvelope {
    let diagnostics = document.diagnostics.clone();
    let envelope = if document.complete {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::OutlookMsg,
            source,
            parser_info(),
            digest,
            SchemaVersion::OUTLOOK_MSG_V1,
            document,
        )
    } else {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::OutlookMsg,
            source,
            parser_info(),
            digest,
            SchemaVersion::OUTLOOK_MSG_V1,
            Some(document),
        )
    };
    envelope
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_diagnostics(diagnostics)
        .with_canonical_payload_identity()
        .expect("MSG payload serializes")
}

fn terminal(
    bytes: &[u8],
    source: SourceInfo,
    digest: String,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> OutlookMsgEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::OutlookMsg,
        status,
        source,
        parser_info(),
        digest,
        SchemaVersion::OUTLOOK_MSG_V1,
    )
    .expect("terminal MSG envelope is valid")
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(vec![diagnostic])
}

pub fn parse_outlook_msg(
    bytes: &[u8],
    source: SourceInfo,
    options: &OutlookMsgOptions,
) -> OutlookMsgEnvelope {
    parse_outlook_msg_with_operation_control(bytes, source, options, &trusted_control())
}
