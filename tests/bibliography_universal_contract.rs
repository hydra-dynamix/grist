#![cfg(all(
    feature = "bibliography",
    feature = "schemas",
    feature = "document-graph"
))]

use grist::bibliography::{
    BibliographyConstructKind, BibliographyDialect, BibliographyOptions, CitationResolutionStatus,
    CrossrefStatus, ValueExpansionStatus, parse_bibliography,
};
use grist::core::{
    BudgetSelection, Input, Limits, OperationStatus, ParseRequest, ProviderSet, RequestId,
    ResourceBudget, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_source};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::builtin_parser_registry;

const RICH: &str = r#"% source comment
@string{conf = "Proceedings of " # series}
@string{series = {RustConf}}
@preamble{"Generated " # conf}
@xdata{shared,
  publisher = {Example Press},
  location = {Vancouver}
}
@proceedings{parent,
  title = conf,
  year = 2026
}
@inproceedings{child,
  author = {Doe, Jane and Roe, Richard},
  title = "Safe " # {Parsing},
  crossref = {parent},
  xdata = {shared},
  month = jan,
  note = {braces {remain} exact}
}
@comment{retained comment}
raw extension text
"#;

#[test]
fn rich_database_is_lossless_resolved_deterministic_and_schema_valid() {
    let first = parse_bibliography(
        RICH,
        SourceInfo::stdin("references.bib"),
        &BibliographyOptions::default(),
    );
    let second = parse_bibliography(
        RICH,
        SourceInfo::stdin("references.bib"),
        &BibliographyOptions::default(),
    );
    assert_eq!(first, second);
    assert_eq!(first.status, OperationStatus::Complete);
    let payload = first.payload.as_ref().unwrap();
    assert_eq!(payload.dialect, BibliographyDialect::Biblatex);
    assert_eq!(payload.entries.len(), 3);
    assert_eq!(payload.strings.len(), 2);
    assert!(payload.constructs.iter().any(|construct| {
        construct.kind == BibliographyConstructKind::LineComment
            && construct.raw == "% source comment\n"
    }));
    assert!(
        payload
            .constructs
            .iter()
            .any(|construct| construct.kind == BibliographyConstructKind::Raw)
    );
    for construct in &payload.constructs {
        assert_eq!(
            construct.raw,
            payload.decoded_text[construct.range.byte_start..construct.range.byte_end]
        );
        construct.locator.validate().unwrap();
    }
    for entry in &payload.entries {
        assert_eq!(
            entry.raw,
            payload.decoded_text[entry.range.byte_start..entry.range.byte_end]
        );
        for field in &entry.fields {
            assert_eq!(
                field.raw,
                payload.decoded_text[field.range.byte_start..field.range.byte_end]
            );
            for part in &field.value.parts {
                assert_eq!(
                    part.raw,
                    payload.decoded_text[part.range.byte_start..part.range.byte_end]
                );
                part.locator.validate().unwrap();
            }
        }
    }
    let child = payload
        .entries
        .iter()
        .find(|entry| entry.key == "child")
        .unwrap();
    assert_eq!(
        child.field("title").unwrap().value.resolved.as_deref(),
        Some("Safe Parsing")
    );
    assert_eq!(
        child.field("month").unwrap().value.resolved.as_deref(),
        Some("January")
    );
    assert_eq!(
        child.effective_field("publisher").unwrap().source_entry_key,
        "shared"
    );
    assert!(child.effective_field("publisher").unwrap().inherited);
    assert_eq!(
        child
            .effective_field("year")
            .unwrap()
            .value
            .resolved
            .as_deref(),
        Some("2026")
    );
    assert_eq!(
        payload.resolve_citation("child").status,
        CitationResolutionStatus::Resolved
    );
    assert_eq!(
        payload.resolve_citation("missing").status,
        CitationResolutionStatus::Missing
    );

    let graph = payload
        .to_document_graph(
            DocumentGraphContext::new("bibliography:rich").with_source(first.source.clone()),
        )
        .unwrap();
    graph.validate_contract().unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::BibliographyEntry)
            .count(),
        3
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Field)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::References)
    );

    for (name, value) in [
        ("bibliography", serde_json::to_value(payload).unwrap()),
        (
            "bibliography-envelope",
            serde_json::to_value(&first).unwrap(),
        ),
        (
            "bibliography-options",
            serde_json::to_value(BibliographyOptions::default()).unwrap(),
        ),
        (
            "bibliography-citation-resolution",
            serde_json::to_value(payload.resolve_citations(["child", "missing"])).unwrap(),
        ),
    ] {
        let report = grist::schema::validate_schema(name, &value).unwrap();
        assert!(report.valid, "{name}: {:?}", report.issues);
    }
}

#[test]
fn duplicates_cycles_limits_and_malformed_values_are_explicit_partial_results() {
    let source = r#"@string{a = b}
@string{b = a}
@book{dup, title = a}
@article{dup, title = {second}}
@book{left, crossref = {right}}
@book{right, crossref = {left}}
@misc{missing, crossref = {nowhere}}
@online{broken, title = "unterminated}
"#;
    let envelope = parse_bibliography(
        source,
        SourceInfo::stdin("hostile.bib"),
        &BibliographyOptions {
            max_string_depth: 2,
            max_crossref_depth: 2,
            ..Default::default()
        },
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let payload = envelope.payload.unwrap();
    assert_eq!(payload.duplicate_keys[0].entry_indices, vec![0, 1]);
    assert_eq!(
        payload.resolve_citation("dup").status,
        CitationResolutionStatus::Ambiguous
    );
    assert!(payload.entries[0].field("title").is_some_and(|field| {
        matches!(
            field.value.expansion_status,
            ValueExpansionStatus::Cycle | ValueExpansionStatus::DepthExceeded
        )
    }));
    assert!(
        payload
            .crossrefs
            .iter()
            .any(|resolution| resolution.status == CrossrefStatus::Cycle)
    );
    assert!(
        payload
            .crossrefs
            .iter()
            .any(|resolution| resolution.status == CrossrefStatus::Unresolved)
    );
    assert!(
        payload
            .constructs
            .iter()
            .any(|construct| construct.kind == BibliographyConstructKind::Entry)
    );
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.partial)
    );
}

#[test]
fn empty_raw_and_every_local_resolution_limit_have_explicit_semantics() {
    let empty = parse_bibliography(
        "",
        SourceInfo::stdin("empty.bib"),
        &BibliographyOptions::default(),
    );
    assert_eq!(empty.status, OperationStatus::Complete);
    assert!(empty.payload.unwrap().entries.is_empty());

    let raw = parse_bibliography(
        "vendor extension without an at-sign\n",
        SourceInfo::stdin("raw.bib"),
        &BibliographyOptions::default(),
    );
    assert_eq!(raw.status, OperationStatus::Complete);
    assert_eq!(
        raw.payload.unwrap().constructs[0].kind,
        BibliographyConstructKind::Raw
    );

    for (options, expected) in [
        (
            BibliographyOptions {
                max_string_depth: 0,
                ..Default::default()
            },
            ValueExpansionStatus::DepthExceeded,
        ),
        (
            BibliographyOptions {
                max_string_expansions: 0,
                ..Default::default()
            },
            ValueExpansionStatus::ExpansionLimit,
        ),
        (
            BibliographyOptions {
                max_expanded_characters: 2,
                ..Default::default()
            },
            ValueExpansionStatus::OutputLimit,
        ),
    ] {
        let limited = parse_bibliography(
            "@string{s={long}}\n@book{k,title=s}",
            SourceInfo::stdin("limited.bib"),
            &options,
        );
        assert_eq!(limited.status, OperationStatus::Partial);
        assert_eq!(
            limited.payload.unwrap().entries[0].fields[0]
                .value
                .expansion_status,
            expected
        );
    }

    let crossref_limited = parse_bibliography(
        "@book{parent,title={P}}\n@book{child,crossref={parent}}",
        SourceInfo::stdin("crossref-limit.bib"),
        &BibliographyOptions {
            max_crossref_depth: 0,
            ..Default::default()
        },
    );
    assert_eq!(crossref_limited.status, OperationStatus::Partial);
    assert_eq!(
        crossref_limited.payload.unwrap().crossrefs[0].status,
        CrossrefStatus::DepthExceeded
    );
}

#[test]
fn detection_registry_aliases_and_budget_contracts_agree() {
    let registry = builtin_parser_registry().unwrap();
    for name in ["references.bib", "extensionless", "mislabeled.bin"] {
        let detection = detect_source(
            &SourceInfo::stdin(name),
            RICH.as_bytes(),
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detection.status, DetectionStatus::Selected, "{name}");
        assert_eq!(
            detection
                .selected_format_identity()
                .map(|identity| identity.format),
            Some("bibtex".to_string())
        );
    }
    for alias in ["bibtex", "biblatex", "bibliography", "bib"] {
        let request = ParseRequest::new(
            RequestId::new(format!("parse-{alias}")).unwrap(),
            Input::bytes(RICH.as_bytes().to_vec()),
            SourceInfo::stdin("references.bib"),
            BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
            ProviderSet::none(),
        );
        let envelope = registry.dispatch(alias, request, None).unwrap();
        assert_eq!(envelope.status, OperationStatus::Complete, "{alias}");
    }
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_decoded_characters = Some(1);
    let request = ParseRequest::new(
        RequestId::new("bibliography-budget").unwrap(),
        Input::bytes(RICH.as_bytes().to_vec()),
        SourceInfo::stdin("references.bib"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let failed = registry.dispatch("bibtex", request, None).unwrap();
    assert_eq!(failed.status, OperationStatus::Failed);
}
