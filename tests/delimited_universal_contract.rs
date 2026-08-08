#![cfg(feature = "csv")]

use grist::core::{
    BudgetSelection, CancellationToken, ContentIdentity, Input, Limits, LocationComponent,
    OperationControl, OperationStatus, ParseRequest, ProviderSet, RequestId, ResourceBudget,
    SourceInfo,
};
use grist::csv::{
    CsvDelimiter, CsvDialectSource, CsvOptions, CsvRecordTerminator, CsvScalarKind,
    CsvStreamRecord, parse_csv, stream_csv,
};
use grist::detect::{ContentKind, DetectionOptions, detect_with_registry};
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::registry::builtin_parser_registry;
use grist::segment::{SegmentOptions, segment_document_graph};
use std::path::Path;

fn control(budget: ResourceBudget) -> OperationControl {
    OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap()
}

#[test]
fn tsv_multiline_quotes_raw_text_candidates_and_exact_ranges_survive() {
    let source = "name\tnote\tvalue\r\nalpha\t\"one\nline\"\t1\r\n";
    let envelope = parse_csv(
        source,
        SourceInfo::stdin("data.tsv"),
        &CsvOptions::default(),
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.dialect.source, CsvDialectSource::Detected);
    assert_eq!(document.dialect.delimiter, CsvDelimiter::Tab);
    assert_eq!(document.newline_fidelity.crlf_count, 2);
    let row = &document.rows[0];
    assert_eq!(row.terminator, CsvRecordTerminator::CrLf);
    assert_eq!(row.range.start_line, 2);
    assert_eq!(row.range.end_line, 3);
    assert_eq!(row.cells[1].raw, "\"one\nline\"");
    assert_eq!(row.cells[1].text, "one\nline");
    assert_eq!(row.cells[2].value, serde_json::json!(1));
    assert!(
        row.cells[2]
            .typed_candidates
            .iter()
            .any(|candidate| candidate.kind == CsvScalarKind::Text)
    );
    assert!(matches!(
        row.cells[1].locator.components()[0],
        LocationComponent::RecordRange { .. }
    ));
}

#[test]
fn malformed_partial_record_is_retained_instead_of_dropped() {
    let envelope = parse_csv(
        "a,b\n1,\"unterminated\n2,3",
        SourceInfo::stdin("bad.csv"),
        &CsvOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let document = envelope.payload().unwrap();
    assert_eq!(document.rows.len(), 1);
    assert!(document.rows[0].malformed);
    assert_eq!(document.rows[0].cells[1].text, "unterminated\n2,3");
}

#[test]
fn extensionless_multiline_tsv_is_detected_structurally() {
    let bytes = b"name\tnote\nalpha\t\"one\nline\"\n";
    let detection = detect_with_registry(
        Path::new("extensionless"),
        bytes,
        None,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(
        detection.content_kind,
        ContentKind::Csv,
        "{:#?}",
        detection.candidates
    );
    assert_eq!(detection.candidates[0].identity.format, "tsv");
}

#[test]
fn declared_dialect_and_stream_budget_terminal_are_explicit() {
    let options = CsvOptions {
        delimiter: CsvDelimiter::Pipe,
        has_headers: false,
        ..Default::default()
    };
    let document = parse_csv("a|b\n1|2\n", SourceInfo::stdin("declared.csv"), &options)
        .into_payload()
        .unwrap();
    assert_eq!(document.dialect.source, CsvDialectSource::Declared);
    assert_eq!(document.rows[0].width, 2);

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_records = Some(1);
    let events = stream_csv(
        "a,b\n1,2\n",
        &CsvOptions::default(),
        RequestId::new("test/stream").unwrap(),
        control(budget),
    )
    .collect::<Vec<_>>();
    assert!(matches!(&events[0], grist::core::StreamEvent::Item { item }
        if matches!(item.payload, CsvStreamRecord::Header(_))));
    let terminal = match events.last().unwrap() {
        grist::core::StreamEvent::Terminal { terminal } => terminal,
        _ => unreachable!(),
    };
    assert_eq!(terminal.status, OperationStatus::Partial);
    assert_eq!(terminal.emitted_items, 1);
    assert_eq!(
        terminal.diagnostics[0].code,
        "grist.budget.records.exhausted"
    );
}

#[test]
fn graph_projection_retains_rows_cells_and_locators() {
    let source = "a,b\n1,2\n";
    let document = parse_csv(
        source,
        SourceInfo::stdin("graph.csv"),
        &CsvOptions::default(),
    )
    .into_payload()
    .unwrap();
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:csv"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Table)
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::TableRow)
            .count(),
        2
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::TableCell)
            .count(),
        4
    );
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::TableCell)
            .all(|node| node.locator.is_some())
    );

    let source_identity = ContentIdentity::for_raw_bytes(source.as_bytes());
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        &source_identity,
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

    let schema = grist::schema::schema_json("csv").unwrap();
    let report = grist::schema::validate_against_schema(
        "csv",
        grist::core::SchemaVersion::CSV_V2,
        &serde_json::to_value(&document).unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
    assert!(
        grist::schema::schema_json_version("csv", grist::core::SchemaVersion::CSV_V1).is_some()
    );
    assert!(grist::schema::schema_json_version("csv-options", "grist/csv-options/v1").is_some());
}

#[test]
fn complete_stream_matches_batch_and_cancellation_is_terminal() {
    let source = "a;b\\r1;true\\n2;2026-08-07";
    let options = CsvOptions::default();
    let batch = parse_csv(source, SourceInfo::stdin("parity.csv"), &options)
        .into_payload()
        .unwrap();
    let events = stream_csv(
        source,
        &options,
        RequestId::new("test/parity").unwrap(),
        control(ResourceBudget::trusted_unbounded()),
    )
    .collect::<Vec<_>>();
    let streamed = events
        .iter()
        .filter_map(|event| match event {
            grist::core::StreamEvent::Item { item } => Some(item.payload.row().clone()),
            grist::core::StreamEvent::Terminal { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(streamed[0], *batch.header_record.as_ref().unwrap());
    assert_eq!(streamed[1..], batch.rows);
    assert!(matches!(
        events.last().unwrap(),
        grist::core::StreamEvent::Terminal { terminal }
            if terminal.status == OperationStatus::Complete
    ));

    let cancellation = CancellationToken::new();
    let cancel_control = OperationControl::new(
        &BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
        cancellation.clone(),
    )
    .unwrap();
    let mut stream = stream_csv(
        source,
        &options,
        RequestId::new("test/cancel").unwrap(),
        cancel_control,
    );
    assert!(matches!(
        stream.next(),
        Some(grist::core::StreamEvent::Item { .. })
    ));
    cancellation.cancel();
    assert!(matches!(
        stream.next(),
        Some(grist::core::StreamEvent::Terminal { terminal })
            if terminal.status == OperationStatus::Cancelled
                && terminal.emitted_items == 1
                && terminal.diagnostics[0].code == "grist.operation.cancelled"
    ));
}

#[test]
fn registry_decodes_utf16_bytes_and_routes_tsv_alias() {
    let mut bytes = vec![0xff, 0xfe];
    for unit in "name\tvalue\r\ncaf\u{00e9}\t1\r\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let request = ParseRequest::new(
        RequestId::new("test/utf16").unwrap(),
        Input::bytes(bytes),
        SourceInfo::stdin("legacy.tsv")
            .with_declared_mime_type("text/tab-separated-values; charset=utf-16le"),
        BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
        ProviderSet::none(),
    );
    let envelope = builtin_parser_registry()
        .unwrap()
        .dispatch("tsv", request, None)
        .unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert_eq!(
        envelope.payload.as_ref().unwrap()["dialect"]["delimiter"],
        "tab"
    );
    assert_eq!(
        envelope.payload.as_ref().unwrap()["rows"][0]["cells"][0]["text"],
        "caf\u{00e9}"
    );
    assert_eq!(
        envelope
            .identity
            .as_ref()
            .unwrap()
            .decoded
            .as_ref()
            .unwrap()
            .encoding,
        "utf-16le"
    );
}

#[test]
fn empty_input_and_cell_budget_do_not_masquerade_as_end_of_input() {
    let empty = parse_csv("", SourceInfo::stdin("empty.csv"), &CsvOptions::default());
    assert_eq!(empty.status, OperationStatus::Complete);
    assert_eq!(empty.payload().unwrap().source_record_count, 0);

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_cells = Some(1);
    let events = stream_csv(
        "a,b\n",
        &CsvOptions::default(),
        RequestId::new("test/cells").unwrap(),
        control(budget),
    )
    .collect::<Vec<_>>();
    let terminal = match events.last().unwrap() {
        grist::core::StreamEvent::Terminal { terminal } => terminal,
        _ => unreachable!(),
    };
    assert_eq!(terminal.status, OperationStatus::Failed);
    assert_eq!(terminal.diagnostics[0].code, "grist.budget.cells.exhausted");
}

#[test]
fn large_input_stream_stops_exactly_at_the_record_budget() {
    let source = (0..5_000)
        .map(|index| format!("{index}\t{}\n", index + 1))
        .collect::<String>();
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_records = Some(64);
    let options = CsvOptions {
        has_headers: false,
        ..Default::default()
    };
    let events = stream_csv(
        &source,
        &options,
        RequestId::new("test/large").unwrap(),
        control(budget),
    )
    .collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, grist::core::StreamEvent::Item { .. }))
            .count(),
        64
    );
    assert!(matches!(
        events.last().unwrap(),
        grist::core::StreamEvent::Terminal { terminal }
            if terminal.status == OperationStatus::Partial && terminal.emitted_items == 64
    ));
}
