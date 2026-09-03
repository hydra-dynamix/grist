#![cfg(feature = "pdf")]

use grist::container::ArtifactExtractionStatus;
use grist::core::{OperationStatus, SourceInfo};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::pdf::{
    PdfActiveContentDisposition, PdfEmbeddedChildStatus, PdfInteractiveRelation, PdfOptions,
    parse_pdf_bytes,
};

fn source() -> SourceInfo {
    SourceInfo::new("interactive-embedded.pdf").with_declared_mime_type("application/pdf")
}

fn stream(dictionary: &str, bytes: &[u8]) -> Vec<u8> {
    let mut value = format!("<< {dictionary} /Length {} >>\nstream\n", bytes.len()).into_bytes();
    value.extend_from_slice(bytes);
    value.extend_from_slice(b"\nendstream");
    value
}

fn pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n%1234\n".to_vec();
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

fn nested_pdf() -> Vec<u8> {
    pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R /Names 4 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_vec(),
        b"<< /EmbeddedFiles 5 0 R >>".to_vec(),
        b"<< /Names [(inner.txt) 6 0 R] >>".to_vec(),
        b"<< /Type /Filespec /F (inner.txt) /EF << /F 7 0 R >> >>".to_vec(),
        stream("/Type /EmbeddedFile /Subtype /text#2Fplain", b"inner"),
    ])
}

fn fixture() -> Vec<u8> {
    let nested = nested_pdf();
    let encrypted_child = [b"%PDF-1.7 ".as_slice(), b"/En", b"crypt"].concat();
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R /Names 7 0 R /AcroForm 10 0 R /OCProperties 14 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [8 0 R 9 0 R 11 0 R 17 0 R] /Contents 4 0 R >>".to_vec(),
        stream("", b""),
        b"<< /Type /Outlines /First 6 0 R /Last 6 0 R >>".to_vec(),
        b"<< /Title (Bookmark) /Parent 5 0 R /Dest (target) /A << /S /Named /N /NextPage >> >>".to_vec(),
        b"<< /Dests 15 0 R /EmbeddedFiles 16 0 R >>".to_vec(),
        b"<< /Type /Annot /Subtype /Link /Rect [10 20 110 40] /Dest (target) /A << /S /Named /N /FirstPage >> >>".to_vec(),
        b"<< /Type /Annot /Subtype /Text /Rect [20 50 40 70] /Contents (First comment) /T (Alice) /Subj (Review) >>".to_vec(),
        b"<< /Fields [11 0 R 12 0 R] /SigFlags 3 /NeedAppearances false >>".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /Rect [120 20 220 40] /P 3 0 R /FT /Tx /T (name) /V (Ada) >>".to_vec(),
        b"<< /FT /Sig /T (approval) /V 13 0 R >>".to_vec(),
        b"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /Name (Signer) /Reason (Approved) /Location (Vancouver) /M (D:20260807120000Z) /ByteRange [0 10 20 30] /Contents <01020304> >>".to_vec(),
        b"<< /OCGs [18 0 R] /D << /ON [18 0 R] >> >>".to_vec(),
        b"<< /Names [(target) [3 0 R /XYZ 0 700 null]] >>".to_vec(),
        b"<< /Names [(note.txt) 19 0 R (nested.pdf) 21 0 R (secret.pdf) 23 0 R] >>".to_vec(),
        b"<< /Type /Annot /Subtype /Text /Rect [20 80 40 100] /Contents (Reply) /T (Bob) /IRT 9 0 R >>".to_vec(),
        b"<< /Type /OCG /Name (Review layer) /Intent [/View /Design] /Usage << /View << /ViewState /ON >> >> >>".to_vec(),
        b"<< /Type /Filespec /F (note.txt) /Desc (plain note) /AFRelationship /Data /EF << /F 20 0 R >> >>".to_vec(),
        stream("/Type /EmbeddedFile /Subtype /text#2Fplain", b"hello"),
        b"<< /Type /Filespec /F (nested.pdf) /AFRelationship /Supplement /EF << /F 22 0 R >> >>".to_vec(),
        stream("/Type /EmbeddedFile /Subtype /application#2Fpdf", &nested),
        b"<< /Type /Filespec /F (secret.pdf) /EF << /F 24 0 R >> >>".to_vec(),
        stream(
            "/Type /EmbeddedFile /Subtype /application#2Fpdf",
            &encrypted_child,
        ),
    ];
    pdf(objects)
}

#[test]
fn preserves_interactive_structure_actions_and_nested_artifact_identities() {
    let envelope = parse_pdf_bytes(&fixture(), source(), &PdfOptions::default());
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "pdf.active_content.inert")
    );
    let document = envelope.payload.unwrap();
    let interactive = &document.interactive;
    assert_eq!(interactive.destinations.len(), 1);
    assert_eq!(interactive.destinations[0].destination.page_index, Some(1));
    assert_eq!(interactive.outlines.len(), 1);
    assert_eq!(
        interactive.outlines[0].action.as_ref().unwrap().disposition,
        PdfActiveContentDisposition::InventoriedNotExecuted
    );
    assert_eq!(interactive.links.len(), 1);
    assert_eq!(interactive.annotations.len(), 4);
    assert_eq!(interactive.comments.len(), 2);
    assert!(
        interactive
            .relationships
            .iter()
            .any(|relationship| { relationship.relation == PdfInteractiveRelation::ReplyTo })
    );
    assert!(
        interactive
            .relationships
            .iter()
            .any(|relationship| { relationship.relation == PdfInteractiveRelation::ResolvesTo })
    );
    assert!(
        interactive
            .relationships
            .iter()
            .any(|relationship| { relationship.relation == PdfInteractiveRelation::FieldWidget })
    );
    let form = interactive.form.as_ref().unwrap();
    assert_eq!(form.fields.len(), 2);
    assert_eq!(
        interactive.signatures[0].signer_name.as_deref(),
        Some("Signer")
    );
    assert_eq!(interactive.signatures[0].byte_range, [0, 10, 20, 30]);
    assert!(interactive.signatures[0].contents_sha256.is_some());
    assert_eq!(interactive.layers[0].initially_visible, Some(true));
    assert_eq!(interactive.embedded_files.len(), 3);
    let nested = interactive
        .embedded_files
        .iter()
        .find(|file| file.artifact.declared_filename.as_deref() == Some("nested.pdf"))
        .unwrap();
    assert_eq!(nested.child_status, PdfEmbeddedChildStatus::Parsed);
    assert_eq!(nested.children.len(), 1);
    assert_ne!(
        nested.artifact.identity.artifact_id,
        nested.children[0].artifact.identity.artifact_id
    );
    assert_eq!(
        nested.children[0].artifact.parent.identity,
        nested.artifact.identity.content
    );
    let encrypted = interactive
        .embedded_files
        .iter()
        .find(|file| file.artifact.declared_filename.as_deref() == Some("secret.pdf"))
        .unwrap();
    assert_eq!(encrypted.child_status, PdfEmbeddedChildStatus::Encrypted);
    assert_eq!(
        encrypted.artifact.extraction.status,
        ArtifactExtractionStatus::Encrypted
    );

    let graph = document
        .to_document_graph(DocumentGraphContext::new("interactive").with_source(source()))
        .unwrap();
    graph.validate_contract().unwrap();
    for kind in [
        DocumentNodeKind::Bookmark,
        DocumentNodeKind::Link,
        DocumentNodeKind::Annotation,
        DocumentNodeKind::Comment,
        DocumentNodeKind::Form,
        DocumentNodeKind::FormField,
        DocumentNodeKind::Attachment,
    ] {
        assert!(graph.nodes.iter().any(|node| node.kind == kind));
    }
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::AttachmentOf)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ReplyTo)
    );
}

#[test]
fn embedded_file_budget_retains_every_child_as_typed_inventory() {
    let mut options = PdfOptions::default();
    options.max_embedded_files = 1;
    let envelope = parse_pdf_bytes(&fixture(), source(), &options);
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "grist.budget.exhausted")
    );
    let files = &envelope.payload.unwrap().interactive.embedded_files;
    assert_eq!(files.len(), 3);
    assert_eq!(
        files
            .iter()
            .filter(|file| file.child_status == PdfEmbeddedChildStatus::BudgetLimited)
            .count(),
        2
    );
    assert!(
        files
            .iter()
            .all(|file| !file.artifact.identity.artifact_id.is_empty())
    );
}
