//! Inert RFC 5322 and MIME parsing with exact nested provenance.

mod decode;
mod graph;
mod model;
mod parser;
mod secure;

pub use model::*;

use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Diagnostic, Envelope, Hashes, OperationControl,
    OperationKind, OperationStatus, ParserInfo, SchemaVersion, SourceInfo,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};

const PARSER: &str = "grist.email";
pub type EmailEnvelope = Envelope<EmailDocument>;

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("grist-rfc5322-mime", env!("CARGO_PKG_VERSION"))
        .with_feature("email-message")
        .with_specification_version("RFC 5322; MIME RFC 2045-2049; RFC 2231; TNEF; S/MIME RFC 8551")
}

pub(crate) fn parse_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    let options: EmailOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed("grist.registry", error.to_string())))?;
    let mut document = parser::parse_document(
        context.bytes(),
        context.source(),
        &options,
        context.control(),
    )?;
    let provider_evidence = secure::apply_smime_decryption(context, &mut document);
    let status = if document.complete {
        OperationStatus::Complete
    } else {
        OperationStatus::Partial
    };
    let payload = serde_json::to_value(&document)
        .map_err(|error| Box::new(Diagnostic::parser_defect(PARSER, error.to_string())))?;
    Ok(ParserOutput {
        status,
        payload: Some(payload),
        diagnostics: document.diagnostics.clone(),
        providers: provider_evidence.providers,
        provenance: provider_evidence.provenance,
    })
}

pub fn parse_email(bytes: &[u8], source: SourceInfo, options: &EmailOptions) -> EmailEnvelope {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .expect("trusted budget is valid");
    parse_email_with_operation_control(bytes, source, options, &control)
}

pub fn parse_email_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &EmailOptions,
    control: &OperationControl,
) -> EmailEnvelope {
    let digest = crate::core::options_digest(options).expect("email options serialize");
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
    let diagnostics = document.diagnostics.clone();
    let envelope = if document.complete {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Email,
            source,
            parser_info(),
            digest,
            SchemaVersion::EMAIL_V1,
            document,
        )
    } else {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Email,
            source,
            parser_info(),
            digest,
            SchemaVersion::EMAIL_V1,
            Some(document),
        )
    };
    envelope
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_diagnostics(diagnostics)
        .with_canonical_payload_identity()
        .expect("email payload serializes")
}

fn terminal(
    bytes: &[u8],
    source: SourceInfo,
    digest: String,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> EmailEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Email,
        status,
        source,
        parser_info(),
        digest,
        SchemaVersion::EMAIL_V1,
    )
    .expect("terminal email envelope is valid")
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(vec![diagnostic])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{LocationComponent, OperationStatus};
    use crate::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};

    #[test]
    fn multipart_alternative_inline_attachment_and_remote_reference_are_inert() {
        let message = concat!(
            "From: a@example.test\r\n",
            "Message-ID: <m@example.test>\r\n",
            "Content-Type: multipart/alternative; boundary=alt\r\n\r\n",
            "preamble\r\n--alt\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n\r\nplain\r\n",
            "--alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n",
            "<img src=https://example.test/tracker>html\r\n",
            "--alt--\r\nepilogue"
        );
        let envelope = parse_email(
            message.as_bytes(),
            SourceInfo::new("mail.eml"),
            &EmailOptions::default(),
        );
        assert_eq!(envelope.status, OperationStatus::Complete);
        let document = envelope.payload.unwrap();
        assert_eq!(document.mime.children.len(), 2);
        assert_eq!(document.external_references.len(), 1);
        assert!(!document.external_references[0].resolved);
        assert!(document.mime.children[1]
            .locator
            .components()
            .iter()
            .any(|component| matches!(component, LocationComponent::EmailPart { mime_path, .. } if mime_path[0].value == 2)));
        let graph = document
            .to_document_graph(DocumentGraphContext::new("g"))
            .unwrap();
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == DocumentNodeKind::MessageBody)
        );
        assert!(graph.edges.iter().any(|edge| edge.relation
            == crate::document_graph::DocumentRelation::AlternativeRepresentationOf));
    }

    #[test]
    fn attachment_identity_uses_decoded_bytes_and_rfc2231_filename() {
        let message = concat!(
            "Content-Type: multipart/mixed; boundary=x\r\n\r\n",
            "--x\r\nContent-Type: text/plain\r\n\r\nbody\r\n",
            "--x\r\nContent-Type: application/octet-stream\r\n",
            "Content-Disposition: attachment; filename*=UTF-8''hello%20world.txt\r\n",
            "Content-Transfer-Encoding: base64\r\n\r\nSGVsbG8=\r\n--x--\r\n"
        );
        let document = parse_email(
            message.as_bytes(),
            SourceInfo::new("attachment.eml"),
            &EmailOptions::default(),
        )
        .payload
        .unwrap();
        let attachment = document.mime.children[1].attachment.as_ref().unwrap();
        assert_eq!(attachment.filename.as_deref(), Some("hello world.txt"));
        assert_eq!(
            attachment
                .artifact
                .identity
                .content
                .raw
                .as_ref()
                .unwrap()
                .byte_length,
            5
        );
    }

    #[test]
    fn encrypted_smime_is_explicitly_partial() {
        let message =
            b"Content-Type: application/pkcs7-mime; smime-type=enveloped-data\r\n\r\nopaque";
        let envelope = parse_email(
            message,
            SourceInfo::new("encrypted.eml"),
            &EmailOptions::default(),
        );
        assert_eq!(envelope.status, OperationStatus::Partial);
        assert!(envelope.payload.unwrap().mime.encrypted);
    }
}
