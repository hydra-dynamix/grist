#![cfg(all(
    feature = "rust",
    feature = "python",
    feature = "javascript",
    feature = "typescript",
    feature = "document-graph"
))]

use grist::core::{ContentIdentity, SourceInfo, SourceRange, canonical_json_bytes};
use grist::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentNodeKind, DocumentRelation, RelationEvidence,
    ToDocumentGraph,
};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::collections::BTreeSet;

fn assert_golden_projection(graph: &DocumentGraph, namespace: &str, source: &str) {
    graph.validate_contract().unwrap();
    assert_eq!(
        serde_json::from_value::<DocumentGraph>(serde_json::to_value(graph).unwrap()).unwrap(),
        *graph,
        "DocumentGraph wire round-trip changed the projection"
    );

    let mut kinds = BTreeSet::new();
    for node in &graph.nodes {
        kinds.insert(format!("{:?}", node.kind));
        if let Some(range) = &node.range {
            assert!(
                source.get(range.byte_start..range.byte_end).is_some(),
                "{} has an invalid source range",
                node.id
            );
            assert!(node.locator.is_some(), "{} lost its exact locator", node.id);
        }
        let native = node
            .extensions
            .get(namespace)
            .unwrap_or_else(|| panic!("{} lost its native {} payload", node.id, namespace));
        if let Some(native_range) = native.get("range") {
            let native_range: SourceRange = serde_json::from_value(native_range.clone()).unwrap();
            assert_eq!(
                node.range.as_ref(),
                Some(&native_range),
                "{} changed its authoritative native range",
                node.id
            );
        }
    }

    for expected in [
        DocumentNodeKind::Module,
        DocumentNodeKind::Class,
        DocumentNodeKind::Method,
        DocumentNodeKind::Import,
        DocumentNodeKind::Export,
        DocumentNodeKind::Assignment,
        DocumentNodeKind::Return,
        DocumentNodeKind::Call,
        DocumentNodeKind::Branch,
        DocumentNodeKind::Comment,
        DocumentNodeKind::CodeSymbol,
        DocumentNodeKind::Other("grammar_node".into()),
    ] {
        assert!(
            graph.nodes.iter().any(|node| node.kind == expected),
            "missing normalized {expected:?}"
        );
    }

    for relation in [
        DocumentRelation::Contains,
        DocumentRelation::Imports,
        DocumentRelation::Exports,
        DocumentRelation::Calls,
        DocumentRelation::Assigns,
        DocumentRelation::Returns,
        DocumentRelation::ConditionalOn,
        DocumentRelation::Other("tests".into()),
    ] {
        assert!(
            graph.edges.iter().any(|edge| edge.relation == relation),
            "missing normalized {relation:?}"
        );
    }

    let inferred = graph
        .edges
        .iter()
        .filter_map(|edge| match &edge.evidence {
            RelationEvidence::Inferred { inference } => Some(inference),
            RelationEvidence::Explicit { .. } => None,
        })
        .collect::<Vec<_>>();
    assert!(!inferred.is_empty());
    assert!(inferred.iter().all(|inference| {
        !inference.rule.trim().is_empty()
            && inference.confidence.get() > 0.0
            && inference.confidence.get() <= 1.0
            && !inference.evidence_locators.is_empty()
    }));

    assert!(graph.nodes.iter().any(|node| {
        node.kind == DocumentNodeKind::Other("grammar_node".into())
            && node.raw.is_some()
            && node.attrs.contains_key("grammar_kind")
    }));
    assert!(!kinds.is_empty());
}

fn assert_code_atomic(graph: &DocumentGraph, source: &str) {
    let source_identity =
        ContentIdentity::for_raw_bytes(source.as_bytes()).with_decoded(source, "utf-8", false);
    let document_identity = ContentIdentity::for_raw_bytes(&canonical_json_bytes(graph).unwrap());
    let options = SegmentOptions {
        target_size: 1,
        maximum_size: 1,
        include_heading_ancestry: false,
        ..SegmentOptions::default()
    };
    let segmented =
        segment_document_graph(graph, &source_identity, &document_identity, &options, None)
            .unwrap();
    let function_ids = graph
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.kind,
                DocumentNodeKind::Function | DocumentNodeKind::Method
            )
        })
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    for function_id in function_ids {
        assert_eq!(
            segmented
                .segments
                .iter()
                .filter(|segment| segment
                    .node_ids
                    .iter()
                    .any(|node_id| node_id == function_id))
                .count(),
            1,
            "each symbol must occur in exactly one atomic code segment"
        );
    }
    let non_renderable = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == DocumentNodeKind::Other("grammar_node".into())
                || node.kind == DocumentNodeKind::Diagnostic
        })
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    assert!(
        segmented.segments.iter().all(|segment| {
            segment
                .node_ids
                .iter()
                .all(|node_id| !non_renderable.contains(node_id.as_str()))
        }),
        "syntax and recovery payloads must not duplicate rendered segment text"
    );
    assert!(segmented.segments.iter().any(|segment| {
        segment
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "segment.maximum_exceeded_for_atomic_source")
    }));
}

#[test]
fn cross_language_golden_projection_is_exact_native_and_order_independent() {
    let python_source = include_str!("../fixtures/generated/code_semantics/python.py");
    let python = grist::python::parse_python(
        python_source,
        SourceInfo::stdin("golden.py"),
        &grist::python::PythonIngestOptions {
            detail: grist::python::PythonDetailMode::SyntaxDebug,
        },
    )
    .payload
    .unwrap();
    let python_graph = python
        .to_document_graph(DocumentGraphContext::new("golden:python"))
        .unwrap();
    assert_golden_projection(&python_graph, "grist.python", python_source);
    assert_code_atomic(&python_graph, python_source);
    assert!(
        python_graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Inherits)
    );
    let mut reordered_python = python.clone();
    reordered_python.symbols.reverse();
    reordered_python.imports.reverse();
    reordered_python.exports.reverse();
    reordered_python.assignments.reverse();
    reordered_python.returns.reverse();
    reordered_python.calls.reverse();
    reordered_python.branches.reverse();
    reordered_python.tests.reverse();
    reordered_python.comments.reverse();
    reordered_python.syntax_nodes.reverse();
    reordered_python.parse_errors.reverse();
    assert_eq!(
        python_graph,
        reordered_python
            .to_document_graph(DocumentGraphContext::new("golden:python"))
            .unwrap()
    );

    let rust_source = include_str!("../fixtures/generated/code_semantics/rust.rs");
    let rust = grist::rust::parse_rust(
        rust_source,
        SourceInfo::stdin("golden.rs"),
        &grist::rust::RustIngestOptions {
            detail: grist::rust::RustDetailMode::SyntaxDebug,
        },
    )
    .payload
    .unwrap();
    let rust_graph = rust
        .to_document_graph(DocumentGraphContext::new("golden:rust"))
        .unwrap();
    assert_golden_projection(&rust_graph, "grist.rust", rust_source);
    assert_code_atomic(&rust_graph, rust_source);
    assert!(
        rust_graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Implements
                && edge.extensions.contains_key("grist.rust"))
    );
    assert!(rust_graph.nodes.iter().any(|node| {
        node.kind == DocumentNodeKind::Other("inheritance".into())
            && node.extensions.get("grist.rust")
                == serde_json::to_value(&rust.inheritances[0]).ok().as_ref()
    }));
    let rust_qualified_names = rust_graph
        .nodes
        .iter()
        .filter_map(|node| {
            let native_id = node.extensions.get("grist.rust")?.get("id")?.as_str()?;
            native_id
                .starts_with("rust-symbol-")
                .then(|| node.qualified_name.clone())
                .flatten()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rust_qualified_names.iter().collect::<BTreeSet<_>>().len(),
        rust.symbols.len(),
        "Rust symbol qualification is not unique: {rust_qualified_names:?}"
    );
    assert!(!rust_qualified_names.iter().any(|name| name == "golden.rs"));
    let mut reordered_rust = rust.clone();
    reordered_rust.symbols.reverse();
    reordered_rust.imports.reverse();
    reordered_rust.exports.reverse();
    reordered_rust.assignments.reverse();
    reordered_rust.returns.reverse();
    reordered_rust.calls.reverse();
    reordered_rust.branches.reverse();
    reordered_rust.inheritances.reverse();
    reordered_rust.tests.reverse();
    reordered_rust.comments.reverse();
    reordered_rust.syntax_nodes.reverse();
    reordered_rust.parse_errors.reverse();
    assert_eq!(
        rust_graph,
        reordered_rust
            .to_document_graph(DocumentGraphContext::new("golden:rust"))
            .unwrap()
    );

    let javascript_source = include_str!("../fixtures/generated/code_semantics/javascript.js");
    let javascript = grist::javascript::parse_javascript(
        javascript_source,
        SourceInfo::stdin("golden.js"),
        &grist::javascript::JavaScriptIngestOptions {
            dialect: grist::javascript::JavaScriptDialect::JavaScript,
            detail: grist::javascript::JavaScriptDetailMode::SyntaxDebug,
        },
    )
    .payload
    .unwrap();
    let javascript_graph = javascript
        .to_document_graph(DocumentGraphContext::new("golden:javascript"))
        .unwrap();
    assert_golden_projection(&javascript_graph, "grist.javascript", javascript_source);
    assert_code_atomic(&javascript_graph, javascript_source);
    assert!(
        javascript_graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Inherits)
    );
    let mut reordered_javascript = javascript.clone();
    reordered_javascript.symbols.reverse();
    reordered_javascript.imports.reverse();
    reordered_javascript.exports.reverse();
    reordered_javascript.assignments.reverse();
    reordered_javascript.returns.reverse();
    reordered_javascript.calls.reverse();
    reordered_javascript.branches.reverse();
    reordered_javascript.tests.reverse();
    reordered_javascript.comments.reverse();
    reordered_javascript.syntax_nodes.reverse();
    reordered_javascript.parse_errors.reverse();
    assert_eq!(
        javascript_graph,
        reordered_javascript
            .to_document_graph(DocumentGraphContext::new("golden:javascript"))
            .unwrap()
    );

    let typescript_source = include_str!("../fixtures/generated/code_semantics/typescript.ts");
    let typescript = grist::typescript::parse_typescript(
        typescript_source,
        SourceInfo::stdin("golden.ts"),
        &grist::typescript::TypeScriptIngestOptions {
            dialect: grist::typescript::TypeScriptDialect::TypeScript,
            detail: grist::typescript::TypeScriptDetailMode::SyntaxDebug,
        },
    )
    .payload
    .unwrap();
    let typescript_graph = typescript
        .to_document_graph(DocumentGraphContext::new("golden:typescript"))
        .unwrap();
    assert_golden_projection(&typescript_graph, "grist.typescript", typescript_source);
    assert_code_atomic(&typescript_graph, typescript_source);
    assert!(
        typescript_graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Inherits)
    );
    assert!(
        typescript_graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Implements)
    );
    let mut reordered_typescript = typescript.clone();
    reordered_typescript.symbols.reverse();
    reordered_typescript.imports.reverse();
    reordered_typescript.exports.reverse();
    reordered_typescript.assignments.reverse();
    reordered_typescript.returns.reverse();
    reordered_typescript.calls.reverse();
    reordered_typescript.branches.reverse();
    reordered_typescript.tests.reverse();
    reordered_typescript.comments.reverse();
    reordered_typescript.syntax_nodes.reverse();
    reordered_typescript.parse_errors.reverse();
    assert_eq!(
        typescript_graph,
        reordered_typescript
            .to_document_graph(DocumentGraphContext::new("golden:typescript"))
            .unwrap()
    );
}

#[test]
fn jsx_and_tsx_project_distinct_dialects_and_relationships() {
    let jsx_source = include_str!("../fixtures/generated/code_semantics/jsx.jsx");
    let jsx = grist::javascript::parse_javascript(
        jsx_source,
        SourceInfo::stdin("golden.jsx"),
        &grist::javascript::JavaScriptIngestOptions {
            dialect: grist::javascript::JavaScriptDialect::Jsx,
            detail: grist::javascript::JavaScriptDetailMode::SyntaxDebug,
        },
    )
    .payload
    .unwrap();
    let jsx_graph = jsx
        .to_document_graph(DocumentGraphContext::new("golden:jsx"))
        .unwrap();
    assert_eq!(jsx_graph.language.as_deref(), Some("jsx"));
    assert_eq!(jsx_graph.dialect.as_deref(), Some("jsx"));
    assert!(
        jsx_graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Inherits)
    );
    assert_code_atomic(&jsx_graph, jsx_source);

    let tsx_source = include_str!("../fixtures/generated/code_semantics/tsx.tsx");
    let tsx = grist::typescript::parse_typescript(
        tsx_source,
        SourceInfo::stdin("golden.tsx"),
        &grist::typescript::TypeScriptIngestOptions {
            dialect: grist::typescript::TypeScriptDialect::Tsx,
            detail: grist::typescript::TypeScriptDetailMode::SyntaxDebug,
        },
    )
    .payload
    .unwrap();
    let tsx_graph = tsx
        .to_document_graph(DocumentGraphContext::new("golden:tsx"))
        .unwrap();
    assert_eq!(tsx_graph.language.as_deref(), Some("tsx"));
    assert_eq!(tsx_graph.dialect.as_deref(), Some("tsx"));
    assert!(
        tsx_graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Implements)
    );
    assert_code_atomic(&tsx_graph, tsx_source);
}

#[test]
fn recovery_projection_is_deterministic_and_non_renderable() {
    let source = include_str!("../fixtures/generated/code_semantics/broken.tsx");
    let parsed = grist::typescript::parse_typescript(
        source,
        SourceInfo::stdin("broken.tsx"),
        &grist::typescript::TypeScriptIngestOptions {
            dialect: grist::typescript::TypeScriptDialect::Tsx,
            detail: grist::typescript::TypeScriptDetailMode::Semantic,
        },
    )
    .payload
    .unwrap();
    assert!(!parsed.parse_errors.is_empty());
    assert!(
        parsed
            .syntax_nodes
            .iter()
            .all(|node| node.error || node.missing)
    );
    let graph = parsed
        .to_document_graph(DocumentGraphContext::new("golden:broken-tsx"))
        .unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.kind == DocumentNodeKind::Other("grammar_node".into()) && node.text.is_none()
    }));
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| { node.kind == DocumentNodeKind::Diagnostic && node.text.is_none() })
    );
    assert_code_atomic(&graph, source);

    let mut reordered = parsed.clone();
    reordered.syntax_nodes.reverse();
    reordered.parse_errors.reverse();
    assert_eq!(
        graph,
        reordered
            .to_document_graph(DocumentGraphContext::new("golden:broken-tsx"))
            .unwrap()
    );
}

#[cfg(feature = "schemas")]
#[test]
fn document_graph_schema_retains_code_extension_contracts() {
    let schema = grist::schema::schema_json("document-graph").unwrap();
    let serialized = schema.to_string();
    assert!(serialized.contains("extensions"));
    assert!(serialized.contains("projection"));
    assert!(serialized.contains("evidence"));
}
