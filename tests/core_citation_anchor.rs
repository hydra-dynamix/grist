use grist::core::{
    CellAddress, CitationAnchor, CitationAnchorOptions, CitationCandidate, CitationSourceVersion,
    CitationTargetKind, CitationVerificationMethod, CitationVerificationOptions,
    CitationVerificationOutcome, ContentIdentity, IndexPosition, LineIndex, LocationComponent,
    SourceInfo, SourceLocator, SourceRange, citation_label, normalize_citation_text,
};
use grist::document_graph::{
    DocumentGraph, DocumentGraphCitationError, DocumentKind, DocumentNode, DocumentNodeKind,
};

fn locator(text: &str, start: usize, end: usize) -> SourceLocator {
    SourceLocator::try_from(SourceRange::new(start, end, &LineIndex::new(text))).unwrap()
}

fn identity(bytes: &[u8]) -> ContentIdentity {
    ContentIdentity::for_raw_bytes(bytes)
}

fn anchor() -> CitationAnchor {
    CitationAnchor::for_node(
        SourceInfo::stdin("paper.md"),
        identity(b"# Methods\nAlpha beta."),
        "node-methods",
        locator("# Methods\nAlpha beta.", 10, 21),
        "  Alpha\t beta.  ",
        Some("§ Methods".to_string()),
        CitationAnchorOptions::new(8).unwrap(),
    )
    .unwrap()
}

fn candidate(id: &str, at: SourceLocator, text: &str) -> CitationCandidate {
    CitationCandidate::new(
        CitationTargetKind::Node,
        id,
        vec![id.to_string()],
        vec![at],
        text,
    )
    .unwrap()
}

#[test]
fn anchors_record_source_hash_normalized_hash_labels_and_bounded_excerpts() {
    let anchor = anchor();
    assert_eq!(anchor.schema_version, "grist/citation-anchor/v1");
    assert_eq!(anchor.node_ids, ["node-methods"]);
    assert_eq!(anchor.label, "§ Methods");
    assert_eq!(normalize_citation_text("  Alpha\t beta.  "), "Alpha beta.");
    assert_eq!(anchor.excerpt.as_deref(), Some("Alpha b…"));
    assert!(anchor.source_content_hash.sha256().starts_with("sha256:"));
    assert!(anchor.normalized_text_hash.starts_with("sha256:"));
}

#[test]
fn segment_anchor_retains_ordered_nodes_and_disjoint_locators() {
    let source = "one two three";
    let first = locator(source, 0, 3);
    let second = locator(source, 8, 13);
    let anchor = CitationAnchor::for_segment(
        SourceInfo::stdin("notes.txt"),
        identity(source.as_bytes()),
        "segment-7",
        vec!["node-1".into(), "node-3".into()],
        vec![first.clone(), second.clone()],
        "one three",
        None,
        CitationAnchorOptions::default(),
    )
    .unwrap();

    assert_eq!(anchor.target_kind, CitationTargetKind::Segment);
    assert_eq!(anchor.node_ids, ["node-1", "node-3"]);
    assert_eq!(anchor.locators, [first, second]);
}

#[test]
fn exact_verification_accepts_the_identical_source_version() {
    let anchor = anchor();
    let version = CitationSourceVersion::new(anchor.source_identity.clone(), Vec::new());
    let result = anchor.verify(&version, CitationVerificationOptions::default());
    assert_eq!(result.outcome, CitationVerificationOutcome::Exact);
    assert_eq!(result.method, CitationVerificationMethod::SourceHash);
    assert!(result.current_locators.is_empty());
}

#[test]
fn relocation_is_explicit_and_retains_original_and_current_locators() {
    let anchor = anchor();
    let moved = locator("prefix\nAlpha beta.", 7, 18);
    let version = CitationSourceVersion::new(
        identity(b"prefix\nAlpha beta."),
        vec![candidate("new-node-id", moved.clone(), "Alpha beta.")],
    );
    let unchanged_anchor = anchor.clone();
    let result = anchor.verify(&version, CitationVerificationOptions::default());

    assert_eq!(anchor, unchanged_anchor);
    assert_eq!(result.outcome, CitationVerificationOutcome::Relocated);
    assert_eq!(
        result.method,
        CitationVerificationMethod::BoundedContentMatch
    );
    assert_eq!(result.original_locators, anchor.locators);
    assert_eq!(result.current_locators, [moved]);
    assert_eq!(result.matched_target_id.as_deref(), Some("new-node-id"));
}

#[test]
fn stable_target_identity_relocates_without_content_search() {
    let anchor = anchor();
    let moved = locator("prefix\nAlpha beta.", 7, 18);
    let version = CitationSourceVersion::new(
        identity(b"prefix\nAlpha beta."),
        vec![candidate("node-methods", moved, "Alpha beta.")],
    );
    let result = anchor.verify(&version, CitationVerificationOptions::default());
    assert_eq!(result.outcome, CitationVerificationOutcome::Relocated);
    assert_eq!(
        result.method,
        CitationVerificationMethod::StableTargetIdentity
    );
}

#[test]
fn stable_target_with_different_text_is_changed() {
    let anchor = anchor();
    let version = CitationSourceVersion::new(
        identity(b"# Methods\nGamma."),
        vec![candidate(
            "node-methods",
            anchor.locators[0].clone(),
            "Gamma.",
        )],
    );
    let result = anchor.verify(&version, CitationVerificationOptions::default());
    assert_eq!(result.outcome, CitationVerificationOutcome::Changed);
    assert_eq!(result.current_locators, anchor.locators);
}

#[test]
fn unmatched_content_is_missing() {
    let anchor = anchor();
    let version = CitationSourceVersion::new(
        identity(b"entirely different"),
        vec![candidate(
            "different-node",
            locator("entirely different", 0, 18),
            "entirely different",
        )],
    );
    let result = anchor.verify(&version, CitationVerificationOptions::default());
    assert_eq!(result.outcome, CitationVerificationOutcome::Missing);
    assert_eq!(result.method, CitationVerificationMethod::NoMatch);
}

#[test]
fn unavailable_ambiguous_and_over_budget_versions_are_unverifiable() {
    let anchor = anchor();
    let unavailable = CitationSourceVersion::new(ContentIdentity::default(), Vec::new());
    let unavailable_result = anchor.verify(&unavailable, CitationVerificationOptions::default());
    assert_eq!(
        unavailable_result.outcome,
        CitationVerificationOutcome::Unverifiable
    );
    assert_eq!(
        unavailable_result.method,
        CitationVerificationMethod::SourceHashUnavailable
    );

    let mut forged = CitationSourceVersion::new(identity(b"different bytes"), Vec::new());
    forged.source_content_hash = Some(anchor.source_content_hash.clone());
    let forged_result = anchor.verify(&forged, CitationVerificationOptions::default());
    assert_eq!(
        forged_result.outcome,
        CitationVerificationOutcome::Unverifiable
    );
    assert_eq!(
        forged_result.method,
        CitationVerificationMethod::IdentityMismatch
    );

    let duplicate_text = CitationSourceVersion::new(
        identity(b"changed source with Alpha beta twice"),
        vec![
            candidate(
                "candidate-a",
                locator("Alpha beta. x", 0, 11),
                "Alpha beta.",
            ),
            candidate(
                "candidate-b",
                locator("x Alpha beta.", 2, 12),
                "Alpha beta.",
            ),
        ],
    );
    let ambiguous = anchor.verify(&duplicate_text, CitationVerificationOptions::default());
    assert_eq!(ambiguous.outcome, CitationVerificationOutcome::Unverifiable);
    assert_eq!(
        ambiguous.method,
        CitationVerificationMethod::AmbiguousContentMatch
    );

    let over_budget = anchor.verify(
        &duplicate_text,
        CitationVerificationOptions { max_candidates: 1 },
    );
    assert_eq!(
        over_budget.outcome,
        CitationVerificationOutcome::Unverifiable
    );
    assert_eq!(
        over_budget.method,
        CitationVerificationMethod::CandidateBudgetExceeded
    );
}

#[test]
fn verification_outcomes_have_stable_golden_wire_names() {
    let outcomes = [
        CitationVerificationOutcome::Exact,
        CitationVerificationOutcome::Relocated,
        CitationVerificationOutcome::Changed,
        CitationVerificationOutcome::Missing,
        CitationVerificationOutcome::Unverifiable,
    ];
    assert_eq!(
        serde_json::to_value(outcomes).unwrap(),
        serde_json::json!(["exact", "relocated", "changed", "missing", "unverifiable"])
    );
}

#[test]
fn graph_emits_anchors_and_version_candidates_for_every_addressable_node() {
    let text = "Methods\nAlpha beta.";
    let mut graph = DocumentGraph::new("graph-1", DocumentKind::Document)
        .with_source(SourceInfo::stdin("paper.md"));
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document));
    graph.add_node(
        DocumentNode::new("heading", DocumentNodeKind::Heading)
            .with_text("Methods")
            .with_range(SourceRange::new(0, 7, &LineIndex::new(text))),
    );
    graph.add_node(
        DocumentNode::new("paragraph", DocumentNodeKind::Paragraph)
            .with_text("Alpha beta.")
            .with_range(SourceRange::new(8, 19, &LineIndex::new(text))),
    );
    let source_identity = identity(text.as_bytes());

    let anchors = graph
        .citation_anchors(&source_identity, CitationAnchorOptions::default())
        .unwrap();
    assert_eq!(anchors.len(), 2);
    assert_eq!(anchors[0].label, "§ Methods");
    let version = graph
        .citation_source_version(source_identity.clone())
        .unwrap();
    assert_eq!(version.candidates.len(), 2);

    let error = graph
        .citation_anchor_for_node(&source_identity, "root", CitationAnchorOptions::default())
        .unwrap_err();
    assert!(matches!(
        error,
        DocumentGraphCitationError::UnaddressableNode(id) if id == "root"
    ));
}

#[test]
fn labels_cover_page_slide_and_sheet_coordinates() {
    let page = SourceLocator::exact(LocationComponent::PdfRegion {
        page: IndexPosition::one_based(14).unwrap(),
        bbox: None,
        rotation_degrees: None,
        tokens: None,
    })
    .unwrap();
    let slide = SourceLocator::exact(LocationComponent::SlideRegion {
        slide: IndexPosition::zero_based(6),
        shape_id: Some("shape-1".into()),
        bbox: None,
    })
    .unwrap();
    let sheet = SourceLocator::exact(LocationComponent::SheetRange {
        sheet: "Sheet1".into(),
        start_cell: CellAddress::a1(4, 2).unwrap(),
        end_cell: CellAddress::a1(9, 4).unwrap(),
    })
    .unwrap();

    assert_eq!(citation_label(&page), "page 14");
    assert_eq!(citation_label(&slide), "slide 7");
    assert_eq!(citation_label(&sheet), "Sheet1!B4:D9");
}

#[cfg(feature = "schemas")]
#[test]
fn citation_schemas_are_discoverable_and_expose_all_outcomes() {
    for name in [
        "citation-anchor",
        "citation-source-version",
        "citation-verification",
    ] {
        assert!(
            grist::schema::list_schemas()
                .iter()
                .any(|entry| entry.name == name)
        );
        assert!(grist::schema::schema_json(name).is_some());
    }
    let schema = grist::schema::schema_json("citation-verification")
        .unwrap()
        .to_string();
    for outcome in ["exact", "relocated", "changed", "missing", "unverifiable"] {
        assert!(schema.contains(outcome));
    }
}
