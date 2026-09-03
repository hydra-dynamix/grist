//! Explicitly authorized isolated parser backend contract.

use super::{
    ProviderContractError, ProviderError, ProviderMetadata, ProviderRequest,
    ProviderRequestContext, ProviderRequestManifest, ProviderResult, parameters_digest,
};
use crate::core::{Diagnostic, NetworkAccess, Provider, ProviderKind};
use crate::security::{PrivateTemporaryStorage, TemporaryRetentionPolicy, TemporaryStorageError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendPermission {
    Denied,
    ExplicitlyAllowed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationPolicy {
    pub private_temporary_filesystem: bool,
    pub read_only_input: bool,
    pub max_processes: u32,
    pub max_wall_time_millis: u64,
    pub max_memory_bytes: u64,
    pub max_temporary_storage_bytes: u64,
    #[serde(default)]
    pub temporary_retention: TemporaryRetentionPolicy,
    pub network_access: NetworkAccess,
    pub active_content_execution: bool,
    #[serde(default)]
    pub inherit_environment: bool,
    #[serde(default)]
    pub host_filesystem_access: bool,
}

impl IsolationPolicy {
    pub fn strict(
        max_wall_time_millis: u64,
        max_memory_bytes: u64,
        max_temporary_storage_bytes: u64,
    ) -> Result<Self, ProviderContractError> {
        let policy = Self {
            private_temporary_filesystem: true,
            read_only_input: true,
            max_processes: 1,
            max_wall_time_millis,
            max_memory_bytes,
            max_temporary_storage_bytes,
            temporary_retention: TemporaryRetentionPolicy::DeleteOnDrop,
            network_access: NetworkAccess::Denied,
            active_content_execution: false,
            inherit_environment: false,
            host_filesystem_access: false,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if !self.private_temporary_filesystem
            || !self.read_only_input
            || self.max_processes == 0
            || self.max_wall_time_millis == 0
            || self.max_memory_bytes == 0
            || self.max_temporary_storage_bytes == 0
            || self.active_content_execution
            || self.inherit_environment
            || self.host_filesystem_access
        {
            return Err(ProviderContractError::InvalidField("isolation_policy"));
        }
        Ok(())
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IsolatedBackendOptions {
    pub format: String,
    pub output_schema_version: String,
    pub isolation: IsolationPolicy,
    #[serde(default)]
    pub backend_options: Value,
}

pub struct IsolatedBackendRequest<'a> {
    pub context: ProviderRequestContext<'a>,
    pub options: IsolatedBackendOptions,
}

impl<'a> IsolatedBackendRequest<'a> {
    pub fn new(
        permission: BackendPermission,
        context: ProviderRequestContext<'a>,
        options: IsolatedBackendOptions,
    ) -> Result<Self, ProviderContractError> {
        if permission != BackendPermission::ExplicitlyAllowed {
            return Err(ProviderContractError::BackendPermissionDenied);
        }
        options.isolation.validate()?;
        if options.isolation.network_access != context.network_access() {
            return Err(ProviderContractError::NetworkPermissionMismatch);
        }
        if options.format.trim().is_empty() || options.output_schema_version.trim().is_empty() {
            return Err(ProviderContractError::InvalidField(
                "backend.format_or_schema",
            ));
        }
        Ok(Self { context, options })
    }

    pub fn manifest(&self) -> Result<ProviderRequestManifest, ProviderContractError> {
        Ok(ProviderRequestManifest {
            kind: ProviderKind::IsolatedParserBackend,
            input_identity: self.context.input().identity().clone(),
            network_access: self.context.network_access(),
            configuration_digest: self.context.configuration_digest().into(),
            parameters_digest: parameters_digest(&self.options)?,
        })
    }

    /// Allocate the policy-declared private filesystem. Backends should place
    /// all writable files here; host filesystem access remains forbidden.
    pub fn create_private_temporary_storage(
        &self,
    ) -> Result<PrivateTemporaryStorage, TemporaryStorageError> {
        PrivateTemporaryStorage::create_limited(
            self.options.isolation.temporary_retention,
            self.options.isolation.max_temporary_storage_bytes,
        )
    }
}

impl std::fmt::Debug for IsolatedBackendRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IsolatedBackendRequest")
            .field("context", &self.context)
            .field("options", &self.options)
            .finish()
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IsolatedBackendResult {
    pub payload_schema_version: String,
    pub payload: Value,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl IsolatedBackendResult {
    pub fn validate(&self) -> Result<(), ProviderContractError> {
        if self.payload_schema_version.trim().is_empty() {
            return Err(ProviderContractError::InvalidResult(
                "isolated backend output schema version is empty".into(),
            ));
        }
        Ok(())
    }
}

pub trait IsolatedParserBackend: Send + Sync + 'static {
    fn parse(
        &self,
        request: &IsolatedBackendRequest<'_>,
    ) -> Result<IsolatedBackendResult, ProviderError>;
}

pub struct IsolatedParserBackendAdapter<P> {
    metadata: ProviderMetadata,
    implementation: P,
}

impl<P> IsolatedParserBackendAdapter<P> {
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

impl<P: IsolatedParserBackend> Provider for IsolatedParserBackendAdapter<P> {
    fn name(&self) -> &str {
        &self.metadata.name
    }

    fn kind(&self) -> Option<ProviderKind> {
        Some(ProviderKind::IsolatedParserBackend)
    }

    fn metadata(&self) -> Option<&ProviderMetadata> {
        Some(&self.metadata)
    }

    fn invoke(&self, request: &ProviderRequest<'_>) -> Result<ProviderResult, ProviderError> {
        let ProviderRequest::IsolatedBackend(request) = request else {
            return Err(ProviderError::failure(
                &self.metadata.name,
                "isolated backend received a different request kind",
            ));
        };
        self.implementation
            .parse(request)
            .map(ProviderResult::IsolatedBackend)
    }
}
