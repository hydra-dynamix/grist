//! Inert Jupyter Notebook (nbformat 3 and 4) parsing.
//!
//! Parsing never starts a kernel, imports code, evaluates a cell, renders
//! active HTML, resolves attachments, or instantiates widget state.

mod graph;
mod model;
mod parser;

pub use model::*;

use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Diagnostic, Envelope, Hashes, OperationControl,
    OperationKind, OperationStatus, ParserInfo, SchemaVersion, SourceInfo,
};

const PARSER: &str = "grist.ipynb";

pub type NotebookEnvelope = Envelope<NotebookDocument>;

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("serde_json", "1")
        .with_feature("notebooks")
        .with_specification_version("Jupyter nbformat 3.x and 4.x")
}

pub fn parse_notebook(
    bytes: &[u8],
    source: SourceInfo,
    options: &NotebookOptions,
) -> NotebookEnvelope {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .expect("trusted budget is valid");
    parse_notebook_with_operation_control(bytes, source, options, &control)
}

pub fn parse_jupyter_notebook(
    bytes: &[u8],
    source: SourceInfo,
    options: &NotebookOptions,
) -> NotebookEnvelope {
    parse_notebook(bytes, source, options)
}

pub fn parse_notebook_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &NotebookOptions,
    control: &OperationControl,
) -> NotebookEnvelope {
    let digest = crate::core::options_digest(options).expect("notebook options serialize");
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
    match parser::parse_document(bytes, options, control) {
        Ok(document) => {
            let diagnostics = document.diagnostics.clone();
            let envelope = if document.complete {
                Envelope::complete(
                    OperationKind::Parse,
                    ArtifactKind::Notebook,
                    source,
                    parser_info(),
                    digest,
                    SchemaVersion::IPYNB_V1,
                    document,
                )
            } else {
                Envelope::partial(
                    OperationKind::Parse,
                    ArtifactKind::Notebook,
                    source,
                    parser_info(),
                    digest,
                    SchemaVersion::IPYNB_V1,
                    Some(document),
                )
            };
            envelope
                .with_hashes(Hashes::for_bytes(bytes, None))
                .with_diagnostics(diagnostics)
                .with_canonical_payload_identity()
                .expect("notebook payload serializes")
        }
        Err(diagnostic) => terminal(bytes, source, digest, OperationStatus::Failed, *diagnostic),
    }
}

fn terminal(
    bytes: &[u8],
    source: SourceInfo,
    digest: String,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> NotebookEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Notebook,
        status,
        source,
        parser_info(),
        digest,
        SchemaVersion::IPYNB_V1,
    )
    .expect("terminal notebook envelope is valid")
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(vec![diagnostic])
}
