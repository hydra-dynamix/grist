#![cfg(all(feature = "odf-word", feature = "document-graph", feature = "schemas"))]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, LocationComponent, OperationStatus,
    ParseRequest, ProviderSet, RequestId, ResourceBudget, SchemaVersion, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::odf_word::{OdfNode, OdfNodeKind, OdfPackageKind, OdfWordDocument, OdfWordOptions};
use grist::registry::{Capability, builtin_parser_registry};
use grist::render::{FidelityMode, RenderFormat, RenderOptions, render_document_graph};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const ODT: &str = "application/vnd.oasis.opendocument.text";
const OTT: &str = "application/vnd.oasis.opendocument.text-template";

const CONTENT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content office:version="1.3"
 xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:xlink="http://www.w3.org/1999/xlink"
 xmlns:dc="http://purl.org/dc/elements/1.1/"
 xmlns:math="http://www.w3.org/1998/Math/MathML">
 <office:automatic-styles><style:style xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" style:name="P1" style:family="paragraph"/></office:automatic-styles>
 <office:body><office:text>
  <text:tracked-changes>
   <text:changed-region text:id="ct1"><text:insertion><office:change-info><dc:creator>Ada</dc:creator><dc:date>2026-01-02</dc:date></office:change-info></text:insertion></text:changed-region>
   <text:changed-region text:id="ct2"><text:deletion><office:change-info><dc:creator>Bob</dc:creator></office:change-info><text:p>old text</text:p></text:deletion></text:changed-region>
  </text:tracked-changes>
  <text:h text:outline-level="1" text:style-name="Heading">ODF Heading</text:h>
  <text:section text:name="S1"><text:p text:style-name="P1">Hello <text:span text:style-name="Bold">world</text:span><text:s text:c="2"/><text:a xlink:href="https://example.invalid">link</text:a>.</text:p></text:section>
  <text:list text:style-name="L1"><text:list-item><text:p>First</text:p><text:list><text:list-item><text:p>Nested</text:p></text:list-item></text:list></text:list-item></text:list>
  <table:table table:name="T1"><table:table-row><table:table-cell table:number-columns-spanned="2"><text:p>A</text:p><table:table table:name="Nested"><table:table-row><table:table-cell><text:p>N</text:p></table:table-cell></table:table-row></table:table></table:table-cell><table:covered-table-cell/></table:table-row></table:table>
  <text:p>before <text:change-start text:change-id="ct1"/><text:span>inserted text</text:span><text:change-end text:change-id="ct1"/> <text:change text:change-id="ct2"/> after</text:p>
  <text:p>note<text:note text:id="fn1" text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>Footnote body</text:p></text:note-body></text:note></text:p>
  <office:annotation office:name="ann1"><dc:creator>Reviewer</dc:creator><dc:date>2026-01-03</dc:date><text:p>Annotation body</text:p></office:annotation>
  <draw:frame draw:name="Figure 1" svg:width="4cm" svg:height="3cm" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"><draw:image xlink:href="Pictures/figure.png"><svg:desc>Figure alt</svg:desc></draw:image><draw:text-box><text:p>Text box</text:p></draw:text-box></draw:frame>
  <draw:object xlink:href="./Object 1"/>
  <math:math><math:mi>x</math:mi><math:mo>=</math:mo><math:mn>1</math:mn></math:math>
  <text:unknown-extension text:value="retained">unknown text</text:unknown-extension>
 </office:text></office:body>
</office:document-content>"#;

const STYLES: &[u8] = br#"<office:document-styles office:version="1.3" xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"><office:styles><style:style style:name="Heading" style:display-name="Heading" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style><text:list-style style:name="L1"><text:list-level-style-number text:level="1" style:num-format="1" text:start-value="1"/><text:list-level-style-bullet text:level="2" text:bullet-char="?"/></text:list-style></office:styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Header text</text:p></style:header><style:footer><text:p>Footer text</text:p></style:footer></style:master-page></office:master-styles></office:document-styles>"#;
const META: &[u8] = br#"<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0"><office:meta><dc:title>ODF title</dc:title><dc:creator>Grist</dc:creator><meta:user-defined meta:name="Project">Complete Parser</meta:user-defined></office:meta></office:document-meta>"#;
const SETTINGS: &[u8] = br#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings><config:config-item config:name="ViewAreaTop">0</config:config-item></office:settings></office:document-settings>"#;
const OBJECT: &[u8] = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:math="http://www.w3.org/1998/Math/MathML"><office:body><math:math><math:mi>y</math:mi><math:mo>=</math:mo><math:mn>2</math:mn></math:math></office:body></office:document-content>"#;

fn add(zip: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str, bytes: &[u8], stored: bool) {
    let method = if stored {
        CompressionMethod::Stored
    } else {
        CompressionMethod::Deflated
    };
    zip.start_file(
        name,
        SimpleFileOptions::default().compression_method(method),
    )
    .unwrap();
    zip.write_all(bytes).unwrap();
}

fn package(media_type: &str, encrypted: bool, hostile: bool, malformed: bool) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    add(&mut zip, "mimetype", media_type.as_bytes(), true);
    let encryption = if encrypted {
        r#"<manifest:encryption-data manifest:checksum="abc" manifest:checksum-type="SHA256"/>"#
    } else {
        ""
    };
    let manifest = format!(
        r#"<manifest:manifest manifest:version="1.3" xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="{media_type}"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml">{encryption}</manifest:file-entry><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="settings.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Pictures/figure.png" manifest:media-type="image/png"/><manifest:file-entry manifest:full-path="Object 1/" manifest:media-type="application/vnd.oasis.opendocument.formula"/><manifest:file-entry manifest:full-path="Object 1/content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#
    );
    add(
        &mut zip,
        "META-INF/manifest.xml",
        manifest.as_bytes(),
        false,
    );
    add(
        &mut zip,
        "content.xml",
        if malformed {
            b"<office:broken"
        } else {
            CONTENT
        },
        false,
    );
    add(&mut zip, "styles.xml", STYLES, false);
    add(&mut zip, "meta.xml", META, false);
    add(&mut zip, "settings.xml", SETTINGS, false);
    add(&mut zip, "Pictures/figure.png", b"PNG fixture", false);
    add(&mut zip, "Object 1/content.xml", OBJECT, false);
    if hostile {
        add(&mut zip, "../escape.bin", b"must remain inert", false);
    }
    zip.finish().unwrap().into_inner()
}

fn package_with_content(content: &[u8]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    add(&mut zip, "mimetype", ODT.as_bytes(), true);
    add(
        &mut zip,
        "META-INF/manifest.xml",
        br#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#,
        false,
    );
    add(&mut zip, "content.xml", content, false);
    zip.finish().unwrap().into_inner()
}

fn dispatch(
    selector: &str,
    bytes: Vec<u8>,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("odf-test").unwrap(),
        Input::bytes(bytes),
        SourceInfo::new(format!("fixture.{selector}")),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch(
            selector,
            request,
            Some(serde_json::to_value(OdfWordOptions::default()).unwrap()),
        )
        .unwrap()
}

fn contains_kind(node: &OdfNode, kind: OdfNodeKind) -> bool {
    node.kind == kind
        || node.content.iter().any(|content| match content {
            grist::odf_word::OdfContent::Element { node } => contains_kind(node, kind.clone()),
            _ => false,
        })
}

#[test]
fn odt_preserves_structure_revisions_rich_content_and_views() {
    let bytes = package(ODT, false, false, false);
    let first = dispatch("odt", bytes.clone(), ResourceBudget::trusted_unbounded());
    let second = dispatch("odt", bytes, ResourceBudget::trusted_unbounded());
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:#?}",
        first.diagnostics
    );
    assert_eq!(first.payload, second.payload);
    let document: OdfWordDocument = serde_json::from_value(first.payload.clone().unwrap()).unwrap();
    assert_eq!(document.package_kind, OdfPackageKind::Document);
    assert_eq!(document.version.as_deref(), Some("1.3"));
    assert!(
        document
            .metadata
            .iter()
            .any(|item| item.value == "ODF title")
    );
    assert!(
        document
            .styles
            .iter()
            .any(|style| style.name.as_deref() == Some("Heading"))
    );
    assert_eq!(document.list_styles[0].levels.len(), 2);
    assert_eq!(document.master_pages[0].headers.len(), 1);
    for kind in [
        OdfNodeKind::Section,
        OdfNodeKind::List,
        OdfNodeKind::Table,
        OdfNodeKind::Link,
        OdfNodeKind::Footnote,
        OdfNodeKind::Annotation,
        OdfNodeKind::Drawing,
        OdfNodeKind::Equation,
        OdfNodeKind::Unknown,
    ] {
        assert!(
            contains_kind(&document.body, kind.clone()),
            "missing {kind:?}"
        );
    }
    assert_eq!(document.revisions.len(), 2);
    assert!(document.views.accepted.contains("inserted text"));
    assert!(!document.views.accepted.contains("old text"));
    assert!(!document.views.original.contains("inserted text"));
    assert!(document.views.original.contains("old text"));
    assert_eq!(document.views.original, document.views.rejected);
    assert!(document.links[0].external);
    assert_eq!(document.notes[0].text, "Footnote body");
    assert_eq!(document.annotations[0].creator.as_deref(), Some("Reviewer"));
    assert!(
        document
            .drawings
            .iter()
            .any(|drawing| { drawing.alt_text.as_deref() == Some("Figure alt") })
    );
    assert!(document.equations.len() >= 2);
    assert!(!document.embedded_objects[0].artifact_ids.is_empty());
    assert_eq!(document.embedded_artifacts.len(), 2);
    let locator = &document.links[0].locator;
    assert!(matches!(
        locator.components(),
        [
            LocationComponent::ArchiveMember { .. },
            LocationComponent::TextRange { .. },
            LocationComponent::XmlPath { .. }
        ]
    ));

    let graph = document
        .to_document_graph(DocumentGraphContext::new("odf-graph"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Table)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::LinksTo)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::RevisionOf)
    );
    let root_extension = &graph.nodes[0].extensions["grist.odf_word"];
    assert_eq!(
        root_extension["styles"].as_array().unwrap().len(),
        document.styles.len()
    );
    assert_eq!(root_extension["views"]["original"], document.views.original);
    let rendered = render_document_graph(
        &graph,
        RenderFormat::PlainText,
        &RenderOptions::new(FidelityMode::RawFallback),
    )
    .unwrap();
    assert!(rendered.content.contains("ODF Heading"));
    let source_identity = first.identity.as_ref().unwrap();
    let document_identity = ContentIdentity::default()
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
    let schema = grist::schema::schema_json("odf-word").unwrap();
    let report = grist::schema::validate_against_schema(
        "odf-word",
        SchemaVersion::ODF_WORD_V1,
        first.payload.as_ref().unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
}

#[test]
fn ott_detection_and_registry_routing_are_distinct() {
    let registry = builtin_parser_registry().unwrap();
    let odt = registry
        .snapshot()
        .parsers
        .into_iter()
        .find(|parser| parser.format.id == "odt")
        .unwrap();
    assert!(odt.capabilities.contains(&Capability::EmbeddedArtifacts));
    for (selector, media_type, kind) in [
        ("odt", ODT, OdfPackageKind::Document),
        ("ott", OTT, OdfPackageKind::Template),
    ] {
        let bytes = package(media_type, false, false, false);
        let detection = detect_with_registry(
            Path::new(&format!("extensionless-{selector}")),
            &bytes,
            None,
            None,
            &Limits::default(),
            &builtin_parser_registry().unwrap(),
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detection.status, DetectionStatus::Selected);
        assert_eq!(detection.candidates[0].identity.format, selector);
        assert_eq!(detection.content_kind, ContentKind::Odt);
        let envelope = dispatch(selector, bytes, ResourceBudget::trusted_unbounded());
        assert_eq!(envelope.status, OperationStatus::Complete);
        let document: OdfWordDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
        assert_eq!(document.package_kind, kind);
    }
}

#[test]
fn encrypted_malformed_hostile_and_budget_cases_are_explicit() {
    let encrypted = dispatch(
        "odt",
        package(ODT, true, false, false),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(encrypted.status, OperationStatus::Encrypted);
    assert!(encrypted.payload.is_none());

    let malformed_xml = dispatch(
        "odt",
        package(ODT, false, false, true),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(malformed_xml.status, OperationStatus::Partial);
    assert!(malformed_xml.payload.is_some());
    assert!(
        malformed_xml
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("malformed XML"))
    );

    let hostile = dispatch(
        "odt",
        package(ODT, false, true, false),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(hostile.status, OperationStatus::Partial);
    assert!(
        hostile.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "grist.security.archive.path_traversal"
        })
    );

    let malformed_zip = dispatch(
        "odt",
        b"PK truncated".to_vec(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(malformed_zip.status, OperationStatus::Failed);
    assert!(malformed_zip.payload.is_none());

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_archive_members = Some(2);
    let limited = dispatch("odt", package(ODT, false, false, false), budget);
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_members.exhausted"
    }));

    let deep = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:text><a><b><c><d><e>deep</e></d></c></b></a></office:text></office:body></office:document-content>"#;
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nesting_depth = Some(4);
    let limited = dispatch("odt", package_with_content(deep), budget);
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(
        limited.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "grist.budget.nesting_depth.exhausted"
        })
    );

    let active_xml = br#"<!DOCTYPE office:document-content [<!ENTITY xxe SYSTEM "file:///secret">]><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"><office:body><office:text><text:p xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">&xxe;</text:p></office:text></office:body></office:document-content>"#;
    let active = dispatch(
        "odt",
        package_with_content(active_xml),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(active.status, OperationStatus::Partial);
    assert!(
        active
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.security.xml.doctype" })
    );
}

#[test]
fn mismatched_package_kind_never_silently_parses() {
    let mismatch = dispatch(
        "ott",
        package(ODT, false, false, false),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(mismatch.status, OperationStatus::Failed);
    assert!(mismatch.payload.is_none());
    assert!(
        mismatch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("requested ott"))
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_routes_odt_and_ott_to_the_public_odf_envelope() {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    for (selector, media_type, package_kind) in [("odt", ODT, "document"), ("ott", OTT, "template")]
    {
        let path = std::env::temp_dir().join(format!("grist-odf-{nonce}.{selector}"));
        fs::write(&path, package(media_type, false, false, false)).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_grist"))
            .args(["parse", selector, path.to_str().unwrap()])
            .output()
            .unwrap();
        let _ = fs::remove_file(&path);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["status"], "complete");
        assert_eq!(envelope["kind"], "odf_word");
        assert_eq!(envelope["payload"]["package_kind"], package_kind);
        assert_eq!(
            envelope["payload"]["views"]["accepted"],
            envelope["payload"]["views"]["visible"]
        );
    }
}
