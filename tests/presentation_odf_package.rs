#![cfg(all(
    feature = "presentation-odf",
    feature = "document-graph",
    feature = "schemas"
))]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, LocationComponent, OperationStatus,
    ParseRequest, ProviderSet, RequestId, ResourceBudget, SchemaVersion, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, RelationEvidence, ToDocumentGraph,
};
use grist::presentation_odf::{
    OdfPresentationDocument, OdfPresentationOptions, OdfPresentationPackageKind,
};
use grist::registry::{Capability, builtin_parser_registry};
use grist::render::{FidelityMode, RenderFormat, RenderOptions, render_document_graph};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const ODP: &str = "application/vnd.oasis.opendocument.presentation";
const OTP: &str = "application/vnd.oasis.opendocument.presentation-template";

const CONTENT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content office:version="1.3"
 xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
 xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
 xmlns:xlink="http://www.w3.org/1999/xlink"
 xmlns:dc="http://purl.org/dc/elements/1.1/"
 xmlns:anim="urn:oasis:names:tc:opendocument:xmlns:animation:1.0"
 xmlns:smil="urn:oasis:names:tc:opendocument:xmlns:smil-compatible:1.0">
 <office:automatic-styles>
  <style:style style:name="dp1" style:family="drawing-page"><style:drawing-page-properties presentation:transition-style="fade-from-center" presentation:duration="PT3S"/></style:style>
  <style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:fill="solid"/></style:style>
 </office:automatic-styles>
 <office:body><office:presentation>
  <draw:page draw:name="Slide One" xml:id="slide1" draw:style-name="dp1" draw:master-page-name="Master1"
    presentation:presentation-page-layout-name="Layout1" presentation:transition-style="fade"
    presentation:transition-on-click="true" presentation:duration="PT5S">
   <draw:frame draw:id="body" draw:name="Body" presentation:class="outline" svg:x="2cm" svg:y="5cm" svg:width="20cm" svg:height="8cm">
    <draw:text-box><text:p text:style-name="P1">Body <text:span text:style-name="Em">text</text:span> <text:a xlink:href="https://example.invalid" office:target-frame-name="_blank">link</text:a></text:p></draw:text-box>
   </draw:frame>
   <draw:frame draw:id="title" draw:name="Title" presentation:class="title" svg:x="2cm" svg:y="1cm" svg:width="20cm" svg:height="2cm">
    <draw:text-box><text:p>ODF Presentation</text:p></draw:text-box>
    <svg:title>Accessible title</svg:title><svg:desc>Accessible description</svg:desc>
   </draw:frame>
   <draw:g draw:id="group" draw:name="Group">
    <draw:rect draw:id="rect" svg:x="1cm" svg:y="10cm" svg:width="3cm" svg:height="2cm"><text:p>Nested shape</text:p></draw:rect>
   </draw:g>
   <draw:frame draw:id="image" svg:x="18cm" svg:y="10cm" svg:width="4cm" svg:height="3cm">
    <draw:image xlink:href="Pictures/photo.png"><svg:desc>Photo alt</svg:desc></draw:image>
   </draw:frame>
   <draw:frame draw:id="table" svg:x="2cm" svg:y="13cm" svg:width="12cm" svg:height="4cm">
    <table:table table:name="Metrics"><table:table-column table:number-columns-repeated="2"/>
     <table:table-row><table:table-cell office:value-type="string"><text:p>A</text:p></table:table-cell><table:table-cell table:number-columns-spanned="2"><text:p>B</text:p></table:table-cell></table:table-row>
     <table:table-row table:number-rows-repeated="2"><table:covered-table-cell/><table:table-cell office:value-type="float" office:value="2"><text:p>2</text:p></table:table-cell></table:table-row>
    </table:table>
   </draw:frame>
   <draw:frame draw:id="chart" svg:x="15cm" svg:y="13cm" svg:width="7cm" svg:height="5cm"><draw:object xlink:href="./Object 1"/></draw:frame>
   <draw:frame draw:id="ole"><draw:object-ole xlink:href="./ObjectReplacements/Object 2"/></draw:frame>
   <office:annotation office:name="comment1"><dc:creator>Reviewer</dc:creator><dc:date>2026-08-07</dc:date><text:p>Check this slide</text:p></office:annotation>
   <presentation:notes><draw:frame draw:id="note"><draw:text-box><text:p>Speaker note</text:p></draw:text-box></draw:frame></presentation:notes>
   <presentation:animations><anim:par smil:begin="0s"><anim:animate smil:targetElement="body" smil:attributeName="opacity" smil:values="0;1" smil:dur="PT1S"/></anim:par></presentation:animations>
   <presentation:unknown-extension presentation:value="retained">raw metadata</presentation:unknown-extension>
  </draw:page>
  <draw:page draw:name="Slide Two" xml:id="slide2" presentation:visibility="hidden"><draw:custom-shape draw:id="second"><text:p>Second slide</text:p></draw:custom-shape></draw:page>
 </office:presentation></office:body>
</office:document-content>"#;

const STYLES: &[u8] = br#"<office:document-styles office:version="1.3"
 xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
 xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0"
 xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"
 xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0">
 <office:styles><style:style style:name="P1" style:display-name="Presentation paragraph" style:family="paragraph"><style:text-properties fo:font-weight="bold"/></style:style></office:styles>
 <office:automatic-styles>
  <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="28cm" fo:page-height="21cm" style:print-orientation="landscape"/></style:page-layout>
  <style:presentation-page-layout style:name="Layout1"><presentation:placeholder presentation:class="title" svg:x="2cm" svg:y="1cm" svg:width="20cm" svg:height="2cm"/></style:presentation-page-layout>
 </office:automatic-styles>
 <office:master-styles><style:master-page style:name="Master1" style:display-name="Master One" style:page-layout-name="pm1" presentation:presentation-page-layout-name="Layout1"><draw:frame draw:id="master-title" presentation:class="title"><draw:text-box><text:p>Master title</text:p></draw:text-box></draw:frame></style:master-page></office:master-styles>
</office:document-styles>"#;

const CHART: &[u8] = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:chart><chart:chart chart:class="chart:bar"><chart:title><text:p>Revenue</text:p></chart:title><chart:plot-area><chart:series chart:values-cell-range-address="local-table.B2:B3" chart:label-cell-address="local-table.B1"><chart:categories table:cell-range-address="local-table.A2:A3" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"/></chart:series></chart:plot-area></chart:chart></office:chart></office:body></office:document-content>"#;

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
        r#"<manifest:manifest manifest:version="1.3" xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="{media_type}"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml">{encryption}</manifest:file-entry><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="settings.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Pictures/photo.png" manifest:media-type="image/png"/><manifest:file-entry manifest:full-path="Object 1/" manifest:media-type="application/vnd.oasis.opendocument.chart"/><manifest:file-entry manifest:full-path="Object 1/content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="ObjectReplacements/Object 2" manifest:media-type="application/x-msdownload"/></manifest:manifest>"#
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
    add(&mut zip, "meta.xml", br#"<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><office:meta><dc:title>Deck metadata</dc:title></office:meta></office:document-meta>"#, false);
    add(&mut zip, "settings.xml", br#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings><config:config-item config:name="ShowNotes">true</config:config-item></office:settings></office:document-settings>"#, false);
    add(&mut zip, "Pictures/photo.png", b"PNG fixture", false);
    add(&mut zip, "Object 1/content.xml", CHART, false);
    add(
        &mut zip,
        "ObjectReplacements/Object 2",
        b"MZ inert executable fixture",
        false,
    );
    if hostile {
        add(&mut zip, "../escape.bin", b"must not escape", false);
    }
    zip.finish().unwrap().into_inner()
}

fn dispatch(
    selector: &str,
    bytes: Vec<u8>,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("presentation-odf-test").unwrap(),
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
            Some(serde_json::to_value(OdfPresentationOptions::default()).unwrap()),
        )
        .unwrap()
}

#[test]
fn odp_preserves_complete_inert_presentation_structure() {
    let bytes = package(ODP, false, false, false);
    let first = dispatch("odp", bytes.clone(), ResourceBudget::trusted_unbounded());
    let second = dispatch("odp", bytes, ResourceBudget::trusted_unbounded());
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:#?}",
        first.diagnostics
    );
    assert_eq!(first.payload, second.payload);
    let document: OdfPresentationDocument =
        serde_json::from_value(first.payload.clone().unwrap()).unwrap();
    assert_eq!(
        document.package_kind,
        OdfPresentationPackageKind::Presentation
    );
    assert_eq!(document.version.as_deref(), Some("1.3"));
    assert_eq!(document.slides.len(), 2);
    assert_eq!(document.master_pages.len(), 1);
    assert!(!document.styles.is_empty());
    assert!(!document.page_layouts.is_empty());
    assert_eq!(document.metadata[0].value, "Deck metadata");
    let slide = &document.slides[0];
    assert_eq!(slide.shapes.len(), 8);
    assert_eq!(slide.notes[0].text, "Speaker note");
    assert!(slide.comments[0].text.contains("Check this slide"));
    assert_eq!(slide.tables[0].rows.len(), 2);
    assert_eq!(slide.tables[0].rows[0].cells[1].column_span, 2);
    assert_eq!(slide.charts[0].title.as_deref(), Some("Revenue"));
    assert_eq!(slide.charts[0].series.len(), 1);
    assert_eq!(
        slide.images[0].alt_description.as_deref(),
        Some("Photo alt")
    );
    assert!(slide.links[0].external);
    assert!(slide.transition.is_some());
    assert!(slide.animations.len() >= 2);
    assert_eq!(slide.embedded_objects.len(), 2);
    assert!(!slide.raw_elements.is_empty());
    assert_eq!(slide.reading_order.entries[0].shape_id, "title");
    assert!(!document.slides[1].visible);
    assert!(document.embedded_artifacts.iter().any(|artifact| {
        artifact.safety.classification == grist::container::ArtifactSafetyClassification::Executable
    }));
    assert!(matches!(
        slide.shapes[0].locator.components(),
        [
            LocationComponent::ArchiveMember { .. },
            LocationComponent::TextRange { .. },
            LocationComponent::XmlPath { .. },
            LocationComponent::SlideRegion { .. }
        ]
    ));

    let graph = document
        .to_document_graph(DocumentGraphContext::new("odp-graph"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Slide)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Table)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Chart)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Image)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Comment)
    );
    assert!(graph.edges.iter().any(|edge| {
        edge.relation == DocumentRelation::Precedes
            && matches!(edge.evidence, RelationEvidence::Inferred { .. })
    }));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::LinksTo)
    );

    let rendered = render_document_graph(
        &graph,
        RenderFormat::PlainText,
        &RenderOptions::new(FidelityMode::RawFallback),
    )
    .unwrap();
    assert!(rendered.content.contains("ODF Presentation"));
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
    assert!(segments.segments.iter().any(|segment| {
        segment.text.contains("ODF Presentation") && !segment.locators.is_empty()
    }));
    let schema = grist::schema::schema_json("presentation-odf").unwrap();
    let report = grist::schema::validate_against_schema(
        "presentation-odf",
        SchemaVersion::PRESENTATION_ODF_V1,
        first.payload.as_ref().unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
}

#[test]
fn odp_and_otp_detection_registry_and_package_kinds_are_distinct() {
    let registry = builtin_parser_registry().unwrap();
    let odp = registry
        .snapshot()
        .parsers
        .into_iter()
        .find(|parser| parser.format.id == "odp")
        .unwrap();
    assert!(
        odp.capabilities
            .contains(&Capability::DocumentGraphProjection)
    );
    assert!(odp.capabilities.contains(&Capability::EmbeddedArtifacts));
    for (selector, media_type, kind) in [
        ("odp", ODP, OdfPresentationPackageKind::Presentation),
        ("otp", OTP, OdfPresentationPackageKind::Template),
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
        assert_eq!(detection.content_kind, ContentKind::Odp);
        let envelope = dispatch(selector, bytes, ResourceBudget::trusted_unbounded());
        assert_eq!(envelope.status, OperationStatus::Complete);
        let document: OdfPresentationDocument =
            serde_json::from_value(envelope.payload.unwrap()).unwrap();
        assert_eq!(document.package_kind, kind);
    }
}

#[test]
fn encrypted_malformed_hostile_and_budget_cases_are_explicit() {
    let encrypted = dispatch(
        "odp",
        package(ODP, true, false, false),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(encrypted.status, OperationStatus::Encrypted);
    assert!(encrypted.payload.is_none());

    let malformed = dispatch(
        "odp",
        package(ODP, false, false, true),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(malformed.status, OperationStatus::Partial);
    assert!(malformed.payload.is_some());
    assert!(
        malformed
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("malformed XML") })
    );

    let hostile = dispatch(
        "odp",
        package(ODP, false, true, false),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(hostile.status, OperationStatus::Partial);
    let document: OdfPresentationDocument =
        serde_json::from_value(hostile.payload.unwrap()).unwrap();
    assert!(document.parts.iter().any(|part| {
        part.path == "../escape.bin"
            && part.rejection_code.as_deref() == Some("grist.security.archive.path_traversal")
    }));

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_archive_members = Some(1);
    let exhausted = dispatch("odp", package(ODP, false, false, false), budget);
    assert_eq!(exhausted.status, OperationStatus::Failed);
    assert!(exhausted.payload.is_none());
    assert!(exhausted.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_members.exhausted"
    }));

    let broken = dispatch(
        "odp",
        b"not a zip".to_vec(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(broken.status, OperationStatus::Failed);
    assert!(broken.payload.is_none());
}

#[cfg(feature = "cli")]
#[test]
fn cli_parse_routes_odp_to_public_envelope() {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("grist-presentation-{nonce}.odp"));
    fs::write(&path, package(ODP, false, false, false)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "odp", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = fs::remove_file(path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["kind"], "presentation_odf");
    assert_eq!(envelope["payload"]["package_kind"], "presentation");
    assert_eq!(envelope["payload"]["slides"].as_array().unwrap().len(), 2);
    assert_eq!(
        envelope["payload"]["slides"][0]["reading_order"]["entries"][0]["shape_id"],
        "title"
    );
}
