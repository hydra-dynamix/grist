//! Stable embedded-artifact records and content-addressed storage boundaries.

use crate::core::{
    ContentIdentity, FormatIdentity, RawContentIdentity, SchemaVersion, SourceLocator,
    canonical_json_sha256, sha256_hex,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// The caller-visible inline cutoff, retained with either extracted storage form.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactCaptureOptions {
    pub inline_threshold_bytes: u64,
}

impl ArtifactCaptureOptions {
    pub const fn new(inline_threshold_bytes: u64) -> Self {
        Self {
            inline_threshold_bytes,
        }
    }
}

/// How the source relates the child to its immediate parent.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRelationship {
    AttachmentOf,
    EmbeddedIn,
    InlineResourceOf,
    AlternativeRepresentationOf,
    PackagePartOf,
    Unknown,
}

/// Content-disposition semantics, independent of storage or safety policy.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactDisposition {
    Attachment,
    Inline,
    Related,
    Alternative,
    PackagePart,
    Unspecified,
}

/// The parent is retained structurally rather than flattened into a path.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactParent {
    pub identity: ContentIdentity,
    pub relationship: ArtifactRelationship,
}

impl ArtifactParent {
    pub fn new(identity: ContentIdentity, relationship: ArtifactRelationship) -> Self {
        Self {
            identity,
            relationship,
        }
    }
}

/// Security classification used to decide whether materialization is allowed.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSafetyClassification {
    Passive,
    Encrypted,
    ActiveContent,
    Script,
    Macro,
    Executable,
    Suspicious,
    Unknown,
}

impl ArtifactSafetyClassification {
    pub const fn requires_explicit_unsafe_opt_in(self) -> bool {
        !matches!(self, Self::Passive)
    }

    fn risk_rank(self) -> u8 {
        match self {
            Self::Passive => 0,
            Self::Encrypted => 1,
            Self::ActiveContent => 2,
            Self::Script => 3,
            Self::Macro => 4,
            Self::Executable => 5,
            Self::Suspicious => 6,
            Self::Unknown => 1,
        }
    }
}

/// Why a safety class was selected.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactSafetyEvidence {
    pub source: String,
    pub value: String,
    pub classification: ArtifactSafetyClassification,
}

/// Auditable, deterministic safety result. Classifiers only inspect supplied metadata and bytes.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactSafety {
    pub classification: ArtifactSafetyClassification,
    #[serde(default)]
    pub evidence: Vec<ArtifactSafetyEvidence>,
}

impl ArtifactSafety {
    pub fn classify(
        media_type: Option<&str>,
        declared_filename: Option<&str>,
        bytes: &[u8],
        hint: Option<ArtifactSafetyClassification>,
    ) -> Self {
        let mut evidence = Vec::new();
        if let Some(hint) = hint {
            evidence.push(ArtifactSafetyEvidence {
                source: "parser_hint".into(),
                value: format!("{hint:?}").to_ascii_lowercase(),
                classification: hint,
            });
        }
        if bytes.starts_with(b"MZ")
            || bytes.starts_with(b"\x7fELF")
            || bytes.starts_with(b"\0asm")
            || bytes.starts_with(b"dex\n")
            || bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
            || matches!(
                bytes.get(..4),
                Some([0xfe, 0xed, 0xfa, 0xce])
                    | Some([0xfe, 0xed, 0xfa, 0xcf])
                    | Some([0xcf, 0xfa, 0xed, 0xfe])
                    | Some([0xce, 0xfa, 0xed, 0xfe])
            )
        {
            evidence.push(ArtifactSafetyEvidence {
                source: "magic_bytes".into(),
                value: "executable signature".into(),
                classification: ArtifactSafetyClassification::Executable,
            });
        }
        if bytes.starts_with(b"#!") {
            evidence.push(ArtifactSafetyEvidence {
                source: "magic_bytes".into(),
                value: "interpreter directive".into(),
                classification: ArtifactSafetyClassification::Script,
            });
        }
        if let Some(media_type) = media_type {
            let normalized = media_type
                .split(';')
                .next()
                .unwrap_or(media_type)
                .trim()
                .to_ascii_lowercase();
            let classification = classify_media_type(&normalized);
            evidence.push(ArtifactSafetyEvidence {
                source: "media_type".into(),
                value: normalized,
                classification,
            });
        }
        if let Some(filename) = declared_filename
            && let Some(extension) = Path::new(filename)
                .extension()
                .and_then(|value| value.to_str())
        {
            let extension = extension.to_ascii_lowercase();
            let classification = classify_extension(&extension);
            evidence.push(ArtifactSafetyEvidence {
                source: "filename_extension".into(),
                value: extension,
                classification,
            });
        }
        let classification = evidence
            .iter()
            .map(|item| item.classification)
            .max_by_key(|class| class.risk_rank())
            .unwrap_or(ArtifactSafetyClassification::Unknown);
        evidence.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.value.cmp(&right.value))
                .then_with(|| left.classification.cmp(&right.classification))
        });
        Self {
            classification,
            evidence,
        }
    }

    pub const fn requires_explicit_unsafe_opt_in(&self) -> bool {
        self.classification.requires_explicit_unsafe_opt_in()
    }
}

fn classify_media_type(media_type: &str) -> ArtifactSafetyClassification {
    match media_type {
        "application/x-msdownload"
        | "application/x-executable"
        | "application/vnd.microsoft.portable-executable"
        | "application/x-mach-binary" => ArtifactSafetyClassification::Executable,
        "application/vnd.ms-word.document.macroenabled.12"
        | "application/vnd.ms-excel.sheet.macroenabled.12"
        | "application/vnd.ms-powerpoint.presentation.macroenabled.12"
        | "application/vnd.ms-office.vbaproject" => ArtifactSafetyClassification::Macro,
        "application/javascript"
        | "application/ecmascript"
        | "text/javascript"
        | "application/x-powershell" => ArtifactSafetyClassification::Script,
        "text/html" | "application/xhtml+xml" | "image/svg+xml" => {
            ArtifactSafetyClassification::ActiveContent
        }
        "application/pkcs7-mime" | "application/pkcs8" => ArtifactSafetyClassification::Encrypted,
        value
            if value.starts_with("image/")
                || value.starts_with("audio/")
                || value.starts_with("video/")
                || value.starts_with("text/")
                || matches!(
                    value,
                    "application/pdf" | "application/json" | "application/xml" | "application/zip"
                ) =>
        {
            ArtifactSafetyClassification::Passive
        }
        _ => ArtifactSafetyClassification::Unknown,
    }
}

fn classify_extension(extension: &str) -> ArtifactSafetyClassification {
    match extension {
        "exe" | "dll" | "com" | "msi" | "scr" | "app" | "elf" | "so" | "dylib" | "class"
        | "jar" | "war" | "apk" | "dex" | "wasm" => ArtifactSafetyClassification::Executable,
        "docm" | "dotm" | "xlsm" | "xlam" | "xltm" | "pptm" | "potm" | "ppam" | "vba" | "vbe" => {
            ArtifactSafetyClassification::Macro
        }
        "bat" | "cmd" | "ps1" | "sh" | "js" | "vbs" | "py" | "rb" | "pl" => {
            ArtifactSafetyClassification::Script
        }
        "html" | "htm" | "xhtml" | "svg" => ArtifactSafetyClassification::ActiveContent,
        "txt" | "text" | "md" | "markdown" | "json" | "xml" | "csv" | "tsv" | "pdf" | "zip"
        | "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "mp3" | "wav"
        | "ogg" | "mp4" | "webm" => ArtifactSafetyClassification::Passive,
        _ => ArtifactSafetyClassification::Unknown,
    }
}

/// Terminal extraction states retained even when no child bytes are available.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactExtractionStatus {
    Extracted,
    Quarantined,
    InventoryOnly,
    Skipped,
    Encrypted,
    Unsupported,
    Rejected,
    BudgetLimited,
    Failed,
}

impl ArtifactExtractionStatus {
    const fn requires_content(self) -> bool {
        matches!(self, Self::Extracted | Self::Quarantined)
    }
}

/// Machine-readable extraction result for one child.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactExtraction {
    pub status: ArtifactExtractionStatus,
    pub status_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub diagnostic_codes: Vec<String>,
}

impl ArtifactExtraction {
    pub fn extracted(quarantined: bool) -> Self {
        if quarantined {
            Self::new(
                ArtifactExtractionStatus::Quarantined,
                "artifact.extracted.quarantined",
            )
        } else {
            Self::new(ArtifactExtractionStatus::Extracted, "artifact.extracted")
        }
    }

    pub fn new(status: ArtifactExtractionStatus, status_code: impl Into<String>) -> Self {
        Self {
            status,
            status_code: status_code.into(),
            message: None,
            diagnostic_codes: Vec::new(),
        }
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn with_diagnostic(mut self, code: impl Into<String>) -> Self {
        self.diagnostic_codes.push(code.into());
        self.diagnostic_codes.sort();
        self.diagnostic_codes.dedup();
        self
    }
}

/// Digest-keyed external reference. Grist does not prescribe persistence policy.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentAddressedArtifactReference {
    pub algorithm: String,
    pub digest: String,
    pub byte_length: u64,
}

impl ContentAddressedArtifactReference {
    pub fn for_bytes(bytes: &[u8]) -> Self {
        Self {
            algorithm: "sha256".into(),
            digest: sha256_hex(bytes),
            byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        }
    }

    pub fn validate_bytes(&self, bytes: &[u8]) -> Result<(), EmbeddedArtifactError> {
        if self.algorithm != "sha256"
            || self.byte_length != u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            || self.digest != sha256_hex(bytes)
        {
            return Err(EmbeddedArtifactError::ContentAddressMismatch);
        }
        Ok(())
    }
}

/// Inline representation with its caller-selected threshold made explicit.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactInlineBytes {
    pub inline_threshold_bytes: u64,
    pub bytes: Vec<u8>,
}

/// Storage representation is deliberately outside the stable artifact identity.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "storage", rename_all = "snake_case")]
pub enum ArtifactContent {
    Inline(ArtifactInlineBytes),
    ContentAddressed {
        inline_threshold_bytes: u64,
        reference: ContentAddressedArtifactReference,
    },
}

/// Stable child identity. It excludes filename, disposition, status, and storage mode.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactIdentity {
    pub artifact_id: String,
    pub content: ContentIdentity,
}

/// Metadata known before a child is stored or materialized.
#[derive(Debug, Clone)]
pub struct ArtifactMetadata {
    pub declared_filename: Option<String>,
    pub media_type: Option<String>,
    pub parent: ArtifactParent,
    pub locator: SourceLocator,
    pub disposition: ArtifactDisposition,
    pub safety_hint: Option<ArtifactSafetyClassification>,
}

impl ArtifactMetadata {
    pub fn new(
        parent: ArtifactParent,
        locator: SourceLocator,
        disposition: ArtifactDisposition,
    ) -> Self {
        Self {
            declared_filename: None,
            media_type: None,
            parent,
            locator,
            disposition,
            safety_hint: None,
        }
    }

    pub fn with_declared_filename(mut self, filename: impl Into<String>) -> Self {
        self.declared_filename = Some(filename.into());
        self
    }

    pub fn with_media_type(mut self, media_type: impl Into<String>) -> Self {
        self.media_type = Some(media_type.into());
        self
    }

    pub fn with_safety_hint(mut self, hint: ArtifactSafetyClassification) -> Self {
        self.safety_hint = Some(hint);
        self
    }
}

/// Sink is supplied explicitly when bytes exceed the inline threshold.
pub trait ContentAddressedArtifactSink {
    fn store(
        &self,
        reference: &ContentAddressedArtifactReference,
        bytes: &[u8],
    ) -> Result<(), ArtifactStoreError>;
}

/// Resolver is supplied explicitly when an external child is materialized.
pub trait ContentAddressedArtifactResolver {
    fn resolve(
        &self,
        reference: &ContentAddressedArtifactReference,
    ) -> Result<Vec<u8>, ArtifactStoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ArtifactStoreError {
    pub message: String,
}

impl ArtifactStoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Complete artifact inventory record.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EmbeddedArtifact {
    pub schema_version: String,
    pub identity: ArtifactIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub parent: ArtifactParent,
    pub locator: SourceLocator,
    pub disposition: ArtifactDisposition,
    pub safety: ArtifactSafety,
    pub extraction: ArtifactExtraction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ArtifactContent>,
}

#[derive(Deserialize)]
struct EmbeddedArtifactWire {
    schema_version: String,
    identity: ArtifactIdentity,
    declared_filename: Option<String>,
    media_type: Option<String>,
    parent: ArtifactParent,
    locator: SourceLocator,
    disposition: ArtifactDisposition,
    safety: ArtifactSafety,
    extraction: ArtifactExtraction,
    content: Option<ArtifactContent>,
}

impl<'de> Deserialize<'de> for EmbeddedArtifact {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EmbeddedArtifactWire::deserialize(deserializer)?;
        let artifact = Self {
            schema_version: wire.schema_version,
            identity: wire.identity,
            declared_filename: wire.declared_filename,
            media_type: wire.media_type,
            parent: wire.parent,
            locator: wire.locator,
            disposition: wire.disposition,
            safety: wire.safety,
            extraction: wire.extraction,
            content: wire.content,
        };
        artifact.validate().map_err(serde::de::Error::custom)?;
        Ok(artifact)
    }
}

impl EmbeddedArtifact {
    /// Capture bytes inline below the explicit cutoff, otherwise through the supplied sink.
    pub fn capture(
        metadata: ArtifactMetadata,
        bytes: &[u8],
        options: ArtifactCaptureOptions,
        sink: Option<&dyn ContentAddressedArtifactSink>,
    ) -> Result<Self, EmbeddedArtifactError> {
        let content =
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= options.inline_threshold_bytes {
                ArtifactContent::Inline(ArtifactInlineBytes {
                    inline_threshold_bytes: options.inline_threshold_bytes,
                    bytes: bytes.to_vec(),
                })
            } else {
                let reference = ContentAddressedArtifactReference::for_bytes(bytes);
                sink.ok_or(EmbeddedArtifactError::ContentAddressedSinkRequired)?
                    .store(&reference, bytes)
                    .map_err(EmbeddedArtifactError::Store)?;
                ArtifactContent::ContentAddressed {
                    inline_threshold_bytes: options.inline_threshold_bytes,
                    reference,
                }
            };
        Self::from_known_bytes(metadata, bytes, None, Some(content))
    }

    /// Capture a child in the explicit inline-payload mode.
    pub fn capture_inline(
        metadata: ArtifactMetadata,
        bytes: &[u8],
    ) -> Result<Self, EmbeddedArtifactError> {
        Self::from_known_bytes(
            metadata,
            bytes,
            None,
            Some(ArtifactContent::Inline(ArtifactInlineBytes {
                inline_threshold_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                bytes: bytes.to_vec(),
            })),
        )
    }

    /// Capture a child in the explicit content-addressed mode.
    pub fn capture_content_addressed(
        metadata: ArtifactMetadata,
        bytes: &[u8],
        sink: &dyn ContentAddressedArtifactSink,
    ) -> Result<Self, EmbeddedArtifactError> {
        let reference = ContentAddressedArtifactReference::for_bytes(bytes);
        sink.store(&reference, bytes)
            .map_err(EmbeddedArtifactError::Store)?;
        Self::from_known_bytes(
            metadata,
            bytes,
            None,
            Some(ArtifactContent::ContentAddressed {
                inline_threshold_bytes: 0,
                reference,
            }),
        )
    }

    /// Inventory exact child identity without retaining its bytes.
    pub fn inventory(
        metadata: ArtifactMetadata,
        bytes: &[u8],
    ) -> Result<Self, EmbeddedArtifactError> {
        Self::from_known_bytes(
            metadata,
            bytes,
            Some(ArtifactExtraction::new(
                ArtifactExtractionStatus::InventoryOnly,
                "artifact.inventory_only",
            )),
            None,
        )
    }

    /// Record a terminal outcome while retaining the identity of bytes that
    /// were available to the container controller.
    pub fn record_known_unavailable(
        metadata: ArtifactMetadata,
        bytes: &[u8],
        extraction: ArtifactExtraction,
    ) -> Result<Self, EmbeddedArtifactError> {
        if extraction.status.requires_content() {
            return Err(EmbeddedArtifactError::MissingContentForStatus(
                extraction.status,
            ));
        }
        Self::from_known_bytes(metadata, bytes, Some(extraction), None)
    }

    /// Record a terminal child status when extraction produced no bytes.
    pub fn record_unavailable(
        metadata: ArtifactMetadata,
        extraction: ArtifactExtraction,
    ) -> Result<Self, EmbeddedArtifactError> {
        if extraction.status.requires_content() {
            return Err(EmbeddedArtifactError::MissingContentForStatus(
                extraction.status,
            ));
        }
        validate_metadata(&metadata)?;
        require_parent_identity(&metadata.parent.identity)?;
        let safety = ArtifactSafety::classify(
            metadata.media_type.as_deref(),
            metadata.declared_filename.as_deref(),
            &[],
            metadata.safety_hint,
        );
        let identity = make_identity(
            &metadata.parent,
            &metadata.locator,
            &ContentIdentity::default(),
        )?;
        let artifact = Self {
            schema_version: SchemaVersion::EMBEDDED_ARTIFACT_V1.into(),
            identity,
            declared_filename: metadata.declared_filename,
            media_type: metadata.media_type,
            parent: metadata.parent,
            locator: metadata.locator,
            disposition: metadata.disposition,
            safety,
            extraction,
            content: None,
        };
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn validate(&self) -> Result<(), EmbeddedArtifactError> {
        if self.schema_version != SchemaVersion::EMBEDDED_ARTIFACT_V1 {
            return Err(EmbeddedArtifactError::UnsupportedSchemaVersion(
                self.schema_version.clone(),
            ));
        }
        require_parent_identity(&self.parent.identity)?;
        self.locator
            .validate()
            .map_err(|error| EmbeddedArtifactError::InvalidLocator(error.to_string()))?;
        validate_optional_metadata(self.declared_filename.as_deref(), "declared filename")?;
        validate_optional_metadata(self.media_type.as_deref(), "media type")?;
        if self.extraction.status_code.trim().is_empty() {
            return Err(EmbeddedArtifactError::EmptyStatusCode);
        }
        if self.safety.requires_explicit_unsafe_opt_in()
            && self.extraction.status == ArtifactExtractionStatus::Extracted
        {
            return Err(EmbeddedArtifactError::UnsafeContentNotQuarantined);
        }
        match (&self.content, self.extraction.status.requires_content()) {
            (Some(_), false) => {
                return Err(EmbeddedArtifactError::UnexpectedContentForStatus(
                    self.extraction.status,
                ));
            }
            (None, true) => {
                return Err(EmbeddedArtifactError::MissingContentForStatus(
                    self.extraction.status,
                ));
            }
            _ => {}
        }
        validate_content(
            &self.identity.content,
            self.content.as_ref(),
            self.extraction.status,
        )?;
        let expected = make_identity(&self.parent, &self.locator, &self.identity.content)?;
        if expected.artifact_id != self.identity.artifact_id {
            return Err(EmbeddedArtifactError::ArtifactIdentityMismatch);
        }
        Ok(())
    }

    pub fn inline_bytes(&self) -> Option<&[u8]> {
        match self.content.as_ref() {
            Some(ArtifactContent::Inline(inline)) => Some(&inline.bytes),
            _ => None,
        }
    }

    pub fn content_reference(&self) -> Option<&ContentAddressedArtifactReference> {
        match self.content.as_ref() {
            Some(ArtifactContent::ContentAddressed { reference, .. }) => Some(reference),
            _ => None,
        }
    }

    /// The explicit cutoff that selected inline or content-addressed storage.
    pub fn inline_threshold_bytes(&self) -> Option<u64> {
        match self.content.as_ref() {
            Some(ArtifactContent::Inline(inline)) => Some(inline.inline_threshold_bytes),
            Some(ArtifactContent::ContentAddressed {
                inline_threshold_bytes,
                ..
            }) => Some(*inline_threshold_bytes),
            None => None,
        }
    }

    fn from_known_bytes(
        metadata: ArtifactMetadata,
        bytes: &[u8],
        extraction: Option<ArtifactExtraction>,
        content: Option<ArtifactContent>,
    ) -> Result<Self, EmbeddedArtifactError> {
        validate_metadata(&metadata)?;
        require_parent_identity(&metadata.parent.identity)?;
        let safety = ArtifactSafety::classify(
            metadata.media_type.as_deref(),
            metadata.declared_filename.as_deref(),
            bytes,
            metadata.safety_hint,
        );
        let content_identity = ContentIdentity::for_raw_bytes(bytes).with_format(
            FormatIdentity::new("embedded_artifact", metadata.media_type.clone()),
        );
        let identity = make_identity(&metadata.parent, &metadata.locator, &content_identity)?;
        let extraction = extraction.unwrap_or_else(|| {
            ArtifactExtraction::extracted(safety.requires_explicit_unsafe_opt_in())
        });
        let artifact = Self {
            schema_version: SchemaVersion::EMBEDDED_ARTIFACT_V1.into(),
            identity,
            declared_filename: metadata.declared_filename,
            media_type: metadata.media_type,
            parent: metadata.parent,
            locator: metadata.locator,
            disposition: metadata.disposition,
            safety,
            extraction,
            content,
        };
        artifact.validate()?;
        Ok(artifact)
    }
}

#[derive(Serialize)]
struct ArtifactIdMaterial<'a> {
    schema_version: &'static str,
    parent_identity: &'a ContentIdentity,
    relationship: &'a ArtifactRelationship,
    locator: &'a SourceLocator,
    child_raw_identity: Option<&'a RawContentIdentity>,
}

fn make_identity(
    parent: &ArtifactParent,
    locator: &SourceLocator,
    content: &ContentIdentity,
) -> Result<ArtifactIdentity, EmbeddedArtifactError> {
    let digest = canonical_json_sha256(&ArtifactIdMaterial {
        schema_version: SchemaVersion::EMBEDDED_ARTIFACT_V1,
        parent_identity: &parent.identity,
        relationship: &parent.relationship,
        locator,
        child_raw_identity: content.raw.as_ref(),
    })
    .map_err(EmbeddedArtifactError::IdentitySerialization)?;
    Ok(ArtifactIdentity {
        artifact_id: format!("artifact:{digest}"),
        content: content.clone(),
    })
}

fn validate_content(
    identity: &ContentIdentity,
    content: Option<&ArtifactContent>,
    status: ArtifactExtractionStatus,
) -> Result<(), EmbeddedArtifactError> {
    let Some(content) = content else {
        if status.requires_content() && identity.raw.is_some() {
            return Err(EmbeddedArtifactError::ContentAddressMismatch);
        }
        return Ok(());
    };
    let raw = identity
        .raw
        .as_ref()
        .ok_or(EmbeddedArtifactError::ContentAddressMismatch)?;
    match content {
        ArtifactContent::Inline(inline) => {
            let length = u64::try_from(inline.bytes.len()).unwrap_or(u64::MAX);
            if length > inline.inline_threshold_bytes {
                return Err(EmbeddedArtifactError::InlineThresholdExceeded {
                    byte_length: length,
                    threshold: inline.inline_threshold_bytes,
                });
            }
            if raw.byte_length != length || raw.sha256 != sha256_hex(&inline.bytes) {
                return Err(EmbeddedArtifactError::ContentAddressMismatch);
            }
        }
        ArtifactContent::ContentAddressed {
            inline_threshold_bytes,
            reference,
        } => {
            if reference.algorithm != "sha256"
                || raw.byte_length != reference.byte_length
                || raw.sha256 != reference.digest
            {
                return Err(EmbeddedArtifactError::ContentAddressMismatch);
            }
            if raw.byte_length > 0 && raw.byte_length <= *inline_threshold_bytes {
                return Err(
                    EmbeddedArtifactError::ExternalContentWithinInlineThreshold {
                        byte_length: raw.byte_length,
                        threshold: *inline_threshold_bytes,
                    },
                );
            }
        }
    }
    Ok(())
}

fn require_parent_identity(identity: &ContentIdentity) -> Result<(), EmbeddedArtifactError> {
    if identity.raw.is_none()
        && identity.decoded.is_none()
        && identity.canonical_payload.is_none()
        && identity.aggregate.is_none()
    {
        return Err(EmbeddedArtifactError::ParentIdentityUnavailable);
    }
    Ok(())
}

fn validate_metadata(metadata: &ArtifactMetadata) -> Result<(), EmbeddedArtifactError> {
    metadata
        .locator
        .validate()
        .map_err(|error| EmbeddedArtifactError::InvalidLocator(error.to_string()))?;
    validate_optional_metadata(metadata.declared_filename.as_deref(), "declared filename")?;
    validate_optional_metadata(metadata.media_type.as_deref(), "media type")
}

fn validate_optional_metadata(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), EmbeddedArtifactError> {
    if let Some(value) = value
        && (value.is_empty() || value.contains('\0') || value.chars().any(char::is_control))
    {
        return Err(EmbeddedArtifactError::InvalidMetadata(field));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddedArtifactError {
    #[error("unsupported embedded-artifact schema version {0}")]
    UnsupportedSchemaVersion(String),
    #[error("parent content identity has no verifiable digest")]
    ParentIdentityUnavailable,
    #[error("invalid artifact locator: {0}")]
    InvalidLocator(String),
    #[error("invalid {0}")]
    InvalidMetadata(&'static str),
    #[error("artifact extraction status code cannot be empty")]
    EmptyStatusCode,
    #[error("unsafe extracted content must have quarantined status")]
    UnsafeContentNotQuarantined,
    #[error("status {0:?} requires artifact content")]
    MissingContentForStatus(ArtifactExtractionStatus),
    #[error("status {0:?} cannot retain artifact content")]
    UnexpectedContentForStatus(ArtifactExtractionStatus),
    #[error("inline artifact has {byte_length} bytes, above threshold {threshold}")]
    InlineThresholdExceeded { byte_length: u64, threshold: u64 },
    #[error("external artifact has {byte_length} bytes, within inline threshold {threshold}")]
    ExternalContentWithinInlineThreshold { byte_length: u64, threshold: u64 },
    #[error("artifact bytes or reference do not match the retained content identity")]
    ContentAddressMismatch,
    #[error("artifact ID does not match parent, locator, relationship, and child identity")]
    ArtifactIdentityMismatch,
    #[error("a content-addressed sink is required above the inline threshold")]
    ContentAddressedSinkRequired,
    #[error("content-addressed store failed: {0}")]
    Store(ArtifactStoreError),
    #[error("artifact identity could not be canonicalized: {0}")]
    IdentitySerialization(serde_json::Error),
}
