//! Speech-transcription request, timed result, and adapter trait.

use super::{
    ProviderConfidence, ProviderContractError, ProviderError, ProviderMetadata, ProviderRequest,
    ProviderRequestContext, ProviderRequestManifest, ProviderResult, parameters_digest,
};
use crate::core::{Diagnostic, Provider, ProviderKind};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptionOptions {
    #[serde(default)]
    pub language_hints: Vec<String>,
    #[serde(default)]
    pub speaker_diarization: bool,
    #[serde(default)]
    pub word_timestamps: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
}

pub struct TranscriptionRequest<'a> {
    pub context: ProviderRequestContext<'a>,
    pub options: TranscriptionOptions,
}

impl<'a> TranscriptionRequest<'a> {
    pub fn new(context: ProviderRequestContext<'a>, options: TranscriptionOptions) -> Self {
        Self { context, options }
    }

    pub fn manifest(&self) -> Result<ProviderRequestManifest, ProviderContractError> {
        Ok(ProviderRequestManifest {
            kind: ProviderKind::Transcription,
            input_identity: self.context.input().identity().clone(),
            network_access: self.context.network_access(),
            configuration_digest: self.context.configuration_digest().into(),
            parameters_digest: parameters_digest(&self.options)?,
        })
    }
}

impl std::fmt::Debug for TranscriptionRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TranscriptionRequest")
            .field("context", &self.context)
            .field("options", &self.options)
            .finish()
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TranscriptionResult {
    pub text: String,
    #[serde(default)]
    pub segments: Vec<TranscriptSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ProviderConfidence>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl TranscriptionResult {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        let mut previous_start = 0;
        for (index, segment) in self.segments.iter().enumerate() {
            if segment.end_ms < segment.start_ms {
                return Err(ProviderContractError::InvalidResult(format!(
                    "transcript segment {index} ends before it starts"
                )));
            }
            if index > 0 && segment.start_ms < previous_start {
                return Err(ProviderContractError::InvalidResult(
                    "transcript segments must be in stable start-time order".into(),
                ));
            }
            previous_start = segment.start_ms;
        }
        Ok(())
    }
}

pub trait TranscriptionProvider: Send + Sync + 'static {
    fn transcribe(
        &self,
        request: &TranscriptionRequest<'_>,
    ) -> Result<TranscriptionResult, ProviderError>;
}

pub struct TranscriptionProviderAdapter<P> {
    metadata: ProviderMetadata,
    implementation: P,
}

impl<P> TranscriptionProviderAdapter<P> {
    pub fn new(
        metadata: ProviderMetadata,
        implementation: P,
    ) -> Result<Self, ProviderContractError> {
        metadata.validate()?;
        Ok(Self {
            metadata,
            implementation,
        })
    }
}

impl<P: TranscriptionProvider> Provider for TranscriptionProviderAdapter<P> {
    fn name(&self) -> &str {
        &self.metadata.name
    }

    fn kind(&self) -> Option<ProviderKind> {
        Some(ProviderKind::Transcription)
    }

    fn metadata(&self) -> Option<&ProviderMetadata> {
        Some(&self.metadata)
    }

    fn invoke(&self, request: &ProviderRequest<'_>) -> Result<ProviderResult, ProviderError> {
        let ProviderRequest::Transcription(request) = request else {
            return Err(ProviderError::failure(
                &self.metadata.name,
                "transcription provider received a different request kind",
            ));
        };
        self.implementation
            .transcribe(request)
            .map(ProviderResult::Transcription)
    }
}
