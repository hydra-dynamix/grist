use grist::container::{
    ArtifactCaptureOptions, ArtifactContent, ArtifactDisposition, ArtifactExtraction,
    ArtifactExtractionStatus, ArtifactMaterializationError, ArtifactMaterializationOutcome,
    ArtifactMetadata, ArtifactParent, ArtifactRelationship, ArtifactSafetyClassification,
    ArtifactStoreError, ContentAddressedArtifactReference, ContentAddressedArtifactResolver,
    ContentAddressedArtifactSink, EmbeddedArtifact, EmbeddedArtifactError, MaterializationRequest,
};
use grist::core::{
    BudgetSelection, BudgetTracker, ContentIdentity, IndexPosition, LocationComponent,
    ResourceBudget, SourceLocator,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct MemoryStore(Mutex<BTreeMap<String, Vec<u8>>>);

impl ContentAddressedArtifactSink for MemoryStore {
    fn store(
        &self,
        reference: &ContentAddressedArtifactReference,
        bytes: &[u8],
    ) -> Result<(), ArtifactStoreError> {
        reference
            .validate_bytes(bytes)
            .map_err(|error| ArtifactStoreError::new(error.to_string()))?;
        self.0
            .lock()
            .unwrap()
            .insert(reference.digest.clone(), bytes.to_vec());
        Ok(())
    }
}

impl ContentAddressedArtifactResolver for MemoryStore {
    fn resolve(
        &self,
        reference: &ContentAddressedArtifactReference,
    ) -> Result<Vec<u8>, ArtifactStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(&reference.digest)
            .cloned()
            .ok_or_else(|| ArtifactStoreError::new("missing content address"))
    }
}

struct WrongResolver;

impl ContentAddressedArtifactResolver for WrongResolver {
    fn resolve(
        &self,
        _reference: &ContentAddressedArtifactReference,
    ) -> Result<Vec<u8>, ArtifactStoreError> {
        Ok(b"wrong bytes".to_vec())
    }
}

fn metadata(filename: &str, media_type: &str) -> ArtifactMetadata {
    let locator = SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: filename.into(),
        member_index: IndexPosition::one_based(2).unwrap(),
    })
    .unwrap();
    ArtifactMetadata::new(
        ArtifactParent::new(
            ContentIdentity::for_raw_bytes(b"stable parent"),
            ArtifactRelationship::EmbeddedIn,
        ),
        locator,
        ArtifactDisposition::Attachment,
    )
    .with_declared_filename(filename)
    .with_media_type(media_type)
}

#[test]
fn identity_is_stable_across_inline_and_content_addressed_modes() {
    let store = MemoryStore::default();
    let bytes = b"the same child bytes";
    let inline = EmbeddedArtifact::capture(
        metadata("child.txt", "text/plain"),
        bytes,
        ArtifactCaptureOptions::new(1_024),
        None,
    )
    .unwrap();
    let external = EmbeddedArtifact::capture(
        metadata("child.txt", "text/plain"),
        bytes,
        ArtifactCaptureOptions::new(0),
        Some(&store),
    )
    .unwrap();

    assert_eq!(inline.identity, external.identity);
    assert!(matches!(inline.content, Some(ArtifactContent::Inline(_))));
    assert!(external.content_reference().is_some());
    assert_eq!(inline.inline_threshold_bytes(), Some(1_024));
    assert_eq!(external.inline_threshold_bytes(), Some(0));
    assert_eq!(
        store
            .resolve(external.content_reference().unwrap())
            .unwrap(),
        bytes
    );
    let mut value = serde_json::to_value(&external).unwrap();
    assert_eq!(value["content"]["inline_threshold_bytes"], 0);
    value["content"]["inline_threshold_bytes"] = serde_json::json!(bytes.len());
    assert!(serde_json::from_value::<EmbeddedArtifact>(value).is_err());
    assert!(matches!(
        EmbeddedArtifact::capture(
            metadata("child.txt", "text/plain"),
            bytes,
            ArtifactCaptureOptions::new(0),
            None,
        ),
        Err(EmbeddedArtifactError::ContentAddressedSinkRequired)
    ));
}

#[test]
fn filenames_are_metadata_and_each_terminal_status_is_serialized() {
    let dangerous_name = "../../outside/launch.exe";
    let executable = EmbeddedArtifact::capture(
        metadata(dangerous_name, "application/octet-stream"),
        b"MZ hostile but inert",
        ArtifactCaptureOptions::new(1_024),
        None,
    )
    .unwrap();
    assert_eq!(
        executable.safety.classification,
        ArtifactSafetyClassification::Executable
    );
    assert_eq!(
        executable.extraction.status,
        ArtifactExtractionStatus::Quarantined
    );
    assert_eq!(
        executable.declared_filename.as_deref(),
        Some(dangerous_name)
    );

    for status in [
        ArtifactExtractionStatus::InventoryOnly,
        ArtifactExtractionStatus::Skipped,
        ArtifactExtractionStatus::Encrypted,
        ArtifactExtractionStatus::Unsupported,
        ArtifactExtractionStatus::Rejected,
        ArtifactExtractionStatus::BudgetLimited,
        ArtifactExtractionStatus::Failed,
    ] {
        let record = EmbeddedArtifact::record_unavailable(
            metadata("child.bin", "application/octet-stream"),
            ArtifactExtraction::new(status, format!("artifact.status.{status:?}"))
                .with_message("auditable terminal reason")
                .with_diagnostic("container.child.terminal"),
        )
        .unwrap();
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["extraction"]["message"], "auditable terminal reason");
        assert_eq!(
            value["extraction"]["diagnostic_codes"][0],
            "container.child.terminal"
        );
        let round_trip: EmbeddedArtifact = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip.extraction.status, status);
    }
}

#[test]
fn deserialization_rejects_identity_and_safety_invariant_tampering() {
    let executable = EmbeddedArtifact::capture(
        metadata("launch.exe", "application/octet-stream"),
        b"MZ inert sample",
        ArtifactCaptureOptions::new(1_024),
        None,
    )
    .unwrap();
    let mut value = serde_json::to_value(&executable).unwrap();
    value["extraction"]["status"] = serde_json::json!("extracted");
    assert!(serde_json::from_value::<EmbeddedArtifact>(value).is_err());

    let passive = EmbeddedArtifact::capture(
        metadata("child.txt", "text/plain"),
        b"identity protected",
        ArtifactCaptureOptions::new(1_024),
        None,
    )
    .unwrap();
    let mut value = serde_json::to_value(&passive).unwrap();
    value["content"]["bytes"][0] = serde_json::json!(0);
    assert!(serde_json::from_value::<EmbeddedArtifact>(value).is_err());
}

#[test]
fn materialization_is_disabled_by_default_and_never_uses_declared_names() {
    let directory = TestDirectory::new();
    let artifact = EmbeddedArtifact::capture(
        metadata("../../escape.txt", "text/plain"),
        b"safe payload",
        ArtifactCaptureOptions::new(1_024),
        None,
    )
    .unwrap();

    let outcome = artifact
        .materialize(&MaterializationRequest::default(), None, None)
        .unwrap();
    assert_eq!(outcome, ArtifactMaterializationOutcome::Disabled);
    assert!(!directory.path.exists());

    let first = artifact
        .materialize(
            &MaterializationRequest::directory(&directory.path),
            None,
            None,
        )
        .unwrap();
    let (first_path, first_name) = materialized_path(first);
    assert_eq!(
        first_path.parent(),
        Some(directory.path.canonicalize().unwrap().as_path())
    );
    assert!(!first_name.contains("escape"));
    assert_eq!(fs::read(&first_path).unwrap(), b"safe payload");

    let second = artifact
        .materialize(
            &MaterializationRequest::directory(&directory.path),
            None,
            None,
        )
        .unwrap();
    let (second_path, _) = materialized_path(second);
    assert_ne!(first_path, second_path);
    assert_eq!(fs::read(&first_path).unwrap(), b"safe payload");
}

#[test]
fn unsafe_materialization_requires_a_second_opt_in_and_uses_quarantine_extension() {
    let directory = TestDirectory::new();
    let artifact = EmbeddedArtifact::capture(
        metadata("../../launch.exe", "application/x-msdownload"),
        b"MZ inert sample",
        ArtifactCaptureOptions::new(1_024),
        None,
    )
    .unwrap();
    let request = MaterializationRequest::directory(&directory.path);
    assert_eq!(
        artifact.materialize(&request, None, None).unwrap(),
        ArtifactMaterializationOutcome::BlockedBySafety {
            classification: ArtifactSafetyClassification::Executable,
        }
    );
    assert!(!directory.path.exists());

    let (path, name) = materialized_path(
        artifact
            .materialize(&request.allowing_unsafe(), None, None)
            .unwrap(),
    );
    assert!(name.ends_with(".quarantine"));
    assert_eq!(
        path.parent(),
        Some(directory.path.canonicalize().unwrap().as_path())
    );
    assert_eq!(fs::read(path).unwrap(), b"MZ inert sample");
}

#[test]
fn unknown_binary_materialization_requires_unsafe_opt_in() {
    let directory = TestDirectory::new();
    let artifact = EmbeddedArtifact::capture(
        metadata("opaque.bin", "application/octet-stream"),
        b"unclassified opaque bytes",
        ArtifactCaptureOptions::new(1_024),
        None,
    )
    .unwrap();
    assert_eq!(
        artifact.safety.classification,
        ArtifactSafetyClassification::Unknown
    );
    assert_eq!(
        artifact
            .materialize(
                &MaterializationRequest::directory(&directory.path),
                None,
                None,
            )
            .unwrap(),
        ArtifactMaterializationOutcome::BlockedBySafety {
            classification: ArtifactSafetyClassification::Unknown,
        }
    );
    assert!(!directory.path.exists());
}

#[test]
fn external_bytes_are_resolved_and_verified_before_any_write() {
    let store = MemoryStore::default();
    let artifact = EmbeddedArtifact::capture(
        metadata("large.txt", "text/plain"),
        b"external payload",
        ArtifactCaptureOptions::new(0),
        Some(&store),
    )
    .unwrap();
    let bad_directory = TestDirectory::new();
    assert!(matches!(
        artifact.materialize(
            &MaterializationRequest::directory(&bad_directory.path),
            Some(&WrongResolver),
            None,
        ),
        Err(ArtifactMaterializationError::ContentAddressMismatch)
    ));
    assert!(!bad_directory.path.exists());

    let good_directory = TestDirectory::new();
    let (path, _) = materialized_path(
        artifact
            .materialize(
                &MaterializationRequest::directory(&good_directory.path),
                Some(&store),
                None,
            )
            .unwrap(),
    );
    assert_eq!(fs::read(path).unwrap(), b"external payload");
}

#[test]
fn temporary_storage_budget_is_checked_before_directory_creation() {
    let directory = TestDirectory::new();
    let artifact = EmbeddedArtifact::capture(
        metadata("child.txt", "text/plain"),
        b"four",
        ArtifactCaptureOptions::new(10),
        None,
    )
    .unwrap();
    let mut resource_budget = ResourceBudget::trusted_unbounded();
    resource_budget.max_temporary_storage_bytes = Some(3);
    let tracker = BudgetTracker::new(&BudgetSelection::custom(resource_budget)).unwrap();

    assert!(matches!(
        artifact.materialize(
            &MaterializationRequest::directory(&directory.path),
            None,
            Some(&tracker),
        ),
        Err(ArtifactMaterializationError::Budget(_))
    ));
    assert!(!directory.path.exists());
}

#[cfg(feature = "schemas")]
#[test]
fn embedded_artifact_schema_is_discoverable() {
    let entry = grist::schema::list_schemas()
        .into_iter()
        .find(|entry| entry.name == "embedded-artifact")
        .unwrap();
    assert_eq!(
        entry.schema_version,
        grist::core::SchemaVersion::EMBEDDED_ARTIFACT_V1
    );
    let schema = grist::schema::schema_json("embedded-artifact").unwrap();
    assert_eq!(schema["title"], "EmbeddedArtifact");
}

fn materialized_path(outcome: ArtifactMaterializationOutcome) -> (PathBuf, String) {
    match outcome {
        ArtifactMaterializationOutcome::Materialized {
            path,
            generated_filename,
            ..
        } => (path, generated_filename),
        other => panic!("expected materialized outcome, got {other:?}"),
    }
}

static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            path: std::env::temp_dir().join(format!(
                "grist-embedded-artifact-{}-{id}",
                std::process::id()
            )),
        }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.path.exists() {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }
}

fn _assert_path_is_path(_path: &Path) {}
