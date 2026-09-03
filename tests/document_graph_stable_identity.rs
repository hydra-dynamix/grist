#![cfg(feature = "document-graph")]

use grist::core::{
    LineIndex, ParserInfo, SchemaVersion, SourceLocator, SourceRange, canonical_json_bytes,
};
use grist::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphFragment, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, GraphProjectionError, ProjectionAddress,
};
use std::collections::BTreeSet;

fn generator(source: &str, schema: &str, parser: &str) -> GraphIdGenerator {
    GraphIdGenerator::new(source, schema, ParserInfo::new(parser)).unwrap()
}

fn locator(text: &str, start: usize, end: usize) -> SourceLocator {
    SourceLocator::try_from(SourceRange::new(start, end, &LineIndex::new(text))).unwrap()
}

#[test]
fn native_identity_is_stable_across_unrelated_locator_edits() {
    let ids = generator("source:stable", "payload/v1", "fixture");
    let before = ProjectionAddress::native(["slides", "deck"], "shape-42")
        .with_locator(locator("alpha", 0, 5));
    let after = ProjectionAddress::native(["slides", "deck"], "shape-42").with_locator(locator(
        "unrelated prefix alpha",
        17,
        22,
    ));

    assert_eq!(ids.node_id(&before).unwrap(), ids.node_id(&after).unwrap());
    assert_eq!(
        ids.edge_id(
            &before,
            "shape-42",
            &DocumentRelation::CaptionFor,
            "shape-43",
        )
        .unwrap(),
        ids.edge_id(
            &after,
            "shape-42",
            &DocumentRelation::CaptionFor,
            "shape-43",
        )
        .unwrap()
    );
}

#[test]
fn property_identity_material_does_not_collide_over_generated_cases() {
    let base = ProjectionAddress::native(["document", "section"], "native-7");
    let mut observed = BTreeSet::new();
    for source in ["source:a", "source:b"] {
        for schema in ["payload/v1", "payload/v2"] {
            for parser in ["parser-a", "parser-b"] {
                let ids = generator(source, schema, parser);
                assert!(observed.insert(ids.node_id(&base).unwrap()));
            }
        }
    }

    let ids = generator("source:a", "payload/v1", "parser-a");
    for index in 0..512 {
        let address = ProjectionAddress::native(
            ["document", "generated", &index.to_string()],
            format!("native-{index}"),
        );
        assert!(observed.insert(ids.node_id(&address).unwrap()));
    }
    let parser_v1 = GraphIdGenerator::new(
        "source:a",
        "payload/v1",
        ParserInfo::new("parser-a").with_implementation("backend", "1"),
    )
    .unwrap();
    let parser_v2 = GraphIdGenerator::new(
        "source:a",
        "payload/v1",
        ParserInfo::new("parser-a").with_implementation("backend", "2"),
    )
    .unwrap();
    assert_ne!(
        parser_v1.node_id(&base).unwrap(),
        parser_v2.node_id(&base).unwrap()
    );
    let node_id = ids.node_id(&base).unwrap();
    let edge_id = ids
        .edge_id(
            &ProjectionAddress::native(["relations"], "native-7"),
            "left",
            &DocumentRelation::References,
            "right",
        )
        .unwrap();
    assert_ne!(node_id, edge_id);
}

#[test]
fn locator_fallback_changes_when_source_coordinates_move() {
    let ids = generator("source:stable", "payload/v1", "fixture");
    let first = ProjectionAddress::located(["document", "paragraph"], locator("one two", 0, 3));
    let second =
        ProjectionAddress::located(["document", "paragraph"], locator("zero one two", 5, 8));
    assert_ne!(ids.node_id(&first).unwrap(), ids.node_id(&second).unwrap());
}

#[test]
fn canonical_node_order_uses_numeric_source_coordinates() {
    let text = "0123456789x";
    let index = LineIndex::new(text);
    let mut graph = DocumentGraph::new("numeric-order", DocumentKind::Document);
    graph.add_node(
        DocumentNode::new("later", DocumentNodeKind::Paragraph)
            .with_range(SourceRange::new(10, 11, &index)),
    );
    graph.add_node(
        DocumentNode::new("earlier", DocumentNodeKind::Paragraph)
            .with_range(SourceRange::new(2, 3, &index)),
    );
    graph.canonicalize().unwrap();
    assert_eq!(graph.nodes[0].id, "earlier");
}

fn fragment(ids: &GraphIdGenerator, ordinal: usize) -> DocumentGraphFragment {
    let range = SourceRange::new(
        ordinal * 2,
        ordinal * 2 + 1,
        &LineIndex::new("a b c d e f g"),
    );
    let id = ids
        .node_id(&ProjectionAddress::located(
            ["document", "paragraphs", &ordinal.to_string()],
            SourceLocator::try_from(range.clone()).unwrap(),
        ))
        .unwrap();
    let root = ids
        .node_id(&ProjectionAddress::native(["document"], "root"))
        .unwrap();
    let node = DocumentNode::new(&id, DocumentNodeKind::Paragraph)
        .with_range(range.clone())
        .with_parent(&root)
        .with_ordinal(ordinal);
    let edge = DocumentEdge::new(root, DocumentRelation::Contains, id).with_range(range);
    DocumentGraphFragment::new().with_node(node).with_edge(edge)
}

fn merged_bytes(order: &[usize]) -> Vec<u8> {
    let ids = generator(
        "source:parallel",
        SchemaVersion::DOCUMENT_GRAPH_V2,
        "fixture",
    );
    let root = ids
        .node_id(&ProjectionAddress::native(["document"], "root"))
        .unwrap();
    let mut graph = DocumentGraph::new("parallel", DocumentKind::Document);
    graph.add_node(DocumentNode::new(root, DocumentNodeKind::Document).with_ordinal(0));
    graph
        .merge_parallel(order.iter().map(|ordinal| fragment(&ids, *ordinal)))
        .unwrap();
    graph.finalize_projection(&ids).unwrap();
    canonical_json_bytes(&graph).unwrap()
}

fn permutations(values: &mut [usize], start: usize, output: &mut Vec<Vec<usize>>) {
    if start == values.len() {
        output.push(values.to_vec());
        return;
    }
    for index in start..values.len() {
        values.swap(start, index);
        permutations(values, start + 1, output);
        values.swap(start, index);
    }
}

#[test]
fn property_all_fragment_permutations_have_identical_canonical_json() {
    let mut values = [0, 1, 2, 3, 4];
    let mut orders = Vec::new();
    permutations(&mut values, 0, &mut orders);
    let expected = merged_bytes(&orders[0]);
    for order in &orders[1..] {
        assert_eq!(merged_bytes(order), expected);
    }
}

#[test]
fn parallel_merge_rejects_identity_collisions() {
    let ids = generator("source:collision", "payload/v1", "fixture");
    let duplicate = fragment(&ids, 1);
    let mut graph = DocumentGraph::new("collision", DocumentKind::Document);
    let error = graph
        .merge_parallel([duplicate.clone(), duplicate])
        .unwrap_err();
    assert!(matches!(error, GraphProjectionError::DuplicateNodeId(_)));
}
