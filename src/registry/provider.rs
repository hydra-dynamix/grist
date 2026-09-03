use super::{NetworkPolicy, ParserOrigin, ProviderDescriptor, RegistrySnapshot};
use crate::core::{NetworkAccess, Provider, ProviderBinding, ProviderKind, ProviderSet};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSelection {
    Available(ProviderDescriptor),
    Unsupported(ProviderKind),
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ProviderRegistryError {
    #[error("invalid provider registration: {0}")]
    Invalid(String),
    #[error("duplicate provider registration: {0}")]
    DuplicateId(String),
    #[error("provider kind already has an implementation at this priority: {0:?}")]
    PriorityConflict(ProviderKind),
    #[error("registration must have caller origin: {0}")]
    InvalidCallerOrigin(String),
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("provider kind does not match the requested capability")]
    KindMismatch,
    #[error("provider registration forbids network access")]
    NetworkDenied,
    #[error("provider registration requires explicit network access")]
    NetworkRequired,
}

struct ProviderEntry {
    descriptor: ProviderDescriptor,
    provider: Arc<dyn Provider>,
}

#[derive(Default)]
pub struct ProviderRegistry {
    entries: BTreeMap<String, ProviderEntry>,
}

/// Built-in providers are intentionally empty: Grist never selects a network,
/// model, decryption, or native backend implicitly.
pub fn builtin_provider_registry() -> ProviderRegistry {
    ProviderRegistry::empty()
}

impl ProviderRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn register_caller(
        &mut self,
        descriptor: ProviderDescriptor,
        provider: Arc<dyn Provider>,
    ) -> Result<(), ProviderRegistryError> {
        if descriptor.origin != ParserOrigin::Caller {
            return Err(ProviderRegistryError::InvalidCallerOrigin(descriptor.id));
        }
        self.register(descriptor, provider)
    }

    fn register(
        &mut self,
        descriptor: ProviderDescriptor,
        provider: Arc<dyn Provider>,
    ) -> Result<(), ProviderRegistryError> {
        validate_provider(&descriptor, provider.as_ref())?;
        if self.entries.contains_key(&descriptor.id) {
            return Err(ProviderRegistryError::DuplicateId(descriptor.id));
        }
        if self.entries.values().any(|entry| {
            entry.descriptor.kind == descriptor.kind
                && entry.descriptor.priority == descriptor.priority
        }) {
            return Err(ProviderRegistryError::PriorityConflict(descriptor.kind));
        }
        self.entries.insert(
            descriptor.id.clone(),
            ProviderEntry {
                descriptor,
                provider,
            },
        );
        Ok(())
    }

    pub fn select(&self, kind: ProviderKind) -> ProviderSelection {
        self.entries
            .values()
            .filter(|entry| entry.descriptor.kind == kind)
            .max_by_key(|entry| entry.descriptor.priority)
            .map(|entry| ProviderSelection::Available(entry.descriptor.clone()))
            .unwrap_or(ProviderSelection::Unsupported(kind))
    }

    pub fn providers(&self) -> Vec<ProviderDescriptor> {
        self.entries
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect()
    }

    pub fn snapshot(&self) -> RegistrySnapshot {
        RegistrySnapshot {
            parsers: Vec::new(),
            unavailable_parsers: Vec::new(),
            providers: self.providers(),
        }
    }

    pub fn bind(
        &self,
        kind: ProviderKind,
        provider_id: &str,
        network: NetworkAccess,
    ) -> Result<ProviderBinding, ProviderRegistryError> {
        let entry = self
            .entries
            .get(provider_id)
            .ok_or_else(|| ProviderRegistryError::UnknownProvider(provider_id.to_string()))?;
        if entry.descriptor.kind != kind {
            return Err(ProviderRegistryError::KindMismatch);
        }
        match (entry.descriptor.network, network) {
            (NetworkPolicy::Forbidden, NetworkAccess::Allowed) => {
                Err(ProviderRegistryError::NetworkDenied)
            }
            (NetworkPolicy::Required, NetworkAccess::Denied) => {
                Err(ProviderRegistryError::NetworkRequired)
            }
            _ => Ok(ProviderBinding::new(entry.provider.clone(), network)),
        }
    }

    pub fn select_into(
        &self,
        providers: &mut ProviderSet,
        kind: ProviderKind,
        provider_id: &str,
        network: NetworkAccess,
    ) -> Result<Option<ProviderBinding>, ProviderRegistryError> {
        self.bind(kind, provider_id, network)?;
        let entry = self
            .entries
            .get(provider_id)
            .ok_or_else(|| ProviderRegistryError::UnknownProvider(provider_id.to_string()))?;
        Ok(providers.select(kind, entry.provider.clone(), network))
    }
}

fn validate_provider(
    descriptor: &ProviderDescriptor,
    provider: &dyn Provider,
) -> Result<(), ProviderRegistryError> {
    let required = [
        descriptor.id.as_str(),
        descriptor.name.as_str(),
        descriptor.implementation.as_str(),
        descriptor.implementation_version.as_str(),
    ];
    if required
        .iter()
        .any(|value| value.trim().is_empty() || value.chars().any(char::is_control))
        || provider.name() != descriptor.name
    {
        return Err(ProviderRegistryError::Invalid(descriptor.id.clone()));
    }
    if let Some(kind) = provider.kind()
        && kind != descriptor.kind
    {
        return Err(ProviderRegistryError::KindMismatch);
    }
    if let Some(metadata) = provider.metadata()
        && (metadata.name != descriptor.name
            || metadata.implementation != descriptor.implementation
            || metadata.implementation_version != descriptor.implementation_version)
    {
        return Err(ProviderRegistryError::Invalid(descriptor.id.clone()));
    }
    match (descriptor.kind, descriptor.isolation.as_ref()) {
        (ProviderKind::IsolatedParserBackend, Some(policy))
            if policy.private_temporary_filesystem
                && policy.subprocess_limit > 0
                && !policy.active_content_execution => {}
        (ProviderKind::IsolatedParserBackend, _) => {
            return Err(ProviderRegistryError::Invalid(descriptor.id.clone()));
        }
        (_, Some(_)) => return Err(ProviderRegistryError::Invalid(descriptor.id.clone())),
        (_, None) => {}
    }
    Ok(())
}

#[derive(Default)]
pub struct IsolatedBackendRegistry {
    providers: ProviderRegistry,
}

impl IsolatedBackendRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn register_caller(
        &mut self,
        descriptor: ProviderDescriptor,
        backend: Arc<dyn Provider>,
    ) -> Result<(), ProviderRegistryError> {
        if descriptor.kind != ProviderKind::IsolatedParserBackend {
            return Err(ProviderRegistryError::KindMismatch);
        }
        self.providers.register_caller(descriptor, backend)
    }

    pub fn select(&self) -> ProviderSelection {
        self.providers.select(ProviderKind::IsolatedParserBackend)
    }

    pub fn bind(
        &self,
        provider_id: &str,
        network: NetworkAccess,
    ) -> Result<ProviderBinding, ProviderRegistryError> {
        self.providers
            .bind(ProviderKind::IsolatedParserBackend, provider_id, network)
    }

    pub fn providers(&self) -> Vec<ProviderDescriptor> {
        self.providers.providers()
    }
}
