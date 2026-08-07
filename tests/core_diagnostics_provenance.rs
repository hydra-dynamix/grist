use grist::core::{
    ArtifactKind, ContentIdentity, DeclaredLoss, Diagnostic, DiagnosticCause, DiagnosticClass,
    DiagnosticDetails, Envelope, LossClass, OperationKind, ParserInfo, ProvenanceStep,
    ProviderInvocation, RecoveryAction, RecoveryKind, SourceInfo, SourceLocator, SourceRange,
    empty_options_digest, options_digest,
};
use serde_json::{Value, json};

#[test]
fn condition_families_have_stable_codes_and_severity() {
    let diagnostics = [
        Diagnostic::malformed("parser", "bad bytes"),
        Diagnostic::unsupported("parser", "disabled format"),
        Diagnostic::lossy("parser", "formatting flattened"),
        Diagnostic::provider_failure("parser", "OCR unavailable"),
        Diagnostic::parser_defect("parser", "backend invariant failed"),
        Diagnostic::security_rejection("parser", "unsafe path"),
        Diagnostic::budget_exhausted("parser", "page limit reached"),
    ];
    let expected = [
        (DiagnosticClass::MalformedInput, "grist.input.malformed"),
        (
            DiagnosticClass::UnsupportedContent,
            "grist.content.unsupported",
        ),
        (
            DiagnosticClass::LossyNormalization,
            "grist.normalization.lossy",
        ),
        (DiagnosticClass::ProviderFailure, "grist.provider.failed"),
        (DiagnosticClass::ParserDefect, "grist.parser.defect"),
        (
            DiagnosticClass::SecurityRejection,
            "grist.security.rejected",
        ),
        (
            DiagnosticClass::ResourceBudgetExhaustion,
            "grist.budget.exhausted",
        ),
    ];

    for (diagnostic, (class, code)) in diagnostics.iter().zip(expected) {
        assert_eq!(diagnostic.class, class);
        assert_eq!(diagnostic.code, code);
        assert_eq!(diagnostic.module, "parser");
        assert_eq!(diagnostic.parser, "parser");
        assert!(diagnostic.explanation_key.is_some());
    }
    assert!(diagnostics[2].partial);
    assert!(diagnostics[6].partial);
}

#[test]
fn diagnostic_retains_source_locator_causes_recovery_and_affected_ids() {
    let range = SourceRange {
        byte_start: 4,
        byte_end: 9,
        start_line: 1,
        start_column: 5,
        end_line: 1,
        end_column: 10,
    };
    let locator = SourceLocator::try_from(range.clone()).unwrap();
    let details = DiagnosticDetails::from_value(json!({
        "object_number": 17,
        "backend_error": "xref entry is truncated"
    }))
    .unwrap();
    let cause = DiagnosticCause::new("pdf.xref.truncated", "xref object ended early")
        .with_emitter("grist.formats.pdf", "pdf-backend")
        .with_details(details)
        .caused_by(DiagnosticCause::new(
            "io.unexpected_eof",
            "input ended at byte 9",
        ));
    let diagnostic = Diagnostic::malformed("pdf-backend", "PDF xref table is malformed")
        .with_module("grist.formats.pdf")
        .with_source("sample.pdf")
        .with_source_identity(ContentIdentity::for_raw_bytes(b"sample"))
        .with_range(range)
        .with_locator(locator)
        .with_cause(cause)
        .with_recovery(RecoveryAction::new(
            RecoveryKind::InspectInput,
            "supply an unrepaired original PDF",
            true,
        ))
        .with_affected_ids(vec!["page-1".into(), "xref-17".into()])
        .with_documentation_uri("https://example.invalid/grist/diagnostics/pdf-xref")
        .with_explanation_key("diagnostic.pdf.xref.truncated")
        .partial();

    let value = serde_json::to_value(&diagnostic).unwrap();
    assert_eq!(value["source_identity"]["raw"]["byte_length"], 6);
    assert_eq!(value["locator"]["components"][0]["type"], "text_range");
    assert_eq!(value["causes"][0]["cause"]["code"], "io.unexpected_eof");
    assert_eq!(value["recovery"]["kind"], "inspect_input");
    assert_eq!(value["affected_ids"], json!(["page-1", "xref-17"]));
    assert!(value["partial"].as_bool().unwrap());
}

#[test]
fn diagnostic_details_reject_secrets_at_every_boundary() {
    let direct = DiagnosticDetails::from_value(json!({"password": "do-not-log"}));
    assert!(direct.unwrap_err().to_string().contains("details.password"));

    let nested = serde_json::from_value::<DiagnosticDetails>(json!({
        "provider": {"api_key": "do-not-log"}
    }));
    assert!(
        nested
            .unwrap_err()
            .to_string()
            .contains("details.provider.api_key")
    );

    let authorization = DiagnosticDetails::from_value(json!({
        "response": "Bearer do-not-log"
    }));
    assert!(
        authorization
            .unwrap_err()
            .to_string()
            .contains("details.response")
    );
}

#[test]
fn legacy_flat_diagnostic_remains_readable() {
    let legacy = json!({
        "severity": "warning",
        "code": "legacy.parse.recovered",
        "message": "legacy parser recovered",
        "parser": "legacy-parser",
        "source": "legacy.txt",
        "range": null,
        "partial": true,
        "cause": ["unexpected delimiter"],
        "details": {"offset": 12}
    });
    let diagnostic: Diagnostic = serde_json::from_value(legacy).unwrap();
    assert_eq!(diagnostic.code, "legacy.parse.recovered");
    assert_eq!(diagnostic.class, DiagnosticClass::Unclassified);
    assert_eq!(diagnostic.cause, ["unexpected delimiter"]);
    assert!(diagnostic.causes.is_empty());

    let mut scalar_details = serde_json::to_value(diagnostic).unwrap();
    scalar_details["details"] = json!(12);
    let diagnostic: Diagnostic = serde_json::from_value(scalar_details).unwrap();
    assert_eq!(diagnostic.details.unwrap().as_value(), &json!(12));
}

#[test]
fn parser_provider_and_loss_metadata_are_complete_and_checked() {
    let parser = ParserInfo::new("grist.pdf")
        .with_implementation("pdfium-isolated", "128.0")
        .with_feature("pdf")
        .with_specification_version("PDF 2.0")
        .with_build_identity("build:fixture");
    parser.validate().unwrap();
    assert_eq!(parser.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(parser.enabled_feature.as_deref(), Some("pdf"));
    assert_eq!(parser.grammar_version.as_deref(), Some("PDF 2.0"));

    let provider =
        ProviderInvocation::new("recorded-ocr", "fixture-provider", empty_options_digest())
            .unwrap()
            .with_model_version("ocr-v3")
            .with_identities("sha256:input", "sha256:provider-output")
            .with_timing("2026-08-06T00:00:00Z/PT0.125S");
    provider.validate().unwrap();

    let digest = options_digest(&json!({"mode": "normalized"})).unwrap();
    let step = ProvenanceStep::new(
        OperationKind::Transform,
        "pdf-native-ocr-reconcile@1",
        "sha256:input",
        "sha256:output",
        digest,
        DeclaredLoss::Lossy(LossClass::new(LossClass::ORDER_INFERRED).unwrap()),
    )
    .unwrap()
    .with_provider("recorded-ocr")
    .with_warning("pdf.reading_order.inferred");
    step.validate().unwrap();
    assert_eq!(
        step.loss_class.as_ref().unwrap().as_str(),
        LossClass::ORDER_INFERRED
    );
}

#[test]
fn operation_envelopes_emit_an_identity_linked_lossless_provenance_step() {
    let payload = json!({"normalized": "hello"});
    let options = options_digest(&json!({"newline": "lf"})).unwrap();
    let envelope = Envelope::complete(
        OperationKind::Parse,
        ArtifactKind::Text,
        SourceInfo::stdin("note.txt"),
        ParserInfo::new("grist.text").with_feature("text-publishing"),
        options.clone(),
        "example/text/v1",
        payload,
    )
    .with_identity(ContentIdentity::for_raw_bytes(b"hello\r\n"));

    let value = serde_json::to_value(envelope).unwrap();
    let provenance = value["provenance"].as_array().unwrap();
    assert_eq!(provenance.len(), 1);
    assert_eq!(provenance[0]["operation"], "parse");
    assert_eq!(provenance[0]["options_digest"], options);
    assert_eq!(
        provenance[0]["input_identity"],
        "sha256:cd2eca3535741f27a8ae40c31b0c41d4057a7a7b912b33b9aed86485d1c84676"
    );
    assert_eq!(
        provenance[0]["output_identity"],
        value["identity"]["canonical_payload"]["sha256"]
    );
    assert!(provenance[0].get("loss_class").is_none());
    assert_eq!(
        value["parser"]["implementation_version"],
        env!("CARGO_PKG_VERSION")
    );

    let round_trip: Envelope<Value> = serde_json::from_value(value).unwrap();
    assert_eq!(round_trip.provenance.len(), 1);
}
