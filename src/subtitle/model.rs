use crate::core::{Diagnostic, SourceLocator, SourceRange};
use crate::decode::{DecodeReport, RawByteRange, TextEncoding};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SubtitleOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    pub max_cues: usize,
    pub max_styles: usize,
    pub max_tracks: usize,
    pub max_regions: usize,
    pub max_nesting_depth: usize,
}

impl Default for SubtitleOptions {
    fn default() -> Self {
        Self {
            encoding: None,
            max_cues: 100_000,
            max_styles: 10_000,
            max_tracks: 1_000,
            max_regions: 10_000,
            max_nesting_depth: 256,
        }
    }
}

impl crate::core::FormatOptions for SubtitleOptions {
    const FORMAT: &'static str = "subtitle";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleFormat {
    Srt,
    WebVtt,
    Ttml,
}

impl SubtitleFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::WebVtt => "webvtt",
            Self::Ttml => "ttml",
        }
    }

    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Srt => "application/x-subrip",
            Self::WebVtt => "text/vtt",
            Self::Ttml => "application/ttml+xml",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleDocument {
    pub schema_version: String,
    pub format: SubtitleFormat,
    pub raw_bytes: Vec<u8>,
    pub raw_range: RawByteRange,
    pub decoded_text: String,
    pub decoded_range: SourceRange,
    pub locator: SourceLocator,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub tracks: Vec<SubtitleTrack>,
    pub regions: Vec<SubtitleRegion>,
    pub styles: Vec<SubtitleStyle>,
    pub cues: Vec<SubtitleCue>,
    pub transcript: TranscriptProjection,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleTrack {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleRegion {
    pub id: String,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleStyle {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
    pub raw: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleCue {
    pub id: String,
    pub source_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub track_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_id: Option<String>,
    pub timing: SubtitleTiming,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    pub raw_text: String,
    pub runs: Vec<SubtitleTextRun>,
    pub locator: SourceLocator,
    pub text_locator: SourceLocator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure_locator: Option<SourceLocator>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleTiming {
    pub raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleTextRun {
    pub kind: SubtitleTextRunKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleTextRunKind {
    Text,
    Voice,
    Span,
    Ruby,
    Timestamp,
    RawMarkup,
    LineBreak,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TranscriptProjection {
    pub text: String,
    pub entries: Vec<TranscriptEntry>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptEntry {
    pub cue_id: String,
    pub source_index: usize,
    pub track_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    pub text_locator: SourceLocator,
    pub time_locator: SourceLocator,
}
