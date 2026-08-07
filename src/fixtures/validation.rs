//! Semantic, path-safety, identity, and canonical-golden validation.

use super::{
    BuilderKind, CORPUS_MANIFEST_SCHEMA_VERSION, EXPECTED_OUTPUT_POLICY_VERSION,
    ExpectedNormalization, FixtureClass, FixtureCorpusManifest, FixtureOrigin, FixtureStorage,
    REQUIRED_FIXTURE_CLASSES, Redistribution,
};
use crate::core::{CanonicalJsonVersion, canonical_json_bytes, sha256_hex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub const CORPUS_VALIDATION_REPORT_SCHEMA_VERSION: &str = "grist/corpus-validation-report/v1";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorpusViolation {
    pub code: String,
    pub location: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CorpusLoadError {
    #[error("fixture corpus manifest could not be read: {0}")]
    Read(#[source] std::io::Error),
    #[error("fixture corpus manifest is not valid JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("fixture corpus validation failed with {0} violation(s)")]
    Invalid(usize),
}

/// Load and fully verify the checked corpus rooted at `<repository>/fixtures`.
pub fn load_corpus(
    repository_root: impl AsRef<Path>,
) -> Result<FixtureCorpusManifest, CorpusLoadError> {
    let fixtures_root = repository_root.as_ref().join("fixtures");
    let bytes = fs::read(fixtures_root.join("corpus.v1.json")).map_err(CorpusLoadError::Read)?;
    let manifest = serde_json::from_slice(&bytes).map_err(CorpusLoadError::Json)?;
    let report = validate_corpus_at(&fixtures_root, &manifest);
    if report.is_valid() {
        Ok(manifest)
    } else {
        Err(CorpusLoadError::Invalid(report.violations.len()))
    }
}

/// Validate manifest semantics and every checked-in byte identity.
pub fn validate_corpus_at(
    fixtures_root: impl AsRef<Path>,
    manifest: &FixtureCorpusManifest,
) -> CorpusValidationReport {
    let root = fixtures_root.as_ref();
    let mut report = CorpusValidationReport::default();
    validate_policy(manifest, &mut report);
    if manifest.formats.is_empty() {
        report.push(
            "grist.fixture.formats.empty",
            "formats",
            "the governed corpus must register at least one format",
        );
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for (format_key, format) in &manifest.formats {
        let format_location = format!("formats.{format_key}");
        if format.format != *format_key || !valid_slug(format_key) {
            report.push(
                "grist.fixture.format.invalid",
                &format_location,
                "format key must be a lowercase slug identical to format.format",
            );
        }
        for case in &format.cases {
            validate_case(
                root,
                &format_location,
                case,
                &mut ids,
                &mut paths,
                &mut report,
            );
        }
    }
    report
}

fn validate_case(
    root: &Path,
    format_location: &str,
    case: &super::FixtureCase,
    ids: &mut BTreeSet<String>,
    paths: &mut BTreeSet<String>,
    report: &mut CorpusValidationReport,
) {
    let location = format!("{format_location}.cases.{}", case.id);
    if !valid_slug(&case.id) || !ids.insert(case.id.clone()) {
        report.push(
            "grist.fixture.id.invalid_or_duplicate",
            &location,
            "fixture IDs must be unique lowercase slugs",
        );
    }
    let classes = case.classes.iter().copied().collect::<BTreeSet<_>>();
    if classes.is_empty() || classes.len() != case.classes.len() {
        report.push(
            "grist.fixture.classes.invalid",
            &location,
            "classes must be non-empty and contain no duplicates",
        );
    }
    validate_provenance(root, case, &location, report);
    validate_builder(case, &location, report);
    validate_file(
        root,
        &case.input.path,
        case.input.byte_length,
        &case.input.sha256,
        &format!("{location}.input"),
        paths,
        report,
    );
    for expected in &case.expected {
        validate_expected(root, expected, &location, paths, report);
    }
}

fn validate_policy(manifest: &FixtureCorpusManifest, report: &mut CorpusValidationReport) {
    if manifest.schema_version != CORPUS_MANIFEST_SCHEMA_VERSION {
        report.push(
            "grist.fixture.schema_version.unsupported",
            "schema_version",
            format!("expected {CORPUS_MANIFEST_SCHEMA_VERSION}"),
        );
    }
    let actual = manifest
        .policy
        .required_classes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let required = REQUIRED_FIXTURE_CLASSES
        .into_iter()
        .collect::<BTreeSet<_>>();
    if actual != required || actual.len() != manifest.policy.required_classes.len() {
        report.push(
            "grist.fixture.policy.required_classes_incomplete",
            "policy.required_classes",
            "policy must enumerate every Section 16.1 class exactly once",
        );
    }
    if manifest.policy.expected_output_policy != EXPECTED_OUTPUT_POLICY_VERSION
        || manifest.policy.canonicalization != CanonicalJsonVersion::CURRENT
        || manifest.policy.checked_in_data_classification != "public"
        || manifest.policy.active_content_execution_allowed
        || manifest.policy.implicit_network_allowed
    {
        report.push(
            "grist.fixture.policy.unsafe",
            "policy",
            "canonical v1 output, public-only check-in, no execution, and no implicit network are mandatory",
        );
    }
}

fn validate_provenance(
    root: &Path,
    case: &super::FixtureCase,
    location: &str,
    report: &mut CorpusValidationReport,
) {
    let checked_in = case.handling.storage == FixtureStorage::CheckedIn;
    if case.provenance.source.trim().is_empty()
        || case.handling.data_classification.trim().is_empty()
    {
        report.push(
            "grist.fixture.provenance.incomplete",
            location,
            "source and data classification are required",
        );
    }
    if case.handling.execution_allowed || case.handling.network_allowed || !case.handling.inert_only
    {
        report.push(
            "grist.fixture.handling.unsafe",
            location,
            "fixtures are inert and may never grant execution or network access",
        );
    }
    if checked_in
        && (case.handling.data_classification != "public"
            || case.provenance.license.redistribution != Redistribution::Permitted
            || case.input.path.is_none())
    {
        report.push(
            "grist.fixture.checkin.not_permitted",
            location,
            "checked-in bytes must be public, redistributable, and have a corpus-relative path",
        );
    }
    if !checked_in && case.input.path.is_some() {
        report.push(
            "grist.fixture.external.path_present",
            location,
            "external-only fixtures record identity metadata but no repository path",
        );
    }
    if case.provenance.license.expression.trim().is_empty()
        || (case
            .provenance
            .license
            .expression
            .starts_with("LicenseRef-")
            && case.provenance.license.license_file.is_none())
    {
        report.push(
            "grist.fixture.license.incomplete",
            location,
            "an SPDX expression or LicenseRef plus license file is required",
        );
    }
    if let Some(license_file) = &case.provenance.license.license_file
        && (!safe_relative_path(license_file)
            || contains_symlink(root, license_file)
            || !root.join(license_file).is_file())
    {
        report.push(
            "grist.fixture.license.file_invalid",
            location,
            "license_file must be a checked, non-symlink path beneath fixtures/",
        );
    }
    validate_origin(case, location, report);
}

fn validate_origin(case: &super::FixtureCase, location: &str, report: &mut CorpusValidationReport) {
    match case.provenance.origin {
        FixtureOrigin::Licensed => {
            if case.provenance.source_uri.is_none() {
                report.push(
                    "grist.fixture.licensed.source_missing",
                    location,
                    "licensed fixtures require an upstream source URI",
                );
            }
        }
        FixtureOrigin::Malicious => {
            if !case.classes.contains(&FixtureClass::Adversarial)
                || !case.classes.contains(&FixtureClass::MaliciousActiveContent)
            {
                report.push(
                    "grist.fixture.malicious.class_missing",
                    location,
                    "malicious fixtures require adversarial and malicious_active_content classes",
                );
            }
        }
        FixtureOrigin::ProviderRecording => {
            if !case.classes.contains(&FixtureClass::ProviderRecording) {
                report.push(
                    "grist.fixture.provider.class_missing",
                    location,
                    "provider recordings require the provider_recording class",
                );
            }
        }
        FixtureOrigin::DownstreamRegression => {
            if !case.classes.contains(&FixtureClass::DownstreamRegression)
                || case.provenance.source_project.is_none()
                || case.provenance.issue_uri.is_none()
                || case.provenance.original_sha256.is_none()
            {
                report.push(
                    "grist.fixture.regression.provenance_incomplete",
                    location,
                    "downstream regressions require class, project, issue URI, and original identity",
                );
            }
        }
        FixtureOrigin::Synthetic => {}
    }
}

fn validate_builder(
    case: &super::FixtureCase,
    location: &str,
    report: &mut CorpusValidationReport,
) {
    let requires_builder = case.classes.iter().any(|class| {
        matches!(
            class,
            FixtureClass::MaximumComplexity
                | FixtureClass::DeeplyNested
                | FixtureClass::NestedAttachments
                | FixtureClass::NestedContainers
                | FixtureClass::Oversized
                | FixtureClass::ProviderRecording
        )
    });
    let Some(builder) = &case.builder else {
        if requires_builder && case.handling.storage == FixtureStorage::CheckedIn {
            report.push(
                "grist.fixture.builder.missing",
                location,
                "generated complex, nested, oversized, and provider fixtures require builder provenance",
            );
        }
        return;
    };
    if builder.command.is_empty()
        || builder.command.iter().any(|part| part.trim().is_empty())
        || builder.generator_version.trim().is_empty()
        || builder.seed.trim().is_empty()
        || !safe_relative_path(&builder.recipe_path)
    {
        report.push(
            "grist.fixture.builder.invalid",
            location,
            "builder requires argv, version, seed, and a safe recipe path",
        );
    }
    if !builder_matches_classes(builder.kind, &case.classes) {
        report.push(
            "grist.fixture.builder.class_mismatch",
            location,
            "builder kind must agree with the registered fixture classes",
        );
    }
}

fn builder_matches_classes(kind: BuilderKind, classes: &[FixtureClass]) -> bool {
    match kind {
        BuilderKind::MaximumComplexity => classes.contains(&FixtureClass::MaximumComplexity),
        BuilderKind::NestedContainer => classes.iter().any(|class| {
            matches!(
                class,
                FixtureClass::DeeplyNested
                    | FixtureClass::NestedAttachments
                    | FixtureClass::NestedContainers
            )
        }),
        BuilderKind::ProviderRecording => classes.contains(&FixtureClass::ProviderRecording),
        BuilderKind::SyntheticBytes => true,
    }
}

fn validate_expected(
    root: &Path,
    expected: &super::ExpectedOutput,
    case_location: &str,
    paths: &mut BTreeSet<String>,
    report: &mut CorpusValidationReport,
) {
    let location = format!("{case_location}.expected.{}", expected.surface);
    if expected.surface.trim().is_empty()
        || expected.schema_version.trim().is_empty()
        || expected.canonicalization != CanonicalJsonVersion::CURRENT
        || !valid_sha256(&expected.canonical_sha256)
    {
        report.push(
            "grist.fixture.expected.contract_invalid",
            &location,
            "surface, schema version, canonical JSON v1, and canonical SHA-256 are required",
        );
    }
    if expected.normalization.is_empty()
        || (expected.normalization.len() > 1
            && expected
                .normalization
                .contains(&ExpectedNormalization::Exact))
    {
        report.push(
            "grist.fixture.expected.normalization_invalid",
            &location,
            "normalization must explicitly be [exact] or a non-empty rule list",
        );
    }
    if expected.normalization.iter().any(|rule| match rule {
        ExpectedNormalization::Exact => false,
        ExpectedNormalization::RemoveCallerTimestamp { json_pointer }
        | ExpectedNormalization::ReplaceFixtureRoot { json_pointer } => {
            !valid_json_pointer(json_pointer)
        }
    }) {
        report.push(
            "grist.fixture.expected.pointer_invalid",
            &location,
            "normalization JSON pointers must be non-root RFC 6901 pointers",
        );
    }
    validate_file(
        root,
        &Some(expected.path.clone()),
        expected.byte_length,
        &expected.sha256,
        &location,
        paths,
        report,
    );
    let Ok(bytes) = fs::read(root.join(&expected.path)) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        report.push(
            "grist.fixture.expected.not_json",
            &location,
            "expected outputs must be canonical JSON",
        );
        return;
    };
    let Ok(canonical) = canonical_json_bytes(&value) else {
        report.push(
            "grist.fixture.expected.canonicalization_failed",
            &location,
            "expected output could not be canonicalized",
        );
        return;
    };
    let mut canonical_file = canonical.clone();
    canonical_file.push(b'\n');
    if bytes != canonical_file || sha256_hex(&canonical) != expected.canonical_sha256 {
        report.push(
            "grist.fixture.expected.not_canonical",
            &location,
            "golden file must be canonical JSON v1 plus one LF and match canonical_sha256",
        );
    }
}

fn validate_file(
    root: &Path,
    path: &Option<String>,
    byte_length: u64,
    sha256: &str,
    location: &str,
    seen_paths: &mut BTreeSet<String>,
    report: &mut CorpusValidationReport,
) {
    if !valid_sha256(sha256) {
        report.push(
            "grist.fixture.identity.invalid",
            location,
            "SHA-256 must be lowercase hexadecimal with a sha256: prefix",
        );
    }
    let Some(relative) = path else {
        return;
    };
    if !safe_relative_path(relative) || !seen_paths.insert(relative.clone()) {
        report.push(
            "grist.fixture.path.invalid_or_duplicate",
            location,
            "checked paths must be unique normalized paths beneath fixtures/",
        );
        return;
    }
    if contains_symlink(root, relative) {
        report.push(
            "grist.fixture.path.symlink",
            location,
            "fixture paths may not traverse symlinks",
        );
        return;
    }
    validate_file_identity(root.join(relative), byte_length, sha256, location, report);
}

fn validate_file_identity(
    path: PathBuf,
    byte_length: u64,
    sha256: &str,
    location: &str,
    report: &mut CorpusValidationReport,
) {
    match fs::read(path) {
        Ok(bytes) => {
            let actual_length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            let actual_sha = sha256_hex(&bytes);
            if actual_length != byte_length || actual_sha != sha256 {
                report.push(
                    "grist.fixture.identity.mismatch",
                    location,
                    format!(
                        "declared {byte_length} bytes/{sha256}, found {actual_length} bytes/{actual_sha}"
                    ),
                );
            }
        }
        Err(error) => report.push("grist.fixture.path.unreadable", location, error.to_string()),
    }
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn contains_symlink(root: &Path, relative: &str) -> bool {
    let mut current = PathBuf::from(root);
    for component in Path::new(relative).components() {
        let Component::Normal(component) = component else {
            return true;
        };
        current.push(component);
        if fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return true;
        }
    }
    false
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn valid_json_pointer(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && value.split('/').skip(1).all(|token| {
            let mut chars = token.chars();
            while let Some(character) = chars.next() {
                if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
                    return false;
                }
            }
            true
        })
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorpusValidationReport {
    pub schema_version: String,
    pub violations: Vec<CorpusViolation>,
}

impl Default for CorpusValidationReport {
    fn default() -> Self {
        Self {
            schema_version: CORPUS_VALIDATION_REPORT_SCHEMA_VERSION.into(),
            violations: Vec::new(),
        }
    }
}

impl CorpusValidationReport {
    pub fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    fn push(
        &mut self,
        code: impl Into<String>,
        location: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.violations.push(CorpusViolation {
            code: code.into(),
            location: location.into(),
            message: message.into(),
        });
    }
}
