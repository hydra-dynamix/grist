#![cfg(all(feature = "notebooks", feature = "schemas"))]

#[cfg(feature = "cli")]
use grist::core::RequestId;
use grist::core::{ContentIdentity, OperationStatus, SourceInfo};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::markdown::{LocalReferenceStatus, MarkdownDialect, MarkdownNodeKind, MarkdownOptions};
use grist::render::{FidelityMode, RenderFormat, RenderOptions, render_document_graph};
use grist::rmarkdown_quarto::{parse_quarto_with_options, parse_r_markdown};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::fs;
use std::path::PathBuf;

fn fixture_root(name: &str) -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"));
    let root = base.join("rmarkdown-quarto-tests").join(name);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn qmd_payload_preserves_every_notebook_construct_inertly_and_resolves_bounded_references() {
    let root = fixture_root("complete");
    fs::write(root.join("refs.bib"), b"@book{smith, title={Safe}}\n").unwrap();
    fs::write(root.join("child.qmd"), b"included source\n").unwrap();
    fs::write(root.join("child.Rmd"), b"child source\n").unwrap();
    fs::write(root.join("plot.png"), b"inert image bytes").unwrap();
    let source_text = r#"---
title: Notebook
bibliography: refs.bib
---

# Analysis

Cites [@smith] and @jones.

{{< include child.qmd >}}

![Plot](plot.png)

```{r fig-plot, child='child.Rmd', fig.cap='Caption', echo=FALSE}
stop('must never execute')
```

```{python}
#| label: fig-python
#| fig-cap: Python caption
raise RuntimeError('must never execute')
```

::: {.cell-output-display}
stored output only
:::
"#;
    let main = root.join("main.qmd");
    fs::write(&main, source_text).unwrap();
    let report = parse_quarto_with_options(
        source_text,
        SourceInfo::from_path(&main),
        &MarkdownOptions {
            project_root: Some(root.clone()),
            ..MarkdownOptions::default()
        },
    );
    assert_eq!(
        report.status,
        OperationStatus::Complete,
        "{:?}",
        report.diagnostics
    );
    let payload = report.payload.unwrap();
    assert_eq!(payload.dialect, MarkdownDialect::Quarto);
    assert_eq!(
        payload
            .frontmatter
            .as_ref()
            .unwrap()
            .value
            .as_ref()
            .unwrap()["title"],
        "Notebook"
    );

    let executable = payload
        .nodes
        .iter()
        .filter_map(|node| node.executable.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(executable.len(), 2);
    assert!(executable.iter().all(|metadata| !metadata.executed));
    assert!(executable.iter().any(|metadata| metadata.engine == "r" && metadata.label.as_deref() == Some("fig-plot")));
    assert!(
        executable.iter().any(|metadata| metadata.engine == "python"
            && metadata.options["fig-cap"] == "Python caption")
    );
    assert!(payload.nodes.iter().any(|node| {
        node.kind == MarkdownNodeKind::Citation
            && node
                .citation
                .as_ref()
                .is_some_and(|citation| citation.keys == ["smith"])
    }));
    assert!(
        payload
            .nodes
            .iter()
            .any(|node| node.kind == MarkdownNodeKind::Figure && node.figure.is_some())
    );
    assert!(payload.nodes.iter().any(|node| node.kind == MarkdownNodeKind::StoredOutput && node.stored_output.is_some()));
    let references = payload
        .nodes
        .iter()
        .filter_map(|node| node.local_reference.as_ref())
        .collect::<Vec<_>>();
    assert!(references.len() >= 4, "{references:#?}");
    assert!(
        references
            .iter()
            .all(|reference| reference.status == LocalReferenceStatus::Resolved)
    );
    assert!(
        references
            .iter()
            .all(|reference| reference.content_sha256.is_some() && reference.content.is_some())
    );
}

#[test]
fn malformed_metadata_and_unbounded_or_remote_references_are_partial_but_preserved() {
    let source = "---\nbibliography: https://network.invalid/never.bib\n---\n\n{{< include ../escape.qmd >}}\n\n![remote](https://network.invalid/never.png)\n\n```{r}\n#| malformed option\nsystem('never')\n```\n";
    let report = parse_quarto_with_options(
        source,
        SourceInfo::stdin("hostile.qmd"),
        &MarkdownOptions::default(),
    );
    assert_eq!(report.status, OperationStatus::Partial);
    let payload = report.payload.unwrap();
    assert_eq!(payload.decoded_text, source);
    assert!(
        payload
            .nodes
            .iter()
            .filter_map(|node| node.executable.as_ref())
            .all(|metadata| !metadata.executed)
    );
    for code in [
        "executable.metadata",
        "reference.project_root_required",
        "reference.remote_disabled",
    ] {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{code}: {:?}",
            report.diagnostics
        );
    }
    assert!(
        payload
            .nodes
            .iter()
            .filter_map(|node| node.local_reference.as_ref())
            .any(|reference| reference.status == LocalReferenceStatus::RemoteDisabled)
    );
}

#[test]
fn rmd_and_qmd_graph_segment_schema_cli_and_render_loss_contracts_hold() {
    let rmd = parse_r_markdown(
        "---\ntitle: R\n---\n\n```{r setup, echo=FALSE}\nwriteLines('never')\n```\n\n[@smith]\n\n::: {.cell-output}\nstored\n:::\n",
        SourceInfo::stdin("analysis.Rmd"),
    );
    assert_eq!(
        rmd.payload.as_ref().unwrap().dialect,
        MarkdownDialect::RMarkdown
    );
    let payload = rmd.payload.as_ref().unwrap();
    let graph = payload
        .to_document_graph(
            DocumentGraphContext::new("rmd:qmd-contract").with_source(rmd.source.clone()),
        )
        .unwrap();
    assert!(graph.nodes.iter().any(|node| node.kind == DocumentNodeKind::CodeBlock && node.attrs["executed"] == false));
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Citation)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::RawBlock)
    );
    assert!(graph.edges.iter().any(
        |edge| edge.relation == DocumentRelation::References && edge.target == "citation:smith"
    ));

    let source_identity = rmd.identity.as_ref().unwrap();
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(!segments.segments.is_empty());
    assert!(
        segments
            .segments
            .iter()
            .all(|segment| !segment.node_ids.is_empty())
    );

    assert!(
        render_document_graph(&graph, RenderFormat::Markdown, &RenderOptions::default()).is_err()
    );
    let raw = render_document_graph(
        &graph,
        RenderFormat::Markdown,
        &RenderOptions::new(FidelityMode::RawFallback),
    )
    .unwrap();
    raw.validate_source_map().unwrap();
    assert!(raw.content.contains("stored"));
    let lossy = render_document_graph(
        &graph,
        RenderFormat::Markdown,
        &RenderOptions::new(FidelityMode::Lossy),
    )
    .unwrap();
    assert!(!lossy.fidelity.losses.is_empty());

    for (name, value) in [
        ("r_markdown", serde_json::to_value(payload).unwrap()),
        ("r_markdown-envelope", serde_json::to_value(&rmd).unwrap()),
        (
            "r_markdown-options",
            serde_json::to_value(MarkdownOptions::default()).unwrap(),
        ),
    ] {
        let validation = grist::schema::validate_schema(name, &value).unwrap();
        assert!(validation.valid, "{name}: {:?}", validation.issues);
    }

    let registry = grist::registry::builtin_parser_registry().unwrap();
    for selector in ["r_markdown", "rmd", "quarto", "qmd"] {
        assert!(
            matches!(
                registry.select_format(selector),
                grist::registry::ParserSelection::Available(_)
            ),
            "{selector}"
        );
    }

    #[cfg(feature = "cli")]
    {
        let parsed = grist::cli::parse_bytes(
            "quarto",
            b"```{python}\nraise RuntimeError('never')\n```\n".to_vec(),
            SourceInfo::stdin("cli.qmd"),
            RequestId::new("rmarkdown-quarto-cli").unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(parsed.kind, grist::core::ArtifactKind::Markdown);
        assert_eq!(parsed.payload.unwrap()["dialect"], "quarto");
    }
}

#[test]
fn commonmark_v2_wire_shape_remains_backward_compatible() {
    let report = grist::markdown::parse_markdown("# Stable\n", SourceInfo::stdin("stable.md"));
    let payload = report.payload.unwrap();
    let value = serde_json::to_value(&payload).unwrap();
    assert!(value.get("dialect").is_none());
    let decoded: grist::markdown::MarkdownDocument = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.dialect, MarkdownDialect::CommonMark);

    let options = serde_json::to_value(MarkdownOptions::default()).unwrap();
    for new_default in [
        "dialect",
        "project_root",
        "resolve_local_references",
        "max_reference_bytes",
        "max_total_reference_bytes",
    ] {
        assert!(options.get(new_default).is_none(), "{new_default}");
    }
}

#[test]
fn executable_metadata_is_quote_aware_and_malformed_yaml_is_partial() {
    let source = "```{r plot, fig.cap=\"A, B\", opts=c(1, 2)}\n1\n```\n\n```{python}\n#| fig-cap: [unterminated\n1\n```\n";
    let report = parse_quarto_with_options(
        source,
        SourceInfo::stdin("metadata.qmd"),
        &MarkdownOptions::default(),
    );
    assert_eq!(report.status, OperationStatus::Partial);
    let payload = report.payload.unwrap();
    let first = payload
        .nodes
        .iter()
        .filter_map(|node| node.executable.as_ref())
        .find(|metadata| metadata.engine == "r")
        .unwrap();
    assert_eq!(first.options["fig-cap"], "A, B");
    assert_eq!(first.options["opts"], "c(1, 2)");
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "executable.metadata"
            && diagnostic.message.contains("malformed executable option")
    }));
}

#[test]
fn repeated_references_use_virtual_source_parent_and_are_aggregate_bounded() {
    let root = fixture_root("aggregate-budget");
    let source_path = root.join("virtual").join("main.qmd");
    fs::create_dir_all(source_path.parent().unwrap()).unwrap();
    fs::write(root.join("virtual").join("asset.bin"), b"four").unwrap();
    let source = "![one](asset.bin)\n\n![two](asset.bin)\n";
    let report = parse_quarto_with_options(
        source,
        SourceInfo::from_path(&source_path),
        &MarkdownOptions {
            project_root: Some(root.clone()),
            max_reference_bytes: 8,
            max_total_reference_bytes: 4,
            ..MarkdownOptions::default()
        },
    );
    assert_eq!(report.status, OperationStatus::Partial);
    let payload = report.payload.unwrap();
    let references = payload
        .nodes
        .iter()
        .filter_map(|node| node.local_reference.as_ref())
        .collect::<Vec<_>>();
    assert!(references.iter().any(|reference| {
        reference.status == LocalReferenceStatus::Resolved
            && reference.resolved_path.as_deref() == Some("virtual/asset.bin")
    }));
    assert!(
        references
            .iter()
            .any(|reference| reference.status == LocalReferenceStatus::BudgetExceeded)
    );
}

#[test]
fn executable_only_graph_requires_fidelity_handling_and_preserves_raw_header() {
    let report = parse_r_markdown(
        "```{r setup, echo=FALSE}\nstop('never')\n```\n",
        SourceInfo::stdin("render.Rmd"),
    );
    let graph = report
        .payload
        .unwrap()
        .to_document_graph(DocumentGraphContext::new("render-executable"))
        .unwrap();
    assert_eq!(graph.dialect.as_deref(), Some("r_markdown"));
    assert!(
        render_document_graph(&graph, RenderFormat::Markdown, &RenderOptions::default()).is_err()
    );
    let raw = render_document_graph(
        &graph,
        RenderFormat::Markdown,
        &RenderOptions::new(FidelityMode::RawFallback),
    )
    .unwrap();
    assert!(raw.content.contains("{r setup, echo=FALSE}"));
    let lossy = render_document_graph(
        &graph,
        RenderFormat::Markdown,
        &RenderOptions::new(FidelityMode::Lossy),
    )
    .unwrap();
    assert!(!lossy.fidelity.losses.is_empty());
}

#[test]
fn citation_scan_excludes_frontmatter_links_and_stored_outputs() {
    let source = "---\nauthor: '@frontmatter'\n---\n\n[link](https://example.test/@path)\n\n::: {.cell-output}\n@stored\n:::\n\nProse @real.\n";
    let report = parse_quarto_with_options(
        source,
        SourceInfo::stdin("citations.qmd"),
        &MarkdownOptions::default(),
    );
    let keys = report
        .payload
        .unwrap()
        .nodes
        .iter()
        .filter_map(|node| node.citation.as_ref())
        .flat_map(|citation| citation.keys.iter().cloned())
        .collect::<Vec<_>>();
    assert_eq!(keys, ["real"]);
}
