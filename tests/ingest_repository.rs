use grist::core::{Limits, OperationStatus};
use grist::ingest::{
    RepoIngestOptions, RepositoryDisposition, RepositorySkipReason, SubmodulePolicy, SymlinkPolicy,
    ingest_repo,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "grist-ingest-repository-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create fixture root");
        Self { path }
    }

    fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture parent");
        }
        fs::write(path, bytes).expect("write fixture file");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(all(feature = "serialization", feature = "pdf"))]
#[test]
fn inventories_every_policy_outcome_and_parses_supported_manifests() {
    let fixture = Fixture::new("inventory");
    fixture.write("vendor/dep/src/lib.rs", b"pub fn vendored() {}\n");
    fixture.write("vendor/dep/.git", b"gitdir: elsewhere\n");
    fixture.write("ignored-dir/nested.txt", b"ignored nested\n");
    fixture.write("ignored.txt", b"ignored\n");
    fixture.write("manual.pdf", b"%PDF-1.7\n");
    fixture.write("blob.bin", [0_u8, 1, 2, 3]);
    fixture.write("README.md", b"# deterministic\n");
    fixture.write("Dockerfile", b"FROM scratch\n");
    fixture.write("go.mod", b"module example.test/fixture\n");
    fixture.write("package-lock.json", br#"{"lockfileVersion":3}"#);
    fixture.write(".github/workflows/ci.yml", b"name: ci\non: [push]\n");
    fixture.write(
        "Cargo.toml",
        b"[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    );
    fixture.write(".gitignore", b"ignored.txt\nignored-dir/\n");

    let envelope = ingest_repo(&fixture.path, &RepoIngestOptions::default()).expect("ingest repo");
    let report = envelope.payload.as_ref().expect("repository payload");

    assert_eq!(envelope.status, OperationStatus::Partial);
    assert_eq!(report.ignored, ["ignored-dir/nested.txt", "ignored.txt"]);
    assert!(report.manifest_paths.contains(&"Cargo.toml".to_string()));
    assert!(report.manifest_paths.contains(&"Dockerfile".to_string()));
    assert!(report.manifest_paths.contains(&"go.mod".to_string()));
    assert!(
        report
            .manifest_paths
            .contains(&".github/workflows/ci.yml".to_string())
    );
    assert!(
        report
            .lockfile_paths
            .contains(&"package-lock.json".to_string())
    );
    assert!(
        report
            .artifacts
            .iter()
            .any(|artifact| artifact.path == "Cargo.toml")
    );
    assert!(!report.unsupported.contains(&"manual.pdf".to_string()));
    assert!(report.entries.iter().any(|entry| {
        entry.path == "blob.bin" && entry.disposition == RepositoryDisposition::Binary
    }));
    assert!(report.entries.iter().any(|entry| {
        entry.path == "manual.pdf" && entry.disposition == RepositoryDisposition::Failed
    }));
    assert!(report.entries.iter().any(|entry| {
        entry.path == "vendor/dep"
            && entry
                .skip_reasons
                .contains(&RepositorySkipReason::SubmodulePolicy)
    }));
    assert!(
        !report
            .entries
            .iter()
            .any(|entry| entry.path == "vendor/dep/src/lib.rs")
    );
    assert!(
        report
            .entries
            .windows(2)
            .all(|entries| entries[0].path <= entries[1].path)
    );
    assert!(
        report
            .aggregate_hashes
            .content_sha256
            .starts_with("sha256:")
    );
    assert!(
        report
            .skipped
            .iter()
            .all(|skipped| !skipped.reason.is_empty())
    );
}

#[test]
fn aggregate_hashes_ignore_creation_order_and_absolute_root() {
    let first = Fixture::new("stable-a");
    let second = Fixture::new("stable-b");
    first.write("z.json", br#"{"z":1}"#);
    first.write("a.md", b"# A\n");
    second.write("a.md", b"# A\n");
    second.write("z.json", br#"{"z":1}"#);

    let first_report = ingest_repo(&first.path, &RepoIngestOptions::default())
        .expect("first ingest")
        .payload
        .expect("first payload");
    let second_report = ingest_repo(&second.path, &RepoIngestOptions::default())
        .expect("second ingest")
        .payload
        .expect("second payload");

    assert_eq!(first_report.files, second_report.files);
    assert_eq!(
        first_report.aggregate_hashes,
        second_report.aggregate_hashes
    );
}

#[test]
fn ignored_entries_can_be_included_and_overlapping_reasons_are_retained() {
    let fixture = Fixture::new("ignore-override");
    fixture.write(".gitignore", b"ignored.txt\n");
    fixture.write("ignored.txt", b"retained when requested\n");

    let excluded = ingest_repo(
        &fixture.path,
        &RepoIngestOptions {
            exclude_globs: vec!["ignored.txt".into()],
            ..RepoIngestOptions::default()
        },
    )
    .expect("excluded ingest")
    .payload
    .expect("excluded payload");
    let entry = excluded
        .entries
        .iter()
        .find(|entry| entry.path == "ignored.txt")
        .expect("ignored inventory entry");
    assert_eq!(
        entry.skip_reasons,
        [
            RepositorySkipReason::IgnoreRule,
            RepositorySkipReason::ExcludeGlob
        ]
    );

    let included = ingest_repo(
        &fixture.path,
        &RepoIngestOptions {
            include_ignored: true,
            ..RepoIngestOptions::default()
        },
    )
    .expect("included ingest")
    .payload
    .expect("included payload");
    assert!(included.files.iter().any(|file| file.path == "ignored.txt"));
    assert!(!included.ignored.contains(&"ignored.txt".to_string()));
}

#[test]
fn budget_limited_paths_remain_in_the_inventory() {
    let fixture = Fixture::new("budgets");
    fixture.write("a.txt", b"a");
    fixture.write("b.txt", b"b");
    fixture.write("c.txt", b"c");
    let limits = Limits {
        max_repo_files: 1,
        ..Limits::default()
    };

    let envelope = ingest_repo(
        &fixture.path,
        &RepoIngestOptions {
            limits,
            ..RepoIngestOptions::default()
        },
    )
    .expect("budgeted ingest");
    let report = envelope.payload.as_ref().expect("partial payload");

    assert_eq!(envelope.status, OperationStatus::Partial);
    assert_eq!(report.files.len(), 1);
    assert_eq!(
        report
            .entries
            .iter()
            .filter(|entry| entry.disposition == RepositoryDisposition::BudgetLimited)
            .count(),
        2
    );
    assert!(report.entries.iter().any(|entry| {
        entry
            .skip_reasons
            .contains(&RepositorySkipReason::RepositoryFileLimit)
    }));
}

#[test]
fn file_size_and_traversal_depth_limits_retain_named_entries() {
    let fixture = Fixture::new("size-depth");
    fixture.write("large.txt", b"too large");
    fixture.write("nested/deeper.txt", b"not traversed");
    let limits = Limits {
        max_file_bytes: 3,
        max_parse_depth: 0,
        ..Limits::default()
    };

    let report = ingest_repo(
        &fixture.path,
        &RepoIngestOptions {
            limits,
            ..RepoIngestOptions::default()
        },
    )
    .expect("limited ingest")
    .payload
    .expect("partial payload");

    assert!(report.entries.iter().any(|entry| {
        entry.path == "large.txt"
            && entry
                .skip_reasons
                .contains(&RepositorySkipReason::FileSizeLimit)
    }));
    assert!(report.entries.iter().any(|entry| {
        entry.path == "nested"
            && entry
                .skip_reasons
                .contains(&RepositorySkipReason::TraversalDepthLimit)
    }));
}

#[test]
fn external_artifact_mode_is_content_addressed_and_does_not_ingest_itself() {
    let fixture = Fixture::new("external-artifacts");
    fixture.write("doc.md", b"# artifact\n");
    let output = fixture.path.join("artifact-output");
    let options = RepoIngestOptions {
        inline_artifacts: false,
        external_artifact_dir: Some(output.clone()),
        ..RepoIngestOptions::default()
    };

    let first = ingest_repo(&fixture.path, &options)
        .expect("first external ingest")
        .payload
        .expect("first payload");
    let second = ingest_repo(&fixture.path, &options)
        .expect("second external ingest")
        .payload
        .expect("second payload");

    assert_eq!(first.aggregate_hashes, second.aggregate_hashes);
    assert!(first.artifacts.iter().all(|artifact| {
        artifact.artifact.is_none()
            && artifact.artifact_ref.is_some()
            && artifact.canonical_payload_hash.is_some()
    }));
    assert!(first.entries.iter().any(|entry| {
        entry.path == "artifact-output"
            && entry
                .skip_reasons
                .contains(&RepositorySkipReason::ExternalArtifactOutput)
    }));
    assert!(
        fs::read_dir(output)
            .expect("artifact directory")
            .all(|entry| entry
                .expect("artifact entry")
                .path()
                .extension()
                .is_some_and(|extension| extension == "json"))
    );
}

#[test]
fn submodule_traversal_is_an_explicit_opt_in() {
    let fixture = Fixture::new("submodule");
    fixture.write("dep/.git", b"gitdir: elsewhere\n");
    fixture.write("dep/src/lib.rs", b"pub fn inside() {}\n");

    let report = ingest_repo(
        &fixture.path,
        &RepoIngestOptions {
            submodule_policy: SubmodulePolicy::Traverse,
            ..RepoIngestOptions::default()
        },
    )
    .expect("traversing ingest")
    .payload
    .expect("payload");

    assert!(
        report
            .files
            .iter()
            .any(|file| file.path == "dep/src/lib.rs")
    );
    assert!(report.entries.iter().any(|entry| {
        entry.path == "dep" && entry.disposition == RepositoryDisposition::Traversed
    }));
    assert!(report.entries.iter().any(|entry| {
        entry.path == "dep/.git"
            && entry
                .skip_reasons
                .contains(&RepositorySkipReason::RepositoryMetadata)
    }));
}

#[test]
fn followed_symlinks_cannot_escape_the_explicit_root() {
    let fixture = Fixture::new("symlink-root");
    let outside = Fixture::new("symlink-outside");
    outside.write("outside.md", b"# outside\n");
    let link = fixture.path.join("escape.md");
    if !create_file_symlink(&outside.path.join("outside.md"), &link) {
        return;
    }

    let envelope = ingest_repo(
        &fixture.path,
        &RepoIngestOptions {
            symlink_policy: SymlinkPolicy::FollowFilesWithinRoot,
            ..RepoIngestOptions::default()
        },
    )
    .expect("symlink ingest");
    let report = envelope.payload.as_ref().expect("payload");

    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(report.entries.iter().any(|entry| {
        entry.path == "escape.md"
            && entry
                .skip_reasons
                .contains(&RepositorySkipReason::SymlinkEscapesRoot)
    }));
    assert!(!report.files.iter().any(|file| file.path == "escape.md"));
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(not(any(unix, windows)))]
fn create_file_symlink(_target: &Path, _link: &Path) -> bool {
    false
}
