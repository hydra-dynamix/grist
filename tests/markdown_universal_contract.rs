#![cfg(all(feature = "markdown", feature = "schemas"))]

use grist::core::{ContentIdentity, OperationStatus, SchemaVersion, SourceInfo};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::markdown::{
    FrontmatterKind, MarkdownDocument, MarkdownNodeKind, MarkdownOptions, parse_markdown,
    parse_markdown_bytes,
};
use grist::render::{FidelityMode, RenderFormat, RenderOptions, render_document_graph};
use grist::segment::{SegmentOptions, segment_document_graph};

const RICH: &str = "---\ntitle: Universal\n---\n# Heading *em* {#heading .wide}\n\nParagraph with **strong**, ~~strike~~, `code`, $math$, [link](https://example.test), and ![alt](image.png).\n\n1. first\n2. second\n\n- [x] done\n- [ ] open\n\n> quoted\n\n| name | score |\n| :--- | ---: |\n| alpha | 1 |\n\nFootnote[^note].\n\n[^note]: exact note\n\nTerm\n: definition\n\n---\n";

#[test]
fn authoritative_payload_accounts_for_every_required_markdown_construct() {
    let report = parse_markdown(RICH, SourceInfo::stdin("rich.md"));
    assert_eq!(report.status, OperationStatus::Complete);
    assert_eq!(report.payload_schema_version.0, SchemaVersion::MARKDOWN_V2);
    let payload = report.payload.unwrap();
    assert_eq!(payload.decoded_text, RICH);
    assert_eq!(payload.raw_bytes, RICH.as_bytes());
    assert_eq!(
        payload.frontmatter.as_ref().unwrap().kind,
        FrontmatterKind::Yaml
    );
    for kind in [
        MarkdownNodeKind::Heading,
        MarkdownNodeKind::Paragraph,
        MarkdownNodeKind::Emphasis,
        MarkdownNodeKind::Strong,
        MarkdownNodeKind::Strikethrough,
        MarkdownNodeKind::InlineCode,
        MarkdownNodeKind::InlineMath,
        MarkdownNodeKind::Link,
        MarkdownNodeKind::Image,
        MarkdownNodeKind::List,
        MarkdownNodeKind::ListItem,
        MarkdownNodeKind::TaskListMarker,
        MarkdownNodeKind::BlockQuote,
        MarkdownNodeKind::Table,
        MarkdownNodeKind::TableRow,
        MarkdownNodeKind::TableCell,
        MarkdownNodeKind::FootnoteReference,
        MarkdownNodeKind::FootnoteDefinition,
        MarkdownNodeKind::DefinitionList,
        MarkdownNodeKind::ThematicBreak,
    ] {
        assert!(
            payload.nodes.iter().any(|node| node.kind == kind),
            "{kind:?}"
        );
    }
    for node in &payload.nodes {
        let range = node.range.as_ref().unwrap();
        assert_eq!(node.raw, RICH[range.byte_start..range.byte_end]);
        assert!(node.locator.is_some());
        assert!(node.raw_range.is_some());
    }
}

#[test]
fn malformed_encoding_fences_frontmatter_and_directives_are_explicit_partial_results() {
    let source = b"---\ntitle: [bad\n---\n::: warning\n```rust\nbad \xff";
    let report = parse_markdown_bytes(
        source,
        SourceInfo::stdin("malformed.md"),
        &MarkdownOptions {
            encoding: Some("utf-8".into()),
            ..MarkdownOptions::default()
        },
    );
    assert_eq!(report.status, OperationStatus::Partial);
    let payload = report.payload.unwrap();
    assert_eq!(payload.raw_bytes, source);
    for code in [
        "decode.replacement.undecodable",
        "frontmatter.parse",
        "fence.unclosed",
        "directive.unclosed",
    ] {
        assert!(
            report.diagnostics.iter().any(|value| value.code == code),
            "{code}"
        );
    }
    assert!(payload.nodes.iter().any(|node| {
        node.kind == MarkdownNodeKind::DirectiveBlock && node.raw.contains("::: warning")
    }));
}

#[test]
fn raw_html_directives_and_unknown_inline_extensions_survive_graph_projection() {
    let source = "<script>never()</script>\n\n::: warning\nbody\n:::\n\n[[Wiki Page]] and {{< card title=demo >}}\n";
    let parsed = parse_markdown(source, SourceInfo::stdin("extensions.md"));
    let payload = parsed.payload.unwrap();
    for kind in [
        MarkdownNodeKind::HtmlBlock,
        MarkdownNodeKind::DirectiveBlock,
        MarkdownNodeKind::ExtensionInline,
    ] {
        assert!(
            payload.nodes.iter().any(|node| node.kind == kind),
            "{kind:?}"
        );
    }
    let graph = payload
        .to_document_graph(DocumentGraphContext::new("markdown:raw"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.kind,
                    DocumentNodeKind::RawBlock | DocumentNodeKind::RawInline
                )
            })
            .all(|node| node.raw.is_some() && node.locator.is_some())
    );
    let rendered = render_document_graph(
        &graph,
        RenderFormat::Markdown,
        &RenderOptions::new(FidelityMode::RawFallback),
    )
    .unwrap();
    rendered.validate_source_map().unwrap();
    assert!(rendered.content.contains("GRIST RAW"));
    assert!(
        rendered
            .content
            .contains("```text\n<script>never()</script>")
    );
}

#[test]
fn graph_segments_and_normalized_render_are_deterministic_and_located() {
    let parsed = parse_markdown(RICH, SourceInfo::stdin("rich.md"));
    let payload = parsed.payload.as_ref().unwrap();
    let context =
        DocumentGraphContext::new("markdown:deterministic").with_source(parsed.source.clone());
    let first_graph = payload.to_document_graph(context.clone()).unwrap();
    let second_graph = payload.to_document_graph(context).unwrap();
    assert_eq!(first_graph, second_graph);
    assert!(
        first_graph
            .nodes
            .iter()
            .filter(|node| { node.kind != DocumentNodeKind::Document })
            .all(|node| node.locator.is_some())
    );
    assert!(first_graph.edges.iter().any(|edge| {
        edge.relation == DocumentRelation::LinksTo && edge.target == "https://example.test"
    }));
    assert!(
        first_graph
            .edges
            .iter()
            .any(|edge| { edge.relation == DocumentRelation::References })
    );

    let source_identity = parsed.identity.as_ref().unwrap();
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(first_graph.schema_version.as_str(), &first_graph)
        .unwrap();
    let first_segments = segment_document_graph(
        &first_graph,
        source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    let second_segments = segment_document_graph(
        &second_graph,
        source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(first_segments, second_segments);
    assert!(
        first_segments
            .segments
            .iter()
            .all(|segment| { !segment.node_ids.is_empty() && !segment.locators.is_empty() })
    );

    let first_render = render_document_graph(
        &first_graph,
        RenderFormat::Markdown,
        &RenderOptions::default(),
    )
    .unwrap();
    let second_render = render_document_graph(
        &second_graph,
        RenderFormat::Markdown,
        &RenderOptions::default(),
    )
    .unwrap();
    assert_eq!(first_render, second_render);
    first_render.validate_source_map().unwrap();
    for expected in [
        "# Heading",
        "~~strike~~",
        "![alt](image.png)",
        "- [x] done",
        "[^note]",
    ] {
        assert!(
            first_render.content.contains(expected),
            "{expected}: {}",
            first_render.content
        );
    }
}

#[test]
fn schema_and_rust_payload_agree_on_v2() {
    let report = parse_markdown(RICH, SourceInfo::stdin("schema.md"));
    let payload_value = serde_json::to_value(report.payload.as_ref().unwrap()).unwrap();
    let _: MarkdownDocument = serde_json::from_value(payload_value.clone()).unwrap();
    for (name, value) in [
        ("markdown", payload_value),
        ("markdown-envelope", serde_json::to_value(&report).unwrap()),
        (
            "markdown-options",
            serde_json::to_value(MarkdownOptions::default()).unwrap(),
        ),
    ] {
        let validation = grist::schema::validate_schema(name, &value).unwrap();
        assert!(validation.valid, "{name}: {:?}", validation.issues);
    }
}
