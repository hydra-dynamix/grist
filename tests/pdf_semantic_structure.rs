#![cfg(feature = "pdf")]

use grist::core::{ContentIdentity, OperationStatus, SourceInfo, canonical_json_bytes};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::pdf::{
    PdfGraphicObjectKind, PdfOptions, PdfRepeatedRegionKind, PdfSemanticBlockKind,
    PdfStructureTreeStatus, parse_pdf_bytes,
};
use grist::segment::{SegmentOptions, segment_document_graph};

fn stream(bytes: &[u8], extra: &str) -> Vec<u8> {
    let mut value = format!("<< /Length {} {extra} >>\nstream\n", bytes.len()).into_bytes();
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

fn font() -> Vec<u8> {
    let widths = (32..=122).map(|_| "600").collect::<Vec<_>>().join(" ");
    format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 122 /Widths [{widths}] /FontDescriptor << /Flags 32 /FontWeight 400 /ItalicAngle 0 >> >>").into_bytes()
}

fn page_content(page: u8, first: bool) -> Vec<u8> {
    let mut value = format!(
        "BT /F1 10 Tf 50 760 Td (Quarterly Report) Tj ET\nBT /F1 10 Tf 300 30 Td ({page}) Tj ET\n"
    )
    .into_bytes();
    if first {
        value.extend_from_slice(b"BT /F1 20 Tf 50 700 Td (Results) Tj ET\nBT /F1 10 Tf 50 650 Td (- Item one) Tj ET\nBT /F1 10 Tf 50 610 Td (- Item two) Tj ET\nBT /F1 10 Tf 50 470 Td (Table 1 Results) Tj ET\nBT /F1 10 Tf 50 430 Td (Header) Tj ET\nBT /F1 10 Tf 50 410 Td (Alpha) Tj 150 0 Td (10) Tj ET\n50 400 220 50 re S 160 400 m 160 425 l S 50 425 m 270 425 l S\nq 120 0 0 80 50 220 cm /Im1 Do Q\nBT /F1 10 Tf 50 200 Td (Figure 1 Chart) Tj ET\n");
    }
    value
}

fn fixture() -> Vec<u8> {
    pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 99 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 7 0 R >> /XObject << /Im1 8 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>".to_vec(),
        stream(&page_content(1, true), ""), stream(&page_content(2, false), ""), font(),
        stream(&[0, 0, 0], "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8"),
    ])
}

#[test]
fn infers_semantics_tables_figures_repetition_and_projects_segments() {
    let bytes = fixture();
    let source = SourceInfo::new("semantic.pdf");
    let envelope = parse_pdf_bytes(&bytes, source.clone(), &PdfOptions::default());
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|value| value.code.as_str() == "pdf.structure_tree.malformed")
    );
    let document = envelope.payload.unwrap();
    assert_eq!(
        document.semantic_structure.structure_tree.status,
        PdfStructureTreeStatus::Malformed
    );
    assert_eq!(document.semantic_structure.pages.len(), 2);
    assert!(
        document
            .semantic_structure
            .repeated_regions
            .iter()
            .any(|value| value.kind == PdfRepeatedRegionKind::Header),
        "{:#?}",
        document.semantic_structure
    );
    assert!(
        document
            .semantic_structure
            .repeated_regions
            .iter()
            .any(|value| value.kind == PdfRepeatedRegionKind::PageNumber)
    );
    let page = &document.semantic_structure.pages[0];
    assert!(
        page.blocks
            .iter()
            .any(|value| value.kind == PdfSemanticBlockKind::Heading && value.confidence > 0.0)
    );
    assert_eq!(page.lists[0].item_block_indices.len(), 2);
    assert_eq!(page.tables[0].rows.len(), 2);
    assert!(page.tables[0].rows[0].cells[0].header_candidate);
    assert_eq!(
        page.tables[0].rows[0].cells[0].column_span, 2,
        "{:#?}",
        page.tables[0]
    );
    let image = page
        .graphics
        .iter()
        .find(|value| value.kind == PdfGraphicObjectKind::RasterImage)
        .unwrap();
    assert_eq!((image.pixel_width, image.pixel_height), (Some(1), Some(1)));
    assert!(
        page.figures
            .iter()
            .any(|value| value.caption_block_index.is_some())
    );
    assert!(
        page.reading_order
            .iter()
            .all(|value| value.confidence > 0.0)
    );

    let graph = document
        .to_document_graph(DocumentGraphContext::new("semantic").with_source(source))
        .unwrap();
    graph.validate_contract().unwrap();
    for kind in [
        DocumentNodeKind::Heading,
        DocumentNodeKind::List,
        DocumentNodeKind::ListItem,
        DocumentNodeKind::Table,
        DocumentNodeKind::TableRow,
        DocumentNodeKind::TableCell,
        DocumentNodeKind::Figure,
        DocumentNodeKind::Image,
        DocumentNodeKind::Caption,
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
            .any(|edge| edge.relation == DocumentRelation::CaptionFor)
    );
    let source_identity = ContentIdentity::for_raw_bytes(&bytes);
    let document_identity = ContentIdentity::for_raw_bytes(&canonical_json_bytes(&graph).unwrap());
    let segments = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| segment
                .node_references
                .iter()
                .any(|reference| graph
                    .nodes
                    .iter()
                    .any(|node| node.id == reference.node_id
                        && node.kind == DocumentNodeKind::TableCell)))
    );
}
