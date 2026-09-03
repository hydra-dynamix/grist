//! Typed, loss-retaining audio/video container model.

use crate::container::ArtifactSafety;
use crate::core::{Diagnostic, SourceLocator};
use crate::provider::{
    NativeRepresentation, ProviderConfidence, ProviderResponse, ReconciledRepresentation,
    TranscriptionOptions,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MediaOptions {
    pub retain_embedded_bytes: bool,
    pub parse_embedded: bool,
    #[cfg_attr(feature = "schemas", schemars(range(min = 1)))]
    pub max_boxes: u64,
    #[cfg_attr(feature = "schemas", schemars(range(min = 1)))]
    pub max_nesting_depth: u64,
    #[cfg_attr(feature = "schemas", schemars(range(min = 1)))]
    pub max_metadata_bytes: u64,
    #[cfg_attr(feature = "schemas", schemars(range(min = 1)))]
    pub max_attachment_bytes: u64,
    #[cfg_attr(feature = "schemas", schemars(range(min = 1)))]
    pub max_subtitle_bytes: u64,
    pub transcription: MediaTranscriptionOptions,
}

impl Default for MediaOptions {
    fn default() -> Self {
        Self {
            retain_embedded_bytes: true,
            parse_embedded: true,
            max_boxes: 100_000,
            max_nesting_depth: 64,
            max_metadata_bytes: 16 * 1024 * 1024,
            max_attachment_bytes: 32 * 1024 * 1024,
            max_subtitle_bytes: 16 * 1024 * 1024,
            transcription: MediaTranscriptionOptions::default(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct MediaTranscriptionOptions {
    pub selection: MediaTranscriptionSelection,
    pub provider_options: TranscriptionOptions,
    #[cfg_attr(feature = "schemas", schemars(range(min = 1)))]
    pub max_streams: u64,
    pub reconcile: bool,
}

impl Default for MediaTranscriptionOptions {
    fn default() -> Self {
        Self {
            selection: MediaTranscriptionSelection::Disabled,
            provider_options: TranscriptionOptions::default(),
            max_streams: 1_000,
            reconcile: false,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum MediaTranscriptionSelection {
    Disabled,
    AllAudioStreams,
    SelectedAudioStreams { stream_ids: Vec<String> },
}

impl Default for MediaTranscriptionSelection {
    fn default() -> Self {
        Self::Disabled
    }
}

impl crate::core::FormatOptions for MediaOptions {
    const FORMAT: &'static str = "media";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaFormat {
    Mp3,
    Mp4,
    QuickTime,
    Wav,
    Flac,
    Matroska,
}

impl MediaFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Mp4 => "mp4",
            Self::QuickTime => "quicktime",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Matroska => "matroska",
        }
    }

    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Mp4 => "video/mp4",
            Self::QuickTime => "video/quicktime",
            Self::Wav => "audio/wav",
            Self::Flac => "audio/flac",
            Self::Matroska => "video/x-matroska",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaDocument {
    pub schema_version: String,
    pub format: MediaFormat,
    pub technical: MediaTechnicalMetadata,
    pub streams: Vec<MediaStream>,
    pub chapters: Vec<MediaChapter>,
    pub attachments: Vec<MediaAttachment>,
    pub subtitle_tracks: Vec<EmbeddedSubtitleTrack>,
    /// Native subtitle tracks remain authoritative; provider attempts and any
    /// reconciliation are retained here as separate additive representations.
    #[serde(default)]
    pub transcription: MediaTranscriptionContent,
    pub artwork: Vec<MediaArtwork>,
    pub metadata: Vec<MediaMetadataEntry>,
    pub encrypted: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaTranscriptionContent {
    pub native: NativeRepresentation<MediaNativeTranscript>,
    #[serde(default)]
    pub provider_attempts: Vec<MediaTranscriptionAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciled: Option<ReconciledRepresentation<MediaReconciledTranscript>>,
}

impl Default for MediaTranscriptionContent {
    fn default() -> Self {
        Self {
            native: NativeRepresentation::new(MediaNativeTranscript::default())
                .expect("empty native media transcript serializes"),
            provider_attempts: Vec::new(),
            reconciled: None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MediaNativeTranscript {
    pub text: String,
    pub items: Vec<MediaNativeTranscriptItem>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaNativeTranscriptItem {
    pub index: u64,
    pub subtitle_track_id: String,
    pub cue_id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaTranscriptionAttempt {
    pub scope: MediaTranscriptionScope,
    pub response: ProviderResponse,
    #[serde(default)]
    pub segments: Vec<MediaTranscriptSegment>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaTranscriptionScope {
    pub stream_id: String,
    pub stream_index: u64,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaTranscriptSegment {
    pub index: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MediaReconciledTranscript {
    pub text: String,
    pub items: Vec<MediaReconciledTranscriptItem>,
    #[serde(default)]
    pub confidence_evidence: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaReconciledTranscriptItem {
    pub index: u64,
    pub text: String,
    pub origin: MediaTranscriptOrigin,
    pub source: MediaReconciledTranscriptSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaTranscriptOrigin {
    NativeSubtitle,
    ProviderTranscription,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum MediaReconciledTranscriptSource {
    NativeSubtitle {
        subtitle_track_id: String,
        cue_id: String,
    },
    ProviderSegment {
        attempt_index: u64,
        segment_index: u64,
    },
    ProviderOverall {
        attempt_index: u64,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MediaTechnicalMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_rate: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_scale: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_profile: Option<String>,
    pub byte_length: u64,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaStreamKind {
    Audio,
    Video,
    Subtitle,
    Data,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodecInspectionStatus {
    MetadataOnly,
    Unsupported,
    Encrypted,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaStream {
    /// Stable identity derived from a native track UID/ID, never source order alone when one exists.
    pub id: String,
    pub index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_track_number: Option<u64>,
    pub kind: MediaStreamKind,
    pub codec: String,
    pub inspection: CodecInspectionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_rate: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    pub encrypted: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaChapter {
    pub id: String,
    pub index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_id: Option<String>,
    pub start_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaAttachment {
    pub id: String,
    pub index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub byte_length: u64,
    pub sha256: String,
    pub safety: ArtifactSafety,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed_subtitle: Option<crate::subtitle::SubtitleDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed_image: Option<crate::image::ImageDocument>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddedSubtitleTrack {
    pub id: String,
    pub stream_id: String,
    pub codec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub byte_length: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<crate::subtitle::SubtitleDocument>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaArtwork {
    pub id: String,
    pub index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub byte_length: u64,
    pub sha256: String,
    pub safety: ArtifactSafety,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<crate::image::ImageDocument>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaMetadataEntry {
    pub key: String,
    pub value: String,
    pub source: String,
    pub locator: SourceLocator,
}

#[derive(Debug, Default)]
pub(crate) struct ParsedMedia {
    pub technical: MediaTechnicalMetadata,
    pub streams: Vec<MediaStream>,
    pub chapters: Vec<MediaChapter>,
    pub attachment_candidates: Vec<EmbeddedCandidate>,
    pub subtitle_candidates: Vec<SubtitleCandidate>,
    pub artwork_candidates: Vec<ArtworkCandidate>,
    pub metadata: Vec<MediaMetadataEntry>,
    pub encrypted: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
}

#[derive(Debug)]
pub(crate) struct EmbeddedCandidate {
    pub native_id: Option<String>,
    pub filename: Option<String>,
    pub media_type: Option<String>,
    pub bytes: Vec<u8>,
    pub locator: SourceLocator,
    pub parsed_subtitle: Option<crate::subtitle::SubtitleDocument>,
    pub parsed_image: Option<crate::image::ImageDocument>,
    pub inventory: Option<(u64, String)>,
}

#[derive(Debug)]
pub(crate) struct SubtitleCandidate {
    pub stream_id: String,
    pub codec: String,
    pub language: Option<String>,
    pub bytes: Vec<u8>,
    pub locator: SourceLocator,
    pub document: Option<crate::subtitle::SubtitleDocument>,
    pub inventory: Option<(u64, String)>,
}

#[derive(Debug)]
pub(crate) struct ArtworkCandidate {
    pub picture_type: Option<String>,
    pub description: Option<String>,
    pub media_type: Option<String>,
    pub bytes: Vec<u8>,
    pub locator: SourceLocator,
    pub image: Option<crate::image::ImageDocument>,
    pub inventory: Option<(u64, String)>,
}
