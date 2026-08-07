use grist::core::{
    BudgetProfile, BudgetSelection, ContentIdentity, Input, OperationStatus, ParseRequest,
    ProviderSet, RequestId, ResourceBudget, SchemaVersion, SourceInfo, canonical_json_bytes,
};
use grist::decode::TextEncoding;
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::registry::builtin_parser_registry;
use grist::segment::{SegmentOptions, segment_document_graph};
use grist::text::{TextDocument, TextOptions};
use serde_json::Value;
#[cfg(feature = "cli")]
use serde_json::json;

fn dispatch(
    bytes: Vec<u8>,
    source: SourceInfo,
    options: Option<TextOptions>,
) -> grist::core::Envelope<Value> {
    dispatch_with_budget(
        bytes,
        source,
        options,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
    )
}

fn dispatch_with_budget(
    bytes: Vec<u8>,
    source: SourceInfo,
    options: Option<TextOptions>,
    budget: BudgetSelection,
) -> grist::core::Envelope<Value> {
    let request = ParseRequest::new(
        RequestId::new("text-contract").unwrap(),
        Input::bytes(bytes),
        source,
        budget,
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch(
            "text",
            request,
            options.map(|options| serde_json::to_value(options).unwrap()),
        )
        .unwrap()
}

fn document(envelope: &grist::core::Envelope<Value>) -> TextDocument {
    serde_json::from_value(envelope.payload.clone().expect("text payload")).unwrap()
}

fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
    let mut bytes = if big_endian {
        vec![0xfe, 0xff]
    } else {
        vec![0xff, 0xfe]
    };
    for unit in text.encode_utf16() {
        bytes.extend(if big_endian {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        });
    }
    bytes
}

fn utf32(text: &str, big_endian: bool) -> Vec<u8> {
    let mut bytes = if big_endian {
        vec![0x00, 0x00, 0xfe, 0xff]
    } else {
        vec![0xff, 0xfe, 0x00, 0x00]
    };
    for character in text.chars() {
        bytes.extend(if big_endian {
            (character as u32).to_be_bytes()
        } else {
            (character as u32).to_le_bytes()
        });
    }
    bytes
}

#[test]
fn utf_variants_preserve_bytes_encoding_text_newlines_and_raw_ranges() {
    let text = "alpha\r\nbeta";
    let mut utf8_bom = vec![0xef, 0xbb, 0xbf];
    utf8_bom.extend_from_slice(text.as_bytes());
    let cases = [
        (text.as_bytes().to_vec(), TextEncoding::Utf8, 0_u64),
        (utf8_bom, TextEncoding::Utf8, 3),
        (utf16(text, false), TextEncoding::Utf16Le, 2),
        (utf16(text, true), TextEncoding::Utf16Be, 2),
        (utf32(text, false), TextEncoding::Utf32Le, 4),
        (utf32(text, true), TextEncoding::Utf32Be, 4),
    ];

    for (bytes, expected_encoding, content_start) in cases {
        let envelope = dispatch(bytes.clone(), SourceInfo::stdin("variant.txt"), None);
        assert_eq!(envelope.status, OperationStatus::Complete);
        assert!(
            envelope
                .identity
                .as_ref()
                .unwrap()
                .canonical_payload
                .is_some()
        );
        let payload = document(&envelope);
        assert_eq!(payload.schema_version, SchemaVersion::TEXT_V2);
        assert_eq!(payload.raw_bytes, bytes);
        assert_eq!(payload.decoded_text, text);
        assert_eq!(payload.encoding, expected_encoding);
        assert_eq!(payload.decoding.newlines.crlf_count, 1);
        assert_eq!(payload.blocks.len(), 1);
        assert_eq!(payload.blocks[0].raw_range.start, content_start);
        assert_eq!(payload.blocks[0].raw_range.end, payload.raw_range.end);
        assert_eq!(payload.blocks[0].range.byte_start, 0);
        assert_eq!(payload.blocks[0].range.byte_end, text.len());
        payload.blocks[0].locator.validate().unwrap();
    }
}

#[test]
fn windows_1252_and_invalid_utf8_have_distinct_complete_and_partial_results() {
    let cp1252 = dispatch(
        b"left \x93quote\x94\r\nright".to_vec(),
        SourceInfo::stdin("cp1252.txt").with_declared_mime_type("text/plain; charset=windows-1252"),
        None,
    );
    assert_eq!(cp1252.status, OperationStatus::Complete);
    let cp1252_payload = document(&cp1252);
    assert_eq!(cp1252_payload.encoding, TextEncoding::Windows1252);
    assert_eq!(cp1252_payload.decoded_text, "left “quote”\r\nright");
    assert_eq!(cp1252_payload.raw_bytes[5], 0x93);

    let invalid = dispatch(
        b"before \xff after".to_vec(),
        SourceInfo::stdin("invalid.txt"),
        Some(TextOptions {
            encoding: Some("utf-8".into()),
        }),
    );
    assert_eq!(invalid.status, OperationStatus::Partial);
    let diagnostic = invalid
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_str() == "decode.replacement.undecodable")
        .expect("partial decoding diagnostic");
    diagnostic.locator.as_ref().unwrap().validate().unwrap();
    assert!(diagnostic.partial);
    let invalid_payload = document(&invalid);
    assert_eq!(invalid_payload.raw_bytes, b"before \xff after");
    assert_eq!(invalid_payload.decoded_text, "before � after");
    assert!(invalid_payload.decoding.is_lossy());
    assert_eq!(invalid_payload.decoding.issues[0].raw_range.start, 7);
    assert!(
        invalid
            .provenance
            .iter()
            .any(|step| step.implementation.starts_with("grist.decode@")
                && step.loss_class.is_some())
    );
}

#[test]
fn empty_large_mixed_newline_and_malformed_decoding_cases_are_explicit() {
    let empty = dispatch(Vec::new(), SourceInfo::stdin("empty.txt"), None);
    assert_eq!(empty.status, OperationStatus::Complete);
    let empty_payload = document(&empty);
    assert!(empty_payload.raw_bytes.is_empty());
    assert!(empty_payload.decoded_text.is_empty());
    assert!(empty_payload.blocks.is_empty());
    assert_eq!(empty_payload.decoded_range.byte_start, 0);
    assert_eq!(empty_payload.decoded_range.byte_end, 0);

    let mixed = dispatch(
        b"one\r\ntwo\n\nthree\rfour".to_vec(),
        SourceInfo::stdin("mixed-newlines.txt"),
        None,
    );
    let mixed_payload = document(&mixed);
    assert_eq!(mixed_payload.decoding.newlines.crlf_count, 1);
    assert_eq!(mixed_payload.decoding.newlines.lf_count, 2);
    assert_eq!(mixed_payload.decoding.newlines.cr_count, 1);
    assert_eq!(mixed_payload.blocks.len(), 2);
    assert_eq!(mixed_payload.blocks[0].text, "one\r\ntwo");
    assert_eq!(mixed_payload.blocks[1].text, "three\rfour");

    let large_text = "0123456789abcdef\n".repeat(4096);
    let large = dispatch(
        large_text.as_bytes().to_vec(),
        SourceInfo::stdin("large.txt"),
        None,
    );
    assert_eq!(large.status, OperationStatus::Complete);
    assert_eq!(document(&large).decoded_text, large_text);

    let malformed = dispatch(
        b"text".to_vec(),
        SourceInfo::stdin("malformed.txt"),
        Some(TextOptions {
            encoding: Some("x-grist-unsupported-charset".into()),
        }),
    );
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(malformed.payload.is_none());
    assert_eq!(
        malformed.diagnostics[0].code.as_str(),
        "decode.encoding.unsupported"
    );
}

#[test]
fn graph_segmentation_schema_and_determinism_share_the_authoritative_payload() {
    let bytes = b"first paragraph\r\ncontinued\r\n\r\nsecond paragraph".to_vec();
    let first = dispatch(bytes.clone(), SourceInfo::stdin("graph.txt"), None);
    let second = dispatch(bytes, SourceInfo::stdin("graph.txt"), None);
    assert_eq!(
        canonical_json_bytes(&first).unwrap(),
        canonical_json_bytes(&second).unwrap()
    );

    let payload = document(&first);
    let graph = payload
        .to_document_graph(
            DocumentGraphContext::new("text:graph").with_source(first.source.clone()),
        )
        .unwrap();
    assert_eq!(
        graph
            .projection
            .as_ref()
            .unwrap()
            .authoritative_payload_schema_version,
        SchemaVersion::TEXT_V2
    );
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
            .filter(|node| node.kind != DocumentNodeKind::Document)
            .all(|node| node
                .locator
                .as_ref()
                .is_some_and(|locator| locator.validate().is_ok()))
    );
    graph.validate_contract().unwrap();

    let source_identity = first.identity.as_ref().unwrap();
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(!segments.segments.is_empty());
    assert!(segments.segments.iter().all(|segment| {
        segment.source_identity == *source_identity
            && !segment.node_ids.is_empty()
            && segment.node_ids.len() == segment.locators.len()
    }));

    #[cfg(feature = "schemas")]
    {
        assert!(
            grist::schema::schema_json_version("text", SchemaVersion::TEXT_V1).is_some(),
            "the archived v1 text schema remains discoverable"
        );
        for (schema, value) in [
            ("text", serde_json::to_value(&payload).unwrap()),
            ("text-envelope", serde_json::to_value(&first).unwrap()),
            (
                "text-options",
                serde_json::to_value(TextOptions::default()).unwrap(),
            ),
        ] {
            let validation = grist::schema::validate_schema(schema, &value).unwrap();
            assert!(validation.valid, "{schema}: {:?}", validation.issues);
        }
    }
}

#[test]
fn decoded_and_node_budgets_fail_with_resource_diagnostics() {
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_decoded_characters = Some(2);
    let limited = dispatch_with_budget(
        b"too long".to_vec(),
        SourceInfo::stdin("limited.txt"),
        None,
        BudgetSelection::custom(budget),
    );
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.class == grist::core::DiagnosticClass::ResourceBudgetExhaustion
    }));
}

#[cfg(feature = "cli")]
#[test]
fn cli_parse_text_emits_the_same_v2_authoritative_contract() {
    use std::fs;
    use std::process::Command;

    let path = std::env::temp_dir().join(format!("grist-text-contract-{}.txt", std::process::id()));
    fs::write(&path, b"cli\r\ntext").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "text", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["payload_schema_version"], SchemaVersion::TEXT_V2);
    assert_eq!(value["payload"]["decoded_text"], "cli\r\ntext");
    assert_eq!(
        value["payload"]["raw_bytes"],
        json!([99, 108, 105, 13, 10, 116, 101, 120, 116])
    );
    assert!(value["identity"]["canonical_payload"].is_object());
    assert_eq!(
        value["provenance"][1]["implementation"],
        "grist.decode@0.1.0"
    );
}
