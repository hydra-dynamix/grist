use grist::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Input, NetworkAccess, OperationStatus,
    ParseRequest, ParserInfo, Provider, ProviderInvocation, ProviderKind, ProviderSet, RequestId,
    SourceInfo, empty_options_digest,
};
use grist::registry::{
    FormatMetadata, IsolationMetadata, NetworkPolicy, OptionsMetadata, Parser, ParserContext,
    ParserDescriptor, ParserError, ParserOutput, ParserRegistry, ParserRegistryError,
    ParserSelection, ProviderDescriptor, ProviderRegistry, ProviderRegistryError, SchemaMetadata,
    builtin_parser_registry,
};
use serde_json::json;
use std::sync::Arc;

fn request(text: &str, providers: ProviderSet) -> ParseRequest {
    ParseRequest::new(
        RequestId::new("registry-test").unwrap(),
        Input::utf8(text),
        SourceInfo::stdin("fixture.demo"),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        providers,
    )
}

fn descriptor(id: &str, format: &str, priority: i32) -> ParserDescriptor {
    let mut descriptor = ParserDescriptor::caller(
        id,
        FormatMetadata::new(format, ArtifactKind::Text)
            .with_aliases([format.to_string() + "_alias"])
            .with_media_types(["application/x-".to_string() + format])
            .with_extensions([format]),
        ParserInfo::new(id),
        SchemaMetadata::new(format, "example/payload/v1"),
        OptionsMetadata::new(
            SchemaMetadata::new("demo-options", "example/options/v1"),
            json!({"mode": "safe"}),
        ),
    );
    descriptor.priority = priority;
    descriptor
}

struct EchoParser;

impl Parser for EchoParser {
    fn parse(&self, context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
        context.checkpoint()?;
        context.consume_nodes(1)?;
        Ok(ParserOutput::complete(json!({
            "text": context.utf8_text()?,
            "options": context.options(),
        })))
    }
}

#[cfg(feature = "pdf")]
#[test]
fn builtins_are_sorted_inspectable_and_pdf_is_available() {
    let registry = builtin_parser_registry().unwrap();
    let ids: Vec<_> = registry.parsers().into_iter().map(|item| item.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    assert!(matches!(
        registry.select_extension(".txt"),
        ParserSelection::Available(descriptor) if descriptor.id == "grist.text"
    ));
    assert!(matches!(
        registry.select_format("pdf"),
        ParserSelection::Available(descriptor) if descriptor.id == "grist.pdf"
    ));
    let envelope = registry
        .dispatch("pdf", request("%PDF", ProviderSet::none()), None)
        .unwrap();
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert!(envelope.payload.is_none());
    assert_eq!(
        envelope.diagnostics[0].code.as_str(),
        "grist.input.malformed"
    );
}

#[cfg(not(feature = "markdown"))]
#[test]
fn disabled_builtin_is_recognized_as_unsupported() {
    let registry = builtin_parser_registry().unwrap();
    assert!(matches!(
        registry.select_extension("md"),
        ParserSelection::Unsupported { unavailable, .. }
            if unavailable.iter().any(|item| matches!(
                item.reason,
                grist::registry::UnavailableReason::FeatureDisabled { .. }
            ))
    ));
}

#[test]
fn caller_priority_and_conflicts_are_explicit() {
    let mut registry = ParserRegistry::empty();
    registry
        .register_caller(descriptor("low", "demo", 10), Arc::new(EchoParser))
        .unwrap();
    registry
        .register_caller(descriptor("high", "demo", 20), Arc::new(EchoParser))
        .unwrap();
    assert!(matches!(
        registry.select_format("demo_alias"),
        ParserSelection::Available(selected) if selected.id == "high"
    ));
    assert_eq!(
        registry.register_caller(descriptor("tie", "demo", 20), Arc::new(EchoParser)),
        Err(ParserRegistryError::PriorityConflict("demo".to_string()))
    );
    assert_eq!(
        registry.register_caller(descriptor("high", "other", 30), Arc::new(EchoParser)),
        Err(ParserRegistryError::DuplicateId("high".to_string()))
    );
}

#[test]
fn selector_aliases_cannot_point_to_different_formats() {
    let mut registry = ParserRegistry::empty();
    let first = descriptor("first", "alpha", 1);
    let mut second = descriptor("second", "beta", 2);
    second.format.aliases.insert("alpha_alias".to_string());
    registry
        .register_caller(first, Arc::new(EchoParser))
        .unwrap();
    assert_eq!(
        registry.register_caller(second, Arc::new(EchoParser)),
        Err(ParserRegistryError::SelectorConflict(
            "alpha_alias".to_string()
        ))
    );
    let mut replacement = descriptor("replacement", "gamma", 3);
    replacement.format.aliases.insert("beta".to_string());
    registry
        .register_caller(replacement, Arc::new(EchoParser))
        .expect("a rejected registration must not retain partial selectors");
}

#[test]
fn registry_owns_envelope_identity_options_and_payload_semantics() {
    let mut registry = ParserRegistry::empty();
    registry
        .register_caller(descriptor("echo", "demo", 1), Arc::new(EchoParser))
        .unwrap();
    let first = registry
        .dispatch("demo", request("hello", ProviderSet::none()), None)
        .unwrap();
    let second = registry
        .dispatch("DEMO-ALIAS", request("hello", ProviderSet::none()), None)
        .unwrap();
    assert_eq!(first.status, OperationStatus::Complete);
    assert_eq!(first.parser.name, "echo");
    assert_eq!(first.options_digest, second.options_digest);
    assert_eq!(
        first
            .identity
            .as_ref()
            .unwrap()
            .raw
            .as_ref()
            .unwrap()
            .byte_length,
        5
    );
    assert_eq!(first.payload.as_ref().unwrap()["text"], "hello");
    assert_eq!(first.payload.as_ref().unwrap()["options"]["mode"], "safe");
    first.validate().unwrap();
}

#[test]
fn untyped_format_options_fail_before_parser_execution() {
    let mut registry = ParserRegistry::empty();
    registry
        .register_caller(descriptor("echo", "demo", 1), Arc::new(EchoParser))
        .unwrap();
    let envelope = registry
        .dispatch(
            "demo",
            request("hello", ProviderSet::none()),
            Some(json!(["not", "an", "object"])),
        )
        .unwrap();
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert_eq!(
        envelope.diagnostics[0].code.as_str(),
        "grist.input.malformed"
    );
}

#[cfg(feature = "csv")]
#[test]
fn builtin_option_schemas_reject_unknown_fields() {
    let registry = builtin_parser_registry().unwrap();
    let envelope = registry
        .dispatch(
            "csv",
            request("a,b\n1,2\n", ProviderSet::none()),
            Some(json!({"unknown": true})),
        )
        .unwrap();
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert_eq!(
        envelope.diagnostics[0].code.as_str(),
        "grist.input.malformed"
    );
}

struct InvalidOutputParser;

impl Parser for InvalidOutputParser {
    fn parse(&self, _context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
        Ok(ParserOutput {
            status: OperationStatus::Complete,
            payload: None,
            diagnostics: Vec::new(),
            providers: Vec::new(),
            provenance: Vec::new(),
        })
    }
}

#[test]
fn invalid_extension_output_becomes_a_parser_defect() {
    let mut registry = ParserRegistry::empty();
    registry
        .register_caller(
            descriptor("invalid", "invalid", 1),
            Arc::new(InvalidOutputParser),
        )
        .unwrap();
    let envelope = registry
        .dispatch("invalid", request("x", ProviderSet::none()), None)
        .unwrap();
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert_eq!(envelope.diagnostics[0].code.as_str(), "grist.parser.defect");
}

struct PanicParser;

impl Parser for PanicParser {
    fn parse(&self, _context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
        panic!("fixture panic")
    }
}

#[test]
fn panic_and_output_budget_cannot_escape_the_registry_boundary() {
    let mut registry = ParserRegistry::empty();
    registry
        .register_caller(descriptor("panic", "panic", 1), Arc::new(PanicParser))
        .unwrap();
    let panic_envelope = registry
        .dispatch("panic", request("x", ProviderSet::none()), None)
        .unwrap();
    assert_eq!(panic_envelope.status, OperationStatus::Failed);
    assert_eq!(
        panic_envelope.diagnostics[0].code.as_str(),
        "grist.parser.defect"
    );

    let mut budget = grist::core::ResourceBudget::trusted_unbounded();
    budget.max_output_bytes = Some(1);
    let limited = ParseRequest::new(
        RequestId::new("output-budget").unwrap(),
        Input::utf8("large output"),
        SourceInfo::stdin("fixture.demo"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let mut registry = ParserRegistry::empty();
    registry
        .register_caller(descriptor("echo", "demo", 1), Arc::new(EchoParser))
        .unwrap();
    let budget_envelope = registry.dispatch("demo", limited, None).unwrap();
    assert_eq!(budget_envelope.status, OperationStatus::Failed);
    assert_eq!(
        budget_envelope.diagnostics[0].code.as_str(),
        "grist.budget.output_bytes.exhausted"
    );
}

struct DemoProvider(&'static str);

impl Provider for DemoProvider {
    fn name(&self) -> &str {
        self.0
    }
}

fn provider_descriptor(id: &str, kind: ProviderKind) -> ProviderDescriptor {
    ProviderDescriptor::caller(id, kind, id, "fixture", "1")
}

#[test]
fn provider_registry_requires_explicit_network_and_rejects_ties() {
    let mut registry = ProviderRegistry::empty();
    let mut local = provider_descriptor("local-ocr", ProviderKind::Ocr);
    local.network = NetworkPolicy::Forbidden;
    registry
        .register_caller(local.clone(), Arc::new(DemoProvider("local-ocr")))
        .unwrap();
    assert!(matches!(
        registry.bind(ProviderKind::Ocr, "local-ocr", NetworkAccess::Allowed),
        Err(ProviderRegistryError::NetworkDenied)
    ));
    assert!(
        registry
            .bind(ProviderKind::Ocr, "local-ocr", NetworkAccess::Denied)
            .is_ok()
    );
    let duplicate_priority = provider_descriptor("other-ocr", ProviderKind::Ocr);
    assert_eq!(
        registry.register_caller(duplicate_priority, Arc::new(DemoProvider("other-ocr"))),
        Err(ProviderRegistryError::PriorityConflict(ProviderKind::Ocr))
    );
}

#[test]
fn isolated_backends_require_a_nonexecuting_isolation_contract() {
    let mut registry = grist::registry::IsolatedBackendRegistry::empty();
    let unsafe_descriptor =
        provider_descriptor("unsafe-backend", ProviderKind::IsolatedParserBackend);
    assert!(matches!(
        registry.register_caller(unsafe_descriptor, Arc::new(DemoProvider("unsafe-backend"))),
        Err(ProviderRegistryError::Invalid(_))
    ));
    let mut safe = provider_descriptor("safe-backend", ProviderKind::IsolatedParserBackend);
    safe.isolation = Some(IsolationMetadata::safe_default());
    registry
        .register_caller(safe, Arc::new(DemoProvider("safe-backend")))
        .unwrap();
    assert!(registry.bind("safe-backend", NetworkAccess::Denied).is_ok());
}

struct ProviderParser {
    record_invocation: bool,
}

impl Parser for ProviderParser {
    fn parse(&self, context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
        let payload = context.run_provider_json(ProviderKind::Ocr, |provider, network| {
            Ok(json!({
                "provider": provider.name(),
                "network": format!("{network:?}"),
            }))
        })?;
        let mut output = ParserOutput::complete(payload);
        if self.record_invocation {
            output
                .providers
                .push(ProviderInvocation::new("ocr", "fixture", empty_options_digest()).unwrap());
        }
        Ok(output)
    }
}

fn provider_parse_setup(record_invocation: bool) -> (ParserRegistry, ProviderSet) {
    let mut providers = ProviderRegistry::empty();
    let provider = provider_descriptor("ocr", ProviderKind::Ocr);
    providers
        .register_caller(provider, Arc::new(DemoProvider("ocr")))
        .unwrap();
    let mut selected = ProviderSet::none();
    providers
        .select_into(
            &mut selected,
            ProviderKind::Ocr,
            "ocr",
            NetworkAccess::Denied,
        )
        .unwrap();
    let mut parsers = ParserRegistry::empty();
    let mut parser = descriptor("provider-parser", "provider_demo", 1);
    parser.allowed_providers.insert(ProviderKind::Ocr);
    parser.required_providers.insert(ProviderKind::Ocr);
    parsers
        .register_caller(parser, Arc::new(ProviderParser { record_invocation }))
        .unwrap();
    (parsers, selected)
}

#[test]
fn provider_use_must_be_allowed_selected_budgeted_and_attributed() {
    let (registry, providers) = provider_parse_setup(false);
    let failed = registry
        .dispatch("provider_demo", request("image", providers), None)
        .unwrap();
    assert_eq!(failed.status, OperationStatus::Failed);
    assert_eq!(failed.diagnostics[0].code.as_str(), "grist.parser.defect");

    let (registry, providers) = provider_parse_setup(true);
    let complete = registry
        .dispatch("provider_demo", request("image", providers), None)
        .unwrap();
    assert_eq!(complete.status, OperationStatus::Complete);
    assert_eq!(complete.providers[0].provider, "ocr");
}
