#![cfg(all(feature = "asciidoc", feature = "schemas"))]

use grist::asciidoc::{
    AsciiDocNodeKind, AsciiDocOptions, IncludeStatus, parse_asciidoc, parse_asciidoc_bytes,
    parse_asciidoc_with_options,
};
use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, OperationStatus, ParseRequest, ProviderSet,
    RequestId, ResourceBudget, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_source};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::fs;

const RICH: &str = "= Universal document\n:toc: left\n:icons: font\n\n== Section\n\nParagraph with _emphasis_, *strong*, `literal`, [.lead]#role#, https://example.test[link], <<target>>, footnote:id[Footnote body], footnote:id[], {toc}, and widget:thing[].\n\n[[target]]\nimage::figure.png[Alt text]\nunknown-extension::retained[raw]\n\n[source,rust,linenums]\n----\nfn inert() {}\n----\n\n|===\n|Name |Value\n|one |1\n|===\n";

#[test]
fn authoritative_payload_covers_rich_syntax_with_exact_locations() {
    let first = parse_asciidoc(RICH, SourceInfo::stdin("rich.adoc"));
    let second = parse_asciidoc(RICH, SourceInfo::stdin("rich.adoc"));
    assert_eq!(first, second);
    assert_eq!(first.status, OperationStatus::Complete);
    let payload = first.payload.unwrap();
    for kind in [
        AsciiDocNodeKind::Heading,
        AsciiDocNodeKind::Paragraph,
        AsciiDocNodeKind::Attribute,
        AsciiDocNodeKind::BlockAttribute,
        AsciiDocNodeKind::Emphasis,
        AsciiDocNodeKind::Strong,
        AsciiDocNodeKind::InlineCode,
        AsciiDocNodeKind::Role,
        AsciiDocNodeKind::Hyperlink,
        AsciiDocNodeKind::CrossReference,
        AsciiDocNodeKind::FootnoteDefinition,
        AsciiDocNodeKind::FootnoteReference,
        AsciiDocNodeKind::Directive,
        AsciiDocNodeKind::CodeBlock,
        AsciiDocNodeKind::Table,
        AsciiDocNodeKind::TableRow,
        AsciiDocNodeKind::TableCell,
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
    let envelope = parse_asciidoc(RICH, SourceInfo::stdin("graph.adoc"));
    let payload = envelope.payload.as_ref().unwrap();
    let context = DocumentGraphContext::new("asciidoc:graph").with_source(envelope.source.clone());
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
        ("asciidoc", serde_json::to_value(payload).unwrap()),
        (
            "asciidoc-envelope",
            serde_json::to_value(&envelope).unwrap(),
        ),
        (
            "asciidoc-options",
            serde_json::to_value(AsciiDocOptions::default()).unwrap(),
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
        registry.select_format("adoc"),
        ParserSelection::Available(_)
    ));
    for (name, bytes) in [
        ("valid.adoc", b"= Title\n".as_slice()),
        (
            "mislabeled.bin",
            b":toc:\ninclude::part.adoc[]\n".as_slice(),
        ),
        ("extensionless", b"= Section\n".as_slice()),
        ("malformed.adoc", b"[source\n----\n".as_slice()),
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
            Some("asciidoc")
        );
    }
    let partial = parse_asciidoc_bytes(
        b"= Title\ninvalid \xff",
        SourceInfo::stdin("invalid.adoc"),
        &AsciiDocOptions {
            encoding: Some("utf-8".into()),
            ..Default::default()
        },
    );
    assert_eq!(partial.status, OperationStatus::Partial);
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_decoded_characters = Some(1);
    let request = ParseRequest::new(
        RequestId::new("asciidoc-budget").unwrap(),
        Input::bytes(b"= Title\n".to_vec()),
        SourceInfo::stdin("budget.adoc"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let failed = registry.dispatch("asciidoc", request, None).unwrap();
    assert_eq!(failed.status, OperationStatus::Failed);
    assert!(failed.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str().starts_with("budget.")
            || diagnostic.code.as_str().contains("resource")
            || diagnostic.message.contains("budget")
    }));
}

#[test]
fn includes_require_explicit_root_and_never_fetch_or_escape() {
    let base = std::env::temp_dir().join(format!("grist-asciidoc-contract-{}", std::process::id()));
    let root = base.join("project");
    fs::create_dir_all(&root).unwrap();
    let main = root.join("main.adoc");
    fs::write(&main, "root").unwrap();
    fs::write(root.join("child.adoc"), "= Child\n").unwrap();
    fs::write(base.join("outside.adoc"), "outside").unwrap();
    let options = AsciiDocOptions {
        project_root: Some(root.clone()),
        ..Default::default()
    };
    let source = SourceInfo::from_path(&main);
    let envelope = parse_asciidoc_with_options(
        "include::child.adoc[]\n\ninclude::../outside.adoc[]\n\ninclude::https://network.invalid/never.adoc[]\n",
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
        .to_document_graph(DocumentGraphContext::new("asciidoc:includes"))
        .unwrap();
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ResolvesTo)
    );
    assert!(graph.nodes.iter().any(|node| {
        node.attrs.get("repository_relative_path") == Some(&serde_json::json!("child.adoc"))
    }));
    fs::remove_dir_all(base).unwrap();
}
