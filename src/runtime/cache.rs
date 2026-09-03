//! Validated content-addressed cache contracts owned by callers.

use super::{MetricEvent, MetricPhase, MetricValues, MetricsHook};
use crate::core::{
    CanonicalJsonVersion, ContentIdentity, DeclaredLoss, MetadataInvariantError, OperationKind,
    ParserInfo, ProvenanceStep, SchemaVersion, canonical_json_bytes, canonical_json_sha256,
    sha256_hex,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::error::Error;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Complete material that selects one cached operation result.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheKey {
    pub schema_version: String,
    pub namespace: String,
    pub canonicalization: CanonicalJsonVersion,
    pub operation: OperationKind,
    pub input_sha256: String,
    pub parser_sha256: String,
    pub payload_schema_version: String,
    pub options_digest: String,
    pub provider_configuration_digests: Vec<String>,
    pub digest: String,
}

#[derive(Serialize)]
struct CacheKeyMaterial<'a> {
    schema_version: &'a str,
    namespace: &'a str,
    canonicalization: CanonicalJsonVersion,
    operation: OperationKind,
    input_sha256: &'a str,
    parser_sha256: &'a str,
    payload_schema_version: &'a str,
    options_digest: &'a str,
    provider_configuration_digests: &'a [String],
}

impl CacheKey {
    pub const NAMESPACE: &'static str = "grist/operation-cache/v1";

    pub fn for_identity(
        operation: OperationKind,
        identity: &ContentIdentity,
        parser: &ParserInfo,
        payload_schema_version: impl Into<String>,
        options_digest: impl Into<String>,
        provider_configuration_digests: impl IntoIterator<Item = String>,
    ) -> Result<Self, CacheKeyError> {
        let input_sha256 = identity
            .raw
            .as_ref()
            .map(|value| value.sha256.clone())
            .or_else(|| {
                identity
                    .aggregate
                    .as_ref()
                    .map(|value| value.sha256.clone())
            })
            .ok_or(CacheKeyError::MissingInputIdentity)?;
        Self::new(
            operation,
            input_sha256,
            parser,
            payload_schema_version,
            options_digest,
            provider_configuration_digests,
        )
    }

    pub fn new(
        operation: OperationKind,
        input_sha256: impl Into<String>,
        parser: &ParserInfo,
        payload_schema_version: impl Into<String>,
        options_digest: impl Into<String>,
        provider_configuration_digests: impl IntoIterator<Item = String>,
    ) -> Result<Self, CacheKeyError> {
        parser.validate().map_err(CacheKeyError::Parser)?;
        let mut provider_configuration_digests = provider_configuration_digests
            .into_iter()
            .collect::<Vec<_>>();
        provider_configuration_digests.sort();
        provider_configuration_digests.dedup();
        let mut key = Self {
            schema_version: SchemaVersion::CACHE_KEY_V1.into(),
            namespace: Self::NAMESPACE.into(),
            canonicalization: CanonicalJsonVersion::CURRENT,
            operation,
            input_sha256: input_sha256.into(),
            parser_sha256: canonical_json_sha256(parser)?,
            payload_schema_version: payload_schema_version.into(),
            options_digest: options_digest.into(),
            provider_configuration_digests,
            digest: String::new(),
        };
        key.validate_material()?;
        key.digest = key.expected_digest()?;
        Ok(key)
    }

    pub fn validate(&self) -> Result<(), CacheKeyError> {
        self.validate_material()?;
        let expected = self.expected_digest()?;
        if self.digest != expected {
            return Err(CacheKeyError::DigestMismatch);
        }
        Ok(())
    }

    fn validate_material(&self) -> Result<(), CacheKeyError> {
        if self.schema_version != SchemaVersion::CACHE_KEY_V1 {
            return Err(CacheKeyError::SchemaVersion);
        }
        if self.namespace != Self::NAMESPACE {
            return Err(CacheKeyError::Namespace);
        }
        if self.payload_schema_version.is_empty() {
            return Err(CacheKeyError::MissingPayloadSchemaVersion);
        }
        for (field, digest) in [
            ("input_sha256", self.input_sha256.as_str()),
            ("parser_sha256", self.parser_sha256.as_str()),
            ("options_digest", self.options_digest.as_str()),
        ] {
            if !is_sha256_digest(digest) {
                return Err(CacheKeyError::InvalidDigest(field));
            }
        }
        if !self
            .provider_configuration_digests
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        {
            return Err(CacheKeyError::ProviderDigestsNotCanonical);
        }
        if self
            .provider_configuration_digests
            .iter()
            .any(|digest| !is_sha256_digest(digest))
        {
            return Err(CacheKeyError::InvalidDigest(
                "provider_configuration_digests",
            ));
        }
        Ok(())
    }

    fn expected_digest(&self) -> Result<String, serde_json::Error> {
        canonical_json_sha256(&CacheKeyMaterial {
            schema_version: &self.schema_version,
            namespace: &self.namespace,
            canonicalization: self.canonicalization,
            operation: self.operation,
            input_sha256: &self.input_sha256,
            parser_sha256: &self.parser_sha256,
            payload_schema_version: &self.payload_schema_version,
            options_digest: &self.options_digest,
            provider_configuration_digests: &self.provider_configuration_digests,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CacheKeyError {
    #[error("content identity has no raw or aggregate hash")]
    MissingInputIdentity,
    #[error("cache key schema version is not supported")]
    SchemaVersion,
    #[error("cache key namespace is not supported")]
    Namespace,
    #[error("cache key payload schema version must not be empty")]
    MissingPayloadSchemaVersion,
    #[error("cache key {0} must contain lowercase SHA-256 digests")]
    InvalidDigest(&'static str),
    #[error("cache key provider digests must be sorted and unique")]
    ProviderDigestsNotCanonical,
    #[error("cache key digest does not match its material")]
    DigestMismatch,
    #[error("invalid parser metadata: {0}")]
    Parser(#[source] MetadataInvariantError),
    #[error("cache key canonicalization failed: {0}")]
    Canonical(#[from] serde_json::Error),
}

/// Stored canonical output plus the hashes required to validate reuse.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheEntry {
    pub schema_version: String,
    pub key_digest: String,
    pub canonical_output_sha256: String,
    pub canonical_output: Vec<u8>,
}

impl CacheEntry {
    pub fn from_value<T: Serialize>(
        key: &CacheKey,
        value: &T,
    ) -> Result<Self, CacheValidationError> {
        key.validate()?;
        let canonical_output = canonical_json_bytes(value)?;
        Ok(Self {
            schema_version: SchemaVersion::CACHE_ENTRY_V1.into(),
            key_digest: key.digest.clone(),
            canonical_output_sha256: sha256_hex(&canonical_output),
            canonical_output,
        })
    }

    pub fn decode<T>(&self, key: &CacheKey) -> Result<T, CacheValidationError>
    where
        T: DeserializeOwned + Serialize,
    {
        key.validate()?;
        if self.schema_version != SchemaVersion::CACHE_ENTRY_V1 {
            return Err(CacheValidationError::SchemaVersion);
        }
        if self.key_digest != key.digest {
            return Err(CacheValidationError::KeyMismatch);
        }
        if self.canonical_output_sha256 != sha256_hex(&self.canonical_output) {
            return Err(CacheValidationError::OutputDigestMismatch);
        }
        let value: serde_json::Value = serde_json::from_slice(&self.canonical_output)?;
        if canonical_json_bytes(&value)? != self.canonical_output {
            return Err(CacheValidationError::NonCanonicalOutput);
        }
        let decoded = T::deserialize(value)?;
        if canonical_json_bytes(&decoded)? != self.canonical_output {
            return Err(CacheValidationError::TypeRoundTripMismatch);
        }
        Ok(decoded)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CacheValidationError {
    #[error(transparent)]
    Key(#[from] CacheKeyError),
    #[error("cache entry schema version is not supported")]
    SchemaVersion,
    #[error("cache entry belongs to another key")]
    KeyMismatch,
    #[error("cache entry output digest does not match its bytes")]
    OutputDigestMismatch,
    #[error("cache entry output is valid JSON but not canonical JSON")]
    NonCanonicalOutput,
    #[error("cache entry output changes when decoded as the requested type")]
    TypeRoundTripMismatch,
    #[error("cache entry JSON validation failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Persistence policy remains entirely with the caller.
pub trait ContentAddressedCache {
    type Error: Error + Send + Sync + 'static;

    fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, Self::Error>;
    fn put(&self, key: &CacheKey, entry: &CacheEntry) -> Result<(), Self::Error>;
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CacheReuseOutcome {
    MissStored,
    HitValidated,
}

/// Public proof that a result was computed or reused only after validation.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheReuseRecord {
    pub schema_version: String,
    pub key_digest: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub options_digest: String,
    pub outcome: CacheReuseOutcome,
}

impl CacheReuseRecord {
    pub fn reused(&self) -> bool {
        self.outcome == CacheReuseOutcome::HitValidated
    }

    /// A validated hit is appended to an operation envelope as lossless reuse.
    /// Misses are already represented by this record and add no derivation.
    pub fn provenance_step(
        &self,
        operation: OperationKind,
    ) -> Result<Option<ProvenanceStep>, MetadataInvariantError> {
        if !self.reused() {
            return Ok(None);
        }
        ProvenanceStep::new(
            operation,
            "grist.cache.reuse/v1",
            self.input_sha256.clone(),
            self.output_sha256.clone(),
            self.options_digest.clone(),
            DeclaredLoss::Lossless,
        )
        .map(|step| Some(step.with_warning("grist.cache.reused")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheValue<T> {
    pub value: T,
    pub reuse: CacheReuseRecord,
}

#[derive(Debug, thiserror::Error)]
pub enum CacheExecutionError<CE, PE>
where
    CE: Error + 'static,
    PE: Error + 'static,
{
    #[error("cache backend failed: {0}")]
    Cache(#[source] CE),
    #[error(transparent)]
    Validation(#[from] CacheValidationError),
    #[error("operation used to fill cache failed: {0}")]
    Compute(#[source] PE),
}

pub fn load_or_compute<C, T, F, PE>(
    cache: &C,
    key: &CacheKey,
    operation: OperationKind,
    compute: F,
) -> Result<CacheValue<T>, CacheExecutionError<C::Error, PE>>
where
    C: ContentAddressedCache,
    T: DeserializeOwned + Serialize,
    F: FnOnce() -> Result<T, PE>,
    PE: Error + 'static,
{
    load_or_compute_with_metrics(cache, key, operation, &MetricsHook::default(), compute)
}

pub fn load_or_compute_with_metrics<C, T, F, PE>(
    cache: &C,
    key: &CacheKey,
    operation: OperationKind,
    metrics: &MetricsHook,
    compute: F,
) -> Result<CacheValue<T>, CacheExecutionError<C::Error, PE>>
where
    C: ContentAddressedCache,
    T: DeserializeOwned + Serialize,
    F: FnOnce() -> Result<T, PE>,
    PE: Error + 'static,
{
    key.validate().map_err(CacheValidationError::from)?;
    if let Some(entry) = cache.get(key).map_err(CacheExecutionError::Cache)? {
        let value = entry.decode(key)?;
        metrics.emit(&MetricEvent::new(
            operation,
            MetricPhase::Cache,
            MetricValues {
                cache_hits: 1,
                ..MetricValues::default()
            },
        ));
        return Ok(CacheValue {
            value,
            reuse: reuse_record(key, &entry, CacheReuseOutcome::HitValidated),
        });
    }

    let value = compute().map_err(CacheExecutionError::Compute)?;
    let entry = CacheEntry::from_value(key, &value)?;
    cache.put(key, &entry).map_err(CacheExecutionError::Cache)?;
    metrics.emit(&MetricEvent::new(
        operation,
        MetricPhase::Cache,
        MetricValues {
            cache_misses: 1,
            ..MetricValues::default()
        },
    ));
    Ok(CacheValue {
        value,
        reuse: reuse_record(key, &entry, CacheReuseOutcome::MissStored),
    })
}

fn reuse_record(
    key: &CacheKey,
    entry: &CacheEntry,
    outcome: CacheReuseOutcome,
) -> CacheReuseRecord {
    CacheReuseRecord {
        schema_version: SchemaVersion::CACHE_REUSE_V1.into(),
        key_digest: key.digest.clone(),
        input_sha256: key.input_sha256.clone(),
        output_sha256: entry.canonical_output_sha256.clone(),
        options_digest: key.options_digest.clone(),
        outcome,
    }
}

fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}
