//! Deterministic structural segmentation over canonical `DocumentGraph` order.

use super::{
    BoundaryKind, Segment, SegmentCollection, SegmentContext, SegmentCounts, SegmentNodeReference,
    SegmentNodeRole, SegmentOptions, SegmentOverlap, SegmentSizeUnit, TokenizerSpec,
};
use crate::core::{
    ContentIdentity, Diagnostic, SchemaVersion, SourceLocator, canonical_json_sha256, sha256_hex,
};
use crate::document_graph::{
    AttrMap, DocumentGraph, DocumentNode, DocumentNodeKind, DocumentRelation,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::thread;
use thiserror::Error;

const ENGINE_NAME: &str = "grist.segment.structural";
const RENDERER_NAME: &str = "grist.segment.node_concatenation";
const RENDERER_VERSION: &str = "1";
const SEGMENT_IDENTITY_VERSION: &str = "grist/segment-identity/v1";

/// A deterministic tokenizer supplied to the segmentation engine.
pub trait SegmentTokenizer: Send + Sync {
    fn specification(&self) -> TokenizerSpec;
    fn count_tokens(&self, text: &str) -> Result<usize, String>;
}

/// Built-in, dependency-free tokenizer used by default options.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnicodeWhitespaceTokenizer;

impl SegmentTokenizer for UnicodeWhitespaceTokenizer {
    fn specification(&self) -> TokenizerSpec {
        TokenizerSpec::unicode_whitespace_v1()
    }

    fn count_tokens(&self, text: &str) -> Result<usize, String> {
        Ok(text.split_whitespace().count())
    }
}

#[derive(Debug, Error)]
pub enum SegmentError {
    #[error("invalid segmentation options: {0}")]
    InvalidOptions(String),
    #[error("invalid document graph: {0}")]
    InvalidGraph(String),
    #[error("{0} identity has no source, aggregate, decoded, or canonical hash")]
    MissingIdentity(&'static str),
    #[error("tokenizer {name}@{version} must be supplied by the caller")]
    MissingTokenizer { name: String, version: String },
    #[error("configured tokenizer does not match supplied tokenizer")]
    TokenizerMismatch,
    #[error("tokenizer failed: {0}")]
    Tokenizer(String),
    #[error("a parallel segmentation worker panicked")]
    WorkerPanic,
    #[error("canonical identity serialization failed: {0}")]
    Canonical(String),
}

/// Segment in canonical order using one worker.
pub fn segment_document_graph(
    graph: &DocumentGraph,
    source_identity: &ContentIdentity,
    document_identity: &ContentIdentity,
    options: &SegmentOptions,
    tokenizer: Option<&dyn SegmentTokenizer>,
) -> Result<SegmentCollection, SegmentError> {
    segment_impl(
        graph,
        source_identity,
        document_identity,
        options,
        tokenizer,
        1,
    )
}

/// Segment using parallel node preparation and canonical serial assembly.
pub fn segment_document_graph_parallel(
    graph: &DocumentGraph,
    source_identity: &ContentIdentity,
    document_identity: &ContentIdentity,
    options: &SegmentOptions,
    tokenizer: Option<&dyn SegmentTokenizer>,
    parallelism: usize,
) -> Result<SegmentCollection, SegmentError> {
    if parallelism == 0 {
        return Err(SegmentError::InvalidOptions(
            "parallelism must be greater than zero".to_string(),
        ));
    }
    segment_impl(
        graph,
        source_identity,
        document_identity,
        options,
        tokenizer,
        parallelism,
    )
}

fn segment_impl(
    graph: &DocumentGraph,
    source_identity: &ContentIdentity,
    document_identity: &ContentIdentity,
    options: &SegmentOptions,
    supplied_tokenizer: Option<&dyn SegmentTokenizer>,
    parallelism: usize,
) -> Result<SegmentCollection, SegmentError> {
    validate_options(options)?;
    validate_identity(source_identity, "source")?;
    validate_identity(document_identity, "document")?;

    let builtin = UnicodeWhitespaceTokenizer;
    let mut effective_options = options.clone();
    if effective_options.tokenizer.is_none() {
        effective_options.tokenizer = Some(builtin.specification());
    }
    let options = &effective_options;
    let configured = options.tokenizer.as_ref().expect("effective tokenizer");
    let tokenizer: &dyn SegmentTokenizer = match supplied_tokenizer {
        Some(tokenizer) => tokenizer,
        None if *configured == builtin.specification() => &builtin,
        None => {
            return Err(SegmentError::MissingTokenizer {
                name: configured.name.clone(),
                version: configured.version.clone(),
            });
        }
    };
    if tokenizer.specification() != *configured {
        return Err(SegmentError::TokenizerMismatch);
    }

    let mut graph = graph.clone();
    graph
        .validate_contract()
        .map_err(|error| SegmentError::InvalidGraph(error.to_string()))?;
    validate_parent_structure(&graph)?;
    graph
        .canonicalize()
        .map_err(|error| SegmentError::InvalidGraph(error.to_string()))?;

    let options_digest = canonical_json_sha256(options)
        .map_err(|error| SegmentError::Canonical(error.to_string()))?;
    let renderer_digest = canonical_json_sha256(&RendererIdentity {
        name: RENDERER_NAME,
        version: RENDERER_VERSION,
        separator_policy: "none",
    })
    .map_err(|error| SegmentError::Canonical(error.to_string()))?;
    let (prepared, mut diagnostics) = prepare_nodes(&graph, options, parallelism)?;
    let units = build_units(&graph, &prepared, options);
    let drafts = build_drafts(&graph, &prepared, &units, options, tokenizer)?;
    let source_digest = canonical_json_sha256(source_identity)
        .map_err(|error| SegmentError::Canonical(error.to_string()))?;
    let document_digest = canonical_json_sha256(document_identity)
        .map_err(|error| SegmentError::Canonical(error.to_string()))?;

    let mut segments = Vec::with_capacity(drafts.len());
    for draft in &drafts {
        let mut segment = materialize_segment(
            draft,
            &graph,
            &prepared,
            &units,
            source_identity,
            document_identity,
            &source_digest,
            &document_digest,
            options,
            &options_digest,
            &renderer_digest,
            tokenizer,
        )?;
        if segment.size > options.maximum_size {
            let atomic = draft
                .all_units()
                .filter_map(|index| units[*index].atomic)
                .next()
                .unwrap_or("source-node");
            segment.diagnostics.push(
                Diagnostic::warning(
                    ENGINE_NAME,
                    "segment.maximum_exceeded_for_atomic_source",
                    format!(
                        "{atomic} integrity requires size {} above configured maximum {}",
                        segment.size, options.maximum_size
                    ),
                )
                .with_module("segment")
                .with_affected_ids(segment.node_ids.clone())
                .with_explanation_key("segment.maximum_exceeded_for_atomic_source"),
            );
        }
        segments.push(segment);
    }
    for index in 1..segments.len() {
        let node_ids = drafts[index].overlap_node_ids(&units);
        if !node_ids.is_empty() {
            let previous_id = segments[index - 1].id.clone();
            segments[index].overlaps.push(SegmentOverlap {
                segment_id: previous_id,
                node_ids,
            });
        }
    }
    diagnostics
        .sort_by_cached_key(|diagnostic| canonical_json_sha256(diagnostic).unwrap_or_default());
    Ok(SegmentCollection {
        schema_version: SchemaVersion::SEGMENT_COLLECTION_V1.to_string(),
        document_graph_schema_version: graph.schema_version,
        options: options.clone(),
        options_digest,
        segments,
        diagnostics,
    })
}

fn validate_options(options: &SegmentOptions) -> Result<(), SegmentError> {
    if options.schema_version != SchemaVersion::SEGMENT_OPTIONS_V1 {
        return Err(SegmentError::InvalidOptions(format!(
            "unsupported schema version {}",
            options.schema_version
        )));
    }
    if options.target_size == 0 || options.maximum_size == 0 {
        return Err(SegmentError::InvalidOptions(
            "target_size and maximum_size must be greater than zero".to_string(),
        ));
    }
    if options.target_size > options.maximum_size {
        return Err(SegmentError::InvalidOptions(
            "target_size must not exceed maximum_size".to_string(),
        ));
    }
    if let Some(tokenizer) = &options.tokenizer {
        if tokenizer.name.trim().is_empty()
            || tokenizer.version.trim().is_empty()
            || !tokenizer.configuration_digest.starts_with("sha256:")
        {
            return Err(SegmentError::InvalidOptions(
                "tokenizer name/version must be non-empty and configuration_digest must be sha256"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_identity(identity: &ContentIdentity, kind: &'static str) -> Result<(), SegmentError> {
    if identity.raw.is_none()
        && identity.decoded.is_none()
        && identity.canonical_payload.is_none()
        && identity.aggregate.is_none()
    {
        return Err(SegmentError::MissingIdentity(kind));
    }
    Ok(())
}

fn validate_parent_structure(graph: &DocumentGraph) -> Result<(), SegmentError> {
    let indexes = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    for node in &graph.nodes {
        let mut current = node.parent.as_deref();
        let mut seen = BTreeSet::from([node.id.as_str()]);
        while let Some(parent) = current {
            let index = indexes.get(parent).ok_or_else(|| {
                SegmentError::InvalidGraph(format!(
                    "node {} references missing parent {parent}",
                    node.id
                ))
            })?;
            if !seen.insert(parent) {
                return Err(SegmentError::InvalidGraph(format!(
                    "parent cycle contains node {}",
                    node.id
                )));
            }
            current = graph.nodes[*index].parent.as_deref();
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct PreparedNode {
    graph_index: usize,
    id: String,
    kind: DocumentNodeKind,
    text: String,
    locator: SourceLocator,
    attrs: AttrMap,
}

struct PreparedOutcome {
    graph_index: usize,
    node: Option<PreparedNode>,
    diagnostic: Option<Diagnostic>,
}

fn prepare_nodes(
    graph: &DocumentGraph,
    options: &SegmentOptions,
    parallelism: usize,
) -> Result<(Vec<PreparedNode>, Vec<Diagnostic>), SegmentError> {
    let outcomes = if parallelism <= 1 || graph.nodes.len() <= 1 {
        graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| prepare_node(index, node, options))
            .collect::<Vec<_>>()
    } else {
        let worker_count = parallelism.min(graph.nodes.len());
        let chunk_size = graph.nodes.len().div_ceil(worker_count);
        let joined = thread::scope(|scope| {
            let handles = graph
                .nodes
                .chunks(chunk_size)
                .enumerate()
                .map(|(chunk_index, chunk)| {
                    scope.spawn(move || {
                        let offset = chunk_index * chunk_size;
                        chunk
                            .iter()
                            .enumerate()
                            .map(|(index, node)| prepare_node(offset + index, node, options))
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().map_err(|_| SegmentError::WorkerPanic))
                .collect::<Result<Vec<_>, _>>()
        })?;
        joined.into_iter().flatten().collect::<Vec<_>>()
    };

    let mut outcomes = outcomes;
    outcomes.sort_by_key(|outcome| outcome.graph_index);
    let mut nodes = Vec::new();
    let mut diagnostics = Vec::new();
    for outcome in outcomes {
        if let Some(node) = outcome.node {
            nodes.push(node);
        }
        if let Some(diagnostic) = outcome.diagnostic {
            diagnostics.push(diagnostic);
        }
    }
    Ok((nodes, diagnostics))
}

fn prepare_node(
    graph_index: usize,
    node: &DocumentNode,
    options: &SegmentOptions,
) -> PreparedOutcome {
    if !selected(node, options) {
        return PreparedOutcome {
            graph_index,
            node: None,
            diagnostic: None,
        };
    }
    let Some(text) = renderable_text(node) else {
        return PreparedOutcome {
            graph_index,
            node: None,
            diagnostic: None,
        };
    };
    let Some(locator) = node.locator.clone() else {
        return PreparedOutcome {
            graph_index,
            node: None,
            diagnostic: Some(
                Diagnostic::warning(
                    ENGINE_NAME,
                    "segment.untraceable_node_omitted",
                    format!(
                        "selected node {} contains text but has no source locator",
                        node.id
                    ),
                )
                .with_module("segment")
                .with_affected_ids(vec![node.id.clone()])
                .with_explanation_key("segment.untraceable_node_omitted")
                .partial(),
            ),
        };
    };
    PreparedOutcome {
        graph_index,
        node: Some(PreparedNode {
            graph_index,
            id: node.id.clone(),
            kind: node.kind.clone(),
            text,
            locator,
            attrs: node.attrs.clone(),
        }),
        diagnostic: None,
    }
}

fn selected(node: &DocumentNode, options: &SegmentOptions) -> bool {
    let kind = node_kind_name(&node.kind);
    let selection = &options.selection;
    if selection.exclude_kinds.iter().any(|value| value == &kind) {
        return false;
    }
    if !selection.include_kinds.is_empty()
        && !selection.include_kinds.iter().any(|value| value == &kind)
    {
        return false;
    }
    let representation_explicitly_selected =
        selection.required_metadata.contains_key("text_origin")
            || selection.required_metadata.contains_key("segment_primary");
    if !representation_explicitly_selected
        && node
            .attrs
            .get("segment_primary")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
    {
        return false;
    }
    selection.required_metadata.iter().all(|(key, expected)| {
        node.attrs.get(key).is_some_and(|actual| {
            actual.as_str() == Some(expected)
                || serde_json::to_string(actual).is_ok_and(|value| value == *expected)
        })
    })
}

fn renderable_text(node: &DocumentNode) -> Option<String> {
    node.text
        .as_deref()
        .or(node.name.as_deref())
        .or(node.qualified_name.as_deref())
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn node_kind_name(kind: &DocumentNodeKind) -> String {
    match kind {
        DocumentNodeKind::Other(value) => value.clone(),
        _ => serde_json::to_value(kind)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

#[derive(Debug)]
struct Unit {
    nodes: Vec<PreparedNode>,
    preferred_boundary: bool,
    atomic: Option<&'static str>,
}

fn build_units(
    graph: &DocumentGraph,
    prepared: &[PreparedNode],
    options: &SegmentOptions,
) -> Vec<Unit> {
    let graph_indexes = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let figure_groups = figure_groups(graph);
    let mut positions = BTreeMap::<String, usize>::new();
    let mut units = Vec::<Unit>::new();
    for node in prepared {
        let (key, atomic) = atomic_group(node, graph, &graph_indexes, &figure_groups, options);
        let position = if let Some(position) = positions.get(&key) {
            *position
        } else {
            let position = units.len();
            positions.insert(key, position);
            units.push(Unit {
                nodes: Vec::new(),
                preferred_boundary: false,
                atomic,
            });
            position
        };
        units[position].nodes.push(node.clone());
    }
    for unit in &mut units {
        unit.preferred_boundary = unit.nodes.iter().any(|node| {
            boundary_for_kind(&node.kind)
                .is_some_and(|boundary| options.preferred_boundaries.contains(&boundary))
        });
    }
    units
}

fn figure_groups(graph: &DocumentGraph) -> BTreeMap<String, String> {
    let kinds = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), &node.kind))
        .collect::<BTreeMap<_, _>>();
    let mut groups = BTreeMap::new();
    for edge in &graph.edges {
        if edge.relation != DocumentRelation::CaptionFor {
            continue;
        }
        match (
            kinds.get(edge.source.as_str()),
            kinds.get(edge.target.as_str()),
        ) {
            (Some(DocumentNodeKind::Caption), Some(DocumentNodeKind::Figure)) => {
                groups.insert(edge.source.clone(), edge.target.clone());
                groups.insert(edge.target.clone(), edge.target.clone());
            }
            (Some(DocumentNodeKind::Figure), Some(DocumentNodeKind::Caption)) => {
                groups.insert(edge.source.clone(), edge.source.clone());
                groups.insert(edge.target.clone(), edge.source.clone());
            }
            _ => {}
        }
    }
    groups
}

fn atomic_group(
    node: &PreparedNode,
    graph: &DocumentGraph,
    indexes: &BTreeMap<&str, usize>,
    figure_groups: &BTreeMap<String, String>,
    options: &SegmentOptions,
) -> (String, Option<&'static str>) {
    let ancestry = ancestry_indexes(node.graph_index, graph, indexes);
    if options.atomicity.tables
        && let Some(root) = ancestry
            .iter()
            .rev()
            .find(|index| graph.nodes[**index].kind == DocumentNodeKind::Table)
    {
        return (format!("table:{}", graph.nodes[*root].id), Some("table"));
    }
    if options.atomicity.code
        && let Some(root) = ancestry
            .iter()
            .find(|index| is_code_atomic(&graph.nodes[**index].kind))
    {
        return (format!("code:{}", graph.nodes[*root].id), Some("code"));
    }
    if options.atomicity.equations
        && let Some(root) = ancestry
            .iter()
            .find(|index| is_equation_atomic(&graph.nodes[**index].kind))
    {
        return (
            format!("equation:{}", graph.nodes[*root].id),
            Some("equation"),
        );
    }
    if options.atomicity.figure_captions {
        if let Some(root) = figure_groups.get(&node.id) {
            return (format!("figure:{root}"), Some("figure-caption"));
        }
        if let Some(root) = ancestry
            .iter()
            .find(|index| graph.nodes[**index].kind == DocumentNodeKind::Figure)
        {
            return (
                format!("figure:{}", graph.nodes[*root].id),
                Some("figure-caption"),
            );
        }
    }
    (format!("node:{}", node.id), None)
}

fn ancestry_indexes(
    graph_index: usize,
    graph: &DocumentGraph,
    indexes: &BTreeMap<&str, usize>,
) -> Vec<usize> {
    let mut output = vec![graph_index];
    let mut current = graph.nodes[graph_index].parent.as_deref();
    while let Some(parent) = current {
        let Some(index) = indexes.get(parent) else {
            break;
        };
        output.push(*index);
        current = graph.nodes[*index].parent.as_deref();
    }
    output
}

fn is_code_atomic(kind: &DocumentNodeKind) -> bool {
    matches!(
        kind,
        DocumentNodeKind::CodeBlock
            | DocumentNodeKind::CodeSymbol
            | DocumentNodeKind::Symbol
            | DocumentNodeKind::Module
            | DocumentNodeKind::Namespace
            | DocumentNodeKind::Package
            | DocumentNodeKind::Class
            | DocumentNodeKind::Function
            | DocumentNodeKind::Method
            | DocumentNodeKind::Constructor
            | DocumentNodeKind::TypeAlias
            | DocumentNodeKind::Enum
            | DocumentNodeKind::Variable
            | DocumentNodeKind::Field
            | DocumentNodeKind::Interface
    )
}

fn is_equation_atomic(kind: &DocumentNodeKind) -> bool {
    matches!(
        kind,
        DocumentNodeKind::Equation | DocumentNodeKind::MathBlock
    )
}

fn boundary_for_kind(kind: &DocumentNodeKind) -> Option<BoundaryKind> {
    match kind {
        DocumentNodeKind::Document => Some(BoundaryKind::Document),
        DocumentNodeKind::Section | DocumentNodeKind::Heading => Some(BoundaryKind::Section),
        DocumentNodeKind::Paragraph => Some(BoundaryKind::Paragraph),
        DocumentNodeKind::ListItem => Some(BoundaryKind::ListItem),
        DocumentNodeKind::TableRow | DocumentNodeKind::Row => Some(BoundaryKind::TableRow),
        DocumentNodeKind::CodeSymbol
        | DocumentNodeKind::Symbol
        | DocumentNodeKind::Module
        | DocumentNodeKind::Namespace
        | DocumentNodeKind::Package
        | DocumentNodeKind::Class
        | DocumentNodeKind::Function
        | DocumentNodeKind::Method
        | DocumentNodeKind::Constructor
        | DocumentNodeKind::Interface
        | DocumentNodeKind::TypeAlias
        | DocumentNodeKind::Enum
        | DocumentNodeKind::Variable
        | DocumentNodeKind::Field => Some(BoundaryKind::CodeSymbol),
        DocumentNodeKind::Page => Some(BoundaryKind::Page),
        DocumentNodeKind::Slide => Some(BoundaryKind::Slide),
        DocumentNodeKind::Sheet => Some(BoundaryKind::SheetRange),
        DocumentNodeKind::Email | DocumentNodeKind::MessageBody => Some(BoundaryKind::Message),
        DocumentNodeKind::NotebookCell => Some(BoundaryKind::NotebookCell),
        DocumentNodeKind::Cue => Some(BoundaryKind::TranscriptCue),
        _ => None,
    }
}

#[derive(Debug)]
struct Draft {
    overlap_units: Vec<usize>,
    primary_units: Vec<usize>,
}

impl Draft {
    fn all_units(&self) -> impl Iterator<Item = &usize> {
        self.overlap_units.iter().chain(&self.primary_units)
    }

    fn overlap_node_ids(&self, units: &[Unit]) -> Vec<String> {
        self.overlap_units
            .iter()
            .flat_map(|index| units[*index].nodes.iter().map(|node| node.id.clone()))
            .collect()
    }
}

fn build_drafts(
    graph: &DocumentGraph,
    prepared: &[PreparedNode],
    units: &[Unit],
    options: &SegmentOptions,
    tokenizer: &dyn SegmentTokenizer,
) -> Result<Vec<Draft>, SegmentError> {
    let mut drafts = Vec::new();
    let mut overlap_units = Vec::new();
    let mut primary_units = Vec::new();
    for unit_index in 0..units.len() {
        if !primary_units.is_empty() {
            let mut candidate = Draft {
                overlap_units: overlap_units.clone(),
                primary_units: primary_units.clone(),
            };
            candidate.primary_units.push(unit_index);
            if draft_size(&candidate, graph, prepared, units, options, tokenizer)?
                > options.maximum_size
            {
                let completed = Draft {
                    overlap_units: std::mem::take(&mut overlap_units),
                    primary_units: std::mem::take(&mut primary_units),
                };
                overlap_units =
                    choose_overlap_units(&completed, units, options.overlap_source_nodes);
                drafts.push(completed);
            }
        }
        primary_units.push(unit_index);
        let current = Draft {
            overlap_units: overlap_units.clone(),
            primary_units: primary_units.clone(),
        };
        let size = draft_size(&current, graph, prepared, units, options, tokenizer)?;
        if size >= options.target_size && units[unit_index].preferred_boundary {
            let completed = Draft {
                overlap_units: std::mem::take(&mut overlap_units),
                primary_units: std::mem::take(&mut primary_units),
            };
            overlap_units = choose_overlap_units(&completed, units, options.overlap_source_nodes);
            drafts.push(completed);
        }
    }
    if !primary_units.is_empty() {
        drafts.push(Draft {
            overlap_units,
            primary_units,
        });
    }
    Ok(drafts)
}

fn choose_overlap_units(draft: &Draft, units: &[Unit], requested_nodes: usize) -> Vec<usize> {
    if requested_nodes == 0 {
        return Vec::new();
    }
    let mut selected = Vec::new();
    let mut count = 0usize;
    for index in draft
        .all_units()
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        selected.push(index);
        count = count.saturating_add(units[index].nodes.len());
        if count >= requested_nodes {
            break;
        }
    }
    selected.reverse();
    selected
}

fn draft_size(
    draft: &Draft,
    graph: &DocumentGraph,
    prepared: &[PreparedNode],
    units: &[Unit],
    options: &SegmentOptions,
    tokenizer: &dyn SegmentTokenizer,
) -> Result<usize, SegmentError> {
    let pieces = draft_pieces(draft, graph, prepared, units, options);
    let text = pieces
        .iter()
        .map(|piece| piece.node.text.as_str())
        .collect::<String>();
    Ok(counts(&text, tokenizer)?.size(options.size_unit))
}

#[derive(Clone, Copy)]
struct Piece<'a> {
    node: &'a PreparedNode,
    role: SegmentNodeRole,
}

fn draft_pieces<'a>(
    draft: &Draft,
    graph: &DocumentGraph,
    prepared: &'a [PreparedNode],
    units: &'a [Unit],
    options: &SegmentOptions,
) -> Vec<Piece<'a>> {
    let content_ids = draft
        .primary_units
        .iter()
        .flat_map(|index| units[*index].nodes.iter().map(|node| node.id.as_str()))
        .collect::<BTreeSet<_>>();
    let overlap_ids = draft
        .overlap_units
        .iter()
        .flat_map(|index| units[*index].nodes.iter().map(|node| node.id.as_str()))
        .collect::<BTreeSet<_>>();
    let first_index = draft
        .primary_units
        .first()
        .and_then(|index| units[*index].nodes.first())
        .map(|node| node.graph_index);
    let prepared_by_index = prepared
        .iter()
        .map(|node| (node.graph_index, node))
        .collect::<BTreeMap<_, _>>();
    let mut output = Vec::new();
    if options.include_heading_ancestry
        && let Some(first_index) = first_index
    {
        for index in heading_ancestry(first_index, graph, &prepared_by_index) {
            let node = prepared_by_index[&index];
            if !content_ids.contains(node.id.as_str()) && !overlap_ids.contains(node.id.as_str()) {
                output.push(Piece {
                    node,
                    role: SegmentNodeRole::Ancestry,
                });
            }
        }
    }
    for index in &draft.overlap_units {
        output.extend(units[*index].nodes.iter().map(|node| Piece {
            node,
            role: SegmentNodeRole::Overlap,
        }));
    }
    for index in &draft.primary_units {
        output.extend(units[*index].nodes.iter().map(|node| Piece {
            node,
            role: SegmentNodeRole::Content,
        }));
    }
    output
}

fn heading_ancestry(
    first_index: usize,
    graph: &DocumentGraph,
    prepared_by_index: &BTreeMap<usize, &PreparedNode>,
) -> Vec<usize> {
    let indexes = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut parent_headings = ancestry_indexes(first_index, graph, &indexes)
        .into_iter()
        .skip(1)
        .filter(|index| is_heading(&graph.nodes[*index].kind))
        .collect::<Vec<_>>();
    parent_headings.reverse();
    parent_headings.retain(|index| prepared_by_index.contains_key(index));
    if !parent_headings.is_empty() {
        return parent_headings;
    }

    let mut active = Vec::<usize>::new();
    for index in 0..first_index {
        let node = &graph.nodes[index];
        if !is_heading(&node.kind) || !prepared_by_index.contains_key(&index) {
            continue;
        }
        let level = node
            .attrs
            .get("level")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(1)
            .max(1);
        active.truncate(level.saturating_sub(1));
        active.push(index);
    }
    active
}

fn is_heading(kind: &DocumentNodeKind) -> bool {
    matches!(kind, DocumentNodeKind::Heading | DocumentNodeKind::Section)
}

#[allow(clippy::too_many_arguments)]
fn materialize_segment(
    draft: &Draft,
    graph: &DocumentGraph,
    prepared: &[PreparedNode],
    units: &[Unit],
    source_identity: &ContentIdentity,
    document_identity: &ContentIdentity,
    source_digest: &str,
    document_digest: &str,
    options: &SegmentOptions,
    options_digest: &str,
    renderer_digest: &str,
    tokenizer: &dyn SegmentTokenizer,
) -> Result<Segment, SegmentError> {
    let pieces = draft_pieces(draft, graph, prepared, units, options);
    let mut text = String::new();
    let mut node_references = Vec::with_capacity(pieces.len());
    for piece in &pieces {
        let start = text.len();
        text.push_str(&piece.node.text);
        node_references.push(SegmentNodeReference {
            node_id: piece.node.id.clone(),
            locator: piece.node.locator.clone(),
            role: piece.role,
            segment_byte_start: start,
            segment_byte_end: text.len(),
        });
    }
    let counts = counts(&text, tokenizer)?;
    let node_ids = node_references
        .iter()
        .map(|reference| reference.node_id.clone())
        .collect::<Vec<_>>();
    let locators = node_references
        .iter()
        .map(|reference| reference.locator.clone())
        .collect::<Vec<_>>();
    let first_graph_index = draft
        .primary_units
        .first()
        .and_then(|index| units[*index].nodes.first())
        .map(|node| node.graph_index)
        .expect("drafts always contain primary nodes");
    let context = segment_context(first_graph_index, graph, prepared);
    let metadata = segment_metadata(&pieces, options);
    let text_digest = sha256_hex(text.as_bytes());
    let identity = SegmentIdentityMaterial {
        identity_version: SEGMENT_IDENTITY_VERSION,
        graph_id: &graph.id,
        graph_schema_version: &graph.schema_version,
        source_identity: source_digest,
        document_identity: document_digest,
        node_references: node_references
            .iter()
            .map(|reference| SegmentIdentityReference {
                node_id: &reference.node_id,
                role: reference.role,
                locator: &reference.locator,
            })
            .collect(),
        text_sha256: &text_digest,
        renderer_digest,
        options_digest,
    };
    let digest = canonical_json_sha256(&identity)
        .map_err(|error| SegmentError::Canonical(error.to_string()))?;
    let id = format!(
        "grist:segment:{}",
        digest.strip_prefix("sha256:").unwrap_or(&digest)
    );
    Ok(Segment {
        schema_version: SchemaVersion::SEGMENT_V1.to_string(),
        id,
        node_ids,
        text,
        locators,
        source_identity: source_identity.clone(),
        document_identity: document_identity.clone(),
        context,
        size: counts.size(options.size_unit),
        token_count: counts.tokens,
        tokenizer: options.tokenizer.clone(),
        renderer: RENDERER_NAME.to_string(),
        renderer_version: RENDERER_VERSION.to_string(),
        options_digest: options_digest.to_string(),
        renderer_digest: renderer_digest.to_string(),
        node_references,
        counts,
        overlaps: Vec::new(),
        diagnostics: Vec::new(),
        metadata,
    })
}

fn counts(text: &str, tokenizer: &dyn SegmentTokenizer) -> Result<SegmentCounts, SegmentError> {
    let tokens = tokenizer
        .count_tokens(text)
        .map_err(SegmentError::Tokenizer)?;
    Ok(SegmentCounts {
        bytes: text.len(),
        unicode_scalars: text.chars().count(),
        tokens: Some(tokens),
    })
}

impl SegmentCounts {
    fn size(&self, unit: SegmentSizeUnit) -> usize {
        match unit {
            SegmentSizeUnit::UnicodeScalars => self.unicode_scalars,
            SegmentSizeUnit::Bytes => self.bytes,
            SegmentSizeUnit::Tokens => self.tokens.expect("validated tokenizer"),
        }
    }
}

fn segment_context(
    first_index: usize,
    graph: &DocumentGraph,
    prepared: &[PreparedNode],
) -> SegmentContext {
    let indexes = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut structural_indexes = ancestry_indexes(first_index, graph, &indexes);
    structural_indexes.reverse();
    let structural_path = structural_indexes
        .iter()
        .map(|index| graph.nodes[*index].id.clone())
        .collect();
    let prepared_by_index = prepared
        .iter()
        .map(|node| (node.graph_index, node))
        .collect::<BTreeMap<_, _>>();
    let section_titles = heading_ancestry(first_index, graph, &prepared_by_index)
        .into_iter()
        .filter_map(|index| prepared_by_index.get(&index).map(|node| node.text.clone()))
        .collect();
    let document_title = graph
        .attrs
        .get("title")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            prepared
                .iter()
                .find(|node| node.kind == DocumentNodeKind::Document)
                .map(|node| node.text.clone())
        });
    SegmentContext {
        structural_path,
        section_titles,
        document_title,
    }
}

fn segment_metadata(pieces: &[Piece<'_>], options: &SegmentOptions) -> BTreeMap<String, Value> {
    let mut metadata = options
        .metadata
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect::<BTreeMap<_, _>>();
    let mut projected = serde_json::Map::new();
    let unique_keys = options
        .project_metadata
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for key in unique_keys {
        let values = pieces
            .iter()
            .filter_map(|piece| {
                piece.node.attrs.get(&key).map(|value| {
                    json!({
                        "node_id": piece.node.id,
                        "value": value,
                    })
                })
            })
            .collect::<Vec<_>>();
        if !values.is_empty() {
            projected.insert(key, Value::Array(values));
        }
    }
    if !projected.is_empty() {
        metadata.insert("source_attributes".to_string(), Value::Object(projected));
    }
    metadata
}

#[derive(Serialize)]
struct RendererIdentity<'a> {
    name: &'a str,
    version: &'a str,
    separator_policy: &'a str,
}

#[derive(Serialize)]
struct SegmentIdentityMaterial<'a> {
    identity_version: &'static str,
    graph_id: &'a str,
    graph_schema_version: &'a str,
    source_identity: &'a str,
    document_identity: &'a str,
    node_references: Vec<SegmentIdentityReference<'a>>,
    text_sha256: &'a str,
    renderer_digest: &'a str,
    options_digest: &'a str,
}

#[derive(Serialize)]
struct SegmentIdentityReference<'a> {
    node_id: &'a str,
    role: SegmentNodeRole,
    locator: &'a SourceLocator,
}
