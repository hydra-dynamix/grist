//! Deterministic, provenance-preserving document segmentation boundary.
//!
//! Segmentation consumes shared document contracts and remains independent of
//! retrieval, indexing, and ranking policy.

mod model;
mod structural;

pub use model::{
    AtomicityOptions, BoundaryKind, NodeSelectionRules, Segment, SegmentCollection, SegmentContext,
    SegmentCounts, SegmentEvent, SegmentNodeReference, SegmentNodeRole, SegmentOptions,
    SegmentOverlap, SegmentSizeUnit, TokenizerSpec,
};
pub use structural::{
    SegmentError, SegmentTokenizer, UnicodeWhitespaceTokenizer, segment_document_graph,
    segment_document_graph_parallel,
};
