#![cfg(all(
    feature = "presentation-ooxml",
    feature = "document-graph",
    feature = "schemas"
))]

use grist::container::ArtifactExtractionStatus;
use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, LocatorPrecision, OperationStatus,
    ParseRequest, ProviderSet, RequestId, ResourceBudget, SchemaVersion, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::presentation_ooxml::{
    PresentationOoxmlDocument, PresentationOoxmlOptions, PresentationPackageKind,
};
use grist::registry::builtin_parser_registry;
use grist::render::{RenderFormat, RenderOptions, render_document_graph};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const OFFICE_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const CORE_REL: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";

fn dispatch(
    selector: &str,
    bytes: Vec<u8>,
    options: Option<PresentationOoxmlOptions>,
) -> grist::core::Envelope<serde_json::Value> {
    dispatch_with_budget(
        selector,
        bytes,
        options,
        ResourceBudget::trusted_unbounded(),
    )
}

fn dispatch_with_budget(
    selector: &str,
    bytes: Vec<u8>,
    options: Option<PresentationOoxmlOptions>,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("presentation-package").unwrap(),
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
            options.map(|value| serde_json::to_value(value).unwrap()),
        )
        .unwrap()
}

fn package(kind: PresentationPackageKind, hostile: bool) -> Vec<u8> {
    let (main_type, selector) = match kind {
        PresentationPackageKind::Presentation => (
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
            "pptx",
        ),
        PresentationPackageKind::MacroEnabledPresentation => (
            "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml",
            "pptm",
        ),
        PresentationPackageKind::Template => (
            "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml",
            "potx",
        ),
        PresentationPackageKind::Slideshow => (
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml",
            "ppsx",
        ),
    };
    let macro_override = if kind.macro_enabled() {
        r#"<Override PartName="/ppt/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/>"#
    } else {
        ""
    };
    let content_types = format!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/ppt/presentation.xml" ContentType="{main_type}"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/><Override PartName="/ppt/slides/slide2.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/><Override PartName="/ppt/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/><Override PartName="/ppt/notesSlides/notesSlide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/><Override PartName="/ppt/comments/comment1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.comments+xml"/><Override PartName="/ppt/commentAuthors.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.commentAuthors+xml"/><Override PartName="/ppt/embeddings/child.bin" ContentType="application/octet-stream"/>{macro_override}</Types>"#
    );
    let root_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{OFFICE_REL}" Target="ppt/presentation.xml"/><Relationship Id="rId2" Type="{CORE_REL}" Target="docProps/core.xml"/></Relationships>"#
    );
    let presentation = br#"<p:presentation xmlns:p="urn:p" xmlns:r="urn:r"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rMaster"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rSlide1"/><p:sldId id="300" r:id="rSlide2" show="0"/></p:sldIdLst></p:presentation>"#;
    let unsafe_rel = if hostile {
        r#"<Relationship Id="rUnsafe" Type="urn:test:package" Target="../../../escape.bin"/>"#
    } else {
        ""
    };
    let macro_rel = if kind.macro_enabled() {
        r#"<Relationship Id="rMacro" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/>"#
    } else {
        ""
    };
    let presentation_rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rSlide1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/><Relationship Id="rSlide2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/><Relationship Id="rMaster" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>{macro_rel}{unsafe_rel}</Relationships>"#
    );
    let master_rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rLayout" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rTheme" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#;
    let slide1 = br#"<p:sld xmlns:p="urn:p" xmlns:a="urn:a" xmlns:r="urn:r" xmlns:m="urn:m"><p:cSld><p:spTree>
      <p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="100" y="100"/><a:ext cx="5000" cy="800"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="en-US" sz="2400" b="1"><a:latin typeface="Aptos"/><a:solidFill><a:srgbClr val="112233"/></a:solidFill><a:hlinkClick r:id="rLink" action="ppaction://hlinksldjump" tooltip="Go"/></a:rPr><a:t>Quarterly results</a:t></a:r></a:p></p:txBody></p:sp>
      <p:sp><p:nvSpPr><p:cNvPr id="3" name="Body" descr="Body alternative"/></p:nvSpPr><p:spPr><a:xfrm rot="60000"><a:off x="100" y="1200"/><a:ext cx="5000" cy="1600"/></a:xfrm></p:spPr><p:txBody><a:p><a:r><a:t>Net income </a:t></a:r><m:oMath><m:r><m:t>x+y</m:t></m:r></m:oMath></a:p></p:txBody></p:sp>
      <p:pic><p:nvPicPr><p:cNvPr id="4" name="Logo" descr="Company logo" title="Brand"/></p:nvPicPr><p:blipFill><a:blip r:embed="rImage"/><a:srcRect l="100"/></p:blipFill><p:spPr><a:xfrm><a:off x="6000" y="100"/><a:ext cx="1000" cy="1000"/></a:xfrm></p:spPr></p:pic>
      <p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="5" name="Table"/></p:nvGraphicFramePr><p:xfrm><a:off x="100" y="3000"/><a:ext cx="4000" cy="2000"/></p:xfrm><a:graphic><a:graphicData><a:tbl><a:tblPr><a:tableStyleId>style-1</a:tableStyleId></a:tblPr><a:tblGrid><a:gridCol w="2000"/><a:gridCol w="2000"/></a:tblGrid><a:tr h="400"><a:tc gridSpan="2"><a:txBody><a:p><a:r><a:t>Header</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>
      <p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="6" name="Chart"/></p:nvGraphicFramePr><a:graphic><a:graphicData><c:chart xmlns:c="urn:c" r:id="rChart"/></a:graphicData></a:graphic></p:graphicFrame>
      <p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="7" name="Object"/></p:nvGraphicFramePr><a:graphic><a:graphicData><p:oleObj r:id="rObject" progId="Package" name="Data" showAsIcon="1"/></a:graphicData></a:graphic></p:graphicFrame>
    </p:spTree></p:cSld><p:transition advClick="1" advTm="5000" spd="fast"><p:fade/></p:transition><p:timing><p:tnLst><p:seq><p:cTn id="1"><p:childTnLst><p:anim><p:cBhvr><p:tgtEl><p:spTgt spid="3"/></p:tgtEl></p:cBhvr></p:anim></p:childTnLst></p:cTn></p:seq></p:tnLst></p:timing></p:sld>"#;
    let slide1_rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rLink" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/never-fetched" TargetMode="External"/><Relationship Id="rImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/><Relationship Id="rObject" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="../embeddings/child.bin"/><Relationship Id="rChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/><Relationship Id="rNotes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/><Relationship Id="rComments" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments/comment1.xml"/></Relationships>"#;
    let core = br#"<cp:coreProperties xmlns:cp="urn:core" xmlns:dc="urn:dc"><dc:title>Deck title</dc:title><dc:creator>Grist</dc:creator></cp:coreProperties>"#;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let entries: &[(&str, &[u8])] = &[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("ppt/presentation.xml", presentation),
        (
            "ppt/_rels/presentation.xml.rels",
            presentation_rels.as_bytes(),
        ),
        ("ppt/slides/slide1.xml", slide1),
        ("ppt/slides/slide2.xml", br#"<p:sld xmlns:p="urn:p"/>"#),
        ("ppt/slides/_rels/slide1.xml.rels", slide1_rels),
        ("ppt/charts/chart1.xml", br#"<c:chartSpace xmlns:c="urn:c" xmlns:r="urn:r"><c:chart><c:title><c:tx><c:rich><a:p xmlns:a="urn:a"><a:r><a:t>Revenue</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:ser><c:idx val="0"/><c:tx><c:v>FY26</c:v></c:tx><c:cat><c:strLit><c:pt><c:v>Q1</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:pt><c:v>42</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart><c:externalData r:id="rWorkbook"/></c:chartSpace>"#),
        ("ppt/notesSlides/notesSlide1.xml", br#"<p:notes xmlns:p="urn:p" xmlns:a="urn:a"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="10" name="Notes"/></p:nvSpPr><p:txBody><a:p><a:r><a:t>Speaker detail</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#),
        ("ppt/comments/comment1.xml", br#"<p:cmLst xmlns:p="urn:p"><p:cm idx="0" authorId="1" dt="2026-08-07T00:00:00Z"><p:pos x="10" y="20"/><p:text>Review this</p:text></p:cm></p:cmLst>"#),
        ("ppt/commentAuthors.xml", br#"<p:cmAuthorLst xmlns:p="urn:p"><p:cmAuthor id="1" name="Ada" initials="AL"/></p:cmAuthorLst>"#),
        (
            "ppt/slideMasters/slideMaster1.xml",
            br#"<p:sldMaster xmlns:p="urn:p"/>"#,
        ),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels", master_rels),
        (
            "ppt/slideLayouts/slideLayout1.xml",
            br#"<p:sldLayout xmlns:p="urn:p"/>"#,
        ),
        (
            "ppt/theme/theme1.xml",
            br#"<a:theme xmlns:a="urn:a" name="Grist"/>"#,
        ),
        ("docProps/core.xml", core),
        ("ppt/media/image1.png", b"\x89PNG\r\n\x1a\nfixture"),
        ("ppt/embeddings/child.bin", b"EMBEDDED-INERT"),
    ];
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    if kind.macro_enabled() {
        writer.start_file("ppt/vbaProject.bin", options).unwrap();
        writer.write_all(b"VBA-PROJECT-INERT").unwrap();
    }
    if hostile {
        writer.start_file("../escape.bin", options).unwrap();
        writer.write_all(b"must-not-extract").unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    assert_eq!(kind.format_id(), selector);
    bytes
}

#[test]
fn package_retains_order_structure_metadata_actions_and_artifacts() {
    let bytes = package(PresentationPackageKind::Presentation, false);
    let first = dispatch("pptx", bytes.clone(), None);
    let second = dispatch("pptx", bytes.clone(), None);
    assert_eq!(first.status, OperationStatus::Complete);
    assert_eq!(first.payload, second.payload);
    let document: PresentationOoxmlDocument =
        serde_json::from_value(first.payload.unwrap()).unwrap();
    assert_eq!(document.package_kind, PresentationPackageKind::Presentation);
    assert_eq!(
        document
            .slides
            .iter()
            .map(|slide| (slide.slide_id.as_str(), slide.part.as_deref(), slide.hidden))
            .collect::<Vec<_>>(),
        vec![
            ("256", Some("ppt/slides/slide1.xml"), false),
            ("300", Some("ppt/slides/slide2.xml"), true),
        ]
    );
    assert_eq!(
        document.masters[0].part,
        "ppt/slideMasters/slideMaster1.xml"
    );
    assert_eq!(
        document.layouts[0].part,
        "ppt/slideLayouts/slideLayout1.xml"
    );
    assert_eq!(document.themes[0].part, "ppt/theme/theme1.xml");
    assert_eq!(document.properties.core[0].value, "Deck title");
    assert_eq!(document.actions.len(), 1);
    assert!(document.actions[0].external);
    assert_eq!(
        document.actions[0].target.as_deref(),
        Some("https://example.invalid/never-fetched")
    );
    assert!(document.macro_projects.is_empty());
    assert_eq!(document.child_artifacts.len(), 2);
    assert!(document.child_artifacts.iter().all(|item| {
        item.artifact.extraction.status == ArtifactExtractionStatus::InventoryOnly
            && item.artifact.inline_bytes().is_none()
    }));
    assert!(document.slides.iter().all(|slide| {
        slide.part_identity.is_some()
            && matches!(slide.locator.precision(), LocatorPrecision::Exact { .. })
    }));
    let content = &document.slide_contents[0];
    assert_eq!(content.shapes.len(), 6);
    assert_eq!(content.reading_order.entries[0].shape_id, "2");
    assert!(
        content
            .reading_order
            .entries
            .iter()
            .all(|entry| { (0.0..=1.0).contains(&entry.confidence) && !entry.evidence.is_empty() })
    );
    assert_eq!(content.notes[0].text, "Speaker detail");
    assert_eq!(content.comments[0].author_name.as_deref(), Some("Ada"));
    assert_eq!(content.comments[0].text, "Review this");
    assert_eq!(content.tables[0].rows[0].cells[0].grid_span, 2);
    assert_eq!(
        content.tables[0].rows[0].cells[0]
            .text_body
            .as_ref()
            .unwrap()
            .text,
        "Header"
    );
    assert_eq!(content.charts[0].title.as_deref(), Some("Revenue"));
    assert_eq!(content.charts[0].series[0].categories, ["Q1"]);
    assert_eq!(content.charts[0].series[0].values, ["42"]);
    assert_eq!(content.equations[0].text, "x+y");
    assert_eq!(content.images[0].alt_text.as_deref(), Some("Company logo"));
    assert_eq!(
        content.images[0].crop.get("l").map(String::as_str),
        Some("100")
    );
    assert_eq!(
        content.links[0].target.as_deref(),
        Some("https://example.invalid/never-fetched")
    );
    assert_eq!(
        content.transition.as_ref().unwrap().kind.as_deref(),
        Some("fade")
    );
    assert!(
        content
            .animations
            .iter()
            .any(|animation| animation.target_shape_ids == ["3"])
    );
    assert_eq!(
        content.embedded_objects[0].program_id.as_deref(),
        Some("Package")
    );

    let graph = document
        .to_document_graph(DocumentGraphContext::new("presentation-graph"))
        .unwrap();
    let second_document: PresentationOoxmlDocument =
        serde_json::from_value(second.payload.unwrap()).unwrap();
    let second_graph = second_document
        .to_document_graph(DocumentGraphContext::new("presentation-graph"))
        .unwrap();
    assert_eq!(graph, second_graph);
    let slides = graph
        .nodes
        .iter()
        .filter(|node| node.kind == DocumentNodeKind::Slide)
        .collect::<Vec<_>>();
    assert_eq!(slides.len(), 2);
    assert!(graph.edges.iter().any(|edge| {
        edge.source == slides[0].id
            && edge.target == slides[1].id
            && edge.relation == DocumentRelation::Precedes
    }));
    graph.validate_contract().unwrap();
    for kind in [
        DocumentNodeKind::Paragraph,
        DocumentNodeKind::TextRun,
        DocumentNodeKind::Table,
        DocumentNodeKind::Chart,
        DocumentNodeKind::Equation,
        DocumentNodeKind::Image,
        DocumentNodeKind::Comment,
        DocumentNodeKind::Annotation,
    ] {
        assert!(
            graph.nodes.iter().any(|node| node.kind == kind),
            "missing {kind:?}"
        );
    }
    assert!(graph.edges.iter().any(|edge| {
        edge.relation == DocumentRelation::Precedes
            && matches!(
                edge.evidence,
                grist::document_graph::RelationEvidence::Inferred { .. }
            )
    }));
    let rendered =
        render_document_graph(&graph, RenderFormat::PlainText, &RenderOptions::default()).unwrap();
    assert!(rendered.content.contains("Quarterly results"));
    assert!(rendered.content.contains("Header"));
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
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| segment.text.contains("Quarterly results"))
    );
    assert_eq!(
        grist::schema::schema_descriptor("presentation-ooxml")
            .unwrap()
            .schema_version,
        grist::core::SchemaVersion::PRESENTATION_OOXML_V1
    );
    let schema = grist::schema::schema_json("presentation-ooxml").unwrap();
    let report = grist::schema::validate_against_schema(
        "presentation-ooxml",
        SchemaVersion::PRESENTATION_OOXML_V1,
        &serde_json::to_value(&document).unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);

    let detection = detect_with_registry(
        Path::new("extensionless-upload"),
        &bytes,
        None,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.candidates[0].identity.format, "pptx");
}

#[test]
fn all_requested_package_kinds_are_classified_and_dispatched() {
    for kind in [
        PresentationPackageKind::Presentation,
        PresentationPackageKind::MacroEnabledPresentation,
        PresentationPackageKind::Template,
        PresentationPackageKind::Slideshow,
    ] {
        let bytes = package(kind, false);
        let registry = builtin_parser_registry().unwrap();
        let detection = detect_with_registry(
            Path::new("extensionless-upload"),
            &bytes,
            None,
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detection.status, DetectionStatus::Selected);
        assert_eq!(detection.candidates[0].identity.format, kind.format_id());
        let envelope = dispatch(kind.format_id(), bytes, None);
        assert_eq!(envelope.status, OperationStatus::Complete);
        let parsed: PresentationOoxmlDocument =
            serde_json::from_value(envelope.payload.unwrap()).unwrap();
        assert_eq!(parsed.package_kind, kind);
    }
    let mismatch = dispatch(
        "pptx",
        package(PresentationPackageKind::MacroEnabledPresentation, false),
        None,
    );
    assert_eq!(mismatch.status, OperationStatus::Failed);
    assert!(mismatch.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "presentation_ooxml.package_kind_mismatch"
    }));
}

#[test]
fn macro_inventory_is_quarantined_and_optional_bytes_remain_inert() {
    let inventory = dispatch(
        "pptm",
        package(PresentationPackageKind::MacroEnabledPresentation, false),
        None,
    );
    let inventory: PresentationOoxmlDocument =
        serde_json::from_value(inventory.payload.unwrap()).unwrap();
    assert_eq!(inventory.macro_projects.len(), 1);
    assert_eq!(
        inventory.macro_projects[0].artifact.extraction.status,
        ArtifactExtractionStatus::InventoryOnly
    );
    assert!(
        inventory.macro_projects[0]
            .artifact
            .inline_bytes()
            .is_none()
    );

    let extracted = dispatch(
        "pptm",
        package(PresentationPackageKind::MacroEnabledPresentation, false),
        Some(PresentationOoxmlOptions {
            inline_child_artifact_bytes: true,
            extract_macro_bytes: true,
        }),
    );
    let extracted: PresentationOoxmlDocument =
        serde_json::from_value(extracted.payload.unwrap()).unwrap();
    assert_eq!(
        extracted.macro_projects[0].artifact.extraction.status,
        ArtifactExtractionStatus::Quarantined
    );
    assert_eq!(
        extracted.macro_projects[0].artifact.inline_bytes(),
        Some(b"VBA-PROJECT-INERT".as_slice())
    );
    assert!(extracted.child_artifacts.iter().all(|item| {
        item.artifact.extraction.status != ArtifactExtractionStatus::InventoryOnly
            && item.artifact.inline_bytes().is_some()
    }));
    assert!(extracted.child_artifacts.iter().any(|item| {
        item.part.ends_with("child.bin")
            && item.artifact.extraction.status == ArtifactExtractionStatus::Quarantined
    }));
}

#[test]
fn malformed_encrypted_and_hostile_packages_are_explicit() {
    let malformed = dispatch("pptx", b"PK\x03\x04truncated".to_vec(), None);
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(malformed.payload.is_none());

    let mut encrypted = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    encrypted.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
    let encrypted = dispatch("pptx", encrypted, None);
    assert_eq!(encrypted.status, OperationStatus::Encrypted);
    assert!(encrypted.payload.is_none());

    let hostile = dispatch(
        "pptx",
        package(PresentationPackageKind::Presentation, true),
        None,
    );
    assert_eq!(hostile.status, OperationStatus::Partial);
    let document: PresentationOoxmlDocument =
        serde_json::from_value(hostile.payload.unwrap()).unwrap();
    assert!(document.parts.iter().any(|part| {
        part.path == "../escape.bin"
            && part.rejection_code.as_deref() == Some("grist.security.archive.path_traversal")
    }));
    assert!(document.relationships.iter().any(|relationship| {
        relationship.id == "rUnsafe"
            && relationship.resolved_part.is_none()
            && relationship.target_exists == Some(false)
    }));

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_archive_members = Some(1);
    let exhausted = dispatch_with_budget(
        "pptx",
        package(PresentationPackageKind::Presentation, false),
        None,
        budget,
    );
    assert_eq!(exhausted.status, OperationStatus::Failed);
    assert!(exhausted.payload.is_none());
    assert!(exhausted.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_members.exhausted"
    }));
}

#[cfg(feature = "cli")]
#[test]
fn cli_parse_routes_pptx_to_the_public_envelope() {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("grist-presentation-{nonce}.pptx"));
    fs::write(&path, package(PresentationPackageKind::Presentation, false)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "pptx", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = fs::remove_file(path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["kind"], "presentation_ooxml");
    assert_eq!(envelope["payload"]["package_kind"], "presentation");
    assert_eq!(envelope["payload"]["slides"].as_array().unwrap().len(), 2);
    assert_eq!(
        envelope["payload"]["slide_contents"][0]["shapes"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert_eq!(
        envelope["payload"]["slide_contents"][0]["reading_order"]["entries"][0]["shape_id"],
        "2"
    );
}
