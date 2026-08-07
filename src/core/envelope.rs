//! Universal operation result envelope and stable operation-state vocabulary.

use super::{
    ArtifactKind, ContentIdentity, Diagnostic, Hashes, MetadataInvariantError, ParserInfo,
    ProvenanceStep, ProviderInvocation, SchemaVersion, SourceInfo, canonical_json_sha256,
};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// The public operation that produced an envelope.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    #[default]
    Parse,
    Ingest,
    Transform,
    Render,
    Segment,
    Validate,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum EnvelopeInvariantError {
    #[error("complete operation requires a payload")]
    CompleteWithoutPayload,
    #[error("terminal operation status must not carry a payload")]
    TerminalStatusWithPayload,
    #[error("envelope schema version must not be empty")]
    MissingEnvelopeSchemaVersion,
    #[error("payload schema version must not be empty")]
    MissingPayloadSchemaVersion,
    #[error("options digest must be a lowercase SHA-256 digest")]
    InvalidOptionsDigest,
    #[error("invalid parser metadata: {0}")]
    InvalidParserMetadata(#[source] MetadataInvariantError),
    #[error("invalid provider invocation at index {index}: {source}")]
    InvalidProviderInvocation {
        index: usize,
        #[source]
        source: MetadataInvariantError,
    },
    #[error("invalid provenance step at index {index}: {source}")]
    InvalidProvenanceStep {
        index: usize,
        #[source]
        source: MetadataInvariantError,
    },
}

impl<T: Serialize> Serialize for Envelope<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        let mut identity = self
            .identity
            .clone()
            .or_else(|| self.hashes.clone().map(ContentIdentity::from));
        if let Some(payload) = self.payload.as_ref()
            && identity
                .as_ref()
                .is_none_or(|identity| identity.canonical_payload.is_none())
        {
            let current = identity.take().unwrap_or_default();
            identity = Some(
                current
                    .with_canonical_payload(self.payload_schema_version.clone(), payload)
                    .map_err(serde::ser::Error::custom)?,
            );
        }
        let provenance = resolved_provenance(&self.provenance, identity.as_ref(), &self.providers);
        let mut state = serializer.serialize_struct("Envelope", 14)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("operation", &self.operation)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("status", &self.status)?;
        state.serialize_field("source", &self.source)?;
        state.serialize_field("identity", &identity)?;
        state.serialize_field("hashes", &self.hashes)?;
        state.serialize_field("parser", &self.parser)?;
        state.serialize_field("options_digest", &self.options_digest)?;
        state.serialize_field("providers", &self.providers)?;
        state.serialize_field("diagnostics", &self.diagnostics)?;
        state.serialize_field("provenance", &provenance)?;
        state.serialize_field("payload_schema_version", &self.payload_schema_version)?;
        state.serialize_field("payload", &self.payload)?;
        state.end()
    }
}

#[derive(Deserialize)]
struct EnvelopeWire<T> {
    schema_version: SchemaVersion,
    #[serde(default)]
    operation: Option<OperationKind>,
    kind: PayloadKind,
    #[serde(default)]
    status: Option<OperationStatus>,
    source: SourceInfo,
    #[serde(default)]
    identity: Option<ContentIdentity>,
    #[serde(default)]
    hashes: Option<Hashes>,
    parser: ParserInfo,
    #[serde(default)]
    options_digest: Option<String>,
    #[serde(default)]
    providers: Option<Vec<ProviderInvocation>>,
    diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    provenance: Option<Vec<ProvenanceStep>>,
    payload_schema_version: SchemaVersion,
    #[serde(default = "no_payload")]
    payload: Option<T>,
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Envelope<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EnvelopeWire::deserialize(deserializer)?;
        let legacy_v1 = wire.schema_version.0 == SchemaVersion::ENVELOPE_V1;
        let operation = match wire.operation {
            Some(operation) => operation,
            None if legacy_v1 => OperationKind::Parse,
            None => return Err(serde::de::Error::missing_field("operation")),
        };
        let status = match wire.status {
            Some(status) => status,
            None if legacy_v1 => OperationStatus::Complete,
            None => return Err(serde::de::Error::missing_field("status")),
        };
        let options_digest = match wire.options_digest {
            Some(options_digest) => options_digest,
            None if legacy_v1 => empty_options_digest(),
            None => return Err(serde::de::Error::missing_field("options_digest")),
        };
        let providers = match wire.providers {
            Some(providers) => providers,
            None if legacy_v1 => Vec::new(),
            None => return Err(serde::de::Error::missing_field("providers")),
        };
        let provenance = match wire.provenance {
            Some(provenance) => provenance,
            None if legacy_v1 => Vec::new(),
            None => return Err(serde::de::Error::missing_field("provenance")),
        };
        let identity = wire
            .identity
            .or_else(|| wire.hashes.clone().map(ContentIdentity::from));
        let envelope = Self {
            schema_version: wire.schema_version,
            operation,
            kind: wire.kind,
            status,
            source: wire.source,
            identity,
            hashes: wire.hashes,
            parser: wire.parser,
            options_digest,
            providers,
            diagnostics: wire.diagnostics,
            provenance,
            payload_schema_version: wire.payload_schema_version,
            payload: wire.payload,
        };
        envelope.validate().map_err(serde::de::Error::custom)?;
        Ok(envelope)
    }
}

/// Deterministically digest a serializable options value.
pub fn options_digest<T: Serialize + ?Sized>(options: &T) -> Result<String, serde_json::Error> {
    canonical_json_sha256(options)
}

pub fn empty_options_digest() -> String {
    // SHA-256 of the JSON object `{}`.
    "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a".to_string()
}

fn is_sha256_digest(value: &str) -> bool {
    super::provenance::is_sha256_digest(value)
}

fn no_payload<T>() -> Option<T> {
    None
}

fn resolved_provenance(
    provenance: &[ProvenanceStep],
    identity: Option<&ContentIdentity>,
    providers: &[ProviderInvocation],
) -> Vec<ProvenanceStep> {
    let mut resolved = provenance.to_vec();
    let Some(root) = resolved.first_mut() else {
        return resolved;
    };
    if let Some(identity) = identity {
        if root.input_identity.is_none() {
            root.input_identity = input_identity_digest(identity);
        }
        if root.output_identity.is_none() {
            root.output_identity = output_identity_digest(identity);
        }
    }
    if root.provider.is_none() && providers.len() == 1 {
        root.provider = Some(providers[0].provider.clone());
    }
    resolved
}

fn input_identity_digest(identity: &ContentIdentity) -> Option<String> {
    identity
        .raw
        .as_ref()
        .map(|raw| raw.sha256.clone())
        .or_else(|| {
            identity
                .aggregate
                .as_ref()
                .map(|aggregate| aggregate.sha256.clone())
        })
        .or_else(|| {
            identity
                .decoded
                .as_ref()
                .map(|decoded| decoded.sha256.clone())
        })
}

fn output_identity_digest(identity: &ContentIdentity) -> Option<String> {
    identity
        .canonical_payload
        .as_ref()
        .map(|payload| payload.sha256.clone())
        .or_else(|| {
            identity
                .aggregate
                .as_ref()
                .map(|aggregate| aggregate.sha256.clone())
        })
}

/// Whether an operation satisfied its requested contract.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    #[default]
    Complete,
    Partial,
    Failed,
    Unsupported,
    Encrypted,
    Ambiguous,
    Cancelled,
}

impl OperationStatus {
    pub const fn permits_payload(self) -> bool {
        matches!(self, Self::Complete | Self::Partial)
    }

    pub const fn requires_payload(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// Alias that names the envelope's artifact discriminator by its v2 role.
pub type PayloadKind = ArtifactKind;

/// Universal result for parse, ingest, transform, render, segment, and
/// validation operations.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope<T> {
    pub schema_version: SchemaVersion,
    pub operation: OperationKind,
    pub kind: PayloadKind,
    pub status: OperationStatus,
    pub source: SourceInfo,
    /// Versioned raw, decoded, canonical-payload, format, and aggregate identity.
    pub identity: Option<ContentIdentity>,
    /// Legacy v1 hashes retained for wire and Rust compatibility.
    pub hashes: Option<Hashes>,
    pub parser: ParserInfo,
    pub options_digest: String,
    pub providers: Vec<ProviderInvocation>,
    pub diagnostics: Vec<Diagnostic>,
    pub provenance: Vec<ProvenanceStep>,
    pub payload_schema_version: SchemaVersion,
    pub payload: Option<T>,
}

impl<T> Envelope<T> {
    /// Compatibility constructor for a complete parse operation.
    pub fn new(
        kind: ArtifactKind,
        source: SourceInfo,
        parser: ParserInfo,
        payload_schema_version: impl Into<SchemaVersion>,
        payload: T,
    ) -> Self {
        Self::complete(
            OperationKind::Parse,
            kind,
            source,
            parser,
            empty_options_digest(),
            payload_schema_version,
            payload,
        )
    }

    pub fn complete(
        operation: OperationKind,
        kind: PayloadKind,
        source: SourceInfo,
        parser: ParserInfo,
        options_digest: impl Into<String>,
        payload_schema_version: impl Into<SchemaVersion>,
        payload: T,
    ) -> Self {
        let options_digest = options_digest.into();
        let provenance = vec![ProvenanceStep::for_operation(
            operation,
            &parser,
            options_digest.clone(),
        )];
        Self {
            schema_version: SchemaVersion::from(SchemaVersion::ENVELOPE_V2),
            operation,
            kind,
            status: OperationStatus::Complete,
            source,
            hashes: None,
            parser,
            identity: None,
            options_digest,
            providers: Vec::new(),
            diagnostics: Vec::new(),
            provenance,
            payload_schema_version: payload_schema_version.into(),
            payload: Some(payload),
        }
    }

    pub fn partial(
        operation: OperationKind,
        kind: PayloadKind,
        source: SourceInfo,
        parser: ParserInfo,
        options_digest: impl Into<String>,
        payload_schema_version: impl Into<SchemaVersion>,
        payload: Option<T>,
    ) -> Self {
        let options_digest = options_digest.into();
        let provenance = vec![ProvenanceStep::for_operation(
            operation,
            &parser,
            options_digest.clone(),
        )];
        Self {
            schema_version: SchemaVersion::from(SchemaVersion::ENVELOPE_V2),
            operation,
            kind,
            status: OperationStatus::Partial,
            source,
            hashes: None,
            parser,
            identity: None,
            options_digest,
            providers: Vec::new(),
            diagnostics: Vec::new(),
            provenance,
            payload_schema_version: payload_schema_version.into(),
            payload,
        }
    }

    pub fn without_payload(
        operation: OperationKind,
        kind: PayloadKind,
        status: OperationStatus,
        source: SourceInfo,
        parser: ParserInfo,
        options_digest: impl Into<String>,
        payload_schema_version: impl Into<SchemaVersion>,
    ) -> Result<Self, EnvelopeInvariantError> {
        let options_digest = options_digest.into();
        let provenance = vec![ProvenanceStep::for_operation(
            operation,
            &parser,
            options_digest.clone(),
        )];
        let envelope = Self {
            schema_version: SchemaVersion::from(SchemaVersion::ENVELOPE_V2),
            operation,
            kind,
            status,
            source,
            hashes: None,
            parser,
            identity: None,
            options_digest,
            providers: Vec::new(),
            diagnostics: Vec::new(),
            provenance,
            payload_schema_version: payload_schema_version.into(),
            payload: None,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn payload(&self) -> Option<&T> {
        self.payload.as_ref()
    }

    pub fn payload_mut(&mut self) -> Option<&mut T> {
        self.payload.as_mut()
    }

    pub fn into_payload(self) -> Option<T> {
        self.payload
    }

    pub fn with_hashes(mut self, hashes: Hashes) -> Self {
        self.hashes = Some(hashes.clone());
        let identity = ContentIdentity::from(hashes);
        self.sync_provenance_identity(&identity);
        self.identity = Some(identity);
        self
    }

    pub fn with_identity(mut self, identity: ContentIdentity) -> Self {
        self.sync_provenance_identity(&identity);
        self.identity = Some(identity);
        self
    }
    pub fn with_operation(mut self, operation: OperationKind) -> Self {
        self.operation = operation;
        if let Some(root) = self.provenance.first_mut() {
            root.operation = operation;
        }
        self
    }

    pub fn with_options_digest(mut self, options_digest: impl Into<String>) -> Self {
        self.options_digest = options_digest.into();
        if let Some(root) = self.provenance.first_mut() {
            root.options_digest.clone_from(&self.options_digest);
        }
        self
    }

    pub fn with_providers(mut self, providers: Vec<ProviderInvocation>) -> Self {
        self.providers = providers;
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Vec<Diagnostic>) -> Self {
        if self.status == OperationStatus::Complete
            && diagnostics.iter().any(|diagnostic| diagnostic.partial)
        {
            self.status = OperationStatus::Partial;
        }
        self.diagnostics = diagnostics;
        self
    }

    pub fn with_provenance(mut self, provenance: Vec<ProvenanceStep>) -> Self {
        self.provenance = provenance;
        self
    }

    fn sync_provenance_identity(&mut self, identity: &ContentIdentity) {
        if let Some(root) = self.provenance.first_mut() {
            if root.input_identity.is_none() {
                root.input_identity = input_identity_digest(identity);
            }
            if root.output_identity.is_none() {
                root.output_identity = output_identity_digest(identity);
            }
        }
    }

    pub fn validate(&self) -> Result<(), EnvelopeInvariantError> {
        if self.schema_version.0.is_empty() {
            return Err(EnvelopeInvariantError::MissingEnvelopeSchemaVersion);
        }
        if self.payload_schema_version.0.is_empty() {
            return Err(EnvelopeInvariantError::MissingPayloadSchemaVersion);
        }
        if !is_sha256_digest(&self.options_digest) {
            return Err(EnvelopeInvariantError::InvalidOptionsDigest);
        }
        self.parser
            .validate()
            .map_err(EnvelopeInvariantError::InvalidParserMetadata)?;
        for (index, provider) in self.providers.iter().enumerate() {
            provider.validate().map_err(|source| {
                EnvelopeInvariantError::InvalidProviderInvocation { index, source }
            })?;
        }
        for (index, step) in self.provenance.iter().enumerate() {
            step.validate()
                .map_err(|source| EnvelopeInvariantError::InvalidProvenanceStep {
                    index,
                    source,
                })?;
        }
        match (
            self.status.requires_payload(),
            self.status.permits_payload(),
            self.payload.is_some(),
        ) {
            (true, _, false) => Err(EnvelopeInvariantError::CompleteWithoutPayload),
            (_, false, true) => Err(EnvelopeInvariantError::TerminalStatusWithPayload),
            _ => Ok(()),
        }
    }
}

impl<T: Serialize> Envelope<T> {
    /// Add the canonical identity of the current payload without hashing
    /// envelope metadata, diagnostics, or provenance.
    pub fn with_canonical_payload_identity(mut self) -> Result<Self, serde_json::Error> {
        if let Some(payload) = self.payload.as_ref() {
            let identity = self.identity.take().unwrap_or_default();
            let identity =
                identity.with_canonical_payload(self.payload_schema_version.clone(), payload)?;
            self.sync_provenance_identity(&identity);
            self.identity = Some(identity);
        }
        Ok(self)
    }
}
