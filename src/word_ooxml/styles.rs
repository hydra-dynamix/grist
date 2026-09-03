//! WordprocessingML styles and inheritance resolution.

use super::archive::PackageEntry;
use super::formatting::{
    child, merge_paragraph_properties, merge_run_formatting, paragraph_properties, run_formatting,
    toggle, value,
};
use super::model::{WordRelationship, WordStyle};
use super::xml_util::{attribute, local_name, parse_xml_part};
use super::{PARSER, xml_locator};
use crate::core::Diagnostic;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn parse_styles(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    main_part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<WordStyle> {
    let Some(part) = related_part(relationships, main_part, "/styles") else {
        return Vec::new();
    };
    parse_styles_part(entries, part, diagnostics)
}

fn parse_styles_part(
    entries: &[PackageEntry],
    part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<WordStyle> {
    let Some(bytes) = entries
        .iter()
        .find(|entry| entry.path == part)
        .and_then(|entry| entry.bytes.as_deref())
    else {
        missing_styles_part(part, diagnostics);
        return Vec::new();
    };
    let Ok(nodes) = parse_xml_part(part, bytes, diagnostics) else {
        return Vec::new();
    };
    parse_styles_xml(part, &nodes, diagnostics)
}

fn parse_styles_xml(
    part: &str,
    nodes: &[super::xml_util::XmlNode],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<WordStyle> {
    let Some(root) = nodes
        .iter()
        .position(|node| local_name(&node.name) == "styles")
    else {
        malformed_styles_part(part, diagnostics);
        return Vec::new();
    };
    let mut styles = nodes[root]
        .children
        .iter()
        .copied()
        .filter(|index| local_name(&nodes[*index].name) == "style")
        .filter_map(|index| parse_style(part, nodes, index, diagnostics))
        .collect::<Vec<_>>();
    resolve_styles(&mut styles, diagnostics);
    styles
}

fn parse_style(
    part: &str,
    nodes: &[super::xml_util::XmlNode],
    index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<WordStyle> {
    let node = &nodes[index];
    let Some(style_id) = attribute(node, "styleId").map(str::to_string) else {
        missing_style_id(part, node, diagnostics);
        return None;
    };
    let child_value = |name: &str| child(nodes, index, name).and_then(|i| value(&nodes[i]));
    Some(WordStyle {
        style_id,
        style_type: attribute(node, "type").unwrap_or("paragraph").to_string(),
        name: child_value("name"),
        based_on: child_value("basedOn"),
        next: child_value("next"),
        linked_style: child_value("link"),
        is_default: attribute(node, "default").is_some_and(parse_bool),
        custom: attribute(node, "customStyle").is_some_and(parse_bool),
        ui_priority: child_value("uiPriority").and_then(|value| value.parse().ok()),
        hidden: child(nodes, index, "hidden").map(|i| toggle(&nodes[i])),
        semi_hidden: child(nodes, index, "semiHidden").map(|i| toggle(&nodes[i])),
        paragraph_properties: paragraph_properties(part, nodes, child(nodes, index, "pPr")),
        run_formatting: run_formatting(part, nodes, child(nodes, index, "rPr")),
        effective_paragraph_properties: Default::default(),
        effective_run_formatting: Default::default(),
        locator: xml_locator(part, &node.path),
    })
}

fn resolve_styles(styles: &mut [WordStyle], diagnostics: &mut Vec<Diagnostic>) {
    let by_id = styles
        .iter()
        .enumerate()
        .map(|(index, style)| (style.style_id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let originals = styles.to_vec();
    for style in &originals {
        if let Some(parent) = &style.based_on
            && !by_id.contains_key(parent)
        {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.styles.missing_base",
                    format!(
                        "style {} is based on missing style {parent}",
                        style.style_id
                    ),
                )
                .with_locator(style.locator.clone())
                .partial(),
            );
        }
    }
    for (index, style) in styles.iter_mut().enumerate() {
        let (paragraph, run) = resolve_style(index, &originals, &by_id, diagnostics);
        style.effective_paragraph_properties = paragraph;
        style.effective_run_formatting = run;
    }
}

fn resolve_style(
    index: usize,
    styles: &[WordStyle],
    by_id: &BTreeMap<String, usize>,
    diagnostics: &mut Vec<Diagnostic>,
) -> (
    super::model::WordParagraphProperties,
    super::model::WordRunFormatting,
) {
    let mut chain = Vec::new();
    let mut visited = BTreeSet::new();
    let mut current = Some(index);
    while let Some(item) = current {
        if !visited.insert(item) {
            style_cycle(&styles[item], diagnostics);
            break;
        }
        chain.push(item);
        current = styles[item]
            .based_on
            .as_ref()
            .and_then(|parent| by_id.get(parent))
            .copied();
    }
    let mut paragraph = Default::default();
    let mut run = Default::default();
    for item in chain.into_iter().rev() {
        paragraph = merge_paragraph_properties(&paragraph, &styles[item].paragraph_properties);
        run = merge_run_formatting(&run, &styles[item].run_formatting);
    }
    (paragraph, run)
}

fn related_part<'a>(
    relationships: &'a [WordRelationship],
    source: &str,
    suffix: &str,
) -> Option<&'a str> {
    relationships
        .iter()
        .find(|relationship| {
            relationship.source_part.as_deref() == Some(source)
                && relationship
                    .relationship_type
                    .to_ascii_lowercase()
                    .ends_with(suffix)
                && relationship.target_exists == Some(true)
        })
        .and_then(|relationship| relationship.resolved_part.as_deref())
}

fn parse_bool(value: &str) -> bool {
    !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "off")
}
fn malformed_styles_part(_part: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(Diagnostic::malformed(PARSER, "styles part has no styles root").partial());
}

fn missing_styles_part(part: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(
        Diagnostic::warning(
            PARSER,
            "word_ooxml.styles.missing_part",
            format!("styles relationship targets unavailable part {part}"),
        )
        .partial(),
    );
}

fn missing_style_id(
    part: &str,
    node: &super::xml_util::XmlNode,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(
        Diagnostic::malformed(PARSER, "Word style has no styleId")
            .with_locator(xml_locator(part, &node.path))
            .partial(),
    );
}

fn style_cycle(style: &WordStyle, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(
        Diagnostic::warning(
            PARSER,
            "word_ooxml.styles.inheritance_cycle",
            format!("style inheritance cycle includes {}", style.style_id),
        )
        .with_locator(style.locator.clone())
        .partial(),
    );
}
