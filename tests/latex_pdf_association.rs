#![cfg(all(feature = "latex", feature = "pdf"))]

use grist::core::{
    BudgetAxis, BudgetSelection, CancellationToken, CitationSourceVersion,
    CitationVerificationMethod, CitationVerificationOptions, CitationVerificationOutcome,
    DeclaredLoss, Diagnostic, LocationComponent, LossClass, OperationControl,
    OperationControlError, OperationStatus, ResourceBudget, SourceInfo,
};
use grist::latex::{
    LatexNodeKind, LatexOptions, LatexPdfAssociation, LatexPdfAssociationError,
    LatexPdfAssociationOptions, LatexPdfCorrespondence, LatexPdfCorrespondenceStatus,
    associate_latex_pdf, associate_latex_pdf_with_control, parse_latex, parse_latex_bytes,
};
use grist::pdf::{PdfOptions, parse_pdf_bytes};
use std::fs;

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

fn font() -> Vec<u8> {
    let widths = (32..=122).map(|_| "600").collect::<Vec<_>>().join(" ");
    format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 122 /Widths [{widths}] /FontDescriptor << /Flags 32 /FontWeight 400 /ItalicAngle 0 >> >>").into_bytes()
}

fn rendered_fixture(lines: &[(&str, i32, i32)]) -> Vec<u8> {
    let mut content = b"BT /F1 12 Tf ".to_vec();
    for (text, x, y) in lines {
        content.extend_from_slice(format!("1 0 0 1 {x} {y} Tm ({text}) Tj ").as_bytes());
    }
    content.extend_from_slice(b"ET");
    pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        stream(&content),
        font(),
    ])
}

fn association(source_text: &str, rendered: &[(&str, i32, i32)]) -> (LatexPdfAssociation, String) {
    let latex = parse_latex(
        &format!("\\section{{{source_text}}}"),
        SourceInfo::stdin("paper.tex"),
        &LatexOptions::default(),
    );
    let section_id = latex
        .payload
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|node| node.kind == LatexNodeKind::Section)
        .unwrap()
        .id
        .clone();
    let pdf = parse_pdf_bytes(
        &rendered_fixture(rendered),
        SourceInfo::new("paper.pdf").with_declared_mime_type("application/pdf"),
        &PdfOptions::default(),
    );
    let association = associate_latex_pdf(&latex, &pdf, &LatexPdfAssociationOptions::default())
        .expect("caller-supplied association");
    (association, section_id)
}

fn section<'a>(
    association: &'a LatexPdfAssociation,
    section_id: &str,
) -> &'a LatexPdfCorrespondence {
    association
        .correspondences
        .iter()
        .find(|item| item.source_node_id == section_id)
        .unwrap()
}

#[test]
fn exact_association_retains_dual_identity_locators_citations_and_provenance() {
    let (first, section_id) = association(
        "Exact rendered phrase",
        &[("Exact rendered phrase", 50, 700)],
    );
    let (second, _) = association(
        "Exact rendered phrase",
        &[("Exact rendered phrase", 50, 700)],
    );
    assert_eq!(first, second, "association replay must be deterministic");

    let mapping = section(&first, &section_id);
    assert_eq!(mapping.status, LatexPdfCorrespondenceStatus::Exact);
    assert_eq!(mapping.confidence.get(), 1.0);
    assert_eq!(mapping.rendered_matches.len(), 1);
    assert!(matches!(
        mapping.source_locator.innermost(),
        LocationComponent::TextRange { .. }
    ));
    assert!(matches!(
        mapping.rendered_matches[0].locator.innermost(),
        LocationComponent::PdfRegion { .. }
    ));
    assert_ne!(
        first.source_representation.identity,
        first.rendered_representation.identity
    );
    assert_eq!(first.source_representation.source.display_name, "paper.tex");
    assert_eq!(
        first.rendered_representation.source.display_name,
        "paper.pdf"
    );
    assert!(!first.source_representation.provenance.is_empty());
    assert!(!first.rendered_representation.provenance.is_empty());
    assert_eq!(first.provenance.len(), 1);
    assert_ne!(
        first.provenance[0].input_identity.as_deref(),
        first
            .source_representation
            .identity
            .raw
            .as_ref()
            .map(|hash| hash.sha256.as_str())
    );
    assert_eq!(
        first.provenance[0].output_identity.as_deref(),
        first
            .identity
            .canonical_payload
            .as_ref()
            .map(|hash| hash.sha256.as_str())
    );
    assert_eq!(first.provenance[0].declared_loss(), DeclaredLoss::Lossless);
    assert!(first.caller_supplied_pdf);

    let source_version =
        CitationSourceVersion::new(first.source_representation.identity.clone(), Vec::new());
    let source_verification =
        mapping.verify_source(&source_version, CitationVerificationOptions::default());
    assert_eq!(
        source_verification.outcome,
        CitationVerificationOutcome::Exact
    );
    assert_eq!(
        source_verification.method,
        CitationVerificationMethod::SourceHash
    );
    let rendered_version =
        CitationSourceVersion::new(first.rendered_representation.identity.clone(), Vec::new());
    let rendered_verification = mapping
        .verify_rendered(0, &rendered_version, CitationVerificationOptions::default())
        .unwrap();
    assert_eq!(
        rendered_verification.outcome,
        CitationVerificationOutcome::Exact
    );
}

#[test]
fn reordered_tokens_are_partial_not_exact() {
    let (association, section_id) =
        association("alpha beta gamma", &[("gamma beta alpha", 50, 700)]);
    let mapping = section(&association, &section_id);
    assert_eq!(mapping.status, LatexPdfCorrespondenceStatus::Partial);
    assert!(mapping.confidence.get() < 1.0);
}

#[test]
fn a_rendered_occurrence_is_not_reused_for_duplicate_source_occurrences() {
    let latex = parse_latex(
        "\\section{Repeated phrase}\\section{Repeated phrase}",
        SourceInfo::stdin("paper.tex"),
        &LatexOptions::default(),
    );
    let section_ids = latex
        .payload
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter(|node| node.kind == LatexNodeKind::Section)
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let pdf = parse_pdf_bytes(
        &rendered_fixture(&[("Repeated phrase", 50, 700)]),
        SourceInfo::new("paper.pdf").with_declared_mime_type("application/pdf"),
        &PdfOptions::default(),
    );
    let result = associate_latex_pdf(&latex, &pdf, &LatexPdfAssociationOptions::default()).unwrap();
    let repeated = result
        .correspondences
        .iter()
        .filter(|item| section_ids.contains(&item.source_node_id))
        .collect::<Vec<_>>();
    assert_eq!(repeated.len(), 2);
    assert_eq!(
        repeated
            .iter()
            .filter(|item| !item.rendered_matches.is_empty())
            .count(),
        1
    );
    assert_eq!(result.status, LatexPdfCorrespondenceStatus::Partial);
}

#[test]
fn partial_inputs_and_stale_expected_identities_cannot_claim_lossless_exactness() {
    let mut latex = parse_latex(
        "\\section{Exact rendered phrase}",
        SourceInfo::stdin("paper.tex"),
        &LatexOptions::default(),
    );
    latex.status = OperationStatus::Partial;
    latex.diagnostics.push(Diagnostic::warning(
        "test",
        "test.partial",
        "fixture is intentionally partial",
    ));
    let pdf = parse_pdf_bytes(
        &rendered_fixture(&[("Exact rendered phrase", 50, 700)]),
        SourceInfo::new("paper.pdf").with_declared_mime_type("application/pdf"),
        &PdfOptions::default(),
    );

    let partial =
        associate_latex_pdf(&latex, &pdf, &LatexPdfAssociationOptions::default()).unwrap();
    assert_eq!(partial.status, LatexPdfCorrespondenceStatus::Partial);
    assert_eq!(
        partial.source_representation.status,
        OperationStatus::Partial
    );
    assert_eq!(partial.source_representation.diagnostics, latex.diagnostics);
    assert!(matches!(
        partial.provenance[0].declared_loss(),
        DeclaredLoss::Lossy(ref class) if class == &LossClass::from(LossClass::PRECISION_REDUCED)
    ));

    let mut stale_options = LatexPdfAssociationOptions::default();
    stale_options.expected_source_identity =
        Some(grist::core::ContentIdentity::for_raw_bytes(b"old source"));
    let stale = associate_latex_pdf(&latex, &pdf, &stale_options).unwrap();
    assert_eq!(stale.status, LatexPdfCorrespondenceStatus::Stale);
    assert!(stale.correspondences.iter().all(|item| {
        item.status == LatexPdfCorrespondenceStatus::Stale
            && item
                .evidence
                .iter()
                .any(|evidence| evidence.contains("stale"))
    }));
    assert!(matches!(
        stale.provenance[0].declared_loss(),
        DeclaredLoss::Lossy(_)
    ));
}

#[test]
fn cartesian_matching_work_is_precharged_to_the_records_budget() {
    let latex = parse_latex(
        "\\section{Budgeted phrase}",
        SourceInfo::stdin("paper.tex"),
        &LatexOptions::default(),
    );
    let pdf = parse_pdf_bytes(
        &rendered_fixture(&[("Budgeted phrase", 50, 700), ("Other phrase", 50, 650)]),
        SourceInfo::new("paper.pdf").with_declared_mime_type("application/pdf"),
        &PdfOptions::default(),
    );
    let mut options = LatexPdfAssociationOptions::default();
    options.budget = ResourceBudget::trusted_unbounded();
    options.budget.max_records = Some(1);
    let error = associate_latex_pdf(&latex, &pdf, &options).unwrap_err();
    assert!(matches!(
        error,
        LatexPdfAssociationError::BudgetExceeded(ref exceeded)
            if exceeded.axis == BudgetAxis::Records
    ));
}

#[test]
fn included_file_citations_use_the_included_files_identity() {
    let base = std::env::temp_dir().join(format!(
        "grist-latex-pdf-association-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let main = base.join("main.tex");
    let child = base.join("child.tex");
    let main_bytes = b"\\input{child}";
    let child_bytes = b"\\section{Included phrase}";
    fs::write(&main, main_bytes).unwrap();
    fs::write(&child, child_bytes).unwrap();
    let options = LatexOptions {
        allowed_roots: vec![base.clone()],
        ..Default::default()
    };
    let latex = parse_latex_bytes(main_bytes, SourceInfo::from_path(&main), &options);
    let pdf = parse_pdf_bytes(
        &rendered_fixture(&[("Included phrase", 50, 700)]),
        SourceInfo::new("paper.pdf").with_declared_mime_type("application/pdf"),
        &PdfOptions::default(),
    );
    let result = associate_latex_pdf(&latex, &pdf, &LatexPdfAssociationOptions::default()).unwrap();
    let included = result
        .correspondences
        .iter()
        .find(|item| {
            item.source_citation
                .source
                .display_name
                .ends_with("child.tex")
        })
        .expect("included node correspondence");
    assert_eq!(included.status, LatexPdfCorrespondenceStatus::Exact);
    assert_eq!(
        included.source_citation.source_identity.raw,
        grist::core::ContentIdentity::for_raw_bytes(child_bytes).raw
    );
    assert_ne!(
        included.source_citation.source_identity.raw,
        result.source_representation.identity.raw
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn caller_owned_cancellation_stops_association_before_matching() {
    let latex = parse_latex(
        "\\section{Cancelled phrase}",
        SourceInfo::stdin("paper.tex"),
        &LatexOptions::default(),
    );
    let pdf = parse_pdf_bytes(
        &rendered_fixture(&[("Cancelled phrase", 50, 700)]),
        SourceInfo::new("paper.pdf").with_declared_mime_type("application/pdf"),
        &PdfOptions::default(),
    );
    let options = LatexPdfAssociationOptions::default();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let control = OperationControl::new(
        &BudgetSelection::custom(options.budget.clone()),
        cancellation,
    )
    .unwrap();
    let error = associate_latex_pdf_with_control(&latex, &pdf, &options, &control).unwrap_err();
    assert!(matches!(
        error,
        LatexPdfAssociationError::Control(OperationControlError::Cancelled(_))
    ));
}

#[test]
fn partial_changed_ambiguous_and_unmatched_states_are_explicit() {
    let (partial, partial_id) = association(
        "Partial rendered phrase",
        &[("Partial rendered phrase extended", 50, 700)],
    );
    let partial = section(&partial, &partial_id);
    assert_eq!(partial.status, LatexPdfCorrespondenceStatus::Partial);
    assert!(partial.confidence.get() > 0.72 && partial.confidence.get() < 1.0);

    let (changed, changed_id) = association(
        "Changed rendered phrase original",
        &[("Changed rendered revised output", 50, 700)],
    );
    let changed = section(&changed, &changed_id);
    assert_eq!(changed.status, LatexPdfCorrespondenceStatus::Changed);
    assert!(changed.confidence.get() >= 0.25 && changed.confidence.get() < 0.72);

    let (ambiguous, ambiguous_id) = association(
        "Duplicated rendered phrase",
        &[
            ("Duplicated rendered phrase", 50, 700),
            ("Duplicated rendered phrase", 350, 700),
        ],
    );
    let ambiguous = section(&ambiguous, &ambiguous_id);
    assert_eq!(ambiguous.status, LatexPdfCorrespondenceStatus::Ambiguous);
    assert_eq!(ambiguous.rendered_matches.len(), 2);
    assert!(ambiguous.confidence.get() < 1.0);

    let (unmatched, unmatched_id) = association(
        "Source words absent",
        &[("Entirely different output", 50, 700)],
    );
    let unmatched_mapping = section(&unmatched, &unmatched_id);
    assert_eq!(
        unmatched_mapping.status,
        LatexPdfCorrespondenceStatus::Unmatched
    );
    assert!(unmatched_mapping.rendered_matches.is_empty());
    assert!(!unmatched.unmatched_rendered.is_empty());
}

#[test]
fn default_latex_parse_has_no_implicit_pdf_association_or_io_surface() {
    let first = parse_latex(
        "\\section{Source only}",
        SourceInfo::stdin("source-only.tex"),
        &LatexOptions::default(),
    );
    let second = parse_latex(
        "\\section{Source only}",
        SourceInfo::stdin("source-only.tex"),
        &LatexOptions::default(),
    );
    assert_eq!(first, second);
    let payload = serde_json::to_value(first.payload.unwrap()).unwrap();
    assert!(payload.get("compiled_pdf").is_none());
    let options = serde_json::to_value(LatexOptions::default()).unwrap();
    assert!(options.get("pdf").is_none());
}
