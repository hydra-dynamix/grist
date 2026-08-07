use grist::core::{LineIndex, SourceRange, canonical_json_bytes};
use grist::document_graph::{
    DocumentGraph, DocumentKind, DocumentNode, DocumentNodeKind, RawNodeContent,
};
use grist::render::{
    FidelityMode, ReconstructionClaim, RenderError, RenderFormat, RenderOptions,
    SourceMapLocatorStatus, render_document_graph,
};
use serde_json::json;

fn located_graph() -> DocumentGraph {
    let source = "Title\n<script>alert(1)</script>\nclick\n";
    let index = LineIndex::new(source);
    let mut graph = DocumentGraph::new("render:test", DocumentKind::Document);
    graph.add_node(
        DocumentNode::new("root", DocumentNodeKind::Document).with_range(SourceRange::new(
            0,
            source.len(),
            &index,
        )),
    );
    graph.add_node(
        DocumentNode::new("heading", DocumentNodeKind::Heading)
            .with_text("Title")
            .with_attr("level", 1)
            .with_range(SourceRange::new(0, 5, &index))
            .with_ordinal(0),
    );
    graph.add_node(
        DocumentNode::new("paragraph", DocumentNodeKind::Paragraph)
            .with_text("<script>alert(1)</script>")
            .with_range(SourceRange::new(6, 31, &index))
            .with_ordinal(1),
    );
    graph.add_node(
        DocumentNode::new("link", DocumentNodeKind::Link)
            .with_text("click")
            .with_attr("destination", "java\nscript:alert(1)")
            .with_range(SourceRange::new(32, 37, &index))
            .with_ordinal(2),
    );
    graph.add_contains("root", "heading");
    graph.add_contains("root", "paragraph");
    graph.add_contains("root", "link");
    graph
}

#[test]
fn every_target_has_gap_free_utf8_node_and_locator_source_maps() {
    let graph = located_graph();
    for format in [
        RenderFormat::Markdown,
        RenderFormat::Latex,
        RenderFormat::Html,
        RenderFormat::PlainText,
        RenderFormat::CanonicalJson,
    ] {
        let result = render_document_graph(&graph, format, &RenderOptions::default()).unwrap();
        result.validate_source_map().unwrap();
        assert_eq!(result.source_map.generated_length, result.content.len());
        assert_eq!(
            result.fidelity.reconstruction_claim,
            ReconstructionClaim::NormalizedNotByteRoundTrip
        );
        assert!(result.fidelity.losses.is_empty());
        assert!(result.source_map.entries.iter().all(|entry| {
            entry.original_locator.is_some()
                && entry.locator_status == SourceMapLocatorStatus::Exact
        }));
        let mapped_ids = result
            .source_map
            .entries
            .iter()
            .map(|entry| entry.node_id.as_str())
            .collect::<Vec<_>>();
        assert!(mapped_ids.contains(&"heading"));
        assert!(mapped_ids.contains(&"paragraph"));
        assert!(mapped_ids.contains(&"link"));
    }
}

#[test]
fn html_is_safe_by_default_and_normalized_targets_block_active_uris() {
    let graph = located_graph();
    let html = render_document_graph(&graph, RenderFormat::Html, &RenderOptions::default())
        .unwrap()
        .content;
    assert!(!html.contains("<script>"));
    assert!(!html.to_ascii_lowercase().contains("javascript:"));
    assert!(html.contains("#grist-blocked-active-uri"));
    assert!(html.contains("&lt;script&gt;"));

    let markdown = render_document_graph(&graph, RenderFormat::Markdown, &RenderOptions::default())
        .unwrap()
        .content;
    assert!(!markdown.contains("<script>"));
    assert!(!markdown.to_ascii_lowercase().contains("javascript:"));

    let latex = render_document_graph(&graph, RenderFormat::Latex, &RenderOptions::default())
        .unwrap()
        .content;
    assert!(!latex.to_ascii_lowercase().contains("javascript:"));
}

fn graph_with_raw_nodes() -> DocumentGraph {
    let mut graph = DocumentGraph::new("render:raw", DocumentKind::Document);
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document));
    for (ordinal, id, raw) in [
        (0, "raw-a", "<script>first()</script>"),
        (1, "raw-b", "\\immediate\\write18{owned}"),
    ] {
        graph.add_node(
            DocumentNode::new(id, DocumentNodeKind::RawBlock)
                .with_text(raw)
                .with_ordinal(ordinal)
                .with_raw(
                    RawNodeContent::new("grist.test", "test_raw", json!({ "raw": raw })).unwrap(),
                ),
        );
        graph.add_contains("root", id);
    }
    graph
}

#[test]
fn strict_rejects_raw_nodes_and_raw_fallback_preserves_marked_inert_syntax() {
    let graph = graph_with_raw_nodes();
    let error = render_document_graph(
        &graph,
        RenderFormat::Html,
        &RenderOptions::new(FidelityMode::Strict),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RenderError::UnsupportedNode { node_id, .. } if node_id == "raw-a"
    ));

    for format in [
        RenderFormat::Markdown,
        RenderFormat::Latex,
        RenderFormat::Html,
        RenderFormat::PlainText,
    ] {
        let result = render_document_graph(
            &graph,
            format,
            &RenderOptions::new(FidelityMode::RawFallback),
        )
        .unwrap();
        result.validate_source_map().unwrap();
        assert!(result.content.contains("GRIST RAW"));
        assert_eq!(result.diagnostics.len(), 2);
        assert!(result.fidelity.losses.is_empty());
        if format == RenderFormat::Html {
            assert!(!result.content.contains("<script>"));
            assert!(result.content.contains("&lt;script&gt;"));
        }
        if format == RenderFormat::Latex {
            assert!(!result.content.contains("\\immediate\\write18"));
            assert!(result.content.contains("textbackslash"));
        }
    }
}

#[test]
fn lossy_mode_enumerates_every_affected_node() {
    let graph = graph_with_raw_nodes();
    let result = render_document_graph(
        &graph,
        RenderFormat::Markdown,
        &RenderOptions::new(FidelityMode::Lossy),
    )
    .unwrap();
    let loss_ids = result
        .fidelity
        .losses
        .iter()
        .map(|loss| loss.node_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(loss_ids, ["raw-a", "raw-b"]);
    assert_eq!(result.diagnostics.len(), 2);
    assert_eq!(
        result
            .diagnostics
            .iter()
            .flat_map(|diagnostic| diagnostic.affected_ids.iter().map(String::as_str))
            .collect::<Vec<_>>(),
        ["raw-a", "raw-b"]
    );
    assert!(!result.content.contains("first()"));
}

#[test]
fn raw_fallback_requires_retained_source_syntax() {
    let mut graph = DocumentGraph::new("render:no-raw", DocumentKind::Document);
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document));
    graph.add_node(DocumentNode::new("function", DocumentNodeKind::Function));
    graph.add_contains("root", "function");
    let error = render_document_graph(
        &graph,
        RenderFormat::PlainText,
        &RenderOptions::new(FidelityMode::RawFallback),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RenderError::RawSourceUnavailable { node_id, .. } if node_id == "function"
    ));
}

#[test]
fn canonical_json_is_byte_stable_and_preserves_raw_graph_data() {
    let graph = graph_with_raw_nodes();
    let options = RenderOptions::new(FidelityMode::Strict);
    let first = render_document_graph(&graph, RenderFormat::CanonicalJson, &options).unwrap();
    let second = render_document_graph(&graph, RenderFormat::CanonicalJson, &options).unwrap();
    assert_eq!(first.content, second.content);
    assert_eq!(
        first.content.as_bytes(),
        canonical_json_bytes(&graph).unwrap()
    );
    assert!(first.content.contains("first()"));
    assert!(first.fidelity.losses.is_empty());
    first.validate_source_map().unwrap();
}

#[test]
fn missing_original_locator_is_explicit_not_silent() {
    let mut graph = DocumentGraph::new("render:unlocated", DocumentKind::Document);
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document));
    graph.add_node(
        DocumentNode::new("paragraph", DocumentNodeKind::Paragraph).with_text("unlocated"),
    );
    graph.add_contains("root", "paragraph");
    let result =
        render_document_graph(&graph, RenderFormat::PlainText, &RenderOptions::default()).unwrap();
    assert!(result.source_map.entries.iter().all(|entry| {
        entry.locator_status == SourceMapLocatorStatus::Unavailable
            && entry.original_locator.is_none()
            && entry.locator_unavailable_reason.is_some()
    }));
}
