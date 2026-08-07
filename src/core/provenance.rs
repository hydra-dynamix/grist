//! Parser, provider, and derivation metadata carried by operation envelopes.

use super::{Diagnostic, OperationKind};
use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum MetadataInvariantError {
    #[error("parser name must not be empty")]
    MissingParserName,
    #[error("Grist version must not be empty")]
    MissingGristVersion,
    #[error("implementation name must not be empty when present")]
    MissingImplementation,
    #[error("provider name must not be empty")]
    MissingProvider,
    #[error("provider implementation must not be empty")]
    MissingProviderImplementation,
    #[error("provenance implementation must not be empty")]
    MissingProvenanceImplementation,
    #[error("{field} identity must not be empty when present")]
    EmptyIdentity { field: &'static str },
    #[error("{field} must be a lowercase SHA-256 digest")]
    InvalidDigest { field: &'static str },
    #[error("declared loss class must not be empty")]
    EmptyLossClass,
}

/// Parser and backend identity used to interpret an input.
///
/// `version` is the Grist crate version retained under its v1 JSON name.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParserInfo {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_feature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grammar_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_identity: Option<String>,
}

impl ParserInfo {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            implementation: Some(name.clone()),
            name,
            version: env!("CARGO_PKG_VERSION").to_string(),
            implementation_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            enabled_feature: None,
            grammar_version: None,
            build_identity: option_env!("GRIST_BUILD_ID").map(str::to_owned),
        }
    }

    pub fn with_implementation(
        mut self,
        implementation: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        self.implementation = Some(implementation.into());
        self.implementation_version = Some(version.into());
        self
    }

    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.enabled_feature = Some(feature.into());
        self
    }

    pub fn with_grammar_version(mut self, version: impl Into<String>) -> Self {
        self.grammar_version = Some(version.into());
        self
    }

    pub fn with_specification_version(self, version: impl Into<String>) -> Self {
        self.with_grammar_version(version)
    }

    pub fn with_build_identity(mut self, identity: impl Into<String>) -> Self {
        self.build_identity = Some(identity.into());
        self
    }

    pub fn validate(&self) -> Result<(), MetadataInvariantError> {
        if self.name.is_empty() {
            return Err(MetadataInvariantError::MissingParserName);
        }
        if self.version.is_empty() {
            return Err(MetadataInvariantError::MissingGristVersion);
        }
        if self.implementation.as_deref() == Some("") {
            return Err(MetadataInvariantError::MissingImplementation);
        }
        Ok(())
    }

    pub(crate) fn effective_implementation(&self) -> String {
        match (
            self.implementation.as_deref(),
            self.implementation_version.as_deref(),
        ) {
            (Some(implementation), Some(version)) => format!("{implementation}@{version}"),
            (Some(implementation), None) => implementation.to_string(),
            (None, _) => self.name.clone(),
        }
    }
}

/// One explicitly attributed external-provider call.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProviderInvocation {
    pub provider: String,
    pub implementation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
    pub configuration_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deterministic: Option<bool>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

impl ProviderInvocation {
    pub fn new(
        provider: impl Into<String>,
        implementation: impl Into<String>,
        configuration_digest: impl Into<String>,
    ) -> Result<Self, MetadataInvariantError> {
        let invocation = Self {
            provider: provider.into(),
            implementation: implementation.into(),
            configuration_digest: configuration_digest.into(),
            ..Self::default()
        };
        invocation.validate()?;
        Ok(invocation)
    }

    pub fn with_model_version(mut self, version: impl Into<String>) -> Self {
        self.model_version = Some(version.into());
        self
    }

    pub fn with_identities(
        mut self,
        input_identity: impl Into<String>,
        output_identity: impl Into<String>,
    ) -> Self {
        self.input_identity = Some(input_identity.into());
        self.output_identity = Some(output_identity.into());
        self
    }

    pub fn with_timing(mut self, caller_supplied_timing: impl Into<String>) -> Self {
        self.timing = Some(caller_supplied_timing.into());
        self
    }

    pub fn validate(&self) -> Result<(), MetadataInvariantError> {
        if self.provider.is_empty() {
            return Err(MetadataInvariantError::MissingProvider);
        }
        if self.implementation.is_empty() {
            return Err(MetadataInvariantError::MissingProviderImplementation);
        }
        validate_digest("configuration_digest", &self.configuration_digest)?;
        validate_optional_identity("input", self.input_identity.as_deref())?;
        validate_optional_identity("output", self.output_identity.as_deref())
    }
}

/// Open loss classification for compatibility across independently shipped parsers.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct LossClass(String);

impl LossClass {
    pub const CONTENT_OMITTED: &'static str = "content_omitted";
    pub const STRUCTURE_FLATTENED: &'static str = "structure_flattened";
    pub const FORMATTING_NORMALIZED: &'static str = "formatting_normalized";
    pub const PRECISION_REDUCED: &'static str = "precision_reduced";
    pub const ORDER_INFERRED: &'static str = "order_inferred";
    pub const REPAIR_APPLIED: &'static str = "repair_applied";

    pub fn new(value: impl Into<String>) -> Result<Self, MetadataInvariantError> {
        let value = value.into();
        if value.is_empty() {
            return Err(MetadataInvariantError::EmptyLossClass);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for LossClass {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for LossClass {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for LossClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Explicit loss outcome required by the checked derivation constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredLoss {
    Lossless,
    Lossy(LossClass),
}

/// One declared derivation step that contributed to an operation result.
///
/// A missing `loss_class` is the stable wire representation of an explicit
/// lossless declaration; any lossy step names its loss class.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProvenanceStep {
    pub operation: OperationKind,
    pub implementation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_identity: Option<String>,
    pub options_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_class: Option<LossClass>,
}

impl ProvenanceStep {
    pub fn new(
        operation: OperationKind,
        implementation: impl Into<String>,
        input_identity: impl Into<String>,
        output_identity: impl Into<String>,
        options_digest: impl Into<String>,
        loss: DeclaredLoss,
    ) -> Result<Self, MetadataInvariantError> {
        let step = Self {
            operation,
            implementation: implementation.into(),
            input_identity: Some(input_identity.into()),
            output_identity: Some(output_identity.into()),
            options_digest: options_digest.into(),
            timestamp: None,
            provider: None,
            warnings: Vec::new(),
            loss_class: match loss {
                DeclaredLoss::Lossless => None,
                DeclaredLoss::Lossy(class) => Some(class),
            },
        };
        step.validate()?;
        Ok(step)
    }

    pub(crate) fn for_operation(
        operation: OperationKind,
        parser: &ParserInfo,
        options_digest: String,
    ) -> Self {
        Self {
            operation,
            implementation: parser.effective_implementation(),
            input_identity: None,
            output_identity: None,
            options_digest,
            timestamp: None,
            provider: None,
            warnings: Vec::new(),
            loss_class: None,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn with_timestamp(mut self, caller_supplied_timestamp: impl Into<String>) -> Self {
        self.timestamp = Some(caller_supplied_timestamp.into());
        self
    }

    pub fn with_warning(mut self, code: impl Into<String>) -> Self {
        self.warnings.push(code.into());
        self
    }

    pub fn declared_loss(&self) -> DeclaredLoss {
        self.loss_class
            .clone()
            .map_or(DeclaredLoss::Lossless, DeclaredLoss::Lossy)
    }

    pub fn validate(&self) -> Result<(), MetadataInvariantError> {
        if self.implementation.is_empty() {
            return Err(MetadataInvariantError::MissingProvenanceImplementation);
        }
        validate_digest("options_digest", &self.options_digest)?;
        validate_optional_identity("input", self.input_identity.as_deref())?;
        validate_optional_identity("output", self.output_identity.as_deref())?;
        if self
            .loss_class
            .as_ref()
            .is_some_and(|loss| loss.0.is_empty())
        {
            return Err(MetadataInvariantError::EmptyLossClass);
        }
        Ok(())
    }
}

pub(crate) fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn validate_digest(field: &'static str, value: &str) -> Result<(), MetadataInvariantError> {
    if is_sha256_digest(value) {
        Ok(())
    } else {
        Err(MetadataInvariantError::InvalidDigest { field })
    }
}

fn validate_optional_identity(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), MetadataInvariantError> {
    if value == Some("") {
        Err(MetadataInvariantError::EmptyIdentity { field })
    } else {
        Ok(())
    }
}
