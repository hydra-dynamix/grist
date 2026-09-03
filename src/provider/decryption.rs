//! Decryption request, byte result, and adapter trait.

use super::{
    ProviderContractError, ProviderError, ProviderMetadata, ProviderRequest,
    ProviderRequestContext, ProviderRequestManifest, ProviderResult, parameters_digest,
};
use crate::core::{Diagnostic, Provider, ProviderKind, RawContentIdentity};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecryptionOptions {
    pub scheme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_media_type: Option<String>,
}

pub struct DecryptionRequest<'a> {
    pub context: ProviderRequestContext<'a>,
    pub options: DecryptionOptions,
}

impl<'a> DecryptionRequest<'a> {
    pub fn new(
        context: ProviderRequestContext<'a>,
        options: DecryptionOptions,
    ) -> Result<Self, ProviderContractError> {
        if context.secrets().is_empty() {
            return Err(ProviderContractError::InvalidField(
                "decryption.credentials",
            ));
        }
        if options.scheme.trim().is_empty() {
            return Err(ProviderContractError::InvalidField("decryption.scheme"));
        }
        Ok(Self { context, options })
    }

    /// Construct a request for an explicitly selected provider that owns its
    /// credentials internally. The provider binding remains runtime-only and
    /// neither the credential material nor its presence enters this request.
    pub fn with_provider_credentials(
        context: ProviderRequestContext<'a>,
        options: DecryptionOptions,
    ) -> Result<Self, ProviderContractError> {
        if options.scheme.trim().is_empty() {
            return Err(ProviderContractError::InvalidField("decryption.scheme"));
        }
        Ok(Self { context, options })
    }

    pub fn manifest(&self) -> Result<ProviderRequestManifest, ProviderContractError> {
        Ok(ProviderRequestManifest {
            kind: ProviderKind::Decryption,
            input_identity: self.context.input().identity().clone(),
            network_access: self.context.network_access(),
            configuration_digest: self.context.configuration_digest().into(),
            parameters_digest: parameters_digest(&self.options)?,
        })
    }
}

impl std::fmt::Debug for DecryptionRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecryptionRequest")
            .field("context", &self.context)
            .field("options", &self.options)
            .finish()
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecryptionResult {
    pub bytes: Vec<u8>,
    pub identity: RawContentIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl DecryptionResult {
    pub fn new(bytes: Vec<u8>, media_type: Option<impl Into<String>>) -> Self {
        let identity = RawContentIdentity::new(&bytes);
        Self {
            bytes,
            identity,
            media_type: media_type.map(Into::into),
            diagnostics: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.identity != RawContentIdentity::new(&self.bytes) {
            return Err(ProviderContractError::InvalidResult(
                "decryption output identity does not match bytes".into(),
            ));
        }
        Ok(())
    }
}

pub trait DecryptionProvider: Send + Sync + 'static {
    fn decrypt(&self, request: &DecryptionRequest<'_>) -> Result<DecryptionResult, ProviderError>;
}

pub struct DecryptionProviderAdapter<P> {
    metadata: ProviderMetadata,
    implementation: P,
}

impl<P> DecryptionProviderAdapter<P> {
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

impl<P: DecryptionProvider> Provider for DecryptionProviderAdapter<P> {
    fn name(&self) -> &str {
        &self.metadata.name
    }

    fn kind(&self) -> Option<ProviderKind> {
        Some(ProviderKind::Decryption)
    }

    fn metadata(&self) -> Option<&ProviderMetadata> {
        Some(&self.metadata)
    }

    fn invoke(&self, request: &ProviderRequest<'_>) -> Result<ProviderResult, ProviderError> {
        let ProviderRequest::Decryption(request) = request else {
            return Err(ProviderError::failure(
                &self.metadata.name,
                "decryption provider received a different request kind",
            ));
        };
        self.implementation
            .decrypt(request)
            .map(ProviderResult::Decryption)
    }
}
