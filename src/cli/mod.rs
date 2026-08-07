//! Thin command-line adapter ownership boundary.
//!
//! The `grist` binary is enabled by the `cli` feature and delegates parsing,
//! ingestion, schemas, transforms, segmentation, validation, and rendering to
//! public library APIs. This module owns only CLI wire models and adapter
//! helpers; format parsing remains in the registry and format modules.

mod model;
mod operations;

pub use crate::capabilities::CapabilityManifest as CliCapabilities;
pub use model::{DetectionReport, OutputDestination, TextOutputManifest};
pub use operations::{
    InputHints, capabilities, default_budget, detect_input, graph_input, output_manifest,
    parse_bytes, parse_input, path_label, project_envelope_to_graph, read_input_bytes,
};

pub type CliResult<T> = Result<T, Box<dyn std::error::Error>>;
