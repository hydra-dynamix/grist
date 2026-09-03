//! Shared WordprocessingML run and paragraph formatting extraction.

use super::model::{WordParagraphProperties, WordRunFormatting, WordXmlProperty};
use super::xml_locator;
use super::xml_util::{XmlNode, attribute, local_name};

pub(super) fn run_formatting(
    part: &str,
    nodes: &[XmlNode],
    properties: Option<usize>,
) -> WordRunFormatting {
    let Some(properties) = properties else {
        return WordRunFormatting::default();
    };
    let mut output = WordRunFormatting::default();
    for child in &nodes[properties].children {
        let node = &nodes[*child];
        let name = local_name(&node.name);
        match name {
            "b" => output.bold = Some(toggle(node)),
            "i" => output.italic = Some(toggle(node)),
            "u" => output.underline = Some(value(node).unwrap_or_else(|| "single".into())),
            "strike" => output.strike = Some(toggle(node)),
            "dstrike" => output.double_strike = Some(toggle(node)),
            "color" => output.color = value(node),
            "highlight" => output.highlight = value(node),
            "sz" | "szCs" => {
                if output.font_size_half_points.is_none() {
                    output.font_size_half_points = value(node).and_then(|value| value.parse().ok());
                }
            }
            "rFonts" => output.fonts.extend(normalized_attributes(node)),
            "lang" => output.language.extend(normalized_attributes(node)),
            "vertAlign" => output.vertical_alignment = value(node),
            "vanish" => output.hidden = Some(toggle(node)),
            "caps" => output.all_caps = Some(toggle(node)),
            "smallCaps" => output.small_caps = Some(toggle(node)),
            _ => {}
        }
        output.properties.push(xml_property(part, node));
    }
    output
}

pub(super) fn paragraph_properties(
    part: &str,
    nodes: &[XmlNode],
    properties: Option<usize>,
) -> WordParagraphProperties {
    let Some(properties) = properties else {
        return WordParagraphProperties::default();
    };
    let mut output = WordParagraphProperties::default();
    for child in &nodes[properties].children {
        let node = &nodes[*child];
        match local_name(&node.name) {
            "jc" => output.alignment = value(node),
            "outlineLvl" => output.outline_level = value(node).and_then(|value| value.parse().ok()),
            "keepNext" => output.keep_next = Some(toggle(node)),
            "keepLines" => output.keep_lines = Some(toggle(node)),
            "pageBreakBefore" => output.page_break_before = Some(toggle(node)),
            "widowControl" => output.widow_control = Some(toggle(node)),
            "contextualSpacing" => output.contextual_spacing = Some(toggle(node)),
            "ind" => output.indentation.extend(normalized_attributes(node)),
            "spacing" => output.spacing.extend(normalized_attributes(node)),
            _ => {}
        }
        output.properties.push(xml_property(part, node));
    }
    output
}

pub(super) fn merge_run_formatting(
    inherited: &WordRunFormatting,
    direct: &WordRunFormatting,
) -> WordRunFormatting {
    let mut output = inherited.clone();
    overlay(&mut output.bold, direct.bold);
    overlay(&mut output.italic, direct.italic);
    overlay(&mut output.underline, direct.underline.clone());
    overlay(&mut output.strike, direct.strike);
    overlay(&mut output.double_strike, direct.double_strike);
    overlay(&mut output.color, direct.color.clone());
    overlay(&mut output.highlight, direct.highlight.clone());
    overlay(
        &mut output.font_size_half_points,
        direct.font_size_half_points,
    );
    output.fonts.extend(direct.fonts.clone());
    output.language.extend(direct.language.clone());
    overlay(
        &mut output.vertical_alignment,
        direct.vertical_alignment.clone(),
    );
    overlay(&mut output.hidden, direct.hidden);
    overlay(&mut output.all_caps, direct.all_caps);
    overlay(&mut output.small_caps, direct.small_caps);
    output.properties.extend(direct.properties.clone());
    output
}

pub(super) fn merge_paragraph_properties(
    inherited: &WordParagraphProperties,
    direct: &WordParagraphProperties,
) -> WordParagraphProperties {
    let mut output = inherited.clone();
    overlay(&mut output.alignment, direct.alignment.clone());
    overlay(&mut output.outline_level, direct.outline_level);
    overlay(&mut output.keep_next, direct.keep_next);
    overlay(&mut output.keep_lines, direct.keep_lines);
    overlay(&mut output.page_break_before, direct.page_break_before);
    overlay(&mut output.widow_control, direct.widow_control);
    overlay(&mut output.contextual_spacing, direct.contextual_spacing);
    output.indentation.extend(direct.indentation.clone());
    output.spacing.extend(direct.spacing.clone());
    output.properties.extend(direct.properties.clone());
    output
}

pub(super) fn child(nodes: &[XmlNode], parent: usize, name: &str) -> Option<usize> {
    nodes[parent]
        .children
        .iter()
        .copied()
        .find(|child| local_name(&nodes[*child].name) == name)
}

pub(super) fn children<'a>(
    nodes: &'a [XmlNode],
    parent: usize,
    name: &'a str,
) -> impl Iterator<Item = usize> + 'a {
    nodes[parent]
        .children
        .iter()
        .copied()
        .filter(move |child| local_name(&nodes[*child].name) == name)
}

pub(super) fn value(node: &XmlNode) -> Option<String> {
    attribute(node, "val")
        .map(str::to_string)
        .or_else(|| (!node.text.is_empty()).then(|| node.text.clone()))
}

pub(super) fn toggle(node: &XmlNode) -> bool {
    !matches!(
        attribute(node, "val")
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("0" | "false" | "off" | "none")
    )
}

pub(super) fn normalized_attributes(node: &XmlNode) -> std::collections::BTreeMap<String, String> {
    node.attributes
        .iter()
        .map(|(key, value)| (local_name(key).to_string(), value.clone()))
        .collect()
}

fn xml_property(part: &str, node: &XmlNode) -> WordXmlProperty {
    WordXmlProperty {
        name: local_name(&node.name).to_string(),
        value: value(node),
        attributes: normalized_attributes(node),
        locator: xml_locator(part, &node.path),
    }
}

fn overlay<T>(target: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *target = value;
    }
}
