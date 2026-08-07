//! Standards-based text decoding with exact byte loss and conflict evidence.

mod codecs;
mod declarations;
mod model;
mod newlines;

pub use model::{
    BomKind, DecodeContext, DecodeError, DecodeIssue, DecodeIssueKind, DecodeOptions, DecodeReport,
    DecodedByteRange, DecodedText, EncodingDeclaration, EncodingDeclarationSource, NewlineFidelity,
    NewlineKind, NewlineSequence, RawByteRange, TextEncoding,
};

use crate::core::{
    DecodedContentIdentity, Diagnostic, DiagnosticClass, DiagnosticDetails, LineIndex,
    RawContentIdentity, SourceLocator, SourceRange,
};
use codecs::MappedText;
use serde_json::json;

/// Decode original bytes while retaining them and every declaration, loss, and newline range.
pub fn decode_text(bytes: &[u8], options: &DecodeOptions) -> Result<DecodedText, DecodeError> {
    let selection = declarations::select(bytes, options)?;
    let mut decoded = codecs::decode(
        &bytes[selection.content_start..],
        &selection.encoding,
        selection.content_start,
    );
    decoded.issues.extend(selection.conflict_issues);
    finish(
        bytes,
        decoded,
        selection.encoding,
        selection.bom,
        selection.declarations,
    )
}

/// Preserve a caller-supplied Rust string exactly, including an initial U+FEFF.
pub(crate) fn decode_declared_utf8(text: &str) -> DecodedText {
    let bytes = text.as_bytes();
    let decoded = codecs::decode(bytes, &TextEncoding::Utf8, 0);
    finish(
        bytes,
        decoded,
        TextEncoding::Utf8,
        None,
        vec![EncodingDeclaration {
            source: EncodingDeclarationSource::Transport,
            label: "utf-8".into(),
            encoding: Some(TextEncoding::Utf8),
            raw_range: None,
            decoded_range: None,
            selected: true,
        }],
    )
    .expect("a Rust string always decodes as UTF-8")
}

fn finish(
    bytes: &[u8],
    decoded: MappedText,
    encoding: TextEncoding,
    bom: Option<BomKind>,
    declarations: Vec<EncodingDeclaration>,
) -> Result<DecodedText, DecodeError> {
    let lossy = decoded.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            DecodeIssueKind::UndecodableSequence | DecodeIssueKind::TruncatedCodeUnit
        )
    });
    let raw_identity = RawContentIdentity::new(bytes);
    let decoded_identity = DecodedContentIdentity::new(&decoded.text, encoding.label(), lossy);
    let diagnostics = decoded
        .issues
        .iter()
        .map(|issue| issue_diagnostic(issue, &decoded.text, &encoding))
        .collect();
    let newlines = newlines::inventory(&decoded, bytes, &encoding);
    let report = DecodeReport {
        schema_version: crate::core::SchemaVersion::TEXT_DECODE_V1.into(),
        encoding,
        bom,
        raw_identity,
        decoded_identity,
        declarations,
        issues: decoded.issues,
        newlines,
        diagnostics,
    };
    Ok(DecodedText::new(
        bytes.to_vec(),
        decoded.map,
        decoded.text,
        report,
    ))
}

fn issue_diagnostic(issue: &DecodeIssue, text: &str, encoding: &TextEncoding) -> Diagnostic {
    let (code, class) = match issue.kind {
        DecodeIssueKind::UndecodableSequence => (
            "decode.replacement.undecodable",
            DiagnosticClass::LossyNormalization,
        ),
        DecodeIssueKind::TruncatedCodeUnit => (
            "decode.replacement.truncated_code_unit",
            DiagnosticClass::LossyNormalization,
        ),
        DecodeIssueKind::EncodingConflict => {
            ("decode.encoding.conflict", DiagnosticClass::MalformedInput)
        }
        DecodeIssueKind::UnsupportedDeclaration => (
            "decode.encoding.declaration_ignored",
            DiagnosticClass::UnsupportedContent,
        ),
    };
    let mut diagnostic = Diagnostic::warning("grist.decode", code, &issue.message)
        .with_module("grist.decode")
        .with_explanation_key(format!("diagnostic.{code}"))
        .partial();
    diagnostic.class = class;
    let details = json!({
        "encoding": encoding.label(),
        "raw_byte_range": issue.raw_range,
        "decoded_byte_range": issue.decoded_range,
        "conflicting_raw_byte_range": issue.conflicting_raw_range,
    });
    diagnostic = diagnostic.with_details(
        DiagnosticDetails::from_value(details).expect("decode detail names are secret-safe"),
    );
    if let Some(range) = issue
        .decoded_range
        .and_then(|range| decoded_source_range(text, range))
    {
        diagnostic = diagnostic.with_range(range.clone());
        if let Ok(locator) = SourceLocator::exact(range) {
            diagnostic = diagnostic.with_locator(locator);
        }
    }
    diagnostic
}

fn decoded_source_range(text: &str, range: DecodedByteRange) -> Option<SourceRange> {
    let start = usize::try_from(range.start).ok()?.min(text.len());
    let end = usize::try_from(range.end).ok()?.min(text.len());
    if end < start || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return None;
    }
    Some(SourceRange::new(start, end, &LineIndex::new(text)))
}
