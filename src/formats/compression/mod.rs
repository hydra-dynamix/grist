//! Stable namespace for compression-container parsing.
//!
//! Stream codecs and 7z use the same archive envelope and recursive container
//! controls as ZIP and TAR. This keeps budgets, cancellation, security policy,
//! content identity, document-graph projection, and CLI serialization uniform.

pub use crate::archive::{
    ArchiveDocument as CompressionDocument, ArchiveEnvelope as CompressionEnvelope,
    ArchiveFormat as CompressionFormat, ArchiveOptions as CompressionOptions,
    ArchiveParseError as CompressionParseError, archive_format as compression_format,
    parse_archive as parse_compression,
};
