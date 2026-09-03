#![cfg(feature = "document-graph")]

use grist::core::{LineIndex, LocatorConfidence, SchemaVersion, SourceRange};
use grist::document_graph::{
    DocumentEdge, DocumentGraph, DocumentKind, DocumentNode, DocumentNodeKind, DocumentRelation,
    InferenceMetadata, RawNodeContent, RelationEvidence,
};
use serde_json::json;

#[cfg(feature = "markdown")]
use grist::document_graph::{DocumentGraphContext, ToDocumentGraph};

#[test]
fn full_cross_format_vocabulary_round_trips() {
    let node_kinds = vec![
        DocumentNodeKind::Document,
        DocumentNodeKind::Container,
        DocumentNodeKind::ArchiveMember,
        DocumentNodeKind::Attachment,
        DocumentNodeKind::Metadata,
        DocumentNodeKind::Page,
        DocumentNodeKind::Slide,
        DocumentNodeKind::Sheet,
        DocumentNodeKind::Section,
        DocumentNodeKind::Heading,
        DocumentNodeKind::Header,
        DocumentNodeKind::Footer,
        DocumentNodeKind::Paragraph,
        DocumentNodeKind::TextRun,
        DocumentNodeKind::Span,
        DocumentNodeKind::List,
        DocumentNodeKind::ListItem,
        DocumentNodeKind::Quote,
        DocumentNodeKind::CodeBlock,
        DocumentNodeKind::CodeSymbol,
        DocumentNodeKind::Import,
        DocumentNodeKind::Export,
        DocumentNodeKind::Call,
        DocumentNodeKind::Branch,
        DocumentNodeKind::Assignment,
        DocumentNodeKind::Return,
        DocumentNodeKind::Math,
        DocumentNodeKind::Equation,
        DocumentNodeKind::Label,
        DocumentNodeKind::Reference,
        DocumentNodeKind::Citation,
        DocumentNodeKind::BibliographyEntry,
        DocumentNodeKind::Table,
        DocumentNodeKind::Row,
        DocumentNodeKind::Cell,
        DocumentNodeKind::Chart,
        DocumentNodeKind::Figure,
        DocumentNodeKind::Image,
        DocumentNodeKind::Caption,
        DocumentNodeKind::Footnote,
        DocumentNodeKind::Endnote,
        DocumentNodeKind::Annotation,
        DocumentNodeKind::Comment,
        DocumentNodeKind::Revision,
        DocumentNodeKind::Bookmark,
        DocumentNodeKind::Link,
        DocumentNodeKind::Form,
        DocumentNodeKind::FormField,
        DocumentNodeKind::ContentControl,
        DocumentNodeKind::Email,
        DocumentNodeKind::MessageBody,
        DocumentNodeKind::MimePart,
        DocumentNodeKind::Thread,
        DocumentNodeKind::Notebook,
        DocumentNodeKind::NotebookCell,
        DocumentNodeKind::CellOutput,
        DocumentNodeKind::Transcript,
        DocumentNodeKind::Cue,
        DocumentNodeKind::MediaTrack,
        DocumentNodeKind::StructuredValue,
        DocumentNodeKind::Record,
        DocumentNodeKind::Field,
        DocumentNodeKind::Raw,
        DocumentNodeKind::Unknown,
    ];
    let relations = vec![
        DocumentRelation::Contains,
        DocumentRelation::Precedes,
        DocumentRelation::ParentOf,
        DocumentRelation::References,
        DocumentRelation::ResolvesTo,
        DocumentRelation::Cites,
        DocumentRelation::CaptionFor,
        DocumentRelation::FootnoteFor,
        DocumentRelation::AnnotationFor,
        DocumentRelation::RevisionOf,
        DocumentRelation::AttachmentOf,
        DocumentRelation::EmbeddedIn,
        DocumentRelation::ReplyTo,
        DocumentRelation::Imports,
        DocumentRelation::Exports,
        DocumentRelation::Calls,
        DocumentRelation::Inherits,
        DocumentRelation::Assigns,
        DocumentRelation::Returns,
        DocumentRelation::FormulaDependsOn,
        DocumentRelation::DerivedFrom,
        DocumentRelation::SourceOf,
        DocumentRelation::EvidenceFor,
        DocumentRelation::AlternativeRepresentationOf,
        DocumentRelation::ReconciledWith,
    ];

    assert_eq!(
        serde_json::from_value::<Vec<DocumentNodeKind>>(serde_json::to_value(&node_kinds).unwrap())
            .unwrap(),
        node_kinds
    );
    assert_eq!(
        serde_json::from_value::<Vec<DocumentRelation>>(serde_json::to_value(&relations).unwrap())
            .unwrap(),
        relations
    );
}

#[test]
fn v1_graph_migrates_locators_extensions_and_raw_content_without_loss() {
    let legacy = json!({
        "schema_version": SchemaVersion::DOCUMENT_GRAPH_V1,
        "id": "legacy:latex",
        "kind": "latex",
        "source": null,
        "language": "latex",
        "dialect": null,
        "nodes": [{
            "id": "legacy:raw",
            "kind": "raw_block",
            "range": {
                "byte_start": 0, "byte_end": 9,
                "start_line": 1, "start_column": 1,
                "end_line": 1, "end_column": 10
            },
            "text": "\\mystery",
            "name": null,
            "qualified_name": null,
            "parent": null,
            "ordinal": 0,
            "attrs": {"command": "mystery", "starred": true}
        }],
        "edges": [{
            "source": "legacy:root",
            "relation": "contains",
            "target": "legacy:raw",
            "range": {
                "byte_start": 0, "byte_end": 9,
                "start_line": 1, "start_column": 1,
                "end_line": 1, "end_column": 10
            },
            "attrs": {"source_role": "command"}
        }],
        "diagnostics": [],
        "attrs": {"engine": "tex"}
    });

    let migrated = serde_json::from_value::<DocumentGraph>(legacy)
        .unwrap()
        .migrate_to_v2()
        .unwrap();
    migrated.validate_contract().unwrap();

    assert_eq!(migrated.schema_version, SchemaVersion::DOCUMENT_GRAPH_V2);
    assert!(migrated.nodes[0].locator.is_some());
    assert_eq!(migrated.nodes[0].text.as_deref(), Some("\\mystery"));
    assert_eq!(migrated.nodes[0].attrs["command"], "mystery");
    assert_eq!(migrated.nodes[0].extensions["grist.latex"]["starred"], true);
    assert_eq!(
        migrated.nodes[0].raw.as_ref().unwrap().payload["text"],
        "\\mystery"
    );
    assert!(matches!(
        migrated.edges[0].evidence,
        RelationEvidence::Explicit { .. }
    ));
    assert_eq!(
        migrated.edges[0].extensions["grist.latex"]["source_role"],
        "command"
    );
}

#[test]
fn explicit_and_inferred_edges_have_distinct_required_evidence() {
    let range = SourceRange::new(0, 4, &LineIndex::new("link"));
    let explicit =
        DocumentEdge::new("link", DocumentRelation::References, "target").with_range(range.clone());
    assert!(matches!(
        explicit.evidence,
        RelationEvidence::Explicit { .. }
    ));

    let inferred = DocumentEdge::inferred(
        "caption",
        DocumentRelation::CaptionFor,
        "figure",
        "grist.layout.nearest-caption.v1",
        LocatorConfidence::new(0.82).unwrap(),
    );
    let RelationEvidence::Inferred { inference } = &inferred.evidence else {
        panic!("inferred edge must retain inference metadata");
    };
    assert_eq!(inference.rule, "grist.layout.nearest-caption.v1");
    assert_eq!(inference.confidence.get(), 0.82);

    let mut graph = DocumentGraph::new("edge-contract", DocumentKind::Document);
    graph.add_edge(explicit);
    graph.add_edge(inferred);
    graph.validate_contract().unwrap();

    let invalid_confidence = json!({
        "origin": "inferred",
        "inference": {
            "rule": "grist.layout.nearest-caption.v1",
            "confidence": 1.01,
            "evidence_locators": []
        }
    });
    assert!(serde_json::from_value::<RelationEvidence>(invalid_confidence).is_err());

    let invalid_rule = RelationEvidence::Inferred {
        inference: InferenceMetadata {
            rule: String::new(),
            confidence: LocatorConfidence::new(0.5).unwrap(),
            evidence_locators: Vec::new(),
        },
    };
    let mut invalid_graph = DocumentGraph::new("invalid-edge", DocumentKind::Document);
    let mut invalid_edge = DocumentEdge::new("a", DocumentRelation::Precedes, "b");
    invalid_edge.evidence = invalid_rule;
    invalid_graph.add_edge(invalid_edge);
    assert!(invalid_graph.validate_contract().is_err());
}

#[cfg(feature = "markdown")]
#[test]
fn markdown_projection_names_authority_and_preserves_explicit_link_locator() {
    let parsed = grist::markdown::parse_markdown(
        "[source](https://example.com)",
        grist::core::SourceInfo::stdin("link.md"),
    );
    let graph = parsed
        .payload
        .as_ref()
        .unwrap()
        .to_document_graph(DocumentGraphContext::new("markdown:v2"))
        .unwrap();

    graph.validate_contract().unwrap();
    let projection = graph.projection.as_ref().unwrap();
    assert_eq!(projection.authoritative_payload_kind, "markdown");
    assert_eq!(
        projection.authoritative_payload_schema_version,
        SchemaVersion::MARKDOWN_V2
    );
    let link = graph
        .nodes
        .iter()
        .find(|node| node.kind == DocumentNodeKind::Link)
        .unwrap();
    assert!(link.locator.is_some());
    assert!(link.extensions.contains_key("grist.markdown"));
    let link_edge = graph
        .edges
        .iter()
        .find(|edge| edge.relation == DocumentRelation::LinksTo)
        .unwrap();
    assert!(matches!(
        link_edge.evidence,
        RelationEvidence::Explicit { .. }
    ));
}

#[test]
fn unknown_node_retains_namespaced_parser_payload() {
    let raw = RawNodeContent::new(
        "vendor.custom_format",
        "future-widget",
        json!({"tag": 77, "bytes": [0, 1, 2]}),
    )
    .unwrap();
    let mut graph = DocumentGraph::new("unknown", DocumentKind::Document);
    graph.add_node(DocumentNode::new("widget", DocumentNodeKind::Unknown).with_raw(raw));
    graph.validate_contract().unwrap();
    assert_eq!(
        graph.nodes[0].raw.as_ref().unwrap().payload,
        json!({"tag": 77, "bytes": [0, 1, 2]})
    );
}

#[cfg(feature = "schemas")]
#[test]
fn generated_v2_schema_requires_relation_evidence() {
    let entry = grist::schema::list_schemas()
        .into_iter()
        .find(|entry| entry.name == "document-graph")
        .unwrap();
    assert_eq!(entry.schema_version, SchemaVersion::DOCUMENT_GRAPH_V2);

    let schema = grist::schema::schema_json("document-graph").unwrap();
    let required = schema["definitions"]["DocumentEdge"]["required"]
        .as_array()
        .unwrap();
    assert!(required.iter().any(|field| field == "id"));
    assert!(required.iter().any(|field| field == "evidence"));
    let node_schema = schema["definitions"]["DocumentNodeKind"].to_string();
    assert!(node_schema.contains("archive_member"));
    assert!(node_schema.contains("unknown"));
    let relation_schema = schema["definitions"]["DocumentRelation"].to_string();
    assert!(relation_schema.contains("formula_depends_on"));
    assert!(relation_schema.contains("reconciled_with"));
}
