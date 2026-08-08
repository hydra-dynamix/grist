#![cfg(feature = "archives")]

use grist::archive::{ArchiveFormat, ArchiveOptions, archive_format, builtin_decoder_registry};
use grist::container::{
    ArtifactContent, ArtifactStoreError, ContainerArtifactMode, ContainerChild,
    ContainerChildStatus, ContainerParseOptions, ContainerParseRequest, ContainerRecursor,
    ContentAddressedArtifactReference, ContentAddressedArtifactSink,
};
use grist::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Input, Limits, OperationStatus, ParseRequest,
    ProviderSet, RequestId, ResourceBudget, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{DocumentGraphContext, ToDocumentGraph};
use grist::ingest::Ingestor;
use grist::registry::builtin_parser_registry;
use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Mutex;
use tar::{Builder, EntryType, Header};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn zip_bytes(entries: &[(&str, &[u8], CompressionMethod)], zip64: bool) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes, method) in entries {
        let options = SimpleFileOptions::default()
            .compression_method(*method)
            .large_file(zip64);
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn patch_zip_method(bytes: &mut [u8], entry_index: usize, method: u16) {
    let mut local_index = 0_usize;
    let mut central_index = 0_usize;
    for offset in 0..bytes.len().saturating_sub(12) {
        if &bytes[offset..offset + 4] == b"PK\x03\x04" {
            if local_index == entry_index {
                bytes[offset + 8..offset + 10].copy_from_slice(&method.to_le_bytes());
            }
            local_index += 1;
        } else if &bytes[offset..offset + 4] == b"PK\x01\x02" {
            if central_index == entry_index {
                bytes[offset + 10..offset + 12].copy_from_slice(&method.to_le_bytes());
            }
            central_index += 1;
        }
    }
}

fn mark_zip_symlink(bytes: &mut [u8], entry_index: usize) {
    let mut central_index = 0_usize;
    for offset in 0..bytes.len().saturating_sub(42) {
        if &bytes[offset..offset + 4] != b"PK\x01\x02" {
            continue;
        }
        if central_index == entry_index {
            bytes[offset + 5] = 3;
            let attributes = (0o120777_u32) << 16;
            bytes[offset + 38..offset + 42].copy_from_slice(&attributes.to_le_bytes());
            return;
        }
        central_index += 1;
    }
    panic!("ZIP central-directory entry {entry_index} not found");
}

fn corrupt_zip_payload(bytes: &mut [u8], entry_index: usize) {
    let mut local_index = 0_usize;
    for offset in 0..bytes.len().saturating_sub(30) {
        if &bytes[offset..offset + 4] != b"PK\x03\x04" {
            continue;
        }
        if local_index == entry_index {
            let name_len =
                usize::from(u16::from_le_bytes([bytes[offset + 26], bytes[offset + 27]]));
            let extra_len =
                usize::from(u16::from_le_bytes([bytes[offset + 28], bytes[offset + 29]]));
            let payload = offset + 30 + name_len + extra_len;
            bytes[payload] ^= 1;
            return;
        }
        local_index += 1;
    }
    panic!("ZIP local entry {entry_index} not found");
}

fn append_tar(
    builder: &mut Builder<Cursor<Vec<u8>>>,
    path: &str,
    kind: EntryType,
    body: &[u8],
    link: Option<&str>,
) {
    let mut header = Header::new_gnu();
    header.set_entry_type(kind);
    header.set_mode(0o644);
    header.set_uid(1000);
    header.set_gid(1000);
    header.set_mtime(1_700_000_000);
    header.set_size(body.len() as u64);
    header.set_path(path).unwrap();
    if let Some(link) = link {
        header.set_link_name(link).unwrap();
    }
    header.set_cksum();
    builder.append(&header, body).unwrap();
}

fn tar_bytes(entries: &[(&str, EntryType, &[u8], Option<&str>)]) -> Vec<u8> {
    let mut builder = Builder::new(Cursor::new(Vec::new()));
    for (path, kind, bytes, link) in entries {
        append_tar(&mut builder, path, *kind, bytes, *link);
    }
    builder.finish().unwrap();
    builder.into_inner().unwrap().into_inner()
}

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

fn traverse(
    bytes: Vec<u8>,
    format: &str,
    mode: ContainerArtifactMode,
    budget: BudgetSelection,
    sink: Option<&dyn ContentAddressedArtifactSink>,
) -> grist::container::ContainerTraversal {
    let ingestor = Ingestor::builtin().unwrap();
    let decoders = builtin_decoder_registry().unwrap();
    ContainerRecursor::new(&ingestor, &decoders)
        .parse(
            ContainerParseRequest::new(
                RequestId::new("archive-test").unwrap(),
                bytes,
                SourceInfo::new(format!("fixture.{format}")),
                format,
                ContainerParseOptions::new(mode).without_leaf_payloads(),
                budget,
            ),
            sink,
        )
        .unwrap()
}

#[cfg(feature = "manifests")]
fn traverse_with_leaf_payloads(
    bytes: Vec<u8>,
    format: &str,
) -> grist::container::ContainerTraversal {
    let ingestor = Ingestor::builtin().unwrap();
    let decoders = builtin_decoder_registry().unwrap();
    ContainerRecursor::new(&ingestor, &decoders)
        .parse(
            ContainerParseRequest::new(
                RequestId::new("archive-manifest-leaf-test").unwrap(),
                bytes,
                SourceInfo::new(format!("manifest-fixture.{format}")),
                format,
                ContainerParseOptions::new(ContainerArtifactMode::InlinePayload),
                BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
            ),
            None,
        )
        .unwrap()
}

fn ids(children: &[ContainerChild]) -> Vec<String> {
    children
        .iter()
        .flat_map(|child| {
            std::iter::once(child.artifact.identity.artifact_id.clone()).chain(ids(&child.children))
        })
        .collect()
}

#[cfg(feature = "manifests")]
#[test]
fn zip_and_tar_leaf_payloads_route_project_and_infrastructure_files_to_manifests() {
    let entries = [
        (
            "package.json",
            b"{\"dependencies\":{\"serde\":\"1\"}}".as_slice(),
        ),
        ("pom.xml", b"<project><dependencies/></project>".as_slice()),
        (
            "Cargo.toml",
            b"[package]\nname = \"demo\"\nversion = \"0.1.0\"\n".as_slice(),
        ),
        (
            ".github/workflows/ci.yml",
            b"name: ci\non: [push]\njobs: {}\n".as_slice(),
        ),
        (
            "deployment.yaml",
            b"apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: demo\n".as_slice(),
        ),
    ];
    let zip_entries = entries
        .iter()
        .map(|(path, body)| (*path, *body, CompressionMethod::Stored))
        .collect::<Vec<_>>();
    let tar_entries = entries
        .iter()
        .map(|(path, body)| (*path, EntryType::Regular, *body, None))
        .collect::<Vec<_>>();

    for traversal in [
        traverse_with_leaf_payloads(zip_bytes(&zip_entries, false), "zip"),
        traverse_with_leaf_payloads(tar_bytes(&tar_entries), "tar"),
    ] {
        assert_eq!(traversal.children.len(), entries.len());
        for child in &traversal.children {
            let parsed = child.parsed.as_ref().unwrap_or_else(|| {
                panic!(
                    "{} was not parsed",
                    child.artifact.declared_filename.as_deref().unwrap()
                )
            });
            assert_eq!(
                parsed.kind,
                ArtifactKind::Manifest,
                "{} was routed to {:?}",
                child.artifact.declared_filename.as_deref().unwrap(),
                parsed.kind
            );
        }
    }
}

#[test]
fn zip_inventory_preserves_order_hashes_compression_and_rejections() {
    let bytes = zip_bytes(
        &[
            ("first.txt", b"one", CompressionMethod::Stored),
            ("SECOND.txt", b"two", CompressionMethod::Deflated),
            ("second.txt", b"collision", CompressionMethod::Stored),
            ("../escape.bin", b"inert", CompressionMethod::Stored),
        ],
        false,
    );
    let result = traverse(
        bytes,
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(result.status, OperationStatus::Partial);
    assert_eq!(
        result
            .children
            .iter()
            .map(|child| child.source_order)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    let first = &result.children[0];
    assert_eq!(first.status, ContainerChildStatus::InventoryOnly);
    assert!(first.artifact.identity.content.raw.is_some());
    assert!(first.artifact.content.is_none());
    assert_eq!(
        first.archive_metadata.as_ref().unwrap().compression_method,
        "stored"
    );
    assert_eq!(
        result.children[1]
            .archive_metadata
            .as_ref()
            .unwrap()
            .compression_method,
        "deflated"
    );
    for collision in &result.children[1..=2] {
        assert_eq!(collision.status, ContainerChildStatus::Rejected);
        assert!(collision.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "grist.security.archive.duplicate_path"
        }));
    }
    assert_eq!(result.children[3].status, ContainerChildStatus::Rejected);
    assert!(
        result.children[3].diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "grist.security.archive.path_traversal"
        })
    );
}

#[test]
fn storage_modes_retain_identical_identities() {
    let bytes = zip_bytes(
        &[("member.bin", b"mode invariant", CompressionMethod::Deflated)],
        false,
    );
    let inventory = traverse(
        bytes.clone(),
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    let inline = traverse(
        bytes.clone(),
        "zip",
        ContainerArtifactMode::InlinePayload,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    let store = MemoryStore::default();
    let addressed = traverse(
        bytes,
        "zip",
        ContainerArtifactMode::ContentAddressed,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Some(&store),
    );
    assert_eq!(ids(&inventory.children), ids(&inline.children));
    assert_eq!(ids(&inventory.children), ids(&addressed.children));
    assert!(matches!(
        inline.children[0].artifact.content,
        Some(ArtifactContent::Inline(_))
    ));
    assert!(matches!(
        addressed.children[0].artifact.content,
        Some(ArtifactContent::ContentAddressed { .. })
    ));
    assert_eq!(
        inventory.children[0].archive_metadata,
        inline.children[0].archive_metadata
    );
}

#[test]
fn tar_metadata_links_devices_and_nested_archives_are_inventory_only() {
    let nested = zip_bytes(
        &[("nested.txt", b"inside", CompressionMethod::Stored)],
        false,
    );
    let bytes = tar_bytes(&[
        ("nested.zip", EntryType::Regular, &nested, None),
        ("alias", EntryType::Symlink, b"", Some("nested.zip")),
        ("device", EntryType::Char, b"", None),
    ]);
    let result = traverse(
        bytes,
        "tar",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(result.children.len(), 3);
    assert_eq!(result.children[0].children.len(), 1);
    assert_eq!(
        result.children[0].archive_metadata.as_ref().unwrap().uid,
        Some(1000)
    );
    assert_eq!(result.children[1].status, ContainerChildStatus::Rejected);
    assert_eq!(
        result.children[1]
            .archive_metadata
            .as_ref()
            .unwrap()
            .link_target
            .as_deref(),
        Some("nested.zip")
    );
    assert_eq!(result.children[2].status, ContainerChildStatus::Rejected);
    assert!(
        result.children[2]
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.security.archive.device_file" })
    );
}

#[test]
fn preflight_limits_malformed_and_encrypted_members_are_explicit() {
    let bytes = zip_bytes(
        &[
            ("one", b"111111", CompressionMethod::Deflated),
            ("two", b"222222", CompressionMethod::Deflated),
        ],
        false,
    );
    let mut member_budget = ResourceBudget::trusted_unbounded();
    member_budget.max_archive_members = Some(1);
    let limited = traverse(
        bytes.clone(),
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::custom(member_budget),
        None,
    );
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_members.exhausted"
    }));

    let mut ratio_budget = ResourceBudget::trusted_unbounded();
    ratio_budget.max_archive_expansion_ratio = Some(0.01);
    let bomb = traverse(
        bytes,
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::custom(ratio_budget),
        None,
    );
    assert_eq!(bomb.status, OperationStatus::Failed);
    assert!(bomb.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_expansion_ratio.exhausted"
    }));

    let malformed = traverse(
        b"PK\x03\x04broken".to_vec(),
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(
        malformed
            .diagnostics
            .iter()
            .any(|item| { item.class == grist::core::DiagnosticClass::MalformedInput })
    );

    let mut encrypted = zip_bytes(
        &[("secret.txt", b"secret", CompressionMethod::Stored)],
        false,
    );
    for offset in 0..encrypted.len().saturating_sub(4) {
        if &encrypted[offset..offset + 4] == b"PK\x03\x04" {
            encrypted[offset + 6] |= 1;
        } else if &encrypted[offset..offset + 4] == b"PK\x01\x02" {
            encrypted[offset + 8] |= 1;
        }
    }
    let encrypted = traverse(
        encrypted,
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(
        encrypted.children[0].status,
        ContainerChildStatus::Encrypted
    );
    assert!(
        encrypted.children[0]
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.archive.encrypted_member" })
    );
}

#[test]
fn zip64_registry_graph_schema_and_detection_surfaces_agree() {
    let bytes = zip_bytes(
        &[("large-declared.txt", b"small", CompressionMethod::Stored)],
        true,
    );
    assert_eq!(archive_format(&bytes, None), Some(ArchiveFormat::Zip64));
    let detection = detect_with_registry(
        Path::new("fixture.bin"),
        &bytes,
        None,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Zip);

    let registry = builtin_parser_registry().unwrap();
    let request = ParseRequest::new(
        RequestId::new("archive-registry").unwrap(),
        Input::bytes(bytes),
        SourceInfo::new("fixture.zip"),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );
    let envelope = registry.dispatch("zip", request, None).unwrap();
    assert_eq!(envelope.kind, ArtifactKind::Archive);
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document: grist::archive::ArchiveDocument =
        serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert_eq!(document.format, ArchiveFormat::Zip64);
    let graph = document
        .to_document_graph(DocumentGraphContext::new("archive-graph"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| { node.kind == grist::document_graph::DocumentNodeKind::ArchiveMember })
    );

    #[cfg(feature = "schemas")]
    for schema in ["archive", "archive-envelope", "archive-options"] {
        assert!(grist::schema::schema_json(schema).is_some(), "{schema}");
    }

    let options = ArchiveOptions::default();
    assert_eq!(options.artifact_mode, ContainerArtifactMode::InventoryOnly);
}

#[test]
fn classic_tar_and_unsupported_zip_methods_remain_explicit() {
    let mut builder = Builder::new(Cursor::new(Vec::new()));
    let mut header = Header::new_old();
    header.set_entry_type(EntryType::Regular);
    header.set_mode(0o600);
    header.set_uid(7);
    header.set_gid(8);
    header.set_mtime(9);
    header.set_size(3);
    header.set_path("v7.txt").unwrap();
    header.set_cksum();
    builder.append(&header, b"old".as_slice()).unwrap();
    builder.finish().unwrap();
    let classic = builder.into_inner().unwrap().into_inner();
    assert_eq!(archive_format(&classic, None), Some(ArchiveFormat::Tar));
    let detection = detect_with_registry(
        Path::new("fixture.bin"),
        &classic,
        None,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Tar);
    let classic = traverse(
        classic,
        "tar",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(classic.status, OperationStatus::Complete);
    assert_eq!(
        classic.children[0].archive_metadata.as_ref().unwrap().uid,
        Some(7)
    );

    let mut malformed_tar = tar_bytes(&[("entry", EntryType::Regular, b"body", None)]);
    malformed_tar[0] ^= 1;
    let malformed_tar = traverse(
        malformed_tar,
        "tar",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(malformed_tar.status, OperationStatus::Failed);
    assert!(
        malformed_tar
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.class == grist::core::DiagnosticClass::MalformedInput })
    );

    let mut unsupported_zip = zip_bytes(
        &[
            ("before.txt", b"before", CompressionMethod::Stored),
            ("legacy.bin", b"bytes", CompressionMethod::Stored),
            ("after.txt", b"after", CompressionMethod::Stored),
        ],
        false,
    );
    patch_zip_method(&mut unsupported_zip, 1, 98);
    let unsupported_zip = traverse(
        unsupported_zip,
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(unsupported_zip.status, OperationStatus::Partial);
    assert_eq!(
        unsupported_zip.children[1].status,
        ContainerChildStatus::Unsupported
    );
    assert_eq!(
        unsupported_zip.children[0].status,
        ContainerChildStatus::InventoryOnly
    );
    assert_eq!(
        unsupported_zip.children[2].status,
        ContainerChildStatus::InventoryOnly
    );
    assert!(
        unsupported_zip.children[1]
            .diagnostics
            .iter()
            .any(|diagnostic| {
                diagnostic.code.as_str() == "grist.archive.unsupported_compression"
            })
    );
}

#[test]
fn declared_member_limits_win_before_inventory_allocation() {
    let mut forged_zip = zip_bytes(&[], false);
    for offset in 0..forged_zip.len().saturating_sub(12) {
        if &forged_zip[offset..offset + 4] == b"PK\x05\x06" {
            forged_zip[offset + 8..offset + 10].copy_from_slice(&500_u16.to_le_bytes());
            forged_zip[offset + 10..offset + 12].copy_from_slice(&500_u16.to_le_bytes());
            break;
        }
    }
    let mut member_budget = ResourceBudget::trusted_unbounded();
    member_budget.max_archive_members = Some(1);
    let limited = traverse(
        forged_zip,
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::custom(member_budget.clone()),
        None,
    );
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_members.exhausted"
    }));

    let mut malformed_second = tar_bytes(&[
        ("one", EntryType::Regular, b"1", None),
        ("two", EntryType::Regular, b"2", None),
    ]);
    malformed_second[1024] ^= 1;
    let limited = traverse(
        malformed_second,
        "tar",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::custom(member_budget),
        None,
    );
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_members.exhausted"
    }));
}

#[test]
fn zip_symlink_targets_and_read_failures_preserve_siblings() {
    let mut bytes = zip_bytes(
        &[
            ("link", b"../escape.txt", CompressionMethod::Stored),
            ("broken.txt", b"broken", CompressionMethod::Stored),
            ("after.txt", b"after", CompressionMethod::Stored),
        ],
        false,
    );
    mark_zip_symlink(&mut bytes, 0);
    corrupt_zip_payload(&mut bytes, 1);
    let result = traverse(
        bytes,
        "zip",
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(result.status, OperationStatus::Partial);
    assert_eq!(
        result.children[0]
            .archive_metadata
            .as_ref()
            .unwrap()
            .link_target
            .as_deref(),
        Some("../escape.txt")
    );
    assert!(result.children[0].diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.security.archive.unsafe_link_target"
    }));
    assert_eq!(result.children[1].status, ContainerChildStatus::Failed);
    assert!(
        result.children[1].diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "grist.archive.zip_member_read_failed"
        })
    );
    assert_eq!(
        result.children[2].status,
        ContainerChildStatus::InventoryOnly
    );
}

#[test]
fn registry_archive_reuses_caller_budget_control_and_security_policy() {
    let registry = builtin_parser_registry().unwrap();
    let bytes = zip_bytes(
        &[
            ("one.txt", b"one", CompressionMethod::Stored),
            ("two.txt", b"two", CompressionMethod::Stored),
        ],
        false,
    );
    let mut exact_input_budget = ResourceBudget::trusted_unbounded();
    exact_input_budget.max_input_bytes = Some(bytes.len() as u64);
    let request = ParseRequest::new(
        RequestId::new("archive-shared-control").unwrap(),
        Input::bytes(bytes),
        SourceInfo::new("fixture.zip"),
        BudgetSelection::custom(exact_input_budget),
        ProviderSet::none(),
    );
    let envelope = registry.dispatch("zip", request, None).unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);

    let bytes = zip_bytes(
        &[
            ("one.txt", b"one", CompressionMethod::Stored),
            ("two.txt", b"two", CompressionMethod::Stored),
        ],
        false,
    );
    let mut member_budget = ResourceBudget::trusted_unbounded();
    member_budget.max_archive_members = Some(1);
    let request = ParseRequest::new(
        RequestId::new("archive-caller-budget").unwrap(),
        Input::bytes(bytes),
        SourceInfo::new("fixture.zip"),
        BudgetSelection::custom(member_budget),
        ProviderSet::none(),
    );
    let envelope = registry.dispatch("zip", request, None).unwrap();
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert!(envelope.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.archive_members.exhausted"
    }));

    let bytes = zip_bytes(
        &[("long-name.txt", b"body", CompressionMethod::Stored)],
        false,
    );
    let mut request = ParseRequest::new(
        RequestId::new("archive-caller-security").unwrap(),
        Input::bytes(bytes),
        SourceInfo::new("fixture.zip"),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );
    request.options.security.archive.max_path_bytes = 1;
    let envelope = registry.dispatch("zip", request, None).unwrap();
    assert_eq!(envelope.status, OperationStatus::Partial);
    let document: grist::archive::ArchiveDocument =
        serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert!(
        document.traversal.children[0]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "grist.security.archive.path_limit")
    );
}
