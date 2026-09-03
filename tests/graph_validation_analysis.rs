#![cfg(feature = "graph")]

use grist::core::{
    BudgetSelection, CancellationToken, LocationComponent, OperationControl, OperationControlError,
    ResourceBudget, SourceLocator,
};
use grist::graph::{
    GraphAnalysisError, GraphAnalysisOptions, GraphDocument, GraphEdge, GraphNode, GraphSourceMap,
    GraphValidationOptions, analyze_graph, analyze_graph_with_operation_control, diagnostic_codes,
    validate_graph, validate_graph_with_operation_control,
};
use serde_json::json;

fn pointer(value: &str) -> SourceLocator {
    SourceLocator::exact(LocationComponent::JsonPointer {
        pointer: value.into(),
    })
    .unwrap()
}

fn source_map(nodes: usize, edges: usize) -> GraphSourceMap {
    GraphSourceMap {
        graph: Some(pointer("")),
        nodes: (0..nodes)
            .map(|position| pointer(&format!("/nodes/{position}")))
            .collect(),
        edges: (0..edges)
            .map(|position| pointer(&format!("/edges/{position}")))
            .collect(),
    }
}

fn graph(node_ids: &[&str], edges: &[(&str, &str, &str, bool)]) -> GraphDocument {
    let mut document = GraphDocument::new(true);
    document.nodes = node_ids.iter().map(|id| GraphNode::new(*id)).collect();
    document.edges = edges
        .iter()
        .map(|(id, source, target, directed)| GraphEdge::new(*id, *source, *target, *directed))
        .collect();
    document
}

#[test]
fn validation_builds_occurrence_indexes_for_directed_and_undirected_edges() {
    let document = graph(
        &["a", "b", "c"],
        &[("ab", "a", "b", true), ("bc", "b", "c", false)],
    );
    let result = validate_graph(
        &document,
        &source_map(3, 2),
        &GraphValidationOptions::default(),
    )
    .unwrap();
    assert!(result.valid);
    assert_eq!(result.indexes.outgoing["a"], vec![0]);
    assert_eq!(result.indexes.incoming["b"], vec![0, 1]);
    assert_eq!(result.indexes.outgoing["c"], vec![1]);
    assert_eq!(result.indexes.incoming["c"], vec![1]);
}

#[test]
fn configurable_validation_reports_deterministic_locatable_findings() {
    let mut document = graph(
        &["a", "a"],
        &[
            ("loop", "a", "a", true),
            ("parallel", "a", "a", true),
            ("missing", "a", "z", false),
        ],
    );
    document.nodes[0]
        .attrs
        .insert("nested".into(), json!({"one": {"two": true}}));
    let options = GraphValidationOptions {
        allow_self_loops: false,
        allow_parallel_edges: false,
        allow_undirected_edges: false,
        require_dag: false,
        max_attribute_depth: Some(2),
    };
    let result = validate_graph(&document, &source_map(2, 3), &options).unwrap();
    let codes = result
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        vec![
            diagnostic_codes::ATTRIBUTE_DEPTH_EXCEEDED,
            diagnostic_codes::NODE_ID_DUPLICATE,
            diagnostic_codes::SELF_LOOP_FORBIDDEN,
            diagnostic_codes::SELF_LOOP_FORBIDDEN,
            diagnostic_codes::PARALLEL_EDGE_FORBIDDEN,
            diagnostic_codes::EDGE_ENDPOINT_UNKNOWN,
            diagnostic_codes::UNDIRECTED_EDGE_FORBIDDEN,
        ]
    );
    assert!(result.diagnostics.iter().all(|item| item.locator.is_some()));
}

#[test]
fn cycles_are_valid_core_graphs_but_can_be_required_to_be_dags() {
    let document = graph(
        &["a", "b", "c"],
        &[
            ("ab", "a", "b", true),
            ("bc", "b", "c", true),
            ("ca", "c", "a", true),
        ],
    );
    let locations = source_map(3, 3);
    assert!(
        validate_graph(&document, &locations, &GraphValidationOptions::default())
            .unwrap()
            .valid
    );
    let result = validate_graph(
        &document,
        &locations,
        &GraphValidationOptions {
            require_dag: true,
            ..GraphValidationOptions::default()
        },
    )
    .unwrap();
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, diagnostic_codes::CYCLE);
    assert!(result.diagnostics[0].locator.is_some());
}

#[test]
fn analysis_is_insertion_order_independent_and_stably_scheduled() {
    let first = graph(
        &["d", "b", "a", "c"],
        &[
            ("ac", "a", "c", true),
            ("bd", "b", "d", true),
            ("cd", "c", "d", true),
        ],
    );
    let second = graph(
        &["c", "a", "d", "b"],
        &[
            ("cd", "c", "d", true),
            ("ac", "a", "c", true),
            ("bd", "b", "d", true),
        ],
    );
    let expected = analyze_graph(
        &first,
        &GraphSourceMap::default(),
        &GraphAnalysisOptions {
            require_directed: true,
        },
    )
    .unwrap();
    let actual = analyze_graph(
        &second,
        &GraphSourceMap::default(),
        &GraphAnalysisOptions {
            require_directed: true,
        },
    )
    .unwrap();
    assert_eq!(expected, actual);
    assert_eq!(actual.topological_order.unwrap(), vec!["a", "b", "c", "d"]);
    assert_eq!(
        actual.execution_layers.unwrap(),
        vec![vec!["a", "b"], vec!["c"], vec!["d"]]
    );
}

#[test]
fn iterative_sccs_return_actionable_cycle_witnesses() {
    let document = graph(
        &["a", "b", "c", "z"],
        &[
            ("ab", "a", "b", true),
            ("bc", "b", "c", true),
            ("ca", "c", "a", true),
            ("zz", "z", "z", true),
        ],
    );
    let result = analyze_graph(
        &document,
        &source_map(4, 4),
        &GraphAnalysisOptions {
            require_directed: true,
        },
    )
    .unwrap();
    assert_eq!(
        result.strongly_connected_components,
        vec![vec!["a", "b", "c"], vec!["z"]]
    );
    assert_eq!(result.cycle_witnesses[0].node_ids, vec!["a", "b", "c", "a"]);
    assert_eq!(result.cycle_witnesses[1].node_ids, vec!["z", "z"]);
    assert!(
        result.cycle_witnesses[0]
            .locators
            .iter()
            .all(Option::is_some)
    );
    assert!(matches!(
        result.require_dag(),
        Err(GraphAnalysisError::Cyclic { .. })
    ));
}

#[test]
fn deep_graph_analysis_is_iterative() {
    let count = 4_000usize;
    let node_ids = (0..count)
        .map(|index| format!("n{index:04}"))
        .collect::<Vec<_>>();
    let mut document = GraphDocument::new(true);
    document.nodes = node_ids.iter().map(GraphNode::new).collect();
    document.edges = (1..count)
        .map(|index| {
            GraphEdge::new(
                format!("e{index:04}"),
                node_ids[index - 1].clone(),
                node_ids[index].clone(),
                true,
            )
        })
        .collect();
    let result = analyze_graph(
        &document,
        &GraphSourceMap::default(),
        &GraphAnalysisOptions {
            require_directed: true,
        },
    )
    .unwrap();
    assert_eq!(result.topological_order.unwrap().len(), count);
}

#[test]
fn analysis_honors_cancellation_and_budgets() {
    let document = graph(&["a", "b"], &[("ab", "a", "b", true)]);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = OperationControl::new(
        &BudgetSelection::custom(ResourceBudget::trusted_unbounded()),
        cancellation,
    )
    .unwrap();
    assert!(matches!(
        analyze_graph_with_operation_control(
            &document,
            &GraphSourceMap::default(),
            &GraphAnalysisOptions::default(),
            &cancelled
        ),
        Err(GraphAnalysisError::Control(
            OperationControlError::Cancelled(_)
        ))
    ));

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nodes = Some(1);
    let limited =
        OperationControl::new(&BudgetSelection::custom(budget), CancellationToken::new()).unwrap();
    assert!(matches!(
        validate_graph_with_operation_control(
            &document,
            &GraphSourceMap::default(),
            &GraphValidationOptions::default(),
            &limited
        ),
        Err(OperationControlError::BudgetExceeded(_))
    ));
}

#[test]
fn directed_analysis_returns_typed_incompatibility_errors() {
    let undirected = graph(&["a", "b"], &[("ab", "a", "b", false)]);
    assert!(matches!(
        analyze_graph(
            &undirected,
            &source_map(2, 1),
            &GraphAnalysisOptions {
                require_directed: true
            }
        ),
        Err(GraphAnalysisError::UndirectedEdge { edge_id, locator: Some(_) })
            if edge_id == "ab"
    ));

    let invalid = graph(&["a"], &[("ab", "a", "b", true)]);
    assert!(matches!(
        analyze_graph(
            &invalid,
            &source_map(1, 1),
            &GraphAnalysisOptions::default()
        ),
        Err(GraphAnalysisError::UnknownEndpoint { node_id, .. }) if node_id == "b"
    ));
}
