//! Serializable contracts for the repository fixture corpus.

use crate::core::CanonicalJsonVersion;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub const CORPUS_MANIFEST_SCHEMA_VERSION: &str = "grist/fixture-corpus-manifest/v1";
pub const EXPECTED_OUTPUT_POLICY_VERSION: &str = "grist/expected-output-policy/v1";

/// Section 16.1 fixture classes. This is deliberately exhaustive so adding a
/// parser cannot silently reinterpret a generic `other` bucket.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FixtureClass {
    MinimalValid,
    RepresentativeRealWorld,
    MaximumComplexity,
    Empty,
    Truncated,
    Malformed,
    Adversarial,
    Encrypted,
    Oversized,
    DeeplyNested,
    MixedEncoding,
    InvalidText,
    NestedAttachments,
    NestedContainers,
    UnsupportedConstruct,
    ProviderRecording,
    MaliciousActiveContent,
    DownstreamRegression,
}

pub const REQUIRED_FIXTURE_CLASSES: [FixtureClass; 18] = [
    FixtureClass::MinimalValid,
    FixtureClass::RepresentativeRealWorld,
    FixtureClass::MaximumComplexity,
    FixtureClass::Empty,
    FixtureClass::Truncated,
    FixtureClass::Malformed,
    FixtureClass::Adversarial,
    FixtureClass::Encrypted,
    FixtureClass::Oversized,
    FixtureClass::DeeplyNested,
    FixtureClass::MixedEncoding,
    FixtureClass::InvalidText,
    FixtureClass::NestedAttachments,
    FixtureClass::NestedContainers,
    FixtureClass::UnsupportedConstruct,
    FixtureClass::ProviderRecording,
    FixtureClass::MaliciousActiveContent,
    FixtureClass::DownstreamRegression,
];

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FixtureOrigin {
    Synthetic,
    Licensed,
    Malicious,
    ProviderRecording,
    DownstreamRegression,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Redistribution {
    Permitted,
    MetadataOnly,
    Prohibited,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FixtureStorage {
    CheckedIn,
    ExternalOnly,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureLicense {
    /// SPDX expression, or `LicenseRef-...` paired with `license_file`.
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
    pub redistribution: Redistribution,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureProvenance {
    pub origin: FixtureOrigin,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_sha256: Option<String>,
    pub license: FixtureLicense,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureHandling {
    pub storage: FixtureStorage,
    /// Only `public` fixtures may be checked in. Other labels document private
    /// intake records whose bytes remain outside the repository.
    pub data_classification: String,
    pub inert_only: bool,
    pub execution_allowed: bool,
    pub network_allowed: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureFile {
    /// Path relative to `fixtures/`. Omitted for metadata-only/private intake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub byte_length: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuilderKind {
    MaximumComplexity,
    NestedContainer,
    SyntheticBytes,
    ProviderRecording,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureBuilder {
    pub kind: BuilderKind,
    /// Argument vector relative to the repository root. It is never executed
    /// through a shell.
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub command: Vec<String>,
    pub generator_version: String,
    pub seed: String,
    pub recipe_path: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExpectedNormalization {
    /// No value is removed or rewritten.
    Exact,
    /// Remove one explicitly named caller-supplied timestamp.
    RemoveCallerTimestamp { json_pointer: String },
    /// Replace a host-dependent path with the literal `$FIXTURE_ROOT`.
    ReplaceFixtureRoot { json_pointer: String },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExpectedOutput {
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub surface: String,
    pub path: String,
    pub schema_version: String,
    pub canonicalization: CanonicalJsonVersion,
    pub byte_length: u64,
    pub sha256: String,
    pub canonical_sha256: String,
    /// Explicit even when exact; there is no implicit volatile-field list.
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub normalization: Vec<ExpectedNormalization>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureCase {
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub id: String,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub classes: Vec<FixtureClass>,
    pub input: FixtureFile,
    pub provenance: FixtureProvenance,
    pub handling: FixtureHandling,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builder: Option<FixtureBuilder>,
    #[serde(default)]
    pub expected: Vec<ExpectedOutput>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureFormat {
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub format: String,
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub cases: Vec<FixtureCase>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorpusPolicy {
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub required_classes: Vec<FixtureClass>,
    pub expected_output_policy: String,
    pub canonicalization: CanonicalJsonVersion,
    pub checked_in_data_classification: String,
    pub active_content_execution_allowed: bool,
    pub implicit_network_allowed: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureCorpusManifest {
    pub schema_version: String,
    pub policy: CorpusPolicy,
    /// Stable format key to independently maintained fixture registrations.
    #[cfg_attr(feature = "schemas", schemars(length(min = 1)))]
    pub formats: BTreeMap<String, FixtureFormat>,
}
