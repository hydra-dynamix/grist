//! Deterministic normalized renderers with complete generated-byte attribution.

use super::{
    FidelityMode, GeneratedRange, NORMALIZED_RENDERER_NAME, NORMALIZED_RENDERER_VERSION,
    RENDER_RESULT_V1, RENDER_SOURCE_MAP_V1, ReconstructionClaim, RenderError, RenderFidelity,
    RenderFormat, RenderLoss, RenderLossKind, RenderOptions, RenderResult, RenderSourceMap,
    RenderSourceMapEntry, SourceMapLocatorStatus,
};
use crate::core::{Diagnostic, LocatorPrecision, canonical_json_bytes, canonical_json_sha256};
use crate::document_graph::{DocumentGraph, DocumentNode, DocumentNodeKind, DocumentRelation};
use crate::security::{
    escape_active_html, escape_latex_text, inert_latex_literal, inert_markdown_code,
    sanitize_link_destination,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize)]
struct RendererIdentity {
    name: &'static str,
    version: &'static str,
    format: RenderFormat,
}

/// Render a graph into one normalized syntax with fidelity and source-map reports.
pub fn render_document_graph(
    graph: &DocumentGraph,
    format: RenderFormat,
    options: &RenderOptions,
) -> Result<RenderResult, RenderError> {
    graph
        .validate_contract()
        .map_err(|error| RenderError::InvalidGraph {
            message: error.to_string(),
        })?;
    let options_digest =
        canonical_json_sha256(options).map_err(|error| RenderError::CanonicalJson {
            message: error.to_string(),
        })?;
    let renderer_digest = canonical_json_sha256(&RendererIdentity {
        name: NORMALIZED_RENDERER_NAME,
        version: NORMALIZED_RENDERER_VERSION,
        format,
    })
    .map_err(|error| RenderError::CanonicalJson {
        message: error.to_string(),
    })?;

    let (content, source_map, diagnostics, losses) = if format == RenderFormat::CanonicalJson {
        render_canonical_json(graph)?
    } else {
        RenderState::new(graph, format, options.fidelity)?.render()?
    };
    let result = RenderResult {
        schema_version: RENDER_RESULT_V1.to_string(),
        format,
        media_type: format.media_type().to_string(),
        renderer: NORMALIZED_RENDERER_NAME.to_string(),
        renderer_version: NORMALIZED_RENDERER_VERSION.to_string(),
        renderer_digest,
        options_digest,
        content,
        source_map,
        fidelity: RenderFidelity {
            mode: options.fidelity,
            reconstruction_claim: ReconstructionClaim::NormalizedNotByteRoundTrip,
            losses,
        },
        diagnostics,
    };
    result.validate_source_map()?;
    Ok(result)
}

#[derive(Default)]
struct MappedWriter {
    content: String,
    entries: Vec<RenderSourceMapEntry>,
}

impl MappedWriter {
    fn push(&mut self, node: &DocumentNode, value: &str) {
        if value.is_empty() {
            return;
        }
        let byte_start = self.content.len();
        self.content.push_str(value);
        let byte_end = self.content.len();
        let original_locator = node
            .locator
            .clone()
            .or_else(|| node.range.clone().and_then(|range| range.try_into().ok()));
        let (locator_status, locator_unavailable_reason) = match original_locator.as_ref() {
            Some(locator) => {
                let status = match locator.precision() {
                    LocatorPrecision::Exact { .. } => SourceMapLocatorStatus::Exact,
                    LocatorPrecision::Approximate { .. } => SourceMapLocatorStatus::Approximate,
                    LocatorPrecision::Synthetic { .. } => SourceMapLocatorStatus::Synthetic,
                };
                (status, None)
            }
            None => (
                SourceMapLocatorStatus::Unavailable,
                Some("source node has no original locator".to_string()),
            ),
        };
        if let Some(last) = self.entries.last_mut()
            && last.node_id == node.id
            && last.original_locator == original_locator
            && last.generated.byte_end == byte_start
        {
            last.generated.byte_end = byte_end;
            return;
        }
        self.entries.push(RenderSourceMapEntry {
            generated: GeneratedRange {
                byte_start,
                byte_end,
            },
            node_id: node.id.clone(),
            original_locator,
            locator_status,
            locator_unavailable_reason,
        });
    }

    fn finish(self) -> (String, RenderSourceMap) {
        let generated_length = self.content.len();
        (
            self.content,
            RenderSourceMap {
                schema_version: RENDER_SOURCE_MAP_V1.to_string(),
                generated_unit: "utf8_bytes".to_string(),
                generated_length,
                entries: self.entries,
            },
        )
    }
}

struct RenderState<'a> {
    graph: &'a DocumentGraph,
    format: RenderFormat,
    mode: FidelityMode,
    writer: MappedWriter,
    children: BTreeMap<usize, Vec<usize>>,
    parented: BTreeSet<usize>,
    visited: BTreeSet<usize>,
    active: BTreeSet<usize>,
    diagnostics: Vec<Diagnostic>,
    losses: Vec<RenderLoss>,
}

impl<'a> RenderState<'a> {
    fn new(
        graph: &'a DocumentGraph,
        format: RenderFormat,
        mode: FidelityMode,
    ) -> Result<Self, RenderError> {
        let ids = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        let mut children = BTreeMap::<usize, Vec<usize>>::new();
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.relation == DocumentRelation::Contains)
        {
            let parent = ids.get(edge.source.as_str()).copied().ok_or_else(|| {
                RenderError::InvalidGraph {
                    message: format!("contains edge has unknown source {}", edge.source),
                }
            })?;
            let child = ids.get(edge.target.as_str()).copied().ok_or_else(|| {
                RenderError::InvalidGraph {
                    message: format!("contains edge has unknown target {}", edge.target),
                }
            })?;
            let values = children.entry(parent).or_default();
            if !values.contains(&child) {
                values.push(child);
            }
        }
        for (child, node) in graph.nodes.iter().enumerate() {
            if let Some(parent_id) = node.parent.as_deref() {
                let parent =
                    ids.get(parent_id)
                        .copied()
                        .ok_or_else(|| RenderError::InvalidGraph {
                            message: format!("node {} has unknown parent {parent_id}", node.id),
                        })?;
                let values = children.entry(parent).or_default();
                if !values.contains(&child) {
                    values.push(child);
                }
            }
        }
        for values in children.values_mut() {
            values.sort_by_key(|index| (graph.nodes[*index].ordinal.unwrap_or(usize::MAX), *index));
        }
        let parented = children.values().flatten().copied().collect();
        Ok(Self {
            graph,
            format,
            mode,
            writer: MappedWriter::default(),
            children,
            parented,
            visited: BTreeSet::new(),
            active: BTreeSet::new(),
            diagnostics: Vec::new(),
            losses: Vec::new(),
        })
    }

    fn render(
        mut self,
    ) -> Result<(String, RenderSourceMap, Vec<Diagnostic>, Vec<RenderLoss>), RenderError> {
        let mut roots = self
            .graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.parented.contains(index))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        roots.sort_by_key(|index| {
            (
                self.graph.nodes[*index].ordinal.unwrap_or(usize::MAX),
                *index,
            )
        });
        for root in roots {
            self.render_node(root)?;
        }
        for index in 0..self.graph.nodes.len() {
            if !self.visited.contains(&index) {
                self.render_node(index)?;
            }
        }
        let (content, source_map) = self.writer.finish();
        Ok((content, source_map, self.diagnostics, self.losses))
    }

    fn render_node(&mut self, index: usize) -> Result<(), RenderError> {
        if self.visited.contains(&index) {
            return Ok(());
        }
        if !self.active.insert(index) {
            return Err(RenderError::InvalidGraph {
                message: format!(
                    "containment cycle includes node {}",
                    self.graph.nodes[index].id
                ),
            });
        }
        let kind = self.graph.nodes[index].kind.clone();
        let loses_executable_metadata = self.graph.nodes[index].attrs.contains_key("executable");
        if is_unsupported_kind(&kind) || loses_executable_metadata {
            self.render_unsupported(index)?;
        } else {
            match self.format {
                RenderFormat::Markdown => self.render_markdown(index)?,
                RenderFormat::Latex => self.render_latex(index)?,
                RenderFormat::Html => self.render_html(index)?,
                RenderFormat::PlainText => self.render_text(index)?,
                RenderFormat::CanonicalJson => unreachable!("canonical JSON has a dedicated path"),
            }
        }
        self.active.remove(&index);
        self.visited.insert(index);
        Ok(())
    }

    fn render_children(&mut self, index: usize) -> Result<(), RenderError> {
        for child in self.children.get(&index).cloned().unwrap_or_default() {
            self.render_node(child)?;
        }
        Ok(())
    }

    fn mark_descendants_visited(&mut self, index: usize) {
        for child in self.children.get(&index).cloned().unwrap_or_default() {
            self.visited.insert(child);
            self.mark_descendants_visited(child);
        }
    }

    fn push(&mut self, index: usize, value: &str) {
        self.writer.push(&self.graph.nodes[index], value);
    }

    fn render_unsupported(&mut self, index: usize) -> Result<(), RenderError> {
        let node = &self.graph.nodes[index];
        match self.mode {
            FidelityMode::Strict => Err(RenderError::UnsupportedNode {
                format: self.format,
                node_id: node.id.clone(),
                node_kind: node.kind.clone(),
            }),
            FidelityMode::RawFallback => {
                let raw =
                    retained_raw_syntax(node).ok_or_else(|| RenderError::RawSourceUnavailable {
                        format: self.format,
                        node_id: node.id.clone(),
                    })?;
                let marker = raw_fallback(self.format, node, &raw);
                self.push(index, &marker);
                let mut diagnostic = Diagnostic::warning(
                    "grist.render",
                    "render.raw_fallback",
                    format!(
                        "preserved unsupported node {} as inert marked raw syntax",
                        node.id
                    ),
                )
                .with_affected_ids(vec![node.id.clone()]);
                if let Some(locator) = node.locator.clone() {
                    diagnostic = diagnostic.with_locator(locator);
                }
                self.diagnostics.push(diagnostic);
                Ok(())
            }
            FidelityMode::Lossy => {
                let message = format!(
                    "omitted node {} of kind {:?} from {:?} output",
                    node.id, node.kind, self.format
                );
                self.losses.push(RenderLoss {
                    node_id: node.id.clone(),
                    node_kind: node.kind.clone(),
                    kind: RenderLossKind::UnsupportedNodeKind,
                    message: message.clone(),
                });
                let mut diagnostic = Diagnostic::lossy("grist.render", message)
                    .with_affected_ids(vec![node.id.clone()]);
                if let Some(locator) = node.locator.clone() {
                    diagnostic = diagnostic.with_locator(locator);
                }
                self.diagnostics.push(diagnostic);
                Ok(())
            }
        }
    }

    fn render_markdown(&mut self, index: usize) -> Result<(), RenderError> {
        let node = &self.graph.nodes[index];
        let kind = node.kind.clone();
        let text = display_text(node).unwrap_or_default().to_string();
        match kind {
            DocumentNodeKind::Document
            | DocumentNodeKind::Transcript
            | DocumentNodeKind::MediaTrack => self.render_children(index),
            DocumentNodeKind::Cue => {
                self.push(index, &format!("{}\n\n", escape_markdown(&text)));
                Ok(())
            }
            DocumentNodeKind::Heading | DocumentNodeKind::Section => {
                let level = heading_level(node);
                self.push(index, &format!("{} ", "#".repeat(level)));
                if self.children.contains_key(&index) {
                    self.render_children(index)?;
                } else {
                    self.push(index, &escape_markdown(&text));
                }
                self.push(index, "\n\n");
                Ok(())
            }
            DocumentNodeKind::Paragraph => {
                if self.children.contains_key(&index) {
                    self.render_children(index)?;
                } else {
                    self.push(index, &escape_markdown(&text));
                }
                self.push(index, "\n\n");
                Ok(())
            }
            DocumentNodeKind::Text | DocumentNodeKind::TextRun => {
                self.push(index, &escape_markdown(&text));
                self.render_children(index)
            }
            DocumentNodeKind::Span => match node.attrs.get("markdown_kind").and_then(Value::as_str)
            {
                Some("strikethrough") => {
                    self.push(index, "~~");
                    if self.children.contains_key(&index) {
                        self.render_children(index)?;
                    } else {
                        self.push(index, &escape_markdown(&text));
                    }
                    self.push(index, "~~");
                    Ok(())
                }
                Some("thematicbreak") => {
                    self.push(index, "---\n\n");
                    Ok(())
                }
                Some("hardbreak") => {
                    self.push(index, "  \n");
                    Ok(())
                }
                Some("softbreak") => {
                    self.push(index, "\n");
                    Ok(())
                }
                Some("tasklistmarker") => Ok(()),
                _ => {
                    self.push(index, &escape_markdown(&text));
                    self.render_children(index)
                }
            },
            DocumentNodeKind::Emphasis | DocumentNodeKind::Strong => {
                let marker = if kind == DocumentNodeKind::Strong {
                    "**"
                } else {
                    "*"
                };
                self.push(index, marker);
                if self.children.contains_key(&index) {
                    self.render_children(index)?;
                } else {
                    self.push(index, &escape_markdown(&text));
                }
                self.push(index, marker);
                Ok(())
            }
            DocumentNodeKind::Link => {
                let destination = link_destination(node)?;
                self.push(index, "[");
                self.push(index, &escape_markdown(&text));
                self.push(
                    index,
                    &format!("]({})", sanitize_link_destination(destination)),
                );
                self.mark_descendants_visited(index);
                Ok(())
            }
            DocumentNodeKind::Image => {
                let destination = link_destination(node)?;
                self.push(
                    index,
                    &format!(
                        "![{}]({})",
                        escape_markdown(&text),
                        sanitize_link_destination(destination)
                    ),
                );
                self.mark_descendants_visited(index);
                Ok(())
            }
            DocumentNodeKind::InlineCode => {
                self.push(index, &inert_markdown_inline(&text));
                Ok(())
            }
            DocumentNodeKind::CodeBlock => {
                self.push(index, &inert_markdown_code(&text));
                self.mark_descendants_visited(index);
                Ok(())
            }
            DocumentNodeKind::MathInline | DocumentNodeKind::Math => {
                self.push(index, &format!("${}$", escape_active_html(&text)));
                Ok(())
            }
            DocumentNodeKind::MathBlock | DocumentNodeKind::Equation => {
                self.push(index, &format!("$$\n{}\n$$\n\n", escape_active_html(&text)));
                Ok(())
            }
            DocumentNodeKind::List => self.render_children(index),
            DocumentNodeKind::ListItem => {
                let marker = if node.attrs.get("ordered").and_then(Value::as_bool) == Some(true) {
                    format!(
                        "{}. ",
                        node.attrs
                            .get("item_number")
                            .and_then(Value::as_u64)
                            .unwrap_or(1)
                    )
                } else {
                    "- ".to_string()
                };
                self.push(index, &marker);
                if let Some(checked) = node.attrs.get("checked").and_then(Value::as_bool) {
                    self.push(index, if checked { "[x] " } else { "[ ] " });
                }
                if self.children.contains_key(&index) {
                    self.render_children(index)?;
                } else {
                    self.push(index, &escape_markdown(&text));
                }
                self.push(index, "\n");
                Ok(())
            }
            DocumentNodeKind::Quote => {
                self.push(index, "> ");
                self.push(index, &escape_markdown(&text).replace('\n', "\n> "));
                self.push(index, "\n\n");
                self.mark_descendants_visited(index);
                Ok(())
            }
            DocumentNodeKind::Table => {
                self.render_table(index)?;
                self.mark_descendants_visited(index);
                Ok(())
            }
            DocumentNodeKind::Frontmatter => {
                let delimiter = node
                    .attrs
                    .get("delimiter")
                    .and_then(Value::as_str)
                    .unwrap_or("---");
                self.push(index, &format!("{delimiter}\n{text}{delimiter}\n"));
                Ok(())
            }
            DocumentNodeKind::Footnote => {
                let label = node
                    .attrs
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("note");
                self.push(
                    index,
                    &format!("[^{label}]: {}\n\n", escape_markdown(&text)),
                );
                self.mark_descendants_visited(index);
                Ok(())
            }
            DocumentNodeKind::Reference
                if node.attrs.get("markdown_kind").and_then(Value::as_str)
                    == Some("footnotereference") =>
            {
                let label = node
                    .attrs
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or(text.as_str());
                self.push(index, &format!("[^{label}]"));
                Ok(())
            }
            _ => self.render_markdown_generic(index, &text),
        }
    }

    fn render_markdown_generic(&mut self, index: usize, text: &str) -> Result<(), RenderError> {
        let kind = self.graph.nodes[index].kind.clone();
        match kind {
            DocumentNodeKind::Label => self.push(index, &format!("{{#{}}}", escape_markdown(text))),
            DocumentNodeKind::Reference => {
                self.push(index, &format!("[{}]", escape_markdown(text)))
            }
            DocumentNodeKind::Citation => {
                self.push(index, &format!("[@{}]", escape_markdown(text)))
            }
            DocumentNodeKind::Caption => {
                self.push(index, &format!("*{}*\n\n", escape_markdown(text)));
            }
            _ if is_transparent(&kind) => {
                if !text.is_empty() {
                    self.push(
                        index,
                        &format!("**{}:** {}\n\n", kind_name(&kind), escape_markdown(text)),
                    );
                }
            }
            _ => {
                self.push(
                    index,
                    &format!("[{}] {}\n\n", kind_name(&kind), escape_markdown(text)),
                );
            }
        }
        self.render_children(index)
    }

    fn render_latex(&mut self, index: usize) -> Result<(), RenderError> {
        let node = &self.graph.nodes[index];
        let kind = node.kind.clone();
        let text = display_text(node).unwrap_or_default().to_string();
        match kind {
            DocumentNodeKind::Document
            | DocumentNodeKind::Transcript
            | DocumentNodeKind::MediaTrack => self.render_children(index),
            DocumentNodeKind::Cue => {
                self.push(index, &format!("{}\n\n", escape_latex_text(&text)));
                Ok(())
            }
            DocumentNodeKind::Heading | DocumentNodeKind::Section => {
                let command = match heading_level(node) {
                    1 => "section",
                    2 => "subsection",
                    3 => "subsubsection",
                    _ => "paragraph",
                };
                self.push(
                    index,
                    &format!("\\{command}{{{}}}\n", escape_latex_text(&text)),
                );
                self.render_children(index)
            }
            DocumentNodeKind::Paragraph => {
                if self.children.contains_key(&index) {
                    self.render_children(index)?;
                } else {
                    self.push(index, &escape_latex_text(&text));
                }
                self.push(index, "\n\n");
                Ok(())
            }
            DocumentNodeKind::Text | DocumentNodeKind::TextRun | DocumentNodeKind::Span => {
                self.push(index, &escape_latex_text(&text));
                self.render_children(index)
            }
            DocumentNodeKind::Emphasis | DocumentNodeKind::Strong => {
                let command = if kind == DocumentNodeKind::Strong {
                    "textbf"
                } else {
                    "emph"
                };
                self.push(index, &format!("\\{command}{{"));
                if self.children.contains_key(&index) {
                    self.render_children(index)?;
                } else {
                    self.push(index, &escape_latex_text(&text));
                }
                self.push(index, "}");
                Ok(())
            }
            DocumentNodeKind::Link => {
                let destination = sanitize_link_destination(link_destination(node)?);
                self.push(
                    index,
                    &format!(
                        "\\href{{{}}}{{{}}}",
                        escape_latex_text(&destination),
                        escape_latex_text(&text)
                    ),
                );
                Ok(())
            }
            DocumentNodeKind::InlineCode | DocumentNodeKind::CodeBlock => {
                self.push(index, &inert_latex_literal(&text));
                self.push(index, "\n");
                Ok(())
            }
            DocumentNodeKind::MathInline | DocumentNodeKind::Math => {
                self.push(index, &format!("${}$", escape_latex_text(&text)));
                Ok(())
            }
            DocumentNodeKind::MathBlock | DocumentNodeKind::Equation => {
                self.push(index, &format!("\\[\n{}\n\\]\n", escape_latex_text(&text)));
                Ok(())
            }
            DocumentNodeKind::Table => self.render_table(index),
            _ => self.render_latex_generic(index, &text),
        }
    }

    fn render_latex_generic(&mut self, index: usize, text: &str) -> Result<(), RenderError> {
        let kind = self.graph.nodes[index].kind.clone();
        match kind {
            DocumentNodeKind::Label => {
                self.push(index, &format!("\\label{{{}}}", escape_latex_text(text)))
            }
            DocumentNodeKind::Reference => {
                self.push(index, &format!("\\ref{{{}}}", escape_latex_text(text)))
            }
            DocumentNodeKind::Citation => {
                self.push(index, &format!("\\cite{{{}}}", escape_latex_text(text)))
            }
            _ if !text.is_empty() => self.push(
                index,
                &format!(
                    "\\textbf{{{}:}} {}\n\n",
                    escape_latex_text(&kind_name(&kind)),
                    escape_latex_text(text)
                ),
            ),
            _ => {}
        }
        self.render_children(index)
    }

    fn render_html(&mut self, index: usize) -> Result<(), RenderError> {
        let node = &self.graph.nodes[index];
        let kind = node.kind.clone();
        let text = display_text(node).unwrap_or_default().to_string();
        let node_id = escape_active_html(&node.id);
        match kind {
            DocumentNodeKind::MediaTrack => self.render_children(index),
            DocumentNodeKind::Cue => {
                self.push(
                    index,
                    &format!(
                        r#"<p data-grist-node="{node_id}" data-grist-kind="cue">{}</p>"#,
                        escape_active_html(&text)
                    ),
                );
                Ok(())
            }
            DocumentNodeKind::Document | DocumentNodeKind::Transcript => {
                self.push(index, &format!(r#"<article data-grist-node="{node_id}">"#));
                self.render_children(index)?;
                self.push(index, "</article>");
                Ok(())
            }
            DocumentNodeKind::Heading | DocumentNodeKind::Section => {
                let level = heading_level(node);
                self.push(
                    index,
                    &format!(
                        r#"<h{level} data-grist-node="{node_id}">{}</h{level}>"#,
                        escape_active_html(&text)
                    ),
                );
                self.render_children(index)
            }
            DocumentNodeKind::Paragraph => {
                self.push(index, &format!(r#"<p data-grist-node="{node_id}">"#));
                if self.children.contains_key(&index) {
                    self.render_children(index)?;
                } else {
                    self.push(index, &escape_active_html(&text));
                }
                self.push(index, "</p>");
                Ok(())
            }
            DocumentNodeKind::Text | DocumentNodeKind::TextRun | DocumentNodeKind::Span => {
                self.push(index, &escape_active_html(&text));
                self.render_children(index)
            }
            DocumentNodeKind::Emphasis | DocumentNodeKind::Strong => {
                let tag = if kind == DocumentNodeKind::Strong {
                    "strong"
                } else {
                    "em"
                };
                self.push(index, &format!(r#"<{tag} data-grist-node="{node_id}">"#));
                self.push(index, &escape_active_html(&text));
                self.render_children(index)?;
                self.push(index, &format!("</{tag}>"));
                Ok(())
            }
            DocumentNodeKind::Link => {
                let destination = sanitize_link_destination(link_destination(node)?);
                self.push(
                    index,
                    &format!(
                        r#"<a data-grist-node="{node_id}" href="{}">{}</a>"#,
                        escape_active_html(&destination),
                        escape_active_html(&text)
                    ),
                );
                Ok(())
            }
            DocumentNodeKind::CodeBlock => {
                self.push(
                    index,
                    &format!(
                        r#"<pre data-grist-node="{node_id}"><code>{}</code></pre>"#,
                        escape_active_html(&text)
                    ),
                );
                Ok(())
            }
            DocumentNodeKind::InlineCode => {
                self.push(
                    index,
                    &format!(
                        r#"<code data-grist-node="{node_id}">{}</code>"#,
                        escape_active_html(&text)
                    ),
                );
                Ok(())
            }
            DocumentNodeKind::Table => self.render_table(index),
            _ => self.render_html_generic(index, &text, &node_id),
        }
    }

    fn render_html_generic(
        &mut self,
        index: usize,
        text: &str,
        node_id: &str,
    ) -> Result<(), RenderError> {
        let kind = self.graph.nodes[index].kind.clone();
        let kind_name = escape_active_html(&kind_name(&kind));
        let tag = if is_inline(&kind) { "span" } else { "div" };
        self.push(
            index,
            &format!(r#"<{tag} data-grist-node="{node_id}" data-grist-kind="{kind_name}">"#),
        );
        self.push(index, &escape_active_html(text));
        self.render_children(index)?;
        self.push(index, &format!("</{tag}>"));
        Ok(())
    }

    fn render_text(&mut self, index: usize) -> Result<(), RenderError> {
        let node = &self.graph.nodes[index];
        let kind = node.kind.clone();
        let text = display_text(node).unwrap_or_default().to_string();
        match kind {
            DocumentNodeKind::Document
            | DocumentNodeKind::List
            | DocumentNodeKind::Transcript
            | DocumentNodeKind::MediaTrack => self.render_children(index),
            DocumentNodeKind::Cue => {
                self.push(index, &text);
                self.push(index, "\n");
                Ok(())
            }
            DocumentNodeKind::Heading | DocumentNodeKind::Section => {
                self.push(index, &text);
                self.push(index, "\n");
                self.push(index, &"=".repeat(text.chars().count().max(1)));
                self.push(index, "\n\n");
                self.render_children(index)
            }
            DocumentNodeKind::Paragraph => {
                if self.children.contains_key(&index) {
                    self.render_children(index)?;
                } else {
                    self.push(index, &text);
                }
                self.push(index, "\n\n");
                Ok(())
            }
            DocumentNodeKind::Text
            | DocumentNodeKind::TextRun
            | DocumentNodeKind::Span
            | DocumentNodeKind::Emphasis
            | DocumentNodeKind::Strong
            | DocumentNodeKind::InlineCode
            | DocumentNodeKind::MathInline
            | DocumentNodeKind::Math => {
                self.push(index, &text);
                self.render_children(index)
            }
            DocumentNodeKind::Link => {
                let destination = sanitize_link_destination(link_destination(node)?);
                self.push(index, &format!("{text} <{destination}>"));
                Ok(())
            }
            DocumentNodeKind::CodeBlock
            | DocumentNodeKind::MathBlock
            | DocumentNodeKind::Equation => {
                self.push(index, &text);
                self.push(index, "\n\n");
                Ok(())
            }
            DocumentNodeKind::ListItem => {
                self.push(index, "- ");
                self.push(index, &text);
                self.render_children(index)?;
                self.push(index, "\n");
                Ok(())
            }
            DocumentNodeKind::Quote => {
                self.push(index, &format!("> {}\n\n", text.replace('\n', "\n> ")));
                self.render_children(index)
            }
            DocumentNodeKind::Table => self.render_table(index),
            _ => {
                if !text.is_empty() {
                    self.push(index, &format!("[{}] {text}\n", kind_name(&kind)));
                }
                self.render_children(index)
            }
        }
    }

    fn render_table(&mut self, index: usize) -> Result<(), RenderError> {
        let rows = table_rows(self.graph, &self.children, index);
        match self.format {
            RenderFormat::Markdown => self.render_markdown_table(index, rows),
            RenderFormat::Latex => self.render_latex_table(index, rows),
            RenderFormat::Html => self.render_html_table(index, rows),
            RenderFormat::PlainText => self.render_text_table(index, rows),
            RenderFormat::CanonicalJson => unreachable!("canonical JSON has a dedicated path"),
        }
    }

    fn render_markdown_table(
        &mut self,
        table: usize,
        rows: Vec<TableRow>,
    ) -> Result<(), RenderError> {
        if rows.is_empty() {
            return Ok(());
        }
        for (row_number, row) in rows.iter().enumerate() {
            self.visited.insert(row.index);
            self.push(table, "|");
            for cell in &row.cells {
                self.visited.insert(cell.index);
                self.push(cell.index, &format!(" {} |", escape_table_cell(&cell.text)));
            }
            self.push(table, "\n");
            if row_number == 0 {
                self.push(table, "|");
                for _ in &row.cells {
                    self.push(table, " --- |");
                }
                self.push(table, "\n");
            }
        }
        self.push(table, "\n");
        Ok(())
    }

    fn render_latex_table(&mut self, table: usize, rows: Vec<TableRow>) -> Result<(), RenderError> {
        let columns = rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0)
            .max(1);
        self.push(
            table,
            &format!("\\begin{{tabular}}{{{}}}\n", "l".repeat(columns)),
        );
        for (row_number, row) in rows.iter().enumerate() {
            self.visited.insert(row.index);
            for (column, cell) in row.cells.iter().enumerate() {
                self.visited.insert(cell.index);
                if column > 0 {
                    self.push(table, " & ");
                }
                self.push(cell.index, &escape_latex_text(&cell.text));
            }
            self.push(table, " \\\\\n");
            if row_number == 0 {
                self.push(table, "\\hline\n");
            }
        }
        self.push(table, "\\end{tabular}\n");
        Ok(())
    }

    fn render_html_table(&mut self, table: usize, rows: Vec<TableRow>) -> Result<(), RenderError> {
        let id = escape_active_html(&self.graph.nodes[table].id);
        self.push(table, &format!(r#"<table data-grist-node="{id}">"#));
        for (row_number, row) in rows.iter().enumerate() {
            self.visited.insert(row.index);
            self.push(row.index, "<tr>");
            let tag = if row_number == 0 { "th" } else { "td" };
            for cell in &row.cells {
                self.visited.insert(cell.index);
                self.push(
                    cell.index,
                    &format!("<{tag}>{}</{tag}>", escape_active_html(&cell.text)),
                );
            }
            self.push(row.index, "</tr>");
        }
        self.push(table, "</table>");
        Ok(())
    }

    fn render_text_table(&mut self, table: usize, rows: Vec<TableRow>) -> Result<(), RenderError> {
        for row in &rows {
            self.visited.insert(row.index);
            for (column, cell) in row.cells.iter().enumerate() {
                self.visited.insert(cell.index);
                if column > 0 {
                    self.push(table, "\t");
                }
                self.push(cell.index, &cell.text.replace(['\r', '\n'], " "));
            }
            self.push(table, "\n");
        }
        self.push(table, "\n");
        Ok(())
    }
}

fn is_unsupported_kind(kind: &DocumentNodeKind) -> bool {
    matches!(
        kind,
        DocumentNodeKind::RawBlock
            | DocumentNodeKind::RawInline
            | DocumentNodeKind::Raw
            | DocumentNodeKind::Unknown
            | DocumentNodeKind::Other(_)
            | DocumentNodeKind::Module
            | DocumentNodeKind::Namespace
            | DocumentNodeKind::Package
            | DocumentNodeKind::Symbol
            | DocumentNodeKind::CodeSymbol
            | DocumentNodeKind::Class
            | DocumentNodeKind::Function
            | DocumentNodeKind::Method
            | DocumentNodeKind::Constructor
            | DocumentNodeKind::Interface
            | DocumentNodeKind::TypeAlias
            | DocumentNodeKind::Enum
            | DocumentNodeKind::Variable
            | DocumentNodeKind::Field
            | DocumentNodeKind::Import
            | DocumentNodeKind::Export
            | DocumentNodeKind::Call
            | DocumentNodeKind::Assignment
            | DocumentNodeKind::Return
            | DocumentNodeKind::Branch
            | DocumentNodeKind::Literal
            | DocumentNodeKind::Identifier
    )
}

fn is_transparent(kind: &DocumentNodeKind) -> bool {
    matches!(
        kind,
        DocumentNodeKind::Document
            | DocumentNodeKind::Container
            | DocumentNodeKind::Page
            | DocumentNodeKind::Slide
            | DocumentNodeKind::Sheet
            | DocumentNodeKind::List
            | DocumentNodeKind::TableRow
            | DocumentNodeKind::Row
    )
}

fn is_inline(kind: &DocumentNodeKind) -> bool {
    matches!(
        kind,
        DocumentNodeKind::Text
            | DocumentNodeKind::TextRun
            | DocumentNodeKind::Span
            | DocumentNodeKind::Emphasis
            | DocumentNodeKind::Strong
            | DocumentNodeKind::Link
            | DocumentNodeKind::InlineCode
            | DocumentNodeKind::MathInline
            | DocumentNodeKind::Reference
            | DocumentNodeKind::Citation
            | DocumentNodeKind::Label
    )
}

fn display_text(node: &DocumentNode) -> Option<&str> {
    node.text
        .as_deref()
        .or(node.name.as_deref())
        .or(node.qualified_name.as_deref())
}

fn heading_level(node: &DocumentNode) -> usize {
    node.attrs
        .get("level")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 6) as usize
}

fn link_destination(node: &DocumentNode) -> Result<&str, RenderError> {
    node.attrs
        .get("destination")
        .and_then(Value::as_str)
        .ok_or_else(|| RenderError::InvalidGraph {
            message: format!("link node {} has no string destination", node.id),
        })
}

fn kind_name(kind: &DocumentNodeKind) -> String {
    match serde_json::to_value(kind).unwrap_or(Value::String("unknown".to_string())) {
        Value::String(value) => value,
        Value::Object(value) => value
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "other".to_string()),
        _ => "unknown".to_string(),
    }
}

fn escape_markdown(text: &str) -> String {
    let text = escape_active_html(text);
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_table_cell(text: &str) -> String {
    escape_markdown(text)
        .replace('|', "\\|")
        .replace(['\r', '\n'], " ")
}

fn inert_markdown_inline(text: &str) -> String {
    let longest = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.saturating_add(1).max(1));
    format!("{fence}{text}{fence}")
}

fn retained_raw_syntax(node: &DocumentNode) -> Option<String> {
    if let Some(raw) = &node.raw {
        for key in ["raw", "source", "text", "value", "syntax"] {
            if let Some(value) = raw.payload.get(key).and_then(Value::as_str) {
                return Some(value.to_string());
            }
        }
        if let Some(value) = raw.payload.as_str() {
            return Some(value.to_string());
        }
        return canonical_json_bytes(&raw.payload)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok());
    }
    node.text.clone()
}

fn raw_fallback(format: RenderFormat, node: &DocumentNode, raw: &str) -> String {
    let namespace = node
        .raw
        .as_ref()
        .map(|value| value.namespace.as_str())
        .unwrap_or("unknown");
    let original_kind = node
        .raw
        .as_ref()
        .map(|value| value.original_kind.as_str())
        .unwrap_or("unknown");
    match format {
        RenderFormat::Markdown => format!(
            "[GRIST RAW node={} namespace={} kind={}]\n{}\n",
            escape_markdown(&node.id),
            escape_markdown(namespace),
            escape_markdown(original_kind),
            inert_markdown_code(raw)
        ),
        RenderFormat::Latex => format!(
            "\\textbf{{GRIST RAW node={} namespace={} kind={}}}\\\\\n{}\\\\\n",
            escape_latex_text(&node.id),
            escape_latex_text(namespace),
            escape_latex_text(original_kind),
            inert_latex_literal(raw)
        ),
        RenderFormat::Html => format!(
            r#"<pre class="grist-raw" data-node-id="{}" data-namespace="{}" data-original-kind="{}">GRIST RAW\n{}</pre>"#,
            escape_active_html(&node.id),
            escape_active_html(namespace),
            escape_active_html(original_kind),
            escape_active_html(raw)
        ),
        RenderFormat::PlainText => format!(
            "[[GRIST RAW node={} namespace={} kind={}]]\n{}\n[[/GRIST RAW]]\n",
            node.id, namespace, original_kind, raw
        ),
        RenderFormat::CanonicalJson => unreachable!("canonical JSON preserves the raw value"),
    }
}

#[derive(Clone)]
struct TableCell {
    index: usize,
    text: String,
}

#[derive(Clone)]
struct TableRow {
    index: usize,
    cells: Vec<TableCell>,
}

fn table_rows(
    graph: &DocumentGraph,
    children: &BTreeMap<usize, Vec<usize>>,
    table: usize,
) -> Vec<TableRow> {
    let mut rows = children
        .get(&table)
        .into_iter()
        .flatten()
        .filter(|index| {
            matches!(
                graph.nodes[**index].kind,
                DocumentNodeKind::TableRow | DocumentNodeKind::Row
            )
        })
        .map(|row| TableRow {
            index: *row,
            cells: children
                .get(row)
                .into_iter()
                .flatten()
                .filter(|index| {
                    matches!(
                        graph.nodes[**index].kind,
                        DocumentNodeKind::TableCell | DocumentNodeKind::Cell
                    )
                })
                .map(|cell| TableCell {
                    index: *cell,
                    text: display_text(&graph.nodes[*cell])
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        rows = table_rows_from_attrs(&graph.nodes[table], table);
    }
    rows
}

fn table_rows_from_attrs(node: &DocumentNode, owner: usize) -> Vec<TableRow> {
    node.attrs
        .get("table")
        .and_then(|value| value.get("rows"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|row| TableRow {
            index: owner,
            cells: row
                .as_array()
                .into_iter()
                .flatten()
                .map(|cell| TableCell {
                    index: owner,
                    text: cell
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| cell.to_string()),
                })
                .collect(),
        })
        .collect()
}

fn render_canonical_json(
    graph: &DocumentGraph,
) -> Result<(String, RenderSourceMap, Vec<Diagnostic>, Vec<RenderLoss>), RenderError> {
    let bytes = canonical_json_bytes(graph).map_err(|error| RenderError::CanonicalJson {
        message: error.to_string(),
    })?;
    let content = String::from_utf8(bytes).map_err(|error| RenderError::CanonicalJson {
        message: error.to_string(),
    })?;
    let root = graph
        .nodes
        .iter()
        .find(|node| node.kind == DocumentNodeKind::Document)
        .or_else(|| graph.nodes.first())
        .ok_or_else(|| RenderError::InvalidGraph {
            message: "canonical JSON source mapping requires at least one graph node".to_string(),
        })?;
    let mut writer = MappedWriter::default();
    let mut cursor = 0;
    let mut search_cursor =
        content
            .find(r#""nodes":["#)
            .ok_or_else(|| RenderError::CanonicalJson {
                message: "canonical graph JSON has no nodes array".to_string(),
            })?;
    for node in &graph.nodes {
        let fragment = canonical_json_bytes(node).map_err(|error| RenderError::CanonicalJson {
            message: error.to_string(),
        })?;
        let fragment = String::from_utf8(fragment).map_err(|error| RenderError::CanonicalJson {
            message: error.to_string(),
        })?;
        let relative =
            content[search_cursor..]
                .find(&fragment)
                .ok_or_else(|| RenderError::CanonicalJson {
                    message: format!("could not locate canonical JSON for node {}", node.id),
                })?;
        let start = search_cursor + relative;
        writer.push(root, &content[cursor..start]);
        writer.push(node, &content[start..start + fragment.len()]);
        cursor = start + fragment.len();
        search_cursor = cursor;
    }
    writer.push(root, &content[cursor..]);
    let (mapped_content, source_map) = writer.finish();
    debug_assert_eq!(mapped_content, content);
    Ok((mapped_content, source_map, Vec::new(), Vec::new()))
}
