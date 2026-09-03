//! Inert OOXML XML tree reader shared by package metadata parts.

use super::{PARSER, part_locator};
use crate::core::Diagnostic;
use crate::security::{XmlSecurityPolicy, inspect_xml};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone)]
pub(super) struct XmlNode {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<usize>,
    pub path: String,
}

pub(super) fn parse_xml_part(
    part: &str,
    bytes: &[u8],
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<XmlNode>, ()> {
    let findings = inspect_xml(bytes, &XmlSecurityPolicy::default());
    if !findings.is_empty() {
        for finding in findings {
            diagnostics.push(
                Diagnostic::warning(PARSER, finding.code, finding.message)
                    .with_locator(part_locator(part).expect("validated package part"))
                    .partial(),
            );
        }
        return Err(());
    }

    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut nodes = Vec::<XmlNode>::new();
    let mut stack = Vec::<usize>::new();
    let mut sibling_counts = Vec::<HashMap<String, usize>>::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if push_node(&start, false, &mut nodes, &mut stack, &mut sibling_counts).is_err() {
                    malformed_attributes(part, diagnostics);
                    return Err(());
                }
            }
            Ok(Event::Empty(start)) => {
                if push_node(&start, true, &mut nodes, &mut stack, &mut sibling_counts).is_err() {
                    malformed_attributes(part, diagnostics);
                    return Err(());
                }
            }
            Ok(Event::End(_)) => {
                stack.pop();
                sibling_counts.pop();
            }
            Ok(Event::Text(value)) => {
                if let Some(index) = stack.last().copied() {
                    nodes[index]
                        .text
                        .push_str(&decode_xml(&String::from_utf8_lossy(value.as_ref())));
                }
            }
            Ok(Event::CData(value)) => {
                if let Some(index) = stack.last().copied() {
                    nodes[index]
                        .text
                        .push_str(&String::from_utf8_lossy(value.as_ref()));
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                diagnostics.push(
                    Diagnostic::malformed(
                        PARSER,
                        format!("malformed XML in OOXML part {part}: {error}"),
                    )
                    .with_locator(part_locator(part).expect("validated package part"))
                    .partial(),
                );
                return Err(());
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                format!("unclosed XML elements in OOXML part {part}"),
            )
            .with_locator(part_locator(part).expect("validated package part"))
            .partial(),
        );
        return Err(());
    }
    Ok(nodes)
}

fn malformed_attributes(part: &str, diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.push(
        Diagnostic::malformed(
            PARSER,
            format!("malformed XML attributes in OOXML part {part}"),
        )
        .with_locator(part_locator(part).expect("validated package part"))
        .partial(),
    );
}

fn push_node(
    start: &BytesStart<'_>,
    empty: bool,
    nodes: &mut Vec<XmlNode>,
    stack: &mut Vec<usize>,
    sibling_counts: &mut Vec<HashMap<String, usize>>,
) -> Result<(), ()> {
    let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
    let count = if let Some(counts) = sibling_counts.last_mut() {
        let count = counts.entry(name.clone()).or_default();
        *count += 1;
        *count
    } else {
        1
    };
    let path = stack.last().map_or_else(
        || format!("/{name}[{count}]"),
        |parent| format!("{}/{name}[{count}]", nodes[*parent].path),
    );
    let attributes = attributes(start)?;
    let index = nodes.len();
    nodes.push(XmlNode {
        name,
        attributes,
        text: String::new(),
        children: Vec::new(),
        path,
    });
    if let Some(parent) = stack.last().copied() {
        nodes[parent].children.push(index);
    }
    if !empty {
        stack.push(index);
        sibling_counts.push(HashMap::new());
    }
    Ok(())
}

fn attributes(start: &BytesStart<'_>) -> Result<BTreeMap<String, String>, ()> {
    let mut output = BTreeMap::new();
    for attribute in start.attributes().with_checks(false) {
        let attribute = attribute.map_err(|_| ())?;
        output.insert(
            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
            decode_xml(&String::from_utf8_lossy(attribute.value.as_ref())),
        );
    }
    Ok(output)
}

pub(super) fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub(super) fn attribute<'a>(node: &'a XmlNode, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name) || local_name(key) == name)
        .map(|(_, value)| value.as_str())
}

pub(super) fn descendant_text(nodes: &[XmlNode], index: usize) -> String {
    let mut output = nodes[index].text.clone();
    for child in &nodes[index].children {
        output.push_str(&descendant_text(nodes, *child));
    }
    output
}

fn decode_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
