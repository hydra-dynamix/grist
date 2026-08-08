#![cfg(not(feature = "media"))]

use grist::registry::{ParserSelection, builtin_parser_registry};

#[test]
fn disabled_subtitle_descriptors_retain_options_and_format_metadata() {
    let registry = builtin_parser_registry().unwrap();
    let expected = serde_json::json!({
        "encoding": null,
        "max_cues": 100_000,
        "max_styles": 10_000,
        "max_tracks": 1_000,
        "max_regions": 10_000,
        "max_nesting_depth": 256,
    });
    for format in ["srt", "webvtt", "ttml"] {
        let ParserSelection::Unsupported { unavailable, .. } = registry.select_format(format)
        else {
            panic!("{format} unexpectedly available without media");
        };
        let descriptor = &unavailable[0].descriptor;
        assert_eq!(
            descriptor.format.artifact_kind,
            grist::core::ArtifactKind::Subtitle
        );
        assert_eq!(descriptor.payload_schema.name, "subtitle");
        assert_eq!(descriptor.payload_schema.version, "grist/subtitle/v1");
        assert_eq!(descriptor.options.schema.name, "subtitle-options");
        assert_eq!(
            descriptor.options.schema.version,
            "grist/subtitle-options/v1"
        );
        assert_eq!(descriptor.options.default, expected);
        assert!(descriptor.allowed_providers.is_empty());
    }
}
