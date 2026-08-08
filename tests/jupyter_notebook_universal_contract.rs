#![cfg(feature = "notebooks")]

use grist::core::{
    BudgetProfile, BudgetSelection, ContentIdentity, IndexPosition, Input, LocationComponent,
    OperationControl, OperationStatus, ParseRequest, ProviderSet, RequestId, ResourceBudget,
    SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::notebook::{NotebookOptions, parse_notebook, parse_notebook_with_operation_control};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::segment::{SegmentOptions, segment_document_graph};
use serde_json::json;

fn v4_notebook(sentinel: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {
            "kernelspec": {"name": "python3"},
            "custom": {"retained": [1, 2, 3]},
            "widgets": {
                "application/vnd.jupyter.widget-state+json": {
                    "state": {"model-1": {"model_name": "IntSliderModel"}}
                }
            }
        },
        "cells": [
            {
                "id": "markdown-id",
                "cell_type": "markdown",
                "metadata": {"tags": ["intro"]},
                "source": ["# Heading\n", "binary attachment"],
                "attachments": {
                    "pixel.png": {
                        "image/png": "AAECA/8=",
                        "text/plain": ["fallback", " text"]
                    }
                }
            },
            {
                "id": "code-id",
                "cell_type": "code",
                "metadata": {"trusted": false},
                "execution_count": 7,
                "source": [
                    "from pathlib import Path\n",
                    format!("Path(r'{}').write_text('EXECUTED')", sentinel.replace('\\', "\\\\"))
                ],
                "outputs": [
                    {
                        "output_type": "stream",
                        "name": "stdout",
                        "text": ["hello", "\n"]
                    },
                    {
                        "output_type": "execute_result",
                        "execution_count": 7,
                        "metadata": {"isolated": true},
                        "data": {
                            "text/plain": ["<Figure size 1x1>"],
                            "text/html": "<script>globalThis.EXECUTED=true</script><b>plot</b>",
                            "image/png": "iVBORw0KGgo=",
                            "application/json": {"series": [1, 2, 3]}
                        }
                    },
                    {
                        "output_type": "error",
                        "ename": "ValueError",
                        "evalue": "boom",
                        "traceback": ["Traceback line", "ValueError: boom"]
                    },
                    {
                        "output_type": "display_data",
                        "metadata": {},
                        "transient": {"display_id": "display-1"},
                        "data": {
                            "application/vnd.jupyter.widget-view+json": {
                                "model_id": "model-1",
                                "version_major": 2
                            }
                        }
                    }
                ]
            },
            {
                "cell_type": "raw",
                "metadata": {"format": "text/plain"},
                "source": "raw cell"
            }
        ],
        "custom_top_level": {"retained": true}
    }))
    .unwrap()
}

#[test]
fn v4_preserves_cells_attachments_outputs_widgets_relations_and_never_executes() {
    let unique = format!(
        "grist-notebook-sentinel-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let sentinel = std::env::temp_dir().join(unique);
    let bytes = v4_notebook(sentinel.to_str().unwrap());
    let source =
        SourceInfo::new("complete.ipynb").with_declared_mime_type("application/x-ipynb+json");

    let first = parse_notebook(&bytes, source.clone(), &NotebookOptions::default());
    let second = parse_notebook(&bytes, source, &NotebookOptions::default());
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:?}",
        first.diagnostics
    );
    assert!(!sentinel.exists(), "parsing executed notebook source");
    let notebook = first.payload().unwrap();
    let again = second.payload().unwrap();
    assert_eq!(notebook.nbformat, 4);
    assert_eq!(notebook.cells.len(), 3);
    assert_eq!(
        notebook
            .cells
            .iter()
            .map(|cell| cell.id.as_deref())
            .collect::<Vec<_>>(),
        [Some("markdown-id"), Some("code-id"), None]
    );
    assert_eq!(notebook.cells[0].source, "# Heading\nbinary attachment");
    assert_eq!(notebook.cells[0].metadata["tags"][0], "intro");
    assert_eq!(
        notebook.cells[0].attachments[0].data["image/png"],
        "AAECA/8="
    );
    assert_eq!(notebook.cells[1].execution_count, Some(json!(7)));
    assert_eq!(notebook.cells[1].outputs.len(), 4);
    assert_eq!(
        notebook.cells[1].outputs[0].text.as_deref(),
        Some("hello\n")
    );
    assert_eq!(
        notebook.cells[1].outputs[1].data["application/json"]["series"],
        json!([1, 2, 3])
    );
    assert_eq!(
        notebook.cells[1].outputs[2].error_name.as_deref(),
        Some("ValueError")
    );
    assert_eq!(notebook.cells[1].outputs[2].traceback.len(), 2);
    assert_eq!(
        notebook.cells[1].outputs[3].widget_view.as_ref().unwrap()["model_id"],
        "model-1"
    );
    assert!(notebook.widgets.is_some());
    assert_eq!(notebook.extra["custom_top_level"]["retained"], true);
    assert_eq!(
        notebook
            .cells
            .iter()
            .map(|cell| &cell.stable_id)
            .collect::<Vec<_>>(),
        again
            .cells
            .iter()
            .map(|cell| &cell.stable_id)
            .collect::<Vec<_>>()
    );
    assert!(matches!(
        notebook.cells[1].outputs[2].locator.components()[0],
        LocationComponent::NotebookCell {
            index: IndexPosition { value: 2, .. },
            output_index: Some(IndexPosition { value: 3, .. }),
            ..
        }
    ));

    let graph = notebook
        .to_document_graph(DocumentGraphContext::new("graph:notebook"))
        .unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::NotebookCell)
            .count(),
        3
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::CellOutput)
            .count(),
        4
    );
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::CellOutput)
            .all(|node| node.attrs.get("stored_result") == Some(&json!(true))
                && node.attrs.get("executed_by_grist") == Some(&json!(false)))
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::DerivedFrom)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::AttachmentOf)
    );

    let payload_identity = first.identity.as_ref().unwrap();
    let graph_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        payload_identity,
        &graph_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| segment.locators.iter().any(|locator| matches!(
                locator.components()[0],
                LocationComponent::NotebookCell { .. }
            )))
    );
}

#[test]
fn v3_worksheets_inputs_legacy_mime_and_errors_are_normalized_without_loss() {
    let bytes = serde_json::to_vec(&json!({
        "nbformat": 3,
        "nbformat_minor": 0,
        "metadata": {"name": "legacy"},
        "worksheets": [{
            "metadata": {"title": "Sheet 1"},
            "cells": [
                {
                    "cell_type": "heading",
                    "level": 2,
                    "source": ["Legacy", " heading"]
                },
                {
                    "cell_type": "code",
                    "input": ["print('v3')"],
                    "prompt_number": 9,
                    "metadata": {"collapsed": false},
                    "outputs": [
                        {
                            "output_type": "pyout",
                            "prompt_number": 9,
                            "text": ["result"],
                            "png": "AAECAw==",
                            "html": "<b>result</b>"
                        },
                        {
                            "output_type": "pyerr",
                            "ename": "RuntimeError",
                            "evalue": "legacy boom",
                            "traceback": ["RuntimeError: legacy boom"]
                        }
                    ]
                }
            ]
        }]
    }))
    .unwrap();
    let envelope = parse_notebook(
        &bytes,
        SourceInfo::new("legacy.ipynb"),
        &NotebookOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let notebook = envelope.payload().unwrap();
    assert_eq!(notebook.worksheets.len(), 1);
    assert_eq!(notebook.worksheets[0].cell_stable_ids.len(), 2);
    assert_eq!(notebook.cells[0].source, "Legacy heading");
    assert_eq!(notebook.cells[0].extra["level"], 2);
    let code = &notebook.cells[1];
    assert_eq!(code.worksheet_index, Some(0));
    assert_eq!(code.execution_count, Some(json!(9)));
    assert_eq!(code.outputs[0].output_type, "pyout");
    assert_eq!(code.outputs[0].normalized_output_type, "execute_result");
    assert_eq!(code.outputs[0].data["image/png"], "AAECAw==");
    assert_eq!(code.outputs[0].data["text/html"], "<b>result</b>");
    assert_eq!(code.outputs[1].normalized_output_type, "error");
    assert_eq!(code.outputs[1].cell_stable_id, code.stable_id);
    assert!(matches!(
        code.locator.components().get(1),
        Some(LocationComponent::JsonPointer { pointer })
            if pointer == "/worksheets/0/cells/1"
    ));
}

#[test]
fn malformed_deep_limited_and_large_outputs_have_explicit_safe_outcomes() {
    let malformed = parse_notebook(
        br#"{"nbformat":4,"cells":["#,
        SourceInfo::new("broken.ipynb"),
        &NotebookOptions::default(),
    );
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert_eq!(malformed.diagnostics[0].code, "ipynb.json.malformed");

    let nested = "[".repeat(20) + &"]".repeat(20);
    let deep = format!(r#"{{"nbformat":4,"nbformat_minor":5,"metadata":{nested},"cells":[]}}"#);
    let deep_result = parse_notebook(
        deep.as_bytes(),
        SourceInfo::new("deep.ipynb"),
        &NotebookOptions {
            max_json_depth: 8,
            ..NotebookOptions::default()
        },
    );
    assert_eq!(deep_result.status, OperationStatus::Failed);
    assert_eq!(deep_result.diagnostics[0].code, "ipynb.limit.json_depth");

    let limited_bytes = serde_json::to_vec(&json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {},
        "cells": [{
            "id": "limited",
            "cell_type": "code",
            "metadata": {},
            "source": "",
            "execution_count": null,
            "outputs": [
                {"output_type": "stream", "name": "stdout", "text": "one"},
                {"output_type": "stream", "name": "stdout", "text": "two"}
            ]
        }]
    }))
    .unwrap();
    let limited = parse_notebook(
        &limited_bytes,
        SourceInfo::new("limited.ipynb"),
        &NotebookOptions {
            max_outputs_per_cell: 1,
            ..NotebookOptions::default()
        },
    );
    assert_eq!(limited.status, OperationStatus::Partial);
    assert_eq!(limited.payload().unwrap().cells[0].outputs.len(), 1);
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "ipynb.limit.outputs")
    );

    let large_text = "x".repeat(512 * 1024);
    let large_bytes = serde_json::to_vec(&json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {},
        "cells": [{
            "cell_type": "code",
            "metadata": {},
            "source": "",
            "outputs": [{"output_type": "stream", "name": "stdout", "text": large_text}]
        }]
    }))
    .unwrap();
    let large = parse_notebook(
        &large_bytes,
        SourceInfo::new("large.ipynb"),
        &NotebookOptions::default(),
    );
    assert_eq!(large.status, OperationStatus::Complete);
    assert_eq!(
        large.payload().unwrap().cells[0].outputs[0]
            .text
            .as_ref()
            .unwrap()
            .len(),
        512 * 1024
    );
}

#[test]
fn malformed_cell_shapes_are_partial_and_neighboring_cells_survive() {
    let bytes = serde_json::to_vec(&json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {},
        "cells": [
            {
                "id": 7,
                "cell_type": null,
                "source": {"not": "text"},
                "attachments": [],
                "outputs": {}
            },
            {
                "id": "good",
                "cell_type": "code",
                "source": "1 + 1",
                "outputs": [{"output_type": null, "data": []}]
            }
        ]
    }))
    .unwrap();
    let envelope = parse_notebook(
        &bytes,
        SourceInfo::new("shape-errors.ipynb"),
        &NotebookOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert_eq!(envelope.payload().unwrap().cells.len(), 2);
    for code in [
        "ipynb.cell.id_invalid",
        "ipynb.cell.cell_type_invalid",
        "ipynb.cell.source_invalid",
        "ipynb.attachments.not_object",
        "ipynb.outputs.not_array",
        "ipynb.output.output_type_invalid",
        "ipynb.mime.not_object",
    ] {
        assert!(
            envelope
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "missing diagnostic {code}"
        );
    }
}

#[test]
fn native_cell_identity_survives_reordering_and_node_budget_is_preflighted() {
    let notebook = |cells| {
        serde_json::to_vec(&json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {},
            "cells": cells
        }))
        .unwrap()
    };
    let a = json!({"id":"a","cell_type":"markdown","source":"A"});
    let b = json!({"id":"b","cell_type":"markdown","source":"B"});
    let first = parse_notebook(
        &notebook(vec![a.clone(), b.clone()]),
        SourceInfo::new("first.ipynb"),
        &NotebookOptions::default(),
    );
    let second = parse_notebook(
        &notebook(vec![b, a]),
        SourceInfo::new("second.ipynb"),
        &NotebookOptions::default(),
    );
    let stable = |envelope: &grist::notebook::NotebookEnvelope, id: &str| {
        envelope
            .payload()
            .unwrap()
            .cells
            .iter()
            .find(|cell| cell.id.as_deref() == Some(id))
            .unwrap()
            .stable_id
            .clone()
    };
    assert_eq!(stable(&first, "a"), stable(&second, "a"));
    assert_eq!(stable(&first, "b"), stable(&second, "b"));

    let bytes = notebook(vec![json!({
        "id": "budget",
        "cell_type": "code",
        "source": "pass",
        "outputs": [{"output_type":"stream","text":"x"}]
    })]);
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nodes = Some(2);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let exhausted = parse_notebook_with_operation_control(
        &bytes,
        SourceInfo::new("budget.ipynb"),
        &NotebookOptions::default(),
        &control,
    );
    assert_eq!(exhausted.status, OperationStatus::Failed);
    assert_eq!(
        exhausted.diagnostics[0].code,
        "grist.budget.nodes.exhausted"
    );
}

#[test]
fn structural_detection_registry_dispatch_and_cli_descriptor_are_available() {
    let bytes = serde_json::to_vec(&json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {},
        "cells": []
    }))
    .unwrap();
    let registry = builtin_parser_registry().unwrap();
    let detection = detect_with_registry(
        std::path::Path::new("extensionless"),
        &bytes,
        None,
        None,
        &grist::core::Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::JupyterNotebook);
    assert_eq!(detection.selected_parser.as_deref(), Some("grist.ipynb"));
    assert!(matches!(
        registry.select_extension(".ipynb"),
        ParserSelection::Available(descriptor)
            if descriptor.id == "grist.ipynb"
                && descriptor.payload_schema.version == "grist/ipynb/v1"
                && descriptor.required_features.contains("notebooks")
    ));

    let request = ParseRequest::new(
        RequestId::new("ipynb-registry-test").unwrap(),
        Input::bytes(bytes),
        SourceInfo::new("dispatch.ipynb"),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );
    let dispatched = registry.dispatch("jupyter", request, None).unwrap();
    assert_eq!(dispatched.status, OperationStatus::Complete);
    assert_eq!(dispatched.payload.as_ref().unwrap()["nbformat"], 4);
}

#[cfg(feature = "schemas")]
#[test]
fn notebook_schemas_are_registered() {
    for name in ["ipynb", "ipynb-envelope", "ipynb-options"] {
        let schema =
            grist::schema::schema_json(name).unwrap_or_else(|| panic!("missing schema {name}"));
        assert!(schema.is_object());
    }
}

#[cfg(feature = "cli")]
#[test]
fn cli_parse_and_graph_surfaces_use_the_notebook_registry_parser() {
    let bytes = serde_json::to_vec(&json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {},
        "cells": [{
            "id": "cli-cell",
            "cell_type": "markdown",
            "metadata": {},
            "source": "CLI notebook"
        }]
    }))
    .unwrap();
    let envelope = grist::cli::parse_bytes(
        "ipynb",
        bytes,
        SourceInfo::new("cli.ipynb"),
        RequestId::new("ipynb-cli-test").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    let graph = grist::cli::project_envelope_to_graph(&envelope, "graph:cli-ipynb").unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::NotebookCell)
    );
}
