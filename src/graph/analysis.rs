use super::{GraphDocument, GraphSourceMap, diagnostic_codes};
use crate::core::{
    BudgetProfile, BudgetSelection, CancellationToken, Diagnostic, OperationControl,
    OperationControlError, SourceLocator,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

const PARSER: &str = "grist.graph.analysis";

/// Configurable semantic policies applied after syntax parsing.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GraphValidationOptions {
    pub allow_self_loops: bool,
    pub allow_parallel_edges: bool,
    pub allow_undirected_edges: bool,
    pub require_dag: bool,
    pub max_attribute_depth: Option<u64>,
}

impl Default for GraphValidationOptions {
    fn default() -> Self {
        Self {
            allow_self_loops: true,
            allow_parallel_edges: true,
            allow_undirected_edges: true,
            require_dag: false,
            max_attribute_depth: None,
        }
    }
}

/// Deterministic occurrence indexes for graph traversal.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GraphIndexes {
    pub node_positions: BTreeMap<String, usize>,
    pub edge_positions: BTreeMap<String, usize>,
    pub incoming: BTreeMap<String, Vec<usize>>,
    pub outgoing: BTreeMap<String, Vec<usize>>,
}

/// Semantic validation output. Invalid user input is represented by diagnostics,
/// while cancellation and resource exhaustion are returned as operation errors.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphValidationResult {
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub indexes: GraphIndexes,
}

/// Resource policy used by deterministic graph analysis.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GraphAnalysisOptions {
    /// Reject mixed/undirected input when producing a directed schedule.
    pub require_directed: bool,
}

/// A stable, locatable closed walk proving a cycle.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphCycleWitness {
    pub node_ids: Vec<String>,
    pub locators: Vec<Option<SourceLocator>>,
}

/// Deterministic SCC and DAG-derived analysis.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GraphAnalysis {
    pub strongly_connected_components: Vec<Vec<String>>,
    pub cycle_witnesses: Vec<GraphCycleWitness>,
    pub topological_order: Option<Vec<String>>,
    pub execution_layers: Option<Vec<Vec<String>>>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum GraphAnalysisError {
    #[error(transparent)]
    Control(#[from] OperationControlError),
    #[error("graph contains an undirected edge: {edge_id}")]
    UndirectedEdge {
        edge_id: String,
        locator: Option<SourceLocator>,
    },
    #[error("duplicate node id: {node_id}")]
    DuplicateNodeId {
        node_id: String,
        locator: Option<SourceLocator>,
    },
    #[error("edge {edge_id} references unknown endpoint {node_id}")]
    UnknownEndpoint {
        edge_id: String,
        node_id: String,
        locator: Option<SourceLocator>,
    },
    #[error("graph is cyclic")]
    Cyclic { witnesses: Vec<GraphCycleWitness> },
}

pub fn validate_graph(
    document: &GraphDocument,
    source_map: &GraphSourceMap,
    options: &GraphValidationOptions,
) -> Result<GraphValidationResult, OperationControlError> {
    let control = trusted_control();
    validate_graph_with_operation_control(document, source_map, options, &control)
}

pub fn validate_graph_with_operation_control(
    document: &GraphDocument,
    source_map: &GraphSourceMap,
    options: &GraphValidationOptions,
    control: &OperationControl,
) -> Result<GraphValidationResult, OperationControlError> {
    control.checkpoint()?;
    control
        .budget()
        .consume_nodes(to_u64(document.nodes.len().saturating_add(1)))
        .map_err(OperationControlError::from)?;
    control
        .budget()
        .consume_records(to_u64(document.edges.len()))
        .map_err(OperationControlError::from)?;
    control
        .budget()
        .observe_memory_bytes(estimated_bytes(document))
        .map_err(OperationControlError::from)?;

    let mut diagnostics = Vec::new();
    let mut indexes = GraphIndexes::default();
    for node in &document.nodes {
        indexes.incoming.entry(node.id.clone()).or_default();
        indexes.outgoing.entry(node.id.clone()).or_default();
    }

    if source_map.nodes.len() != document.nodes.len()
        || source_map.edges.len() != document.edges.len()
    {
        diagnostics.push(Diagnostic::error(
            PARSER,
            diagnostic_codes::SOURCE_MAP_MISMATCH,
            format!(
                "source map has {} node and {} edge locators for {} nodes and {} edges",
                source_map.nodes.len(),
                source_map.edges.len(),
                document.nodes.len(),
                document.edges.len()
            ),
        ));
    }

    check_attribute_depth(
        document.attrs.values(),
        options.max_attribute_depth,
        source_map.graph.clone(),
        &mut diagnostics,
        control,
    )?;

    for (position, node) in document.nodes.iter().enumerate() {
        control.checkpoint()?;
        let locator = source_map.nodes.get(position).cloned();
        if node.id.is_empty() {
            diagnostics.push(located(
                Diagnostic::error(PARSER, diagnostic_codes::ID_EMPTY, "node id is empty"),
                locator.clone(),
            ));
        }
        if indexes
            .node_positions
            .insert(node.id.clone(), position)
            .is_some()
        {
            diagnostics.push(located(
                Diagnostic::error(
                    PARSER,
                    diagnostic_codes::NODE_ID_DUPLICATE,
                    format!("duplicate node id `{}`", node.id),
                )
                .with_affected_ids(vec![node.id.clone()]),
                locator.clone(),
            ));
        }
        check_attribute_depth(
            node.attrs.values(),
            options.max_attribute_depth,
            locator,
            &mut diagnostics,
            control,
        )?;
    }

    let mut parallel = BTreeSet::new();
    for (position, edge) in document.edges.iter().enumerate() {
        control.checkpoint()?;
        let locator = source_map.edges.get(position).cloned();
        if edge.id.is_empty() {
            diagnostics.push(located(
                Diagnostic::error(PARSER, diagnostic_codes::ID_EMPTY, "edge id is empty"),
                locator.clone(),
            ));
        }
        if indexes
            .edge_positions
            .insert(edge.id.clone(), position)
            .is_some()
        {
            diagnostics.push(located(
                Diagnostic::error(
                    PARSER,
                    diagnostic_codes::EDGE_ID_DUPLICATE,
                    format!("duplicate edge id `{}`", edge.id),
                )
                .with_affected_ids(vec![edge.id.clone()]),
                locator.clone(),
            ));
        }
        for endpoint in [&edge.source, &edge.target] {
            if !indexes.node_positions.contains_key(endpoint) {
                diagnostics.push(located(
                    Diagnostic::error(
                        PARSER,
                        diagnostic_codes::EDGE_ENDPOINT_UNKNOWN,
                        format!(
                            "edge `{}` references unknown endpoint `{endpoint}`",
                            edge.id
                        ),
                    )
                    .with_affected_ids(vec![edge.id.clone(), endpoint.clone()]),
                    locator.clone(),
                ));
            }
        }
        if !options.allow_self_loops && edge.source == edge.target {
            diagnostics.push(located(
                Diagnostic::error(
                    PARSER,
                    diagnostic_codes::SELF_LOOP_FORBIDDEN,
                    format!("self-loop edge `{}` is forbidden", edge.id),
                ),
                locator.clone(),
            ));
        }
        if !options.allow_undirected_edges && !edge.directed {
            diagnostics.push(located(
                Diagnostic::error(
                    PARSER,
                    diagnostic_codes::UNDIRECTED_EDGE_FORBIDDEN,
                    format!("undirected edge `{}` is forbidden", edge.id),
                ),
                locator.clone(),
            ));
        }
        let pair = if edge.directed || edge.source <= edge.target {
            (edge.source.clone(), edge.target.clone(), edge.directed)
        } else {
            (edge.target.clone(), edge.source.clone(), edge.directed)
        };
        if !parallel.insert(pair) && !options.allow_parallel_edges {
            diagnostics.push(located(
                Diagnostic::error(
                    PARSER,
                    diagnostic_codes::PARALLEL_EDGE_FORBIDDEN,
                    format!("parallel edge `{}` is forbidden", edge.id),
                ),
                locator.clone(),
            ));
        }
        indexes
            .outgoing
            .entry(edge.source.clone())
            .or_default()
            .push(position);
        indexes
            .incoming
            .entry(edge.target.clone())
            .or_default()
            .push(position);
        if !edge.directed && edge.source != edge.target {
            indexes
                .outgoing
                .entry(edge.target.clone())
                .or_default()
                .push(position);
            indexes
                .incoming
                .entry(edge.source.clone())
                .or_default()
                .push(position);
        }
        check_attribute_depth(
            edge.attrs.values(),
            options.max_attribute_depth,
            locator,
            &mut diagnostics,
            control,
        )?;
    }

    if options.require_dag {
        match analyze_graph_with_operation_control(
            document,
            source_map,
            &GraphAnalysisOptions {
                require_directed: true,
            },
            control,
        ) {
            Ok(analysis) => {
                for witness in analysis.cycle_witnesses {
                    let locator = witness.locators.first().cloned().flatten();
                    diagnostics.push(located(
                        Diagnostic::error(
                            PARSER,
                            diagnostic_codes::CYCLE,
                            format!("cycle detected: {}", witness.node_ids.join(" -> ")),
                        )
                        .with_affected_ids(witness.node_ids),
                        locator,
                    ));
                }
            }
            Err(GraphAnalysisError::Control(error)) => return Err(error),
            Err(GraphAnalysisError::UndirectedEdge { edge_id, locator }) => {
                diagnostics.push(located(
                    Diagnostic::error(
                        PARSER,
                        diagnostic_codes::UNDIRECTED_EDGE_FORBIDDEN,
                        format!("undirected edge `{edge_id}` is incompatible with DAG analysis"),
                    ),
                    locator,
                ))
            }
            Err(_) => {}
        }
    }

    Ok(GraphValidationResult {
        valid: diagnostics.is_empty(),
        diagnostics,
        indexes,
    })
}

pub fn analyze_graph(
    document: &GraphDocument,
    source_map: &GraphSourceMap,
    options: &GraphAnalysisOptions,
) -> Result<GraphAnalysis, GraphAnalysisError> {
    let control = trusted_control();
    analyze_graph_with_operation_control(document, source_map, options, &control)
}

pub fn analyze_graph_with_operation_control(
    document: &GraphDocument,
    source_map: &GraphSourceMap,
    options: &GraphAnalysisOptions,
    control: &OperationControl,
) -> Result<GraphAnalysis, GraphAnalysisError> {
    control.checkpoint()?;
    control
        .budget()
        .consume_nodes(to_u64(document.nodes.len().saturating_add(1)))
        .map_err(OperationControlError::from)?;
    control
        .budget()
        .consume_records(to_u64(document.edges.len()))
        .map_err(OperationControlError::from)?;
    control
        .budget()
        .observe_memory_bytes(estimated_bytes(document))
        .map_err(OperationControlError::from)?;

    let mut locators = BTreeMap::new();
    let mut adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (position, node) in document.nodes.iter().enumerate() {
        if adjacency.insert(node.id.clone(), BTreeSet::new()).is_some() {
            return Err(GraphAnalysisError::DuplicateNodeId {
                node_id: node.id.clone(),
                locator: source_map.nodes.get(position).cloned(),
            });
        }
        reverse.insert(node.id.clone(), BTreeSet::new());
        locators.insert(node.id.clone(), source_map.nodes.get(position).cloned());
    }
    for (position, edge) in document.edges.iter().enumerate() {
        control.checkpoint()?;
        if options.require_directed && !edge.directed {
            return Err(GraphAnalysisError::UndirectedEdge {
                edge_id: edge.id.clone(),
                locator: source_map.edges.get(position).cloned(),
            });
        }
        for endpoint in [&edge.source, &edge.target] {
            if !adjacency.contains_key(endpoint) {
                return Err(GraphAnalysisError::UnknownEndpoint {
                    edge_id: edge.id.clone(),
                    node_id: endpoint.clone(),
                    locator: source_map.edges.get(position).cloned(),
                });
            }
        }
        adjacency
            .get_mut(&edge.source)
            .expect("checked source")
            .insert(edge.target.clone());
        reverse
            .get_mut(&edge.target)
            .expect("checked target")
            .insert(edge.source.clone());
        if !edge.directed {
            adjacency
                .get_mut(&edge.target)
                .expect("checked target")
                .insert(edge.source.clone());
            reverse
                .get_mut(&edge.source)
                .expect("checked source")
                .insert(edge.target.clone());
        }
    }

    let components = iterative_scc(&adjacency, &reverse, control)?;
    let witnesses = components
        .iter()
        .filter_map(|component| cycle_witness(component, &adjacency, &locators))
        .collect::<Vec<_>>();
    let (topological_order, execution_layers) = if witnesses.is_empty() {
        let (order, layers) = stable_schedule(&adjacency, control)?;
        (Some(order), Some(layers))
    } else {
        (None, None)
    };
    Ok(GraphAnalysis {
        strongly_connected_components: components,
        cycle_witnesses: witnesses,
        topological_order,
        execution_layers,
    })
}

impl GraphAnalysis {
    pub fn require_dag(self) -> Result<Self, GraphAnalysisError> {
        if self.cycle_witnesses.is_empty() {
            Ok(self)
        } else {
            Err(GraphAnalysisError::Cyclic {
                witnesses: self.cycle_witnesses,
            })
        }
    }
}

fn iterative_scc(
    adjacency: &BTreeMap<String, BTreeSet<String>>,
    reverse: &BTreeMap<String, BTreeSet<String>>,
    control: &OperationControl,
) -> Result<Vec<Vec<String>>, OperationControlError> {
    let mut visited = BTreeSet::new();
    let mut finish = Vec::new();
    for start in adjacency.keys() {
        if visited.contains(start) {
            continue;
        }
        visited.insert(start.clone());
        let mut stack = vec![(
            start.clone(),
            adjacency[start].iter().cloned().collect::<Vec<_>>(),
            0usize,
        )];
        while let Some((_node, neighbors, next_position)) = stack.last_mut() {
            control.checkpoint()?;
            if let Some(next) = neighbors.get(*next_position).cloned() {
                *next_position += 1;
                if visited.insert(next.clone()) {
                    stack.push((next.clone(), adjacency[&next].iter().cloned().collect(), 0));
                }
            } else {
                let (finished, _, _) = stack.pop().expect("non-empty DFS stack");
                finish.push(finished);
            }
        }
    }
    visited.clear();
    let mut components = Vec::new();
    while let Some(start) = finish.pop() {
        if !visited.insert(start.clone()) {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            control.checkpoint()?;
            component.push(node.clone());
            for next in reverse[&node].iter().rev() {
                if visited.insert(next.clone()) {
                    stack.push(next.clone());
                }
            }
        }
        component.sort();
        components.push(component);
    }
    components.sort();
    Ok(components)
}

fn cycle_witness(
    component: &[String],
    adjacency: &BTreeMap<String, BTreeSet<String>>,
    locators: &BTreeMap<String, Option<SourceLocator>>,
) -> Option<GraphCycleWitness> {
    let start = component.first()?;
    if component.len() == 1 {
        if !adjacency[start].contains(start) {
            return None;
        }
        return Some(witness(vec![start.clone(), start.clone()], locators));
    }
    let members = component.iter().cloned().collect::<BTreeSet<_>>();
    for first in adjacency[start]
        .iter()
        .filter(|node| members.contains(*node))
    {
        if let Some(mut path) = shortest_path(first, start, &members, adjacency) {
            path.insert(0, start.clone());
            return Some(witness(path, locators));
        }
    }
    None
}

fn shortest_path(
    from: &str,
    to: &str,
    members: &BTreeSet<String>,
    adjacency: &BTreeMap<String, BTreeSet<String>>,
) -> Option<Vec<String>> {
    let mut queue = VecDeque::from([from.to_string()]);
    let mut previous = BTreeMap::<String, String>::new();
    let mut visited = BTreeSet::from([from.to_string()]);
    while let Some(node) = queue.pop_front() {
        if node == to {
            let mut path = vec![node.clone()];
            let mut cursor = node;
            while let Some(parent) = previous.get(&cursor) {
                path.push(parent.clone());
                cursor = parent.clone();
            }
            path.reverse();
            return Some(path);
        }
        for next in adjacency[&node]
            .iter()
            .filter(|next| members.contains(*next))
        {
            if visited.insert(next.clone()) {
                previous.insert(next.clone(), node.clone());
                queue.push_back(next.clone());
            }
        }
    }
    None
}

fn witness(
    node_ids: Vec<String>,
    locators: &BTreeMap<String, Option<SourceLocator>>,
) -> GraphCycleWitness {
    let witness_locators = node_ids
        .iter()
        .map(|id| locators.get(id).cloned().flatten())
        .collect();
    GraphCycleWitness {
        node_ids,
        locators: witness_locators,
    }
}

fn stable_schedule(
    adjacency: &BTreeMap<String, BTreeSet<String>>,
    control: &OperationControl,
) -> Result<(Vec<String>, Vec<Vec<String>>), OperationControlError> {
    let mut indegree = adjacency
        .keys()
        .map(|node| (node.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for targets in adjacency.values() {
        for target in targets {
            *indegree.get_mut(target).expect("known target") += 1;
        }
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(node, _)| node.clone())
        .collect::<BTreeSet<_>>();
    let mut order = Vec::new();
    let mut layers = Vec::new();
    while !ready.is_empty() {
        control.checkpoint()?;
        let layer = ready.iter().cloned().collect::<Vec<_>>();
        ready.clear();
        for node in &layer {
            order.push(node.clone());
            for target in &adjacency[node] {
                let degree = indegree.get_mut(target).expect("known target");
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(target.clone());
                }
            }
        }
        layers.push(layer);
    }
    Ok((order, layers))
}

fn check_attribute_depth<'a>(
    roots: impl Iterator<Item = &'a Value>,
    maximum: Option<u64>,
    locator: Option<SourceLocator>,
    diagnostics: &mut Vec<Diagnostic>,
    control: &OperationControl,
) -> Result<(), OperationControlError> {
    let mut observed = 0u64;
    let mut stack = roots.map(|value| (value, 1u64)).collect::<Vec<_>>();
    while let Some((value, depth)) = stack.pop() {
        control.checkpoint()?;
        observed = observed.max(depth);
        match value {
            Value::Array(values) => stack.extend(values.iter().map(|value| (value, depth + 1))),
            Value::Object(values) => stack.extend(values.values().map(|value| (value, depth + 1))),
            _ => {}
        }
    }
    control.budget().observe_nesting_depth(observed)?;
    if maximum.is_some_and(|maximum| observed > maximum) {
        diagnostics.push(located(
            Diagnostic::error(
                PARSER,
                diagnostic_codes::ATTRIBUTE_DEPTH_EXCEEDED,
                format!("attribute nesting depth {observed} exceeds configured maximum"),
            ),
            locator,
        ));
    }
    Ok(())
}

fn located(diagnostic: Diagnostic, locator: Option<SourceLocator>) -> Diagnostic {
    match locator {
        Some(locator) => diagnostic.with_locator(locator),
        None => diagnostic,
    }
}

fn trusted_control() -> OperationControl {
    OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        CancellationToken::new(),
    )
    .expect("trusted graph analysis budget is valid")
}

fn estimated_bytes(document: &GraphDocument) -> u64 {
    to_u64(
        document
            .nodes
            .len()
            .saturating_mul(192)
            .saturating_add(document.edges.len().saturating_mul(160)),
    )
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
