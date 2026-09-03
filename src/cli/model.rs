use crate::core::{ContentIdentity, Diagnostic, OperationKind, RequestId, SourceInfo};
use crate::detect::Detection;
use crate::render::{RenderFidelity, RenderFormat, RenderSourceMap};
use crate::transform::GraphTransformSourceMap;
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Ranked detection result with stable request correlation and source identity.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectionReport {
    pub schema_version: String,
    pub request_id: RequestId,
    pub source: SourceInfo,
    pub identity: ContentIdentity,
    pub detection: Detection,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl DetectionReport {
    pub const SCHEMA_VERSION: &'static str = "grist/cli-detection-report/v1";
}

/// Where an explicitly requested raw text rendering was written.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputDestination {
    Stdout,
    Path { path: String },
}

/// Machine-readable companion for an explicit raw text output.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextOutputManifest {
    pub schema_version: String,
    pub request_id: RequestId,
    pub operation: OperationKind,
    pub format: RenderFormat,
    pub media_type: String,
    pub destination: OutputDestination,
    pub byte_length: usize,
    pub sha256: String,
    pub renderer: String,
    pub renderer_version: String,
    pub renderer_digest: String,
    pub options_digest: String,
    pub source_map: RenderSourceMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_transform_source_map: Option<GraphTransformSourceMap>,
    pub fidelity: RenderFidelity,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl TextOutputManifest {
    pub const SCHEMA_VERSION: &'static str = "grist/cli-text-output-manifest/v1";
}
