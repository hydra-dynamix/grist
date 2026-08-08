#![cfg(all(
    feature = "rust",
    feature = "python",
    feature = "javascript",
    feature = "typescript",
    feature = "document-graph",
    feature = "schemas"
))]

use grist::core::{ArtifactKind, DetectionEvidenceKind, Limits, SchemaVersion, SourceInfo};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::{ParserSelection, builtin_parser_registry};
use std::collections::BTreeSet;
use std::path::Path;

#[test]
fn rust_preserves_semantics_tests_comments_syntax_and_recovery() {
    let source = r#"use std::fmt as formatting;
pub trait Render { fn render(&self) -> String; }
pub struct View;
impl Render for View {
    /// Render a view.
    fn render(&self) -> String {
        let mut value = String::new();
        if value.is_empty() { value = formatting::format(format_args!("ok")); }
        return value;
    }
}
#[test]
fn renders() { assert_eq!(View.render(), "ok"); }
fn broken( {
"#;
    let envelope = grist::rust::parse_rust(
        source,
        SourceInfo::stdin("lib.rs"),
        &grist::rust::RustIngestOptions {
            detail: grist::rust::RustDetailMode::SyntaxDebug,
        },
    );
    let payload = envelope.payload.as_ref().unwrap();
    assert!(!payload.imports.is_empty());
    assert!(!payload.exports.is_empty());
    assert!(!payload.assignments.is_empty());
    assert!(!payload.returns.is_empty());
    assert!(!payload.calls.is_empty());
    assert!(!payload.branches.is_empty());
    assert!(!payload.inheritances.is_empty());
    assert_eq!(payload.tests[0].name, "renders");
    assert!(payload.comments.iter().any(|comment| comment.doc));
    assert!(
        payload
            .syntax_nodes
            .iter()
            .any(|node| node.kind == "function_item")
    );
    assert!(!payload.parse_errors.is_empty());
    assert!(payload.parse_errors.iter().all(|error| {
        source.get(error.range.byte_start..error.range.byte_end) == Some(error.raw.as_str())
    }));

    let first = payload
        .to_document_graph(DocumentGraphContext::new("code:rust"))
        .unwrap();
    let second = payload
        .to_document_graph(DocumentGraphContext::new("code:rust"))
        .unwrap();
    assert_eq!(first, second);
    assert!(
        first
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Class)
    );
}

#[test]
fn python_preserves_exports_inheritance_decorators_tests_comments_and_recovery() {
    let source = r#"from base import Base as Parent
__all__ = ["Service"]
# service comment
class Service(Parent):
    """Service docs."""
    @classmethod
    def build(cls):
        result = helper()
        if result:
            return result

@pytest.mark.unit
def test_service():
    Service.build()

def broken(:
"#;
    let envelope = grist::python::parse_python(
        source,
        SourceInfo::stdin("service.py"),
        &grist::python::PythonIngestOptions {
            detail: grist::python::PythonDetailMode::SyntaxDebug,
        },
    );
    let payload = envelope.payload.as_ref().unwrap();
    assert_eq!(payload.exports[0].names, vec!["Service"]);
    assert!(
        payload
            .symbols
            .iter()
            .any(|symbol| symbol.superclasses == ["Parent"])
    );
    assert!(payload.symbols.iter().any(|symbol| {
        symbol
            .decorators
            .iter()
            .any(|value| value.contains("classmethod"))
    }));
    assert_eq!(payload.tests[0].name, "test_service");
    assert!(!payload.comments.is_empty());
    assert!(!payload.syntax_nodes.is_empty());
    assert!(!payload.parse_errors.is_empty());
    assert_eq!(
        serde_json::to_vec(payload).unwrap(),
        serde_json::to_vec(payload).unwrap()
    );
}

#[test]
fn javascript_typescript_tsx_and_jsx_are_registered_and_semantic() {
    let registry = builtin_parser_registry().unwrap();
    for format in ["javascript", "typescript", "tsx", "jsx"] {
        assert!(matches!(
            registry.select_format(format),
            ParserSelection::Available(_)
        ));
    }

    let javascript = r#"#!/usr/bin/env node
/** suite */
import value from "pkg";
export class Child extends Base {
  run() { if (value) return helper(value); }
}
test("child", () => new Child().run());
"#;
    let detected = detect_with_registry(
        Path::new("extensionless"),
        javascript.as_bytes(),
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detected.status, DetectionStatus::Selected);
    assert_eq!(detected.content_kind, ContentKind::JavaScript);
    assert!(detected.candidates[0].evidence.iter().any(|evidence| {
        matches!(
            evidence.kind,
            DetectionEvidenceKind::Shebang | DetectionEvidenceKind::GrammarProbe
        )
    }));

    for (source, expected) in [
        (
            "interface User { name: string }\nconst user: User = { name: 'a' };",
            ContentKind::TypeScript,
        ),
        ("export default <section>Hello</section>;", ContentKind::Jsx),
        (
            "interface Props { label: string }\nexport const View = (p: Props) => <div>{p.label}</div>;",
            ContentKind::Tsx,
        ),
    ] {
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
        assert_eq!(detected.status, DetectionStatus::Selected, "{source}");
        assert_eq!(detected.content_kind, expected, "{source}");
    }

    let envelope = grist::javascript::parse_javascript(
        javascript,
        SourceInfo::stdin("extensionless"),
        &grist::javascript::JavaScriptIngestOptions {
            dialect: grist::javascript::JavaScriptDialect::JavaScript,
            detail: grist::javascript::JavaScriptDetailMode::SyntaxDebug,
        },
    );
    assert_eq!(envelope.kind, ArtifactKind::JavaScriptCode);
    assert_eq!(
        envelope.payload_schema_version.0,
        SchemaVersion::JAVASCRIPT_CODE_V1
    );
    let payload = envelope.payload.as_ref().unwrap();
    assert_eq!(
        payload.dialect,
        grist::javascript::JavaScriptDialect::JavaScript
    );
    assert!(
        payload
            .syntax_nodes
            .iter()
            .all(|node| node.id.starts_with("javascript-syntax-"))
    );
    assert!(
        payload
            .symbols
            .iter()
            .any(|symbol| symbol.language == "javascript")
    );
    assert!(
        payload
            .symbols
            .iter()
            .any(|symbol| symbol.extends == ["Base"])
    );
    assert_eq!(payload.tests.len(), 1);
    assert!(!payload.comments.is_empty());
    assert!(!payload.syntax_nodes.is_empty());
    assert!(!payload.imports.is_empty() && !payload.exports.is_empty());
    assert!(
        !payload.calls.is_empty() && !payload.returns.is_empty() && !payload.branches.is_empty()
    );
    let graph = payload
        .to_document_graph(DocumentGraphContext::new("code:javascript"))
        .unwrap();
    assert_eq!(graph.kind, grist::document_graph::DocumentKind::JavaScript);
    assert_eq!(graph.language.as_deref(), Some("javascript"));
    assert_eq!(
        graph
            .projection
            .as_ref()
            .map(|projection| projection.authoritative_payload_schema_version.as_str()),
        Some(SchemaVersion::JAVASCRIPT_CODE_V1)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Inherits)
    );

    for (dialect, source) in [
        (
            grist::typescript::TypeScriptDialect::TypeScript,
            "interface A { x: number }",
        ),
        (
            grist::typescript::TypeScriptDialect::Tsx,
            "const V = () => <div />;",
        ),
        (
            grist::typescript::TypeScriptDialect::Jsx,
            "const V = () => <div />;",
        ),
    ] {
        let parsed = grist::typescript::parse_typescript(
            source,
            SourceInfo::stdin("code"),
            &grist::typescript::TypeScriptIngestOptions {
                dialect,
                detail: grist::typescript::TypeScriptDetailMode::SyntaxDebug,
            },
        );
        assert!(parsed.payload.unwrap().syntax_nodes.len() > 1);
    }

    let schema = grist::schema::schema_json("javascript-code").unwrap();
    assert!(schema.to_string().contains("syntax_nodes"));
}

#[test]
fn semantic_recovery_nodes_are_retained_with_unique_ids() {
    let source = "function broken(a { return a; }";
    let parsed = grist::javascript::parse_javascript(
        source,
        SourceInfo::stdin("broken.js"),
        &grist::javascript::JavaScriptIngestOptions::default(),
    );
    let payload = parsed.payload.unwrap();
    assert!(!payload.syntax_nodes.is_empty());
    assert!(
        payload
            .syntax_nodes
            .iter()
            .all(|node| node.error || node.missing)
    );
    let ids = payload
        .syntax_nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), payload.syntax_nodes.len());
}
