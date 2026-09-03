//! Normalized, provenance-preserving document renderers.

mod model;
mod normalized;

pub use model::{
    FidelityMode, GeneratedRange, NORMALIZED_RENDERER_NAME, NORMALIZED_RENDERER_VERSION,
    RENDER_RESULT_V1, RENDER_SOURCE_MAP_V1, ReconstructionClaim, RenderError, RenderFidelity,
    RenderFormat, RenderLoss, RenderLossKind, RenderOptions, RenderResult, RenderSourceMap,
    RenderSourceMapEntry, SourceMapError, SourceMapLocatorStatus,
};
pub use normalized::render_document_graph;

#[cfg(feature = "latex")]
pub use crate::document_graph::render_latex;
#[cfg(feature = "markdown")]
pub use crate::document_graph::render_markdown;
pub use crate::document_graph::{
    TransformError, TransformOptions, TransformWarning, TransformWarningKind,
};
