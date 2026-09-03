//! Public normalized-rendering contracts.

use crate::core::{Diagnostic, SchemaVersion, SourceLocator};
use crate::document_graph::DocumentNodeKind;
#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const RENDER_RESULT_V1: &str = SchemaVersion::RENDER_RESULT_V1;
pub const RENDER_SOURCE_MAP_V1: &str = SchemaVersion::RENDER_SOURCE_MAP_V1;
pub const NORMALIZED_RENDERER_NAME: &str = "grist.render.normalized";
pub const NORMALIZED_RENDERER_VERSION: &str = "1";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderFormat {
    Markdown,
    Latex,
    Html,
    PlainText,
    CanonicalJson,
}

impl RenderFormat {
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Latex => "application/x-latex; charset=utf-8",
            Self::Html => "text/html; charset=utf-8",
            Self::PlainText => "text/plain; charset=utf-8",
            Self::CanonicalJson => "application/json",
        }
    }
}

/// Fidelity is always explicit; there is no implicit lossy mode.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FidelityMode {
    Strict,
    RawFallback,
    Lossy,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderOptions {
    pub fidelity: FidelityMode,
}

impl RenderOptions {
    pub const fn new(fidelity: FidelityMode) -> Self {
        Self { fidelity }
    }
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self::new(FidelityMode::Strict)
    }
}

/// Half-open UTF-8 byte range in generated output.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeneratedRange {
    pub byte_start: usize,
    pub byte_end: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceMapLocatorStatus {
    Exact,
    Approximate,
    Synthetic,
    Unavailable,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderSourceMapEntry {
    pub generated: GeneratedRange,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_locator: Option<SourceLocator>,
    pub locator_status: SourceMapLocatorStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator_unavailable_reason: Option<String>,
}

/// Complete, gap-free generated-byte attribution for a renderer result.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderSourceMap {
    pub schema_version: String,
    pub generated_unit: String,
    pub generated_length: usize,
    pub entries: Vec<RenderSourceMapEntry>,
}

impl RenderSourceMap {
    pub fn validate(&self, output: &str) -> Result<(), SourceMapError> {
        if self.schema_version != RENDER_SOURCE_MAP_V1 {
            return Err(SourceMapError::UnsupportedSchemaVersion(
                self.schema_version.clone(),
            ));
        }
        if self.generated_unit != "utf8_bytes" {
            return Err(SourceMapError::UnsupportedGeneratedUnit(
                self.generated_unit.clone(),
            ));
        }
        if self.generated_length != output.len() {
            return Err(SourceMapError::LengthMismatch {
                declared: self.generated_length,
                actual: output.len(),
            });
        }
        let mut cursor = 0;
        for entry in &self.entries {
            if entry.generated.byte_start != cursor {
                return Err(SourceMapError::CoverageGap {
                    expected: cursor,
                    actual: entry.generated.byte_start,
                });
            }
            if entry.generated.byte_end <= entry.generated.byte_start
                || entry.generated.byte_end > output.len()
                || !output.is_char_boundary(entry.generated.byte_start)
                || !output.is_char_boundary(entry.generated.byte_end)
            {
                return Err(SourceMapError::InvalidGeneratedRange(entry.generated));
            }
            if entry.node_id.is_empty() {
                return Err(SourceMapError::MissingNodeId);
            }
            validate_locator_declaration(entry)?;
            cursor = entry.generated.byte_end;
        }
        if cursor != output.len() {
            return Err(SourceMapError::CoverageGap {
                expected: output.len(),
                actual: cursor,
            });
        }
        Ok(())
    }
}

fn validate_locator_declaration(entry: &RenderSourceMapEntry) -> Result<(), SourceMapError> {
    match (
        entry.locator_status,
        entry.original_locator.as_ref(),
        entry.locator_unavailable_reason.as_deref(),
    ) {
        (SourceMapLocatorStatus::Unavailable, None, Some(reason)) if !reason.trim().is_empty() => {
            Ok(())
        }
        (SourceMapLocatorStatus::Unavailable, _, _) => {
            Err(SourceMapError::InvalidLocatorDeclaration {
                node_id: entry.node_id.clone(),
            })
        }
        (_, Some(locator), None) => {
            locator
                .validate()
                .map_err(|error| SourceMapError::InvalidOriginalLocator {
                    node_id: entry.node_id.clone(),
                    message: error.to_string(),
                })
        }
        _ => Err(SourceMapError::InvalidLocatorDeclaration {
            node_id: entry.node_id.clone(),
        }),
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderLossKind {
    UnsupportedNodeKind,
    RawSourceUnavailable,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderLoss {
    pub node_id: String,
    pub node_kind: DocumentNodeKind,
    pub kind: RenderLossKind,
    pub message: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconstructionClaim {
    NormalizedNotByteRoundTrip,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderFidelity {
    pub mode: FidelityMode,
    pub reconstruction_claim: ReconstructionClaim,
    pub losses: Vec<RenderLoss>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderResult {
    pub schema_version: String,
    pub format: RenderFormat,
    pub media_type: String,
    pub renderer: String,
    pub renderer_version: String,
    pub renderer_digest: String,
    pub options_digest: String,
    pub content: String,
    pub source_map: RenderSourceMap,
    pub fidelity: RenderFidelity,
    pub diagnostics: Vec<Diagnostic>,
}

impl RenderResult {
    pub fn validate_source_map(&self) -> Result<(), SourceMapError> {
        self.source_map.validate(&self.content)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceMapError {
    #[error("unsupported render source-map schema version {0}")]
    UnsupportedSchemaVersion(String),
    #[error("unsupported generated-range unit {0}")]
    UnsupportedGeneratedUnit(String),
    #[error("source-map output length mismatch: declared {declared}, actual {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("source-map coverage gap: expected offset {expected}, found {actual}")]
    CoverageGap { expected: usize, actual: usize },
    #[error("invalid generated range {0:?}")]
    InvalidGeneratedRange(GeneratedRange),
    #[error("source-map entry has no node ID")]
    MissingNodeId,
    #[error("invalid locator declaration for node {node_id}")]
    InvalidLocatorDeclaration { node_id: String },
    #[error("invalid original locator for node {node_id}: {message}")]
    InvalidOriginalLocator { node_id: String, message: String },
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("invalid document graph: {message}")]
    InvalidGraph { message: String },
    #[error("{format:?} cannot represent node {node_id} of kind {node_kind:?} in strict mode")]
    UnsupportedNode {
        format: RenderFormat,
        node_id: String,
        node_kind: DocumentNodeKind,
    },
    #[error("node {node_id} has no retained raw source for {format:?} raw fallback")]
    RawSourceUnavailable {
        format: RenderFormat,
        node_id: String,
    },
    #[error("canonical JSON rendering failed: {message}")]
    CanonicalJson { message: String },
    #[error("render source-map construction failed: {0}")]
    SourceMap(#[from] SourceMapError),
}
