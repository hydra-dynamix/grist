#![cfg(feature = "serialization")]

use grist::core::{
    BudgetSelection, CancellationToken, ContentIdentity, LocationComponent, OperationControl,
    OperationStatus, RequestId, ResourceBudget, SourceInfo,
};
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::segment::{SegmentOptions, segment_document_graph};
use grist::serialization::{
    MalformedRecoveryPolicy, SerializationFormat, SerializationOptions, StructuredScalar,
    StructuredValueKind, parse_serialization, parse_serialization_with_options, stream_jsonl,
};

fn control(budget: ResourceBudget) -> OperationControl {
    OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap()
}

#[test]
fn json_preserves_order_duplicates_types_and_exact_pointers() {
    let source = r#"{"z":null,"a":1,"a":2,"big":184467440737095516160,"f":1.25,"s":"x","b":true}"#;
    let envelope = parse_serialization(
        source,
        SerializationFormat::Json,
        SourceInfo::stdin("data.json"),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document = envelope.payload().unwrap();
    let root = &document.documents[0];
    assert_eq!(
        root.entries
            .iter()
            .filter_map(|entry| entry.key_text.as_deref())
            .collect::<Vec<_>>(),
        ["z", "a", "a", "big", "f", "s", "b"]
    );
    assert_eq!(document.duplicate_keys.len(), 1);
    assert_eq!(root.entries[2].duplicate_ordinal, 2);
    assert_eq!(
        root.entries[3].value.scalar,
        Some(StructuredScalar::Integer {
            canonical: "184467440737095516160".into()
        })
    );
    assert_eq!(root.entries[4].value.kind, StructuredValueKind::Float);
    assert!(matches!(
        root.entries[1].value.locator.components().last(),
        Some(LocationComponent::JsonPointer { pointer }) if pointer == "/a"
    ));
    let range = &root.entries[3].value.range;
    assert_eq!(
        &source[range.byte_start..range.byte_end],
        "184467440737095516160"
    );
}

#[test]
fn json_malformed_policy_is_explicit_and_raw_is_not_silent() {
    let strict = parse_serialization(
        "{\"a\":",
        SerializationFormat::Json,
        SourceInfo::stdin("bad.json"),
    );
    assert_eq!(strict.status, OperationStatus::Failed);
    assert!(strict.payload.is_none());
    let recovered = parse_serialization_with_options(
        "{\"a\":",
        SerializationFormat::Json,
        SourceInfo::stdin("bad.json"),
        &SerializationOptions {
            malformed_recovery: MalformedRecoveryPolicy::PreserveRaw,
            ..Default::default()
        },
    );
    assert_eq!(recovered.status, OperationStatus::Partial);
    assert_eq!(recovered.payload().unwrap().raw_unknowns[0].raw, "{\"a\":");
}

#[test]
fn explicit_nesting_limit_never_masquerades_as_complete() {
    let envelope = parse_serialization_with_options(
        "[[[1]]]",
        SerializationFormat::Json,
        SourceInfo::stdin("deep.json"),
        &SerializationOptions {
            max_nesting_depth: 2,
            ..Default::default()
        },
    );
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert_eq!(envelope.diagnostics[0].code, "structured.nesting_limit");
}

#[test]
fn ndjson_stream_is_record_isolated_budgeted_and_cancellable() {
    let source = "{\"a\":1}\r\nnot-json\n{\"b\":2}\n";
    let events = stream_jsonl(
        source,
        &SerializationOptions::default(),
        RequestId::new("test/ndjson").unwrap(),
        control(ResourceBudget::trusted_unbounded()),
    )
    .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, grist::core::StreamEvent::Item { .. }))
            .count(),
        3
    );
    assert!(matches!(
        events.last().unwrap(),
        grist::core::StreamEvent::Terminal { terminal }
            if terminal.status == OperationStatus::Partial && terminal.emitted_items == 3
    ));
    let bad = match &events[1] {
        grist::core::StreamEvent::Item { item } => &item.payload,
        _ => unreachable!(),
    };
    assert!(bad.value.is_none());
    assert!(matches!(
        bad.locator.components().get(1),
        Some(LocationComponent::RecordRange { .. })
    ));

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_records = Some(1);
    let budgeted = stream_jsonl(
        source,
        &SerializationOptions::default(),
        RequestId::new("test/budget").unwrap(),
        control(budget),
    )
    .collect::<Vec<_>>();
    assert!(matches!(
        budgeted.last().unwrap(),
        grist::core::StreamEvent::Terminal { terminal }
            if terminal.status == OperationStatus::Partial && terminal.emitted_items == 1
    ));

    let cancellation = CancellationToken::new();
    let cancel_control = OperationControl::new(
        &BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
        cancellation.clone(),
    )
    .unwrap();
    let mut stream = stream_jsonl(
        source,
        &SerializationOptions::default(),
        RequestId::new("test/cancel").unwrap(),
        cancel_control,
    );
    assert!(matches!(
        stream.next(),
        Some(grist::core::StreamEvent::Item { .. })
    ));
    cancellation.cancel();
    assert!(matches!(
        stream.next(),
        Some(grist::core::StreamEvent::Terminal { terminal })
            if terminal.status == OperationStatus::Cancelled && terminal.emitted_items == 1
    ));
}

#[test]
fn yaml_preserves_documents_duplicate_keys_tags_anchors_and_aliases() {
    let source = "---\nbase: &base {x: 1}\ncopy: *base\ndup: one\ndup: two\ntagged: !example value\n---\n- true\n- null\n";
    let envelope = parse_serialization(
        source,
        SerializationFormat::Yaml,
        SourceInfo::stdin("data.yaml"),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let document = envelope.payload().unwrap();
    assert_eq!(document.documents.len(), 2);
    assert_eq!(document.duplicate_keys.len(), 1);
    assert_eq!(document.aliases.len(), 1);
    assert!(document.aliases[0].resolved);
    assert!(!document.aliases[0].expanded);
    assert_eq!(
        document.documents[0].entries[4].value.tag.as_deref(),
        Some("!example")
    );
    assert_eq!(
        document.documents[1].items[0].kind,
        StructuredValueKind::Boolean
    );
}

#[test]
fn toml_retains_source_order_native_datetime_and_precise_ranges() {
    let source =
        "title = \"demo\"\nwhen = 2026-08-07T12:34:56Z\nnums = [1, 2]\n[owner]\nname = \"Ada\"\n";
    let envelope = parse_serialization(
        source,
        SerializationFormat::Toml,
        SourceInfo::stdin("data.toml"),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let root = &envelope.payload().unwrap().documents[0];
    assert_eq!(root.entries[0].key_text.as_deref(), Some("title"));
    assert_eq!(root.entries[1].value.kind, StructuredValueKind::DateTime);
    let range = &root.entries[1].value.range;
    assert_eq!(
        &source[range.byte_start..range.byte_end],
        "2026-08-07T12:34:56Z"
    );
}

#[test]
fn xml_projection_reuses_secure_ordered_xml_parser() {
    let source = "<root b=\"2\" a=\"1\">left<child/>right</root>";
    let envelope = parse_serialization(
        source,
        SerializationFormat::Xml,
        SourceInfo::stdin("data.xml"),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let root = &envelope.payload().unwrap().documents[0];
    assert_eq!(root.kind, StructuredValueKind::XmlElement);
    assert_eq!(root.entries[0].key_text.as_deref(), Some("@b"));
    assert_eq!(root.entries[1].key_text.as_deref(), Some("@a"));
    assert_eq!(root.items.len(), 3);
    assert!(matches!(
        root.locator.components().last(),
        Some(LocationComponent::XmlPath { .. })
    ));

    let hostile = parse_serialization(
        "<!DOCTYPE x [<!ENTITY e SYSTEM \"https://invalid.example/e\">]><x>&e;</x>",
        SerializationFormat::Xml,
        SourceInfo::stdin("hostile.xml"),
    );
    assert_ne!(hostile.status, OperationStatus::Complete);
    assert!(hostile.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str().contains("external") || diagnostic.code.as_str().contains("entity")
    }));
}

#[test]
fn graph_segments_schema_and_canonical_identity_are_deterministic() {
    let source = "{\"a\":[1,true,null],\"b\":{\"c\":\"text\"}}";
    let first = parse_serialization(
        source,
        SerializationFormat::Json,
        SourceInfo::stdin("graph.json"),
    );
    let second = parse_serialization(
        source,
        SerializationFormat::Json,
        SourceInfo::stdin("graph.json"),
    );
    assert_eq!(first.identity, second.identity);
    let document = first.into_payload().unwrap();
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:structured"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Field)
    );
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::StructuredValue)
            .all(|node| node.locator.is_some())
    );
    let source_identity = ContentIdentity::for_raw_bytes(source.as_bytes());
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(!segments.segments.is_empty());
    assert!(
        segments
            .segments
            .iter()
            .all(|segment| !segment.locators.is_empty())
    );

    let schema = grist::schema::schema_json("serialization").unwrap();
    let report = grist::schema::validate_against_schema(
        "serialization",
        grist::core::SchemaVersion::STRUCTURED_TEXT_V2,
        &serde_json::to_value(&document).unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
}
