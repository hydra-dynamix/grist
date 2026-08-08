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
        diagnostics.extend(findings.into_iter().map(|finding| {
            Diagnostic::warning(PARSER, finding.code, finding.message)
                .with_locator(part_locator(part))
                .partial()
        }));
        return Err(());
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut nodes = Vec::new();
    let mut stack = Vec::<usize>::new();
    let mut siblings = Vec::<HashMap<String, usize>>::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                push_node(&start, false, &mut nodes, &mut stack, &mut siblings).map_err(|_| ())?
            }
            Ok(Event::Empty(start)) => {
                push_node(&start, true, &mut nodes, &mut stack, &mut siblings).map_err(|_| ())?
            }
            Ok(Event::End(_)) => {
                stack.pop();
                siblings.pop();
            }
            Ok(Event::Text(text)) => {
                if let Some(index) = stack.last() {
                    nodes[*index]
                        .text
                        .push_str(&decode_xml(&String::from_utf8_lossy(text.as_ref())));
                }
            }
            Ok(Event::CData(text)) => {
                if let Some(index) = stack.last() {
                    nodes[*index]
                        .text
                        .push_str(&String::from_utf8_lossy(text.as_ref()));
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                diagnostics.push(
                    Diagnostic::malformed(
                        PARSER,
                        format!("malformed XML in OOXML part {part}: {error}"),
                    )
                    .with_locator(part_locator(part))
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
            .with_locator(part_locator(part))
            .partial(),
        );
        return Err(());
    }
    Ok(nodes)
}

fn push_node(
    start: &BytesStart<'_>,
    empty: bool,
    nodes: &mut Vec<XmlNode>,
    stack: &mut Vec<usize>,
    siblings: &mut Vec<HashMap<String, usize>>,
) -> Result<(), quick_xml::Error> {
    let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
    let count = siblings.last_mut().map_or(1, |values| {
        let count = values.entry(name.clone()).or_default();
        *count += 1;
        *count
    });
    let path = stack.last().map_or_else(
        || format!("/{name}[{count}]"),
        |parent| format!("{}/{name}[{count}]", nodes[*parent].path),
    );
    let mut attributes = BTreeMap::new();
    for item in start.attributes().with_checks(false) {
        let item = item.map_err(quick_xml::Error::InvalidAttr)?;
        attributes.insert(
            String::from_utf8_lossy(item.key.as_ref()).into_owned(),
            decode_xml(&String::from_utf8_lossy(item.value.as_ref())),
        );
    }
    let index = nodes.len();
    nodes.push(XmlNode {
        name,
        attributes,
        text: String::new(),
        children: Vec::new(),
        path,
    });
    if let Some(parent) = stack.last() {
        nodes[*parent].children.push(index);
    }
    if !empty {
        stack.push(index);
        siblings.push(HashMap::new());
    }
    Ok(())
}

pub(super) fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub(super) fn attr<'a>(node: &'a XmlNode, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(key, _)| local_name(key) == name)
        .map(|(_, value)| value.as_str())
}

pub(super) fn descendants(
    nodes: &[XmlNode],
    index: usize,
    name: &str,
) -> std::vec::IntoIter<usize> {
    let mut output = Vec::new();
    collect_descendants(nodes, index, name, &mut output);
    output.into_iter()
}

fn collect_descendants(nodes: &[XmlNode], index: usize, name: &str, output: &mut Vec<usize>) {
    for child in &nodes[index].children {
        if local_name(&nodes[*child].name) == name {
            output.push(*child);
        }
        collect_descendants(nodes, *child, name, output);
    }
}

pub(super) fn descendant_text(nodes: &[XmlNode], index: usize) -> String {
    let mut text = nodes[index].text.clone();
    for child in &nodes[index].children {
        text.push_str(&descendant_text(nodes, *child));
    }
    text
}

pub(super) fn parse_bool(value: Option<&str>) -> Option<bool> {
    value.map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "on"))
}

fn decode_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}
