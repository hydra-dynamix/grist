//! Stable, domain-separated identities for normalized graph projections.

use crate::core::{ContentIdentity, ParserInfo, SourceLocator, canonical_json_sha256, sha256_hex};
use serde::{Deserialize, Serialize};

use super::DocumentRelation;

/// Compatibility boundary for node and edge identity material.
pub const GRAPH_IDENTITY_VERSION: &str = "grist/document-graph-identity/v1";

/// The strongest stability property available for one projection address.
#[cfg_attr(feature = "schemas", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphIdStability {
    /// A parser-native identifier anchors the address; locator movement is ignored.
    Native,
    /// A stable semantic structural path anchors the address.
    StructuralPath,
    /// The locator anchors the address and moves when source positions move.
    Locator,
}

/// One address in an authoritative payload.
#[cfg_attr(feature = "schemas", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectionAddress {
    pub structural_path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
}

impl ProjectionAddress {
    pub fn native<I, S>(structural_path: I, native_id: impl Into<String>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            structural_path: structural_path.into_iter().map(Into::into).collect(),
            native_id: Some(native_id.into()),
            locator: None,
        }
    }

    pub fn structural<I, S>(structural_path: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            structural_path: structural_path.into_iter().map(Into::into).collect(),
            native_id: None,
            locator: None,
        }
    }

    pub fn located<I, S>(structural_path: I, locator: SourceLocator) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            structural_path: structural_path.into_iter().map(Into::into).collect(),
            native_id: None,
            locator: Some(locator),
        }
    }

    pub fn with_locator(mut self, locator: SourceLocator) -> Self {
        self.locator = Some(locator);
        self
    }

    pub fn stability(&self) -> GraphIdStability {
        if self.native_id.is_some() {
            GraphIdStability::Native
        } else if self.locator.is_some() {
            GraphIdStability::Locator
        } else {
            GraphIdStability::StructuralPath
        }
    }

    fn validate(&self) -> Result<(), GraphIdentityError> {
        if self
            .structural_path
            .iter()
            .any(|segment| segment.trim().is_empty())
        {
            return Err(GraphIdentityError::EmptyPathSegment);
        }
        if self
            .native_id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err(GraphIdentityError::EmptyNativeId);
        }
        if self.structural_path.is_empty() && self.native_id.is_none() && self.locator.is_none() {
            return Err(GraphIdentityError::Unaddressable);
        }
        if let Some(locator) = &self.locator {
            locator
                .validate()
                .map_err(|error| GraphIdentityError::InvalidLocator(error.to_string()))?;
        }
        Ok(())
    }
}

/// Deterministic identity scope shared by all format projections.
#[derive(Debug, Clone)]
pub struct GraphIdGenerator {
    source_identity: String,
    payload_schema_version: String,
    parser: ParserInfo,
}

impl GraphIdGenerator {
    pub fn new(
        source_identity: impl Into<String>,
        payload_schema_version: impl Into<String>,
        parser: ParserInfo,
    ) -> Result<Self, GraphIdentityError> {
        let generator = Self {
            source_identity: source_identity.into(),
            payload_schema_version: payload_schema_version.into(),
            parser,
        };
        generator.validate()?;
        Ok(generator)
    }

    pub fn from_content_identity(
        identity: &ContentIdentity,
        payload_schema_version: impl Into<String>,
        parser: ParserInfo,
    ) -> Result<Self, GraphIdentityError> {
        let source_identity = canonical_json_sha256(identity)
            .map_err(|error| GraphIdentityError::Serialization(error.to_string()))?;
        Self::new(source_identity, payload_schema_version, parser)
    }

    pub fn source_identity(&self) -> &str {
        &self.source_identity
    }

    pub fn payload_schema_version(&self) -> &str {
        &self.payload_schema_version
    }

    pub fn parser(&self) -> &ParserInfo {
        &self.parser
    }

    pub fn node_id(&self, address: &ProjectionAddress) -> Result<String, GraphIdentityError> {
        self.id_for("node", address, None)
    }

    pub fn edge_id(
        &self,
        address: &ProjectionAddress,
        source: &str,
        relation: &DocumentRelation,
        target: &str,
    ) -> Result<String, GraphIdentityError> {
        if source.is_empty() || target.is_empty() {
            return Err(GraphIdentityError::EmptyEdgeEndpoint);
        }
        self.id_for(
            "edge",
            address,
            Some(EdgeIdentity {
                source,
                relation,
                target,
            }),
        )
    }

    fn validate(&self) -> Result<(), GraphIdentityError> {
        if self.source_identity.trim().is_empty() {
            return Err(GraphIdentityError::EmptySourceIdentity);
        }
        if self.payload_schema_version.trim().is_empty() {
            return Err(GraphIdentityError::EmptyPayloadSchema);
        }
        self.parser
            .validate()
            .map_err(|error| GraphIdentityError::InvalidParser(error.to_string()))
    }

    fn id_for(
        &self,
        domain: &'static str,
        address: &ProjectionAddress,
        edge: Option<EdgeIdentity<'_>>,
    ) -> Result<String, GraphIdentityError> {
        address.validate()?;
        let material = IdentityMaterial {
            identity_version: GRAPH_IDENTITY_VERSION,
            domain,
            source_identity: &self.source_identity,
            payload_schema_version: &self.payload_schema_version,
            parser: &self.parser,
            structural_path: &address.structural_path,
            native_id: address.native_id.as_deref(),
            // Native identities deliberately outrank moving coordinates.
            locator: address
                .native_id
                .is_none()
                .then_some(address.locator.as_ref())
                .flatten(),
            edge,
        };
        let bytes = crate::core::canonical_json_bytes(&material)
            .map_err(|error| GraphIdentityError::Serialization(error.to_string()))?;
        let digest = sha256_hex(&bytes);
        Ok(format!(
            "grist:{domain}:{}",
            digest.strip_prefix("sha256:").unwrap_or(&digest)
        ))
    }
}

#[derive(Serialize)]
struct IdentityMaterial<'a> {
    identity_version: &'static str,
    domain: &'static str,
    source_identity: &'a str,
    payload_schema_version: &'a str,
    parser: &'a ParserInfo,
    structural_path: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    native_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    locator: Option<&'a SourceLocator>,
    #[serde(skip_serializing_if = "Option::is_none")]
    edge: Option<EdgeIdentity<'a>>,
}

#[derive(Serialize)]
struct EdgeIdentity<'a> {
    source: &'a str,
    relation: &'a DocumentRelation,
    target: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphIdentityError {
    EmptySourceIdentity,
    EmptyPayloadSchema,
    EmptyPathSegment,
    EmptyNativeId,
    Unaddressable,
    EmptyEdgeEndpoint,
    InvalidParser(String),
    InvalidLocator(String),
    Serialization(String),
}

impl std::fmt::Display for GraphIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySourceIdentity => formatter.write_str("source identity must not be empty"),
            Self::EmptyPayloadSchema => formatter.write_str("payload schema must not be empty"),
            Self::EmptyPathSegment => formatter.write_str("structural path has an empty segment"),
            Self::EmptyNativeId => formatter.write_str("native identity must not be empty"),
            Self::Unaddressable => formatter.write_str("projection address has no identity"),
            Self::EmptyEdgeEndpoint => formatter.write_str("edge endpoint must not be empty"),
            Self::InvalidParser(message)
            | Self::InvalidLocator(message)
            | Self::Serialization(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for GraphIdentityError {}
