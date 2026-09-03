use grist::core::{
    ArtifactKind, Envelope, OperationKind, OperationStatus, ParserInfo, ProvenanceStep,
    ProviderInvocation, SchemaVersion, SourceInfo, empty_options_digest, options_digest,
};
use serde_json::{Value, json};

fn terminal(status: OperationStatus) -> Envelope<Value> {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Unsupported,
        status,
        SourceInfo::stdin("input.bin"),
        ParserInfo::new("test-parser"),
        empty_options_digest(),
        "example/payload/v7",
    )
    .expect("terminal status must be valid without a payload")
}

#[test]
fn every_operation_and_status_has_stable_snake_case_json() {
    let operations = [
        (OperationKind::Parse, "parse"),
        (OperationKind::Ingest, "ingest"),
        (OperationKind::Transform, "transform"),
        (OperationKind::Render, "render"),
        (OperationKind::Segment, "segment"),
        (OperationKind::Validate, "validate"),
    ];
    for (operation, expected) in operations {
        assert_eq!(serde_json::to_value(operation).unwrap(), expected);
    }

    let terminal_statuses = [
        (OperationStatus::Failed, "failed"),
        (OperationStatus::Unsupported, "unsupported"),
        (OperationStatus::Encrypted, "encrypted"),
        (OperationStatus::Ambiguous, "ambiguous"),
        (OperationStatus::Cancelled, "cancelled"),
    ];
    for (status, expected) in terminal_statuses {
        let value = serde_json::to_value(terminal(status)).unwrap();
        assert_eq!(value["status"], expected);
        assert!(value["payload"].is_null());
    }

    let complete = Envelope::complete(
        OperationKind::Render,
        ArtifactKind::Text,
        SourceInfo::stdin("graph.json"),
        ParserInfo::new("test-renderer"),
        empty_options_digest(),
        "example/rendered-text/v9",
        "rendered".to_string(),
    );
    assert_eq!(
        serde_json::to_value(&complete).unwrap()["status"],
        "complete"
    );

    for payload in [Some(json!({"usable": true})), None] {
        let partial = Envelope::partial(
            OperationKind::Segment,
            ArtifactKind::Text,
            SourceInfo::stdin("document.json"),
            ParserInfo::new("test-segmenter"),
            empty_options_digest(),
            "example/segments/v3",
            payload,
        );
        let value = serde_json::to_value(partial).unwrap();
        assert_eq!(value["status"], "partial");
    }
}

#[test]
fn invalid_success_and_terminal_payload_combinations_never_serialize() {
    let mut complete = Envelope::complete(
        OperationKind::Parse,
        ArtifactKind::Text,
        SourceInfo::stdin("input.txt"),
        ParserInfo::new("test-parser"),
        empty_options_digest(),
        "example/text/v1",
        json!({"text": "ok"}),
    );
    complete.payload = None;
    assert!(serde_json::to_value(complete).is_err());

    let mut failed = terminal(OperationStatus::Failed);
    failed.payload = Some(json!({"fabricated": true}));
    assert!(serde_json::to_value(failed).is_err());

    let invalid_wire = json!({
        "schema_version": SchemaVersion::ENVELOPE_V2,
        "operation": "parse",
        "kind": "text",
        "status": "failed",
        "source": {"path": null, "display_name": "input.txt"},
        "hashes": null,
        "parser": {"name": "test-parser", "version": "0.1.0"},
        "options_digest": empty_options_digest(),
        "providers": [],
        "diagnostics": [],
        "provenance": [],
        "payload_schema_version": "example/text/v1",
        "payload": {"fabricated": true}
    });
    assert!(serde_json::from_value::<Envelope<Value>>(invalid_wire).is_err());
}

#[test]
fn envelope_and_payload_versions_are_independent() {
    let envelope = Envelope::complete(
        OperationKind::Transform,
        ArtifactKind::Text,
        SourceInfo::stdin("source.json"),
        ParserInfo::new("test-transform"),
        empty_options_digest(),
        "vendor/payload/v42",
        json!({"value": 42}),
    );
    assert_eq!(envelope.schema_version.0, SchemaVersion::ENVELOPE_V2);
    assert_eq!(envelope.payload_schema_version.0, "vendor/payload/v42");
}

#[test]
fn v1_envelopes_deserialize_with_compatible_defaults() {
    let legacy = json!({
        "schema_version": SchemaVersion::ENVELOPE_V1,
        "kind": "text",
        "source": {"path": null, "display_name": "legacy.txt"},
        "hashes": null,
        "parser": {"name": "legacy-parser", "version": "0.1.0"},
        "diagnostics": [],
        "payload_schema_version": "grist/text/v1",
        "payload": {"schema_version": "grist/text/v1", "blocks": []}
    });
    let envelope: Envelope<Value> = serde_json::from_value(legacy).unwrap();
    assert_eq!(envelope.schema_version.0, SchemaVersion::ENVELOPE_V1);
    assert_eq!(envelope.operation, OperationKind::Parse);
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert!(envelope.payload.is_some());
    assert_eq!(envelope.options_digest, empty_options_digest());
    assert!(envelope.providers.is_empty());
    assert!(envelope.provenance.is_empty());
}

#[test]
fn v2_envelopes_require_the_new_contract_fields() {
    let incomplete_v2 = json!({
        "schema_version": SchemaVersion::ENVELOPE_V2,
        "kind": "text",
        "source": {"path": null, "display_name": "input.txt"},
        "hashes": null,
        "parser": {"name": "parser", "version": "0.1.0"},
        "diagnostics": [],
        "payload_schema_version": "grist/text/v1",
        "payload": {"text": "content"}
    });
    let error = serde_json::from_value::<Envelope<Value>>(incomplete_v2).unwrap_err();
    assert!(error.to_string().contains("operation"));
}

#[test]
fn metadata_fields_and_option_digests_round_trip() {
    let digest = options_digest(&json!({"mode": "strict"})).unwrap();
    let provider = ProviderInvocation {
        provider: "recorded-ocr".into(),
        implementation: "fixture".into(),
        model_version: Some("v3".into()),
        configuration_digest: empty_options_digest(),
        deterministic: Some(true),
        ..ProviderInvocation::default()
    };
    let provenance = ProvenanceStep {
        operation: OperationKind::Render,
        implementation: "test-renderer".into(),
        input_identity: Some("sha256:input".into()),
        output_identity: Some("sha256:output".into()),
        options_digest: digest.clone(),
        timestamp: None,
        provider: Some("recorded-ocr".into()),
        warnings: vec![],
        loss_class: None,
    };
    let envelope = Envelope::complete(
        OperationKind::Render,
        ArtifactKind::Text,
        SourceInfo::stdin("source"),
        ParserInfo::new("renderer").with_feature("text-publishing"),
        digest,
        "example/text/v1",
        "output",
    )
    .with_providers(vec![provider])
    .with_provenance(vec![provenance]);

    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(value["parser"]["enabled_feature"], "text-publishing");
    assert_eq!(value["providers"][0]["provider"], "recorded-ocr");
    assert_eq!(value["provenance"][0]["operation"], "render");
}
