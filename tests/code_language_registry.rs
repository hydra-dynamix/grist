use grist::core::{
    BudgetProfile, BudgetSelection, DiagnosticClass, Input, Limits, OperationStatus, ParseRequest,
    ProviderSet, RequestId, ResourceBudget, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentKind, DocumentNodeKind, ToDocumentGraph,
};
use grist::registry::{
    ParserRegistry, ParserRegistryError, ParserSelection, builtin_parser_registry,
};
use std::path::Path;

fn request(text: &str, name: &str) -> ParseRequest {
    ParseRequest::new(
        RequestId::new(format!("code-language-{name}")).unwrap(),
        Input::utf8(text),
        SourceInfo::stdin(name),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    )
}

#[cfg(not(feature = "secondary-code"))]
#[test]
fn missing_secondary_grammars_are_explicitly_unsupported() {
    let registry = builtin_parser_registry().unwrap();
    for config in grist::code::builtin_language_configs() {
        assert!(
            matches!(
                registry.select_format(&config.language),
                ParserSelection::Unsupported { unavailable, .. }
                    if unavailable.iter().any(|item| matches!(
                        &item.reason,
                        grist::registry::UnavailableReason::FeatureDisabled { feature }
                            if feature == &config.enabled_feature
                    ))
            ),
            "{}",
            config.language
        );
    }
}

#[cfg(feature = "schemas")]
#[test]
fn code_adapter_contract_has_standalone_schema_registration() {
    let descriptor = grist::schema::schema_descriptor("tree-sitter-language-adapter")
        .expect("tree-sitter adapter metadata schema is registered");
    assert_eq!(
        descriptor.schema_version,
        grist::code::TREE_SITTER_ADAPTER_SCHEMA_V1
    );
    assert_eq!(
        descriptor.file_name,
        "grist.tree-sitter-language-adapter.v1.schema.json"
    );
    assert!(
        grist::schema::schema_json("tree-sitter-language-adapter").is_some(),
        "tree-sitter adapter metadata schema is generated"
    );
}

#[cfg(all(feature = "c", feature = "cpp"))]
#[test]
fn shared_header_extension_is_ambiguous_until_content_disambiguates_it() {
    let registry = builtin_parser_registry().unwrap();
    let neutral = detect_with_registry(
        Path::new("value.h"),
        b"int value;",
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(neutral.status, DetectionStatus::Ambiguous);
    let formats = neutral
        .candidates
        .iter()
        .take(2)
        .map(|candidate| candidate.identity.format.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(formats, std::collections::BTreeSet::from(["c", "cpp"]));

    let cpp = detect_with_registry(
        Path::new("value.h"),
        b"namespace demo { int value; }",
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(cpp.status, DetectionStatus::Selected);
    assert_eq!(cpp.candidates[0].identity.format, "cpp");
}

#[cfg(feature = "kotlin")]
#[test]
fn kotlin_feature_uses_the_pinned_compatible_grammar() {
    use grist::code::CodeFile;

    let registry = builtin_parser_registry().unwrap();
    let descriptor = match registry.select_format("kotlin") {
        ParserSelection::Available(descriptor) => descriptor,
        other => panic!("kotlin was not available: {other:?}"),
    };
    assert_eq!(
        descriptor.parser.implementation.as_deref(),
        Some("tree-sitter-kotlin-sg")
    );
    assert_eq!(
        descriptor.parser.implementation_version.as_deref(),
        Some("0.4.1")
    );
    assert_eq!(descriptor.parser.grammar_version.as_deref(), Some("0.4.1"));

    let envelope = registry
        .dispatch(
            "kotlin",
            request("data class User(val name: String)", "User.kt"),
            None,
        )
        .unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    let payload: CodeFile = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert_eq!(payload.adapter.grammar, "tree-sitter-kotlin-sg");
    assert_eq!(payload.adapter.grammar_version, "0.4.1");
}

#[cfg(feature = "cli")]
#[test]
fn cli_code_parse_round_trips_through_document_graph_projection() {
    let envelope = grist::cli::parse_bytes(
        "kotlin",
        b"data class User(val name: String)".to_vec(),
        SourceInfo::stdin("User.kt"),
        RequestId::new("cli-code-round-trip").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    let graph = grist::cli::project_envelope_to_graph(&envelope, "code:cli").unwrap();
    assert_eq!(graph.kind, DocumentKind::Code);
    assert_eq!(graph.language.as_deref(), Some("kotlin"));
    assert!(!graph.nodes.is_empty());
}
#[cfg(all(
    feature = "secondary-code",
    feature = "rust",
    feature = "document-graph",
    feature = "schemas"
))]
#[test]
fn caller_adapter_needs_no_registry_or_detection_policy_fork() {
    use grist::code::{
        CodeFile, CodeIngestOptions, TreeSitterLanguageAdapter, TreeSitterLanguageConfig,
    };
    use std::sync::Arc;

    let config = TreeSitterLanguageConfig::new(
        "future-language",
        "tree-sitter-future-language",
        "tree-sitter-rust",
        "0.23.3",
        "rust",
    )
    .with_extensions(["future"])
    .with_media_types(["text/x-future"])
    .with_probe_markers(["future_marker"], 1, true);
    let adapter =
        TreeSitterLanguageAdapter::new(config, tree_sitter_rust::LANGUAGE.into()).unwrap();
    let descriptor = adapter.caller_descriptor();
    let mut registry = ParserRegistry::empty();
    registry
        .register_caller(descriptor, Arc::new(adapter.clone()))
        .unwrap();

    let source = "// future_marker";
    let detected = detect_with_registry(
        Path::new("extensionless"),
        source.as_bytes(),
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detected.status, DetectionStatus::Selected);
    assert_eq!(detected.candidates[0].identity.format, "future-language");

    let parsed = adapter
        .parse_file("fn broken( {", &CodeIngestOptions::default())
        .unwrap();
    assert!(!parsed.syntax_nodes.is_empty());
    assert!(!parsed.parse_errors.is_empty());
    assert!(parsed.syntax_nodes.iter().all(|node| {
        node.range.byte_start <= node.range.byte_end
            && "fn broken( {".get(node.range.byte_start..node.range.byte_end)
                == Some(node.raw.as_str())
    }));
    let graph = parsed
        .to_document_graph(DocumentGraphContext::new("code:future"))
        .unwrap();
    assert_eq!(graph.kind, DocumentKind::Code);
    assert_eq!(graph.language.as_deref(), Some("future-language"));
    assert!(!graph.nodes.is_empty());
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Diagnostic),
        "tree-sitter recovery nodes project as diagnostics"
    );

    let conflict_config = TreeSitterLanguageConfig::new(
        "future-language",
        "tree-sitter-future-conflict",
        "tree-sitter-rust",
        "0.23.3",
        "rust",
    );
    let conflict =
        TreeSitterLanguageAdapter::new(conflict_config, tree_sitter_rust::LANGUAGE.into()).unwrap();
    assert_eq!(
        registry.register_caller(conflict.caller_descriptor(), Arc::new(conflict)),
        Err(ParserRegistryError::PriorityConflict(
            "future_language".to_string()
        ))
    );

    let complete = registry
        .dispatch("future-language", request(source, "extensionless"), None)
        .unwrap();
    assert_eq!(complete.status, OperationStatus::Complete);

    let recovered = registry
        .dispatch(
            "future-language",
            request("fn broken( {", "broken.future"),
            None,
        )
        .unwrap();
    assert_eq!(recovered.status, OperationStatus::Partial);
    assert!(!recovered.diagnostics.is_empty());
    assert!(recovered.diagnostics.iter().all(|diagnostic| {
        diagnostic.partial
            && diagnostic.range.is_some()
            && diagnostic
                .recovery
                .as_ref()
                .is_some_and(|recovery| recovery.kind == grist::core::RecoveryKind::InspectInput)
    }));
    let value = recovered.payload.unwrap();
    let payload: CodeFile = serde_json::from_value(value).unwrap();
    assert_eq!(
        payload.adapter.schema_version,
        grist::code::TREE_SITTER_ADAPTER_SCHEMA_V1
    );
    assert!(!payload.adapter.active_content_execution);
    assert!(!payload.adapter.network_access);

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nodes = Some(1);
    let limited_request = ParseRequest::new(
        RequestId::new("code-language-node-budget").unwrap(),
        Input::utf8("fn main() {}"),
        SourceInfo::stdin("budget.future"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let limited = registry
        .dispatch("future-language", limited_request, None)
        .unwrap();
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(limited.payload.is_none());
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.class == DiagnosticClass::ResourceBudgetExhaustion })
    );

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nesting_depth = Some(1);
    let depth_limited_request = ParseRequest::new(
        RequestId::new("code-language-depth-budget").unwrap(),
        Input::utf8("fn main() {}"),
        SourceInfo::stdin("depth.future"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let depth_limited = registry
        .dispatch("future-language", depth_limited_request, None)
        .unwrap();
    assert_eq!(depth_limited.status, OperationStatus::Failed);
    assert!(depth_limited.payload.is_none());
    assert!(
        depth_limited
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.class == DiagnosticClass::ResourceBudgetExhaustion })
    );
}

#[cfg(all(
    feature = "go",
    feature = "java",
    feature = "kotlin",
    feature = "c",
    feature = "cpp",
    feature = "csharp",
    feature = "ruby",
    feature = "php",
    feature = "swift",
    feature = "bash",
    feature = "sql",
    feature = "css",
    feature = "document-graph"
))]
#[test]
fn every_enabled_secondary_grammar_passes_common_gates() {
    use grist::code::{CodeFile, TreeSitterAdapterCapability};

    let registry = builtin_parser_registry().unwrap();
    let cases = [
        (
            "go",
            "main.go",
            "package main\nfunc main() {}",
            "package main\nfunc broken( {",
        ),
        (
            "java",
            "Main.java",
            "public class Main {}",
            "public class Broken { void f( {",
        ),
        (
            "kotlin",
            "main.kt",
            "data class User(val name: String)",
            "data class Broken(val name: String",
        ),
        ("c", "main.c", "typedef int number;", "int main( {"),
        (
            "cpp",
            "main.cpp",
            "namespace demo { auto value = std::vector<int>{}; }",
            "namespace demo { void broken( {",
        ),
        (
            "csharp",
            "Main.cs",
            "using System; class Main { void Run() { Console.WriteLine(1); } }",
            "using System; class Broken { void Run( {",
        ),
        ("ruby", "main.rb", "def value\n  1\nend", "def broken("),
        (
            "php",
            "main.php",
            "<?php function value() { return 1; }",
            "<?php function broken( {",
        ),
        (
            "swift",
            "main.swift",
            "import Foundation\nfunc value() -> Int { let x = 1; return x }",
            "func broken( {",
        ),
        (
            "shell",
            "main.sh",
            "#!/bin/bash\necho ok",
            "#!/bin/bash\nif then",
        ),
        (
            "sql",
            "main.sql",
            "SELECT value FROM items;",
            "SELECT ( FROM",
        ),
        ("css", "main.css", ".item { color: red; }", ".item { color:"),
    ];

    for (format, filename, valid, malformed) in cases {
        let selected = registry.select_format(format);
        let descriptor = match selected {
            ParserSelection::Available(descriptor) => descriptor,
            other => panic!("{format} was not available: {other:?}"),
        };
        assert_eq!(
            descriptor.parser.grammar_version.as_deref(),
            descriptor.parser.implementation_version.as_deref()
        );
        assert!(
            descriptor
                .capabilities
                .contains(&grist::registry::Capability::GrammarDetection)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&grist::registry::Capability::SourceRanges)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&grist::registry::Capability::ErrorRecovery)
        );

        let detected = detect_with_registry(
            Path::new(filename),
            valid.as_bytes(),
            None,
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detected.status, DetectionStatus::Selected, "{format}");
        assert_eq!(
            detected.selected_parser.as_deref(),
            Some(descriptor.id.as_str()),
            "{format}"
        );
        assert!(
            detected.candidates[0]
                .evidence
                .iter()
                .any(|evidence| matches!(
                    evidence.kind,
                    grist::core::DetectionEvidenceKind::GrammarProbe
                )),
            "{format}"
        );

        let extensionless = detect_with_registry(
            Path::new("extensionless"),
            valid.as_bytes(),
            None,
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(
            extensionless.status,
            DetectionStatus::Selected,
            "{format}: {extensionless:#?}"
        );
        assert_eq!(
            extensionless.candidates[0].identity.format, format,
            "{format}"
        );

        let envelope = registry
            .dispatch(format, request(malformed, filename), None)
            .unwrap();
        assert_eq!(envelope.status, OperationStatus::Partial, "{format}");
        assert!(!envelope.diagnostics.is_empty(), "{format}");
        assert!(
            envelope.diagnostics.iter().all(|diagnostic| {
                diagnostic.partial
                    && diagnostic.range.is_some()
                    && diagnostic.recovery.as_ref().is_some_and(|recovery| {
                        recovery.kind == grist::core::RecoveryKind::InspectInput
                    })
            }),
            "{format}"
        );
        let payload: CodeFile = serde_json::from_value(envelope.payload.unwrap()).unwrap();
        assert_eq!(payload.adapter.language, format);
        assert!(
            payload
                .adapter
                .capabilities
                .contains(&TreeSitterAdapterCapability::SourceRanges)
        );
        assert!(!payload.syntax_nodes.is_empty(), "{format}");
        assert!(!payload.parse_errors.is_empty(), "{format}");
        for node in &payload.syntax_nodes {
            assert_eq!(
                malformed.get(node.range.byte_start..node.range.byte_end),
                Some(node.raw.as_str()),
                "{format}"
            );
        }
        let graph = payload
            .to_document_graph(DocumentGraphContext::new(format!("code:{format}")))
            .unwrap();
        assert_eq!(graph.kind, DocumentKind::Code);
        assert_eq!(graph.language.as_deref(), Some(format));
        assert!(!graph.nodes.is_empty(), "{format}");
    }
}
