//! Public text-decoding contracts and byte-coordinate diagnostics.

use crate::core::{DecodedContentIdentity, Diagnostic, DiagnosticClass, RawContentIdentity};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// The syntax whose encoding declaration rules apply to an input.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecodeContext {
    #[default]
    Auto,
    PlainText,
    Html,
    Xml,
}

/// Caller-controlled decoding evidence. A BOM always has higher precedence.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecodeOptions {
    pub context: DecodeContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_encoding: Option<String>,
}

impl DecodeOptions {
    pub fn for_media_type(media_type: Option<&str>, format: Option<&str>) -> Self {
        let context = format
            .map(str::to_ascii_lowercase)
            .as_deref()
            .map(context_for_label)
            .unwrap_or_else(|| {
                media_type
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                    .map(context_for_label)
                    .unwrap_or_default()
            });
        Self {
            context,
            transport_encoding: media_type.and_then(media_type_charset),
        }
    }

    pub fn with_transport_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.transport_encoding = Some(encoding.into());
        self
    }
}

fn context_for_label(label: &str) -> DecodeContext {
    let essence = label.split(';').next().unwrap_or(label).trim();
    if matches!(
        essence,
        "html" | "xhtml" | "text/html" | "application/xhtml+xml"
    ) {
        DecodeContext::Html
    } else if essence == "xml"
        || essence.ends_with("+xml")
        || matches!(essence, "application/xml" | "text/xml")
    {
        DecodeContext::Xml
    } else {
        DecodeContext::PlainText
    }
}

fn media_type_charset(media_type: &str) -> Option<String> {
    media_type.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches(['\'', '"']).to_ascii_lowercase())
    })
}

/// Canonical encoding selected by the decoder.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "label", rename_all = "snake_case")]
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
    Windows1252,
    Other(String),
}

impl TextEncoding {
    pub fn label(&self) -> &str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16Le => "utf-16le",
            Self::Utf16Be => "utf-16be",
            Self::Utf32Le => "utf-32le",
            Self::Utf32Be => "utf-32be",
            Self::Windows1252 => "windows-1252",
            Self::Other(label) => label,
        }
    }

    pub(crate) fn same_family(&self, other: &Self) -> bool {
        self == other
            || matches!(
                (self, other),
                (Self::Utf16Le | Self::Utf16Be, Self::Utf16Le | Self::Utf16Be)
                    | (Self::Utf32Le | Self::Utf32Be, Self::Utf32Le | Self::Utf32Be)
            )
    }
}

/// A zero-based half-open range in the exact original input bytes.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawByteRange {
    pub start: u64,
    pub end: u64,
}

impl RawByteRange {
    pub(crate) fn from_usize(start: usize, end: usize) -> Self {
        Self {
            start: u64::try_from(start).unwrap_or(u64::MAX),
            end: u64::try_from(end).unwrap_or(u64::MAX),
        }
    }
}

/// A zero-based half-open UTF-8 byte range in the decoded text.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecodedByteRange {
    pub start: u64,
    pub end: u64,
}

impl DecodedByteRange {
    pub(crate) fn from_usize(start: usize, end: usize) -> Self {
        Self {
            start: u64::try_from(start).unwrap_or(u64::MAX),
            end: u64::try_from(end).unwrap_or(u64::MAX),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BomKind {
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncodingDeclarationSource {
    ByteOrderMark,
    Transport,
    HtmlMeta,
    XmlDeclaration,
    XmlSignature,
    Heuristic,
    Default,
}

/// One retained piece of encoding evidence, whether selected or contradicted.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncodingDeclaration {
    pub source: EncodingDeclarationSource,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<TextEncoding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_range: Option<RawByteRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_range: Option<DecodedByteRange>,
    pub selected: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecodeIssueKind {
    UndecodableSequence,
    TruncatedCodeUnit,
    EncodingConflict,
    UnsupportedDeclaration,
}

/// A precise loss or conflict range in both available coordinate systems.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecodeIssue {
    pub kind: DecodeIssueKind,
    pub raw_range: RawByteRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_range: Option<DecodedByteRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicting_raw_range: Option<RawByteRange>,
    pub message: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NewlineKind {
    Lf,
    CrLf,
    Cr,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewlineSequence {
    pub kind: NewlineKind,
    pub raw_range: RawByteRange,
    pub decoded_range: DecodedByteRange,
}

/// Ordered newline evidence. The decoded text itself is never normalized.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewlineFidelity {
    pub sequences: Vec<NewlineSequence>,
    pub lf_count: u64,
    pub crlf_count: u64,
    pub cr_count: u64,
    pub final_line_terminated: bool,
}

/// Serializable decoding evidence. Raw bytes remain on [`DecodedText`].
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecodeReport {
    pub schema_version: String,
    pub encoding: TextEncoding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bom: Option<BomKind>,
    pub raw_identity: RawContentIdentity,
    pub decoded_identity: DecodedContentIdentity,
    pub declarations: Vec<EncodingDeclaration>,
    pub issues: Vec<DecodeIssue>,
    pub newlines: NewlineFidelity,
    pub diagnostics: Vec<Diagnostic>,
}

impl DecodeReport {
    pub fn is_lossy(&self) -> bool {
        self.decoded_identity.lossy
    }

    pub fn makes_operation_partial(&self) -> bool {
        self.diagnostics.iter().any(|diagnostic| diagnostic.partial)
    }
}

/// Decoded text paired with the exact original bytes and full evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedText {
    raw_bytes: Vec<u8>,
    mapping: Vec<super::codecs::MapSegment>,
    pub text: String,
    pub report: DecodeReport,
}

impl DecodedText {
    pub(crate) fn new(
        raw_bytes: Vec<u8>,
        mapping: Vec<super::codecs::MapSegment>,
        text: String,
        report: DecodeReport,
    ) -> Self {
        Self {
            raw_bytes,
            mapping,
            text,
            report,
        }
    }

    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw_bytes
    }

    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.raw_bytes
    }

    /// Map a half-open UTF-8 range in the decoded text back to the exact
    /// original byte span that produced it.
    pub fn raw_range_for_decoded(&self, range: DecodedByteRange) -> Option<RawByteRange> {
        let start = usize::try_from(range.start).ok()?;
        let end = usize::try_from(range.end).ok()?;
        if start > end
            || end > self.text.len()
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
        {
            return None;
        }
        if start == end {
            let raw_offset = self
                .mapping
                .iter()
                .find(|segment| segment.decoded_start >= start)
                .map(|segment| segment.raw_start)
                .or_else(|| self.mapping.last().map(|segment| segment.raw_end))
                .unwrap_or(self.raw_bytes.len());
            return Some(RawByteRange::from_usize(raw_offset, raw_offset));
        }
        let first = self
            .mapping
            .iter()
            .find(|segment| segment.decoded_end > start)?;
        let last = self
            .mapping
            .iter()
            .rev()
            .find(|segment| segment.decoded_start < end)?;
        Some(RawByteRange::from_usize(first.raw_start, last.raw_end))
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("declared encoding {label:?} is not supported by this build")]
    UnsupportedEncoding {
        label: String,
        raw_range: Option<RawByteRange>,
    },
}

impl DecodeError {
    pub fn diagnostic(&self) -> Diagnostic {
        match self {
            Self::UnsupportedEncoding { label, .. } => {
                let mut diagnostic = Diagnostic::unsupported(
                    "grist.decode",
                    format!("declared encoding {label:?} is not supported by this build"),
                );
                diagnostic.class = DiagnosticClass::UnsupportedContent;
                diagnostic.code = "decode.encoding.unsupported".into();
                diagnostic.with_module("grist.decode")
            }
        }
    }
}
