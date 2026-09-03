//! OCR, transcription, decryption, and isolated-backend provider boundary.
//!
//! Providers are supplied and selected by the caller. Every request carries an
//! explicit network decision and a digest of non-secret configuration. Secrets
//! remain runtime-only. Responses keep provider results separate from native
//! extraction and record caller-supplied timing without consulting a clock.

mod backend;
mod common;
mod decryption;
mod ocr;
mod recording;
mod representation;
mod transcription;

pub use backend::{
    BackendPermission, IsolatedBackendOptions, IsolatedBackendRequest, IsolatedBackendResult,
    IsolatedParserBackend, IsolatedParserBackendAdapter, IsolationPolicy,
};
pub use common::{
    PROVIDER_RESPONSE_SCHEMA_VERSION, ProviderConfidence, ProviderContractError,
    ProviderDeterminism, ProviderError, ProviderExecutionMetadata, ProviderInput, ProviderMetadata,
    ProviderOutcome, ProviderRequestContext, ProviderRequestManifest, ProviderResponse,
    ProviderSecrets, ProviderTiming, SecretRef,
};
pub use decryption::{
    DecryptionOptions, DecryptionProvider, DecryptionProviderAdapter, DecryptionRequest,
    DecryptionResult,
};
pub use ocr::{OcrOptions, OcrProvider, OcrProviderAdapter, OcrRegion, OcrRequest, OcrResult};
pub use recording::{
    PROVIDER_RECORDING_CATALOG_SCHEMA_VERSION, ProviderRecordingCatalog, ProviderRecordingEntry,
    RecordedProvider,
};
pub use representation::{NativeRepresentation, ReconciledRepresentation, RepresentationSet};
pub use transcription::{
    TranscriptSegment, TranscriptionOptions, TranscriptionProvider, TranscriptionProviderAdapter,
    TranscriptionRequest, TranscriptionResult,
};

use crate::core::{Provider, ProviderKind, canonical_json_sha256};
use serde::{Deserialize, Serialize};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub(crate) use common::parameters_digest;

pub enum ProviderRequest<'a> {
    Ocr(OcrRequest<'a>),
    Transcription(TranscriptionRequest<'a>),
    Decryption(DecryptionRequest<'a>),
    IsolatedBackend(IsolatedBackendRequest<'a>),
}

impl ProviderRequest<'_> {
    pub const fn kind(&self) -> ProviderKind {
        match self {
            Self::Ocr(_) => ProviderKind::Ocr,
            Self::Transcription(_) => ProviderKind::Transcription,
            Self::Decryption(_) => ProviderKind::Decryption,
            Self::IsolatedBackend(_) => ProviderKind::IsolatedParserBackend,
        }
    }

    pub fn context(&self) -> &ProviderRequestContext<'_> {
        match self {
            Self::Ocr(request) => &request.context,
            Self::Transcription(request) => &request.context,
            Self::Decryption(request) => &request.context,
            Self::IsolatedBackend(request) => &request.context,
        }
    }

    pub fn manifest(&self) -> Result<ProviderRequestManifest, ProviderContractError> {
        match self {
            Self::Ocr(request) => request.manifest(),
            Self::Transcription(request) => request.manifest(),
            Self::Decryption(request) => request.manifest(),
            Self::IsolatedBackend(request) => request.manifest(),
        }
    }
}

impl std::fmt::Debug for ProviderRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderRequest")
            .field("kind", &self.kind())
            .field("context", self.context())
            .finish_non_exhaustive()
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum ProviderResult {
    Ocr(OcrResult),
    Transcription(TranscriptionResult),
    Decryption(DecryptionResult),
    IsolatedBackend(IsolatedBackendResult),
}

impl ProviderResult {
    pub const fn kind(&self) -> ProviderKind {
        match self {
            Self::Ocr(_) => ProviderKind::Ocr,
            Self::Transcription(_) => ProviderKind::Transcription,
            Self::Decryption(_) => ProviderKind::Decryption,
            Self::IsolatedBackend(_) => ProviderKind::IsolatedParserBackend,
        }
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        match self {
            Self::Ocr(result) => result.validate(),
            Self::Transcription(result) => result.validate(),
            Self::Decryption(result) => result.validate(),
            Self::IsolatedBackend(result) => result.validate(),
        }
    }

    pub(crate) fn has_confidence(&self) -> bool {
        match self {
            Self::Ocr(result) => {
                result.confidence.is_some()
                    || result.reading_order_confidence.is_some()
                    || result.layout_confidence.is_some()
                    || result
                        .regions
                        .iter()
                        .any(|region| region.confidence.is_some())
            }
            Self::Transcription(result) => {
                result.confidence.is_some()
                    || result
                        .segments
                        .iter()
                        .any(|segment| segment.confidence.is_some())
            }
            Self::Decryption(_) | Self::IsolatedBackend(_) => false,
        }
    }

    pub(crate) fn diagnostics(&self) -> Vec<crate::core::Diagnostic> {
        match self {
            Self::Ocr(result) => result.diagnostics.clone(),
            Self::Transcription(result) => result.diagnostics.clone(),
            Self::Decryption(result) => result.diagnostics.clone(),
            Self::IsolatedBackend(result) => result.diagnostics.clone(),
        }
    }
}

/// Invoke a provider without adding clocks, network permission, or secrets.
/// Bindings call this after checking explicit per-request selection.
pub fn execute_provider(
    provider: &dyn Provider,
    request: &ProviderRequest<'_>,
) -> Result<ProviderResponse, ProviderContractError> {
    if provider.kind() != Some(request.kind()) {
        return Err(ProviderContractError::ProviderDoesNotImplementContract);
    }
    let metadata = provider
        .metadata()
        .ok_or(ProviderContractError::ProviderDoesNotImplementContract)?;
    metadata.validate()?;
    let redactor = request.context().secrets().redactor();
    if redactor
        .contains_serialized(metadata)
        .map_err(|error| ProviderContractError::Digest(error.to_string()))?
    {
        return Err(ProviderContractError::SensitiveOutput);
    }
    let manifest = request.manifest()?;
    let request_digest = manifest.request_digest()?;
    let context = request.context();
    let base_metadata = |output_identity, diagnostics| ProviderExecutionMetadata {
        provider: metadata.clone(),
        kind: request.kind(),
        request_digest: request_digest.clone(),
        configuration_digest: context.configuration_digest().into(),
        input_identity: context.input().identity().clone(),
        output_identity,
        network_access: context.network_access(),
        timing: context.caller_timing().cloned(),
        recorded: provider.is_recorded(),
        diagnostics,
    };

    match catch_unwind(AssertUnwindSafe(|| provider.invoke(request))) {
        Ok(Ok(result)) => {
            if result.kind() != request.kind() {
                return Err(ProviderContractError::ProviderKindMismatch {
                    expected: request.kind(),
                    actual: result.kind(),
                });
            }
            result.validate()?;
            if redactor
                .contains_serialized(&result)
                .map_err(|error| ProviderContractError::Digest(error.to_string()))?
            {
                return ProviderResponse::failed(base_metadata(
                    None,
                    vec![crate::core::Diagnostic::security_rejection(
                        metadata.name.clone(),
                        "provider result was rejected because it contained sensitive request material",
                    )],
                ));
            }
            if result.has_confidence() && metadata.confidence_model.is_none() {
                return Err(ProviderContractError::InvalidResult(
                    "confidence values require named confidence_model metadata".into(),
                ));
            }
            let output_identity = canonical_json_sha256(&result)
                .map_err(|error| ProviderContractError::Digest(error.to_string()))?;
            let diagnostics = result.diagnostics();
            ProviderResponse::succeeded(base_metadata(Some(output_identity), diagnostics), result)
        }
        Ok(Err(error)) => {
            let diagnostic = if redactor
                .contains_serialized(error.diagnostic.as_ref())
                .map_err(|source| ProviderContractError::Digest(source.to_string()))?
            {
                crate::core::Diagnostic::provider_failure(
                    metadata.name.clone(),
                    "provider failure contained redacted sensitive material",
                )
            } else {
                *error.diagnostic
            };
            ProviderResponse::failed(base_metadata(None, vec![diagnostic]))
        }
        Err(_) => ProviderResponse::failed(base_metadata(
            None,
            vec![crate::core::Diagnostic::parser_defect(
                metadata.name.clone(),
                "provider panicked inside the public execution boundary",
            )],
        )),
    }
}
