#![cfg(feature = "email-message")]

use grist::core::{
    BudgetSelection, ContentIdentity, OperationControl, OperationStatus, RequestId, ResourceBudget,
    SourceInfo, StreamEvent,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::mbox::{
    MboxOptions, MboxVariant, parse_mbox, parse_mbox_with_operation_control, stream_mbox,
};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::segment::{SegmentOptions, segment_document_graph};

fn message(sender: &str, id: &str, extra: &str, body: &str) -> String {
    format!("From {sender} Fri Jul  8 12:08:34 2022\nMessage-ID: <{id}>\n{extra}\n{body}")
}

#[test]
fn extensionless_detection_registry_graph_and_schema_are_integrated() {
    let bytes = message(
        "alice@example.test",
        "one@example.test",
        "Subject: Topic\n",
        "body\n",
    );
    let registry = builtin_parser_registry().unwrap();
    let detection = detect_with_registry(
        std::path::Path::new("mailbox"),
        bytes.as_bytes(),
        None,
        None,
        &grist::core::Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Mbox);
    assert_eq!(detection.candidates[0].identity.format, "mbox");
    assert!(matches!(
        registry.select_format("mbox"),
        ParserSelection::Available(_)
    ));

    let envelope = parse_mbox(
        bytes.as_bytes(),
        SourceInfo::new("mailbox"),
        &MboxOptions::default(),
    );
    let source_identity = envelope.identity.as_ref().unwrap();
    let document = envelope.payload.as_ref().unwrap();
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:mbox"))
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
            .any(|node| node.kind == DocumentNodeKind::MessageBody)
    );
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| !segment.node_ids.is_empty() && !segment.locators.is_empty())
    );
    for name in ["mbox", "mbox-envelope", "mbox-options", "mbox-stream-event"] {
        assert!(grist::schema::schema_json(name).is_some(), "missing {name}");
    }
}

#[cfg(feature = "cli")]
#[test]
fn cli_parses_mbox_through_the_registered_surface() {
    use std::fs;
    use std::process::Command;

    let path =
        std::env::temp_dir().join(format!("grist-mbox-contract-{}.mbox", std::process::id()));
    fs::write(
        &path,
        message(
            "alice@example.test",
            "cli@example.test",
            "Subject: CLI\n",
            "body\n",
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "mbox", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["payload"]["messages"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["payload"]["messages"][0]["email"]["headers"][0]["name"],
        "Message-ID"
    );
}

#[test]
fn content_length_protects_unescaped_separator_like_body_lines() {
    let body = b"alpha\nFrom fake Fri Jul  8 12:09:34 2022\nomega\n";
    let mut mailbox = format!(
        "From alice@example.test Fri Jul  8 12:08:34 2022\nMessage-ID: <one>\nContent-Length: {}\n\n",
        body.len()
    )
    .into_bytes();
    mailbox.extend_from_slice(body);
    mailbox.extend_from_slice(
        b"From bob@example.test Fri Jul  8 12:10:34 2022\nMessage-ID: <two>\n\nsecond\n",
    );
    let document = parse_mbox(
        &mailbox,
        SourceInfo::new("mailbox.mbox"),
        &MboxOptions::default(),
    )
    .payload
    .unwrap();
    assert_eq!(document.messages.len(), 2);
    assert_eq!(document.variant, MboxVariant::Mixed);
    assert!(
        document.messages[0]
            .content_length
            .as_ref()
            .unwrap()
            .honored
    );
    assert!(
        document.messages[0]
            .email
            .as_ref()
            .unwrap()
            .mime
            .text
            .as_ref()
            .unwrap()
            .text
            .contains("From fake")
    );
}

#[test]
fn identities_survive_unrelated_insertions_and_exact_duplicates_are_distinct() {
    let first = message("alice@example.test", "one", "", "one\n");
    let second = message("bob@example.test", "two", "", "two\n");
    let original = format!("{first}{second}");
    let inserted = format!(
        "{}{}{}{}",
        message("new@example.test", "new", "", "new\n"),
        first,
        second,
        second
    );
    let original = parse_mbox(
        original.as_bytes(),
        SourceInfo::new("original.mbox"),
        &MboxOptions::default(),
    )
    .payload
    .unwrap();
    let inserted = parse_mbox(
        inserted.as_bytes(),
        SourceInfo::new("inserted.mbox"),
        &MboxOptions::default(),
    )
    .payload
    .unwrap();
    assert_eq!(
        original.messages[0].stable_id,
        inserted.messages[1].stable_id
    );
    assert_eq!(
        original.messages[1].stable_id,
        inserted.messages[2].stable_id
    );
    assert_ne!(
        inserted.messages[2].stable_id,
        inserted.messages[3].stable_id
    );
    assert_eq!(inserted.messages[3].duplicate_occurrence, 2);
}

#[test]
fn truncation_missing_separator_and_shared_attachment_budget_are_explicit() {
    let truncated =
        b"From alice@example.test Fri Jul  8 12:08:34 2022\nContent-Length: 100\n\nshort";
    let envelope = parse_mbox(
        truncated,
        SourceInfo::new("truncated.mbox"),
        &MboxOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "mbox.content_length.truncated" })
    );

    let malformed = parse_mbox(
        b"Message-ID: <recovered>\n\nbody",
        SourceInfo::new("malformed.mbox"),
        &MboxOptions::default(),
    );
    assert_eq!(malformed.status, OperationStatus::Partial);
    assert_eq!(malformed.payload.unwrap().messages.len(), 1);

    let attachment_message = |id: &str| {
        message(
            "alice@example.test",
            id,
            "Content-Type: multipart/mixed; boundary=x\n",
            "--x\nContent-Type: application/octet-stream\nContent-Disposition: attachment; filename=x.bin\n\nbytes\n--x--\n",
        )
    };
    let mailbox = format!("{}{}", attachment_message("one"), attachment_message("two"));
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_child_artifacts = Some(1);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let envelope = parse_mbox_with_operation_control(
        mailbox.as_bytes(),
        SourceInfo::new("budget.mbox"),
        &MboxOptions::default(),
        &control,
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert_eq!(control.usage().child_artifacts, 2);
}

#[test]
fn streaming_order_and_thread_edges_match_batch() {
    let mailbox = format!(
        "{}{}",
        message("alice@example.test", "one", "Subject: Topic\n", "one\n"),
        message(
            "bob@example.test",
            "two",
            "In-Reply-To: <one>\nReferences: <one>\nSubject: Re: Topic\n",
            "two\n"
        )
    );
    let control = OperationControl::new(
        &BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
        Default::default(),
    )
    .unwrap();
    let streamed = stream_mbox(
        mailbox.as_bytes(),
        SourceInfo::new("thread.mbox"),
        &MboxOptions::default(),
        RequestId::new("stream").unwrap(),
        control,
    )
    .filter_map(|event| match event {
        StreamEvent::Item { item } => Some(item.payload.stable_id),
        StreamEvent::Terminal { .. } => None,
    })
    .collect::<Vec<_>>();
    let document = parse_mbox(
        mailbox.as_bytes(),
        SourceInfo::new("thread.mbox"),
        &MboxOptions::default(),
    )
    .payload
    .unwrap();
    assert_eq!(
        streamed,
        document
            .messages
            .iter()
            .map(|message| message.stable_id.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        document
            .thread_evidence
            .links
            .iter()
            .all(|link| link.resolved)
    );
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:thread"))
        .unwrap();
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
            .any(|edge| edge.relation == DocumentRelation::Precedes)
    );
}
