use crate::core::{
    ArtifactKind, ContentIdentity, DeclaredLoss, Envelope, FormatIdentity, LineIndex, LossClass,
    OperationKind, OperationStatus, ParserInfo, ProvenanceStep, SchemaVersion, SourceInfo,
    SourceLocator, SourceRange, options_digest,
};
use crate::decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeReport, DecodedByteRange, DecodedText,
    RawByteRange, TextEncoding, decode_text,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Plain-text-specific decoding controls.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TextOptions {
    /// Optional WHATWG/IANA charset label. BOM evidence still takes precedence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

impl crate::core::FormatOptions for TextOptions {
    const FORMAT: &'static str = "text";
}

/// Authoritative plain-text payload. The graph and segments are projections of
/// this structure; neither replaces the original bytes or decoding evidence.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextDocument {
    pub schema_version: String,
    /// Exact input bytes, including any byte-order mark and malformed units.
    pub raw_bytes: Vec<u8>,
    pub raw_range: RawByteRange,
    /// Decoded text exactly as produced; line endings are never normalized.
    pub decoded_text: String,
    pub decoded_range: SourceRange,
    pub locator: SourceLocator,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub blocks: Vec<TextBlock>,
}

/// One paragraph-like run separated by one or more blank physical lines.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextBlock {
    pub id: String,
    /// Exact decoded slice for this block, including internal line endings.
    pub text: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
    /// Exact original bytes that produced the block text.
    pub raw_range: RawByteRange,
}

pub type TextEnvelope = Envelope<TextDocument>;

/// Compatibility entry point for a caller-supplied Rust UTF-8 string.
pub fn parse_text(text: &str, source: SourceInfo) -> TextEnvelope {
    let decoded = crate::decode::decode_declared_utf8(text);
    envelope_from_decoded(&decoded, source, &TextOptions::default())
}

/// Primary byte-facing entry point for plain text.
pub fn parse_text_bytes(bytes: &[u8], source: SourceInfo, options: &TextOptions) -> TextEnvelope {
    let mut decode_options =
        DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("text"));
    decode_options.context = DecodeContext::PlainText;
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    match decode_text(bytes, &decode_options) {
        Ok(decoded) => envelope_from_decoded(&decoded, source, options),
        Err(error) => failed_decode_envelope(bytes, source, options, error),
    }
}

pub(crate) fn document_from_decoded(decoded: &DecodedText) -> TextDocument {
    let index = LineIndex::new(&decoded.text);
    let decoded_range = SourceRange::new(0, decoded.text.len(), &index);
    let locator = SourceLocator::exact(decoded_range.clone())
        .expect("a whole decoded text range is always exact");
    let blocks = block_ranges(&decoded.text)
        .into_iter()
        .enumerate()
        .map(|(ordinal, (start, end))| {
            let range = SourceRange::new(start, end, &index);
            let locator =
                SourceLocator::exact(range.clone()).expect("block ranges are UTF-8 boundaries");
            let raw_range = decoded
                .raw_range_for_decoded(DecodedByteRange {
                    start: start as u64,
                    end: end as u64,
                })
                .expect("every non-empty decoded block has raw mapping");
            TextBlock {
                id: format!("text-block-{ordinal:06}"),
                text: decoded.text[start..end].to_string(),
                range,
                locator,
                raw_range,
            }
        })
        .collect();
    TextDocument {
        schema_version: SchemaVersion::TEXT_V2.to_string(),
        raw_bytes: decoded.raw_bytes().to_vec(),
        raw_range: RawByteRange {
            start: 0,
            end: decoded.raw_bytes().len() as u64,
        },
        decoded_text: decoded.text.clone(),
        decoded_range,
        locator,
        encoding: decoded.report.encoding.clone(),
        decoding: decoded.report.clone(),
        blocks,
    }
}

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new("grist.text")
        .with_implementation("grist-native-text", env!("CARGO_PKG_VERSION"))
        .with_specification_version("Unicode + WHATWG Encoding Standard")
}

pub(crate) fn decoding_provenance(report: &DecodeReport) -> ProvenanceStep {
    let loss = if report.is_lossy() {
        DeclaredLoss::Lossy(LossClass::from(LossClass::REPAIR_APPLIED))
    } else {
        DeclaredLoss::Lossless
    };
    let mut step = ProvenanceStep::new(
        OperationKind::Parse,
        format!("grist.decode@{}", env!("CARGO_PKG_VERSION")),
        report.raw_identity.sha256.clone(),
        report.decoded_identity.sha256.clone(),
        options_digest(&report.declarations).expect("decode declarations always serialize"),
        loss,
    )
    .expect("decoder identities and options digest satisfy provenance invariants");
    for diagnostic in &report.diagnostics {
        step = step.with_warning(diagnostic.code.as_str());
    }
    step
}

fn envelope_from_decoded(
    decoded: &DecodedText,
    source: SourceInfo,
    options: &TextOptions,
) -> TextEnvelope {
    let payload = document_from_decoded(decoded);
    let digest = options_digest(options).expect("text options always serialize");
    let diagnostics = decoded.report.diagnostics.clone();
    let mut envelope = if decoded.report.makes_operation_partial() {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Text,
            source,
            parser_info(),
            digest,
            SchemaVersion::TEXT_V2,
            Some(payload),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Text,
            source,
            parser_info(),
            digest,
            SchemaVersion::TEXT_V2,
            payload,
        )
    };
    envelope.diagnostics = diagnostics;
    envelope
        .provenance
        .push(decoding_provenance(&decoded.report));
    let identity = ContentIdentity::for_raw_bytes(decoded.raw_bytes())
        .with_decoded(
            &decoded.text,
            decoded.report.encoding.label(),
            decoded.report.is_lossy(),
        )
        .with_format(FormatIdentity::new("text", Some("text/plain")));
    envelope
        .with_identity(identity)
        .with_canonical_payload_identity()
        .expect("text payload canonicalization is infallible")
}

fn failed_decode_envelope(
    bytes: &[u8],
    source: SourceInfo,
    options: &TextOptions,
    error: DecodeError,
) -> TextEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Text,
        OperationStatus::Failed,
        source,
        parser_info(),
        options_digest(options).expect("text options always serialize"),
        SchemaVersion::TEXT_V2,
    )
    .expect("failed text decode has valid envelope status")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes)
            .with_format(FormatIdentity::new("text", Some("text/plain"))),
    )
    .with_diagnostics(vec![error.diagnostic().with_parser("grist.text")])
}

fn block_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut block_start = None;
    let mut block_end = 0;
    for (start, content_end) in physical_lines(text) {
        if text[start..content_end].trim().is_empty() {
            if let Some(start) = block_start.take() {
                blocks.push((start, block_end));
            }
        } else {
            block_start.get_or_insert(start);
            block_end = content_end;
        }
    }
    if let Some(start) = block_start {
        blocks.push((start, block_end));
    }
    blocks
}

fn physical_lines(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    while offset < bytes.len() {
        match bytes[offset] {
            b'\r' => {
                lines.push((start, offset));
                offset += 1;
                if bytes.get(offset) == Some(&b'\n') {
                    offset += 1;
                }
                start = offset;
            }
            b'\n' => {
                lines.push((start, offset));
                offset += 1;
                start = offset;
            }
            _ => offset += 1,
        }
    }
    if start < bytes.len() {
        lines.push((start, bytes.len()));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_all_newline_styles_without_normalizing_text() {
        let source = "one\r\ncontinued\r\rthree\n\nfour";
        let report = parse_text(source, SourceInfo::stdin("note.txt"));
        let payload = report.payload.expect("complete operation payload");
        assert_eq!(payload.decoded_text, source);
        assert_eq!(
            payload
                .blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>(),
            ["one\r\ncontinued", "three", "four"]
        );
        assert_eq!(payload.decoding.newlines.crlf_count, 1);
        assert_eq!(payload.decoding.newlines.cr_count, 2);
        assert_eq!(payload.decoding.newlines.lf_count, 2);
    }
}
