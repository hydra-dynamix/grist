#![cfg(all(feature = "xml", feature = "schemas", feature = "document-graph"))]

use grist::core::{LocationComponent, OperationStatus, SourceInfo};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::jats::{JatsReferenceScope, JatsRelationshipKind, JatsResolution, JatsTargetKind};
use grist::xml::parse_xml;

const ARTICLE: &str = r#"<article xmlns="http://jats.nlm.nih.gov" xmlns:xlink="http://www.w3.org/1999/xlink">
<front><article-meta id="meta"><title-group><article-title>Links</article-title></title-group>
<contrib-group><contrib id="author-1" rid="aff-1"><name><surname>Example</surname></name></contrib></contrib-group>
<aff id="aff-1"><label>A</label>Institute</aff></article-meta></front>
<body><sec id="intro"><title>Introduction</title><p>
<xref ref-type="bibr" rid="ref-1">[1]</xref>
<xref ref-type="fig" rid="fig-1">Figure 1</xref>
<xref ref-type="table" rid="table-1">Table 1</xref>
<xref ref-type="supplementary-material" rid="supp-1">Supplement</xref></p>
<fig id="fig-1"><label>Figure 1</label><caption><p>A figure.</p></caption><graphic xlink:href="figure.png"/></fig>
<table-wrap id="table-1"><label>Table 1</label><caption><p>A table.</p></caption><table><tr><td>x</td></tr></table></table-wrap>
<supplementary-material id="supp-1" xlink:href="supp.zip"><label>S1</label></supplementary-material>
</sec></body><back><ref-list><ref id="ref-1"><label>1</label><mixed-citation>Reference one.</mixed-citation></ref></ref-list></back>
</article>"#;

#[test]
fn authoritative_links_cover_citations_objects_labels_and_metadata() {
    let first = parse_xml(
        ARTICLE,
        SourceInfo::stdin("article.nxml"),
        &Default::default(),
    );
    let second = parse_xml(
        ARTICLE,
        SourceInfo::stdin("article.nxml"),
        &Default::default(),
    );
    assert_eq!(first, second);
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:#?}",
        first.diagnostics
    );
    let document = first.payload.as_ref().unwrap();
    let links = document.scholarly_links.as_ref().unwrap();

    for kind in [
        JatsTargetKind::BibliographyEntry,
        JatsTargetKind::Figure,
        JatsTargetKind::Table,
        JatsTargetKind::Supplement,
        JatsTargetKind::Section,
        JatsTargetKind::Metadata,
    ] {
        assert!(
            links.targets.iter().any(|target| target.kind == kind),
            "{kind:?}"
        );
    }
    assert_eq!(links.labels.len(), 5);
    assert!(
        links
            .labels
            .iter()
            .all(|label| label.owner_node_id.is_some())
    );
    assert_eq!(links.relationships.len(), 5);
    assert!(links.relationships.iter().all(|relationship| {
        relationship.resolution == JatsResolution::Resolved
            && relationship.target_node_id.is_some()
            && relationship.locator.validate().is_ok()
            && matches!(
                relationship.locator.components().last(),
                Some(LocationComponent::XmlPath { .. })
            )
    }));
    assert!(links.relationships.iter().any(|relationship| {
        relationship.scope == JatsReferenceScope::Body
            && relationship.kind == JatsRelationshipKind::Citation
    }));
    assert!(links.relationships.iter().any(|relationship| {
        relationship.scope == JatsReferenceScope::FrontMetadata
            && relationship.kind == JatsRelationshipKind::MetadataReference
    }));

    let graph = document
        .to_document_graph(
            DocumentGraphContext::new("graph:jats").with_source(first.source.clone()),
        )
        .unwrap();
    graph.validate_contract().unwrap();
    for kind in [
        DocumentNodeKind::Citation,
        DocumentNodeKind::Reference,
        DocumentNodeKind::BibliographyEntry,
        DocumentNodeKind::Figure,
        DocumentNodeKind::Table,
        DocumentNodeKind::Attachment,
        DocumentNodeKind::Label,
        DocumentNodeKind::Metadata,
    ] {
        assert!(graph.nodes.iter().any(|node| node.kind == kind), "{kind:?}");
    }
    let scholarly_edges = graph
        .edges
        .iter()
        .filter(|edge| edge.attrs.contains_key("jats_relationship_id"))
        .collect::<Vec<_>>();
    assert_eq!(scholarly_edges.len(), links.relationships.len());
    assert!(scholarly_edges.iter().all(|edge| {
        matches!(
            edge.relation,
            DocumentRelation::Cites | DocumentRelation::References
        ) && graph.nodes.iter().any(|node| node.id == edge.target)
            && edge
                .attrs
                .get("resolution")
                .and_then(|value| value.as_str())
                == Some("resolved")
    }));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::CaptionFor)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Defines)
    );

    for (schema, value) in [
        ("xml", serde_json::to_value(document).unwrap()),
        ("xml-envelope", serde_json::to_value(&first).unwrap()),
    ] {
        let result = grist::schema::validate_schema(schema, &value).unwrap();
        assert!(result.valid, "{schema}: {:?}", result.issues);
    }
}

#[test]
fn malformed_links_keep_every_idref_and_candidate() {
    let article = r#"<article><body><p><xref ref-type="bibr" rid="known missing duplicate"/><xref ref-type="fig" rid=""/></p><fig id="duplicate"/><table-wrap id="duplicate"/></body><back><ref-list><ref id="known"/></ref-list></back></article>"#;
    let envelope = parse_xml(
        article,
        SourceInfo::stdin("malformed.nxml"),
        &Default::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let document = envelope.payload.unwrap();
    let links = document.scholarly_links.as_ref().unwrap();
    assert_eq!(links.relationships.len(), 4);
    assert_eq!(links.relationships[0].resolution, JatsResolution::Resolved);
    assert_eq!(
        links.relationships[1].target_xml_id.as_deref(),
        Some("missing")
    );
    assert_eq!(
        links.relationships[1].resolution,
        JatsResolution::Unresolved
    );
    assert_eq!(links.relationships[2].resolution, JatsResolution::Ambiguous);
    assert_eq!(links.relationships[2].candidate_node_ids.len(), 2);
    assert!(links.relationships[3].target_xml_id.is_none());
    assert_eq!(links.relationships[3].raw_rid, "");
    for code in [
        "jats.target.duplicate_id",
        "jats.xref.unresolved",
        "jats.xref.ambiguous",
        "jats.xref.empty_rid",
    ] {
        assert!(
            envelope
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == code),
            "missing {code}: {:#?}",
            envelope.diagnostics
        );
    }

    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:malformed-jats"))
        .unwrap();
    let unresolved = graph.edges.iter().find(|edge| {
        edge.attrs
            .get("resolution")
            .and_then(|value| value.as_str())
            == Some("unresolved")
    });
    assert_eq!(unresolved.map(|edge| edge.target.as_str()), Some("missing"));
}

#[test]
fn native_jats_target_ids_survive_unrelated_source_movement() {
    let moved = ARTICLE.replace(
        r#"<fig id="fig-1">"#,
        r#"<p>Unrelated insertion.</p><fig id="fig-1">"#,
    );
    let graphs = [ARTICLE, moved.as_str()].map(|source| {
        parse_xml(
            source,
            SourceInfo::stdin("stable.nxml"),
            &Default::default(),
        )
        .payload
        .unwrap()
        .to_document_graph(DocumentGraphContext::new("graph:stable-jats"))
        .unwrap()
    });
    for xml_id in ["fig-1", "table-1", "supp-1", "ref-1"] {
        let ids = graphs.each_ref().map(|graph| {
            graph
                .nodes
                .iter()
                .find(|node| {
                    node.attrs.get("xml_id").and_then(|value| value.as_str()) == Some(xml_id)
                })
                .unwrap()
                .id
                .as_str()
        });
        assert_eq!(ids[0], ids[1], "{xml_id}");
    }
}
