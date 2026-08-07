#![cfg(feature = "document-graph")]

use grist::core::{ContentIdentity, LineIndex, SourceRange, canonical_json_bytes, sha256_hex};
use grist::document_graph::{
    DocumentEdge, DocumentGraph, DocumentKind, DocumentNode, DocumentNodeKind, DocumentRelation,
};
use grist::segment::{
    SegmentNodeRole, SegmentOptions, SegmentSizeUnit, SegmentTokenizer, TokenizerSpec,
    segment_document_graph, segment_document_graph_parallel,
};
use serde_json::json;
use std::collections::BTreeMap;

fn identities(source: &str, graph: &DocumentGraph) -> (ContentIdentity, ContentIdentity) {
    (
        ContentIdentity::for_raw_bytes(source.as_bytes()).with_decoded(source, "utf-8", false),
        ContentIdentity::for_raw_bytes(&canonical_json_bytes(graph).unwrap()),
    )
}

fn text_node(
    source: &str,
    id: &str,
    kind: DocumentNodeKind,
    text: &str,
    start: usize,
    parent: &str,
    ordinal: usize,
) -> DocumentNode {
    DocumentNode::new(id, kind)
        .with_text(text)
        .with_range(SourceRange::new(
            start,
            start + text.len(),
            &LineIndex::new(source),
        ))
        .with_parent(parent)
        .with_ordinal(ordinal)
}

fn structural_fixture() -> (String, DocumentGraph, BTreeMap<String, String>) {
    let source = "Intro alpha beta gamma ".to_string();
    let mut graph = DocumentGraph::new("graph:segments", DocumentKind::Document);
    graph.attrs.insert("title".to_string(), json!("Fixture"));
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document).with_ordinal(0));
    graph.add_node(
        text_node(
            &source,
            "heading",
            DocumentNodeKind::Heading,
            "Intro ",
            0,
            "root",
            0,
        )
        .with_attr("level", 1)
        .with_attr("audience", "public"),
    );
    graph.add_node(
        text_node(
            &source,
            "p1",
            DocumentNodeKind::Paragraph,
            "alpha ",
            6,
            "root",
            1,
        )
        .with_attr("audience", "public")
        .with_attr("topic", "a"),
    );
    graph.add_node(
        text_node(
            &source,
            "p2",
            DocumentNodeKind::Paragraph,
            "beta ",
            12,
            "root",
            2,
        )
        .with_attr("audience", "public")
        .with_attr("topic", "b"),
    );
    graph.add_node(
        text_node(
            &source,
            "p3",
            DocumentNodeKind::Paragraph,
            "gamma ",
            17,
            "root",
            3,
        )
        .with_attr("audience", "private")
        .with_attr("topic", "c"),
    );
    let texts = [
        ("heading", "Intro "),
        ("p1", "alpha "),
        ("p2", "beta "),
        ("p3", "gamma "),
    ]
    .into_iter()
    .map(|(id, text)| (id.to_string(), text.to_string()))
    .collect();
    (source, graph, texts)
}

#[test]
fn serial_parallel_overlap_ancestry_and_span_provenance_are_identical() {
    let (source, graph, texts) = structural_fixture();
    let (source_identity, document_identity) = identities(&source, &graph);
    let mut options = SegmentOptions {
        target_size: 11,
        maximum_size: 24,
        overlap_source_nodes: 1,
        project_metadata: vec!["topic".to_string()],
        ..SegmentOptions::default()
    };
    options
        .metadata
        .insert("pipeline".to_string(), "test".to_string());

    let serial =
        segment_document_graph(&graph, &source_identity, &document_identity, &options, None)
            .unwrap();
    let parallel = segment_document_graph_parallel(
        &graph,
        &source_identity,
        &document_identity,
        &options,
        None,
        4,
    )
    .unwrap();
    assert_eq!(
        canonical_json_bytes(&serial).unwrap(),
        canonical_json_bytes(&parallel).unwrap()
    );
    assert_eq!(serial.segments.len(), 3);
    assert!(serial.options_digest.starts_with("sha256:"));

    for segment in &serial.segments {
        let mut cursor = 0;
        assert_eq!(segment.node_ids.len(), segment.locators.len());
        assert_eq!(segment.node_ids.len(), segment.node_references.len());
        for reference in &segment.node_references {
            assert_eq!(reference.segment_byte_start, cursor);
            assert_eq!(
                &segment.text[reference.segment_byte_start..reference.segment_byte_end],
                texts[&reference.node_id]
            );
            cursor = reference.segment_byte_end;
        }
        assert_eq!(cursor, segment.text.len());
        assert_eq!(segment.counts.bytes, segment.text.len());
        assert_eq!(segment.counts.unicode_scalars, segment.text.chars().count());
        assert_eq!(segment.counts.tokens, segment.token_count);
        assert!(segment.id.starts_with("grist:segment:"));
        assert!(segment.renderer_digest.starts_with("sha256:"));
    }

    let second = &serial.segments[1];
    assert_eq!(second.context.section_titles, vec!["Intro "]);
    assert_eq!(second.context.document_title.as_deref(), Some("Fixture"));
    assert_eq!(second.overlaps[0].segment_id, serial.segments[0].id);
    assert_eq!(second.overlaps[0].node_ids, vec!["p1"]);
    assert!(second.node_references.iter().any(|reference| {
        reference.node_id == "heading" && reference.role == SegmentNodeRole::Ancestry
    }));
    assert!(second.node_references.iter().any(|reference| {
        reference.node_id == "p1" && reference.role == SegmentNodeRole::Overlap
    }));
    assert_eq!(second.metadata["pipeline"], json!("test"));
    assert_eq!(
        second.metadata["source_attributes"]["topic"][0]["node_id"],
        json!("p1")
    );
}

fn atomic_fixture() -> (String, DocumentGraph) {
    let source = "rowcellcodebodyeqmathfigcaption".to_string();
    let mut graph = DocumentGraph::new("graph:atomic", DocumentKind::Document);
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document).with_ordinal(0));
    graph.add_node(
        DocumentNode::new("table", DocumentNodeKind::Table)
            .with_parent("root")
            .with_ordinal(0),
    );
    graph.add_node(text_node(
        &source,
        "row",
        DocumentNodeKind::TableRow,
        "row",
        0,
        "table",
        0,
    ));
    graph.add_node(text_node(
        &source,
        "cell",
        DocumentNodeKind::TableCell,
        "cell",
        3,
        "row",
        0,
    ));
    graph.add_node(text_node(
        &source,
        "code",
        DocumentNodeKind::CodeBlock,
        "code",
        7,
        "root",
        1,
    ));
    graph.add_node(text_node(
        &source,
        "code-body",
        DocumentNodeKind::Text,
        "body",
        11,
        "code",
        0,
    ));
    graph.add_node(text_node(
        &source,
        "equation",
        DocumentNodeKind::Equation,
        "eq",
        15,
        "root",
        2,
    ));
    graph.add_node(text_node(
        &source,
        "math",
        DocumentNodeKind::Text,
        "math",
        17,
        "equation",
        0,
    ));
    graph.add_node(text_node(
        &source,
        "figure",
        DocumentNodeKind::Figure,
        "fig",
        21,
        "root",
        3,
    ));
    graph.add_node(text_node(
        &source,
        "caption",
        DocumentNodeKind::Caption,
        "caption",
        24,
        "root",
        4,
    ));
    graph.add_edge(DocumentEdge::new(
        "caption",
        DocumentRelation::CaptionFor,
        "figure",
    ));
    (source, graph)
}

#[test]
fn table_code_equation_and_caption_groups_remain_atomic() {
    let (source, graph) = atomic_fixture();
    let (source_identity, document_identity) = identities(&source, &graph);
    let options = SegmentOptions {
        target_size: 1,
        maximum_size: 4,
        include_heading_ancestry: false,
        ..SegmentOptions::default()
    };
    let result =
        segment_document_graph(&graph, &source_identity, &document_identity, &options, None)
            .unwrap();
    for expected in [
        &["row", "cell"][..],
        &["code", "code-body"][..],
        &["equation", "math"][..],
        &["figure", "caption"][..],
    ] {
        assert!(result.segments.iter().any(|segment| {
            expected
                .iter()
                .all(|id| segment.node_ids.iter().any(|node| node == id))
        }));
    }
    assert!(result.segments.iter().all(|segment| {
        segment
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "segment.maximum_exceeded_for_atomic_source")
    }));

    let mut split_options = options;
    split_options.atomicity.tables = false;
    split_options.atomicity.code = false;
    split_options.atomicity.equations = false;
    split_options.atomicity.figure_captions = false;
    let split = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &split_options,
        None,
    )
    .unwrap();
    for pair in [
        ["row", "cell"],
        ["code", "code-body"],
        ["equation", "math"],
        ["figure", "caption"],
    ] {
        assert!(!split.segments.iter().any(|segment| {
            pair.iter()
                .all(|id| segment.node_ids.iter().any(|node| node == id))
        }));
    }
}

#[derive(Debug)]
struct PipeTokenizer;

impl SegmentTokenizer for PipeTokenizer {
    fn specification(&self) -> TokenizerSpec {
        TokenizerSpec {
            name: "fixture.pipe".to_string(),
            version: "7".to_string(),
            configuration_digest: sha256_hex(b"pipe-tokenizer-v7"),
        }
    }

    fn count_tokens(&self, text: &str) -> Result<usize, String> {
        Ok(text.split('|').filter(|token| !token.is_empty()).count())
    }
}

#[test]
fn size_units_and_configured_tokenizer_are_recorded_and_enforced() {
    let source = "a|b".to_string();
    let mut graph = DocumentGraph::new("graph:tokenizer", DocumentKind::Document);
    graph.add_node(text_node(
        &source,
        "value",
        DocumentNodeKind::Paragraph,
        &source,
        0,
        "root",
        0,
    ));
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document).with_ordinal(0));
    let (source_identity, document_identity) = identities(&source, &graph);
    let tokenizer = PipeTokenizer;
    let options = SegmentOptions {
        target_size: 2,
        maximum_size: 3,
        size_unit: SegmentSizeUnit::Tokens,
        tokenizer: Some(tokenizer.specification()),
        include_heading_ancestry: false,
        ..SegmentOptions::default()
    };
    let token_result = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &options,
        Some(&tokenizer),
    )
    .unwrap();
    assert_eq!(token_result.segments[0].size, 2);
    assert_eq!(token_result.segments[0].token_count, Some(2));
    assert_eq!(
        token_result.segments[0].tokenizer,
        Some(tokenizer.specification())
    );
    assert!(
        segment_document_graph(&graph, &source_identity, &document_identity, &options, None,)
            .is_err()
    );

    let unicode_source = "\u{00e9}".to_string();
    let mut unicode_graph = DocumentGraph::new("graph:unicode", DocumentKind::Document);
    unicode_graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document).with_ordinal(0));
    unicode_graph.add_node(text_node(
        &unicode_source,
        "unicode",
        DocumentNodeKind::Paragraph,
        &unicode_source,
        0,
        "root",
        0,
    ));
    let (source_identity, document_identity) = identities(&unicode_source, &unicode_graph);
    let byte_options = SegmentOptions {
        target_size: 1,
        maximum_size: 4,
        size_unit: SegmentSizeUnit::Bytes,
        include_heading_ancestry: false,
        ..SegmentOptions::default()
    };
    let byte_result = segment_document_graph(
        &unicode_graph,
        &source_identity,
        &document_identity,
        &byte_options,
        None,
    )
    .unwrap();
    assert_eq!(byte_result.segments[0].size, 2);
    assert_eq!(byte_result.segments[0].counts.unicode_scalars, 1);
    assert_eq!(byte_result.segments[0].counts.bytes, 2);
}

#[test]
fn inclusion_metadata_rules_and_untraceable_loss_are_explicit() {
    let (source, mut graph, _) = structural_fixture();
    graph.add_node(
        DocumentNode::new("missing-locator", DocumentNodeKind::Paragraph)
            .with_text("lost")
            .with_parent("root")
            .with_ordinal(4)
            .with_attr("audience", "public")
            .with_attr("topic", "lost"),
    );
    let (source_identity, document_identity) = identities(&source, &graph);
    let mut options = SegmentOptions {
        target_size: 100,
        maximum_size: 200,
        include_heading_ancestry: false,
        project_metadata: vec!["topic".to_string()],
        ..SegmentOptions::default()
    };
    options.selection.include_kinds = vec!["paragraph".to_string()];
    options
        .selection
        .required_metadata
        .insert("audience".to_string(), "public".to_string());
    let result =
        segment_document_graph(&graph, &source_identity, &document_identity, &options, None)
            .unwrap();
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments[0].node_ids, vec!["p1", "p2"]);
    assert_eq!(result.segments[0].text, "alpha beta ");
    assert_eq!(
        result.segments[0].metadata["source_attributes"]["topic"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "segment.untraceable_node_omitted"
            && diagnostic.partial
            && diagnostic.affected_ids == vec!["missing-locator"]
    }));

    let mut excluded = options;
    excluded.selection.exclude_kinds = vec!["paragraph".to_string()];
    let excluded_result = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &excluded,
        None,
    )
    .unwrap();
    assert!(excluded_result.segments.is_empty());
    assert!(excluded_result.diagnostics.is_empty());
}
