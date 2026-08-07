#![cfg(all(feature = "xml", feature = "schemas", feature = "document-graph"))]
use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, LocationComponent, OperationStatus,
    ParseRequest, ProviderSet, RequestId, ResourceBudget, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_source};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::render::{FidelityMode, RenderFormat, RenderOptions, render_document_graph};
use grist::segment::{SegmentOptions, segment_document_graph};
use grist::xml::{
    XmlDialect, XmlEntityDisposition, XmlNodeKind, XmlOptions, parse_xml, parse_xml_bytes,
};
const JATS: &str = r#"<?xml version="1.0"?><article xmlns="http://jats.nlm.nih.gov" xmlns:xlink="http://www.w3.org/1999/xlink"><front><article-meta><title-group><article-title>Universal JATS</article-title></title-group><article-id pub-id-type="doi">10.1/example</article-id><contrib-group><contrib contrib-type="author"><name><surname>Lovelace</surname><given-names>Ada</given-names></name></contrib></contrib-group></article-meta></front><body><sec id="methods"><title>Methods</title><p>Ordered <bold>text</bold> with <xref ref-type="bibr" rid="r1">citation</xref>.</p><table-wrap><table><tr><th>Name</th><th>Value</th></tr><tr><td>one</td><td rowspan="2">1</td></tr></table></table-wrap><fig id="f1"><caption><p>Figure caption</p></caption><graphic xlink:href="figure.png"/></fig><custom:opaque xmlns:custom="urn:custom" custom:state="raw">retained</custom:opaque></sec></body><back><ref-list><ref id="r1"><mixed-citation>Reference one</mixed-citation></ref></ref-list></back></article>"#;
#[test]
fn authoritative_structure_namespace_paths_and_unknowns_are_exact() {
    let first = parse_xml(JATS, SourceInfo::stdin("article.nxml"), &Default::default());
    let second = parse_xml(JATS, SourceInfo::stdin("article.nxml"), &Default::default());
    assert_eq!(first, second);
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:#?}",
        first.diagnostics
    );
    let d = first.payload.unwrap();
    assert_eq!(d.dialect, XmlDialect::Jats);
    assert!(d.well_formed);
    assert!(d.nodes.iter().all(|n| n.locator.validate().is_ok()
        && n.raw == d.decoded_text[n.range.byte_start..n.range.byte_end]));
    assert!(d.nodes.iter().all(|n|matches!(n.locator.components().last(),Some(LocationComponent::XmlPath{path}) if path==&n.xml_path)));
    let opaque = d
        .nodes
        .iter()
        .find(|n| n.qualified_name.as_deref() == Some("custom:opaque"))
        .unwrap();
    assert!(!opaque.known_jats_element);
    assert_eq!(opaque.namespace_uri.as_deref(), Some("urn:custom"));
    assert!(
        opaque
            .attributes
            .iter()
            .any(|a| a.qualified_name == "custom:state"
                && a.namespace_uri.as_deref() == Some("urn:custom"))
    );
    assert!(d.metadata.iter().any(|m| m.value == "Universal JATS"));
    assert!(d.links.iter().any(|l| l.destination == "r1"));
    assert_eq!(d.tables[0].rows[1].cells[1].row_span, 2);
    assert_eq!(d.media[0].destination.as_deref(), Some("figure.png"));
    assert!(
        d.sections
            .iter()
            .any(|s| s.title.as_deref() == Some("Methods"))
    );
    let sec = d
        .nodes
        .iter()
        .find(|n| n.local_name.as_deref() == Some("p"))
        .unwrap();
    let ordered = sec
        .children
        .iter()
        .filter_map(|id| d.nodes.iter().find(|n| &n.id == id))
        .map(|n| (n.kind, n.text.clone()))
        .collect::<Vec<_>>();
    assert!(
        matches!(ordered.first(),Some((XmlNodeKind::Text,Some(text))) if text.starts_with("Ordered "))
    );
}
#[test]
fn entities_xinclude_schemas_and_malformed_content_are_inert() {
    let x = r#"<!DOCTYPE article [<!ENTITY xxe SYSTEM "file:///secret"><!ENTITY local "safe">]><article xmlns:xi="http://www.w3.org/2001/XInclude" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="urn:test https://network.invalid/schema.xsd"><xi:include href="https://network.invalid/never"/><p title="&local;">&xxe; &amp;</article>tail"#;
    let e = parse_xml(x, SourceInfo::stdin("hostile.xml"), &Default::default());
    assert_eq!(e.status, OperationStatus::Partial);
    let d = e.payload.unwrap();
    assert!(!d.well_formed);
    assert!(
        d.nodes
            .iter()
            .any(|n| n.kind == XmlNodeKind::RawUnknown && n.raw == "tail")
    );
    assert!(
        d.entities
            .iter()
            .any(|x| x.name == "xxe" && x.disposition == XmlEntityDisposition::Inert)
    );
    assert!(
        d.entities
            .iter()
            .any(|x| x.name == "local" && x.disposition == XmlEntityDisposition::Inert)
    );
    for code in [
        "grist.security.xml.doctype",
        "grist.security.xml.entity_declaration",
        "grist.security.xml.xinclude",
        "grist.security.xml.remote_schema",
        "xml.entity.unresolved",
        "xml.document.text_outside_root",
    ] {
        assert!(
            e.diagnostics.iter().any(|d| d.code.as_str() == code),
            "missing {code}: {:#?}",
            e.diagnostics
        )
    }
    assert!(d.decoded_text.contains("file:///secret"));
}
#[test]
fn graph_segments_render_schemas_registry_detection_and_budget_agree() {
    let e = parse_xml(JATS, SourceInfo::stdin("graph.nxml"), &Default::default());
    let payload = e.payload.as_ref().unwrap();
    let context = DocumentGraphContext::new("xml:graph").with_source(e.source.clone());
    let first = payload.to_document_graph(context.clone()).unwrap();
    let second = payload.to_document_graph(context).unwrap();
    assert_eq!(first, second);
    for kind in [
        DocumentNodeKind::Section,
        DocumentNodeKind::Heading,
        DocumentNodeKind::Paragraph,
        DocumentNodeKind::Citation,
        DocumentNodeKind::Table,
        DocumentNodeKind::TableRow,
        DocumentNodeKind::TableCell,
        DocumentNodeKind::Figure,
        DocumentNodeKind::Image,
        DocumentNodeKind::BibliographyEntry,
        DocumentNodeKind::RawBlock,
    ] {
        assert!(first.nodes.iter().any(|n| n.kind == kind), "{kind:?}")
    }
    assert!(first.edges.iter().any(|e| {
        e.relation == DocumentRelation::Cites
            && e.attrs
                .get("target_xml_id")
                .and_then(|value| value.as_str())
                == Some("r1")
            && first.nodes.iter().any(|node| node.id == e.target)
    }));
    first.validate_contract().unwrap();
    let source = e.identity.as_ref().unwrap();
    let document = ContentIdentity::default()
        .with_canonical_payload(first.schema_version.as_str(), &first)
        .unwrap();
    let segments =
        segment_document_graph(&first, source, &document, &SegmentOptions::default(), None)
            .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .all(|s| !s.node_ids.is_empty() && !s.locators.is_empty())
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
    assert!(rendered.content.contains("Universal JATS"));
    for (name, value) in [
        ("xml", serde_json::to_value(payload).unwrap()),
        ("xml-envelope", serde_json::to_value(&e).unwrap()),
        (
            "xml-options",
            serde_json::to_value(XmlOptions::default()).unwrap(),
        ),
    ] {
        let result = grist::schema::validate_schema(name, &value).unwrap();
        assert!(result.valid, "{name}: {:?}", result.issues)
    }
    let registry = builtin_parser_registry().unwrap();
    for alias in ["xml", "jats", "nxml"] {
        assert!(matches!(
            registry.select_format(alias),
            ParserSelection::Available(_)
        ))
    }
    for (name, bytes) in [
        ("valid.xml", b"<?xml version=\"1.0\"?><root/>".as_slice()),
        (
            "mislabeled.bin",
            b"<article><body><p>x</p></body></article>".as_slice(),
        ),
        ("extensionless", b"<root><value>x</value></root>".as_slice()),
        ("malformed.xml", b"<root><open></root>".as_slice()),
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
        assert_eq!(
            detection.status,
            DetectionStatus::Selected,
            "{name}: {detection:#?}"
        );
        assert_eq!(
            detection
                .selected_format_identity()
                .as_ref()
                .map(|x| x.format.as_str()),
            Some("xml")
        )
    }
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nodes = Some(1);
    let request = ParseRequest::new(
        RequestId::new("xml-budget").unwrap(),
        Input::bytes(JATS.as_bytes().to_vec()),
        SourceInfo::stdin("budget.xml"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let failed = registry.dispatch("xml", request, None).unwrap();
    assert_eq!(failed.status, OperationStatus::Failed);
}
#[test]
fn declared_utf16_and_empty_inputs_have_explicit_status() {
    let text = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><root>snowman ☃</root>";
    let mut bytes = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes())
    }
    let e = parse_xml_bytes(&bytes, SourceInfo::stdin("utf16.xml"), &Default::default());
    assert_eq!(e.status, OperationStatus::Complete, "{:#?}", e.diagnostics);
    assert!(e.payload.unwrap().decoded_text.contains("snowman ☃"));
    let empty = parse_xml("", SourceInfo::stdin("empty.xml"), &Default::default());
    assert_eq!(empty.status, OperationStatus::Partial);
    assert!(empty.payload.is_some());
}
