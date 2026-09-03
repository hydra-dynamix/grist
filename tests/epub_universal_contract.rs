#![cfg(all(feature = "epub", feature = "document-graph", feature = "schemas"))]

use grist::core::{
    BudgetSelection, Input, LocationComponent, OperationStatus, ParseRequest, ProviderSet,
    RequestId, ResourceBudget, SourceInfo,
};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::epub::EpubDocument;
use grist::registry::builtin_parser_registry;
use grist::segment::{SegmentOptions, segment_document_graph};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const CONTAINER: &[u8] = br#"<container><rootfiles><rootfile full-path="OPS/book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
const EPUB2_OPF: &[u8] = br#"<package version="2.0" unique-identifier="uid"><metadata><title>EPUB Two</title></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="ncx"><itemref idref="chapter"/></spine></package>"#;
const EPUB3_OPF: &[u8] = br#"<package version="3.0"><metadata><title>EPUB Three</title></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="css" href="style.css" media-type="text/css"/><item id="image" href="figure.png" media-type="image/png"/><item id="font" href="font.woff2" media-type="font/woff2"/></manifest><spine><itemref idref="chapter"/><itemref idref="nav" linear="no"/></spine></package>"#;
const CHAPTER: &[u8] = concat!("<html xmlns=\"http", "://www.w3.org/1999/xhtml\" xmlns:epub=\"http", "://www.idpf.org/2007/ops\"><head><title>Chapter</title></head><body><h1>Chapter</h1><p>Text<a epub:type=\"noteref\" href=\"#note\">1</a></p><aside epub:type=\"footnote\" id=\"note\"><p>Footnote text</p></aside><figure><img src=\"figure.png\" alt=\"Diagram\"/></figure></body></html>").as_bytes();
const NAV: &[u8] = concat!("<html xmlns=\"http", "://www.w3.org/1999/xhtml\" xmlns:epub=\"http", "://www.idpf.org/2007/ops\"><head><title>Contents</title></head><body><nav epub:type=\"toc\"><ol><li><a href=\"chapter.xhtml\">Chapter</a></li></ol></nav><nav epub:type=\"landmarks\"><a href=\"chapter.xhtml\">Body</a></nav></body></html>").as_bytes();
const NCX: &[u8] = br#"<ncx><navMap><navPoint id="n1" playOrder="1"><navLabel><text>Chapter</text></navLabel><content src="chapter.xhtml"/></navPoint></navMap></ncx>"#;

fn add(zip: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str, bytes: &[u8], deflated: bool) {
    let method = if deflated {
        CompressionMethod::Deflated
    } else {
        CompressionMethod::Stored
    };
    zip.start_file(
        name,
        SimpleFileOptions::default().compression_method(method),
    )
    .unwrap();
    zip.write_all(bytes).unwrap();
}

fn package(version: u8, hostile: bool, encrypted_spine: bool) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    add(&mut zip, "mimetype", b"application/epub+zip", false);
    add(&mut zip, "META-INF/container.xml", CONTAINER, true);
    if encrypted_spine {
        add(
            &mut zip,
            "META-INF/encryption.xml",
            br#"<encryption><CipherReference URI="../OPS/chapter.xhtml"/></encryption>"#,
            true,
        );
    }
    add(
        &mut zip,
        "OPS/book.opf",
        if version == 3 { EPUB3_OPF } else { EPUB2_OPF },
        true,
    );
    add(&mut zip, "OPS/chapter.xhtml", CHAPTER, true);
    if version == 3 {
        add(&mut zip, "OPS/nav.xhtml", NAV, true);
        add(
            &mut zip,
            "OPS/style.css",
            b"@font-face{src:url('font.woff2')} body{background:url(figure.png)}",
            true,
        );
        add(&mut zip, "OPS/figure.png", b"PNG fixture", true);
        add(&mut zip, "OPS/font.woff2", b"wOF2fixture", true);
    } else {
        add(&mut zip, "OPS/toc.ncx", NCX, true);
    }
    if hostile {
        add(&mut zip, "../escape.txt", b"must never materialize", true);
    }
    zip.finish().unwrap().into_inner()
}

fn package_with_opf(opf: &[u8]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    add(&mut zip, "mimetype", b"application/epub+zip", false);
    add(&mut zip, "META-INF/container.xml", CONTAINER, true);
    add(&mut zip, "OPS/book.opf", opf, true);
    add(&mut zip, "OPS/chapter.xhtml", CHAPTER, true);
    zip.finish().unwrap().into_inner()
}

fn dispatch(bytes: Vec<u8>, budget: ResourceBudget) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("epub-test").unwrap(),
        Input::bytes(bytes),
        SourceInfo::stdin("book.epub").with_declared_mime_type("application/epub+zip"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch("epub", request, None)
        .unwrap()
}

#[test]
fn epub3_retains_package_order_navigation_notes_and_resources() {
    let first = dispatch(
        package(3, false, false),
        ResourceBudget::trusted_unbounded(),
    );
    let second = dispatch(
        package(3, false, false),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:#?}",
        first.diagnostics
    );
    assert_eq!(first.payload, second.payload);
    let document: EpubDocument = serde_json::from_value(first.payload.clone().unwrap()).unwrap();
    assert_eq!(document.spine.items[0].idref, "chapter");
    assert!(
        document
            .navigation
            .iter()
            .any(|navigation| navigation.kind == "toc")
    );
    assert!(
        document
            .navigation
            .iter()
            .any(|navigation| navigation.kind == "landmarks")
    );
    assert_eq!(document.footnotes[0].text, "Footnote text");
    assert!(document.footnotes[0].references[0].resolved);
    assert_eq!(
        document.images[0].usages[0].alt_text.as_deref(),
        Some("Diagram")
    );
    assert_eq!(
        document.styles[0].referenced_resources,
        vec!["figure.png", "font.woff2"]
    );
    assert_eq!(document.resources.len(), 5);
    assert!(
        document
            .resources
            .iter()
            .all(|resource| { resource.artifact.identity.content.raw.is_some() })
    );
    assert!(document.chapters[0].document.nodes.iter().all(|node| {
        matches!(
            node.locator.components().first(),
            Some(LocationComponent::ArchiveMember { .. })
        )
    }));
}

#[test]
fn epub_graph_segments_and_schema_retain_nested_provenance() {
    let envelope = dispatch(
        package(3, false, false),
        ResourceBudget::trusted_unbounded(),
    );
    let document: EpubDocument = serde_json::from_value(envelope.payload.clone().unwrap()).unwrap();
    let graph = document
        .to_document_graph(
            DocumentGraphContext::new("epub:fixture").with_source(envelope.source.clone()),
        )
        .unwrap();
    graph.validate_contract().unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Footnote)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::FootnoteFor)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Precedes)
    );
    let source_identity = envelope.identity.as_ref().unwrap();
    let document_identity = grist::core::ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(!segments.segments.is_empty());
    assert!(
        segments
            .segments
            .iter()
            .all(|segment| !segment.locators.is_empty())
    );
    let schema = grist::schema::schema_json("epub").unwrap();
    let report = grist::schema::validate_against_schema(
        "epub",
        grist::core::SchemaVersion::EPUB_V1,
        envelope.payload.as_ref().unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
}

#[test]
fn epub2_ncx_and_hostile_member_are_handled_safely() {
    let complete = dispatch(
        package(2, false, false),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(
        complete.status,
        OperationStatus::Complete,
        "{:#?}",
        complete.diagnostics
    );
    let document: EpubDocument = serde_json::from_value(complete.payload.unwrap()).unwrap();
    assert_eq!(document.navigation[0].entries[0].label, "Chapter");
    assert_eq!(document.navigation[0].entries[0].play_order, Some(1));

    let hostile = dispatch(package(2, true, false), ResourceBudget::trusted_unbounded());
    assert_eq!(hostile.status, OperationStatus::Partial);
    assert!(hostile.diagnostics.iter().any(|diagnostic| {
        diagnostic.explanation_key.as_deref() == Some("grist.security.archive.path_traversal")
    }));
}

#[test]
fn encryption_budget_and_malformed_zip_have_distinct_statuses() {
    let encrypted = dispatch(package(3, false, true), ResourceBudget::trusted_unbounded());
    assert_eq!(encrypted.status, OperationStatus::Encrypted);
    assert!(encrypted.payload.is_none());

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_archive_expansion_ratio = Some(1.0);
    let limited = dispatch(package(3, false, false), budget);
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_expansion_ratio.exhausted"
    }));

    let malformed = dispatch(
        b"PK truncated".to_vec(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(malformed.payload.is_none());

    let malformed_xml = dispatch(
        package_with_opf(br#"<package version="3.0"><metadata><title>Malformed</title></metadata><manifest><item id="chapter" id="duplicate" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(malformed_xml.status, OperationStatus::Partial);
    assert!(
        malformed_xml
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("malformed XML attributes") })
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_routes_epub_to_the_same_parser() {
    use std::fs;
    use std::process::Command;
    let mut path = std::env::temp_dir();
    path.push(format!("grist-epub-{}.epub", std::process::id()));
    fs::write(&path, package(3, false, false)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "epub", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["kind"], "epub");
    assert_eq!(value["status"], "complete");
}
