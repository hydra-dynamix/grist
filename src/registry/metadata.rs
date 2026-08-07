use crate::core::{ArtifactKind, ParserInfo, ProviderKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ParserOrigin {
    BuiltIn,
    Caller,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    NativeExtraction,
    TypedPayload,
    DocumentGraphProjection,
    Streaming,
    EmbeddedArtifacts,
    ProviderDerivedContent,
    IsolatedBackend,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Determinism {
    Guaranteed,
    GuaranteedWithRecording,
    NotGuaranteed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    Forbidden,
    Optional,
    Required,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IsolationMetadata {
    pub private_temporary_filesystem: bool,
    pub subprocess_limit: u32,
    pub network: NetworkPolicy,
    pub active_content_execution: bool,
}

impl IsolationMetadata {
    pub fn safe_default() -> Self {
        Self {
            private_temporary_filesystem: true,
            subprocess_limit: 1,
            network: NetworkPolicy::Forbidden,
            active_content_execution: false,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SchemaMetadata {
    pub name: String,
    pub version: String,
}

impl SchemaMetadata {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionsMetadata {
    pub schema: SchemaMetadata,
    pub default: Value,
}

impl OptionsMetadata {
    pub fn new(schema: SchemaMetadata, default: Value) -> Self {
        Self { schema, default }
    }

    pub fn empty(format: &str) -> Self {
        Self::new(
            SchemaMetadata::new(
                format!("{format}-options"),
                format!("grist/{format}-options/v1"),
            ),
            Value::Object(Default::default()),
        )
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FormatMetadata {
    pub id: String,
    pub display_name: String,
    pub artifact_kind: ArtifactKind,
    #[serde(default)]
    pub aliases: BTreeSet<String>,
    #[serde(default)]
    pub media_types: BTreeSet<String>,
    #[serde(default)]
    pub extensions: BTreeSet<String>,
}

impl FormatMetadata {
    pub fn new(id: impl Into<String>, artifact_kind: ArtifactKind) -> Self {
        let id = id.into();
        Self {
            display_name: id.clone(),
            id,
            artifact_kind,
            aliases: BTreeSet::new(),
            media_types: BTreeSet::new(),
            extensions: BTreeSet::new(),
        }
    }

    pub fn with_aliases(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.aliases.extend(values.into_iter().map(Into::into));
        self
    }

    pub fn with_media_types(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.media_types.extend(values.into_iter().map(Into::into));
        self
    }

    pub fn with_extensions(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.extensions.extend(values.into_iter().map(Into::into));
        self
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParserDescriptor {
    pub id: String,
    pub origin: ParserOrigin,
    pub priority: i32,
    pub format: FormatMetadata,
    pub parser: ParserInfo,
    pub payload_schema: SchemaMetadata,
    pub options: OptionsMetadata,
    #[serde(default)]
    pub required_features: BTreeSet<String>,
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub allowed_providers: BTreeSet<ProviderKind>,
    #[serde(default)]
    pub required_providers: BTreeSet<ProviderKind>,
}

impl ParserDescriptor {
    pub const BUILTIN_PRIORITY: i32 = 0;
    pub const CALLER_PRIORITY: i32 = 100;

    pub fn caller(
        id: impl Into<String>,
        format: FormatMetadata,
        parser: ParserInfo,
        payload_schema: SchemaMetadata,
        options: OptionsMetadata,
    ) -> Self {
        Self {
            id: id.into(),
            origin: ParserOrigin::Caller,
            priority: Self::CALLER_PRIORITY,
            format,
            parser,
            payload_schema,
            options,
            required_features: BTreeSet::new(),
            capabilities: BTreeSet::from([Capability::NativeExtraction, Capability::TypedPayload]),
            allowed_providers: BTreeSet::new(),
            required_providers: BTreeSet::new(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UnavailableReason {
    FeatureDisabled { feature: String },
    NotImplemented,
    BackendUnavailable { backend: String },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnavailableParser {
    pub descriptor: ParserDescriptor,
    pub reason: UnavailableReason,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: String,
    pub origin: ParserOrigin,
    pub kind: ProviderKind,
    pub priority: i32,
    pub name: String,
    pub implementation: String,
    pub implementation_version: String,
    pub enabled_feature: Option<String>,
    pub capabilities: BTreeSet<String>,
    pub network: NetworkPolicy,
    pub determinism: Determinism,
    pub isolation: Option<IsolationMetadata>,
}

impl ProviderDescriptor {
    pub fn caller(
        id: impl Into<String>,
        kind: ProviderKind,
        name: impl Into<String>,
        implementation: impl Into<String>,
        implementation_version: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            origin: ParserOrigin::Caller,
            kind,
            priority: ParserDescriptor::CALLER_PRIORITY,
            name: name.into(),
            implementation: implementation.into(),
            implementation_version: implementation_version.into(),
            enabled_feature: None,
            capabilities: BTreeSet::new(),
            network: NetworkPolicy::Forbidden,
            determinism: Determinism::NotGuaranteed,
            isolation: None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistrySnapshot {
    pub parsers: Vec<ParserDescriptor>,
    pub unavailable_parsers: Vec<UnavailableParser>,
    pub providers: Vec<ProviderDescriptor>,
}
