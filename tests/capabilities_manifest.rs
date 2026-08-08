use grist::capabilities::{CapabilityManifest, discover};
#[cfg(feature = "pdf")]
use grist::core::{BudgetAxis, ProviderKind, SchemaVersion};
use grist::registry::{builtin_parser_registry, builtin_provider_registry};
use grist::transform::{
    FormatReconstructionClaim, ReconstructionFidelity, ReconstructionFixtureEvidence,
};

#[test]
fn builtin_manifest_is_deterministic_sorted_and_schema_versioned() {
    let first = discover().expect("built-in capability discovery");
    let second = discover().expect("repeat capability discovery");
    assert_eq!(first, second);
    assert_eq!(first.schema_version, CapabilityManifest::SCHEMA_VERSION);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert!(
        first
            .features
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
    );
    assert!(
        first
            .formats
            .windows(2)
            .all(|pair| pair[0].format.id < pair[1].format.id)
    );
}

#[cfg(feature = "pdf")]
#[test]
fn manifest_reports_versions_policies_requirements_and_unavailability() {
    let manifest = discover().unwrap();
    let text = manifest
        .formats
        .iter()
        .find(|format| format.format.id == "text")
        .unwrap();
    assert!(text.available);
    assert!(
        text.payload_schemas
            .iter()
            .any(|schema| schema.version == SchemaVersion::TEXT_V2)
    );
    let pdf = manifest
        .formats
        .iter()
        .find(|format| format.format.id == "pdf")
        .unwrap();
    assert!(pdf.available);
    assert!(pdf.allowed_providers.contains(&ProviderKind::Ocr));
    assert!(manifest.backends.iter().all(|backend| {
        !backend.implementation.is_empty() && !backend.implementation_version.is_empty()
    }));
    assert_eq!(manifest.budgets.axes, BudgetAxis::ALL);
    assert_eq!(manifest.budgets.profiles.len(), 2);
    assert!(!manifest.security.implicit_network_access);
    assert!(!manifest.security.active_content_execution);
    assert!(manifest.reconstruction.iter().all(|entry| !entry.supported));
    assert!(
        manifest
            .unsupported_capabilities
            .iter()
            .all(|entry| entry.id != "format.pdf")
    );
    assert_eq!(
        manifest.envelope_schema_versions,
        [SchemaVersion::ENVELOPE_V1, SchemaVersion::ENVELOPE_V2]
    );
}

#[test]
fn registered_fixture_backed_reconstruction_is_discoverable() {
    let parsers = builtin_parser_registry().unwrap();
    let providers = builtin_provider_registry();
    let digest = grist::core::sha256_hex(b"fixture");
    let claim = FormatReconstructionClaim {
        format: "text".to_string(),
        media_type: "text/plain".to_string(),
        package_profile: "plain-text-v1".to_string(),
        implementation: "fixture-reconstructor".to_string(),
        implementation_version: "1.0.0".to_string(),
        maximum_fidelity: ReconstructionFidelity::ByteIdentical,
        fixture_evidence: vec![ReconstructionFixtureEvidence {
            fixture_id: "text-minimal".to_string(),
            input_sha256: digest.clone(),
            expected_package_sha256: digest,
            verified_fidelity: ReconstructionFidelity::ByteIdentical,
        }],
    };
    let manifest = CapabilityManifest::from_registries(&parsers, &providers, vec![claim]).unwrap();
    let text = manifest
        .reconstruction
        .iter()
        .find(|entry| entry.format == "text")
        .unwrap();
    assert!(text.supported);
    assert_eq!(text.claims.len(), 1);
}

#[cfg(feature = "schemas")]
#[test]
fn generated_manifest_validates_against_checked_contract() {
    let value = serde_json::to_value(discover().unwrap()).unwrap();
    let report = grist::schema::validate_schema("capability-manifest", &value).unwrap();
    assert!(report.valid, "{:?}", report.issues);
}
