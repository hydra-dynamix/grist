//! Shared identities, envelopes, source locations, diagnostics, and limits.

mod budget;
mod cancellation;
mod citation;
mod diagnostic;
mod envelope;
mod identity;
mod input;
mod locator;
mod options;
mod provenance;
mod provider;
mod request;
mod source;
mod streaming;

pub use budget::{
    BudgetAmount, BudgetAxis, BudgetExceeded, BudgetProfile, BudgetProfileDefinition,
    BudgetSelection, BudgetTracker, BudgetUsage, ResourceBudget, ResourceBudgetValidationError,
};
pub use cancellation::{
    CancellationError, CancellationToken, OperationControl, OperationControlError,
};
pub use citation::{
    CITATION_TEXT_NORMALIZATION_VERSION, CitationAnchor, CitationAnchorError,
    CitationAnchorOptions, CitationCandidate, CitationSourceVersion, CitationTargetKind,
    CitationVerification, CitationVerificationMethod, CitationVerificationOptions,
    CitationVerificationOutcome, MAX_CITATION_EXCERPT_CHARS, SourceContentHash, citation_label,
    normalize_citation_text, normalized_text_hash,
};

pub use diagnostic::{
    Diagnostic, DiagnosticCause, DiagnosticClass, DiagnosticCode, DiagnosticDetails,
    DiagnosticDetailsError, RecoveryAction, RecoveryKind, Severity,
};
pub use envelope::{
    Envelope, EnvelopeInvariantError, OperationKind, OperationStatus, PayloadKind,
    empty_options_digest, options_digest,
};
pub use identity::{
    AggregateContentIdentity, AggregateMemberIdentity, CanonicalJsonVersion,
    CanonicalPayloadIdentity, ContentIdentity, DecodedContentIdentity, DetectionCandidate,
    DetectionEvidence, DetectionEvidenceKind, FormatIdentity, ParserAvailability,
    RawContentIdentity, canonical_json_bytes, canonical_json_bytes_with_version,
    canonical_json_sha256,
};
pub use input::{
    CompoundMemberInput, Input, InputError, InputKind, InputOrigin, ReadSeek, ResolvedInput,
};
pub use locator::{
    BoundingBox, CellAddress, CoordinateOrigin, CoordinateUnit, DerivedNodeReference, IndexBase,
    IndexPosition, IndexRange, LineColumn, LineIndex, LocationComponent, LocatorConfidence,
    LocatorPrecision, SourceLocator, SourceLocatorError, SourceRange,
};
pub use options::{AutoFormatOptions, FormatOptions, ParseOptions, RecoveryMode};
pub use provenance::{
    DeclaredLoss, LossClass, MetadataInvariantError, ParserInfo, ProvenanceStep, ProviderInvocation,
};
pub use provider::{
    NetworkAccess, Provider, ProviderBinding, ProviderKind, ProviderSet, SecretBytes, SecretString,
};
pub use request::{FormatHint, ParseRequest, RequestId, RequestIdError, ResolvedParseRequest};
pub use source::SourceInfo;
pub use streaming::{
    BatchResult, OperationStream, StreamEvent, StreamItem, StreamProtocolError, StreamTerminal,
    collect_batch,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::PathBuf;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaVersion(pub String);

impl SchemaVersion {
    pub const METRIC_EVENT_V1: &'static str = "grist/metric-event/v1";
    pub const CACHE_KEY_V1: &'static str = "grist/cache-key/v1";
    pub const CACHE_ENTRY_V1: &'static str = "grist/cache-entry/v1";
    pub const CACHE_REUSE_V1: &'static str = "grist/cache-reuse/v1";
    pub const PARALLELISM_OPTIONS_V1: &'static str = "grist/parallelism-options/v1";
    pub const ENVELOPE_V1: &'static str = "grist/envelope/v1";
    pub const ENVELOPE_V2: &'static str = "grist/envelope/v2";
    pub const CONTENT_IDENTITY_V1: &'static str = "grist/content-identity/v1";
    pub const SOURCE_LOCATOR_V1: &'static str = "grist/source-locator/v1";
    pub const CITATION_ANCHOR_V1: &'static str = "grist/citation-anchor/v1";
    pub const CITATION_SOURCE_VERSION_V1: &'static str = "grist/citation-source-version/v1";
    pub const CITATION_VERIFICATION_V1: &'static str = "grist/citation-verification/v1";
    pub const DIAGNOSTIC_V1: &'static str = "grist/diagnostic/v1";
    pub const TEXT_DECODE_V1: &'static str = "grist/text-decode/v1";
    pub const RESOURCE_BUDGET_V1: &'static str = "grist/resource-budget/v1";
    pub const STREAM_EVENT_V1: &'static str = "grist/stream-event/v1";
    pub const BATCH_RESULT_V1: &'static str = "grist/batch-result/v1";
    pub const EMBEDDED_ARTIFACT_V1: &'static str = "grist/embedded-artifact/v1";
    pub const CONTAINER_TRAVERSAL_V1: &'static str = "grist/container-traversal/v1";
    pub const PROVIDER_RESPONSE_V1: &'static str = "grist/provider-response/v1";
    pub const PROVIDER_REQUEST_MANIFEST_V1: &'static str = "grist/provider-request-manifest/v1";
    pub const PROVIDER_RECORDING_CATALOG_V1: &'static str = "grist/provider-recording-catalog/v1";
    pub const FIXTURE_CORPUS_MANIFEST_V1: &'static str = "grist/fixture-corpus-manifest/v1";
    pub const CORPUS_VALIDATION_REPORT_V1: &'static str = "grist/corpus-validation-report/v1";
    pub const PARSER_CONFORMANCE_REPORT_V1: &'static str = "grist/parser-conformance-report/v1";
    pub const REGISTRY_SNAPSHOT_V1: &'static str = "grist/registry-snapshot/v1";
    pub const FILE_INGEST_V1: &'static str = "grist/file-ingest/v1";
    pub const MARKDOWN_V1: &'static str = "grist/markdown/v1";
    pub const MARKDOWN_V2: &'static str = "grist/markdown/v2";
    pub const RESTRUCTURED_TEXT_V1: &'static str = "grist/restructured-text/v1";
    pub const ASCIIDOC_V1: &'static str = "grist/asciidoc/v1";
    pub const HTML_V1: &'static str = "grist/html/v1";
    pub const HTML_V2: &'static str = "grist/html/v2";
    pub const EPUB_V1: &'static str = "grist/epub/v1";
    pub const PDF_V1: &'static str = "grist/pdf/v1";
    pub const WORD_OOXML_V1: &'static str = "grist/word-ooxml/v1";
    pub const PRESENTATION_OOXML_V1: &'static str = "grist/presentation-ooxml/v1";
    pub const SPREADSHEET_OOXML_V1: &'static str = "grist/spreadsheet-ooxml/v1";
    pub const SPREADSHEET_ODF_V1: &'static str = "grist/spreadsheet-odf/v1";
    pub const PRESENTATION_ODF_V1: &'static str = "grist/presentation-odf/v1";
    pub const ODF_WORD_V1: &'static str = "grist/odf-word/v1";
    pub const RTF_V1: &'static str = "grist/rtf/v1";
    pub const XML_V1: &'static str = "grist/xml/v1";
    pub const CSV_V1: &'static str = "grist/csv/v1";
    pub const CSV_V2: &'static str = "grist/csv/v2";
    pub const RUST_CODE_V1: &'static str = "grist/rust-code/v1";
    pub const PYTHON_CODE_V1: &'static str = "grist/python-code/v1";
    pub const JAVASCRIPT_CODE_V1: &'static str = "grist/javascript-code/v1";
    pub const TYPESCRIPT_CODE_V1: &'static str = "grist/typescript-code/v1";
    pub const CODE_V1: &'static str = "grist/code/v1";
    pub const MANIFEST_V1: &'static str = "grist/manifest/v1";
    pub const LATEX_V1: &'static str = "grist/latex/v1";
    pub const BIBLIOGRAPHY_V1: &'static str = "grist/bibliography/v1";
    pub const BIBLIOGRAPHY_CITATION_RESOLUTION_V1: &'static str =
        "grist/bibliography-citation-resolution/v1";
    pub const SERIALIZATION_V1: &'static str = "grist/serialization/v1";
    pub const STRUCTURED_TEXT_V2: &'static str = "grist/structured-text/v2";
    pub const STRUCTURED_BINARY_V1: &'static str = "grist/structured-binary/v1";
    pub const COLUMNAR_V1: &'static str = "grist/columnar/v1";
    pub const SQLITE_V1: &'static str = "grist/sqlite/v1";
    pub const ARCHIVE_V1: &'static str = "grist/archive/v1";
    pub const EMAIL_V1: &'static str = "grist/email/v1";
    pub const MBOX_V1: &'static str = "grist/mbox/v1";
    pub const OUTLOOK_MSG_V1: &'static str = "grist/outlook-msg/v1";
    pub const ICALENDAR_V1: &'static str = "grist/icalendar/v1";
    pub const VCARD_V1: &'static str = "grist/vcard/v1";
    pub const IPYNB_V1: &'static str = "grist/ipynb/v1";
    pub const MODEL_OUTPUT_V1: &'static str = "grist/model-output/v1";
    pub const REPO_INGEST_V1: &'static str = "grist/repo-ingest/v1";
    pub const LDGR_PROJECTION_V1: &'static str = "grist.ldgr_projection.v1";
    pub const DOCUMENT_GRAPH_V1: &'static str = "grist/document-graph/v1";
    pub const DOCUMENT_GRAPH_V2: &'static str = "grist/document-graph/v2";
    pub const RENDERED_SUMMARY_V1: &'static str = "grist/rendered-summary/v1";
    pub const TEXT_V1: &'static str = "grist/text/v1";
    pub const TEXT_V2: &'static str = "grist/text/v2";
    pub const SEGMENT_V1: &'static str = "grist/segment/v1";
    pub const SEGMENT_COLLECTION_V1: &'static str = "grist/segment-collection/v1";
    pub const SEGMENT_EVENT_V1: &'static str = "grist/segment-event/v1";
    pub const SEGMENT_OPTIONS_V1: &'static str = "grist/segment-options/v1";
    pub const RENDER_RESULT_V1: &'static str = "grist/render-result/v1";
    pub const RENDER_SOURCE_MAP_V1: &'static str = "grist/render-source-map/v1";
    pub const RENDER_OPTIONS_V1: &'static str = "grist/render-options/v1";
    pub const GRAPH_TRANSFORM_RESULT_V1: &'static str = "grist/graph-transform-result/v1";
    pub const GRAPH_TRANSFORM_SOURCE_MAP_V1: &'static str = "grist/graph-transform-source-map/v1";
    pub const GRAPH_TRANSFORM_OPTIONS_V1: &'static str = "grist/graph-transform-options/v1";
    pub const FORMAT_RECONSTRUCTION_RESULT_V1: &'static str =
        "grist/format-reconstruction-result/v1";
    pub const RECONSTRUCTION_FIDELITY_REPORT_V1: &'static str =
        "grist/reconstruction-fidelity-report/v1";
    pub const RECONSTRUCTION_OPTIONS_V1: &'static str = "grist/reconstruction-options/v1";
    pub const SCHEMA_CATALOG_V1: &'static str = "grist/schema-catalog/v1";
    pub const SCHEMA_VALIDATION_V1: &'static str = "grist/schema-validation/v1";
    pub const SCHEMA_MIGRATION_MANIFEST_V1: &'static str = "grist/schema-migration-manifest/v1";
    pub const CANONICAL_EXAMPLES_V1: &'static str = "grist/canonical-examples/v1";
    pub const BACKEND_OUTPUT_MANIFEST_V1: &'static str = "grist/backend-output-manifest/v1";
    pub const CLI_DETECTION_REPORT_V1: &'static str = "grist/cli-detection-report/v1";
    pub const CLI_TEXT_OUTPUT_MANIFEST_V1: &'static str = "grist/cli-text-output-manifest/v1";
    pub const CLI_CAPABILITIES_V1: &'static str = "grist/cli-capabilities/v1";
    pub const CAPABILITY_MANIFEST_V1: &'static str = "grist/capability-manifest/v1";
}

impl From<&str> for SchemaVersion {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Archive,
    Markdown,
    RestructuredText,
    AsciiDoc,
    Html,
    Epub,
    Pdf,
    WordOoxml,
    PresentationOoxml,
    SpreadsheetOoxml,
    SpreadsheetOdf,
    PresentationOdf,
    OdfWord,
    Rtf,
    Xml,
    Csv,
    RustCode,
    PythonCode,
    #[serde(rename = "javascript_code")]
    JavaScriptCode,
    #[serde(rename = "typescript_code")]
    TypeScriptCode,
    Code,
    Manifest,
    Latex,
    Bibliography,
    Serialization,
    StructuredBinary,
    Columnar,
    Sqlite,
    Email,
    Mbox,
    OutlookMsg,
    ICalendar,
    VCard,
    Notebook,
    ModelOutput,
    RepoIngest,
    FileIngest,
    Text,
    LdgrProjection,
    Detection,
    DocumentGraph,
    GraphTransformResult,
    RenderResult,
    ReconstructionResult,
    SegmentCollection,
    SchemaValidation,
    ContainerTraversal,
    Capabilities,
    Unsupported,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hashes {
    pub sha256: String,
    pub text_sha256: Option<String>,
    pub size_bytes: usize,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

impl Hashes {
    pub fn for_bytes(bytes: &[u8], text: Option<&str>) -> Self {
        Self {
            sha256: sha256_hex(bytes),
            text_sha256: text.map(|value| sha256_hex(value.as_bytes())),
            size_bytes: bytes.len(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: usize,
    pub max_repo_files: usize,
    pub max_model_output_bytes: usize,
    pub max_parse_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 64 * 1024 * 1024,
            max_repo_files: 250_000,
            max_model_output_bytes: 128 * 1024 * 1024,
            max_parse_depth: 4096,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SourceOptions {
    pub display_name: Option<String>,
    pub filename_hint: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticOptions {
    pub include_info: bool,
    pub include_details: bool,
}

impl Default for DiagnosticOptions {
    fn default() -> Self {
        Self {
            include_info: true,
            include_details: true,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SchemaOptions {
    pub schema_path: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum GristError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("UTF-8 decode error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Message(String),
}

impl From<String> for GristError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for GristError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
