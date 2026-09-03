#![cfg(not(feature = "media"))]

use grist::registry::{Capability, ParserSelection, builtin_parser_registry};
use grist::{
    core::Limits,
    detect::{ContentKind, DetectionOptions, detect_with_registry},
};
use std::path::Path;

#[test]
fn disabled_media_descriptors_retain_contract_metadata() {
    let registry = builtin_parser_registry().unwrap();
    let expected = serde_json::json!({
        "retain_embedded_bytes": true,
        "parse_embedded": true,
        "max_boxes": 100_000,
        "max_nesting_depth": 64,
        "max_metadata_bytes": 16 * 1024 * 1024,
        "max_attachment_bytes": 32 * 1024 * 1024,
        "max_subtitle_bytes": 16 * 1024 * 1024,
        "transcription": {
            "selection": {"mode": "disabled"},
            "provider_options": {
                "language_hints": [],
                "speaker_diarization": false,
                "word_timestamps": false,
            },
            "max_streams": 1_000,
            "reconcile": false,
        },
    });

    for format in ["mp3", "mp4", "quicktime", "wav", "flac", "matroska"] {
        let ParserSelection::Unsupported { unavailable, .. } = registry.select_format(format)
        else {
            panic!("{format} unexpectedly available without media");
        };
        let descriptor = &unavailable[0].descriptor;
        assert_eq!(
            descriptor.format.artifact_kind,
            grist::core::ArtifactKind::Media
        );
        assert_eq!(descriptor.payload_schema.name, "media");
        assert_eq!(descriptor.payload_schema.version, "grist/media/v1");
        assert_eq!(descriptor.options.schema.name, "media-options");
        assert_eq!(descriptor.options.schema.version, "grist/media-options/v1");
        assert_eq!(descriptor.options.default, expected);
        assert!(
            descriptor
                .allowed_providers
                .contains(&grist::core::ProviderKind::Transcription)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&Capability::EmbeddedArtifacts)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&Capability::ProviderDerivedContent)
        );
    }
}

#[test]
fn disabled_media_does_not_claim_heif_as_mp4() {
    let mut payload = b"heic".to_vec();
    payload.extend_from_slice(&0u32.to_be_bytes());
    payload.extend_from_slice(b"mif1");
    let mut bytes = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(b"ftyp");
    bytes.extend(payload);
    let registry = builtin_parser_registry().unwrap();
    let detection = detect_with_registry(
        Path::new("sample.heic"),
        &bytes,
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.content_kind, ContentKind::Heif);
    assert_ne!(detection.candidates[0].identity.format, "mp4");
}
