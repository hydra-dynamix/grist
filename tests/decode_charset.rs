use grist::core::{
    BudgetProfile, BudgetSelection, Input, OperationStatus, ParseRequest, ProviderSet, RequestId,
    SourceInfo, sha256_hex,
};
use grist::decode::{
    BomKind, DecodeContext, DecodeIssueKind, DecodeOptions, NewlineKind, RawByteRange,
    TextEncoding, decode_text,
};
use grist::registry::builtin_parser_registry;

fn options(context: DecodeContext, encoding: Option<&str>) -> DecodeOptions {
    DecodeOptions {
        context,
        transport_encoding: encoding.map(str::to_string),
    }
}

fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
    let mut bytes = if big_endian {
        vec![0xfe, 0xff]
    } else {
        vec![0xff, 0xfe]
    };
    for unit in text.encode_utf16() {
        let encoded = if big_endian {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        };
        bytes.extend_from_slice(&encoded);
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
        let encoded = if big_endian {
            u32::from(character).to_be_bytes()
        } else {
            u32::from(character).to_le_bytes()
        };
        bytes.extend_from_slice(&encoded);
    }
    bytes
}

#[test]
fn utf_bom_variants_decode_without_retaining_the_signature_as_text() {
    let fixtures = [
        (
            [vec![0xef, 0xbb, 0xbf], "alpha".as_bytes().to_vec()].concat(),
            TextEncoding::Utf8,
            BomKind::Utf8,
        ),
        (
            utf16("alpha", false),
            TextEncoding::Utf16Le,
            BomKind::Utf16Le,
        ),
        (
            utf16("alpha", true),
            TextEncoding::Utf16Be,
            BomKind::Utf16Be,
        ),
        (
            utf32("alpha", false),
            TextEncoding::Utf32Le,
            BomKind::Utf32Le,
        ),
        (
            utf32("alpha", true),
            TextEncoding::Utf32Be,
            BomKind::Utf32Be,
        ),
    ];
    for (bytes, encoding, bom) in fixtures {
        let decoded = decode_text(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(decoded.text, "alpha");
        assert_eq!(decoded.report.encoding, encoding);
        assert_eq!(decoded.report.bom, Some(bom));
        assert_eq!(decoded.raw_bytes(), bytes);
        assert!(!decoded.report.is_lossy());
    }
}

#[test]
fn windows_1252_and_undefined_bytes_have_exact_loss_ranges() {
    let bytes = [b'A', 0x93, b'B', 0x81, b'C'];
    let decoded = decode_text(
        &bytes,
        &options(DecodeContext::PlainText, Some("windows-1252")),
    )
    .unwrap();

    assert_eq!(decoded.text, "A\u{201c}B\u{fffd}C");
    assert_eq!(decoded.report.encoding, TextEncoding::Windows1252);
    assert_eq!(decoded.report.issues.len(), 1);
    assert_eq!(
        decoded.report.issues[0].raw_range,
        RawByteRange { start: 3, end: 4 }
    );
    assert_eq!(
        decoded.report.diagnostics[0].code.as_str(),
        "decode.replacement.undecodable"
    );
    assert!(decoded.report.diagnostics[0].locator.is_some());
}

#[test]
fn mixed_utf8_and_invalid_bytes_preserve_valid_scalars_and_locate_replacement() {
    let bytes = b"snowman \xe2\x98\x83 then \x93";
    let invalid = bytes.iter().position(|byte| *byte == 0x93).unwrap();
    let decoded = decode_text(bytes, &DecodeOptions::default()).unwrap();

    assert_eq!(decoded.report.encoding, TextEncoding::Utf8);
    assert_eq!(decoded.text, "snowman \u{2603} then \u{fffd}");
    assert_eq!(
        decoded.report.issues[0].raw_range,
        RawByteRange {
            start: invalid as u64,
            end: invalid as u64 + 1,
        }
    );
    assert_eq!(
        decoded.report.issues[0].kind,
        DecodeIssueKind::UndecodableSequence
    );
}

#[test]
fn html_and_xml_declarations_select_the_declared_decoder() {
    let html = b"<meta charset='windows-1252'><p>caf\xe9</p>";
    let html = decode_text(html, &options(DecodeContext::Html, None)).unwrap();
    assert_eq!(html.report.encoding, TextEncoding::Windows1252);
    assert!(html.text.ends_with("<p>caf\u{e9}</p>"));
    assert!(html.report.declarations.iter().any(|declaration| {
        declaration.selected && declaration.raw_range == Some(RawByteRange { start: 15, end: 27 })
    }));

    let xml = b"<?xml version='1.0' encoding='windows-1252'?><x>\x80</x>";
    let xml = decode_text(xml, &options(DecodeContext::Xml, None)).unwrap();
    assert_eq!(xml.report.encoding, TextEncoding::Windows1252);
    assert!(xml.text.ends_with("<x>\u{20ac}</x>"));
}

#[test]
fn bom_conflicts_are_diagnostic_and_both_ranges_are_retained() {
    let body = b"<meta charset=windows-1252><p>ok</p>";
    let bytes = [vec![0xef, 0xbb, 0xbf], body.to_vec()].concat();
    let label_start = bytes
        .windows("windows-1252".len())
        .position(|window| window == b"windows-1252")
        .unwrap();
    let decoded = decode_text(&bytes, &options(DecodeContext::Html, None)).unwrap();

    assert_eq!(decoded.report.encoding, TextEncoding::Utf8);
    let conflict = decoded
        .report
        .issues
        .iter()
        .find(|issue| issue.kind == DecodeIssueKind::EncodingConflict)
        .unwrap();
    assert_eq!(
        conflict.raw_range,
        RawByteRange {
            start: label_start as u64,
            end: (label_start + "windows-1252".len()) as u64,
        }
    );
    assert_eq!(
        conflict.conflicting_raw_range,
        Some(RawByteRange { start: 0, end: 3 })
    );
    assert_eq!(
        decoded.report.diagnostics[0].code.as_str(),
        "decode.encoding.conflict"
    );
    assert!(decoded.report.makes_operation_partial());
}

#[test]
fn newline_order_and_raw_ranges_are_preserved_across_utf16() {
    let bytes = utf16("a\r\nb\nc\rd", false);
    let decoded = decode_text(&bytes, &DecodeOptions::default()).unwrap();
    let kinds = decoded
        .report
        .newlines
        .sequences
        .iter()
        .map(|sequence| sequence.kind)
        .collect::<Vec<_>>();

    assert_eq!(decoded.text, "a\r\nb\nc\rd");
    assert_eq!(kinds, [NewlineKind::CrLf, NewlineKind::Lf, NewlineKind::Cr]);
    assert_eq!(
        decoded.report.newlines.sequences[0].raw_range,
        RawByteRange { start: 4, end: 8 }
    );
    assert!(!decoded.report.newlines.final_line_terminated);
}

#[test]
fn raw_and_decoded_hashes_are_distinct_and_recomputable() {
    let bytes = utf16("hash me", false);
    let decoded = decode_text(&bytes, &DecodeOptions::default()).unwrap();

    assert_eq!(decoded.report.raw_identity.sha256, sha256_hex(&bytes));
    assert_eq!(
        decoded.report.decoded_identity.sha256,
        sha256_hex(decoded.text.as_bytes())
    );
    assert_ne!(
        decoded.report.raw_identity.sha256,
        decoded.report.decoded_identity.sha256
    );
}

#[test]
fn registry_parsers_receive_decoded_text_and_publish_decoded_identity() {
    let source =
        SourceInfo::stdin("note.txt").with_declared_mime_type("text/plain; charset=windows-1252");
    let request = ParseRequest::new(
        RequestId::new("decode-registry").unwrap(),
        Input::bytes(b"caf\xe9"),
        source,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );
    let envelope = builtin_parser_registry()
        .unwrap()
        .dispatch("text", request, None)
        .unwrap();

    assert_eq!(envelope.status, OperationStatus::Complete);
    assert_eq!(envelope.payload.unwrap()["blocks"][0]["text"], "caf\u{e9}");
    let identity = envelope.identity.unwrap();
    assert_eq!(identity.decoded.as_ref().unwrap().encoding, "windows-1252");
    assert_ne!(
        identity.raw.unwrap().sha256,
        identity.decoded.unwrap().sha256
    );

    let source =
        SourceInfo::stdin("lossy.txt").with_declared_mime_type("text/plain; charset=windows-1252");
    let request = ParseRequest::new(
        RequestId::new("decode-registry-loss").unwrap(),
        Input::bytes([b'A', 0x81, b'B']),
        source,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );
    let envelope = builtin_parser_registry()
        .unwrap()
        .dispatch("text", request, None)
        .unwrap();
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert_eq!(
        envelope.diagnostics[0].code.as_str(),
        "decode.replacement.undecodable"
    );
    assert!(envelope.identity.unwrap().decoded.unwrap().lossy);
}

#[cfg(feature = "extended-encodings")]
#[test]
fn optional_whatwg_encodings_are_selected_by_declared_labels() {
    let html = b"<meta charset=shift_jis><p>\x82\xb1\x82\xf1\x82\xc9\x82\xbf\x82\xcd</p>";
    let decoded = decode_text(html, &options(DecodeContext::Html, None)).unwrap();

    assert_eq!(decoded.report.encoding.label(), "shift_jis");
    assert!(
        decoded
            .text
            .ends_with("<p>\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}</p>")
    );
    assert!(!decoded.report.is_lossy());

    let malformed = b"<meta charset=shift_jis><p>\x82 </p>";
    let invalid = malformed.iter().position(|byte| *byte == 0x82).unwrap();
    let decoded = decode_text(malformed, &options(DecodeContext::Html, None)).unwrap();
    assert_eq!(
        decoded.report.issues[0].raw_range,
        RawByteRange {
            start: invalid as u64,
            end: invalid as u64 + 1,
        }
    );
}

#[cfg(not(feature = "extended-encodings"))]
#[test]
fn disabled_optional_encoding_returns_an_explicit_unsupported_error() {
    let error = decode_text(
        b"<meta charset=shift_jis>",
        &options(DecodeContext::Html, None),
    )
    .unwrap_err();
    assert_eq!(
        error.diagnostic().code.as_str(),
        "decode.encoding.unsupported"
    );
}
