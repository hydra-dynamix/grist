//! Shared request metadata, confidence, timing, response, and secret references.

use super::ProviderResult;
use crate::core::{
    Diagnostic, NetworkAccess, ProviderInvocation, ProviderKind, RawContentIdentity, SecretBytes,
    SecretString, canonical_json_sha256,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[cfg(feature = "schemas")]
use schemars::{
    JsonSchema,
    schema::{InstanceType, NumberValidation, Schema, SchemaObject, SingleOrVec},
};

pub const PROVIDER_RESPONSE_SCHEMA_VERSION: &str = "grist/provider-response/v1";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDeterminism {
    Guaranteed,
    GuaranteedWithRecording,
    NotGuaranteed,
}

impl ProviderDeterminism {
    pub const fn deterministic_for_record(self, recorded: bool) -> bool {
        match self {
            Self::Guaranteed => true,
            Self::GuaranteedWithRecording => recorded,
            Self::NotGuaranteed => false,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderMetadata {
    pub name: String,
    pub implementation: String,
    pub implementation_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
    pub determinism: ProviderDeterminism,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_model: Option<String>,
}

impl ProviderMetadata {
    pub fn new(
        name: impl Into<String>,
        implementation: impl Into<String>,
        implementation_version: impl Into<String>,
        determinism: ProviderDeterminism,
    ) -> Result<Self, ProviderContractError> {
        let metadata = Self {
            name: name.into(),
            implementation: implementation.into(),
            implementation_version: implementation_version.into(),
            model_version: None,
            determinism,
            confidence_model: None,
        };
        metadata.validate()?;
        Ok(metadata)
    }

    pub fn with_model_version(mut self, version: impl Into<String>) -> Self {
        self.model_version = Some(version.into());
        self
    }

    pub fn with_confidence_model(mut self, model: impl Into<String>) -> Self {
        self.confidence_model = Some(model.into());
        self
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        for (field, value) in [
            ("name", self.name.as_str()),
            ("implementation", self.implementation.as_str()),
            (
                "implementation_version",
                self.implementation_version.as_str(),
            ),
        ] {
            validate_label(field, value)?;
        }
        if let Some(value) = self.model_version.as_deref() {
            validate_label("model_version", value)?;
        }
        if let Some(value) = self.confidence_model.as_deref() {
            validate_label("confidence_model", value)?;
        }
        Ok(())
    }

    pub fn effective_implementation(&self) -> String {
        format!("{}@{}", self.implementation, self.implementation_version)
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderTiming {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_millis: Option<u64>,
    pub source: String,
}

impl ProviderTiming {
    pub fn caller(
        source: impl Into<String>,
        started_at: Option<impl Into<String>>,
        elapsed_millis: Option<u64>,
    ) -> Result<Self, ProviderContractError> {
        let timing = Self {
            started_at: started_at.map(Into::into),
            elapsed_millis,
            source: source.into(),
        };
        timing.validate()?;
        Ok(timing)
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        validate_label("timing.source", &self.source)?;
        if self.started_at.as_deref() == Some("") {
            return Err(ProviderContractError::InvalidField("timing.started_at"));
        }
        if self.started_at.is_none() && self.elapsed_millis.is_none() {
            return Err(ProviderContractError::InvalidField("timing"));
        }
        Ok(())
    }
}

/// Normalized provider confidence in the inclusive range zero through one.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(transparent)]
pub struct ProviderConfidence(f64);

impl ProviderConfidence {
    pub fn new(value: f64) -> Result<Self, ProviderContractError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ProviderContractError::InvalidConfidence(value))
        }
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for ProviderConfidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "schemas")]
impl JsonSchema for ProviderConfidence {
    fn schema_name() -> String {
        "ProviderConfidence".into()
    }

    fn json_schema(_generator: &mut schemars::r#gen::SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Number))),
            number: Some(Box::new(NumberValidation {
                minimum: Some(0.0),
                maximum: Some(1.0),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// Raw provider input. Debug text contains only its identity, never content.
pub struct ProviderInput<'a> {
    bytes: &'a [u8],
    identity: RawContentIdentity,
}

impl<'a> ProviderInput<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            identity: RawContentIdentity::new(bytes),
        }
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn identity(&self) -> &RawContentIdentity {
        &self.identity
    }
}

impl fmt::Debug for ProviderInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderInput")
            .field("identity", &self.identity)
            .finish()
    }
}

pub enum SecretRef<'a> {
    Text(&'a SecretString),
    Bytes(&'a SecretBytes),
}

impl SecretRef<'_> {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.expose()),
            Self::Bytes(_) => None,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Text(value) => value.expose().as_bytes(),
            Self::Bytes(value) => value.expose(),
        }
    }
}

/// Runtime-only secret references. Names, values, and presence are excluded
/// from serialization, request digests, and debug output.
#[derive(Default)]
pub struct ProviderSecrets<'a> {
    values: BTreeMap<String, SecretRef<'a>>,
}

impl<'a> ProviderSecrets<'a> {
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        value: SecretRef<'a>,
    ) -> Result<(), ProviderContractError> {
        let name = name.into();
        validate_label("secret.name", &name)?;
        self.values.insert(name, value);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&SecretRef<'a>> {
        self.values.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub(crate) fn redactor(&self) -> crate::security::SecretRedactor<'_> {
        crate::security::SecretRedactor::new(self.values.values().map(SecretRef::bytes))
    }
}

impl fmt::Debug for ProviderSecrets<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderSecrets(<redacted>)")
    }
}

/// Shared runtime context for every provider request.
pub struct ProviderRequestContext<'a> {
    input: ProviderInput<'a>,
    network_access: NetworkAccess,
    configuration_digest: String,
    caller_timing: Option<ProviderTiming>,
    secrets: ProviderSecrets<'a>,
}

impl<'a> ProviderRequestContext<'a> {
    pub fn new<T: Serialize + ?Sized>(
        bytes: &'a [u8],
        network_access: NetworkAccess,
        public_configuration: &T,
    ) -> Result<Self, ProviderContractError> {
        Ok(Self {
            input: ProviderInput::new(bytes),
            network_access,
            configuration_digest: canonical_json_sha256(public_configuration)
                .map_err(|error| ProviderContractError::Digest(error.to_string()))?,
            caller_timing: None,
            secrets: ProviderSecrets::default(),
        })
    }

    pub fn with_timing(mut self, timing: ProviderTiming) -> Self {
        self.caller_timing = Some(timing);
        self
    }

    pub fn with_text_secret(
        mut self,
        name: impl Into<String>,
        secret: &'a SecretString,
    ) -> Result<Self, ProviderContractError> {
        self.secrets.insert(name, SecretRef::Text(secret))?;
        Ok(self)
    }

    pub fn with_binary_secret(
        mut self,
        name: impl Into<String>,
        secret: &'a SecretBytes,
    ) -> Result<Self, ProviderContractError> {
        self.secrets.insert(name, SecretRef::Bytes(secret))?;
        Ok(self)
    }

    pub fn input(&self) -> &ProviderInput<'a> {
        &self.input
    }

    pub const fn network_access(&self) -> NetworkAccess {
        self.network_access
    }

    pub fn configuration_digest(&self) -> &str {
        &self.configuration_digest
    }

    pub fn caller_timing(&self) -> Option<&ProviderTiming> {
        self.caller_timing.as_ref()
    }

    pub fn secrets(&self) -> &ProviderSecrets<'a> {
        &self.secrets
    }
}

impl fmt::Debug for ProviderRequestContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRequestContext")
            .field("input", &self.input)
            .field("network_access", &self.network_access)
            .field("configuration_digest", &self.configuration_digest)
            .field("caller_timing", &self.caller_timing)
            .field("secrets", &self.secrets)
            .finish()
    }
}

/// Serializable request metadata. Input bytes and every secret are excluded.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRequestManifest {
    pub kind: ProviderKind,
    pub input_identity: RawContentIdentity,
    pub network_access: NetworkAccess,
    pub configuration_digest: String,
    pub parameters_digest: String,
}

impl ProviderRequestManifest {
    pub fn request_digest(&self) -> Result<String, ProviderContractError> {
        canonical_json_sha256(self)
            .map_err(|error| ProviderContractError::Digest(error.to_string()))
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum ProviderContractError {
    #[error("invalid provider field: {0}")]
    InvalidField(&'static str),
    #[error("provider confidence must be finite and in the inclusive range zero through one: {0}")]
    InvalidConfidence(f64),
    #[error("provider digest failed: {0}")]
    Digest(String),
    #[error("provider kind mismatch: expected {expected:?}, got {actual:?}")]
    ProviderKindMismatch {
        expected: ProviderKind,
        actual: ProviderKind,
    },
    #[error("provider was not explicitly selected for {0:?}")]
    ProviderNotSelected(ProviderKind),
    #[error("request network permission does not match the selected binding")]
    NetworkPermissionMismatch,
    #[error("provider does not implement the typed execution contract")]
    ProviderDoesNotImplementContract,
    #[error("isolated backend invocation requires explicit permission")]
    BackendPermissionDenied,
    #[error("provider output contained sensitive request material")]
    SensitiveOutput,
    #[error("invalid provider result: {0}")]
    InvalidResult(String),
    #[error("duplicate provider recording: {0}")]
    DuplicateRecording(String),
    #[error("recorded provider has no response for request digest {0}")]
    MissingRecording(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderError {
    pub diagnostic: Box<Diagnostic>,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.diagnostic.message)
    }
}

impl std::error::Error for ProviderError {}

impl ProviderError {
    pub fn new(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostic: Box::new(diagnostic),
        }
    }

    pub fn failure(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Diagnostic::provider_failure(provider, message))
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderExecutionMetadata {
    pub provider: ProviderMetadata,
    pub kind: ProviderKind,
    pub request_digest: String,
    pub configuration_digest: String,
    pub input_identity: RawContentIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_identity: Option<String>,
    pub network_access: NetworkAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<ProviderTiming>,
    pub recorded: bool,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl ProviderExecutionMetadata {
    pub fn envelope_invocation(&self) -> ProviderInvocation {
        let mut invocation = ProviderInvocation::new(
            self.provider.name.clone(),
            self.provider.effective_implementation(),
            self.configuration_digest.clone(),
        )
        .expect("validated provider execution metadata has a valid digest");
        invocation.model_version = self.provider.model_version.clone();
        invocation.input_identity = Some(self.input_identity.sha256.clone());
        invocation.output_identity = self.output_identity.clone();
        invocation.confidence_model = self.provider.confidence_model.clone();
        invocation.deterministic = Some(
            self.provider
                .determinism
                .deterministic_for_record(self.recorded),
        );
        invocation.diagnostics = self.diagnostics.clone();
        if let Some(timing) = &self.timing {
            invocation.timing = Some(
                serde_json::to_string(timing).expect("validated provider timing always serializes"),
            );
        }
        invocation
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProviderOutcome {
    Succeeded { result: ProviderResult },
    Failed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderResponse {
    pub schema_version: String,
    pub metadata: ProviderExecutionMetadata,
    pub outcome: ProviderOutcome,
}

#[derive(Deserialize)]
struct ProviderResponseWire {
    schema_version: String,
    metadata: ProviderExecutionMetadata,
    outcome: ProviderOutcome,
}

impl<'de> Deserialize<'de> for ProviderResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProviderResponseWire::deserialize(deserializer)?;
        let response = Self {
            schema_version: wire.schema_version,
            metadata: wire.metadata,
            outcome: wire.outcome,
        };
        response.validate().map_err(serde::de::Error::custom)?;
        Ok(response)
    }
}

impl ProviderResponse {
    pub fn succeeded(
        metadata: ProviderExecutionMetadata,
        result: ProviderResult,
    ) -> Result<Self, ProviderContractError> {
        result.validate()?;
        if metadata.output_identity.is_none() {
            return Err(ProviderContractError::InvalidResult(
                "successful response requires output identity".into(),
            ));
        }
        let response = Self {
            schema_version: PROVIDER_RESPONSE_SCHEMA_VERSION.into(),
            metadata,
            outcome: ProviderOutcome::Succeeded { result },
        };
        response.validate()?;
        Ok(response)
    }

    pub fn failed(metadata: ProviderExecutionMetadata) -> Result<Self, ProviderContractError> {
        if metadata.output_identity.is_some() || metadata.diagnostics.is_empty() {
            return Err(ProviderContractError::InvalidResult(
                "failed response requires diagnostics and no output identity".into(),
            ));
        }
        let response = Self {
            schema_version: PROVIDER_RESPONSE_SCHEMA_VERSION.into(),
            metadata,
            outcome: ProviderOutcome::Failed,
        };
        response.validate()?;
        Ok(response)
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.schema_version != PROVIDER_RESPONSE_SCHEMA_VERSION {
            return Err(ProviderContractError::InvalidField(
                "provider_response.schema_version",
            ));
        }
        self.metadata.provider.validate()?;
        validate_digest("request_digest", &self.metadata.request_digest)?;
        validate_digest("configuration_digest", &self.metadata.configuration_digest)?;
        validate_digest("input_identity", &self.metadata.input_identity.sha256)?;
        if let Some(timing) = &self.metadata.timing {
            timing.validate()?;
        }
        if self.metadata.recorded
            && self.metadata.provider.determinism != ProviderDeterminism::GuaranteedWithRecording
        {
            return Err(ProviderContractError::InvalidResult(
                "recorded response must declare guaranteed_with_recording".into(),
            ));
        }
        match &self.outcome {
            ProviderOutcome::Succeeded { result } => {
                result.validate()?;
                if result.kind() != self.metadata.kind {
                    return Err(ProviderContractError::ProviderKindMismatch {
                        expected: self.metadata.kind,
                        actual: result.kind(),
                    });
                }
                if result.has_confidence() && self.metadata.provider.confidence_model.is_none() {
                    return Err(ProviderContractError::InvalidResult(
                        "confidence values require named confidence_model metadata".into(),
                    ));
                }
                let expected = canonical_json_sha256(result)
                    .map_err(|error| ProviderContractError::Digest(error.to_string()))?;
                if self.metadata.output_identity.as_deref() != Some(expected.as_str()) {
                    return Err(ProviderContractError::InvalidResult(
                        "provider output identity does not match result".into(),
                    ));
                }
                if self.metadata.diagnostics != result.diagnostics() {
                    return Err(ProviderContractError::InvalidResult(
                        "provider result diagnostics do not match invocation metadata".into(),
                    ));
                }
            }
            ProviderOutcome::Failed => {
                if self.metadata.output_identity.is_some() || self.metadata.diagnostics.is_empty() {
                    return Err(ProviderContractError::InvalidResult(
                        "failed response requires diagnostics and no output identity".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn result(&self) -> Option<&ProviderResult> {
        match &self.outcome {
            ProviderOutcome::Succeeded { result } => Some(result),
            ProviderOutcome::Failed => None,
        }
    }

    pub fn is_success(&self) -> bool {
        self.result().is_some()
    }

    pub fn envelope_invocation(&self) -> ProviderInvocation {
        self.metadata.envelope_invocation()
    }
}

pub(crate) fn parameters_digest<T: Serialize + ?Sized>(
    parameters: &T,
) -> Result<String, ProviderContractError> {
    canonical_json_sha256(parameters)
        .map_err(|error| ProviderContractError::Digest(error.to_string()))
}

pub(crate) fn validate_digest(
    field: &'static str,
    value: &str,
) -> Result<(), ProviderContractError> {
    let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if valid {
        Ok(())
    } else {
        Err(ProviderContractError::InvalidField(field))
    }
}

pub(crate) fn validate_label(
    field: &'static str,
    value: &str,
) -> Result<(), ProviderContractError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(ProviderContractError::InvalidField(field))
    } else {
        Ok(())
    }
}
