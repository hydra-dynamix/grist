#![cfg(all(
    feature = "spreadsheet-ooxml",
    feature = "document-graph",
    feature = "schemas"
))]

use grist::core::{
    BudgetSelection, Input, Limits, LocationComponent, OperationStatus, ParseRequest, ProviderSet,
    RequestId, ResourceBudget, SchemaVersion, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::builtin_parser_registry;
use grist::segment::{SegmentOptions, segment_document_graph};
use grist::spreadsheet_ooxml::{
    SpreadsheetOoxmlDocument, SpreadsheetPackageKind, SpreadsheetVisibility,
};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn dispatch(
    selector: &str,
    bytes: Vec<u8>,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("sheet-test").unwrap(),
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

fn add(writer: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str, bytes: &[u8]) {
    writer
        .start_file(
            name,
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
    writer.write_all(bytes).unwrap();
}

fn package(kind: SpreadsheetPackageKind, extra_rows: usize) -> Vec<u8> {
    let main_type = match kind {
        SpreadsheetPackageKind::Workbook => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
        }
        SpreadsheetPackageKind::MacroEnabledWorkbook => {
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml"
        }
    };
    let macro_override = (kind == SpreadsheetPackageKind::MacroEnabledWorkbook).then_some(
        r#"<Override PartName="/xl/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/>"#).unwrap_or("");
    let types = format!(
        r#"<Types xmlns="urn:types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/xl/workbook.xml" ContentType="{main_type}"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/><Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/><Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/><Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>{macro_override}</Types>"#
    );
    let root_rels = br#"<Relationships xmlns="urn:rels"><Relationship Id="rBook" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook = br#"<workbook xmlns="urn:s" xmlns:r="urn:r"><workbookPr date1904="1"/><sheets><sheet name="Visible" sheetId="1" r:id="rSheet1"/><sheet name="Secret" sheetId="2" state="veryHidden" r:id="rSheet2"/></sheets><definedNames><definedName name="InputRange">Visible!$A$2:$A$3</definedName></definedNames><calcPr calcId="99" calcMode="auto" fullCalcOnLoad="1"/></workbook>"#;
    let macro_rel = (kind == SpreadsheetPackageKind::MacroEnabledWorkbook).then_some(
        r#"<Relationship Id="rMacro" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/>"#).unwrap_or("");
    let workbook_rels = format!(
        r#"<Relationships xmlns="urn:rels"><Relationship Id="rSheet1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rSheet2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rStrings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>{macro_rel}</Relationships>"#
    );
    let more = (0..extra_rows)
        .map(|index| {
            format!(
                r#"<row r="{}"><c r="A{}"><v>{}</v></c></row>"#,
                index + 4,
                index + 4,
                index
            )
        })
        .collect::<String>();
    let sheet1 = format!(
        r#"<worksheet xmlns="urn:s" xmlns:r="urn:r"><dimension ref="A1:D{}"/><sheetViews><sheetView><pane state="frozen" ySplit="1" topLeftCell="A2"/><selection activeCell="B1" sqref="B1"/></sheetView></sheetViews><cols><col min="4" max="4" hidden="1" width="12"/></cols><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" s="1"><f>SUM(A2:A3)</f><v>3</v></c><c r="C1" t="inlineStr"><is><t>inline</t></is></c></row><row r="2"><c r="A2"><v>1</v></c></row><row r="3" hidden="1"><c r="A3"><v>2</v></c></row>{more}</sheetData><mergeCells><mergeCell ref="C1:D1"/></mergeCells><hyperlinks><hyperlink ref="A1" r:id="rLink" tooltip="docs"/></hyperlinks><tableParts><tablePart r:id="rTable"/></tableParts><drawing r:id="rDrawing"/></worksheet>"#,
        extra_rows + 3
    );
    let sheet_rels = br#"<Relationships xmlns="urn:rels"><Relationship Id="rLink" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/never-fetched" TargetMode="External"/><Relationship Id="rTable" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/table" Target="../tables/table1.xml"/><Relationship Id="rComments" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/><Relationship Id="rDrawing" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#;
    let styles = br#"<styleSheet xmlns="urn:s"><numFmts count="1"><numFmt numFmtId="164" formatCode="0.00%"/></numFmts><fonts count="1"><font><name val="Aptos"/><b/></font></fonts><fills count="1"><fill><patternFill patternType="solid"/></fill></fills><borders count="1"><border/></borders><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0"><alignment horizontal="right" wrapText="1"/><protection locked="1"/></xf></cellXfs></styleSheet>"#;
    let table = br#"<table xmlns="urn:s" id="1" name="Data" displayName="Data" ref="A1:B3"><tableColumns count="2"><tableColumn id="1" name="Label"/><tableColumn id="2" name="Value"><calculatedColumnFormula>A2*2</calculatedColumnFormula></tableColumn></tableColumns><tableStyleInfo name="TableStyleMedium2"/></table>"#;
    let comments = br#"<comments xmlns="urn:s"><authors><author>Ada</author></authors><commentList><comment ref="B1" authorId="0"><text><t>Stored cache</t></text></comment></commentList></comments>"#;
    let drawing = br#"<xdr:wsDr xmlns:xdr="urn:xdr" xmlns:a="urn:a" xmlns:r="urn:r" xmlns:c="urn:c"><xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:row>4</xdr:row></xdr:from><xdr:to><xdr:col>4</xdr:col><xdr:row>12</xdr:row></xdr:to><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="2" name="Logo"/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rImage"/></xdr:blipFill></xdr:pic><xdr:graphicFrame><a:graphic><a:graphicData><c:chart r:id="rChart"/></a:graphicData></a:graphic></xdr:graphicFrame></xdr:twoCellAnchor></xdr:wsDr>"#;
    let drawing_rels = br#"<Relationships xmlns="urn:rels"><Relationship Id="rImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/><Relationship Id="rChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#;
    let chart = br#"<c:chartSpace xmlns:c="urn:c"><c:chart><c:title><c:tx><c:rich><c:t>Revenue</c:t></c:rich></c:tx></c:title><c:ser><c:val><c:numRef><c:f>Visible!$A$2:$A$3</c:f><c:numCache><c:pt><c:v>1</c:v></c:pt><c:pt><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:chart></c:chartSpace>"#;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [("[Content_Types].xml", types.as_bytes()), ("_rels/.rels", root_rels),
        ("xl/workbook.xml", workbook), ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet1.as_bytes()), ("xl/worksheets/sheet2.xml", br#"<worksheet xmlns="urn:s"><sheetData/></worksheet>"#),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels), ("xl/styles.xml", styles),
        ("xl/sharedStrings.xml", br#"<sst xmlns="urn:s"><si><r><t>Hello </t></r><r><t>world</t></r></si></sst>"#),
        ("xl/tables/table1.xml", table), ("xl/comments1.xml", comments), ("xl/drawings/drawing1.xml", drawing),
        ("xl/drawings/_rels/drawing1.xml.rels", drawing_rels), ("xl/charts/chart1.xml", chart),
        ("xl/media/image1.png", b"PNG-INERT"), ("docProps/core.xml", br#"<cp:coreProperties xmlns:cp="urn:cp" xmlns:dc="urn:dc"><dc:title>Workbook title</dc:title></cp:coreProperties>"#)] {
        add(&mut writer, name, bytes);
    }
    if kind == SpreadsheetPackageKind::MacroEnabledWorkbook {
        add(&mut writer, "xl/vbaProject.bin", b"VBA-INERT");
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn workbook_constructs_formula_caches_and_exact_cells_survive() {
    let bytes = package(SpreadsheetPackageKind::Workbook, 0);
    let envelope = dispatch("xlsx", bytes.clone(), ResourceBudget::trusted_unbounded());
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:#?}",
        envelope.diagnostics
    );
    let document: SpreadsheetOoxmlDocument =
        serde_json::from_value(envelope.payload.clone().unwrap()).unwrap();
    assert_eq!(
        document.sheets[1].visibility,
        SpreadsheetVisibility::VeryHidden
    );
    let sheet = &document.sheets[0];
    assert_eq!(
        sheet.pane.as_ref().unwrap().state.as_deref(),
        Some("frozen")
    );
    assert_eq!(sheet.merges[0].range, "C1:D1");
    assert_eq!(sheet.tables[0].name.as_deref(), Some("Data"));
    assert_eq!(sheet.comments[0].text, "Stored cache");
    assert_eq!(sheet.hyperlinks[0].external, true);
    assert!(
        sheet
            .objects
            .iter()
            .any(|item| item.kind == grist::spreadsheet_ooxml::SpreadsheetObjectKind::Chart)
    );
    assert!(
        sheet
            .objects
            .iter()
            .any(|item| item.kind == grist::spreadsheet_ooxml::SpreadsheetObjectKind::Image)
    );
    let formula = &sheet.rows[0].cells[1];
    assert_eq!(formula.formula.as_ref().unwrap().source, "SUM(A2:A3)");
    assert!(!formula.formula.as_ref().unwrap().calculate);
    assert_eq!(formula.cached_value.as_ref().unwrap().stored_value, "3");
    assert!(matches!(
        formula.locator.components()[0],
        LocationComponent::SheetRange { .. }
    ));
    assert_eq!(
        sheet.rows[0].cells[0].displayed_value.as_deref(),
        Some("Hello world")
    );
    assert_eq!(
        document.styles.cell_formats[1]
            .number_format_code
            .as_deref(),
        Some("0.00%")
    );
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:sheet"))
        .unwrap();
    graph.validate_contract().unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Sheet)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Chart)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::FormulaDependsOn)
    );
    let identity = envelope.identity.as_ref().unwrap();
    let document_identity = grist::core::ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| segment.text.contains("Hello world"))
    );
    let schema = grist::schema::schema_json("spreadsheet-ooxml").unwrap();
    let report = grist::schema::validate_against_schema(
        "spreadsheet-ooxml",
        SchemaVersion::SPREADSHEET_OOXML_V1,
        &serde_json::to_value(document).unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
    let detection = detect_with_registry(
        Path::new("upload"),
        &bytes,
        None,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Xlsx);
}

#[test]
fn macros_encryption_malformed_large_and_budget_cases_are_explicit() {
    let bytes = package(SpreadsheetPackageKind::MacroEnabledWorkbook, 2_000);
    let envelope = dispatch("xlsm", bytes.clone(), ResourceBudget::trusted_unbounded());
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document: SpreadsheetOoxmlDocument =
        serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert_eq!(document.sheets[0].rows.len(), 2_003);
    assert_eq!(document.macro_projects.len(), 1);
    assert!(document.macro_projects[0].quarantined);
    assert!(!document.macro_projects[0].executable);
    let detection = detect_with_registry(
        Path::new("upload"),
        &bytes,
        None,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.content_kind, ContentKind::Xlsm);
    let mut encrypted = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    encrypted.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(
        dispatch("xlsx", encrypted, ResourceBudget::trusted_unbounded()).status,
        OperationStatus::Encrypted
    );
    assert_eq!(
        dispatch(
            "xlsx",
            b"PK\x03\x04truncated".to_vec(),
            ResourceBudget::trusted_unbounded()
        )
        .status,
        OperationStatus::Failed
    );
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_cells = Some(2);
    let exhausted = dispatch("xlsx", package(SpreadsheetPackageKind::Workbook, 0), budget);
    assert_eq!(exhausted.status, OperationStatus::Failed);
    assert!(
        exhausted
            .diagnostics
            .iter()
            .any(|item| item.code.as_str() == "grist.budget.cells.exhausted")
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_parse_auto_and_transform_use_the_same_workbook_payload() {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut directory = std::env::temp_dir();
    directory.push(format!(
        "grist-xlsx-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("fixture.xlsx");
    fs::write(&path, package(SpreadsheetPackageKind::Workbook, 0)).unwrap();
    let binary = env!("CARGO_BIN_EXE_grist");
    let run = |args: &[&str]| {
        let output = Command::new(binary).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let path_text = path.to_string_lossy();
    let parsed = run(&["parse", "xlsx", &path_text]);
    assert_eq!(parsed["kind"], "spreadsheet_ooxml");
    assert_eq!(
        parsed["payload"]["sheets"][0]["rows"][0]["cells"][1]["formula"]["source"],
        "SUM(A2:A3)"
    );
    let automatic = run(&["parse", "auto", &path_text]);
    assert_eq!(
        automatic["payload"]["schema_version"],
        SchemaVersion::SPREADSHEET_OOXML_V1
    );
    let graph = run(&["transform", &path_text, "--to", "graph"]);
    assert!(
        graph["payload"]["graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["kind"] == "sheet")
    );
    let schema = run(&["schema", "emit", "spreadsheet-ooxml-envelope"]);
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&parsed)
    );
    fs::remove_dir_all(directory).unwrap();
}
