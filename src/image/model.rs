//! Typed, loss-retaining image and camera-metadata model.

use crate::core::SourceLocator;
use crate::provider::{
    NativeRepresentation, ProviderConfidence, ProviderResponse, ReconciledRepresentation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ImageOptions {
    pub retain_metadata_bytes: bool,
    pub retain_unknown_chunk_bytes: bool,
    pub max_frames: u64,
    pub max_dimension: u32,
    pub max_metadata_bytes: u64,
    pub max_unknown_chunk_bytes: u64,
    pub max_chunks: u64,
    pub max_svg_elements: u64,
    pub max_svg_depth: u64,
    pub max_svg_path_bytes: u64,
    pub ocr: ImageOcrOptions,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            retain_metadata_bytes: true,
            retain_unknown_chunk_bytes: true,
            max_frames: 10_000,
            max_dimension: 1_000_000,
            max_metadata_bytes: 16 * 1024 * 1024,
            max_unknown_chunk_bytes: 16 * 1024 * 1024,
            max_chunks: 100_000,
            max_svg_elements: 1_000_000,
            max_svg_depth: 256,
            max_svg_path_bytes: 64 * 1024,
            ocr: ImageOcrOptions::default(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ImageOcrOptions {
    pub mode: ImageOcrMode,
    pub language_hints: Vec<String>,
    pub recognize_tables: bool,
    pub max_scopes: u64,
    pub reconcile: bool,
}

impl Default for ImageOcrOptions {
    fn default() -> Self {
        Self {
            mode: ImageOcrMode::AllFrames,
            language_hints: Vec::new(),
            recognize_tables: false,
            max_scopes: 10_000,
            reconcile: true,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageOcrMode {
    Disabled,
    #[default]
    AllFrames,
}

impl crate::core::FormatOptions for ImageOptions {
    const FORMAT: &'static str = "image";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageDocument {
    pub schema_version: String,
    pub format: ImageFormat,
    pub dimensions: ImageDimensions,
    pub frames: Vec<ImageFrame>,
    pub color: ImageColorInfo,
    pub orientation: ImageOrientation,
    pub metadata: ImageMetadata,
    pub embedded_text: Vec<ImageText>,
    /// Source-native embedded text, provider OCR attempts, and any explicitly
    /// recorded reconciliation remain independently addressable.
    #[serde(default)]
    pub text: ImageTextContent,
    pub chunks: Vec<ImageChunk>,
    pub vector: Option<SvgContent>,
    pub links: Vec<ImageLink>,
    pub active_content: Vec<ImageActiveContent>,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Tiff,
    Webp,
    Gif,
    Bmp,
    Heif,
    Svg,
}

impl ImageFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Tiff => "tiff",
            Self::Webp => "webp",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Heif => "heif",
            Self::Svg => "svg",
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageFrame {
    pub index: u64,
    pub dimensions: ImageDimensions,
    pub x: u32,
    pub y: u32,
    pub duration_ms: Option<u64>,
    pub disposal: Option<String>,
    pub blend: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ImageColorInfo {
    pub model: Option<String>,
    pub bits_per_component: Option<u8>,
    pub component_count: Option<u8>,
    pub alpha: Option<bool>,
    pub color_space: Option<String>,
    pub icc_profile_sha256: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ImageOrientation {
    pub exif_value: Option<u16>,
    pub rotation_degrees: i16,
    pub mirrored: bool,
}

impl ImageOrientation {
    pub fn from_exif(value: u16) -> Self {
        let (rotation_degrees, mirrored) = match value {
            2 => (0, true),
            3 => (180, false),
            4 => (180, true),
            5 => (90, true),
            6 => (90, false),
            7 => (270, true),
            8 => (270, false),
            _ => (0, false),
        };
        Self {
            exif_value: Some(value),
            rotation_degrees,
            mirrored,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ImageMetadata {
    pub blocks: Vec<ImageMetadataBlock>,
    pub camera: ImageCameraMetadata,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageMetadataBlock {
    pub kind: ImageMetadataKind,
    pub sha256: String,
    pub byte_length: u64,
    pub text: Option<String>,
    pub fields: BTreeMap<String, String>,
    pub raw_bytes: Option<Vec<u8>>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageMetadataKind {
    Exif,
    Xmp,
    Iptc,
    Icc,
    Text,
    Comment,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ImageCameraMetadata {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens_model: Option<String>,
    pub captured_at: Option<String>,
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub iso_speed: Option<String>,
    pub focal_length: Option<String>,
    pub gps_latitude: Option<String>,
    pub gps_longitude: Option<String>,
    pub tags: BTreeMap<String, String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageText {
    pub kind: ImageTextKind,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageTextKind {
    Metadata,
    Comment,
    VectorText,
    VectorTitle,
    VectorDescription,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageTextContent {
    pub native: NativeRepresentation<ImageNativeText>,
    #[serde(default)]
    pub ocr_attempts: Vec<ImageOcrAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciled: Option<ReconciledRepresentation<ImageReconciledText>>,
}

impl Default for ImageTextContent {
    fn default() -> Self {
        Self {
            native: NativeRepresentation::new(ImageNativeText::default())
                .expect("empty native image text serializes"),
            ocr_attempts: Vec::new(),
            reconciled: None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImageNativeText {
    pub text: String,
    pub entries: Vec<ImageText>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageOcrAttempt {
    pub scope: ImageOcrScope,
    pub response: ProviderResponse,
    #[serde(default)]
    pub regions: Vec<ImageOcrTextRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_order_confidence: Option<ProviderConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_confidence: Option<ProviderConfidence>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageOcrScope {
    pub frame_index: u64,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageOcrTextRegion {
    pub index: u64,
    pub text: String,
    pub bbox: crate::core::BoundingBox,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub reading_order: u64,
    pub reading_order_inferred: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImageReconciledText {
    pub text: String,
    pub items: Vec<ImageReconciledTextItem>,
    #[serde(default)]
    pub confidence_evidence: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageReconciledTextItem {
    pub index: u64,
    pub text: String,
    pub origin: ImageTextOrigin,
    pub source: ImageReconciledTextSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<crate::core::BoundingBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageTextOrigin {
    Native,
    Ocr,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum ImageReconciledTextSource {
    Native {
        entry_index: u64,
    },
    OcrOverall {
        attempt_index: u64,
    },
    Ocr {
        attempt_index: u64,
        region_index: u64,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageChunk {
    pub kind: String,
    pub known: bool,
    pub byte_length: u64,
    pub sha256: String,
    pub raw_bytes: Option<Vec<u8>>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SvgContent {
    pub view_box: Option<[f64; 4]>,
    pub element_count: u64,
    pub unknown_elements: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageLink {
    pub target: String,
    pub external: bool,
    pub locator: SourceLocator,
    pub disposition: ImageActiveContentDisposition,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageActiveContent {
    pub kind: String,
    pub name: Option<String>,
    pub value_sha256: String,
    pub locator: SourceLocator,
    pub disposition: ImageActiveContentDisposition,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageActiveContentDisposition {
    InventoriedNotExecuted,
}
