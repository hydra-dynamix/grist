#![cfg(not(feature = "graph"))]

use grist::core::{Limits, SourceInfo};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_source};
use grist::registry::{ParserSelection, UnavailableReason, builtin_parser_registry};

#[test]
fn graph_registration_is_explicitly_unavailable_when_feature_is_off() {
    let registry = builtin_parser_registry().unwrap();
    match registry.select_format("graph") {
        ParserSelection::Unsupported { unavailable, .. } => {
            assert_eq!(unavailable.len(), 1);
            assert_eq!(unavailable[0].descriptor.id, "grist.graph");
            assert!(matches!(
                unavailable[0].reason,
                UnavailableReason::FeatureDisabled { ref feature } if feature == "graph"
            ));
        }
        other => panic!("expected disabled graph parser, got {other:?}"),
    }
}

#[test]
fn canonical_graph_input_is_detected_as_unsupported_not_generic_json() {
    let registry = builtin_parser_registry().unwrap();
    let detection = detect_source(
        &SourceInfo::stdin("sample.graph.json"),
        br#"{"schema_version":"grist/graph-document/v1","directed":true,"nodes":[],"edges":[],"attrs":{}}"#,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.content_kind, ContentKind::Graph);
    assert_eq!(detection.status, DetectionStatus::Unsupported);
    assert_eq!(detection.candidates[0].identity.format, "graph");
}
