#![cfg(feature = "pdf")]

#[cfg(feature = "schemas")]
use grist::core::ParseRequest;
use grist::core::{ArtifactKind, OperationStatus, SchemaVersion, SecretString, SourceInfo};
#[cfg(feature = "schemas")]
use grist::core::{BudgetSelection, Input, ProviderSet, RequestId, ResourceBudget};
#[cfg(feature = "schemas")]
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::pdf::{PdfObjectLocation, PdfOptions, PdfPageLabelStyle, parse_pdf_bytes};
#[cfg(feature = "schemas")]
use grist::registry::{ParserSelection, builtin_parser_registry};
// Fixtures are assembled as exact byte vectors to preserve binary stream data.

fn source() -> SourceInfo {
    SourceInfo::new("fixture.pdf").with_declared_mime_type("application/pdf")
}

fn fixture(
    active: bool,
    unsupported_filter: bool,
    encrypted: bool,
    damaged_xref: bool,
    eof: bool,
) -> Vec<u8> {
    let xmp = b"<?xpacket?><x:xmpmeta xmlns:x='adobe:ns:meta/'/>";
    let mut objects = vec![
        format!(
            "<< /Type /Catalog /Pages 2 0 R /PageLabels 5 0 R /Metadata 7 0 R{} >>",
            if active { " /OpenAction 8 0 R" } else { "" }
        )
        .into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /CropBox [0 0 300 400] /Rotate 90 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /UserUnit 2 >>".to_vec(),
        b"<< /Nums [0 << /S /r /P (A-) /St 2 >> 1 << /S /D >>] >>".to_vec(),
        b"<< /Title (Core fixture) /Author <FEFF00470072006900730074> >>".to_vec(),
    ];
    let mut metadata = format!(
        "<< /Type /Metadata /Subtype /XML /Length {}{} >>\nstream\n",
        xmp.len(),
        if unsupported_filter {
            " /Filter /LZWDecode"
        } else {
            ""
        }
    )
    .into_bytes();
    metadata.extend_from_slice(xmp);
    metadata.extend_from_slice(b"\nendstream");
    objects.push(metadata);
    if active {
        objects.push(b"<< /S /JavaScript /JS (this text is inert) >>".to_vec());
    }
    if encrypted {
        objects.push(b"<< /Filter /Standard /V 2 /R 3 /Length 128 /P -4 >>".to_vec());
    }

    let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        write_bytes(&mut pdf, format!("{} 0 obj\n", index + 1).as_bytes());
        write_bytes(&mut pdf, object);
        write_bytes(&mut pdf, b"\nendobj\n");
    }
    let xref_offset = pdf.len();
    write_bytes(
        &mut pdf,
        format!("xref\n0 {}\n", objects.len() + 1).as_bytes(),
    );
    write_bytes(&mut pdf, b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        let recorded = if damaged_xref { offset + 3 } else { *offset };
        write_bytes(&mut pdf, format!("{recorded:010} 00000 n \n").as_bytes());
    }
    let info = 6;
    let encrypt_entry = if encrypted {
        format!(" /Encrypt {} 0 R", objects.len())
    } else {
        String::new()
    };
    write_bytes(&mut pdf, format!("trailer\n<< /Size {} /Root 1 0 R /Info {info} 0 R{encrypt_entry} /ID [<0011> <0011>] >>\nstartxref\n{}\n", objects.len() + 1, if damaged_xref { 0 } else { xref_offset }).as_bytes());
    if eof {
        write_bytes(&mut pdf, b"%%EOF\n");
    }
    pdf
}

fn write_bytes(target: &mut Vec<u8>, bytes: &[u8]) {
    target.extend_from_slice(bytes);
}

#[test]
fn parses_catalog_xref_pages_labels_dimensions_rotation_metadata_and_locators() {
    let envelope = parse_pdf_bytes(
        &fixture(false, false, false, false, true),
        source(),
        &PdfOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    assert_eq!(envelope.kind, ArtifactKind::Pdf);
    assert_eq!(envelope.payload_schema_version.0, SchemaVersion::PDF_V1);
    let document = envelope.payload.unwrap();
    assert_eq!(document.header.version, "1.7");
    assert!(document.header.binary_marker);
    assert_eq!(document.catalog.object.object_number, 1);
    assert_eq!(document.trailer.info.unwrap().object_number, 6);
    assert_eq!(document.pages.len(), 2);
    assert_eq!(document.pages[0].rotation_degrees, 90);
    assert_eq!(
        (
            document.pages[0].width_points,
            document.pages[0].height_points
        ),
        (400.0, 300.0)
    );
    assert_eq!(
        (
            document.pages[1].width_points,
            document.pages[1].height_points
        ),
        (1224.0, 1584.0)
    );
    assert_eq!(document.pages[0].label.as_deref(), Some("A-ii"));
    assert_eq!(document.pages[1].label.as_deref(), Some("1"));
    assert_eq!(
        document.page_labels[0].style,
        Some(PdfPageLabelStyle::LowerRoman)
    );
    assert_eq!(
        document
            .metadata
            .fields
            .iter()
            .find(|field| field.name == "Title")
            .unwrap()
            .value
            .clone(),
        grist::pdf::PdfValue::String(grist::pdf::PdfString {
            raw_hex: "436F72652066697874757265".into(),
            text: "Core fixture".into(),
            encoding: grist::pdf::PdfStringEncoding::PdfDoc
        })
    );
    assert!(
        document
            .metadata
            .xmp
            .as_deref()
            .unwrap()
            .contains("x:xmpmeta")
    );
    assert!(
        document
            .pages
            .iter()
            .all(|page| page.locator.validate().is_ok())
    );
    assert!(
        document
            .objects
            .iter()
            .all(|object| !object.locator.stable_id.is_empty())
    );
    assert!(
        document
            .xref
            .entries
            .iter()
            .filter(|entry| entry.byte_offset.is_some())
            .all(|entry| entry.valid)
    );
}

#[test]
fn damaged_xref_and_missing_eof_are_partial_with_explicit_repairs() {
    let envelope = parse_pdf_bytes(
        &fixture(false, false, false, true, false),
        source(),
        &PdfOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let document = envelope.payload.unwrap();
    assert!(document.xref.repaired);
    assert!(
        document
            .repairs
            .iter()
            .any(|repair| repair.code == "pdf.xref.invalid_start"
                || repair.code == "pdf.xref.reconstructed")
    );
    assert!(
        document
            .repairs
            .iter()
            .any(|repair| repair.code == "pdf.eof.missing")
    );
    assert!(envelope.provenance.iter().any(|step| {
        step.warnings
            .iter()
            .any(|warning| warning == "pdf.repaired")
    }));
}

#[test]
fn encryption_is_terminal_and_password_never_serializes_or_debugs() {
    let bytes = fixture(false, false, true, false, true);
    let options = PdfOptions::default().with_password(SecretString::new("top-secret"));
    let serialized = serde_json::to_string(&options).unwrap();
    let debug = format!("{options:?}");
    assert!(!serialized.contains("top-secret"));
    assert!(!debug.contains("top-secret"));
    assert!(debug.contains("<redacted>"));
    let envelope = parse_pdf_bytes(&bytes, source(), &options);
    assert_eq!(envelope.status, OperationStatus::Encrypted);
    assert!(envelope.payload.is_none());
    let wire = serde_json::to_string(&envelope).unwrap();
    assert!(!wire.contains("top-secret"));
    assert_eq!(envelope.diagnostics[0].code.as_str(), "pdf.encrypted");
}

#[test]
fn unsupported_filters_active_content_and_budgets_are_never_silent() {
    let unsupported = parse_pdf_bytes(
        &fixture(false, true, false, false, true),
        source(),
        &PdfOptions::default(),
    );
    assert_eq!(unsupported.status, OperationStatus::Partial);
    assert!(
        unsupported
            .payload
            .as_ref()
            .unwrap()
            .filters
            .iter()
            .any(|usage| !usage.supported)
    );
    assert!(
        unsupported
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "grist.content.unsupported")
    );

    let hostile = parse_pdf_bytes(
        &fixture(true, false, false, false, true),
        source(),
        &PdfOptions::default(),
    );
    assert_eq!(hostile.status, OperationStatus::Partial);
    let active = &hostile.payload.as_ref().unwrap().active_content;
    assert!(active.iter().any(|item| item.key == "OpenAction"));
    assert!(
        active
            .iter()
            .any(|item| item.action_type.as_deref() == Some("JavaScript"))
    );

    let mut limited_options = PdfOptions::default();
    limited_options.max_pages = 1;
    let limited = parse_pdf_bytes(
        &fixture(false, false, false, false, true),
        source(),
        &limited_options,
    );
    assert_eq!(limited.status, OperationStatus::Partial);
    assert_eq!(limited.payload.as_ref().unwrap().pages.len(), 1);
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "grist.budget.exhausted")
    );
}

#[test]
fn malformed_inputs_fail_without_panicking_and_output_is_deterministic() {
    let malformed = parse_pdf_bytes(b"not a pdf", source(), &PdfOptions::default());
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(malformed.payload.is_none());

    let bytes = fixture(false, false, false, false, true);
    let first = parse_pdf_bytes(&bytes, source(), &PdfOptions::default());
    let second = parse_pdf_bytes(&bytes, source(), &PdfOptions::default());
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
}

#[cfg(feature = "schemas")]
#[test]
fn registry_graph_and_schema_surfaces_are_live() {
    let registry = builtin_parser_registry().unwrap();
    let descriptor = match registry.select_format("pdf") {
        ParserSelection::Available(descriptor) => descriptor,
        ParserSelection::Unsupported { .. } => panic!("PDF parser unavailable"),
    };
    assert_eq!(descriptor.payload_schema.version, SchemaVersion::PDF_V1);
    let request = ParseRequest::new(
        RequestId::new("pdf-test").unwrap(),
        Input::bytes(fixture(false, false, false, false, true)),
        source(),
        BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
        ProviderSet::none(),
    );
    let envelope = registry.dispatch("pdf", request, None).unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document: grist::pdf::PdfDocument =
        serde_json::from_value(envelope.payload.unwrap()).unwrap();
    let graph = document
        .to_document_graph(DocumentGraphContext::new("pdf-fixture").with_source(source()))
        .unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::Page)
            .count(),
        2
    );
    graph.validate_contract().unwrap();

    let schema = grist::schema::schema_json("pdf").unwrap();
    let payload = serde_json::to_value(document).unwrap();
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&payload)
    );
    assert!(grist::schema::schema_json("pdf-envelope").is_some());
    assert!(grist::schema::schema_json("pdf-options").is_some());
}

#[test]
fn direct_and_compressed_locator_shapes_are_stable() {
    let envelope = parse_pdf_bytes(
        &fixture(false, false, false, false, true),
        source(),
        &PdfOptions::default(),
    );
    let document = envelope.payload.unwrap();
    assert!(
        document
            .objects
            .iter()
            .all(|object| matches!(object.locator.location, PdfObjectLocation::Direct { .. }))
    );
    let ids = document
        .objects
        .iter()
        .map(|object| object.locator.stable_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), document.objects.len());
}

fn xref_and_object_stream_fixture() -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = [0usize; 7];
    let regular = [
        (1usize, b"<< /Type /Catalog /Pages 2 0 R >>".as_slice()),
        (
            2,
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".as_slice(),
        ),
        (3, b"<< /Type /Page /Parent 2 0 R >>".as_slice()),
    ];
    for (number, value) in regular {
        offsets[number] = pdf.len();
        write_bytes(&mut pdf, format!("{number} 0 obj\n").as_bytes());
        write_bytes(&mut pdf, value);
        write_bytes(&mut pdf, b"\nendobj\n");
    }
    let compressed = b"6 0 << /Title (Compressed metadata) >>";
    offsets[5] = pdf.len();
    write_bytes(
        &mut pdf,
        format!(
            "5 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n",
            compressed.len()
        )
        .as_bytes(),
    );
    write_bytes(&mut pdf, compressed);
    write_bytes(&mut pdf, b"\nendstream\nendobj\n");

    offsets[4] = pdf.len();
    let mut rows = Vec::new();
    xref_row(&mut rows, 0, 0, 65_535);
    xref_row(&mut rows, 1, offsets[1] as u32, 0);
    xref_row(&mut rows, 1, offsets[2] as u32, 0);
    xref_row(&mut rows, 1, offsets[3] as u32, 0);
    xref_row(&mut rows, 1, offsets[4] as u32, 0);
    xref_row(&mut rows, 1, offsets[5] as u32, 0);
    xref_row(&mut rows, 2, 5, 0);
    write_bytes(
        &mut pdf,
        format!("4 0 obj\n<< /Type /XRef /Size 7 /Root 1 0 R /Info 6 0 R /W [1 4 2] /Length {} >>\nstream\n", rows.len()).as_bytes(),
    );
    write_bytes(&mut pdf, &rows);
    write_bytes(&mut pdf, b"\nendstream\nendobj\n");
    write_bytes(
        &mut pdf,
        format!("startxref\n{}\n%%EOF\n", offsets[4]).as_bytes(),
    );
    pdf
}

fn xref_row(rows: &mut Vec<u8>, kind: u8, field1: u32, field2: u16) {
    rows.push(kind);
    rows.extend_from_slice(&field1.to_be_bytes());
    rows.extend_from_slice(&field2.to_be_bytes());
}

#[test]
fn xref_streams_and_compressed_objects_are_resolved_with_stable_locators() {
    let envelope = parse_pdf_bytes(
        &xref_and_object_stream_fixture(),
        source(),
        &PdfOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload.unwrap();
    assert!(
        document
            .xref
            .sections
            .iter()
            .any(|section| section.kind == grist::pdf::PdfXrefKind::Stream)
    );
    let compressed = document
        .objects
        .iter()
        .find(|object| object.object.object_number == 6)
        .unwrap();
    assert!(
        matches!(compressed.locator.location, PdfObjectLocation::ObjectStream { container, .. } if container.object_number == 5)
    );
    assert!(
        document
            .metadata
            .fields
            .iter()
            .any(|field| field.name == "Title")
    );
    assert!(
        document
            .xref
            .entries
            .iter()
            .any(|entry| entry.object_number == 6
                && entry.kind == grist::pdf::PdfXrefEntryKind::Compressed)
    );
}
