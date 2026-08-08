#[cfg(feature = "markdown")]
use grist::core::OperationKind;
use grist::core::{
    BudgetProfile, BudgetSelection, CancellationToken, FormatHint, Input, OperationStatus,
    ParseRequest, ProviderSet, RequestId, ResourceBudget, SourceInfo, StreamEvent,
};
use grist::ingest::Ingestor;
#[cfg(feature = "markdown")]
use std::fs;
#[cfg(feature = "markdown")]
use std::io::Cursor;

fn trusted() -> BudgetSelection {
    BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1)
}

fn request(id: &str, input: Input, source_name: &str) -> ParseRequest {
    ParseRequest::new(
        RequestId::new(id).unwrap(),
        input,
        SourceInfo::new(source_name),
        trusted(),
        ProviderSet::none(),
    )
}

#[cfg(feature = "markdown")]
#[test]
fn equivalent_bytes_share_identity_and_payload_across_every_input_adapter() {
    let bytes = b"# unified\n\nbody\n".to_vec();
    let path = std::env::temp_dir().join(format!(
        "grist-unified-ingest-{}-{}.md",
        std::process::id(),
        bytes.len()
    ));
    fs::write(&path, &bytes).unwrap();
    let inputs = vec![
        Input::bytes(bytes.clone()),
        Input::utf8(String::from_utf8(bytes.clone()).unwrap()),
        Input::stream(Cursor::new(bytes.clone())),
        Input::seekable(Cursor::new(bytes.clone())),
        Input::path(path.clone()),
    ];
    let ingestor = Ingestor::builtin().unwrap();
    let envelopes = inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| {
            ingestor
                .ingest(request(&format!("equivalent/{index}"), input, "fixture.md"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    fs::remove_file(path).unwrap();

    let expected_identity = envelopes[0].identity.as_ref().unwrap();
    let expected_payload = envelopes[0].payload.as_ref().unwrap();
    for envelope in &envelopes {
        assert_eq!(envelope.operation, OperationKind::Ingest);
        assert_eq!(envelope.status, OperationStatus::Complete);
        assert_eq!(envelope.identity.as_ref().unwrap(), expected_identity);
        assert_eq!(envelope.payload.as_ref().unwrap(), expected_payload);
        assert_eq!(
            envelope.identity.as_ref().unwrap().raw,
            expected_identity.raw
        );
        assert!(envelope.identity.as_ref().unwrap().decoded.is_some());
        assert!(
            envelope
                .identity
                .as_ref()
                .unwrap()
                .canonical_payload
                .is_some()
        );
    }
}

#[cfg(all(feature = "code", feature = "pdf"))]
#[test]
fn ambiguous_and_malformed_inputs_are_machine_readable_envelopes() {
    let ingestor = Ingestor::builtin().unwrap();
    let ambiguous = ingestor
        .ingest(request("ambiguous", Input::bytes(b"x = 1\n"), "snippet"))
        .unwrap();
    assert_eq!(ambiguous.status, OperationStatus::Ambiguous);
    assert!(ambiguous.payload.is_none());
    assert!(
        ambiguous
            .identity
            .as_ref()
            .unwrap()
            .detection_candidates
            .len()
            >= 2
    );

    let malformed = ingestor
        .ingest(request(
            "malformed",
            Input::bytes(b"%PDF-1.7\n"),
            "fixture.pdf",
        ))
        .unwrap();
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(malformed.payload.is_none());
    assert!(malformed.identity.as_ref().unwrap().raw.is_some());
    assert!(
        malformed
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.input.malformed" })
    );
}

#[test]
fn byte_ingestion_uses_shared_charset_decoding_and_retains_loss() {
    let ingestor = Ingestor::builtin().unwrap();
    let source =
        SourceInfo::new("note.txt").with_declared_mime_type("text/plain; charset=windows-1252");
    let parsed = ingestor
        .ingest(ParseRequest::new(
            RequestId::new("decode").unwrap(),
            Input::bytes(b"caf\xe9"),
            source,
            trusted(),
            ProviderSet::none(),
        ))
        .unwrap();
    assert_eq!(parsed.status, OperationStatus::Complete);
    assert_eq!(
        parsed.payload.as_ref().unwrap()["blocks"][0]["text"],
        "café"
    );
    assert_eq!(
        parsed
            .identity
            .as_ref()
            .unwrap()
            .decoded
            .as_ref()
            .unwrap()
            .encoding,
        "windows-1252"
    );

    let lossy_source =
        SourceInfo::new("lossy.txt").with_declared_mime_type("text/plain; charset=windows-1252");
    let lossy = ingestor
        .ingest(ParseRequest::new(
            RequestId::new("decode-lossy").unwrap(),
            Input::bytes([b'A', 0x81, b'B']),
            lossy_source,
            trusted(),
            ProviderSet::none(),
        ))
        .unwrap();
    assert_eq!(lossy.status, OperationStatus::Partial);
    assert_eq!(
        lossy.diagnostics[0].code.as_str(),
        "decode.replacement.undecodable"
    );
    assert!(
        lossy
            .identity
            .as_ref()
            .unwrap()
            .decoded
            .as_ref()
            .unwrap()
            .lossy
    );
}

#[test]
fn stable_batch_order_and_ids_use_one_shared_budget_tree() {
    let ingestor = Ingestor::builtin().unwrap();
    let requests = ["a", "b", "c"].map(|id| {
        request(id, Input::bytes(b"one"), &format!("{id}.txt"))
            .with_format_hint(FormatHint::exact("text"))
    });
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_input_bytes = Some(5);
    let batch = ingestor
        .batch(
            requests,
            BudgetSelection::custom(budget),
            CancellationToken::new(),
        )
        .unwrap();

    assert_eq!(batch.status, OperationStatus::Partial);
    assert_eq!(batch.items.len(), 2);
    assert_eq!(batch.items[0].sequence, 0);
    assert_eq!(batch.items[0].request_id.as_str(), "a");
    assert_eq!(batch.items[1].sequence, 1);
    assert_eq!(batch.items[1].request_id.as_str(), "b");
    assert_eq!(batch.items[1].payload.status, OperationStatus::Failed);
    assert_eq!(
        batch.items[1].payload.diagnostics[0].code.as_str(),
        "grist.budget.input_bytes.exhausted"
    );
    assert_eq!(batch.budget_usage.input_bytes, 6);
}

#[test]
fn stream_cancellation_is_correlated_and_closes_once() {
    let ingestor = Ingestor::builtin().unwrap();
    let cancellation = CancellationToken::new();
    let requests = ["first", "second"].map(|id| {
        request(id, Input::bytes(b"ok"), &format!("{id}.txt"))
            .with_format_hint(FormatHint::exact("text"))
    });
    let mut stream = ingestor
        .stream(requests, trusted(), cancellation.clone())
        .unwrap();
    assert!(matches!(stream.next(), Some(StreamEvent::Item { .. })));
    cancellation.cancel();
    let cancelled = stream.next().unwrap();
    let StreamEvent::Item { item } = cancelled else {
        panic!("cancelled request must retain correlation");
    };
    assert_eq!(item.request_id.as_str(), "second");
    assert_eq!(item.payload.status, OperationStatus::Cancelled);
    let terminal = stream.next().unwrap();
    let StreamEvent::Terminal { terminal } = terminal else {
        panic!("stream must close with one terminal event");
    };
    assert_eq!(terminal.status, OperationStatus::Cancelled);
    assert_eq!(terminal.emitted_items, 2);
    assert!(stream.next().is_none());
}

#[test]
fn duplicate_batch_ids_are_rejected_before_emission() {
    let ingestor = Ingestor::builtin().unwrap();
    let requests = [
        request("same", Input::bytes(b"a"), "a.txt"),
        request("same", Input::bytes(b"b"), "b.txt"),
    ];
    assert!(
        ingestor
            .stream(requests, trusted(), CancellationToken::new())
            .is_err()
    );
}
