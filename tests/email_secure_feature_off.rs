#![cfg(not(feature = "email-message"))]

use grist::registry::{ParserSelection, UnavailableReason, builtin_parser_registry};

#[test]
fn email_secure_content_is_explicitly_feature_disabled() {
    let registry = builtin_parser_registry().unwrap();
    match registry.select_format("eml") {
        ParserSelection::Unsupported { unavailable, .. } => {
            assert!(unavailable.iter().any(|entry| {
                matches!(
                    &entry.reason,
                    UnavailableReason::FeatureDisabled { feature }
                        if feature == "email-message"
                )
            }))
        }
        ParserSelection::Available(_) => {
            panic!("email parser must be disabled without the feature")
        }
    }
}
