//! File, byte-stream, batch, and deterministic repository ingestion.

mod batch;
mod engine;
mod file;
mod repository;

pub use batch::{IngestBatchResult, IngestStream, IngestStreamEvent};
pub use engine::{IngestError, Ingestor};
pub use file::{
    FileIngestEnvelope, FileIngestOptions, FileIngestReport, ingest_bytes, ingest_path,
};
pub use repository::{
    FileArtifact, FileInventoryEntry, RepoAggregateHashes, RepoIngestEnvelope, RepoIngestOptions,
    RepoIngestOptionsSummary, RepoIngestReport, RepositoryDisposition, RepositoryEntry,
    RepositoryEntryKind, RepositorySkipReason, SkippedFile, SubmodulePolicy, SymlinkPolicy,
    TestHint, ingest_repo,
};
