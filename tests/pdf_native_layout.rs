#![cfg(feature = "pdf")]

use grist::core::{LocationComponent, LocatorPrecision, OperationStatus, SourceInfo};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::pdf::{
    PdfNativeTextStatus, PdfOptions, PdfReadingEvidenceKind, PdfUnicodeMapStatus,
    PdfWritingDirection, parse_pdf_bytes,
};

fn source() -> SourceInfo {
    SourceInfo::new("native-layout.pdf").with_declared_mime_type("application/pdf")
}

fn stream(bytes: &[u8]) -> Vec<u8> {
    let mut value = format!("<< /Length {} >>\nstream\n", bytes.len()).into_bytes();
    value.extend_from_slice(bytes);
    value.extend_from_slice(b"\nendstream");
    value
}

fn pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    bytes.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.into_iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn simple_font() -> Vec<u8> {
    let widths = (32..=122).map(|_| "600").collect::<Vec<_>>().join(" ");
    format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 122 /Widths [{widths}] /FontDescriptor << /Flags 32 /FontWeight 400 /ItalicAngle 0 >> >>").into_bytes()
}

fn born_digital_hybrid_fixture() -> Vec<u8> {
    let content = b"q BT /F1 12 Tf 1 0 0 1 50 700 Tm (Left top) Tj 1 0 0 1 50 680 Tm (Left lower) Tj 1 0 0 1 300 700 Tm (Right top) Tj 1 0 0 1 300 680 Tm (Right lower) Tj ET BI /W 1 /H 1 /CS /RGB /BPC 8 ID abc EI Q";
    pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Rotate 90 /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(content),
        simple_font(),
    ])
}

#[test]
fn extracts_precise_native_geometry_columns_blocks_and_source_maps() {
    let envelope = parse_pdf_bytes(
        &born_digital_hybrid_fixture(),
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
    assert_eq!(document.native_layout.fonts.len(), 1);
    assert_eq!(
        document.native_layout.fonts[0].to_unicode,
        PdfUnicodeMapStatus::StandardEncoding
    );
    let layout = &document.native_layout.pages[0];
    assert_eq!(layout.status, PdfNativeTextStatus::Extracted);
    assert_eq!(layout.rotation_degrees, 90);
    assert_eq!(layout.content_objects[0].object_number, 4);
    assert_eq!(layout.blocks.len(), 2);
    assert_eq!(layout.blocks[0].text, "Left top\nLeft lower");
    assert_eq!(layout.blocks[1].text, "Right top\nRight lower");
    assert_eq!(layout.reading_order.block_order, [1, 2]);
    assert!(
        layout
            .reading_order
            .evidence
            .iter()
            .any(|evidence| evidence.kind == PdfReadingEvidenceKind::ColumnSeparation)
    );
    assert!(
        layout
            .glyphs
            .iter()
            .all(|glyph| glyph.bbox.width > 0.0 && glyph.bbox.height > 0.0)
    );
    assert!(
        layout
            .glyphs
            .iter()
            .all(|glyph| matches!(glyph.locator.precision(), LocatorPrecision::Exact { .. }))
    );
    for token in &layout.tokens {
        assert!(token.locator.validate().is_ok());
        let LocationComponent::PdfRegion {
            page,
            bbox,
            rotation_degrees,
            tokens,
        } = token.locator.innermost()
        else {
            panic!("token locator is not a PDF region");
        };
        assert_eq!(page.value, 1);
        assert_eq!(*rotation_degrees, Some(90));
        assert_eq!(bbox.unwrap(), token.bbox);
        assert!(tokens.is_some());
    }

    let graph = document
        .to_document_graph(DocumentGraphContext::new("native-layout").with_source(source()))
        .unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::Paragraph)
            .count(),
        2
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::TextRun
                && node.text.as_deref() == Some("Left top"))
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Precedes)
    );
    graph.validate_contract().unwrap();
}

fn directional_fixture() -> Vec<u8> {
    let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange 2 beginbfchar <0001> <05D0> <0002> <05D1> endbfchar endcmap end end";
    let mut cmap_stream = format!("<< /Length {} >>\nstream\n", cmap.len()).into_bytes();
    cmap_stream.extend_from_slice(cmap);
    cmap_stream.extend_from_slice(b"\nendstream");
    pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 300] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(b"BT /F0 14 Tf 1 0 0 1 40 200 Tm <00010002> Tj ET"),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /SyntheticHebrew /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SyntheticHebrew /DW 1000 /W [1 [600 600]] >>".to_vec(),
        cmap_stream,
    ])
}

#[test]
fn decodes_to_unicode_and_retains_directional_evidence() {
    let envelope = parse_pdf_bytes(&directional_fixture(), source(), &PdfOptions::default());
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload.unwrap();
    assert_eq!(
        document.native_layout.fonts[0].to_unicode,
        PdfUnicodeMapStatus::Present
    );
    let layout = &document.native_layout.pages[0];
    assert_eq!(layout.glyphs.len(), 2);
    assert!(
        layout
            .glyphs
            .iter()
            .all(|glyph| glyph.direction == PdfWritingDirection::RightToLeft)
    );
    assert_eq!(layout.tokens[0].direction, PdfWritingDirection::RightToLeft);
    assert!(
        layout
            .reading_order
            .evidence
            .iter()
            .any(|evidence| evidence.kind == PdfReadingEvidenceKind::UnicodeDirectionality)
    );
}

#[test]
fn diagnoses_missing_fonts_maps_invalid_content_and_native_budgets() {
    let missing_font = pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << >> >> /Contents 4 0 R >>".to_vec(),
        stream(b"BT /Missing 12 Tf 1 0 0 1 10 100 Tm (fallback) Tj ET"),
    ]);
    let envelope = parse_pdf_bytes(&missing_font, source(), &PdfOptions::default());
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "pdf.font.missing")
    );
    assert!(matches!(
        envelope.payload.as_ref().unwrap().native_layout.pages[0].glyphs[0]
            .locator
            .precision(),
        LocatorPrecision::Approximate { .. }
    ));

    let missing_map = pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(b"BT /F0 12 Tf 1 0 0 1 10 100 Tm <0001> Tj ET"),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /NoMap /Encoding /Identity-H /DescendantFonts [6 0 R] >>".to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /NoMap /DW 1000 >>".to_vec(),
    ]);
    let envelope = parse_pdf_bytes(&missing_map, source(), &PdfOptions::default());
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "pdf.font.to_unicode_missing")
    );
    assert_eq!(
        envelope.payload.as_ref().unwrap().native_layout.pages[0].glyphs[0].text,
        "\u{fffd}"
    );

    let invalid_content = pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        b"<< /NotAStream true >>".to_vec(),
    ]);
    let envelope = parse_pdf_bytes(&invalid_content, source(), &PdfOptions::default());
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "pdf.content.invalid_object")
    );

    let mut options = PdfOptions::default();
    options.max_native_glyphs = 3;
    let limited = parse_pdf_bytes(&born_digital_hybrid_fixture(), source(), &options);
    assert_eq!(limited.status, OperationStatus::Partial);
    assert_eq!(
        limited.payload.as_ref().unwrap().native_layout.pages[0].status,
        PdfNativeTextStatus::BudgetExceeded
    );
    assert_eq!(
        limited.payload.as_ref().unwrap().native_layout.pages[0]
            .glyphs
            .len(),
        3
    );
}
