//! Original WordprocessingML numbering definitions and deterministic list labels.

use super::archive::PackageEntry;
use super::formatting::{child, paragraph_properties, run_formatting, value};
use super::model::*;
use super::xml_util::{XmlNode, attribute, local_name, parse_xml_part};
use super::{PARSER, xml_locator};
use crate::core::{Diagnostic, SourceLocator};
use std::collections::BTreeMap;

pub(super) fn parse_numbering(
    entries: &[PackageEntry],
    relationships: &[WordRelationship],
    main_part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> WordNumbering {
    let Some(part) = relationships
        .iter()
        .find(|relationship| {
            relationship.source_part.as_deref() == Some(main_part)
                && relationship
                    .relationship_type
                    .to_ascii_lowercase()
                    .ends_with("/numbering")
                && relationship.target_exists == Some(true)
        })
        .and_then(|relationship| relationship.resolved_part.as_deref())
    else {
        return WordNumbering::default();
    };
    let Some(bytes) = entries
        .iter()
        .find(|entry| entry.path == part)
        .and_then(|entry| entry.bytes.as_deref())
    else {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "word_ooxml.numbering.missing_part",
                format!("numbering relationship targets unavailable part {part}"),
            )
            .partial(),
        );
        return WordNumbering::default();
    };
    let Ok(nodes) = parse_xml_part(part, bytes, diagnostics) else {
        return WordNumbering::default();
    };
    let Some(root) = nodes
        .iter()
        .position(|node| local_name(&node.name) == "numbering")
    else {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                format!("numbering part {part} has no numbering root"),
            )
            .partial(),
        );
        return WordNumbering::default();
    };
    WordNumbering {
        abstract_definitions: nodes[root]
            .children
            .iter()
            .copied()
            .filter(|index| local_name(&nodes[*index].name) == "abstractNum")
            .filter_map(|index| parse_abstract(part, &nodes, index, diagnostics))
            .collect(),
        instances: nodes[root]
            .children
            .iter()
            .copied()
            .filter(|index| local_name(&nodes[*index].name) == "num")
            .filter_map(|index| parse_instance(part, &nodes, index, diagnostics))
            .collect(),
    }
}

fn parse_abstract(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<WordAbstractNumbering> {
    let node = &nodes[index];
    let Some(abstract_num_id) =
        attribute(node, "abstractNumId").and_then(|value| value.parse().ok())
    else {
        diagnostics.push(
            Diagnostic::malformed(PARSER, "abstract numbering definition has no numeric ID")
                .with_locator(xml_locator(part, &node.path))
                .partial(),
        );
        return None;
    };
    let child_value = |name: &str| child(nodes, index, name).and_then(|item| value(&nodes[item]));
    Some(WordAbstractNumbering {
        abstract_num_id,
        multi_level_type: child_value("multiLevelType"),
        number_style_link: child_value("numStyleLink"),
        style_link: child_value("styleLink"),
        levels: nodes[index]
            .children
            .iter()
            .copied()
            .filter(|item| local_name(&nodes[*item].name) == "lvl")
            .filter_map(|item| parse_level(part, nodes, item, diagnostics))
            .collect(),
        locator: xml_locator(part, &node.path),
    })
}

fn parse_level(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<WordNumberingLevel> {
    let node = &nodes[index];
    let Some(level) = attribute(node, "ilvl").and_then(|value| value.parse().ok()) else {
        diagnostics.push(
            Diagnostic::malformed(PARSER, "numbering level has no numeric ilvl")
                .with_locator(xml_locator(part, &node.path))
                .partial(),
        );
        return None;
    };
    let child_value = |name: &str| child(nodes, index, name).and_then(|item| value(&nodes[item]));
    Some(WordNumberingLevel {
        level,
        start: child_value("start")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1),
        number_format: child_value("numFmt").unwrap_or_else(|| "decimal".into()),
        level_text: child_value("lvlText").unwrap_or_else(|| format!("%{}", level + 1)),
        suffix: child_value("suff"),
        alignment: child_value("lvlJc"),
        paragraph_style: child_value("pStyle"),
        restart_after_level: child_value("lvlRestart").and_then(|value| value.parse().ok()),
        picture_bullet_id: child_value("lvlPicBulletId").and_then(|value| value.parse().ok()),
        paragraph_properties: paragraph_properties(part, nodes, child(nodes, index, "pPr")),
        run_formatting: run_formatting(part, nodes, child(nodes, index, "rPr")),
        locator: xml_locator(part, &node.path),
    })
}

fn parse_instance(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<WordNumberingInstance> {
    let node = &nodes[index];
    let num_id = attribute(node, "numId").and_then(|value| value.parse().ok())?;
    let abstract_num_id = child(nodes, index, "abstractNumId")
        .and_then(|item| value(&nodes[item]))
        .and_then(|value| value.parse().ok());
    let Some(abstract_num_id) = abstract_num_id else {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                format!("numbering instance {num_id} has no abstractNumId"),
            )
            .with_locator(xml_locator(part, &node.path))
            .partial(),
        );
        return None;
    };
    Some(WordNumberingInstance {
        num_id,
        abstract_num_id,
        overrides: nodes[index]
            .children
            .iter()
            .copied()
            .filter(|item| local_name(&nodes[*item].name) == "lvlOverride")
            .filter_map(|item| parse_override(part, nodes, item, diagnostics))
            .collect(),
        locator: xml_locator(part, &node.path),
    })
}

fn parse_override(
    part: &str,
    nodes: &[XmlNode],
    index: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<WordNumberingOverride> {
    let node = &nodes[index];
    let level = attribute(node, "ilvl").and_then(|value| value.parse().ok())?;
    Some(WordNumberingOverride {
        level,
        start_override: child(nodes, index, "startOverride")
            .and_then(|item| value(&nodes[item]))
            .and_then(|value| value.parse().ok()),
        level_definition: child(nodes, index, "lvl")
            .and_then(|item| parse_level(part, nodes, item, diagnostics)),
        locator: xml_locator(part, &node.path),
    })
}

pub(super) struct NumberingResolver<'a> {
    numbering: &'a WordNumbering,
    counters: BTreeMap<i64, Vec<Option<i64>>>,
}

impl<'a> NumberingResolver<'a> {
    pub(super) fn new(numbering: &'a WordNumbering) -> Self {
        Self {
            numbering,
            counters: BTreeMap::new(),
        }
    }

    pub(super) fn resolve(
        &mut self,
        num_id: i64,
        level: u8,
        locator: SourceLocator,
    ) -> Option<WordResolvedNumbering> {
        let (abstract_num_id, definition, start) = self.definition(num_id, level)?;
        let counters = self.counters.entry(num_id).or_insert_with(|| vec![None; 9]);
        let level_index = usize::from(level);
        if counters.len() <= level_index {
            counters.resize(level_index + 1, None);
        }
        counters
            .iter_mut()
            .skip(level_index + 1)
            .for_each(|value| *value = None);
        let ordinal = counters[level_index].map_or(start, |value| value + 1);
        counters[level_index] = Some(ordinal);
        let label = render_label(self.numbering, num_id, &definition.level_text, counters);
        Some(WordResolvedNumbering {
            num_id,
            abstract_num_id,
            level,
            ordinal,
            label,
            ordered: !matches!(definition.number_format.as_str(), "bullet" | "none"),
            number_format: definition.number_format,
            level_text: definition.level_text,
            definition_locator: definition.locator,
            locator,
        })
    }

    fn definition(&self, num_id: i64, level: u8) -> Option<(i64, WordNumberingLevel, i64)> {
        let instance = self
            .numbering
            .instances
            .iter()
            .find(|item| item.num_id == num_id)?;
        let abstract_definition = self
            .numbering
            .abstract_definitions
            .iter()
            .find(|item| item.abstract_num_id == instance.abstract_num_id)?;
        let override_definition = instance.overrides.iter().find(|item| item.level == level);
        let definition = override_definition
            .and_then(|item| item.level_definition.clone())
            .or_else(|| {
                abstract_definition
                    .levels
                    .iter()
                    .find(|item| item.level == level)
                    .cloned()
            })?;
        let start = override_definition
            .and_then(|item| item.start_override)
            .unwrap_or(definition.start);
        Some((instance.abstract_num_id, definition, start))
    }
}

fn render_label(
    numbering: &WordNumbering,
    num_id: i64,
    template: &str,
    counters: &[Option<i64>],
) -> String {
    let instance = numbering
        .instances
        .iter()
        .find(|item| item.num_id == num_id);
    let abstract_definition = instance.and_then(|instance| {
        numbering
            .abstract_definitions
            .iter()
            .find(|item| item.abstract_num_id == instance.abstract_num_id)
    });
    let mut label = template.to_string();
    for level in 0..9 {
        let Some(value) = counters.get(level).copied().flatten() else {
            continue;
        };
        let format = abstract_definition
            .and_then(|item| {
                item.levels
                    .iter()
                    .find(|item| usize::from(item.level) == level)
            })
            .map(|item| item.number_format.as_str())
            .unwrap_or("decimal");
        label = label.replace(&format!("%{}", level + 1), &format_number(value, format));
    }
    label
}

fn format_number(value: i64, format: &str) -> String {
    match format {
        "lowerLetter" => alphabetic(value, false),
        "upperLetter" => alphabetic(value, true),
        "lowerRoman" => roman(value).to_ascii_lowercase(),
        "upperRoman" => roman(value),
        "ordinal" => format!("{value}{}", ordinal_suffix(value)),
        "bullet" | "none" => String::new(),
        _ => value.to_string(),
    }
}

fn alphabetic(mut value: i64, uppercase: bool) -> String {
    if value <= 0 {
        return value.to_string();
    }
    let mut output = String::new();
    while value > 0 {
        value -= 1;
        let base = if uppercase { b'A' } else { b'a' };
        output.insert(0, char::from(base + u8::try_from(value % 26).unwrap_or(0)));
        value /= 26;
    }
    output
}

fn roman(mut value: i64) -> String {
    if !(1..=3999).contains(&value) {
        return value.to_string();
    }
    let mut output = String::new();
    for (number, token) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while value >= number {
            output.push_str(token);
            value -= number;
        }
    }
    output
}

fn ordinal_suffix(value: i64) -> &'static str {
    if (11..=13).contains(&(value.rem_euclid(100))) {
        "th"
    } else {
        match value.rem_euclid(10) {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    }
}
