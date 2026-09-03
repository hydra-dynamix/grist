#![cfg(feature = "email-message")]

use grist::core::{LocationComponent, OperationStatus, SourceInfo};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::email::{EmailOptions, parse_email};
use grist::registry::builtin_parser_registry;

#[test]
fn extensionless_message_is_structurally_detected_and_registered() {
    let bytes = b"From: sender@example.test\r\nTo: reader@example.test\r\nMessage-ID: <id@example.test>\r\n\r\nbody";
    let detected = detect_with_registry(
        std::path::Path::new("extensionless"),
        bytes,
        None,
        None,
        &grist::core::Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detected.status, DetectionStatus::Selected);
    assert_eq!(detected.content_kind, ContentKind::Eml);
    assert_eq!(detected.candidates[0].identity.format, "eml");
}

#[test]
fn mime_hierarchy_alternatives_inline_resources_attachments_and_graph_survive() {
    let bytes = concat!(
        "From: Sender <sender@example.test>\r\n",
        "To: Reader <reader@example.test>\r\n",
        "Message-ID: <current@example.test>\r\n",
        "In-Reply-To: <parent@example.test>\r\n",
        "References: <root@example.test> <parent@example.test>\r\n",
        "Content-Type: multipart/mixed; boundary=mix\r\n\r\n",
        "--mix\r\nContent-Type: multipart/alternative; boundary=alt\r\n\r\n",
        "--alt\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nplain\r\n",
        "--alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n",
        "<img src=https://example.test/pixel><img src=cid:logo>html\r\n--alt--\r\n",
        "--mix\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\n",
        "Content-Disposition: inline; filename=logo.png\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\n",
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgABSK+kcQAAAABJRU5ErkJggg==\r\n",
        "--mix--\r\n"
    );
    let envelope = parse_email(
        bytes.as_bytes(),
        SourceInfo::new("complex.eml"),
        &EmailOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.mime.children[0].children.len(), 2);
    assert!(
        document.mime.children[1]
            .attachment
            .as_ref()
            .unwrap()
            .inline_resource
    );
    assert_eq!(document.external_references.len(), 1);
    assert!(!document.external_references[0].resolved);
    let locator = &document.mime.children[0].children[1].locator;
    assert!(matches!(
        &locator.components()[0],
        LocationComponent::EmailPart { mime_path, .. }
            if mime_path.iter().map(|position| position.value).collect::<Vec<_>>() == vec![1, 2]
    ));
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:email"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Email)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Attachment)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ReplyTo)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::AlternativeRepresentationOf)
    );
}

#[test]
fn supported_attachment_is_parsed_recursively_under_the_shared_budget() {
    let bytes = concat!(
        "Content-Type: multipart/mixed; boundary=x\r\n\r\n",
        "--x\r\nContent-Type: application/json\r\n",
        "Content-Disposition: attachment; filename=data.json\r\n\r\n",
        "42\r\n--x--\r\n"
    );
    let document = parse_email(
        bytes.as_bytes(),
        SourceInfo::new("nested.eml"),
        &EmailOptions::default(),
    )
    .payload()
    .unwrap()
    .clone();
    let nested = document.mime.children[0]
        .attachment
        .as_ref()
        .unwrap()
        .nested
        .as_ref()
        .unwrap();
    assert_eq!(nested.format, "json");
    assert_eq!(nested.status, OperationStatus::Complete);
    assert_eq!(
        nested.envelope["source"]["parent"]["display_name"],
        "nested.eml"
    );
}

#[test]
fn malformed_deep_and_encrypted_mail_is_explicitly_partial_without_panics() {
    let malformed = b"Content-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nBroken\r\nbody";
    let envelope = parse_email(
        malformed,
        SourceInfo::new("malformed.eml"),
        &EmailOptions {
            max_mime_depth: 1,
            ..EmailOptions::default()
        },
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.partial)
    );

    let encrypted =
        b"Content-Type: application/pkcs7-mime; smime-type=enveloped-data\r\n\r\nopaque";
    let envelope = parse_email(
        encrypted,
        SourceInfo::new("encrypted.eml"),
        &EmailOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert_eq!(
        envelope.payload().unwrap().encrypted_parts,
        vec![Vec::<usize>::new()]
    );
}

#[cfg(feature = "schemas")]
#[test]
fn payload_envelope_and_options_schemas_are_registered() {
    for name in ["email", "email-envelope", "email-options"] {
        assert!(grist::schema::schema_json(name).is_some(), "missing {name}");
    }
}
