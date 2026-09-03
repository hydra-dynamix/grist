#![cfg(not(feature = "media"))]

use grist::registry::{Capability, ParserSelection, builtin_parser_registry};

#[test]
fn disabled_image_descriptors_retain_options_and_capability_metadata() {
    let registry = builtin_parser_registry().unwrap();
    let expected = serde_json::json!({
        "retain_metadata_bytes": true,
        "retain_unknown_chunk_bytes": true,
        "max_frames": 10_000,
        "max_dimension": 1_000_000,
        "max_metadata_bytes": 16 * 1024 * 1024,
        "max_unknown_chunk_bytes": 16 * 1024 * 1024,
        "max_chunks": 100_000,
        "max_svg_elements": 1_000_000,
        "max_svg_depth": 256,
        "max_svg_path_bytes": 64 * 1024,
        "ocr": {
            "mode": "all_frames",
            "language_hints": [],
            "recognize_tables": false,
            "max_scopes": 10_000,
            "reconcile": true,
        },
    });
    for format in ["png", "jpeg", "tiff", "webp", "gif", "bmp", "heif", "svg"] {
        let ParserSelection::Unsupported { unavailable, .. } = registry.select_format(format)
        else {
            panic!("{format} unexpectedly available without media");
        };
        let descriptor = &unavailable[0].descriptor;
        assert_eq!(descriptor.options.schema.name, "image-options");
        assert_eq!(descriptor.options.schema.version, "grist/image-options/v1");
        assert_eq!(descriptor.options.default, expected);
        assert!(descriptor.allowed_providers.is_empty());
        assert!(
            !descriptor
                .capabilities
                .contains(&Capability::ProviderDerivedContent)
        );
    }
}
