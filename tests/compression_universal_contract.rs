#![cfg(feature = "archives")]

use bzip2::write::BzEncoder;
use flate2::GzBuilder;
use flate2::write::GzEncoder;
use grist::archive::{ArchiveFormat, archive_format, builtin_decoder_registry};
use grist::container::{
    ArtifactContent, ContainerArtifactMode, ContainerChildStatus, ContainerParseOptions,
    ContainerParseRequest, ContainerRecursor,
};
#[cfg(feature = "manifests")]
use grist::core::ArtifactKind;
use grist::core::{
    BudgetProfile, BudgetSelection, CancellationToken, OperationStatus, RequestId, ResourceBudget,
    SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::ingest::Ingestor;
use grist::registry::builtin_parser_registry;
use sevenz_rust::{AesEncoderOptions, Password, SevenZArchiveEntry, SevenZMethod, SevenZWriter};
use std::io::{Cursor, Write};
use std::path::Path;
use tar::{Builder as TarBuilder, Header as TarHeader};
use xz2::write::XzEncoder;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = GzEncoder::new(Vec::new(), flate2::Compression::best());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn gzip_member(bytes: &[u8], filename: &str, comment: &str, mtime: u32) -> Vec<u8> {
    let mut writer = GzBuilder::new()
        .filename(filename)
        .comment(comment)
        .mtime(mtime)
        .write(Vec::new(), flate2::Compression::best());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn bzip2(bytes: &[u8]) -> Vec<u8> {
    let mut writer = BzEncoder::new(Vec::new(), bzip2::Compression::best());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut writer = XzEncoder::new(Vec::new(), 9);
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn zstandard(bytes: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(bytes, 3).unwrap()
}

struct SevenZipFixtureEntry<'a> {
    name: &'a str,
    bytes: &'a [u8],
    attributes: Option<u32>,
    modified_time: Option<u64>,
}

fn seven_zip_entries(entries: &[SevenZipFixtureEntry<'_>], encrypted: bool) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = SevenZWriter::new(cursor).unwrap();
    if encrypted {
        writer.set_content_methods(vec![
            AesEncoderOptions::new(Password::from("secret")).into(),
            SevenZMethod::LZMA2.into(),
        ]);
    }
    for fixture in entries {
        let mut entry = SevenZArchiveEntry::new();
        entry.name = fixture.name.into();
        if let Some(attributes) = fixture.attributes {
            entry.has_windows_attributes = true;
            entry.windows_attributes = attributes;
        }
        if let Some(seconds) = fixture.modified_time {
            entry.has_last_modified_date = true;
            entry.last_modified_date =
                sevenz_rust::nt_time::FileTime::new(116_444_736_000_000_000 + seconds * 10_000_000);
        }
        writer
            .push_archive_entry(entry, Some(Cursor::new(fixture.bytes)))
            .unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn seven_zip(bytes: &[u8], encrypted: bool) -> Vec<u8> {
    seven_zip_entries(
        &[SevenZipFixtureEntry {
            name: "payload.txt",
            bytes,
            attributes: None,
            modified_time: None,
        }],
        encrypted,
    )
}

fn tar_member(path: &str, bytes: &[u8]) -> Vec<u8> {
    let mut builder = TarBuilder::new(Cursor::new(Vec::new()));
    let mut header = TarHeader::new_gnu();
    header.set_path(path).unwrap();
    header.set_mode(0o644);
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    builder.append(&header, bytes).unwrap();
    builder.finish().unwrap();
    builder.into_inner().unwrap().into_inner()
}

fn traverse(
    bytes: Vec<u8>,
    format: &str,
    budget: BudgetSelection,
) -> grist::container::ContainerTraversal {
    let ingestor = Ingestor::builtin().unwrap();
    let decoders = builtin_decoder_registry().unwrap();
    ContainerRecursor::new(&ingestor, &decoders)
        .parse(
            ContainerParseRequest::new(
                RequestId::new("compression-contract").unwrap(),
                bytes,
                SourceInfo::new(format!("payload.{format}")),
                format,
                ContainerParseOptions::new(ContainerArtifactMode::InlinePayload)
                    .without_leaf_payloads(),
                budget,
            ),
            None,
        )
        .unwrap()
}

fn inline_bytes(child: &grist::container::ContainerChild) -> &[u8] {
    match child.artifact.content.as_ref().unwrap() {
        ArtifactContent::Inline(value) => &value.bytes,
        ArtifactContent::ContentAddressed { .. } => panic!("expected inline payload"),
    }
}

#[test]
fn every_compression_container_round_trips_with_metadata_and_detection() {
    let payload = b"bounded compression payload";
    let fixtures = [
        (
            "gzip",
            ArchiveFormat::Gzip,
            ContentKind::Gzip,
            gzip(payload),
        ),
        (
            "bzip2",
            ArchiveFormat::Bzip2,
            ContentKind::Bzip2,
            bzip2(payload),
        ),
        ("xz", ArchiveFormat::Xz, ContentKind::Xz, xz(payload)),
        (
            "zstandard",
            ArchiveFormat::Zstandard,
            ContentKind::Zstd,
            zstandard(payload),
        ),
        (
            "7z",
            ArchiveFormat::SevenZ,
            ContentKind::SevenZip,
            seven_zip(payload, false),
        ),
    ];
    let parser_registry = builtin_parser_registry().unwrap();
    for (format, expected_format, expected_kind, bytes) in fixtures {
        assert_eq!(archive_format(&bytes, None), Some(expected_format));
        let detected = detect_with_registry(
            Path::new(&format!("fixture.{format}")),
            &bytes,
            None,
            None,
            &grist::core::Limits::default(),
            &parser_registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detected.status, DetectionStatus::Selected, "{format}");
        assert_eq!(detected.content_kind, expected_kind, "{format}");

        let traversal = traverse(
            bytes,
            format,
            BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        );
        assert_eq!(traversal.status, OperationStatus::Partial, "{format}");
        assert_eq!(traversal.children.len(), 1, "{format}");
        assert_eq!(inline_bytes(&traversal.children[0]), payload, "{format}");
        let metadata = traversal.children[0].archive_metadata.as_ref().unwrap();
        assert_eq!(metadata.uncompressed_size, payload.len() as u64, "{format}");
        assert!(!metadata.encrypted, "{format}");
        assert_eq!(
            traversal.children[0]
                .artifact
                .identity
                .content
                .byte_length(),
            payload.len() as u64,
            "{format}"
        );
    }
}

#[test]
fn nested_stream_chain_is_preserved_and_outputs_are_deterministic() {
    let payload = b"nested payload";
    let bytes = gzip(&zstandard(payload));
    let first = traverse(
        bytes.clone(),
        "gzip",
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
    );
    let second = traverse(
        bytes,
        "gzip",
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
    );
    assert_eq!(first.status, OperationStatus::Partial);
    assert_eq!(first.children[0].status, ContainerChildStatus::Parsed);
    assert_eq!(first.children[0].children.len(), 1);
    assert_eq!(
        first.children[0].children[0]
            .artifact
            .identity
            .content
            .raw
            .as_ref()
            .unwrap()
            .sha256
            .as_str(),
        second.children[0].children[0]
            .artifact
            .identity
            .content
            .raw
            .as_ref()
            .unwrap()
            .sha256
            .as_str()
    );
    assert_eq!(inline_bytes(&first.children[0].children[0]), payload);
}

#[test]
fn gzip_members_preserve_header_provenance_and_nested_tar_chain() {
    let mut members = gzip_member(b"first", "first.txt", "first comment", 1_700_000_001);
    members.extend(gzip_member(
        b"second",
        "second.txt",
        "second comment",
        1_700_000_002,
    ));
    let traversal = traverse(
        members,
        "gzip",
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
    );
    assert_eq!(traversal.children.len(), 2);
    assert_eq!(inline_bytes(&traversal.children[0]), b"first");
    assert_eq!(inline_bytes(&traversal.children[1]), b"second");
    let first = traversal.children[0].archive_metadata.as_ref().unwrap();
    assert_eq!(first.original_filename.as_deref(), Some("first.txt"));
    assert_eq!(first.comment.as_deref(), Some("first comment"));
    assert_eq!(first.modified_time, Some(1_700_000_001));

    let nested = gzip_member(
        &tar_member("inside.txt", b"nested through tar"),
        "nested.tar",
        "tar payload",
        1_700_000_003,
    );
    let nested = traverse(
        nested,
        "gzip",
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
    );
    assert_eq!(nested.children[0].status, ContainerChildStatus::Parsed);
    assert_eq!(nested.children[0].children.len(), 1);
    assert_eq!(
        inline_bytes(&nested.children[0].children[0]),
        b"nested through tar"
    );
}

#[test]
fn seven_zip_rejects_unsafe_and_cross_platform_colliding_entries_before_materialization() {
    let bytes = seven_zip_entries(
        &[
            SevenZipFixtureEntry {
                name: "safe.txt",
                bytes: b"safe",
                attributes: Some(0x20),
                modified_time: Some(1_700_000_000),
            },
            SevenZipFixtureEntry {
                name: "Folder/Name.txt",
                bytes: b"collision-one",
                attributes: None,
                modified_time: None,
            },
            SevenZipFixtureEntry {
                name: "folder/name.TXT",
                bytes: b"collision-two",
                attributes: None,
                modified_time: None,
            },
            SevenZipFixtureEntry {
                name: "../escape.bin",
                bytes: b"escape",
                attributes: None,
                modified_time: None,
            },
            SevenZipFixtureEntry {
                name: "/absolute.bin",
                bytes: b"absolute",
                attributes: None,
                modified_time: None,
            },
            SevenZipFixtureEntry {
                name: "unsafe-link",
                bytes: b"target",
                attributes: Some(0o120777_u32 << 16),
                modified_time: None,
            },
        ],
        false,
    );
    let traversal = traverse(
        bytes,
        "7z",
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
    );
    assert_eq!(traversal.children.len(), 6);
    let safe = &traversal.children[0];
    assert_eq!(inline_bytes(safe), b"safe");
    let metadata = safe.archive_metadata.as_ref().unwrap();
    assert_eq!(metadata.external_attributes, Some(0x20));
    assert_eq!(metadata.modified_time, Some(1_700_000_000));
    assert_eq!(metadata.original_filename.as_deref(), Some("safe.txt"));
    for child in &traversal.children[1..] {
        assert_eq!(child.status, ContainerChildStatus::Rejected);
        assert!(child.artifact.content.is_none());
    }
    assert!(traversal.children[1..3].iter().all(|child| {
        child
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "grist.security.archive.duplicate_path")
    }));
    assert!(
        traversal.children[5]
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.security.archive.unsafe_link" })
    );
}

#[cfg(feature = "manifests")]
#[test]
fn compressed_manifest_leaf_retains_typed_source_parent_provenance() {
    let bytes = gzip_member(
        b"[package]\nname = \"compressed-demo\"\nversion = \"0.1.0\"\n",
        "Cargo.toml",
        "manifest",
        1_700_000_004,
    );
    let ingestor = Ingestor::builtin().unwrap();
    let decoders = builtin_decoder_registry().unwrap();
    let traversal = ContainerRecursor::new(&ingestor, &decoders)
        .parse(
            ContainerParseRequest::new(
                RequestId::new("compressed-manifest").unwrap(),
                bytes,
                SourceInfo::new("bundle.gz"),
                "gzip",
                ContainerParseOptions::new(ContainerArtifactMode::InlinePayload),
                BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
            ),
            None,
        )
        .unwrap();
    let parsed = traversal.children[0].parsed.as_ref().unwrap();
    assert_eq!(parsed.kind, ArtifactKind::Manifest);
    assert_eq!(parsed.source.display_name, "Cargo.toml");
    assert_eq!(
        parsed
            .source
            .parent
            .as_ref()
            .map(|value| value.display_name.as_str()),
        Some("bundle.gz")
    );
}

#[test]
fn corrupt_encrypted_and_oversized_inputs_have_explicit_terminal_status() {
    for (format, mut bytes) in [
        ("gzip", gzip(b"payload")),
        ("bzip2", bzip2(b"payload")),
        ("xz", xz(b"payload")),
        ("zstandard", zstandard(b"payload")),
        ("7z", seven_zip(b"payload", false)),
    ] {
        bytes.truncate(bytes.len().saturating_sub(3));
        let traversal = traverse(
            bytes,
            format,
            BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        );
        assert_eq!(traversal.status, OperationStatus::Failed, "{format}");
    }

    let encrypted = traverse(
        seven_zip(b"secret payload", true),
        "7z",
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
    );
    assert_eq!(encrypted.status, OperationStatus::Encrypted);
    assert!(
        encrypted
            .diagnostics
            .iter()
            .any(|item| item.code.as_str() == "grist.archive.encrypted")
    );

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_archive_expansion_ratio = Some(0.5);
    let oversized = traverse(
        gzip(&vec![0_u8; 128 * 1024]),
        "gzip",
        BudgetSelection::custom(budget),
    );
    assert_eq!(oversized.status, OperationStatus::Failed);
    assert!(
        oversized
            .diagnostics
            .iter()
            .any(|item| { item.code.as_str() == "grist.budget.archive_expansion_ratio.exhausted" })
    );
}

#[test]
fn compression_decode_honors_shared_cancellation_before_expansion() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let ingestor = Ingestor::builtin().unwrap();
    let decoders = builtin_decoder_registry().unwrap();
    let traversal = ContainerRecursor::new(&ingestor, &decoders)
        .parse(
            ContainerParseRequest::new(
                RequestId::new("compression-cancelled").unwrap(),
                gzip(&vec![0_u8; 128 * 1024]),
                SourceInfo::new("cancelled.gz"),
                "gzip",
                ContainerParseOptions::new(ContainerArtifactMode::InlinePayload)
                    .without_leaf_payloads(),
                BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
            )
            .with_cancellation(cancellation),
            None,
        )
        .unwrap();
    assert_eq!(traversal.status, OperationStatus::Cancelled);
    assert!(traversal.children.is_empty());
}
