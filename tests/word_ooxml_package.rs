#![cfg(all(
    feature = "word-ooxml",
    feature = "document-graph",
    feature = "schemas"
))]

use grist::container::ArtifactExtractionStatus;
use grist::core::{
    BudgetSelection, Input, LocationComponent, OperationStatus, ParseRequest, ProviderSet,
    RequestId, ResourceBudget, SchemaVersion, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::builtin_parser_registry;
use grist::render::{RenderFormat, RenderOptions, render_document_graph};
use grist::segment::{SegmentOptions, segment_document_graph};
use grist::word_ooxml::{
    WordBlock, WordDrawingKind, WordInline, WordOoxmlDocument, WordOoxmlOptions, WordPackageKind,
    WordRevisionKind,
};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const OFFICE_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const CORE_REL: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const EXT_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
const CUSTOM_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";

fn dispatch(
    selector: &str,
    bytes: Vec<u8>,
    options: Option<WordOoxmlOptions>,
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
    options: Option<WordOoxmlOptions>,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("word-package").unwrap(),
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

fn package(kind: WordPackageKind, unsafe_relationship: bool, unsafe_member: bool) -> Vec<u8> {
    package_with_core(
        kind,
        unsafe_relationship,
        unsafe_member,
        br#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Package title</dc:title><dc:creator>Grist</dc:creator></cp:coreProperties>"#,
    )
}

fn package_with_core(
    kind: WordPackageKind,
    unsafe_relationship: bool,
    unsafe_member: bool,
    core: &[u8],
) -> Vec<u8> {
    let main_type = match kind {
        WordPackageKind::Document => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
        }
        WordPackageKind::MacroEnabledDocument => {
            "application/vnd.ms-word.document.macroEnabled.main+xml"
        }
        WordPackageKind::Template => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml"
        }
        WordPackageKind::MacroEnabledTemplate => {
            "application/vnd.ms-word.template.macroEnabledTemplate.main+xml"
        }
    };
    let macro_override = if kind.macro_enabled() {
        r#"<Override PartName="/word/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/>"#
    } else {
        ""
    };
    let content_types = format!(
        r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="{main_type}"/><Override PartName="/word/embeddings/child.pdf" ContentType="application/pdf"/>{macro_override}</Types>"#,
    );
    let root_relationships = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{OFFICE_REL}" Target="word/document.xml"/><Relationship Id="rId2" Type="{CORE_REL}" Target="docProps/core.xml"/><Relationship Id="rId3" Type="{EXT_REL}" Target="docProps/app.xml"/><Relationship Id="rId4" Type="{CUSTOM_REL}" Target="docProps/custom.xml"/></Relationships>"#,
    );
    let unsafe_target = if unsafe_relationship {
        r#"<Relationship Id="rUnsafe" Type="urn:test:package" Target="../../../escape.bin"/>"#
    } else {
        ""
    };
    let macro_relationship = if kind.macro_enabled() {
        r#"<Relationship Id="rMacro" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/>"#
    } else {
        ""
    };
    let document_relationships = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rObject" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="embeddings/child.pdf"/><Relationship Id="rExternal" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/never-fetched" TargetMode="External"/>{macro_relationship}{unsafe_target}</Relationships>"#,
    );
    let extended = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Word</Application><Pages>2</Pages></Properties>"#;
    let custom = br#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="ReviewState"><vt:lpwstr>Ready</vt:lpwstr></property></Properties>"#;

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in [
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_relationships.as_bytes()),
        (
            "word/document.xml",
            b"<w:document xmlns:w=\"urn:w\"/>".as_slice(),
        ),
        (
            "word/_rels/document.xml.rels",
            document_relationships.as_bytes(),
        ),
        ("docProps/core.xml", core),
        ("docProps/app.xml", extended.as_slice()),
        ("docProps/custom.xml", custom.as_slice()),
        (
            "word/media/image1.png",
            b"\x89PNG\r\n\x1a\nfixture".as_slice(),
        ),
        ("word/embeddings/child.pdf", b"%PDF-1.7\nchild".as_slice()),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    if kind.macro_enabled() {
        writer.start_file("word/vbaProject.bin", options).unwrap();
        writer.write_all(b"VBA-PROJECT-INERT-BYTES").unwrap();
    }
    if unsafe_member {
        writer.start_file("../escape.bin", options).unwrap();
        writer.write_all(b"must-not-extract").unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn complex_content_package() -> Vec<u8> {
    let content_types = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let root_relationships = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{OFFICE_REL}" Target="word/document.xml"/></Relationships>"#
    );
    let relationships = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
      <Relationship Id="rStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
      <Relationship Id="rNums" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>
      <Relationship Id="rNotes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/>
      <Relationship Id="rEndNotes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes" Target="endnotes.xml"/>
      <Relationship Id="rHeader" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>
      <Relationship Id="rFooter" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>
      <Relationship Id="rLink" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/reference" TargetMode="External"/>
    </Relationships>"#;
    let styles = br#"<w:styles xmlns:w="urn:w">
      <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:b/></w:rPr></w:style>
      <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="Heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:i/></w:rPr></w:style>
    </w:styles>"#;
    let numbering = br#"<w:numbering xmlns:w="urn:w">
      <w:abstractNum w:abstractNumId="2"><w:multiLevelType w:val="multilevel"/>
        <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:suff w:val="space"/></w:lvl>
      </w:abstractNum>
      <w:num w:numId="7"><w:abstractNumId w:val="2"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="3"/></w:lvlOverride></w:num>
    </w:numbering>"#;
    let document = br#"<w:document xmlns:w="urn:w" xmlns:r="urn:r"><w:body>
      <w:p><w:pPr><w:pStyle w:val="Heading1"/><w:jc w:val="center"/></w:pPr><w:bookmarkStart w:id="9" w:name="Target"/>
        <w:r><w:rPr><w:color w:val="FF0000"/></w:rPr><w:t>Heading</w:t></w:r>
        <w:hyperlink r:id="rLink" w:tooltip="Reference"><w:r><w:t xml:space="preserve"> link</w:t></w:r></w:hyperlink>
        <w:fldSimple w:instr=" CITATION Smith2024 \l 1033 "><w:r><w:t>[1]</w:t></w:r></w:fldSimple>
        <w:bookmarkEnd w:id="9"/></w:p>
      <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr><w:r><w:t>First</w:t></w:r></w:p>
      <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr></w:pPr>
        <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> REF Target </w:instrText></w:r>
        <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Heading</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
        <w:r><w:br w:type="page"/><w:footnoteReference w:id="2"/><w:endnoteReference w:id="3"/></w:r></w:p>
      <w:tbl><w:tblPr><w:tblStyle w:val="Grid"/><w:tblW w:w="5000"/><w:tblLayout w:val="fixed"/></w:tblPr>
        <w:tblGrid><w:gridCol w:w="2500"/><w:gridCol w:w="2500"/></w:tblGrid>
        <w:tr><w:trPr><w:tblHeader/></w:trPr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>Merged</w:t></w:r></w:p>
          <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Nested</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:tc></w:tr>
        <w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge/></w:tcPr><w:p><w:r><w:t>Continuation</w:t></w:r></w:p></w:tc></w:tr>
      </w:tbl>
      <w:p><w:pPr><w:sectPr><w:type w:val="nextPage"/><w:cols w:num="2" w:space="720"/><w:headerReference w:type="default" r:id="rHeader"/><w:footerReference w:type="default" r:id="rFooter"/></w:sectPr></w:pPr><w:r><w:t>Section end</w:t></w:r></w:p>
    </w:body></w:document>"#;
    let footnotes = br#"<w:footnotes xmlns:w="urn:w"><w:footnote w:id="2"><w:p><w:r><w:t>Footnote text</w:t></w:r></w:p></w:footnote></w:footnotes>"#;
    let endnotes = br#"<w:endnotes xmlns:w="urn:w"><w:endnote w:id="3"><w:p><w:r><w:t>Endnote text</w:t></w:r></w:p></w:endnote></w:endnotes>"#;
    let header = br#"<w:hdr xmlns:w="urn:w"><w:p><w:r><w:t>Header text</w:t></w:r></w:p></w:hdr>"#;
    let footer = br#"<w:ftr xmlns:w="urn:w"><w:p><w:r><w:t>Footer text</w:t></w:r></w:p></w:ftr>"#;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in [
        ("[Content_Types].xml", content_types.as_slice()),
        ("_rels/.rels", root_relationships.as_bytes()),
        ("word/document.xml", document.as_slice()),
        ("word/_rels/document.xml.rels", relationships.as_slice()),
        ("word/styles.xml", styles.as_slice()),
        ("word/numbering.xml", numbering.as_slice()),
        ("word/footnotes.xml", footnotes.as_slice()),
        ("word/endnotes.xml", endnotes.as_slice()),
        ("word/header1.xml", header.as_slice()),
        ("word/footer1.xml", footer.as_slice()),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn rich_content_package(unpaired_move: bool) -> Vec<u8> {
    let content_types = br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
      <Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
      <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
      <Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>
      <Override PartName="/word/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>
      <Override PartName="/word/embeddings/book.xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/>
    </Types>"#;
    let root_relationships = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{OFFICE_REL}" Target="word/document.xml"/></Relationships>"#
    );
    let relationships = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
      <Relationship Id="rComments" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>
      <Relationship Id="rChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/>
      <Relationship Id="rEmbed" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="embeddings/book.xlsx"/>
    </Relationships>"#;
    let move_to = if unpaired_move {
        ""
    } else {
        r#"<w:moveTo w:id="3" w:author="Reviewer"><w:r><w:t>destination</w:t></w:r></w:moveTo>"#
    };
    let document = format!(
        r#"<w:document xmlns:w="urn:w" xmlns:w14="urn:w14" xmlns:r="urn:r" xmlns:m="urn:m" xmlns:wp="urn:wp" xmlns:a="urn:a" xmlns:c="urn:c" xmlns:wps="urn:wps" xmlns:o="urn:o" xmlns:v="urn:v"><w:body>
          <w:p><w:pPr><w:pPrChange w:id="4" w:author="Reviewer"><w:pPr><w:jc w:val="left"/></w:pPr></w:pPrChange></w:pPr>
            <w:commentRangeStart w:id="0"/><w:del w:id="1" w:author="Reviewer" w:date="2026-01-01T00:00:00Z"><w:r><w:delText>old</w:delText></w:r></w:del>
            <w:ins w:id="2" w:author="Reviewer"><w:r><w:t>new</w:t></w:r></w:ins>
            <w:moveFrom w:id="3" w:author="Reviewer"><w:r><w:delText>source</w:delText></w:r></w:moveFrom>{move_to}
            <w:moveFromRangeStart w:id="5"/><w:r><w:t>ranged</w:t></w:r><w:moveFromRangeEnd w:id="5"/>
            <w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>
          <w:sdt><w:sdtPr><w:id w:val="42"/><w:alias w:val="Customer"/><w:tag w:val="customer-name"/><w:text/><w:dataBinding w:xpath="/root/name" w:storeItemID="store-1"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Controlled</w:t></w:r></w:p></w:sdtContent></w:sdt>
          <w:p><w:r><m:oMath><m:r><m:t>x+1</m:t></m:r></m:oMath></w:r>
            <w:r><w:drawing><wp:inline><wp:docPr id="7" name="Sales chart" descr="Quarterly sales chart"/><a:graphic><a:graphicData><c:chart r:id="rChart"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>
          <w:p><w:pPr><w:pStyle w:val="Caption"/></w:pPr><w:r><w:t>Figure 1: Sales</w:t></w:r></w:p>
          <w:p><w:r><w:object><v:shape><v:textbox><w:txbxContent><w:p><w:r><w:t>Box text</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape><o:OLEObject r:id="rEmbed" ProgID="Excel.Sheet.12" Type="Embed" DrawAspect="Content" ShapeID="shape1"/></w:object></w:r></w:p>
        </w:body></w:document>"#
    );
    let comments = br#"<w:comments xmlns:w="urn:w" xmlns:w14="urn:w14">
      <w:comment w:id="0" w:author="Ada" w:initials="AL" w:date="2026-01-01T00:00:00Z"><w:p w14:paraId="AAAA0001"><w:r><w:t>Root comment</w:t></w:r></w:p></w:comment>
      <w:comment w:id="1" w:author="Ben"><w:p w14:paraId="BBBB0002"><w:r><w:t>Reply comment</w:t></w:r></w:p></w:comment>
    </w:comments>"#;
    let comments_extended = br#"<w15:commentsEx xmlns:w15="urn:w15"><w15:commentEx w15:paraId="AAAA0001" w15:done="1"/><w15:commentEx w15:paraId="BBBB0002" w15:paraIdParent="AAAA0001"/></w15:commentsEx>"#;
    let comments_ids = br#"<w16cid:commentsIds xmlns:w16cid="urn:w16cid"><w16cid:commentId w16cid:paraId="AAAA0001" w16cid:durableId="DURABLE-ROOT"/><w16cid:commentId w16cid:paraId="BBBB0002" w16cid:durableId="DURABLE-REPLY"/></w16cid:commentsIds>"#;
    let chart = br#"<c:chartSpace xmlns:c="urn:c" xmlns:a="urn:a"><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>Quarterly sales</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:ser><c:tx><c:v>Revenue</c:v></c:tx></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let members: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", content_types.as_slice()),
        ("_rels/.rels", root_relationships.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", relationships.as_slice()),
        ("word/comments.xml", comments.as_slice()),
        ("word/commentsExtended.xml", comments_extended.as_slice()),
        ("word/commentsIds.xml", comments_ids.as_slice()),
        ("word/charts/chart1.xml", chart.as_slice()),
        (
            "word/embeddings/book.xlsx",
            b"PK\x03\x04inert-child-package",
        ),
    ];
    for (name, bytes) in members {
        writer.start_file(name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn docx_package_retains_parts_relationships_properties_artifacts_and_exact_locators() {
    let bytes = package(WordPackageKind::Document, false, false);
    let first = dispatch("docx", bytes.clone(), None);
    let second = dispatch("docx", bytes.clone(), None);
    assert_eq!(first.status, OperationStatus::Complete);
    assert_eq!(first, second, "package parsing must be deterministic");
    let document: WordOoxmlDocument =
        serde_json::from_value(first.payload.clone().unwrap()).unwrap();
    assert_eq!(document.package_kind, WordPackageKind::Document);
    assert_eq!(document.main_document_part, "word/document.xml");
    assert!(document.parts.iter().all(|part| matches!(
        part.locator.components().first(),
        Some(LocationComponent::OoxmlPart { part: locator_part, .. }) if locator_part == &part.path
    )));
    assert!(document.relationships.iter().all(|relationship| matches!(
        relationship.locator.components(),
        [LocationComponent::OoxmlPart { part, .. }, LocationComponent::XmlPath { path }]
            if part == &relationship.relationship_part && path.starts_with('/')
    )));
    assert!(document.relationships.iter().any(|relationship| {
        relationship.id == "rExternal"
            && relationship.resolved_part.is_none()
            && relationship.target_exists.is_none()
    }));
    assert_eq!(document.properties.core[0].name, "title");
    assert_eq!(document.properties.core[0].value, "Package title");
    assert_eq!(
        document.properties.custom[0].name.as_deref(),
        Some("ReviewState")
    );
    assert_eq!(
        document.properties.custom[0].value.as_deref(),
        Some("Ready")
    );
    assert_eq!(document.child_artifacts.len(), 2);
    assert!(document.child_artifacts.iter().all(|child| {
        child.artifact.extraction.status == ArtifactExtractionStatus::InventoryOnly
            && child.artifact.content.is_none()
    }));

    let graph = document
        .to_document_graph(
            DocumentGraphContext::new("word:fixture").with_source(first.source.clone()),
        )
        .unwrap();
    graph.validate_contract().unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.kind == DocumentNodeKind::Metadata && node.text.as_deref() == Some("Package title")
    }));
    assert!(graph.edges.iter().any(|edge| {
        edge.relation == DocumentRelation::LinksTo
            && edge.target == "https://example.invalid/never-fetched"
    }));
    let source_identity = first.identity.as_ref().unwrap();
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
    let rendered =
        render_document_graph(&graph, RenderFormat::PlainText, &RenderOptions::default()).unwrap();
    assert!(rendered.content.contains("Package title"));

    let payload_schema = grist::schema::schema_json("word-ooxml").unwrap();
    let report = grist::schema::validate_against_schema(
        "word-ooxml",
        SchemaVersion::WORD_OOXML_V1,
        first.payload.as_ref().unwrap(),
        &payload_schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
}

#[test]
fn wordprocessingml_content_preserves_styles_lists_tables_and_references() {
    let envelope = dispatch("docx", complex_content_package(), None);
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:#?}",
        envelope.diagnostics
    );
    let document: WordOoxmlDocument =
        serde_json::from_value(envelope.payload.clone().unwrap()).unwrap();
    assert_eq!(
        document.body.visible_text.lines().next(),
        Some("Heading link[1]")
    );
    let heading = match &document.body.blocks[0] {
        WordBlock::Paragraph(paragraph) => paragraph,
        _ => panic!("expected heading paragraph"),
    };
    assert_eq!(heading.heading_level, Some(1));
    assert_eq!(
        heading.direct_properties.alignment.as_deref(),
        Some("center")
    );
    assert_eq!(heading.style_name.as_deref(), Some("Heading 1"));
    let heading_run = match &heading.inlines[1] {
        WordInline::Run(run) => run,
        _ => panic!("expected producing run"),
    };
    assert_eq!(heading_run.effective_formatting.bold, Some(true));
    assert_eq!(heading_run.effective_formatting.italic, Some(true));
    assert_eq!(
        heading_run.effective_formatting.color.as_deref(),
        Some("FF0000")
    );
    assert_eq!(heading.citations[0].tags, ["Smith2024"]);
    assert!(heading.inlines.iter().any(|inline| matches!(
        inline,
        WordInline::Hyperlink(link)
            if link.target.as_deref() == Some("https://example.invalid/reference")
    )));

    let first = match &document.body.blocks[1] {
        WordBlock::Paragraph(paragraph) => paragraph,
        _ => panic!("expected list paragraph"),
    };
    let second = match &document.body.blocks[2] {
        WordBlock::Paragraph(paragraph) => paragraph,
        _ => panic!("expected list paragraph"),
    };
    assert_eq!(first.numbering.as_ref().unwrap().label, "3.");
    assert_eq!(first.numbering_reference.as_ref().unwrap().num_id, 7);
    assert_eq!(second.numbering.as_ref().unwrap().label, "4.");
    assert_eq!(second.cross_references[0].target, "Target");

    let table = match &document.body.blocks[3] {
        WordBlock::Table(table) => table,
        _ => panic!("expected table"),
    };
    assert!(table.rows[0].is_header);
    assert_eq!(table.rows[0].cells[0].column_span, 2);
    assert_eq!(table.rows[0].cells[0].row_span, 2);
    assert_eq!(table.rows[1].cells[0].merged_into, Some((1, 1)));
    assert!(matches!(
        table.rows[0].cells[0].blocks[1],
        WordBlock::Table(_)
    ));
    assert_eq!(document.body.sections.len(), 0);
    let section_paragraph = match &document.body.blocks[4] {
        WordBlock::Paragraph(paragraph) => paragraph,
        _ => panic!("expected section paragraph"),
    };
    let section = section_paragraph.section.as_ref().unwrap();
    assert_eq!(section.columns.count, 2);
    assert_eq!(
        section.header_references[0].part.as_deref(),
        Some("word/header1.xml")
    );
    assert_eq!(document.footnotes[0].story.visible_text, "Footnote text");
    assert_eq!(document.endnotes[0].story.visible_text, "Endnote text");
    assert_eq!(document.headers[0].story.visible_text, "Header text");
    assert_eq!(document.footers[0].story.visible_text, "Footer text");

    let graph = document
        .to_document_graph(
            DocumentGraphContext::new("word:content").with_source(envelope.source.clone()),
        )
        .unwrap();
    graph.validate_contract().unwrap();
    for kind in [
        DocumentNodeKind::Heading,
        DocumentNodeKind::TextRun,
        DocumentNodeKind::List,
        DocumentNodeKind::ListItem,
        DocumentNodeKind::Table,
        DocumentNodeKind::TableCell,
        DocumentNodeKind::Link,
        DocumentNodeKind::Bookmark,
        DocumentNodeKind::Citation,
        DocumentNodeKind::Footnote,
        DocumentNodeKind::Endnote,
        DocumentNodeKind::Header,
        DocumentNodeKind::Footer,
    ] {
        assert!(
            graph.nodes.iter().any(|node| node.kind == kind),
            "missing {kind:?}"
        );
    }
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(
                node.kind,
                DocumentNodeKind::Heading
                    | DocumentNodeKind::Paragraph
                    | DocumentNodeKind::TextRun
                    | DocumentNodeKind::List
                    | DocumentNodeKind::ListItem
                    | DocumentNodeKind::Table
                    | DocumentNodeKind::TableRow
                    | DocumentNodeKind::TableCell
                    | DocumentNodeKind::Link
                    | DocumentNodeKind::Bookmark
                    | DocumentNodeKind::Citation
                    | DocumentNodeKind::Reference
                    | DocumentNodeKind::Footnote
                    | DocumentNodeKind::Endnote
                    | DocumentNodeKind::Header
                    | DocumentNodeKind::Footer
            ))
            .all(|node| node.locator.is_some())
    );
    let rendered =
        render_document_graph(&graph, RenderFormat::PlainText, &RenderOptions::default()).unwrap();
    assert!(rendered.content.contains("Heading link[1]"));
    assert!(rendered.content.contains("Nested"));
    let source_identity = envelope.identity.as_ref().unwrap();
    let graph_identity = grist::core::ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        source_identity,
        &graph_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .all(|segment| !segment.locators.is_empty())
    );
}

#[test]
fn revisions_comments_controls_and_rich_objects_are_authoritative_and_projectable() {
    let envelope = dispatch(
        "docx",
        rich_content_package(false),
        Some(WordOoxmlOptions {
            inline_child_artifact_bytes: true,
            extract_macro_bytes: false,
        }),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:#?}",
        envelope.diagnostics
    );
    let document: WordOoxmlDocument =
        serde_json::from_value(envelope.payload.clone().unwrap()).unwrap();

    assert_eq!(document.revision_graph.revisions.len(), 7);
    assert!(document.revision_graph.revisions.iter().any(|revision| {
        revision.revision_kind == WordRevisionKind::ParagraphProperties
            && !revision.content.children.is_empty()
    }));
    assert!(
        document
            .revision_graph
            .edges
            .iter()
            .any(|edge| { edge.relation == grist::word_ooxml::WordRevisionRelation::MovePair })
    );
    assert!(
        document
            .revision_graph
            .edges
            .iter()
            .any(|edge| { edge.relation == grist::word_ooxml::WordRevisionRelation::RangePair })
    );
    let projection = |view: &Vec<grist::word_ooxml::WordStoryTextProjection>| {
        view.iter()
            .find(|projection| projection.story_part == "word/document.xml")
            .unwrap()
            .text
            .clone()
    };
    let original = projection(&document.revision_graph.projections.original);
    assert!(original.contains("oldnewsource"));
    assert!(original.contains("destination"));
    let accepted = projection(&document.revision_graph.projections.accepted);
    assert!(accepted.contains("newdestination"));
    assert!(!accepted.contains("old"));
    assert!(!accepted.contains("source"));
    let rejected = projection(&document.revision_graph.projections.rejected);
    assert!(rejected.contains("oldsource"));
    assert!(!rejected.contains("new"));
    assert!(!rejected.contains("destination"));

    assert_eq!(document.comments.len(), 2);
    assert_eq!(
        document.comments[0].durable_id.as_deref(),
        Some("DURABLE-ROOT")
    );
    assert_eq!(document.comments[0].reply_ids, ["1"]);
    assert_eq!(document.comments[1].parent_comment_id.as_deref(), Some("0"));
    assert_eq!(document.comments[0].anchors.len(), 3);
    assert_eq!(document.content_controls[0].control_type, "text");
    assert_eq!(document.content_controls[0].text, "Controlled");
    assert_eq!(document.equations[0].text, "x+1");
    assert!(document.drawings.iter().any(|drawing| {
        drawing.drawing_kind == WordDrawingKind::Chart
            && drawing.alt_text.as_deref() == Some("Quarterly sales chart")
    }));
    assert_eq!(document.charts[0].chart_types, ["barChart"]);
    assert_eq!(document.charts[0].title.as_deref(), Some("Quarterly sales"));
    assert_eq!(document.captions[0].text, "Figure 1: Sales");
    assert!(document.captions[0].target_object_id.is_some());
    assert_eq!(document.text_boxes[0].text, "Box text");
    assert_eq!(
        document.embedded_objects[0].child_artifact_part.as_deref(),
        Some("word/embeddings/book.xlsx")
    );
    let embedded_child = document
        .child_artifacts
        .iter()
        .find(|artifact| artifact.part == "word/embeddings/book.xlsx")
        .unwrap();
    assert_eq!(
        embedded_child.artifact.extraction.status,
        ArtifactExtractionStatus::Quarantined
    );
    assert!(embedded_child.artifact.inline_bytes().is_some());

    let graph = document
        .to_document_graph(
            DocumentGraphContext::new("word:rich").with_source(envelope.source.clone()),
        )
        .unwrap();
    graph.validate_contract().unwrap();
    for kind in [
        DocumentNodeKind::Revision,
        DocumentNodeKind::Comment,
        DocumentNodeKind::ContentControl,
        DocumentNodeKind::Equation,
        DocumentNodeKind::Figure,
        DocumentNodeKind::Chart,
        DocumentNodeKind::Caption,
        DocumentNodeKind::Attachment,
    ] {
        assert!(graph.nodes.iter().any(|node| node.kind == kind), "{kind:?}");
    }
    for relation in [
        DocumentRelation::RevisionOf,
        DocumentRelation::ReplyTo,
        DocumentRelation::CaptionFor,
        DocumentRelation::EmbeddedIn,
    ] {
        assert!(
            graph.edges.iter().any(|edge| edge.relation == relation),
            "{relation:?}"
        );
    }
}

#[test]
fn incomplete_revision_relationships_are_named_loss_diagnostics() {
    let envelope = dispatch("docx", rich_content_package(true), None);
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(envelope.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "word_ooxml.revision.unpaired_move" && diagnostic.partial
    }));
    let document: WordOoxmlDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert!(document.revision_graph.revisions.iter().any(|revision| {
        revision.revision_kind == WordRevisionKind::MoveFrom && revision.text == "source"
    }));
}

#[test]
fn all_four_word_package_kinds_are_detected_and_dispatched_exactly() {
    for kind in [
        WordPackageKind::Document,
        WordPackageKind::MacroEnabledDocument,
        WordPackageKind::Template,
        WordPackageKind::MacroEnabledTemplate,
    ] {
        let bytes = package(kind, false, false);
        let registry = builtin_parser_registry().unwrap();
        let detection = detect_with_registry(
            Path::new("extensionless-upload"),
            &bytes,
            None,
            None,
            &grist::core::Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detection.status, DetectionStatus::Selected);
        assert_eq!(detection.candidates[0].identity.format, kind.format_id());
        let envelope = dispatch(kind.format_id(), bytes, None);
        assert_eq!(envelope.status, OperationStatus::Complete);
        let parsed: WordOoxmlDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
        assert_eq!(parsed.package_kind, kind);
    }
    let mismatch = dispatch(
        "docx",
        package(WordPackageKind::MacroEnabledDocument, false, false),
        None,
    );
    assert_eq!(mismatch.status, OperationStatus::Failed);
    assert!(
        mismatch
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "word_ooxml.package_kind_mismatch" })
    );
}

#[test]
fn macros_are_inventory_only_by_default_and_quarantined_when_requested() {
    let bytes = package(WordPackageKind::MacroEnabledDocument, false, false);
    let inventory = dispatch("docm", bytes.clone(), None);
    let inventory: WordOoxmlDocument = serde_json::from_value(inventory.payload.unwrap()).unwrap();
    assert_eq!(inventory.macro_projects.len(), 1);
    assert_eq!(
        inventory.macro_projects[0].artifact.extraction.status,
        ArtifactExtractionStatus::InventoryOnly
    );
    assert!(inventory.macro_projects[0].artifact.content.is_none());

    let extracted = dispatch(
        "docm",
        bytes,
        Some(WordOoxmlOptions {
            extract_macro_bytes: true,
            inline_child_artifact_bytes: true,
        }),
    );
    let extracted: WordOoxmlDocument = serde_json::from_value(extracted.payload.unwrap()).unwrap();
    let macro_project = &extracted.macro_projects[0];
    assert_eq!(
        macro_project.artifact.extraction.status,
        ArtifactExtractionStatus::Quarantined
    );
    assert_eq!(
        macro_project.artifact.inline_bytes(),
        Some(b"VBA-PROJECT-INERT-BYTES".as_slice())
    );
    assert!(extracted.child_artifacts.iter().all(|artifact| {
        artifact.artifact.extraction.status == ArtifactExtractionStatus::Extracted
            && artifact.artifact.inline_bytes().is_some()
    }));
}

#[test]
fn malformed_encrypted_and_hostile_packages_fail_or_degrade_explicitly() {
    let malformed = dispatch("docx", b"PK\x03\x04truncated".to_vec(), None);
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(malformed.payload.is_none());

    let mut encrypted = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1".to_vec();
    encrypted.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
    let encrypted = dispatch("docx", encrypted, None);
    assert_eq!(encrypted.status, OperationStatus::Encrypted);
    assert!(encrypted.payload.is_none());

    let hostile = dispatch("docx", package(WordPackageKind::Document, true, true), None);
    assert_eq!(hostile.status, OperationStatus::Partial);
    let payload: WordOoxmlDocument =
        serde_json::from_value(hostile.payload.clone().unwrap()).unwrap();
    assert!(payload.parts.iter().any(|part| {
        part.path == "../escape.bin"
            && part.rejection_code.as_deref() == Some("grist.security.archive.path_traversal")
    }));
    assert!(payload.relationships.iter().any(|relationship| {
        relationship.id == "rUnsafe"
            && relationship.resolved_part.is_none()
            && relationship.target_exists == Some(false)
    }));
    assert!(
        hostile.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "word_ooxml.relationship.unsafe_target"
        })
    );

    let active_xml = dispatch(
        "docx",
        package_with_core(
            WordPackageKind::Document,
            false,
            false,
            br#"<!DOCTYPE x [<!ENTITY leak SYSTEM "file:///secret">]><cp:coreProperties xmlns:cp="urn:core"><cp:title>&leak;</cp:title></cp:coreProperties>"#,
        ),
        None,
    );
    assert_eq!(active_xml.status, OperationStatus::Partial);
    assert!(active_xml.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.security.xml.doctype"
            || diagnostic.code.as_str() == "grist.security.xml.entity_declaration"
    }));
    let active_xml: WordOoxmlDocument =
        serde_json::from_value(active_xml.payload.unwrap()).unwrap();
    assert!(active_xml.properties.core.is_empty());

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_archive_members = Some(1);
    let exhausted = dispatch_with_budget(
        "docx",
        package(WordPackageKind::Document, false, false),
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
fn cli_parse_routes_docx_to_the_same_public_envelope() {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("grist-word-package-{nonce}.docx"));
    fs::write(&path, complex_content_package()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "docx", path.to_str().unwrap()])
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
    assert_eq!(envelope["kind"], "word_ooxml");
    assert_eq!(envelope["payload"]["package_kind"], "document");
    assert_eq!(
        envelope["payload"]["body"]["visible_text"]
            .as_str()
            .unwrap()
            .lines()
            .next(),
        Some("Heading link[1]")
    );
}
