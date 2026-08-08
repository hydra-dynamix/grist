//! Deterministic discovery of the compiled Grist capability surface.

use crate::core::{
    BudgetAxis, BudgetProfile, BudgetProfileDefinition, NetworkAccess, ProviderKind, SchemaVersion,
};
use crate::registry::{
    Capability, FormatMetadata, IsolationMetadata, ParserDescriptor, ParserRegistry,
    ParserRegistryError, ProviderDescriptor, ProviderRegistry, RegistrySnapshot, SchemaMetadata,
    UnavailableReason, builtin_parser_registry, builtin_provider_registry,
};
use crate::render::ReconstructionClaim;
use crate::schema::{SchemaCatalog, SchemaEntry, schema_catalog};
use crate::security::SecurityPolicy;
use crate::transform::{FormatReconstructionClaim, ReconstructionError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Whether a compiled feature gate is active in this build.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeatureCapability {
    pub name: String,
    pub enabled: bool,
}

/// Backend identity used by one registered parser implementation.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct BackendCapability {
    pub parser_id: String,
    pub parser_name: String,
    pub available: bool,
    pub implementation: String,
    pub implementation_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grammar_or_specification_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_identity: Option<String>,
}

/// Canonical availability record for one format known to the active registry.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormatCapability {
    pub format: FormatMetadata,
    pub available: bool,
    pub parser_ids: Vec<String>,
    pub payload_schemas: Vec<SchemaMetadata>,
    pub options_schemas: Vec<SchemaMetadata>,
    pub required_features: Vec<String>,
    pub parser_capabilities: Vec<Capability>,
    pub allowed_providers: Vec<ProviderKind>,
    pub required_providers: Vec<ProviderKind>,
    pub backends: Vec<BackendCapability>,
    pub unavailable_reasons: Vec<UnavailableReason>,
}

/// Availability and request requirements for one provider kind.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderCapability {
    pub kind: ProviderKind,
    pub available: bool,
    pub implementations: Vec<ProviderDescriptor>,
    pub allowed_by_formats: Vec<String>,
    pub required_by_formats: Vec<String>,
    pub explicit_selection_required: bool,
    pub network_permission_modes: Vec<NetworkAccess>,
    pub unavailable_reason: Option<String>,
}

/// Discoverable named budget policies and every enforceable budget axis.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetCapabilities {
    pub explicit_selection_required: bool,
    pub cli_default_profile: String,
    pub profiles: Vec<BudgetProfileDefinition>,
    pub axes: Vec<BudgetAxis>,
}

/// Closed security modes supported by the compiled library.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SecurityMode {
    ActiveContentPreservedInert,
    ExplicitProvidersOnly,
    XmlActiveReferencesRejected,
    HostileArchiveProtection,
    ActiveRenderingEscaped,
    TemporaryFilesDeletedOnDrop,
    CallerDirectedInputMetadata,
    SecretsNeverSerialized,
}

/// Security policy and isolation behavior available to callers.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityCapabilities {
    pub policy_schema_version: String,
    pub default_policy: SecurityPolicy,
    pub supported_modes: Vec<SecurityMode>,
    pub implicit_network_access: bool,
    pub active_content_execution: bool,
    pub secrets_serialized: bool,
    pub isolated_backend_minimum: IsolationMetadata,
}

/// Faithful reconstruction support for one known source format.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconstructionCapability {
    pub format: String,
    pub supported: bool,
    pub claims: Vec<FormatReconstructionClaim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

/// One deliberately or currently unsupported public capability.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnsupportedCapability {
    pub id: String,
    pub reason: String,
}

/// Canonical library and CLI discovery manifest.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityManifest {
    pub schema_version: String,
    pub grist_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_identity: Option<String>,
    pub features: Vec<FeatureCapability>,
    pub enabled_features: Vec<String>,
    pub operations: Vec<String>,
    pub registry: RegistrySnapshot,
    pub formats: Vec<FormatCapability>,
    pub backends: Vec<BackendCapability>,
    pub providers: Vec<ProviderCapability>,
    pub schema_catalog: SchemaCatalog,
    pub schemas: Vec<SchemaEntry>,
    pub envelope_schema_versions: Vec<String>,
    pub budgets: BudgetCapabilities,
    pub security: SecurityCapabilities,
    pub reconstruction: Vec<ReconstructionCapability>,
    pub normalized_rendering_reconstruction_claim: ReconstructionClaim,
    pub unsupported_capabilities: Vec<UnsupportedCapability>,
}

impl CapabilityManifest {
    pub const SCHEMA_VERSION: &'static str = "grist/capability-manifest/v1";

    /// Build a deterministic manifest from caller-visible registries and
    /// independently registered, fixture-backed reconstruction claims.
    pub fn from_registries(
        parsers: &ParserRegistry,
        providers: &ProviderRegistry,
        reconstruction_claims: Vec<FormatReconstructionClaim>,
    ) -> Result<Self, CapabilityDiscoveryError> {
        let registry = RegistrySnapshot::from_registries(parsers, providers);
        let formats = format_capabilities(&registry);
        let backends = formats
            .iter()
            .flat_map(|format| format.backends.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let provider_capabilities = provider_capabilities(&registry, &formats);
        let reconstruction = reconstruction_capabilities(&formats, reconstruction_claims)?;
        let features = compiled_features();
        let enabled_features = features
            .iter()
            .filter(|feature| feature.enabled)
            .map(|feature| feature.name.clone())
            .collect();
        let catalog = schema_catalog();
        let schemas = catalog
            .schemas
            .iter()
            .map(|schema| SchemaEntry {
                name: schema.name.clone(),
                schema_version: schema.schema_version.clone(),
            })
            .collect();
        let unsupported_capabilities =
            unsupported_capabilities(&formats, &provider_capabilities, &reconstruction);

        Ok(Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            grist_version: crate::version().to_string(),
            build_identity: option_env!("GRIST_BUILD_ID").map(str::to_owned),
            features,
            enabled_features,
            operations: operation_ids(),
            registry,
            formats,
            backends,
            providers: provider_capabilities,
            schema_catalog: catalog,
            schemas,
            envelope_schema_versions: vec![
                SchemaVersion::ENVELOPE_V1.to_string(),
                SchemaVersion::ENVELOPE_V2.to_string(),
            ],
            budgets: budget_capabilities(),
            security: security_capabilities(),
            reconstruction,
            normalized_rendering_reconstruction_claim:
                ReconstructionClaim::NormalizedNotByteRoundTrip,
            unsupported_capabilities,
        })
    }
}

/// Return the manifest for exactly the built-in surface compiled into this crate.
pub fn discover() -> Result<CapabilityManifest, CapabilityDiscoveryError> {
    let parsers = builtin_parser_registry()?;
    let providers = builtin_provider_registry();
    CapabilityManifest::from_registries(&parsers, &providers, Vec::new())
}

#[derive(Debug, thiserror::Error)]
pub enum CapabilityDiscoveryError {
    #[error(transparent)]
    ParserRegistry(#[from] ParserRegistryError),
    #[error(transparent)]
    Reconstruction(#[from] ReconstructionError),
}

#[derive(Default)]
struct FormatBuilder {
    format: Option<FormatMetadata>,
    available: bool,
    parser_ids: BTreeSet<String>,
    payload_schemas: BTreeSet<SchemaMetadata>,
    options_schemas: BTreeSet<SchemaMetadata>,
    required_features: BTreeSet<String>,
    parser_capabilities: BTreeSet<Capability>,
    allowed_providers: BTreeSet<ProviderKind>,
    required_providers: BTreeSet<ProviderKind>,
    backends: BTreeSet<BackendCapability>,
    unavailable_reasons: BTreeSet<UnavailableReason>,
}

fn format_capabilities(registry: &RegistrySnapshot) -> Vec<FormatCapability> {
    let mut formats = BTreeMap::<String, FormatBuilder>::new();
    for parser in &registry.parsers {
        add_parser(&mut formats, parser, true, None);
    }
    for unavailable in &registry.unavailable_parsers {
        add_parser(
            &mut formats,
            &unavailable.descriptor,
            false,
            Some(&unavailable.reason),
        );
    }
    formats
        .into_values()
        .map(|builder| FormatCapability {
            format: builder.format.unwrap(),
            available: builder.available,
            parser_ids: builder.parser_ids.into_iter().collect(),
            payload_schemas: builder.payload_schemas.into_iter().collect(),
            options_schemas: builder.options_schemas.into_iter().collect(),
            required_features: builder.required_features.into_iter().collect(),
            parser_capabilities: builder.parser_capabilities.into_iter().collect(),
            allowed_providers: builder.allowed_providers.into_iter().collect(),
            required_providers: builder.required_providers.into_iter().collect(),
            backends: builder.backends.into_iter().collect(),
            unavailable_reasons: builder.unavailable_reasons.into_iter().collect(),
        })
        .collect()
}

fn add_parser(
    formats: &mut BTreeMap<String, FormatBuilder>,
    parser: &ParserDescriptor,
    available: bool,
    reason: Option<&UnavailableReason>,
) {
    let builder = formats.entry(parser.format.id.clone()).or_default();
    builder.format.get_or_insert_with(|| parser.format.clone());
    builder.available |= available;
    builder.parser_ids.insert(parser.id.clone());
    builder
        .payload_schemas
        .insert(parser.payload_schema.clone());
    builder
        .options_schemas
        .insert(parser.options.schema.clone());
    builder
        .required_features
        .extend(parser.required_features.iter().cloned());
    builder
        .parser_capabilities
        .extend(parser.capabilities.iter().copied());
    builder
        .allowed_providers
        .extend(parser.allowed_providers.iter().copied());
    builder
        .required_providers
        .extend(parser.required_providers.iter().copied());
    if let Some(reason) = reason {
        builder.unavailable_reasons.insert(reason.clone());
    }
    builder.backends.insert(BackendCapability {
        parser_id: parser.id.clone(),
        parser_name: parser.parser.name.clone(),
        available,
        implementation: parser
            .parser
            .implementation
            .clone()
            .unwrap_or_else(|| parser.parser.name.clone()),
        implementation_version: parser
            .parser
            .implementation_version
            .clone()
            .unwrap_or_else(|| parser.parser.version.clone()),
        grammar_or_specification_version: parser.parser.grammar_version.clone(),
        build_identity: parser.parser.build_identity.clone(),
    });
}

fn provider_capabilities(
    registry: &RegistrySnapshot,
    formats: &[FormatCapability],
) -> Vec<ProviderCapability> {
    [
        ProviderKind::Ocr,
        ProviderKind::Transcription,
        ProviderKind::Decryption,
        ProviderKind::IsolatedParserBackend,
    ]
    .into_iter()
    .map(|kind| provider_capability(kind, registry, formats))
    .collect()
}

fn provider_capability(
    kind: ProviderKind,
    registry: &RegistrySnapshot,
    formats: &[FormatCapability],
) -> ProviderCapability {
    provider_capability_impl(kind, registry, formats)
}

fn provider_capability_impl(
    kind: ProviderKind,
    registry: &RegistrySnapshot,
    formats: &[FormatCapability],
) -> ProviderCapability {
    let implementations = registry
        .providers
        .iter()
        .filter(|provider| provider.kind == kind)
        .cloned()
        .collect::<Vec<_>>();
    let available = !implementations.is_empty();
    ProviderCapability {
        kind,
        available,
        implementations,
        allowed_by_formats: provider_formats(kind, formats, false),
        required_by_formats: provider_formats(kind, formats, true),
        explicit_selection_required: true,
        network_permission_modes: vec![NetworkAccess::Denied, NetworkAccess::Allowed],
        unavailable_reason: unavailable_provider_reason(available),
    }
}

fn unavailable_provider_reason(available: bool) -> Option<String> {
    (!available).then(|| "caller registration required".to_string())
}

fn provider_formats(
    kind: ProviderKind,
    formats: &[FormatCapability],
    required: bool,
) -> Vec<String> {
    formats
        .iter()
        .filter(|format| {
            if required {
                format.required_providers.contains(&kind)
            } else {
                format.allowed_providers.contains(&kind)
            }
        })
        .map(|format| format.format.id.clone())
        .collect()
}

fn budget_capabilities() -> BudgetCapabilities {
    BudgetCapabilities {
        explicit_selection_required: true,
        cli_default_profile: "untrusted_service@1".to_string(),
        profiles: [
            BudgetProfile::UntrustedServiceV1.definition(),
            BudgetProfile::TrustedUnboundedV1.definition(),
        ]
        .into_iter()
        .collect(),
        axes: BudgetAxis::ALL.to_vec(),
    }
}

fn security_capabilities() -> SecurityCapabilities {
    SecurityCapabilities {
        policy_schema_version: "grist/security-policy/v1".to_string(),
        default_policy: SecurityPolicy::default(),
        supported_modes: vec![
            SecurityMode::ActiveContentPreservedInert,
            SecurityMode::ExplicitProvidersOnly,
            SecurityMode::XmlActiveReferencesRejected,
            SecurityMode::HostileArchiveProtection,
            SecurityMode::ActiveRenderingEscaped,
            SecurityMode::TemporaryFilesDeletedOnDrop,
            SecurityMode::CallerDirectedInputMetadata,
            SecurityMode::SecretsNeverSerialized,
        ],
        implicit_network_access: false,
        active_content_execution: false,
        secrets_serialized: false,
        isolated_backend_minimum: IsolationMetadata::safe_default(),
    }
}

fn reconstruction_capabilities(
    formats: &[FormatCapability],
    claims: Vec<FormatReconstructionClaim>,
) -> Result<Vec<ReconstructionCapability>, CapabilityDiscoveryError> {
    let known = formats
        .iter()
        .map(|format| format.format.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut by_format = BTreeMap::<String, Vec<FormatReconstructionClaim>>::new();
    for claim in claims {
        claim.validate()?;
        if !known.contains(claim.format.as_str()) {
            return Err(ReconstructionError::InvalidClaim {
                message: format!(
                    "reconstruction claim for {} does not match a registered format",
                    claim.format
                ),
            }
            .into());
        }
        by_format
            .entry(claim.format.clone())
            .or_default()
            .push(claim);
    }
    for claims in by_format.values_mut() {
        claims.sort_by(|left, right| {
            (
                left.package_profile.as_str(),
                left.implementation.as_str(),
                left.implementation_version.as_str(),
                left.media_type.as_str(),
            )
                .cmp(&(
                    right.package_profile.as_str(),
                    right.implementation.as_str(),
                    right.implementation_version.as_str(),
                    right.media_type.as_str(),
                ))
        });
    }
    Ok(reconstruction_records(formats, by_format))
}

fn reconstruction_records(
    formats: &[FormatCapability],
    mut by_format: BTreeMap<String, Vec<FormatReconstructionClaim>>,
) -> Vec<ReconstructionCapability> {
    formats
        .iter()
        .map(|format| {
            let claims = by_format.remove(&format.format.id).unwrap_or_default();
            let supported = !claims.is_empty();
            let unavailable_reason = (!supported).then(|| {
                if format.available {
                    "no fixture-backed format reconstructor is registered"
                } else {
                    "the format parser and a fixture-backed reconstructor are unavailable"
                }
                .to_string()
            });
            ReconstructionCapability {
                format: format.format.id.clone(),
                supported,
                claims,
                unavailable_reason,
            }
        })
        .collect()
}

fn unsupported_capabilities(
    formats: &[FormatCapability],
    providers: &[ProviderCapability],
    reconstruction: &[ReconstructionCapability],
) -> Vec<UnsupportedCapability> {
    let mut unsupported = BTreeSet::from([
        UnsupportedCapability {
            id: "execution.active_content".to_string(),
            reason: "active content is preserved as inert data and is never executed".to_string(),
        },
        UnsupportedCapability {
            id: "network.implicit_fetch".to_string(),
            reason: "core parsing never fetches network resources".to_string(),
        },
        UnsupportedCapability {
            id: "render.normalized_byte_round_trip".to_string(),
            reason: "normalized rendering is not faithful package reconstruction".to_string(),
        },
    ]);
    add_unsupported_formats(&mut unsupported, formats);
    add_unsupported_providers(&mut unsupported, providers);
    add_unsupported_reconstruction(&mut unsupported, reconstruction);
    unsupported.into_iter().collect()
}

fn add_unsupported_formats(
    unsupported: &mut BTreeSet<UnsupportedCapability>,
    formats: &[FormatCapability],
) {
    for format in formats.iter().filter(|format| !format.available) {
        unsupported.insert(UnsupportedCapability {
            id: format!("format.{}", format.format.id),
            reason: format_unavailable_reason(&format.unavailable_reasons),
        });
    }
}

fn add_unsupported_providers(
    unsupported: &mut BTreeSet<UnsupportedCapability>,
    providers: &[ProviderCapability],
) {
    for provider in providers.iter().filter(|provider| !provider.available) {
        unsupported.insert(UnsupportedCapability {
            id: format!("provider.{}", provider_kind_id(provider.kind)),
            reason: provider.unavailable_reason.clone().unwrap(),
        });
    }
}

fn add_unsupported_reconstruction(
    unsupported: &mut BTreeSet<UnsupportedCapability>,
    reconstruction: &[ReconstructionCapability],
) {
    for capability in reconstruction
        .iter()
        .filter(|capability| !capability.supported)
    {
        unsupported.insert(UnsupportedCapability {
            id: format!("reconstruction.{}", capability.format),
            reason: capability.unavailable_reason.clone().unwrap(),
        });
    }
}

fn format_unavailable_reason(reasons: &[UnavailableReason]) -> String {
    if reasons.is_empty() {
        return "no registered parser is available".to_string();
    }
    reasons
        .iter()
        .map(|reason| match reason {
            UnavailableReason::FeatureDisabled { feature } => {
                format!("compiled feature {feature} is disabled")
            }
            UnavailableReason::NotImplemented => {
                "recognized format has no implemented parser".to_string()
            }
            UnavailableReason::BackendUnavailable { backend } => {
                format!("required isolated backend {backend} is unavailable")
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn provider_kind_id(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Ocr => "ocr",
        ProviderKind::Transcription => "transcription",
        ProviderKind::Decryption => "decryption",
        ProviderKind::IsolatedParserBackend => "isolated_parser_backend",
    }
}

fn operation_ids() -> Vec<String> {
    [
        "detect",
        "parse",
        "ingest_file",
        "ingest_repo",
        "ingest_archive",
        "inspect",
        "transform",
        "render",
        "segment",
        "validate",
        "schema",
        "capabilities",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn compiled_features() -> Vec<FeatureCapability> {
    let mut features = Vec::new();
    macro_rules! feature {
        ($name:literal) => {
            features.push(FeatureCapability {
                name: $name.to_string(),
                enabled: cfg!(feature = $name),
            });
        };
    }
    feature!("archives");
    feature!("asciidoc");
    feature!("bash");
    feature!("basin");
    feature!("bibliography");
    feature!("c");
    feature!("cli");
    feature!("code");
    feature!("columnar");
    feature!("cpp");
    feature!("csharp");
    feature!("css");
    feature!("csv");
    feature!("document-graph");
    feature!("email-message");
    feature!("epub");
    feature!("extended-encodings");
    feature!("full");
    feature!("go");
    feature!("html");
    feature!("java");
    feature!("kotlin");
    feature!("latex");
    feature!("ldgr-projection");
    feature!("markdown");
    feature!("media");
    feature!("model-output");
    feature!("notebooks");
    feature!("odf-word");
    feature!("pdf");
    feature!("php");
    feature!("presentation-ooxml");
    feature!("presentations");
    feature!("python");
    feature!("restructured-text");
    feature!("rtf");
    feature!("ruby");
    feature!("rust");
    feature!("schemas");
    feature!("scholarly");
    feature!("secondary-code");
    feature!("serialization");
    feature!("spreadsheet-odf");
    feature!("spreadsheet-ooxml");
    feature!("spreadsheets");
    feature!("sql");
    feature!("sqlite");
    feature!("structured-binary");
    feature!("structured-data");
    feature!("swift");
    feature!("text-publishing");
    feature!("typescript");
    feature!("word-ooxml");
    feature!("word-processing");
    feature!("xml");
    features
}
