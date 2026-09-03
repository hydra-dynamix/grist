//! Deterministic replay of captured provider results.

use super::{
    ProviderContractError, ProviderDeterminism, ProviderError, ProviderMetadata, ProviderRequest,
    ProviderRequestManifest, ProviderResult,
};
use crate::core::{NetworkAccess, Provider, ProviderKind, canonical_json_sha256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub const PROVIDER_RECORDING_CATALOG_SCHEMA_VERSION: &str = "grist/provider-recording-catalog/v1";

/// One replay key and output. Both identities are recomputed when loaded.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderRecordingEntry {
    pub request: ProviderRequestManifest,
    pub request_digest: String,
    pub output_sha256: String,
    pub result: ProviderResult,
}

/// Portable, secret-free recording file for OCR and transcription fixtures.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderRecordingCatalog {
    pub schema_version: String,
    pub kind: ProviderKind,
    pub provider: ProviderMetadata,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub entries: Vec<ProviderRecordingEntry>,
}

impl ProviderRecordingCatalog {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.schema_version != PROVIDER_RECORDING_CATALOG_SCHEMA_VERSION {
            return Err(ProviderContractError::InvalidField(
                "provider_recording.schema_version",
            ));
        }
        if self.kind == ProviderKind::Decryption {
            return Err(ProviderContractError::InvalidField(
                "recording.decryption_not_supported",
            ));
        }
        self.provider.validate()?;
        if self.provider.determinism != ProviderDeterminism::GuaranteedWithRecording {
            return Err(ProviderContractError::InvalidResult(
                "recording metadata must declare guaranteed_with_recording".into(),
            ));
        }
        if self.entries.is_empty() {
            return Err(ProviderContractError::InvalidResult(
                "recording catalog must contain at least one entry".into(),
            ));
        }
        let mut digests = BTreeMap::new();
        for entry in &self.entries {
            if entry.request.kind != self.kind {
                return Err(ProviderContractError::InvalidResult(
                    "recorded request kind does not match its catalog".into(),
                ));
            }
            if entry.result.kind() != self.kind {
                return Err(ProviderContractError::ProviderKindMismatch {
                    expected: self.kind,
                    actual: entry.result.kind(),
                });
            }
            if entry.request.network_access != NetworkAccess::Denied {
                return Err(ProviderContractError::InvalidResult(
                    "recordings must capture network-denied requests".into(),
                ));
            }
            entry.result.validate()?;
            if entry.result.has_confidence() && self.provider.confidence_model.is_none() {
                return Err(ProviderContractError::InvalidResult(
                    "recorded confidence requires named confidence_model metadata".into(),
                ));
            }
            let request_digest = entry.request.request_digest()?;
            if request_digest != entry.request_digest {
                return Err(ProviderContractError::InvalidResult(
                    "recorded request digest does not match its manifest".into(),
                ));
            }
            let output_sha256 = canonical_json_sha256(&entry.result)
                .map_err(|error| ProviderContractError::Digest(error.to_string()))?;
            if output_sha256 != entry.output_sha256 {
                return Err(ProviderContractError::InvalidResult(
                    "recorded output digest does not match its result".into(),
                ));
            }
            if digests.insert(request_digest.clone(), ()).is_some() {
                return Err(ProviderContractError::DuplicateRecording(request_digest));
            }
        }
        Ok(())
    }

    pub fn into_provider(self) -> Result<RecordedProvider, ProviderContractError> {
        self.validate()?;
        RecordedProvider::new(
            self.kind,
            self.provider,
            self.entries
                .into_iter()
                .map(|entry| (entry.request_digest, entry.result)),
        )
    }
}

pub struct RecordedProvider {
    kind: ProviderKind,
    metadata: ProviderMetadata,
    recordings: BTreeMap<String, ProviderResult>,
}

impl RecordedProvider {
    pub fn new(
        kind: ProviderKind,
        mut metadata: ProviderMetadata,
        recordings: impl IntoIterator<Item = (String, ProviderResult)>,
    ) -> Result<Self, ProviderContractError> {
        if kind == ProviderKind::Decryption {
            return Err(ProviderContractError::InvalidField(
                "recording.decryption_not_supported",
            ));
        }
        metadata.determinism = ProviderDeterminism::GuaranteedWithRecording;
        metadata.validate()?;
        let mut indexed = BTreeMap::new();
        for (request_digest, result) in recordings {
            result.validate()?;
            if result.kind() != kind {
                return Err(ProviderContractError::ProviderKindMismatch {
                    expected: kind,
                    actual: result.kind(),
                });
            }
            if indexed.insert(request_digest.clone(), result).is_some() {
                return Err(ProviderContractError::DuplicateRecording(request_digest));
            }
        }
        Ok(Self {
            kind,
            metadata,
            recordings: indexed,
        })
    }
}

impl Provider for RecordedProvider {
    fn name(&self) -> &str {
        &self.metadata.name
    }

    fn kind(&self) -> Option<ProviderKind> {
        Some(self.kind)
    }

    fn metadata(&self) -> Option<&ProviderMetadata> {
        Some(&self.metadata)
    }

    fn is_recorded(&self) -> bool {
        true
    }

    fn invoke(&self, request: &ProviderRequest<'_>) -> Result<ProviderResult, ProviderError> {
        let digest = request
            .manifest()
            .and_then(|manifest| manifest.request_digest())
            .map_err(|error| ProviderError::failure(&self.metadata.name, error.to_string()))?;
        self.recordings.get(&digest).cloned().ok_or_else(|| {
            ProviderError::failure(
                &self.metadata.name,
                ProviderContractError::MissingRecording(digest).to_string(),
            )
        })
    }
}
