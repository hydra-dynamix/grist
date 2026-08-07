//! Versioned wire contracts consumed and produced by segmentation engines.

use crate::core::{ContentIdentity, Diagnostic, SchemaVersion, SourceLocator};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentSizeUnit {
    #[default]
    UnicodeScalars,
    Bytes,
    Tokens,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenizerSpec {
    pub name: String,
    pub version: String,
    pub configuration_digest: String,
}

impl TokenizerSpec {
    pub fn unicode_whitespace_v1() -> Self {
        Self {
            name: "grist.unicode_whitespace".to_string(),
            version: "1".to_string(),
            configuration_digest: crate::core::sha256_hex(
                b"grist/unicode-whitespace-tokenizer/config/v1",
            ),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    Document,
    Section,
    Paragraph,
    ListItem,
    TableRow,
    CodeSymbol,
    Page,
    Slide,
    SheetRange,
    Message,
    NotebookCell,
    TranscriptCue,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AtomicityOptions {
    pub tables: bool,
    pub code: bool,
    pub equations: bool,
    pub figure_captions: bool,
}

impl Default for AtomicityOptions {
    fn default() -> Self {
        Self {
            tables: true,
            code: true,
            equations: true,
            figure_captions: true,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSelectionRules {
    #[serde(default)]
    pub include_kinds: Vec<String>,
    #[serde(default)]
    pub exclude_kinds: Vec<String>,
    #[serde(default)]
    pub required_metadata: BTreeMap<String, String>,
}

/// A serializable, versioned segmentation options file.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SegmentOptions {
    pub schema_version: String,
    pub target_size: usize,
    pub maximum_size: usize,
    pub size_unit: SegmentSizeUnit,
    pub tokenizer: Option<TokenizerSpec>,
    pub include_heading_ancestry: bool,
    pub overlap_source_nodes: usize,
    pub preferred_boundaries: Vec<BoundaryKind>,
    pub atomicity: AtomicityOptions,
    pub selection: NodeSelectionRules,
    /// Node attribute keys copied into deterministic, source-node-keyed metadata arrays.
    #[serde(default)]
    pub project_metadata: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}

impl Default for SegmentOptions {
    fn default() -> Self {
        Self {
            schema_version: SchemaVersion::SEGMENT_OPTIONS_V1.to_string(),
            target_size: 1_000,
            maximum_size: 1_500,
            size_unit: SegmentSizeUnit::UnicodeScalars,
            tokenizer: None,
            include_heading_ancestry: true,
            overlap_source_nodes: 0,
            preferred_boundaries: vec![
                BoundaryKind::Section,
                BoundaryKind::Paragraph,
                BoundaryKind::ListItem,
                BoundaryKind::TableRow,
                BoundaryKind::CodeSymbol,
            ],
            atomicity: AtomicityOptions::default(),
            selection: NodeSelectionRules::default(),
            project_metadata: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentContext {
    #[serde(default)]
    pub structural_path: Vec<String>,
    #[serde(default)]
    pub section_titles: Vec<String>,
    pub document_title: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentOverlap {
    pub segment_id: String,
    pub node_ids: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentNodeRole {
    Ancestry,
    Overlap,
    #[default]
    Content,
}

/// Exact mapping from one contiguous rendered span to its original graph node.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SegmentNodeReference {
    pub node_id: String,
    pub locator: SourceLocator,
    pub role: SegmentNodeRole,
    pub segment_byte_start: usize,
    pub segment_byte_end: usize,
}

/// Unit-independent counts retained even when `size` selects only one unit.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentCounts {
    pub bytes: usize,
    pub unicode_scalars: usize,
    pub tokens: Option<usize>,
}

/// A source-traceable structural segment. Engines must reuse `node_ids` for overlap.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub schema_version: String,
    pub id: String,
    pub node_ids: Vec<String>,
    pub text: String,
    pub locators: Vec<SourceLocator>,
    pub source_identity: ContentIdentity,
    pub document_identity: ContentIdentity,
    pub context: SegmentContext,
    pub size: usize,
    pub token_count: Option<usize>,
    pub tokenizer: Option<TokenizerSpec>,
    pub renderer: String,
    pub renderer_version: String,
    pub options_digest: String,
    /// Canonical digest of the renderer name, version, and separator policy.
    #[serde(default)]
    pub renderer_digest: String,
    /// Ordered, exact rendered-span mappings. Legacy `node_ids` and `locators`
    /// are projections of this collection for v1 consumers.
    #[serde(default)]
    pub node_references: Vec<SegmentNodeReference>,
    #[serde(default)]
    pub counts: SegmentCounts,
    #[serde(default)]
    pub overlaps: Vec<SegmentOverlap>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SegmentCollection {
    pub schema_version: String,
    pub document_graph_schema_version: String,
    pub options: SegmentOptions,
    pub options_digest: String,
    pub segments: Vec<Segment>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SegmentEvent {
    Segment {
        sequence: u64,
        segment: Box<Segment>,
    },
    Diagnostic {
        diagnostic: Box<Diagnostic>,
    },
    Terminal {
        segment_count: u64,
        diagnostics: Vec<Diagnostic>,
    },
}
