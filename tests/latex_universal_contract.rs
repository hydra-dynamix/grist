#![cfg(all(feature = "latex", feature = "schemas", feature = "document-graph"))]

use grist::core::{
    BudgetSelection, Input, Limits, OperationStatus, ParseRequest, ProviderSet, RequestId,
    ResourceBudget, SourceInfo,
};
use grist::detect::{DetectionOptions, DetectionStatus, detect_source};
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::latex::{
    LatexIncludeStatus, LatexNode, LatexNodeKind, LatexOptions, parse_latex, parse_latex_bytes,
};
use grist::registry::builtin_parser_registry;
use std::fs;

const RICH: &str = r#"\documentclass{article}
\usepackage{graphicx}
\newcommand{\greet}[1]{Hello #1}
\title{Universal paper}
\begin{document}
\section{Introduction}\label{sec:intro}
\greet{reader}; see \ref{sec:intro}, \eqref{eq:one}, and \citep{source}.
\begin{equation}x=1\label{eq:one}\end{equation}
\begin{itemize}\item One\item Two\end{itemize}
\begin{table}\begin{tabular}{cc}A&B\\1&2\end{tabular}\caption{Data}\end{table}
\begin{figure}\includegraphics{plot.png}\caption{Plot}\end{figure}
% retained comment
\unknowncommand{raw}
\write18{must-not-run}
\end{document}"#;

#[test]
fn rich_payload_graph_and_schema_are_deterministic() {
    let first = parse_latex(
        RICH,
        SourceInfo::stdin("paper.tex"),
        &LatexOptions::default(),
    );
    let second = parse_latex(
        RICH,
        SourceInfo::stdin("paper.tex"),
        &LatexOptions::default(),
    );
    assert_eq!(first, second);
    assert_eq!(first.status, OperationStatus::Complete);
    let payload = first.payload.as_ref().unwrap();
    for kind in [
        LatexNodeKind::DocumentClass,
        LatexNodeKind::Metadata,
        LatexNodeKind::MacroDefinition,
        LatexNodeKind::MacroUse,
        LatexNodeKind::Section,
        LatexNodeKind::Equation,
        LatexNodeKind::List,
        LatexNodeKind::ListItem,
        LatexNodeKind::Table,
        LatexNodeKind::TableRow,
        LatexNodeKind::TableCell,
        LatexNodeKind::Figure,
        LatexNodeKind::Caption,
        LatexNodeKind::Label,
        LatexNodeKind::Ref,
        LatexNodeKind::Citation,
        LatexNodeKind::Comment,
        LatexNodeKind::RawCommand,
    ] {
        assert!(
            payload.nodes.iter().any(|node| node.kind == kind),
            "{kind:?}"
        );
    }
    assert!(payload.nodes.iter().all(|node| {
        node.raw == payload.decoded_text[node.range.byte_start..node.range.byte_end]
            && node
                .locator
                .as_ref()
                .is_some_and(|locator| locator.validate().is_ok())
    }));
    let graph = payload
        .to_document_graph(
            DocumentGraphContext::new("latex:rich").with_source(first.source.clone()),
        )
        .unwrap();
    graph.validate_contract().unwrap();
    for kind in [
        DocumentNodeKind::Heading,
        DocumentNodeKind::Equation,
        DocumentNodeKind::List,
        DocumentNodeKind::Table,
        DocumentNodeKind::Figure,
        DocumentNodeKind::Caption,
        DocumentNodeKind::Citation,
        DocumentNodeKind::Reference,
    ] {
        assert!(graph.nodes.iter().any(|node| node.kind == kind), "{kind:?}");
    }
    for (name, value) in [
        ("latex", serde_json::to_value(payload).unwrap()),
        ("latex-envelope", serde_json::to_value(&first).unwrap()),
        (
            "latex-options",
            serde_json::to_value(LatexOptions::default()).unwrap(),
        ),
    ] {
        let report = grist::schema::validate_schema(name, &value).unwrap();
        assert!(report.valid, "{name}: {:?}", report.issues);
    }
}

fn include_statuses(nodes: &[LatexNode], statuses: &mut Vec<LatexIncludeStatus>) {
    for node in nodes {
        if let Some(include) = &node.include {
            statuses.push(include.status.clone());
            if let Some(resolved) = &include.resolved {
                include_statuses(&resolved.nodes, statuses);
            }
        }
    }
}

#[test]
fn project_cycles_and_boundary_escapes_are_safe() {
    let base = std::env::temp_dir().join(format!("grist-latex-contract-{}", std::process::id()));
    let root = base.join("project");
    fs::create_dir_all(root.join("chapters")).unwrap();
    let main = root.join("main.tex");
    fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\\input{chapters/one}\n\\input{../outside}\n\\end{document}\n",
    ).unwrap();
    fs::write(
        root.join("chapters/one.tex"),
        "\\section{Child}\\input{../main}\n",
    )
    .unwrap();
    fs::write(base.join("outside.tex"), "outside").unwrap();
    let options = LatexOptions {
        allowed_roots: vec![root.clone()],
        ..Default::default()
    };
    let envelope = parse_latex_bytes(
        &fs::read(&main).unwrap(),
        SourceInfo::from_path(&main),
        &options,
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let payload = envelope.payload.unwrap();
    assert_eq!(payload.project.selected_root.as_deref(), Some("main.tex"));
    let mut statuses = Vec::new();
    include_statuses(&payload.nodes, &mut statuses);
    assert!(statuses.contains(&LatexIncludeStatus::Resolved));
    assert!(statuses.contains(&LatexIncludeStatus::Cycle));
    assert!(statuses.contains(&LatexIncludeStatus::OutsideAllowedRoots));
    let child = payload
        .nodes
        .iter()
        .filter_map(|node| node.include.as_ref())
        .find_map(|include| include.resolved.as_ref())
        .unwrap();
    assert!(child.nodes.iter().all(|node| {
        node.source
            .as_ref()
            .and_then(|source| source.repository_relative_path.as_deref())
            == Some("chapters/one.tex")
    }));
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn macro_limits_and_malformed_tex_are_partial() {
    let options = LatexOptions {
        max_macro_depth: 2,
        max_macro_expansions: 2,
        ..Default::default()
    };
    let limited = parse_latex(
        "\\newcommand{\\loop}{\\loop}\\loop\n\\begin{figure}\n",
        SourceInfo::stdin("limited.tex"),
        &options,
    );
    assert_eq!(limited.status, OperationStatus::Partial);
    assert!(limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "latex.macro.expansion_limited"
            || diagnostic.code.as_str() == "latex.parse.malformed"
    }));
}

#[test]
fn detection_registry_and_budget_contracts_agree() {
    let registry = builtin_parser_registry().unwrap();
    for (name, bytes) in [
        ("extensionless", RICH.as_bytes()),
        (
            "mislabeled.bin",
            b"\\section{One}\n\\begin{equation}x\\end{equation}",
        ),
        ("malformed.tex", b"\\documentclass{article"),
    ] {
        let detection = detect_source(
            &SourceInfo::stdin(name),
            bytes,
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detection.status, DetectionStatus::Selected, "{name}");
        assert_eq!(
            detection
                .selected_format_identity()
                .map(|value| value.format),
            Some("latex".to_string())
        );
    }
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_decoded_characters = Some(1);
    let request = ParseRequest::new(
        RequestId::new("latex-budget").unwrap(),
        Input::bytes(RICH.as_bytes().to_vec()),
        SourceInfo::stdin("budget.tex"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    let failed = registry.dispatch("latex", request, None).unwrap();
    assert_eq!(failed.status, OperationStatus::Failed);
}
