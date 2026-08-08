#![cfg(feature = "archives")]

use bzip2::write::BzEncoder;
use flate2::write::GzEncoder;
use grist::archive::{ArchiveFormat, archive_format, builtin_decoder_registry};
use grist::container::{
    ArtifactContent, ContainerArtifactMode, ContainerChildStatus, ContainerParseOptions,
    ContainerParseRequest, ContainerRecursor,
};
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
use xz2::write::XzEncoder;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = GzEncoder::new(Vec::new(), flate2::Compression::best());
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

fn seven_zip(bytes: &[u8], encrypted: bool) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = SevenZWriter::new(cursor).unwrap();
    if encrypted {
        writer.set_content_methods(vec![
            AesEncoderOptions::new(Password::from("secret")).into(),
            SevenZMethod::LZMA2.into(),
        ]);
    }
    let mut entry = SevenZArchiveEntry::new();
    entry.name = "payload.txt".into();
    writer
        .push_archive_entry(entry, Some(Cursor::new(bytes)))
        .unwrap();
    writer.finish().unwrap().into_inner()
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
