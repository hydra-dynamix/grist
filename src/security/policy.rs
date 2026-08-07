//! One strict policy shared by every byte-facing operation.

use super::{ArchiveSecurityPolicy, TemporaryRetentionPolicy, XmlSecurityPolicy};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPolicy {
    /// Active constructs are retained as data and can never be invoked.
    #[default]
    PreserveInert,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// Only an explicitly selected provider receives a caller's per-request
    /// permission. Core parsing never performs network I/O.
    #[default]
    ExplicitProvidersOnly,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActiveContentPolicy {
    #[default]
    Escape,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputMetadataPolicy {
    #[default]
    CallerDirectedOnly,
}

/// Serializable security behavior carried by every parse request. Its closed
/// enums intentionally provide no unsafe execution or implicit-network mode.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityPolicy {
    pub execution: ExecutionPolicy,
    pub network: NetworkPolicy,
    pub xml: XmlSecurityPolicy,
    pub archive: ArchiveSecurityPolicy,
    pub active_rendering: ActiveContentPolicy,
    pub temporary_retention: TemporaryRetentionPolicy,
    pub input_metadata: InputMetadataPolicy,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            execution: ExecutionPolicy::PreserveInert,
            network: NetworkPolicy::ExplicitProvidersOnly,
            xml: XmlSecurityPolicy::default(),
            archive: ArchiveSecurityPolicy::default(),
            active_rendering: ActiveContentPolicy::Escape,
            temporary_retention: TemporaryRetentionPolicy::DeleteOnDrop,
            input_metadata: InputMetadataPolicy::CallerDirectedOnly,
        }
    }
}
