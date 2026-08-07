use crate::container::EmbeddedArtifact;
use crate::core::{SecretString, SourceInfo, SourceLocator};
use crate::provider::{
    NativeRepresentation, ProviderConfidence, ProviderResponse, ReconciledRepresentation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct PdfOcrOptions {
    pub mode: PdfOcrMode,
    pub language_hints: Vec<String>,
    pub recognize_tables: bool,
    pub max_scopes: u64,
    pub duplicate_text_similarity: f64,
    pub duplicate_overlap: f64,
}

impl Default for PdfOcrOptions {
    fn default() -> Self {
        Self {
            mode: PdfOcrMode::Auto,
            language_hints: Vec::new(),
            recognize_tables: true,
            max_scopes: 100_000,
            duplicate_text_similarity: 0.9,
            duplicate_overlap: 0.5,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfOcrMode {
    Disabled,
    /// OCR pages without usable native text and raster regions on hybrid pages.
    #[default]
    Auto,
    /// OCR every complete page, even when native text is usable.
    AllPages,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PdfOptions {
    pub max_objects: u32,
    pub max_object_depth: u16,
    pub max_pages: u32,
    pub max_page_tree_depth: u16,
    pub max_page_label_nodes: u32,
    pub max_decoded_stream_bytes: u64,
    pub max_content_operations: u64,
    pub max_native_glyphs: u64,
    pub max_semantic_graphics: u64,
    pub max_interactive_objects: u64,
    pub max_embedded_files: u64,
    pub max_embedded_bytes: u64,
    pub max_embedded_depth: u16,
    pub ocr: PdfOcrOptions,
    #[serde(skip)]
    #[cfg_attr(feature = "schemas", schemars(skip))]
    password: Option<SecretString>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            max_objects: 500_000,
            max_object_depth: 128,
            max_pages: 100_000,
            max_page_tree_depth: 256,
            max_page_label_nodes: 100_000,
            max_decoded_stream_bytes: 64 * 1024 * 1024,
            max_content_operations: 5_000_000,
            max_native_glyphs: 10_000_000,
            max_semantic_graphics: 1_000_000,
            max_interactive_objects: 1_000_000,
            max_embedded_files: 10_000,
            max_embedded_bytes: 256 * 1024 * 1024,
            max_embedded_depth: 16,
            ocr: PdfOcrOptions::default(),
            password: None,
        }
    }
}

impl PdfOptions {
    pub fn with_password(mut self, password: SecretString) -> Self {
        self.password = Some(password);
        self
    }

    pub fn set_password(&mut self, password: SecretString) {
        self.password = Some(password);
    }

    pub fn password(&self) -> Option<&SecretString> {
        self.password.as_ref()
    }
}

impl fmt::Debug for PdfOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PdfOptions")
            .field("max_objects", &self.max_objects)
            .field("max_object_depth", &self.max_object_depth)
            .field("max_pages", &self.max_pages)
            .field("max_page_tree_depth", &self.max_page_tree_depth)
            .field("max_page_label_nodes", &self.max_page_label_nodes)
            .field("max_decoded_stream_bytes", &self.max_decoded_stream_bytes)
            .field("max_content_operations", &self.max_content_operations)
            .field("max_native_glyphs", &self.max_native_glyphs)
            .field("max_semantic_graphics", &self.max_semantic_graphics)
            .field("max_interactive_objects", &self.max_interactive_objects)
            .field("max_embedded_files", &self.max_embedded_files)
            .field("max_embedded_bytes", &self.max_embedded_bytes)
            .field("max_embedded_depth", &self.max_embedded_depth)
            .field("ocr", &self.ocr)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl crate::core::FormatOptions for PdfOptions {
    const FORMAT: &'static str = "pdf";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfDocument {
    pub schema_version: String,
    pub source: SourceInfo,
    pub header: PdfHeader,
    pub catalog: PdfCatalog,
    pub trailer: PdfTrailer,
    pub xref: PdfCrossReference,
    pub objects: Vec<PdfIndirectObject>,
    pub page_tree: Vec<PdfPageTreeNode>,
    pub pages: Vec<PdfPage>,
    pub native_layout: PdfNativeLayout,
    /// Native, provider-derived, and reconciled text remain independently
    /// addressable. This is additive so older v1 payloads deserialize with an
    /// empty collection rather than conflating representations.
    #[serde(default)]
    pub text: PdfTextContent,
    pub semantic_structure: PdfSemanticStructure,
    pub page_labels: Vec<PdfPageLabel>,
    pub metadata: PdfMetadata,
    pub filters: Vec<PdfFilterUsage>,
    pub encryption: PdfEncryption,
    pub repairs: Vec<PdfRepair>,
    pub active_content: Vec<PdfActiveContent>,
    pub interactive: PdfInteractiveContent,
}

pub type PdfEnvelope = crate::core::Envelope<PdfDocument>;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfHeader {
    pub version: String,
    pub byte_offset: u64,
    pub binary_marker: bool,
    pub eof_marker_offset: Option<u64>,
    pub trailing_bytes: u64,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PdfReference {
    pub object_number: u32,
    pub generation: u16,
}

impl fmt::Display for PdfReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {} R", self.object_number, self.generation)
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PdfObjectLocation {
    Direct {
        byte_start: u64,
        byte_end: u64,
    },
    ObjectStream {
        container: PdfReference,
        decoded_byte_start: u64,
        decoded_byte_end: u64,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfObjectLocator {
    pub stable_id: String,
    pub object: PdfReference,
    pub location: PdfObjectLocation,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_path: Vec<String>,
}

impl PdfObjectLocator {
    pub fn direct(object: PdfReference, byte_start: usize, byte_end: usize) -> Self {
        Self {
            stable_id: format!("pdf-object-{}-{}", object.object_number, object.generation),
            object,
            location: PdfObjectLocation::Direct {
                byte_start: byte_start as u64,
                byte_end: byte_end as u64,
            },
            key_path: Vec::new(),
        }
    }

    pub fn at_key(&self, key: impl Into<String>) -> Self {
        let mut locator = self.clone();
        locator.key_path.push(key.into());
        locator
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PdfValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    Name(String),
    String(PdfString),
    Array(Vec<PdfValue>),
    Dictionary(BTreeMap<String, PdfValue>),
    Reference(PdfReference),
    Keyword(String),
}

impl PdfValue {
    pub fn as_dictionary(&self) -> Option<&BTreeMap<String, PdfValue>> {
        match self {
            Self::Dictionary(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_reference(&self) -> Option<PdfReference> {
        match self {
            Self::Reference(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfString {
    pub raw_hex: String,
    pub text: String,
    pub encoding: PdfStringEncoding,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfStringEncoding {
    PdfDoc,
    Utf16BigEndian,
    Binary,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfIndirectObject {
    pub object: PdfReference,
    pub revision: u32,
    pub locator: PdfObjectLocator,
    pub value: PdfValue,
    pub stream: Option<PdfStream>,
    pub raw_sha256: String,
    pub repaired: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfStream {
    pub byte_start: u64,
    pub byte_end: u64,
    pub declared_length: Option<u64>,
    pub actual_length: u64,
    pub encoded_sha256: String,
    pub decoded_length: Option<u64>,
    pub decoded_sha256: Option<String>,
    pub decode_status: PdfStreamDecodeStatus,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfStreamDecodeStatus {
    NotFiltered,
    Decoded,
    NativeEncoded,
    Unsupported,
    Malformed,
    BudgetExceeded,
    Encrypted,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfCatalog {
    pub object: PdfReference,
    pub locator: PdfObjectLocator,
    pub pages: PdfReference,
    pub version: Option<String>,
    pub page_labels: Option<PdfReference>,
    pub metadata: Option<PdfReference>,
    pub outlines: Option<PdfReference>,
    pub names: Option<PdfReference>,
    pub acro_form: Option<PdfReference>,
    pub destinations: Option<PdfReference>,
    pub optional_content: Option<PdfReference>,
    pub structure_tree_root: Option<PdfReference>,
    pub language: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfTrailer {
    pub locator: PdfObjectLocator,
    pub size: Option<u64>,
    pub root: PdfReference,
    pub info: Option<PdfReference>,
    pub encrypt: Option<PdfReference>,
    pub id: Vec<PdfString>,
    pub previous_xref: Option<u64>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfCrossReference {
    pub startxref: Option<u64>,
    pub sections: Vec<PdfXrefSection>,
    pub entries: Vec<PdfXrefEntry>,
    pub repaired: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfXrefSection {
    pub byte_offset: u64,
    pub kind: PdfXrefKind,
    pub entry_count: u64,
    pub previous: Option<u64>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfXrefKind {
    Table,
    Stream,
    RecoveredObjectScan,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfXrefEntry {
    pub object_number: u32,
    pub generation: u16,
    pub kind: PdfXrefEntryKind,
    pub byte_offset: Option<u64>,
    pub object_stream: Option<PdfReference>,
    pub object_stream_index: Option<u32>,
    pub valid: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfXrefEntryKind {
    Free,
    InUse,
    Compressed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfPageTreeNode {
    pub object: PdfReference,
    pub parent: Option<PdfReference>,
    pub kids: Vec<PdfReference>,
    pub declared_count: Option<u64>,
    pub actual_descendant_pages: u64,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfPage {
    pub index: u64,
    pub object: PdfReference,
    pub parent: Option<PdfReference>,
    pub media_box: PdfRectangle,
    pub crop_box: PdfRectangle,
    pub width_points: f64,
    pub height_points: f64,
    pub rotation_degrees: i16,
    pub user_unit: f64,
    pub label: Option<String>,
    pub locator: SourceLocator,
    pub object_locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PdfRectangle {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfNativeLayout {
    pub fonts: Vec<PdfFont>,
    pub pages: Vec<PdfPageNativeLayout>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfFont {
    pub id: String,
    pub resource_name: String,
    pub object: Option<PdfReference>,
    pub subtype: Option<String>,
    pub base_font: Option<String>,
    pub encoding: Option<String>,
    pub embedded: bool,
    pub to_unicode: PdfUnicodeMapStatus,
    pub direction: PdfWritingDirection,
    pub style: PdfFontStyle,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfFontStyle {
    pub weight: Option<u16>,
    pub italic_angle: Option<f64>,
    pub bold: bool,
    pub italic: bool,
    pub serif: bool,
    pub monospaced: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfUnicodeMapStatus {
    Present,
    StandardEncoding,
    Missing,
    Malformed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PdfWritingDirection {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
    Mixed,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfPageNativeLayout {
    pub page_index: u64,
    pub rotation_degrees: i16,
    pub status: PdfNativeTextStatus,
    pub content_objects: Vec<PdfReference>,
    pub glyphs: Vec<PdfGlyph>,
    pub tokens: Vec<PdfTextToken>,
    pub lines: Vec<PdfTextLine>,
    pub blocks: Vec<PdfTextBlock>,
    pub reading_order: PdfReadingOrder,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfNativeTextStatus {
    Extracted,
    NoNativeText,
    Unusable,
    BudgetExceeded,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PdfTextContent {
    pub pages: Vec<PdfPageTextContent>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfPageTextContent {
    pub page_index: u64,
    pub native: NativeRepresentation<PdfNativePageText>,
    #[serde(default)]
    pub ocr_attempts: Vec<PdfOcrAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciled: Option<ReconciledRepresentation<PdfReconciledPageText>>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfNativePageText {
    pub page_index: u64,
    pub text: String,
    pub status: PdfNativeTextStatus,
    pub regions: Vec<PdfNativeTextRegion>,
    pub reading_order_confidence: f64,
    pub layout_confidence: f64,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfNativeTextRegion {
    pub index: u64,
    pub text: String,
    pub bbox: crate::core::BoundingBox,
    pub reading_order: u64,
    pub layout_confidence: f64,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfOcrAttempt {
    pub scope: PdfOcrScope,
    pub response: ProviderResponse,
    #[serde(default)]
    pub regions: Vec<PdfOcrTextRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_order_confidence: Option<ProviderConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_confidence: Option<ProviderConfidence>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfOcrScope {
    pub kind: PdfOcrScopeKind,
    pub reason: PdfOcrScopeReason,
    pub page_index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphic_index: Option<u64>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfOcrScopeKind {
    Page,
    Region,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfOcrScopeReason {
    NoNativeText,
    NativeTextUnusable,
    NativeTextBudgetExceeded,
    RasterImageOnHybridPage,
    AllPagesRequested,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfOcrTextRegion {
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfReconciledPageText {
    pub text: String,
    pub items: Vec<PdfReconciledTextItem>,
    pub suppressions: Vec<PdfTextSuppression>,
    pub reading_order_confidence: f64,
    pub layout_confidence: f64,
    pub confidence_evidence: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfReconciledTextItem {
    pub index: u64,
    pub text: String,
    pub origin: PdfTextOrigin,
    pub source: PdfReconciledTextSource,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfTextOrigin {
    Native,
    Ocr,
    Reconciled,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum PdfReconciledTextSource {
    Native {
        region_index: u64,
    },
    Ocr {
        attempt_index: u64,
        region_index: u64,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfTextSuppression {
    pub reason: PdfTextSuppressionReason,
    pub suppressed: PdfReconciledTextSource,
    pub retained: PdfReconciledTextSource,
    pub suppressed_text: String,
    pub text_similarity: f64,
    pub geometric_overlap: f64,
    pub locator: SourceLocator,
    pub retained_locator: SourceLocator,
    pub loss_class: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfTextSuppressionReason {
    DuplicateNativePreferred,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfGlyph {
    pub index: u64,
    pub text: String,
    pub raw_code_hex: String,
    pub font_id: String,
    pub font_size: f64,
    pub direction: PdfWritingDirection,
    pub rendering_mode: u8,
    pub mapping_confidence: f64,
    pub geometry_confidence: f64,
    pub bbox: crate::core::BoundingBox,
    pub content_object: PdfReference,
    pub operation_index: u64,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfTextToken {
    pub index: u64,
    pub text: String,
    pub glyph_start: u64,
    pub glyph_end: u64,
    pub font_ids: Vec<String>,
    pub direction: PdfWritingDirection,
    pub bbox: crate::core::BoundingBox,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfTextLine {
    pub index: u64,
    pub text: String,
    pub token_start: u64,
    pub token_end: u64,
    pub direction: PdfWritingDirection,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
    pub evidence: Vec<PdfReadingEvidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfTextBlock {
    pub index: u64,
    pub text: String,
    pub line_start: u64,
    pub line_end: u64,
    pub token_start: u64,
    pub token_end: u64,
    pub direction: PdfWritingDirection,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfReadingOrder {
    pub block_order: Vec<u64>,
    pub confidence: f64,
    pub evidence: Vec<PdfReadingEvidence>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfReadingEvidence {
    pub kind: PdfReadingEvidenceKind,
    pub description: String,
    pub confidence: f64,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfReadingEvidenceKind {
    ContentStreamSequence,
    TextMatrixBaseline,
    GeometricLineClustering,
    ColumnSeparation,
    UnicodeDirectionality,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfSemanticStructure {
    pub structure_tree: PdfStructureTreeSummary,
    pub pages: Vec<PdfPageSemanticStructure>,
    pub repeated_regions: Vec<PdfRepeatedRegion>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfStructureTreeSummary {
    pub root: Option<PdfReference>,
    pub status: PdfStructureTreeStatus,
    pub element_count: u64,
    pub confidence: f64,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfStructureTreeStatus {
    Absent,
    Valid,
    Malformed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfPageSemanticStructure {
    pub page_index: u64,
    pub columns: Vec<PdfColumn>,
    pub blocks: Vec<PdfSemanticBlock>,
    pub lists: Vec<PdfList>,
    pub tables: Vec<PdfTable>,
    pub graphics: Vec<PdfGraphicObject>,
    pub figures: Vec<PdfFigure>,
    pub reading_order: Vec<PdfSemanticReadingItem>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfColumn {
    pub index: u64,
    pub block_indices: Vec<u64>,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
    pub evidence: Vec<PdfSemanticEvidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfSemanticBlock {
    pub index: u64,
    pub native_block_index: u64,
    pub kind: PdfSemanticBlockKind,
    pub heading_level: Option<u8>,
    pub list_marker: Option<String>,
    pub text: String,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
    pub evidence: Vec<PdfSemanticEvidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfSemanticBlockKind {
    Heading,
    Paragraph,
    ListItem,
    Header,
    Footer,
    PageNumber,
    Caption,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfList {
    pub index: u64,
    pub item_block_indices: Vec<u64>,
    pub ordered: bool,
    pub confidence: f64,
    pub evidence: Vec<PdfSemanticEvidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfRepeatedRegion {
    pub index: u64,
    pub kind: PdfRepeatedRegionKind,
    pub normalized_pattern: String,
    pub occurrences: Vec<PdfRepeatedRegionOccurrence>,
    pub confidence: f64,
    pub evidence: Vec<PdfSemanticEvidence>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PdfRepeatedRegionKind {
    Header,
    Footer,
    PageNumber,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfRepeatedRegionOccurrence {
    pub page_index: u64,
    pub block_index: u64,
    pub text: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfTable {
    pub index: u64,
    pub rows: Vec<PdfTableRow>,
    pub column_count: u64,
    pub caption_block_index: Option<u64>,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
    pub evidence: Vec<PdfSemanticEvidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfTableRow {
    pub index: u64,
    pub cells: Vec<PdfTableCell>,
    pub bbox: crate::core::BoundingBox,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfTableCell {
    pub row: u64,
    pub column: u64,
    pub row_span: u64,
    pub column_span: u64,
    pub text: String,
    pub header_candidate: bool,
    pub header_confidence: f64,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
    pub evidence: Vec<PdfSemanticEvidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfGraphicObject {
    pub index: u64,
    pub kind: PdfGraphicObjectKind,
    pub source_object: PdfReference,
    pub resource_name: Option<String>,
    pub pixel_width: Option<u64>,
    pub pixel_height: Option<u64>,
    pub bbox: crate::core::BoundingBox,
    pub segments: Vec<PdfLineSegment>,
    pub confidence: f64,
    pub evidence: Vec<PdfSemanticEvidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfGraphicObjectKind {
    RasterImage,
    VectorPath,
    FormXObject,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct PdfLineSegment {
    pub start: [f64; 2],
    pub end: [f64; 2],
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfFigure {
    pub index: u64,
    pub graphic_indices: Vec<u64>,
    pub caption_block_index: Option<u64>,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
    pub evidence: Vec<PdfSemanticEvidence>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfSemanticReadingItem {
    pub kind: PdfSemanticReadingItemKind,
    pub index: u64,
    pub bbox: crate::core::BoundingBox,
    pub confidence: f64,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfSemanticReadingItemKind {
    Block,
    Table,
    Figure,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfSemanticEvidence {
    pub kind: PdfSemanticEvidenceKind,
    pub description: String,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_blocks: Vec<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_objects: Vec<PdfReference>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfSemanticEvidenceKind {
    TaggedStructure,
    FontScale,
    FontStyle,
    TextPattern,
    GeometricAlignment,
    ColumnSeparation,
    RepeatedPosition,
    RulingLines,
    XObjectInvocation,
    CaptionProximity,
    NativeReadingOrder,
}

impl PdfRectangle {
    pub fn width(self) -> f64 {
        (self.right - self.left).abs()
    }
    pub fn height(self) -> f64 {
        (self.top - self.bottom).abs()
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfPageLabel {
    pub page_index: u64,
    pub label: String,
    pub style: Option<PdfPageLabelStyle>,
    pub prefix: Option<String>,
    pub start: u64,
    pub rule_locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfPageLabelStyle {
    Decimal,
    UpperRoman,
    LowerRoman,
    UpperLetters,
    LowerLetters,
    None,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfMetadata {
    pub info_object: Option<PdfReference>,
    pub fields: Vec<PdfMetadataField>,
    pub xmp_object: Option<PdfReference>,
    pub xmp: Option<String>,
    pub xmp_sha256: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfMetadataField {
    pub name: String,
    pub value: PdfValue,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfFilterUsage {
    pub object: PdfReference,
    pub filters: Vec<String>,
    pub supported: bool,
    pub status: PdfStreamDecodeStatus,
    pub decoded_bytes: Option<u64>,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfEncryption {
    pub encrypted: bool,
    pub object: Option<PdfReference>,
    pub handler: Option<String>,
    pub sub_filter: Option<String>,
    pub algorithm_version: Option<i64>,
    pub revision: Option<i64>,
    pub key_length_bits: Option<u64>,
    pub permissions: Option<i64>,
    pub encrypt_metadata: Option<bool>,
    pub string_filter: Option<String>,
    pub stream_filter: Option<String>,
    pub embedded_file_filter: Option<String>,
    pub credential_supplied: bool,
    pub locator: Option<PdfObjectLocator>,
}

impl PdfEncryption {
    pub fn unencrypted() -> Self {
        Self {
            encrypted: false,
            object: None,
            handler: None,
            sub_filter: None,
            algorithm_version: None,
            revision: None,
            key_length_bits: None,
            permissions: None,
            encrypt_metadata: None,
            string_filter: None,
            stream_filter: None,
            embedded_file_filter: None,
            credential_supplied: false,
            locator: None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfRepair {
    pub code: String,
    pub description: String,
    pub object: Option<PdfReference>,
    pub original_offset: Option<u64>,
    pub recovered_offset: Option<u64>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfActiveContent {
    pub object: PdfReference,
    pub key: String,
    pub action_type: Option<String>,
    pub locator: PdfObjectLocator,
    pub disposition: PdfActiveContentDisposition,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfActiveContentDisposition {
    InventoriedNotExecuted,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PdfInteractiveContent {
    pub destinations: Vec<PdfNamedDestination>,
    pub outlines: Vec<PdfOutlineItem>,
    pub links: Vec<PdfLink>,
    pub annotations: Vec<PdfAnnotation>,
    pub comments: Vec<PdfComment>,
    pub form: Option<PdfForm>,
    pub signatures: Vec<PdfSignature>,
    pub layers: Vec<PdfLayer>,
    pub embedded_files: Vec<PdfEmbeddedFile>,
    pub relationships: Vec<PdfInteractiveRelationship>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfNamedDestination {
    pub id: String,
    pub name: String,
    pub destination: PdfDestination,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfDestination {
    pub page_object: Option<PdfReference>,
    pub page_index: Option<u64>,
    pub view: PdfDestinationView,
    pub parameters: Vec<Option<f64>>,
    pub raw: PdfValue,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfDestinationView {
    Xyz,
    Fit,
    FitHorizontal,
    FitVertical,
    FitRectangle,
    FitBoundingBox,
    FitBoundingBoxHorizontal,
    FitBoundingBoxVertical,
    Unknown(String),
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAction {
    pub action_type: String,
    pub destination: Option<PdfDestination>,
    pub uri: Option<String>,
    pub file: Option<String>,
    pub named_action: Option<String>,
    pub script_sha256: Option<String>,
    pub disposition: PdfActiveContentDisposition,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfOutlineItem {
    pub id: String,
    pub object: PdfReference,
    pub title: Option<String>,
    pub parent: Option<PdfReference>,
    pub first_child: Option<PdfReference>,
    pub next_sibling: Option<PdfReference>,
    pub previous_sibling: Option<PdfReference>,
    pub destination: Option<PdfDestination>,
    pub named_destination: Option<String>,
    pub action: Option<PdfAction>,
    pub open: Option<bool>,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfLink {
    pub id: String,
    pub annotation_object: PdfReference,
    pub page_index: u64,
    pub rectangle: Option<PdfRectangle>,
    pub destination: Option<PdfDestination>,
    pub named_destination: Option<String>,
    pub action: Option<PdfAction>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfAnnotation {
    pub id: String,
    pub object: PdfReference,
    pub page_index: u64,
    pub subtype: String,
    pub rectangle: Option<PdfRectangle>,
    pub contents: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub modified: Option<String>,
    pub in_reply_to: Option<PdfReference>,
    pub popup: Option<PdfReference>,
    pub action: Option<PdfAction>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfComment {
    pub id: String,
    pub annotation_id: String,
    pub text: String,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub in_reply_to: Option<PdfReference>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfForm {
    pub object: PdfReference,
    pub need_appearances: Option<bool>,
    pub signature_flags: Option<i64>,
    pub calculation_order: Vec<PdfReference>,
    pub fields: Vec<PdfFormField>,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfFormField {
    pub id: String,
    pub object: PdfReference,
    pub parent: Option<PdfReference>,
    pub children: Vec<PdfReference>,
    pub field_type: Option<String>,
    pub partial_name: Option<String>,
    pub alternate_name: Option<String>,
    pub mapping_name: Option<String>,
    pub value: Option<PdfValue>,
    pub default_value: Option<PdfValue>,
    pub flags: Option<i64>,
    pub action: Option<PdfAction>,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfSignature {
    pub id: String,
    pub field_object: PdfReference,
    pub signature_object: Option<PdfReference>,
    pub filter: Option<String>,
    pub sub_filter: Option<String>,
    pub signer_name: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact_info: Option<String>,
    pub signing_time: Option<String>,
    pub byte_range: Vec<i64>,
    pub contents_sha256: Option<String>,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfLayer {
    pub id: String,
    pub object: PdfReference,
    pub name: Option<String>,
    pub intent: Vec<String>,
    pub usage: Option<PdfValue>,
    pub initially_visible: Option<bool>,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfEmbeddedFile {
    pub id: String,
    pub file_spec_object: PdfReference,
    pub stream_object: Option<PdfReference>,
    pub description: Option<String>,
    pub relationship: Option<String>,
    pub artifact: EmbeddedArtifact,
    pub child_status: PdfEmbeddedChildStatus,
    pub children: Vec<PdfEmbeddedFile>,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfEmbeddedChildStatus {
    Parsed,
    Unsupported,
    Encrypted,
    BudgetLimited,
    Malformed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PdfInteractiveRelationship {
    pub source_id: String,
    pub relation: PdfInteractiveRelation,
    pub target_id: String,
    pub locator: PdfObjectLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PdfInteractiveRelation {
    ResolvesTo,
    ParentOf,
    NextSibling,
    PreviousSibling,
    AnnotationFor,
    ReplyTo,
    FieldWidget,
    AttachmentOf,
    EmbeddedIn,
}
