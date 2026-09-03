#![cfg(all(feature = "html", feature = "schemas", feature = "document-graph"))]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, OperationStatus, ParseRequest, ProviderSet,
    RequestId, ResourceBudget, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_source};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::html::{
    HtmlActiveContentDisposition, HtmlNodeKind, HtmlOptions, HtmlParseMode, HtmlSourceTokenKind,
    HtmlSyntax, parse_html, parse_html_bytes,
};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::render::{FidelityMode, RenderFormat, RenderOptions, render_document_graph};
use grist::segment::{SegmentOptions, segment_document_graph};

const RICH: &str = r#"<!doctype html>
<html lang="en">
<head>
<title>Universal HTML</title>
<meta name="author" content="Ada">
<link rel="stylesheet" href="https://network.invalid/style.css">
</head>
<body>
<header><h1>Universal HTML</h1></header>
<main>
<section id="intro"><h2>Introduction</h2><p>Text <strong>bold</strong> and <a href="/next">next</a>.</p></section>
<table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody><tr><td>one</td><td rowspan="2">1</td></tr></tbody></table>
<picture><source srcset="large.webp 2x"><img src="small.png" alt="Figure"></picture>
<script src="https://network.invalid/app.js">never()</script>
<form action="https://network.invalid/post"><button onclick="never()">Send</button></form>
<custom-widget data-state="opaque">retained</custom-widget>
</main>
</body>
</html>"#;

#[test]
fn authoritative_dom_metadata_tables_media_and_active_content_are_lossless() {
    let first = parse_html(
        RICH,
        SourceInfo::stdin("rich.html"),
        &HtmlOptions::default(),
    );
    let second = parse_html(
        RICH,
        SourceInfo::stdin("rich.html"),
        &HtmlOptions::default(),
    );
    assert_eq!(first, second);
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:#?}",
        first.diagnostics
    );
    let payload = first.payload.unwrap();
    assert_eq!(payload.mode, HtmlParseMode::Document);
    assert_eq!(payload.syntax, HtmlSyntax::Html5);
    assert_eq!(payload.tables[0].rows[0].cells[0].text, "Name");
    assert!(payload.metadata.iter().any(|item| item.kind == "title"));
    assert!(payload.links.iter().any(|link| link.destination == "/next"));
    assert!(payload.media.iter().any(|media| media.tag_name == "img"));
    assert!(
        payload
            .sections
            .iter()
            .any(|section| section.heading.as_deref() == Some("Introduction"))
    );
    assert!(
        payload
            .active_content
            .iter()
            .all(|active| active.disposition == HtmlActiveContentDisposition::Inert)
    );
    assert!(payload.active_content.len() >= 3);
    assert!(
        payload
            .nodes
            .iter()
            .all(|node| node.locator.validate().is_ok())
    );
    assert!(payload.source_tokens.iter().all(|token| {
        token.raw == payload.decoded_text[token.range.byte_start..token.range.byte_end]
            && token.locator.validate().is_ok()
    }));
    assert!(
        payload
            .nodes
            .iter()
            .filter(|node| !node.synthetic)
            .all(|node| {
                node.raw == payload.decoded_text[node.range.byte_start..node.range.byte_end]
            })
    );
    assert!(payload.nodes.iter().any(|node| {
        node.tag_name.as_deref() == Some("custom-widget")
            && !node.known_element
            && node.raw.contains("custom-widget")
    }));
}

#[test]
fn html5_recovery_and_xhtml_well_formedness_are_diagnosed() {
    let malformed = parse_html(
        "<table><td>cell</table><p>after",
        SourceInfo::stdin("malformed.html"),
        &HtmlOptions {
            mode: HtmlParseMode::Fragment,
            ..Default::default()
        },
    );
    assert_eq!(malformed.status, OperationStatus::Partial);
    assert!(malformed.payload.as_ref().is_some_and(|payload| {
        payload
            .nodes
            .iter()
            .any(|node| node.tag_name.as_deref() == Some("tbody") && node.synthetic)
    }));
    assert!(
        malformed.diagnostics.iter().any(|diagnostic| {
            diagnostic.explanation_key.as_deref() == Some("html.tree_recovery")
        })
    );

    let xhtml = parse_html(
        r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><body><P unquoted=value></p></body></html>"#,
        SourceInfo::stdin("malformed.xhtml").with_declared_mime_type("application/xhtml+xml"),
        &HtmlOptions::default(),
    );
    assert_eq!(xhtml.status, OperationStatus::Partial);
    assert!(xhtml.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.explanation_key.as_deref(),
            Some("xhtml.attribute_unquoted" | "xhtml.element_mismatch")
        )
    }));
    assert!(xhtml.payload.as_ref().is_some_and(|payload| {
        payload
            .source_tokens
            .iter()
            .any(|token| token.kind == HtmlSourceTokenKind::ProcessingInstruction)
    }));
}

#[test]
fn graph_segments_render_schema_and_cli_contracts_agree() {
    let envelope = parse_html(
        RICH,
        SourceInfo::stdin("graph.html"),
        &HtmlOptions::default(),
    );
    let payload = envelope.payload.as_ref().unwrap();
    let context = DocumentGraphContext::new("html:graph").with_source(envelope.source.clone());
    let first = payload.to_document_graph(context.clone()).unwrap();
    let second = payload.to_document_graph(context).unwrap();
    assert_eq!(first, second);
    for kind in [
        DocumentNodeKind::Section,
        DocumentNodeKind::Heading,
        DocumentNodeKind::Paragraph,
        DocumentNodeKind::Strong,
        DocumentNodeKind::Link,
        DocumentNodeKind::Table,
        DocumentNodeKind::TableRow,
        DocumentNodeKind::TableCell,
        DocumentNodeKind::Image,
        DocumentNodeKind::Form,
        DocumentNodeKind::RawBlock,
    ] {
        assert!(first.nodes.iter().any(|node| node.kind == kind), "{kind:?}");
    }
    assert!(
        first
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::LinksTo)
    );
    first.validate_contract().unwrap();
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
    let rendered = render_document_graph(
        &first,
        RenderFormat::Html,
        &RenderOptions {
            fidelity: FidelityMode::RawFallback,
        },
    )
    .unwrap();
    rendered.validate_source_map().unwrap();
    assert!(rendered.content.contains("Universal HTML"));

    for (name, value) in [
        ("html", serde_json::to_value(payload).unwrap()),
        ("html-envelope", serde_json::to_value(&envelope).unwrap()),
        (
            "html-options",
            serde_json::to_value(HtmlOptions::default()).unwrap(),
        ),
    ] {
        let result = grist::schema::validate_schema(name, &value).unwrap();
        assert!(result.valid, "{name}: {:?}", result.issues);
    }
}

#[test]
fn detection_decoding_registry_and_budget_use_the_shared_contract() {
    let registry = builtin_parser_registry().unwrap();
    assert!(matches!(
        registry.select_format("xhtml"),
        ParserSelection::Available(_)
    ));
    for (name, bytes) in [
        ("valid.html", b"<!doctype html><title>x</title>".as_slice()),
        (
            "mislabeled.bin",
            b"<html><body><p>x</p></body></html>".as_slice(),
        ),
        ("extensionless", b"<!doctype html><html></html>".as_slice()),
        ("malformed.html", b"<table><td>x</table>".as_slice()),
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
            Some("html")
        );
    }

    let mut encoded = br#"<!doctype html><meta charset="windows-1252"><p>caf"#.to_vec();
    encoded.push(0xe9);
    encoded.extend_from_slice(b"</p>");
    let decoded = parse_html_bytes(
        &encoded,
        SourceInfo::stdin("encoded.html"),
        &HtmlOptions::default(),
    );
    assert_eq!(decoded.status, OperationStatus::Complete);
    assert_eq!(
        decoded.payload.as_ref().unwrap().decoded_text,
        r#"<!doctype html><meta charset="windows-1252"><p>café</p>"#
    );

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nodes = Some(1);
    let request = ParseRequest::new(
        RequestId::new("html-budget").unwrap(),
        Input::bytes(RICH.as_bytes().to_vec()),
        SourceInfo::stdin("budget.html"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let failed = registry.dispatch("html", request, None).unwrap();
    assert_eq!(failed.status, OperationStatus::Failed);
}

#[test]
fn active_content_and_remote_references_remain_inert() {
    let source = r#"<script src="https://network.invalid/a.js">fetch('/never')</script><iframe src="https://network.invalid/frame"></iframe><form action="https://network.invalid/post"><input formaction="javascript:never()"></form>"#;
    let envelope = parse_html(
        source,
        SourceInfo::stdin("hostile.html"),
        &HtmlOptions {
            mode: HtmlParseMode::Fragment,
            ..Default::default()
        },
    );
    let payload = envelope.payload.unwrap();
    assert!(payload.decoded_text.contains("fetch('/never')"));
    assert!(payload.links.iter().any(|link| link.remote));
    assert!(payload.links.iter().any(|link| link.active_scheme));
    assert!(payload.active_content.iter().any(|item| {
        item.source.contains("javascript:never()")
            && item.disposition == HtmlActiveContentDisposition::Inert
    }));
    assert!(payload.nodes.iter().any(|node| {
        node.kind == HtmlNodeKind::Text
            && node
                .text
                .as_deref()
                .is_some_and(|text| text.contains("fetch"))
    }));
}
