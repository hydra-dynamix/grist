//! Authoritative tracked revisions and rich WordprocessingML object parsing.

use super::archive::PackageEntry;
use super::model::*;
use super::xml_util::{XmlNode, attribute, descendant_text, local_name, parse_xml_part};
use super::{PARSER, part_locator};
use crate::core::{Diagnostic, LocationComponent, SourceLocator};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(super) struct RichContentResult {
    pub revision_graph: WordRevisionGraph,
    pub comments: Vec<WordComment>,
    pub content_controls: Vec<WordContentControl>,
    pub equations: Vec<WordEquation>,
    pub drawings: Vec<WordDrawing>,
    pub charts: Vec<WordChart>,
    pub captions: Vec<WordCaption>,
    pub text_boxes: Vec<WordTextBox>,
    pub embedded_objects: Vec<WordEmbeddedObject>,
    pub node_count: usize,
}

struct ParsedPart {
    part: String,
    nodes: Vec<XmlNode>,
}

#[derive(Clone)]
struct LocatedObject {
    node_index: usize,
    object_id: String,
}

pub(super) fn parse_rich_content(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    main_part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> RichContentResult {
    let parts = parse_story_parts(entries, relationships, main_part, diagnostics);
    let mut revision_graph = WordRevisionGraph::default();
    for parsed in &parts {
        collect_revisions(
            &parsed.part,
            &parsed.nodes,
            0,
            None,
            &mut revision_graph,
            diagnostics,
        );
        for view in [
            WordRevisionView::Original,
            WordRevisionView::Accepted,
            WordRevisionView::Rejected,
        ] {
            let projection = project_story(&parsed.part, &parsed.nodes, view, &revision_graph);
            match view {
                WordRevisionView::Original => revision_graph.projections.original.push(projection),
                WordRevisionView::Accepted => revision_graph.projections.accepted.push(projection),
                WordRevisionView::Rejected => revision_graph.projections.rejected.push(projection),
            }
        }
    }
    pair_moves(&mut revision_graph, diagnostics);
    pair_revision_ranges(&mut revision_graph, diagnostics);

    let comments = parse_comments(entries, relationships, &parts, diagnostics);
    let mut content_controls = Vec::new();
    let mut equations = Vec::new();
    let mut drawings = Vec::new();
    let mut text_boxes = Vec::new();
    let mut embedded_objects = Vec::new();
    let mut charts = Vec::new();
    let mut captions = Vec::new();
    for parsed in &parts {
        let mut located_objects = Vec::new();
        collect_content_controls(&parsed.part, &parsed.nodes, 0, None, &mut content_controls);
        collect_equations(
            &parsed.part,
            &parsed.nodes,
            0,
            false,
            &mut equations,
            &mut located_objects,
        );
        collect_drawings(
            entries,
            relationships,
            &parsed.part,
            &parsed.nodes,
            0,
            false,
            &mut drawings,
            &mut charts,
            &mut text_boxes,
            &mut embedded_objects,
            &mut located_objects,
            diagnostics,
        );
        collect_captions(&parsed.part, &parsed.nodes, &located_objects, &mut captions);
    }
    let node_count = revision_graph.revisions.len()
        + revision_graph.edges.len()
        + revision_graph.projections.original.len()
        + revision_graph.projections.accepted.len()
        + revision_graph.projections.rejected.len()
        + comments.len()
        + content_controls.len()
        + equations.len()
        + drawings.len()
        + charts.len()
        + captions.len()
        + text_boxes.len()
        + embedded_objects.len();
    RichContentResult {
        revision_graph,
        comments,
        content_controls,
        equations,
        drawings,
        charts,
        captions,
        text_boxes,
        embedded_objects,
        node_count,
    }
}

fn parse_story_parts(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    main_part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<ParsedPart> {
    let mut selected = BTreeSet::new();
    selected.insert(main_part.to_string());
    for relationship in relationships.iter().filter(|relationship| {
        relationship.source_part.as_deref() == Some(main_part)
            && ["/header", "/footer", "/footnotes", "/endnotes", "/comments"]
                .iter()
                .any(|suffix| relationship.relationship_type.ends_with(suffix))
    }) {
        if let Some(part) = &relationship.resolved_part {
            selected.insert(part.clone());
        }
    }
    let mut ordered = Vec::new();
    if selected.remove(main_part) {
        ordered.push(main_part.to_string());
    }
    ordered.extend(selected);
    ordered
        .into_iter()
        .filter_map(|part| {
            let bytes = part_bytes(entries, &part)?;
            parse_xml_part(&part, bytes, diagnostics)
                .ok()
                .filter(|nodes| !nodes.is_empty())
                .map(|nodes| ParsedPart { part, nodes })
        })
        .collect()
}

fn part_bytes<'a>(entries: &'a [PackageEntry], part: &str) -> Option<&'a [u8]> {
    entries
        .iter()
        .find(|entry| entry.path == part && entry.rejected.is_none())
        .and_then(|entry| entry.bytes.as_deref())
}

fn rich_locator(part: &str, path: &str, object_id: impl Into<String>) -> SourceLocator {
    SourceLocator::exact(LocationComponent::OoxmlPart {
        part: part.to_string(),
        paragraph: None,
        run: None,
        table: None,
        row: None,
        column: None,
        object_id: Some(object_id.into()),
    })
    .expect("validated rich OOXML part")
    .nested(LocationComponent::XmlPath { path: path.into() })
    .expect("parser-generated absolute rich XML path")
}

fn revision_kind(name: &str) -> Option<WordRevisionKind> {
    match name {
        "ins" => Some(WordRevisionKind::Insertion),
        "del" => Some(WordRevisionKind::Deletion),
        "moveFrom" => Some(WordRevisionKind::MoveFrom),
        "moveTo" => Some(WordRevisionKind::MoveTo),
        "moveFromRangeStart" => Some(WordRevisionKind::MoveFromRangeStart),
        "moveFromRangeEnd" => Some(WordRevisionKind::MoveFromRangeEnd),
        "moveToRangeStart" => Some(WordRevisionKind::MoveToRangeStart),
        "moveToRangeEnd" => Some(WordRevisionKind::MoveToRangeEnd),
        "customXmlInsRangeStart" => Some(WordRevisionKind::CustomXmlInsertionRangeStart),
        "customXmlInsRangeEnd" => Some(WordRevisionKind::CustomXmlInsertionRangeEnd),
        "customXmlDelRangeStart" => Some(WordRevisionKind::CustomXmlDeletionRangeStart),
        "customXmlDelRangeEnd" => Some(WordRevisionKind::CustomXmlDeletionRangeEnd),
        "customXmlMoveFromRangeStart" => Some(WordRevisionKind::CustomXmlMoveFromRangeStart),
        "customXmlMoveFromRangeEnd" => Some(WordRevisionKind::CustomXmlMoveFromRangeEnd),
        "customXmlMoveToRangeStart" => Some(WordRevisionKind::CustomXmlMoveToRangeStart),
        "customXmlMoveToRangeEnd" => Some(WordRevisionKind::CustomXmlMoveToRangeEnd),
        "rPrChange" => Some(WordRevisionKind::RunProperties),
        "pPrChange" => Some(WordRevisionKind::ParagraphProperties),
        "tblPrChange" => Some(WordRevisionKind::TableProperties),
        "tblGridChange" => Some(WordRevisionKind::TableGridProperties),
        "trPrChange" => Some(WordRevisionKind::TableRowProperties),
        "tcPrChange" => Some(WordRevisionKind::TableCellProperties),
        "cellIns" => Some(WordRevisionKind::CellInsertion),
        "cellDel" => Some(WordRevisionKind::CellDeletion),
        "cellMerge" => Some(WordRevisionKind::CellMerge),
        "sectPrChange" => Some(WordRevisionKind::SectionProperties),
        "numPrChange" | "numberingChange" => Some(WordRevisionKind::NumberingProperties),
        _ => None,
    }
}

fn collect_revisions(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    parent: Option<&str>,
    graph: &mut WordRevisionGraph,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let node = &nodes[index];
    let kind = revision_kind(local_name(&node.name));
    let mut current_parent = parent.map(str::to_string);
    if let Some(kind) = kind {
        let native_id = attribute(node, "id").map(str::to_string);
        let node_id = format!(
            "{}#revision:{}:{}",
            part,
            native_id.as_deref().unwrap_or("unidentified"),
            node.path
        );
        let locator = rich_locator(part, &node.path, &node_id);
        diagnose_missing_revision_id(kind, native_id.as_deref(), &locator, diagnostics);
        if let Some(parent_id) = parent {
            graph.edges.push(WordRevisionEdge {
                source_revision_node_id: parent_id.to_string(),
                relation: WordRevisionRelation::Contains,
                target_revision_node_id: node_id.clone(),
                locator: locator.clone(),
            });
        }
        graph.revisions.push(WordRevision {
            node_id: node_id.clone(),
            revision_id: native_id,
            revision_kind: kind,
            author: attribute(node, "author").map(str::to_string),
            date: attribute(node, "date").map(str::to_string),
            story_part: part.to_string(),
            target_xml_path: node.path.clone(),
            parent_revision_node_id: parent.map(str::to_string),
            text: source_text(nodes, index),
            content: rich_xml(nodes, index),
            locator,
        });
        current_parent = Some(node_id);
    }
    for child in &node.children {
        collect_revisions(
            part,
            nodes,
            *child,
            current_parent.as_deref(),
            graph,
            diagnostics,
        );
    }
}

// Revision diagnostics are emitted before graph insertion.
fn diagnose_missing_revision_id(
    _kind: WordRevisionKind,
    id: Option<&str>,
    locator: &crate::core::SourceLocator,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if id.is_none() {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "word_ooxml.revision.missing_id",
                "tracked revision has no native revision ID",
            )
            .with_locator(locator.clone())
            .partial(),
        );
    }
}

fn pair_moves(graph: &mut WordRevisionGraph, diagnostics: &mut Vec<Diagnostic>) {
    let mut pairs = BTreeMap::<String, (Vec<usize>, Vec<usize>)>::new();
    for (index, revision) in graph.revisions.iter().enumerate() {
        let Some(id) = &revision.revision_id else {
            continue;
        };
        let pair = pairs.entry(id.clone()).or_default();
        match revision.revision_kind {
            WordRevisionKind::MoveFrom => pair.0.push(index),
            WordRevisionKind::MoveTo => pair.1.push(index),
            _ => {}
        }
    }
    for (id, (from, to)) in pairs {
        if from.len() == 1 && to.len() == 1 {
            graph.edges.push(WordRevisionEdge {
                source_revision_node_id: graph.revisions[from[0]].node_id.clone(),
                relation: WordRevisionRelation::MovePair,
                target_revision_node_id: graph.revisions[to[0]].node_id.clone(),
                locator: graph.revisions[to[0]].locator.clone(),
            });
        } else if !from.is_empty() || !to.is_empty() {
            let index = from.first().or_else(|| to.first()).copied().unwrap();
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.revision.unpaired_move",
                    format!(
                        "move revision {id} has {} source and {} destination nodes",
                        from.len(),
                        to.len()
                    ),
                )
                .with_locator(graph.revisions[index].locator.clone())
                .partial(),
            );
        }
    }
}

fn range_end_kind(kind: WordRevisionKind) -> Option<WordRevisionKind> {
    match kind {
        WordRevisionKind::MoveFromRangeStart => Some(WordRevisionKind::MoveFromRangeEnd),
        WordRevisionKind::MoveToRangeStart => Some(WordRevisionKind::MoveToRangeEnd),
        WordRevisionKind::CustomXmlInsertionRangeStart => {
            Some(WordRevisionKind::CustomXmlInsertionRangeEnd)
        }
        WordRevisionKind::CustomXmlDeletionRangeStart => {
            Some(WordRevisionKind::CustomXmlDeletionRangeEnd)
        }
        WordRevisionKind::CustomXmlMoveFromRangeStart => {
            Some(WordRevisionKind::CustomXmlMoveFromRangeEnd)
        }
        WordRevisionKind::CustomXmlMoveToRangeStart => {
            Some(WordRevisionKind::CustomXmlMoveToRangeEnd)
        }
        _ => None,
    }
}

fn pair_revision_ranges(graph: &mut WordRevisionGraph, diagnostics: &mut Vec<Diagnostic>) {
    let starts = graph
        .revisions
        .iter()
        .enumerate()
        .filter_map(|(index, revision)| {
            range_end_kind(revision.revision_kind).map(|end| (index, end))
        })
        .collect::<Vec<_>>();
    for (start_index, end_kind) in starts {
        let start = &graph.revisions[start_index];
        let ends = graph
            .revisions
            .iter()
            .enumerate()
            .filter(|(_, revision)| {
                revision.revision_kind == end_kind
                    && revision.revision_id == start.revision_id
                    && revision.story_part == start.story_part
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if ends.len() == 1 {
            graph.edges.push(WordRevisionEdge {
                source_revision_node_id: start.node_id.clone(),
                relation: WordRevisionRelation::RangePair,
                target_revision_node_id: graph.revisions[ends[0]].node_id.clone(),
                locator: graph.revisions[ends[0]].locator.clone(),
            });
        } else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.revision.unpaired_range",
                    format!(
                        "revision range {} has {} matching end markers",
                        start.node_id,
                        ends.len()
                    ),
                )
                .with_locator(start.locator.clone())
                .partial(),
            );
        }
    }
}

fn project_story(
    part: &str,
    nodes: &[XmlNode],
    view: WordRevisionView,
    graph: &WordRevisionGraph,
) -> WordStoryTextProjection {
    let mut text = String::new();
    append_projected(nodes, 0, view, true, &mut text);
    while text.ends_with(['\n', '\t']) {
        text.pop();
    }
    WordStoryTextProjection {
        view,
        story_part: part.to_string(),
        text,
        source_revision_node_ids: graph
            .revisions
            .iter()
            .filter(|revision| revision.story_part == part)
            .map(|revision| revision.node_id.clone())
            .collect(),
        locator: part_locator(part).expect("validated story part"),
    }
}

fn append_projected(
    nodes: &[XmlNode],
    index: usize,
    view: WordRevisionView,
    included: bool,
    output: &mut String,
) {
    let node = &nodes[index];
    let kind = revision_kind(local_name(&node.name));
    let included = included
        && !matches!(
            (view, kind),
            (WordRevisionView::Accepted, Some(WordRevisionKind::Deletion))
                | (WordRevisionView::Accepted, Some(WordRevisionKind::MoveFrom))
                | (
                    WordRevisionView::Rejected,
                    Some(WordRevisionKind::Insertion)
                )
                | (WordRevisionView::Rejected, Some(WordRevisionKind::MoveTo))
        );
    if !included {
        return;
    }
    match local_name(&node.name) {
        "t" | "delText" => output.push_str(&node.text),
        "tab" | "ptab" => output.push('\t'),
        "br" | "cr" => output.push('\n'),
        _ => {}
    }
    for child in &node.children {
        append_projected(nodes, *child, view, included, output);
    }
    match local_name(&node.name) {
        "p" | "tr" => output.push('\n'),
        "tc" => output.push('\t'),
        _ => {}
    }
}

fn source_text(nodes: &[XmlNode], index: usize) -> String {
    let mut text = String::new();
    append_projected(nodes, index, WordRevisionView::Original, true, &mut text);
    text.trim_end_matches(['\n', '\t']).to_string()
}

fn rich_xml(nodes: &[XmlNode], index: usize) -> WordRichXmlNode {
    let node = &nodes[index];
    WordRichXmlNode {
        name: node.name.clone(),
        attributes: node.attributes.clone(),
        text: node.text.clone(),
        children: node
            .children
            .iter()
            .map(|child| rich_xml(nodes, *child))
            .collect(),
    }
}

fn descendants(nodes: &[XmlNode], index: usize, output: &mut Vec<usize>) {
    for child in &nodes[index].children {
        output.push(*child);
        descendants(nodes, *child, output);
    }
}

fn first_descendant(nodes: &[XmlNode], index: usize, name: &str) -> Option<usize> {
    let mut all = Vec::new();
    descendants(nodes, index, &mut all);
    all.into_iter()
        .find(|candidate| local_name(&nodes[*candidate].name) == name)
}

fn parse_bool(value: &str) -> bool {
    !matches!(
        value.to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no"
    )
}

fn parse_comments(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    parts: &[ParsedPart],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<WordComment> {
    let Some(comment_part) = relationships
        .iter()
        .find(|relationship| relationship.relationship_type.ends_with("/comments"))
        .and_then(|relationship| relationship.resolved_part.as_deref())
    else {
        return Vec::new();
    };
    let Some(parsed) = parts.iter().find(|parsed| parsed.part == comment_part) else {
        return Vec::new();
    };
    let mut anchors = HashMap::<String, Vec<WordCommentAnchor>>::new();
    for story in parts {
        for node in &story.nodes {
            let anchor_kind = match local_name(&node.name) {
                "commentRangeStart" => Some(WordCommentAnchorKind::RangeStart),
                "commentRangeEnd" => Some(WordCommentAnchorKind::RangeEnd),
                "commentReference" => Some(WordCommentAnchorKind::Reference),
                _ => None,
            };
            if let (Some(anchor_kind), Some(id)) = (anchor_kind, attribute(node, "id")) {
                anchors
                    .entry(id.to_string())
                    .or_default()
                    .push(WordCommentAnchor {
                        anchor_kind,
                        story_part: story.part.clone(),
                        locator: rich_locator(
                            &story.part,
                            &node.path,
                            format!("comment-anchor:{id}"),
                        ),
                    });
            }
        }
    }
    let mut comments = Vec::new();
    for (index, node) in parsed.nodes.iter().enumerate() {
        if local_name(&node.name) != "comment" {
            continue;
        }
        let id = attribute(node, "id")
            .map(str::to_string)
            .unwrap_or_else(|| format!("unidentified:{index}"));
        let paragraph_id = first_descendant(&parsed.nodes, index, "p")
            .and_then(|paragraph| attribute(&parsed.nodes[paragraph], "paraId"))
            .map(str::to_string);
        comments.push(WordComment {
            id: id.clone(),
            durable_id: None,
            parent_comment_id: attribute(node, "parentId").map(str::to_string),
            reply_ids: Vec::new(),
            author: attribute(node, "author").map(str::to_string),
            initials: attribute(node, "initials").map(str::to_string),
            date: attribute(node, "date").map(str::to_string),
            resolved: None,
            paragraph_id,
            text: source_text(&parsed.nodes, index),
            anchors: anchors.remove(&id).unwrap_or_default(),
            content: rich_xml(&parsed.nodes, index),
            locator: rich_locator(comment_part, &node.path, format!("comment:{id}")),
        });
    }
    apply_comment_extensions(entries, &mut comments, diagnostics);
    for (id, orphan_anchors) in anchors {
        if let Some(anchor) = orphan_anchors.first() {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.comment.orphan_anchor",
                    format!("comment anchor {id} has no matching comment"),
                )
                .with_locator(anchor.locator.clone())
                .partial(),
            );
        }
    }
    let replies = comments
        .iter()
        .filter_map(|comment| {
            comment
                .parent_comment_id
                .as_ref()
                .map(|parent| (parent.clone(), comment.id.clone()))
        })
        .collect::<Vec<_>>();
    for (parent, reply) in replies {
        if let Some(comment) = comments.iter_mut().find(|comment| comment.id == parent) {
            comment.reply_ids.push(reply);
        } else if let Some(comment) = comments.iter().find(|comment| comment.id == reply) {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.comment.orphan_reply",
                    format!("comment {reply} replies to missing comment {parent}"),
                )
                .with_locator(comment.locator.clone())
                .partial(),
            );
        }
    }
    comments
}

fn apply_comment_extensions(
    entries: &[PackageEntry],
    comments: &mut [WordComment],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let para_to_comment = comments
        .iter()
        .filter_map(|comment| {
            comment
                .paragraph_id
                .as_ref()
                .map(|paragraph| (paragraph.clone(), comment.id.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut parents = HashMap::<String, String>::new();
    let mut resolved = HashMap::<String, bool>::new();
    let mut durable = HashMap::<String, String>::new();
    for entry in entries.iter().filter(|entry| {
        let path = entry.path.to_ascii_lowercase();
        (path.contains("commentsextended") || path.contains("commentsids"))
            && entry.bytes.is_some()
            && entry.rejected.is_none()
    }) {
        let Some(bytes) = entry.bytes.as_deref() else {
            continue;
        };
        let Ok(nodes) = parse_xml_part(&entry.path, bytes, diagnostics) else {
            continue;
        };
        for node in &nodes {
            match local_name(&node.name) {
                "commentEx" => {
                    let Some(paragraph) = attribute(node, "paraId") else {
                        continue;
                    };
                    if let Some(parent) = attribute(node, "paraIdParent") {
                        parents.insert(paragraph.to_string(), parent.to_string());
                    }
                    if let Some(done) = attribute(node, "done") {
                        resolved.insert(paragraph.to_string(), parse_bool(done));
                    }
                }
                "commentId" => {
                    if let (Some(paragraph), Some(id)) =
                        (attribute(node, "paraId"), attribute(node, "durableId"))
                    {
                        durable.insert(paragraph.to_string(), id.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    for comment in comments {
        let Some(paragraph) = comment.paragraph_id.as_ref() else {
            continue;
        };
        comment.durable_id = durable.get(paragraph).cloned();
        comment.resolved = resolved.get(paragraph).copied();
        if let Some(parent_paragraph) = parents.get(paragraph) {
            if let Some(parent_id) = para_to_comment.get(parent_paragraph) {
                comment.parent_comment_id = Some(parent_id.clone());
            } else {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "word_ooxml.comment.unresolved_parent",
                        format!(
                            "comment {} names missing parent paragraph {parent_paragraph}",
                            comment.id
                        ),
                    )
                    .with_locator(comment.locator.clone())
                    .partial(),
                );
            }
        }
    }
}

fn direct_child(nodes: &[XmlNode], index: usize, name: &str) -> Option<usize> {
    nodes[index]
        .children
        .iter()
        .copied()
        .find(|child| local_name(&nodes[*child].name) == name)
}

fn node_value(node: &XmlNode) -> Option<String> {
    attribute(node, "val")
        .or_else(|| attribute(node, "value"))
        .map(str::to_string)
}

fn collect_content_controls(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    parent: Option<&str>,
    output: &mut Vec<WordContentControl>,
) {
    let node = &nodes[index];
    let mut current_parent = parent.map(str::to_string);
    if local_name(&node.name) == "sdt" {
        let properties = direct_child(nodes, index, "sdtPr");
        let content = direct_child(nodes, index, "sdtContent").unwrap_or(index);
        let native_id = properties
            .and_then(|item| direct_child(nodes, item, "id"))
            .and_then(|item| node_value(&nodes[item]));
        let control_id = native_id.unwrap_or_else(|| format!("{part}#sdt:{}", node.path));
        let property_value = |name: &str| {
            properties
                .and_then(|item| direct_child(nodes, item, name))
                .and_then(|item| node_value(&nodes[item]))
        };
        let control_type = properties
            .and_then(|item| {
                [
                    "checkbox",
                    "comboBox",
                    "date",
                    "docPartList",
                    "dropDownList",
                    "equation",
                    "group",
                    "picture",
                    "repeatingSection",
                    "repeatingSectionItem",
                    "richText",
                    "text",
                ]
                .iter()
                .find(|name| direct_child(nodes, item, name).is_some())
                .map(|name| (*name).to_string())
            })
            .unwrap_or_else(|| "unknown".to_string());
        let data_binding = properties
            .and_then(|item| direct_child(nodes, item, "dataBinding"))
            .map(|item| nodes[item].attributes.clone())
            .unwrap_or_default();
        output.push(WordContentControl {
            control_id: control_id.clone(),
            parent_control_id: parent.map(str::to_string),
            story_part: part.to_string(),
            alias: property_value("alias"),
            tag: property_value("tag"),
            control_type,
            lock: property_value("lock"),
            placeholder: properties
                .and_then(|item| first_descendant(nodes, item, "docPart"))
                .and_then(|item| node_value(&nodes[item])),
            showing_placeholder: properties
                .is_some_and(|item| direct_child(nodes, item, "showingPlcHdr").is_some()),
            data_binding,
            text: source_text(nodes, content),
            properties: properties.map(|item| rich_xml(nodes, item)),
            content: rich_xml(nodes, content),
            locator: rich_locator(part, &node.path, &control_id),
        });
        current_parent = Some(control_id);
    }
    for child in &node.children {
        collect_content_controls(part, nodes, *child, current_parent.as_deref(), output);
    }
}

fn collect_equations(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    in_equation: bool,
    output: &mut Vec<WordEquation>,
    located: &mut Vec<LocatedObject>,
) {
    let node = &nodes[index];
    let is_equation = matches!(local_name(&node.name), "oMath" | "oMathPara");
    if is_equation && !in_equation {
        let equation_id = format!("{part}#equation:{}", node.path);
        output.push(WordEquation {
            equation_id: equation_id.clone(),
            story_part: part.to_string(),
            display: local_name(&node.name) == "oMathPara",
            text: source_text(nodes, index),
            omml: rich_xml(nodes, index),
            locator: rich_locator(part, &node.path, &equation_id),
        });
        located.push(LocatedObject {
            node_index: index,
            object_id: equation_id,
        });
    }
    for child in &node.children {
        collect_equations(
            part,
            nodes,
            *child,
            in_equation || is_equation,
            output,
            located,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_drawings(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    in_object: bool,
    drawings: &mut Vec<WordDrawing>,
    charts: &mut Vec<WordChart>,
    text_boxes: &mut Vec<WordTextBox>,
    embedded_objects: &mut Vec<WordEmbeddedObject>,
    located: &mut Vec<LocatedObject>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let node = &nodes[index];
    let name = local_name(&node.name);
    let is_object = matches!(name, "drawing" | "object") || (name == "pict" && !in_object);
    if is_object {
        let mut all = Vec::new();
        descendants(nodes, index, &mut all);
        let metadata = all
            .iter()
            .copied()
            .find(|candidate| matches!(local_name(&nodes[*candidate].name), "docPr" | "cNvPr"))
            .or_else(|| {
                all.iter()
                    .copied()
                    .find(|candidate| local_name(&nodes[*candidate].name) == "shape")
            });
        let drawing_id = metadata
            .and_then(|item| attribute(&nodes[item], "id"))
            .map(|id| format!("{part}#drawing:{id}"))
            .unwrap_or_else(|| format!("{part}#drawing:{}", node.path));
        let object_relationships = relationships_for_node(relationships, part, nodes, index);
        let drawing_kind = drawing_kind(nodes, index);
        let description = metadata
            .and_then(|item| {
                attribute(&nodes[item], "descr").or_else(|| attribute(&nodes[item], "alt"))
            })
            .map(str::to_string);
        let title = metadata
            .and_then(|item| attribute(&nodes[item], "title"))
            .map(str::to_string);
        drawings.push(WordDrawing {
            drawing_id: drawing_id.clone(),
            story_part: part.to_string(),
            drawing_kind,
            name: metadata
                .and_then(|item| attribute(&nodes[item], "name"))
                .map(str::to_string),
            title: title.clone(),
            description: description.clone(),
            alt_text: description.or(title),
            relationships: object_relationships.clone(),
            content: rich_xml(nodes, index),
            locator: rich_locator(part, &node.path, &drawing_id),
        });
        located.push(LocatedObject {
            node_index: index,
            object_id: drawing_id.clone(),
        });
        collect_text_boxes(part, nodes, index, &drawing_id, text_boxes);
        collect_embedded_object(
            part,
            nodes,
            index,
            &drawing_id,
            &object_relationships,
            embedded_objects,
        );
        collect_charts(
            entries,
            relationships,
            part,
            nodes,
            index,
            charts,
            diagnostics,
        );
    }
    for child in &node.children {
        collect_drawings(
            entries,
            relationships,
            part,
            nodes,
            *child,
            in_object || is_object,
            drawings,
            charts,
            text_boxes,
            embedded_objects,
            located,
            diagnostics,
        );
    }
}

fn drawing_kind(nodes: &[XmlNode], index: usize) -> WordDrawingKind {
    let mut all = Vec::new();
    descendants(nodes, index, &mut all);
    if all
        .iter()
        .any(|item| local_name(&nodes[*item].name) == "OLEObject")
    {
        WordDrawingKind::OleObject
    } else if all
        .iter()
        .any(|item| local_name(&nodes[*item].name) == "chart")
    {
        WordDrawingKind::Chart
    } else if all
        .iter()
        .any(|item| matches!(local_name(&nodes[*item].name), "txbx" | "txbxContent"))
    {
        WordDrawingKind::TextBox
    } else if all
        .iter()
        .any(|item| local_name(&nodes[*item].name) == "blip")
    {
        WordDrawingKind::Image
    } else if all
        .iter()
        .any(|item| matches!(local_name(&nodes[*item].name), "grpSp" | "group"))
    {
        WordDrawingKind::Group
    } else if all
        .iter()
        .any(|item| matches!(local_name(&nodes[*item].name), "sp" | "shape"))
    {
        WordDrawingKind::Shape
    } else {
        WordDrawingKind::Unknown
    }
}

fn relationships_for_node(
    relationships: &[WordRelationship],
    part: &str,
    nodes: &[XmlNode],
    index: usize,
) -> Vec<WordObjectRelationship> {
    let mut all = vec![index];
    descendants(nodes, index, &mut all);
    let candidate_ids = all
        .iter()
        .flat_map(|item| nodes[*item].attributes.values())
        .cloned()
        .collect::<BTreeSet<_>>();
    relationships
        .iter()
        .filter(|relationship| {
            relationship.source_part.as_deref() == Some(part)
                && candidate_ids.contains(&relationship.id)
        })
        .map(object_relationship)
        .collect()
}

fn object_relationship(relationship: &WordRelationship) -> WordObjectRelationship {
    WordObjectRelationship {
        relationship_id: relationship.id.clone(),
        relationship_type: relationship.relationship_type.clone(),
        target: relationship.target.clone(),
        resolved_part: relationship.resolved_part.clone(),
        external: relationship.target_mode == WordRelationshipTargetMode::External,
        locator: relationship.locator.clone(),
    }
}

fn collect_text_boxes(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    drawing_id: &str,
    output: &mut Vec<WordTextBox>,
) {
    let mut all = Vec::new();
    descendants(nodes, index, &mut all);
    for content in all
        .into_iter()
        .filter(|item| local_name(&nodes[*item].name) == "txbxContent")
    {
        let text_box_id = format!("{drawing_id}#textbox:{}", nodes[content].path);
        output.push(WordTextBox {
            text_box_id: text_box_id.clone(),
            drawing_id: Some(drawing_id.to_string()),
            story_part: part.to_string(),
            text: source_text(nodes, content),
            content: rich_xml(nodes, content),
            locator: rich_locator(part, &nodes[content].path, text_box_id),
        });
    }
}

fn collect_embedded_object(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    drawing_id: &str,
    relationships: &[WordObjectRelationship],
    output: &mut Vec<WordEmbeddedObject>,
) {
    let ole = if local_name(&nodes[index].name) == "OLEObject" {
        Some(index)
    } else {
        first_descendant(nodes, index, "OLEObject")
    };
    let Some(ole) = ole else {
        return;
    };
    let relationship_id = attribute(&nodes[ole], "id").map(str::to_string);
    let relationship = relationship_id
        .as_ref()
        .and_then(|id| {
            relationships
                .iter()
                .find(|item| &item.relationship_id == id)
        })
        .cloned();
    let object_id = format!("{drawing_id}#embedded:{}", nodes[ole].path);
    output.push(WordEmbeddedObject {
        object_id: object_id.clone(),
        story_part: part.to_string(),
        relationship_id,
        child_artifact_part: relationship
            .as_ref()
            .and_then(|item| item.resolved_part.clone()),
        relationship,
        program_id: attribute(&nodes[ole], "ProgID").map(str::to_string),
        object_type: attribute(&nodes[ole], "Type").map(str::to_string),
        draw_aspect: attribute(&nodes[ole], "DrawAspect").map(str::to_string),
        shape_id: attribute(&nodes[ole], "ShapeID").map(str::to_string),
        content: rich_xml(nodes, index),
        locator: rich_locator(part, &nodes[index].path, object_id),
    });
}

#[allow(clippy::too_many_arguments)]
fn collect_charts(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    output: &mut Vec<WordChart>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut all = Vec::new();
    descendants(nodes, index, &mut all);
    for chart_ref in all
        .into_iter()
        .filter(|item| local_name(&nodes[*item].name) == "chart")
    {
        let Some(relationship_id) = attribute(&nodes[chart_ref], "id") else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.chart.missing_relationship",
                    "chart reference has no relationship ID",
                )
                .with_locator(rich_locator(
                    part,
                    &nodes[chart_ref].path,
                    format!("chart-reference:{}", nodes[chart_ref].path),
                ))
                .partial(),
            );
            continue;
        };
        let chart_id = format!("{part}#chart:{relationship_id}");
        if output.iter().any(|chart| chart.chart_id == chart_id) {
            continue;
        }
        let relationship = relationships.iter().find(|relationship| {
            relationship.source_part.as_deref() == Some(part) && relationship.id == relationship_id
        });
        let chart_part = relationship.and_then(|item| item.resolved_part.clone());
        let parsed = chart_part.as_deref().and_then(|chart_part| {
            let bytes = part_bytes(entries, chart_part)?;
            parse_xml_part(chart_part, bytes, diagnostics).ok()
        });
        if relationship.is_none() || chart_part.as_deref().is_some_and(|_| parsed.is_none()) {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.chart.unavailable_part",
                    format!("chart relationship {relationship_id} cannot be resolved"),
                )
                .with_locator(rich_locator(part, &nodes[chart_ref].path, &chart_id))
                .partial(),
            );
        }
        let chart_types = parsed
            .as_ref()
            .map(|chart_nodes| {
                chart_nodes
                    .iter()
                    .map(|node| local_name(&node.name))
                    .filter(|name| name.ends_with("Chart") && *name != "chart")
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();
        let title = parsed.as_ref().and_then(|chart_nodes| {
            chart_nodes
                .iter()
                .position(|node| local_name(&node.name) == "title")
                .map(|title| descendant_text(chart_nodes, title))
                .filter(|title| !title.is_empty())
        });
        let text = parsed
            .as_ref()
            .map(|chart_nodes| source_text(chart_nodes, 0))
            .unwrap_or_default();
        output.push(WordChart {
            chart_id: chart_id.clone(),
            relationship_id: relationship_id.to_string(),
            source_part: part.to_string(),
            chart_part,
            chart_types,
            title,
            text,
            content: parsed.as_ref().map(|chart_nodes| rich_xml(chart_nodes, 0)),
            locator: rich_locator(part, &nodes[chart_ref].path, chart_id.clone()),
        });
    }
}

fn collect_captions(
    part: &str,
    nodes: &[XmlNode],
    located: &[LocatedObject],
    output: &mut Vec<WordCaption>,
) {
    for (index, node) in nodes.iter().enumerate() {
        if local_name(&node.name) != "p" {
            continue;
        }
        let style = direct_child(nodes, index, "pPr")
            .and_then(|properties| direct_child(nodes, properties, "pStyle"))
            .and_then(|style| node_value(&nodes[style]));
        let mut all = Vec::new();
        descendants(nodes, index, &mut all);
        let instruction = all
            .iter()
            .filter(|item| local_name(&nodes[**item].name) == "instrText")
            .map(|item| nodes[*item].text.as_str())
            .collect::<String>();
        let is_caption_style = style
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("caption"));
        let sequence = instruction
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .find(|pair| pair[0].eq_ignore_ascii_case("SEQ"))
            .map(|pair| pair[1].to_string());
        if !is_caption_style && sequence.is_none() {
            continue;
        }
        let target = located
            .iter()
            .filter(|object| object.node_index < index)
            .max_by_key(|object| object.node_index)
            .map(|object| object.object_id.clone());
        let caption_id = format!("{part}#caption:{}", node.path);
        output.push(WordCaption {
            caption_id: caption_id.clone(),
            story_part: part.to_string(),
            text: source_text(nodes, index),
            label: sequence.or(style),
            target_object_id: target.clone(),
            inference_rule: target
                .map(|_| "grist.word_ooxml.nearest-preceding-rich-object.v1".to_string()),
            locator: rich_locator(part, &node.path, caption_id),
        });
    }
}
