//! Basin retrieval over typed code-structure walks.
//!
//! This module ports the basin-retrieval operator used by the research harness:
//! concrete node identities are canonicalized by first occurrence, relation labels
//! are optionally preserved, and retrieval is prefix-consistency pruning over the
//! canonical traces.

use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use crate::python::PythonFile;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const UNTYPED: &str = "—";

/// One step in a rooted typed walk.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalkStep {
    pub node: String,
    pub edge_type: Option<String>,
    pub direction: Option<String>,
}

impl WalkStep {
    pub fn root(node: impl Into<String>) -> Self {
        Self {
            node: node.into(),
            edge_type: None,
            direction: None,
        }
    }

    pub fn out(node: impl Into<String>, edge_type: impl Into<String>) -> Self {
        Self {
            node: node.into(),
            edge_type: Some(edge_type.into()),
            direction: Some("out".to_string()),
        }
    }
}

/// Canonical first-occurrence signature for a typed walk.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TypedSignature {
    pub node_count: usize,
    pub node_trace: Vec<usize>,
    pub edge_label_trace: Vec<String>,
    pub typed_edges: Vec<(usize, String, usize)>,
    pub typed_edge_counts: Vec<(usize, String, usize, usize)>,
}

impl TypedSignature {
    pub fn key(&self) -> String {
        let trace = self
            .node_trace
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(">");
        let labels = self.edge_label_trace.join(",");
        let edges = self
            .typed_edges
            .iter()
            .map(|(a, label, b)| format!("{a}-{label}->{b}"))
            .collect::<Vec<_>>()
            .join(",");
        let counts = self
            .typed_edge_counts
            .iter()
            .map(|(a, label, b, n)| format!("{a}-{label}->{b}:{n}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "n={};t={trace};el={labels};e={edges};c={counts}",
            self.node_count
        )
    }

    pub fn label_free(&self) -> Self {
        let mut counts: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for (a, _label, b, n) in &self.typed_edge_counts {
            *counts.entry((*a, *b)).or_default() += *n;
        }
        let typed_edges = counts
            .keys()
            .map(|(a, b)| (*a, UNTYPED.to_string(), *b))
            .collect();
        let typed_edge_counts = counts
            .into_iter()
            .map(|((a, b), n)| (a, UNTYPED.to_string(), b, n))
            .collect();
        Self {
            node_count: self.node_count,
            node_trace: self.node_trace.clone(),
            edge_label_trace: vec![UNTYPED.to_string(); self.edge_label_trace.len()],
            typed_edges,
            typed_edge_counts,
        }
    }
}

/// Signature variant used by relaxation.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignatureVariant {
    Typed,
    LabelFree,
}

pub fn typed_canonical_signature(walk: &[WalkStep]) -> TypedSignature {
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let mut node_trace = Vec::with_capacity(walk.len());
    for step in walk {
        let next_id = ids.len();
        let id = *ids.entry(step.node.as_str()).or_insert(next_id);
        node_trace.push(id);
    }

    let edge_label_trace = walk
        .iter()
        .enumerate()
        .map(|(idx, step)| {
            if idx == 0 {
                UNTYPED.to_string()
            } else {
                encode_edge_label(step.edge_type.as_deref(), step.direction.as_deref())
            }
        })
        .collect::<Vec<_>>();

    let mut counts: BTreeMap<(usize, String, usize), usize> = BTreeMap::new();
    for idx in 1..node_trace.len() {
        let key = (
            node_trace[idx - 1],
            edge_label_trace[idx].clone(),
            node_trace[idx],
        );
        *counts.entry(key).or_default() += 1;
    }

    let typed_edges = counts
        .keys()
        .map(|(a, label, b)| (*a, label.clone(), *b))
        .collect();
    let typed_edge_counts = counts
        .into_iter()
        .map(|((a, label, b), n)| (a, label, b, n))
        .collect();

    TypedSignature {
        node_count: node_trace.iter().copied().collect::<HashSet<_>>().len(),
        node_trace,
        edge_label_trace,
        typed_edges,
        typed_edge_counts,
    }
}

pub fn label_free_canonical_signature(walk: &[WalkStep]) -> TypedSignature {
    typed_canonical_signature(walk).label_free()
}

fn signature(walk: &[WalkStep], variant: SignatureVariant) -> TypedSignature {
    match variant {
        SignatureVariant::Typed => typed_canonical_signature(walk),
        SignatureVariant::LabelFree => label_free_canonical_signature(walk),
    }
}

fn encode_edge_label(edge_type: Option<&str>, direction: Option<&str>) -> String {
    match edge_type {
        Some(edge_type) => format!("{}|{}", edge_type, direction.unwrap_or("")),
        None => UNTYPED.to_string(),
    }
}

/// One indexed/queryable basin instance.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BasinInstance {
    pub family: String,
    pub walk: Vec<WalkStep>,
    pub focal: String,
}

impl BasinInstance {
    pub fn new(family: impl Into<String>, walk: Vec<WalkStep>) -> Self {
        let focal = walk
            .first()
            .map(|step| step.node.clone())
            .unwrap_or_default();
        Self {
            family: family.into(),
            walk,
            focal,
        }
    }
}

#[derive(Debug, Clone)]
struct IndexedInstance {
    instance: BasinInstance,
    signature: TypedSignature,
}

/// Snapshot of the active basin after a given evidence length.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelaxationState {
    pub evidence_length: usize,
    pub active_instances: Vec<String>,
    pub active_families: Vec<String>,
    pub family_counts: Vec<(String, usize)>,
    pub n_active: usize,
    pub n_distinct_families: usize,
}

impl RelaxationState {
    pub fn dominant_family(&self) -> Option<&str> {
        let (family, top_count) = self.family_counts.first()?;
        if self
            .family_counts
            .iter()
            .filter(|(_, count)| count == top_count)
            .count()
            > 1
        {
            None
        } else {
            Some(family.as_str())
        }
    }
}

/// Prefix-consistency basin index.
#[derive(Debug, Clone)]
pub struct RelaxationIndex {
    pub variant: SignatureVariant,
    entries: Vec<IndexedInstance>,
}

impl RelaxationIndex {
    pub fn new(variant: SignatureVariant) -> Self {
        Self {
            variant,
            entries: Vec::new(),
        }
    }

    pub fn add(&mut self, instance: BasinInstance) {
        let signature = signature(&instance.walk, self.variant);
        self.entries.push(IndexedInstance {
            instance,
            signature,
        });
    }

    pub fn add_all<I>(&mut self, instances: I)
    where
        I: IntoIterator<Item = BasinInstance>,
    {
        for instance in instances {
            self.add(instance);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn relax(&self, evidence_walk: &[WalkStep], length: Option<usize>) -> RelaxationState {
        let length = length.unwrap_or(evidence_walk.len());
        let query = signature(evidence_walk, self.variant);
        let query_nodes = prefix(&query.node_trace, length);
        let query_labels = prefix(&query.edge_label_trace, length);

        let active = self
            .entries
            .iter()
            .filter(|entry| {
                prefix(&entry.signature.node_trace, length) == query_nodes
                    && prefix(&entry.signature.edge_label_trace, length) == query_labels
            })
            .map(|entry| entry.instance.clone())
            .collect::<Vec<_>>();
        snapshot(length, &active)
    }

    pub fn relax_trajectory(&self, evidence_walk: &[WalkStep]) -> Vec<RelaxationState> {
        (1..=evidence_walk.len())
            .map(|length| self.relax(evidence_walk, Some(length)))
            .collect()
    }

    pub fn perturb_drop(&self, evidence_walk: &[WalkStep], drop_index: usize) -> RelaxationState {
        assert!(drop_index > 0, "cannot drop the root step");
        let shortened = evidence_walk
            .iter()
            .enumerate()
            .filter_map(|(idx, step)| (idx != drop_index).then(|| step.clone()))
            .collect::<Vec<_>>();
        self.relax(&shortened, None)
    }
}

fn prefix<T>(values: &[T], length: usize) -> &[T] {
    &values[..values.len().min(length)]
}

fn snapshot(length: usize, active: &[BasinInstance]) -> RelaxationState {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for instance in active {
        *counts.entry(instance.family.clone()).or_default() += 1;
    }
    let mut family_counts = counts.into_iter().collect::<Vec<_>>();
    family_counts.sort_by(|(fam_a, count_a), (fam_b, count_b)| {
        count_b.cmp(count_a).then_with(|| fam_a.cmp(fam_b))
    });
    RelaxationState {
        evidence_length: length,
        active_instances: active.iter().map(|i| i.focal.clone()).collect(),
        active_families: active.iter().map(|i| i.family.clone()).collect(),
        n_active: active.len(),
        n_distinct_families: family_counts.len(),
        family_counts,
    }
}

/// A typed directed code graph used to produce basin walks.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeGraph {
    pub module: String,
    pub nodes: BTreeSet<String>,
    pub edges: Vec<CodeEdge>,
    pub roots: BTreeSet<String>,
}

impl CodeGraph {
    pub fn new(module: impl Into<String>) -> Self {
        let module = module.into();
        let mut nodes = BTreeSet::new();
        nodes.insert(module.clone());
        Self {
            module,
            nodes,
            edges: Vec::new(),
            roots: BTreeSet::new(),
        }
    }

    pub fn add_edge(
        &mut self,
        src: impl Into<String>,
        relation: impl Into<String>,
        dst: impl Into<String>,
    ) {
        let src = src.into();
        let dst = dst.into();
        self.nodes.insert(src.clone());
        self.nodes.insert(dst.clone());
        self.edges.push(CodeEdge {
            src,
            relation: relation.into(),
            dst,
        });
    }

    pub fn successors(&self, node: &str) -> Vec<(&str, &str)> {
        self.edges
            .iter()
            .filter(|edge| edge.src == node)
            .map(|edge| (edge.relation.as_str(), edge.dst.as_str()))
            .collect()
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeEdge {
    pub src: String,
    pub relation: String,
    pub dst: String,
}

/// Walk extraction parameters. Defaults mirror the research harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkOptions {
    pub max_depth: usize,
    pub max_walks: usize,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            max_depth: 6,
            max_walks: 40,
        }
    }
}

/// Build a typed basin graph from a normalized `DocumentGraph`.
pub fn graph_from_document_graph(document: &DocumentGraph) -> CodeGraph {
    let mut graph = CodeGraph::new(document.id.clone());

    for node in &document.nodes {
        graph.nodes.insert(node.id.clone());
        if matches!(
            node.kind,
            DocumentNodeKind::Function | DocumentNodeKind::Method | DocumentNodeKind::Constructor
        ) {
            graph.roots.insert(node.id.clone());
        }
    }

    for edge in &document.edges {
        if let Some(relation) = basin_relation(&edge.relation) {
            graph.add_edge(edge.source.clone(), relation, edge.target.clone());
        }
    }

    graph
}

/// Build a typed code graph from grist's Python parser output via `DocumentGraph`.
pub fn graph_from_python_file(module: impl Into<String>, file: &PythonFile) -> CodeGraph {
    let document = file
        .to_document_graph(DocumentGraphContext::new(module.into()).with_language("python"))
        .expect("PythonFile to DocumentGraph projection should not fail");
    graph_from_document_graph(&document)
}

pub fn walks_from_document_graph(
    document: &DocumentGraph,
    options: WalkOptions,
) -> Vec<Vec<WalkStep>> {
    let graph = graph_from_document_graph(document);
    walks_from_graph(&graph, options)
}

pub fn walks_from_python_file(
    module: impl Into<String>,
    file: &PythonFile,
    options: WalkOptions,
) -> Vec<Vec<WalkStep>> {
    let document = file
        .to_document_graph(DocumentGraphContext::new(module.into()).with_language("python"))
        .expect("PythonFile to DocumentGraph projection should not fail");
    walks_from_document_graph(&document, options)
}

/// Produce bounded DFS walks rooted at function/method symbols.
pub fn walks_from_graph(graph: &CodeGraph, options: WalkOptions) -> Vec<Vec<WalkStep>> {
    if options.max_walks == 0 {
        return Vec::new();
    }

    let mut roots = if graph.roots.is_empty() {
        edge_connected_roots(graph)
    } else {
        graph.roots.iter().cloned().collect::<Vec<_>>()
    };
    roots.sort();
    roots.truncate(options.max_walks);

    let mut walks = Vec::new();
    for root in roots {
        let mut walk = vec![WalkStep::root(root.clone())];
        let mut visited = HashSet::from([root.clone()]);
        let mut stack = vec![(root, 0_usize)];

        while !stack.is_empty() && walk.len() < options.max_depth + 1 {
            let current = stack.last().map(|(node, _)| node.clone()).unwrap();
            let successors = graph.successors(&current);
            let mut advanced = false;

            while let Some((_, next_index)) = stack.last_mut() {
                if *next_index >= successors.len() {
                    break;
                }
                let (relation, dst) = successors[*next_index];
                *next_index += 1;

                if visited.contains(dst) && walk.len() > 2 {
                    walk.push(WalkStep::out(dst, relation));
                    if walk.len() >= options.max_depth + 1 {
                        break;
                    }
                    continue;
                }

                walk.push(WalkStep::out(dst, relation));
                visited.insert(dst.to_string());
                stack.push((dst.to_string(), 0));
                advanced = true;
                break;
            }

            if !advanced {
                stack.pop();
            }
        }

        if walk.len() >= 3 {
            walks.push(walk);
        }
    }

    walks
}

fn edge_connected_roots(graph: &CodeGraph) -> Vec<String> {
    let mut roots = BTreeSet::new();
    for edge in &graph.edges {
        roots.insert(edge.src.clone());
        roots.insert(edge.dst.clone());
    }
    if roots.is_empty() {
        roots.extend(graph.nodes.iter().cloned());
    }
    roots.into_iter().collect()
}

fn basin_relation(relation: &DocumentRelation) -> Option<&'static str> {
    match relation {
        DocumentRelation::Contains => Some("CONTAINS"),
        DocumentRelation::Calls => Some("CALLS"),
        DocumentRelation::Imports => Some("IMPORTS"),
        DocumentRelation::Exports => Some("EXPORTS"),
        DocumentRelation::Inherits => Some("INHERITS"),
        DocumentRelation::Implements => Some("IMPLEMENTS"),
        DocumentRelation::References => Some("REFERENCES"),
        DocumentRelation::Defines => Some("DEFINES"),
        DocumentRelation::Assigns => Some("ASSIGNS"),
        DocumentRelation::Returns => Some("RETURNS"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SourceInfo;
    use crate::python::{PythonDetailMode, PythonIngestOptions, parse_python};

    #[test]
    fn canonical_signature_preserves_typed_recurrence() {
        let walk = vec![
            WalkStep::root("A"),
            WalkStep::out("B", "CALLS"),
            WalkStep::out("A", "CALLS"),
        ];
        let sig = typed_canonical_signature(&walk);
        assert_eq!(sig.node_count, 2);
        assert_eq!(sig.node_trace, vec![0, 1, 0]);
        assert_eq!(
            sig.edge_label_trace,
            vec![
                UNTYPED.to_string(),
                "CALLS|out".to_string(),
                "CALLS|out".to_string()
            ]
        );
        assert!(sig.key().contains("t=0>1>0"));
    }

    #[test]
    fn relaxation_prunes_by_prefix_consistency() {
        let family_a = vec![
            WalkStep::root("x"),
            WalkStep::out("y", "CALLS"),
            WalkStep::out("z", "IMPORTS"),
        ];
        let same_shape = vec![
            WalkStep::root("a"),
            WalkStep::out("b", "CALLS"),
            WalkStep::out("c", "IMPORTS"),
        ];
        let different = vec![
            WalkStep::root("m"),
            WalkStep::out("n", "IMPORTS"),
            WalkStep::out("o", "CALLS"),
        ];

        let mut index = RelaxationIndex::new(SignatureVariant::Typed);
        index.add(BasinInstance::new("fam-a", family_a.clone()));
        index.add(BasinInstance::new("fam-a", same_shape));
        index.add(BasinInstance::new("fam-b", different));

        let state = index.relax(&family_a, None);
        assert_eq!(state.n_active, 2);
        assert_eq!(state.dominant_family(), Some("fam-a"));
    }

    #[test]
    fn python_graph_includes_inherits_contains_and_calls() {
        let src = r#"
from base import Base
class Form(Base):
    def create(self):
        return helper()

def helper():
    return int(1)
"#;
        let parsed = parse_python(
            src,
            SourceInfo::stdin("forms.py"),
            &PythonIngestOptions {
                detail: PythonDetailMode::Semantic,
            },
        );
        let graph = graph_from_python_file("repo:forms", &parsed.payload);

        assert!(graph.edges.iter().any(|edge| {
            edge.relation == "INHERITS" && edge.src.contains("Form") && edge.dst == "Base"
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.relation == "CONTAINS"
                && edge.src.contains("Form")
                && edge.dst.contains("Form_create")
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.relation == "CALLS" && edge.src.contains("Form_create") && edge.dst == "helper"
        }));

        let document = parsed
            .payload
            .to_document_graph(crate::document_graph::DocumentGraphContext::new(
                "repo:forms",
            ))
            .expect("python document graph projection should succeed");
        let walks = walks_from_document_graph(&document, WalkOptions::default());
        assert!(!walks.is_empty());
    }
}
