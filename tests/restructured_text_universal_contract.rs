#![cfg(all(feature = "restructured-text", feature = "schemas"))]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, OperationStatus, ParseRequest, ProviderSet,
    RequestId, ResourceBudget, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_source};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::restructured_text::{
    IncludeStatus, RestructuredTextNodeKind, RestructuredTextOptions, parse_restructured_text,
    parse_restructured_text_bytes, parse_restructured_text_with_options,
};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::fs;

const RICH: &str = "====================\nUniversal document\n====================\n\nParagraph with *emphasis*, **strong**, ``literal``, :code:`role`, `link <https://example.test>`_, target_, [1]_, [cite]_, and |name|.\n\n.. _target: destination\n.. |name| replace:: Grist\n.. [1] Footnote body.\n.. [cite] Citation body.\n\n.. warning:: inert\n   :class: important\n\n.. unknown-extension:: retained raw\n\n.. code-block:: rust\n\n   fn inert() {}\n\n+------+-------+\n| Name | Value |\n+======+=======+\n| one  | 1     |\n+------+-------+\n";

#[test]
fn authoritative_payload_covers_rich_syntax_with_exact_locations() {
    let first = parse_restructured_text(RICH, SourceInfo::stdin("rich.rst"));
    let second = parse_restructured_text(RICH, SourceInfo::stdin("rich.rst"));
    assert_eq!(first, second);
    assert_eq!(first.status, OperationStatus::Complete);
    let payload = first.payload.unwrap();
    for kind in [
        RestructuredTextNodeKind::Heading,
        RestructuredTextNodeKind::Paragraph,
        RestructuredTextNodeKind::Emphasis,
        RestructuredTextNodeKind::Strong,
        RestructuredTextNodeKind::InlineCode,
        RestructuredTextNodeKind::Role,
        RestructuredTextNodeKind::Hyperlink,
        RestructuredTextNodeKind::CrossReference,
        RestructuredTextNodeKind::FootnoteDefinition,
        RestructuredTextNodeKind::FootnoteReference,
        RestructuredTextNodeKind::CitationDefinition,
        RestructuredTextNodeKind::CitationReference,
        RestructuredTextNodeKind::Directive,
        RestructuredTextNodeKind::CodeBlock,
        RestructuredTextNodeKind::Table,
        RestructuredTextNodeKind::TableRow,
        RestructuredTextNodeKind::TableCell,
    ] {
        assert!(
            payload.nodes.iter().any(|node| node.kind == kind),
            "{kind:?}"
        );
    }
    assert!(payload.nodes.iter().all(|node| {
        node.range.byte_end <= payload.decoded_text.len()
            && node.raw == payload.decoded_text[node.range.byte_start..node.range.byte_end]
            && node.locator.validate().is_ok()
    }));
}

#[test]
fn graph_relations_segments_and_schema_are_deterministic() {
    let envelope = parse_restructured_text(RICH, SourceInfo::stdin("graph.rst"));
    let payload = envelope.payload.as_ref().unwrap();
    let context = DocumentGraphContext::new("rst:graph").with_source(envelope.source.clone());
    let first = payload.to_document_graph(context.clone()).unwrap();
    let second = payload.to_document_graph(context).unwrap();
    assert_eq!(first, second);
    for kind in [
        DocumentNodeKind::Heading,
        DocumentNodeKind::Paragraph,
        DocumentNodeKind::Emphasis,
        DocumentNodeKind::Strong,
        DocumentNodeKind::InlineCode,
        DocumentNodeKind::Link,
        DocumentNodeKind::Reference,
        DocumentNodeKind::Footnote,
        DocumentNodeKind::Citation,
        DocumentNodeKind::BibliographyEntry,
        DocumentNodeKind::CodeBlock,
        DocumentNodeKind::Table,
        DocumentNodeKind::TableRow,
        DocumentNodeKind::TableCell,
        DocumentNodeKind::RawBlock,
        DocumentNodeKind::RawInline,
    ] {
        assert!(first.nodes.iter().any(|node| node.kind == kind), "{kind:?}");
    }
    assert!(first.edges.iter().any(|edge| {
        edge.relation == DocumentRelation::LinksTo && edge.target == "https://example.test"
    }));
    assert!(
        first
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::References)
    );
    let source_identity = envelope.identity.as_ref().unwrap();
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(first.schema_version.as_str(), &first)
        .unwrap();
    let segments = segment_document_graph(
        &first,
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
            .all(|segment| { !segment.node_ids.is_empty() && !segment.locators.is_empty() })
    );
    for (name, value) in [
        ("restructured-text", serde_json::to_value(payload).unwrap()),
        (
            "restructured-text-envelope",
            serde_json::to_value(&envelope).unwrap(),
        ),
        (
            "restructured-text-options",
            serde_json::to_value(RestructuredTextOptions::default()).unwrap(),
        ),
    ] {
        let result = grist::schema::validate_schema(name, &value).unwrap();
        assert!(result.valid, "{name}: {:?}", result.issues);
    }
}

#[test]
fn detection_registry_status_and_budget_contracts_agree() {
    let registry = builtin_parser_registry().unwrap();
    assert!(matches!(
        registry.select_format("rst"),
        ParserSelection::Available(_)
    ));
    for (name, bytes) in [
        ("valid.rst", b"Title\n=====\n".as_slice()),
        ("mislabeled.bin", b".. warning:: inert\n".as_slice()),
        ("extensionless", b"Section\n=======\n".as_slice()),
        ("malformed.rst", b".. broken::\n   :bad:\n".as_slice()),
    ] {
        let detection = detect_source(
            &SourceInfo::stdin(name),
            bytes,
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detection.status, DetectionStatus::Selected, "{name}");
        assert_eq!(
            detection
                .selected_format_identity()
                .as_ref()
                .map(|identity| identity.format.as_str()),
            Some("restructured-text")
        );
    }
    let partial = parse_restructured_text_bytes(
        b"Title\n=====\ninvalid \xff",
        SourceInfo::stdin("invalid.rst"),
        &RestructuredTextOptions {
            encoding: Some("utf-8".into()),
            ..Default::default()
        },
    );
    assert_eq!(partial.status, OperationStatus::Partial);
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_decoded_characters = Some(1);
    let request = ParseRequest::new(
        RequestId::new("rst-budget").unwrap(),
        Input::bytes(b"Title\n=====\n".to_vec()),
        SourceInfo::stdin("budget.rst"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let failed = registry
        .dispatch("restructured-text", request, None)
        .unwrap();
    assert_eq!(failed.status, OperationStatus::Failed);
    assert!(failed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str().starts_with("budget.")
            || diagnostic.code.as_str().contains("resource")
            || diagnostic.message.contains("budget")
    }));
}

#[test]
fn includes_require_explicit_root_and_never_fetch_or_escape() {
    let base = std::env::temp_dir().join(format!("grist-rst-contract-{}", std::process::id()));
    let root = base.join("project");
    fs::create_dir_all(&root).unwrap();
    let main = root.join("main.rst");
    fs::write(&main, "root").unwrap();
    fs::write(root.join("child.rst"), "Child\n=====\n").unwrap();
    fs::write(base.join("outside.rst"), "outside").unwrap();
    let options = RestructuredTextOptions {
        project_root: Some(root.clone()),
        ..Default::default()
    };
    let source = SourceInfo::from_path(&main);
    let envelope = parse_restructured_text_with_options(
        ".. include:: child.rst\n\n.. include:: ../outside.rst\n\n.. include:: https://network.invalid/never.rst\n",
        source,
        &options,
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let statuses = envelope
        .payload
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|node| node.include.as_ref().map(|include| include.status.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        [
            IncludeStatus::Resolved,
            IncludeStatus::OutsideProjectRoot,
            IncludeStatus::RemoteDisabled,
        ]
    );
    let graph = envelope
        .payload
        .unwrap()
        .to_document_graph(DocumentGraphContext::new("rst:includes"))
        .unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ResolvesTo)
    );
    assert!(graph.nodes.iter().any(|node| {
        node.attrs.get("repository_relative_path") == Some(&serde_json::json!("child.rst"))
    }));
    fs::remove_dir_all(base).unwrap();
}
