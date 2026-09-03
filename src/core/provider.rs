//! Explicit, runtime-only provider selection and secret values.

use crate::provider::{
    ProviderContractError, ProviderMetadata, ProviderRequest, ProviderResponse, execute_provider,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use zeroize::Zeroizing;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Provider capabilities selectable by a parse request.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Ocr,
    Transcription,
    Decryption,
    IsolatedParserBackend,
}

/// Whether a selected provider may access the network for this request.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    Denied,
    Allowed,
}

/// Type-erased execution boundary for one caller-supplied provider.
///
/// Capability-specific adapters in crate::provider implement this trait, so
/// parsers retain one runtime-only ProviderSet without erasing typed contracts.
pub trait Provider: Send + Sync + 'static {
    fn name(&self) -> &str;

    fn kind(&self) -> Option<ProviderKind> {
        None
    }

    fn metadata(&self) -> Option<&ProviderMetadata> {
        None
    }

    fn invoke(
        &self,
        _request: &ProviderRequest<'_>,
    ) -> Result<crate::provider::ProviderResult, crate::provider::ProviderError> {
        Err(crate::provider::ProviderError::failure(
            self.name(),
            "provider does not implement the typed execution contract",
        ))
    }

    fn is_recorded(&self) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct ProviderBinding {
    provider: Arc<dyn Provider>,
    network_access: NetworkAccess,
}

impl ProviderBinding {
    pub fn new(provider: Arc<dyn Provider>, network_access: NetworkAccess) -> Self {
        Self {
            provider,
            network_access,
        }
    }

    pub fn provider(&self) -> &dyn Provider {
        self.provider.as_ref()
    }

    pub fn network_access(&self) -> NetworkAccess {
        self.network_access
    }

    /// Execute only when the request kind and per-request network permission
    /// exactly match this explicitly selected binding.
    pub fn invoke(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ProviderContractError> {
        if self.provider.kind() != Some(request.kind()) {
            return Err(ProviderContractError::ProviderDoesNotImplementContract);
        }
        if request.context().network_access() != self.network_access {
            return Err(ProviderContractError::NetworkPermissionMismatch);
        }
        execute_provider(self.provider.as_ref(), request)
    }
}

impl fmt::Debug for ProviderBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderBinding")
            .field("provider", &self.provider.name())
            .field("network_access", &self.network_access)
            .finish()
    }
}

/// Providers explicitly selected for one request.
///
/// This runtime container deliberately does not implement serde traits: trait
/// objects, credentials, and provider-local configuration must never enter a
/// request JSON representation.
#[derive(Clone, Default)]
pub struct ProviderSet {
    bindings: BTreeMap<ProviderKind, ProviderBinding>,
}

impl ProviderSet {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn select(
        &mut self,
        kind: ProviderKind,
        provider: Arc<dyn Provider>,
        network_access: NetworkAccess,
    ) -> Option<ProviderBinding> {
        self.bindings
            .insert(kind, ProviderBinding::new(provider, network_access))
    }

    pub fn selected(&self, kind: ProviderKind) -> Option<&ProviderBinding> {
        self.bindings.get(&kind)
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Invoke a provider only after explicit selection for this request.
    pub fn invoke(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ProviderContractError> {
        let kind = request.kind();
        let binding = self
            .selected(kind)
            .ok_or(ProviderContractError::ProviderNotSelected(kind))?;
        binding.invoke(request)
    }
}

impl fmt::Debug for ProviderSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_map().entries(self.bindings.iter()).finish()
    }
}

/// An in-memory UTF-8 secret excluded from serialization, hashing, cloning,
/// and logs. Its allocation is overwritten before release.
pub struct SecretString(Zeroizing<Box<[u8]>>);

impl SecretString {
    pub fn new(secret: impl Into<Box<str>>) -> Self {
        let secret: Box<str> = secret.into();
        Self(Zeroizing::new(
            String::from(secret).into_bytes().into_boxed_slice(),
        ))
    }

    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.0).expect("SecretString is constructed from valid UTF-8")
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString(<redacted>)")
    }
}

/// An in-memory binary secret with the same non-serializable, non-debuggable,
/// zero-on-drop behavior as SecretString.
pub struct SecretBytes(Zeroizing<Box<[u8]>>);

impl SecretBytes {
    pub fn new(secret: impl Into<Box<[u8]>>) -> Self {
        Self(Zeroizing::new(secret.into()))
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretBytes(<redacted>)")
    }
}
