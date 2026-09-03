#![cfg(all(
    feature = "document-graph",
    feature = "markdown",
    feature = "latex",
    feature = "python",
    feature = "rust",
    feature = "typescript"
))]

use grist::core::SourceInfo;
use grist::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    DocumentRelation, ToDocumentGraph, TransformOptions, extract_conditional_obligations,
    render_latex, render_markdown,
};

#[test]
fn cross_format_document_graph_golden_matrix() {
    let markdown = grist::markdown::parse_markdown(
        "# Intro\n\nIf the file is executable, it must have a shebang.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n",
        SourceInfo::stdin("rules.md"),
    )
    .payload.as_ref().expect("complete operation payload")
    .to_document_graph(DocumentGraphContext::new("golden:markdown"))
    .expect("markdown graph");
    assert_eq!(markdown.kind, DocumentKind::Markdown);
    assert!(
        markdown
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Heading)
    );
    assert!(
        markdown
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::TableCell)
    );

    let mut obligations = markdown.clone();
    assert_eq!(extract_conditional_obligations(&mut obligations), 1);
    assert!(
        obligations
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ConditionalOn)
    );

    let latex = render_latex(&markdown, TransformOptions::default()).expect("markdown to latex");
    assert!(latex.contains("\\section{Intro}"));
    assert!(latex.contains("If the file is executable"));

    let latex_graph = grist::latex::parse_latex(
        "\\section{Intro}\nSee \\label{sec:intro} \\ref{sec:intro} \\cite{paper} and $x$.\n",
        SourceInfo::stdin("paper.tex"),
        &grist::latex::LatexOptions::default(),
    )
    .payload
    .as_ref()
    .expect("complete operation payload")
    .to_document_graph(DocumentGraphContext::new("golden:latex"))
    .expect("latex graph");
    assert_eq!(latex_graph.kind, DocumentKind::Latex);
    assert!(latex_graph
        .edges
        .iter()
        .any(|edge| edge.relation == DocumentRelation::References && edge.target == "sec:intro"));
    let markdown_from_latex =
        render_markdown(&latex_graph, TransformOptions::default()).expect("latex to markdown");
    assert!(markdown_from_latex.contains("# Intro"));

    let python = grist::python::parse_python(
        "from base import Base\nclass Form(Base):\n    def create(self):\n        return helper()\ndef helper():\n    return 1\n",
        SourceInfo::stdin("forms.py"),
        &grist::python::PythonIngestOptions::default(),
    )
    .payload.as_ref().expect("complete operation payload")
    .to_document_graph(DocumentGraphContext::new("golden:python"))
    .expect("python graph");
    assert!(
        python
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Inherits)
    );
    assert!(
        python
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Calls)
    );

    let rust = grist::rust::parse_rust(
        "use std::fmt;\npub struct Form;\nimpl Form { pub fn create(&self) {} }\n",
        SourceInfo::stdin("lib.rs"),
        &grist::rust::RustIngestOptions::default(),
    )
    .payload
    .as_ref()
    .expect("complete operation payload")
    .to_document_graph(DocumentGraphContext::new("golden:rust"))
    .expect("rust graph");
    assert!(
        rust.nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Class)
    );
    assert!(
        rust.edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Imports)
    );

    let ts = grist::typescript::parse_typescript(
        "import { helper } from './helper';\nexport class Form { create() { return helper(); } }\n",
        SourceInfo::stdin("form.ts"),
        &grist::typescript::TypeScriptIngestOptions::default(),
    )
    .payload
    .as_ref()
    .expect("complete operation payload")
    .to_document_graph(DocumentGraphContext::new("golden:typescript"))
    .expect("typescript graph");
    assert!(
        ts.edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Imports)
    );
    assert!(
        ts.edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::Exports)
    );

    let mut unsupported = DocumentGraph::new("golden:unsupported", DocumentKind::Markdown);
    unsupported.add_node(DocumentNode::new("root", DocumentNodeKind::Document));
    unsupported.add_node(DocumentNode::new("fn", DocumentNodeKind::Function));
    unsupported.add_contains("root", "fn");
    assert!(render_markdown(&unsupported, TransformOptions::default()).is_err());
}
