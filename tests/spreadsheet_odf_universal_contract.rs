#![cfg(all(
    feature = "spreadsheet-odf",
    feature = "document-graph",
    feature = "schemas"
))]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, LocationComponent, OperationStatus,
    ParseRequest, ProviderSet, RequestId, ResourceBudget, SchemaVersion, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::builtin_parser_registry;
use grist::segment::{SegmentOptions, segment_document_graph};
use grist::spreadsheet_odf::{
    SpreadsheetOdfDocument, SpreadsheetOdfObjectKind, SpreadsheetOdfPackageKind,
    SpreadsheetOdfVisibility,
};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";
const OTS: &str = "application/vnd.oasis.opendocument.spreadsheet-template";

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

fn package(
    media_type: &str,
    encrypted: bool,
    hostile: bool,
    malformed: bool,
    extra_rows: usize,
) -> Vec<u8> {
    let more = (0..extra_rows)
        .map(|index| {
            format!(
                r#"<table:table-row><table:table-cell office:value-type="float" office:value="{index}"><text:p>{index}</text:p></table:table-cell></table:table-row>"#
            )
        })
        .collect::<String>();
    let content = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content office:version="1.3"
 xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
 xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
 xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:xlink="http://www.w3.org/1999/xlink"
 xmlns:dc="http://purl.org/dc/elements/1.1/"
 xmlns:calcext="urn:example:calcext">
 <office:automatic-styles>
  <style:style style:name="HiddenTable" style:family="table"><style:table-properties table:display="false"/></style:style>
  <style:style style:name="Wide" style:family="table-column"><style:table-column-properties style:column-width="4cm"/></style:style>
  <style:style style:name="Tall" style:family="table-row"><style:table-row-properties style:row-height="1cm"/></style:style>
  <style:style style:name="Percent" style:family="table-cell" style:data-style-name="N1"><style:table-cell-properties fo:background-color="#ffff00" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"/></style:style>
 </office:automatic-styles>
 <office:body><office:spreadsheet>
  <table:calculation-settings table:case-sensitive="true" table:null-year="1930" table:iteration="true" table:iteration-steps="25"/>
  <table:named-expressions><table:named-range table:name="Inputs" table:base-cell-address="$Visible.$A$1" table:cell-range-address="$Visible.$A$2:$A$3"/></table:named-expressions>
  <table:table table:name="Visible" table:print-ranges="$Visible.$A$1:$D$5">
   <table:table-column table:style-name="Wide" table:number-columns-repeated="2"/>
   <table:table-row table:style-name="Tall">
    <table:table-cell office:value-type="string" office:string-value="Hello"><text:p><text:a xlink:href="https://example.invalid/never-fetched">Hello link</text:a></text:p><office:annotation><dc:creator>Ada</dc:creator><dc:date>2026-08-07</dc:date><text:p>Review value</text:p></office:annotation></table:table-cell>
    <table:table-cell table:style-name="Percent" office:value-type="float" office:value="3" table:formula="of:=[.A2]+[.A3]"><text:p>3.00</text:p></table:table-cell>
    <table:table-cell table:number-columns-spanned="2" office:value-type="string" office:string-value="Merged"><text:p>Merged</text:p></table:table-cell>
    <table:covered-table-cell/>
   </table:table-row>
   <table:table-row><table:table-cell office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell></table:table-row>
   <table:table-row><table:table-cell office:value-type="float" office:value="2"><text:p>2</text:p></table:table-cell></table:table-row>
   <table:table-row table:number-rows-repeated="2"><table:table-cell table:number-columns-repeated="3" office:value-type="boolean" office:boolean-value="true"><text:p>TRUE</text:p></table:table-cell></table:table-row>
   {more}
   <table:shapes>
    <draw:frame draw:name="Logo" table:anchor-cell-address="$Visible.$D$5" table:end-cell-address="$Visible.$E$8"><draw:image xlink:href="Pictures/logo.png"/></draw:frame>
    <draw:frame draw:name="Revenue chart" table:anchor-cell-address="$Visible.$F$2"><draw:object xlink:href="./Object 1"/></draw:frame>
   </table:shapes>
   <calcext:conditional-formats calcext:target-range-address="$Visible.$B$1"><calcext:condition calcext:value="cell-content()&gt;0"/></calcext:conditional-formats>
  </table:table>
  <table:table table:name="Secret" table:style-name="HiddenTable" table:protected="true"><table:table-row><table:table-cell/></table:table-row></table:table>
 </office:spreadsheet></office:body>
</office:document-content>"##
    );
    let styles = br#"<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"><office:styles><style:style style:name="DefaultText" style:family="text"><style:text-properties style:font-name="Liberation Sans"/></style:style></office:styles></office:document-styles>"#;
    let encryption = if encrypted {
        r#"<manifest:encryption-data manifest:checksum="abc" manifest:checksum-type="SHA256"/>"#
    } else {
        ""
    };
    let manifest = format!(
        r#"<manifest:manifest manifest:version="1.3" xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="{media_type}"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml">{encryption}</manifest:file-entry><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="settings.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Pictures/logo.png" manifest:media-type="image/png"/><manifest:file-entry manifest:full-path="Object 1/" manifest:media-type="application/vnd.oasis.opendocument.chart"/><manifest:file-entry manifest:full-path="Object 1/content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#
    );
    let chart = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:chart><chart:chart><chart:title><text:p>Revenue</text:p></chart:title><chart:plot-area><chart:series chart:values-cell-range-address="Visible.B2:B3"/><table:table><table:table-row><table:table-cell office:value-type="float" office:value="1"/></table:table-row></table:table></chart:plot-area></chart:chart></office:chart></office:body></office:document-content>"#;
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    add(&mut zip, "mimetype", media_type.as_bytes(), true);
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
            content.as_bytes()
        },
        false,
    );
    add(&mut zip, "styles.xml", styles, false);
    add(&mut zip, "meta.xml", br#"<office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><office:meta><dc:title>Workbook metadata</dc:title><dc:creator>Grace</dc:creator></office:meta></office:document-meta>"#, false);
    add(&mut zip, "settings.xml", br#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings><config:config-item config:name="ActiveTable">Visible</config:config-item><config:config-item config:name="HorizontalSplitMode">2</config:config-item><config:config-item config:name="VerticalSplitMode">freeze</config:config-item><config:config-item config:name="HorizontalSplitPosition">1</config:config-item></office:settings></office:document-settings>"#, false);
    add(&mut zip, "Pictures/logo.png", b"PNG-INERT", false);
    add(&mut zip, "Object 1/content.xml", chart, false);
    if hostile {
        add(&mut zip, "../escape.bin", b"must-not-escape", false);
    }
    zip.finish().unwrap().into_inner()
}

fn dispatch(
    selector: &str,
    bytes: Vec<u8>,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("spreadsheet-odf-test").unwrap(),
        Input::bytes(bytes),
        SourceInfo::new(format!("fixture.{selector}")),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch(selector, request, None)
        .unwrap()
}

#[test]
fn ods_preserves_source_workbook_without_calculation() {
    let bytes = package(ODS, false, false, false, 0);
    let first = dispatch("ods", bytes.clone(), ResourceBudget::trusted_unbounded());
    let second = dispatch("ods", bytes.clone(), ResourceBudget::trusted_unbounded());
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:#?}",
        first.diagnostics
    );
    assert_eq!(first.payload, second.payload);
    let document: SpreadsheetOdfDocument =
        serde_json::from_value(first.payload.clone().unwrap()).unwrap();
    assert_eq!(document.package_kind, SpreadsheetOdfPackageKind::Workbook);
    assert_eq!(document.version.as_deref(), Some("1.3"));
    assert!(!document.calculation.formulas_calculated_by_grist);
    assert_eq!(document.active_table.as_deref(), Some("Visible"));
    assert!(document.panes[0].frozen);
    assert_eq!(document.named_ranges[0].name, "Inputs");
    assert_eq!(document.metadata[0].value, "Workbook metadata");
    assert!(!document.styles.is_empty());
    assert!(!document.parts.is_empty());
    let sheet = &document.sheets[0];
    assert_eq!(sheet.columns[0].repeated, 2);
    assert_eq!(sheet.columns[0].width.as_deref(), Some("4cm"));
    assert_eq!(sheet.rows[3].repeated, 2);
    assert_eq!(sheet.rows[3].cells[0].repeated, 3);
    assert_eq!(sheet.comments[0].text, "Review value");
    assert!(sheet.links[0].external);
    assert_eq!(sheet.merges[0].range, "C1:D1");
    assert_eq!(
        document.sheets[1].visibility,
        SpreadsheetOdfVisibility::Hidden
    );
    assert!(document.sheets[1].protected);
    let formula = &sheet.rows[0].cells[1];
    assert_eq!(formula.formula.as_ref().unwrap().source, "of:=[.A2]+[.A3]");
    assert!(!formula.formula.as_ref().unwrap().calculate);
    assert_eq!(formula.cached_value.as_ref().unwrap().stored_value, "3");
    assert_eq!(formula.displayed_value.as_deref(), Some("3.00"));
    assert!(formula.source_xml.contains("table:formula"));
    assert!(matches!(
        formula.locator.components()[0],
        LocationComponent::SheetRange { .. }
    ));
    assert!(
        sheet
            .objects
            .iter()
            .any(|item| item.kind == SpreadsheetOdfObjectKind::Image
                && item.name.as_deref() == Some("Logo"))
    );
    assert!(
        sheet
            .objects
            .iter()
            .any(|item| item.kind == SpreadsheetOdfObjectKind::Chart
                && item.title.as_deref() == Some("Revenue"))
    );
    assert!(
        sheet
            .raw_elements
            .iter()
            .any(|item| item.name.ends_with("conditional-formats")
                && item.raw_xml.contains("cell-content"))
    );

    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:ods"))
        .unwrap();
    graph.validate_contract().unwrap();
    for kind in [
        DocumentNodeKind::Sheet,
        DocumentNodeKind::Cell,
        DocumentNodeKind::Chart,
        DocumentNodeKind::Image,
        DocumentNodeKind::Comment,
        DocumentNodeKind::Raw,
    ] {
        assert!(
            graph.nodes.iter().any(|node| node.kind == kind),
            "missing {kind:?}"
        );
    }
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::FormulaDependsOn)
    );
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
            .any(|segment| segment.text.contains("Hello link") && !segment.locators.is_empty())
    );
    let schema = grist::schema::schema_json("spreadsheet-odf").unwrap();
    let report = grist::schema::validate_against_schema(
        "spreadsheet-odf",
        SchemaVersion::SPREADSHEET_ODF_V1,
        first.payload.as_ref().unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
    let detection = detect_with_registry(
        Path::new("extensionless"),
        &bytes,
        None,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Ods);
    assert_eq!(detection.candidates[0].identity.format, "ods");
}

#[test]
fn templates_large_malformed_encrypted_hostile_and_budget_cases_are_explicit() {
    let template_bytes = package(OTS, false, false, false, 0);
    let detection = detect_with_registry(
        Path::new("extensionless-template"),
        &template_bytes,
        None,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Ods);
    assert_eq!(detection.candidates[0].identity.format, "ots");
    let template = dispatch(
        "ots",
        template_bytes.clone(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(template.status, OperationStatus::Complete);
    let template_document: SpreadsheetOdfDocument =
        serde_json::from_value(template.payload.unwrap()).unwrap();
    assert_eq!(
        template_document.package_kind,
        SpreadsheetOdfPackageKind::Template
    );
    assert_eq!(
        dispatch("ods", template_bytes, ResourceBudget::trusted_unbounded()).status,
        OperationStatus::Failed
    );

    let encrypted = dispatch(
        "ods",
        package(ODS, true, false, false, 0),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(encrypted.status, OperationStatus::Encrypted);
    assert!(encrypted.payload.is_none());
    let malformed = dispatch(
        "ods",
        package(ODS, false, false, true, 0),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(malformed.payload.is_none());
    let hostile = dispatch(
        "ods",
        package(ODS, false, true, false, 0),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(hostile.status, OperationStatus::Partial);
    let hostile_document: SpreadsheetOdfDocument =
        serde_json::from_value(hostile.payload.unwrap()).unwrap();
    assert!(
        hostile_document
            .parts
            .iter()
            .any(|part| part.path == "../escape.bin"
                && part.rejection_code.as_deref() == Some("grist.security.archive.path_traversal"))
    );
    let large = dispatch(
        "ods",
        package(ODS, false, false, false, 1_000),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(large.status, OperationStatus::Complete);
    let large_document: SpreadsheetOdfDocument =
        serde_json::from_value(large.payload.unwrap()).unwrap();
    assert_eq!(large_document.sheets[0].rows.len(), 1_004);
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_cells = Some(3);
    let exhausted = dispatch("ods", package(ODS, false, false, false, 0), budget);
    assert_eq!(exhausted.status, OperationStatus::Failed);
    assert!(
        exhausted
            .diagnostics
            .iter()
            .any(|item| item.code.as_str() == "grist.budget.cells.exhausted")
    );
    assert_eq!(
        dispatch(
            "ods",
            b"PK\x03\x04truncated".to_vec(),
            ResourceBudget::trusted_unbounded()
        )
        .status,
        OperationStatus::Failed
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_parse_auto_transform_and_schema_use_the_registered_payload() {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("grist-ods-{nonce}"));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("fixture.ods");
    fs::write(&path, package(ODS, false, false, false, 0)).unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_grist"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let path_text = path.to_string_lossy();
    let parsed = run(&["parse", "ods", &path_text]);
    assert_eq!(parsed["kind"], "spreadsheet_odf");
    assert_eq!(
        parsed["payload"]["sheets"][0]["rows"][0]["cells"][1]["formula"]["source"],
        "of:=[.A2]+[.A3]"
    );
    let automatic = run(&["parse", "auto", &path_text]);
    assert_eq!(
        automatic["payload"]["schema_version"],
        SchemaVersion::SPREADSHEET_ODF_V1
    );
    let graph = run(&["transform", &path_text, "--to", "graph"]);
    assert!(
        graph["payload"]["graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["kind"] == "sheet")
    );
    let schema = run(&["schema", "emit", "spreadsheet-odf-envelope"]);
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&parsed)
    );
    fs::remove_dir_all(directory).unwrap();
}
