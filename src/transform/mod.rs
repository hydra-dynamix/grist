//! Provenance-preserving graph transforms and format-scoped reconstruction.
//!
//! Normalized graph operations and faithful package reconstruction are
//! deliberately separate APIs. Rendering a graph never implies that the
//! original package can be reconstructed.

mod graph;
mod model;
mod reconstruction;

pub use graph::transform_document_graph;
pub use model::{
    GRAPH_TRANSFORM_RESULT_V1, GRAPH_TRANSFORM_SOURCE_MAP_V1, GraphTransformEnvelope,
    GraphTransformError, GraphTransformFidelity, GraphTransformLoss, GraphTransformLossKind,
    GraphTransformOptions, GraphTransformResult, GraphTransformSourceMap,
    GraphTransformSourceMapEntry, NormalizedGraphOperation, TransformMapStatus,
};
pub use reconstruction::{
    FORMAT_RECONSTRUCTION_RESULT_V1, FormatReconstructionClaim, FormatReconstructor,
    PackageByteRange, RECONSTRUCTION_FIDELITY_REPORT_V1, ReconstructionDifference,
    ReconstructionDifferenceKind, ReconstructionEnvelope, ReconstructionError,
    ReconstructionFidelity, ReconstructionFidelityReport, ReconstructionFixtureEvidence,
    ReconstructionOptions, ReconstructionProduct, ReconstructionResult,
    ReconstructionSourceMapEntry, reconstruct_package,
};

// Compatibility exports for parser-specific graph projections. These legacy
// options describe projection behavior, not the normalized operation envelope
// or package reconstruction contract above.
pub use crate::document_graph::{
    FromDocumentGraph, ToDocumentGraph, TransformError, TransformOptions, TransformWarning,
    TransformWarningKind,
};
