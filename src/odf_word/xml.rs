//! Inert XML reader with exact package-member and byte locators.

use super::archive::PackageEntry;
use super::{PARSER, member_locator};
use crate::core::{Diagnostic, LineIndex, LocationComponent, SourceLocator, SourceRange};
use crate::registry::{ParserContext, ParserError};
use crate::security::{XmlSecurityPolicy, inspect_xml};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone)]
pub(super) enum XmlContent {
    Text {
        value: String,
        start: usize,
        end: usize,
    },
    Child(usize),
}

#[derive(Debug, Clone)]
pub(super) struct XmlElement {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub content: Vec<XmlContent>,
    pub start: usize,
    pub end: usize,
    pub path: String,
}

pub(super) struct XmlDocument {
    pub nodes: Vec<XmlElement>,
    pub text: String,
    pub line_index: LineIndex,
}

pub(super) fn observe_xml_nesting(
    entries: &[PackageEntry],
    context: &ParserContext<'_>,
) -> Result<(), ParserError> {
    for entry in entries.iter().filter(|entry| {
        entry.path.ends_with(".xml") && entry.bytes.is_some() && entry.rejected.is_none()
    }) {
        let bytes = entry.bytes.as_deref().expect("filtered XML bytes");
        let mut reader = Reader::from_reader(bytes);
        let mut depth = 0u64;
        let mut maximum = 0u64;
        let mut events = 0u64;
        loop {
            match reader.read_event() {
                Ok(Event::Start(_)) => {
                    depth = depth.saturating_add(1);
                    maximum = maximum.max(depth);
                }
                Ok(Event::End(_)) => depth = depth.saturating_sub(1),
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            events = events.saturating_add(1);
            if events % 256 == 0 {
                context.checkpoint()?;
            }
        }
        context.observe_nesting_depth(maximum)?;
    }
    Ok(())
}

pub(super) fn parse_xml(
    entry: &PackageEntry,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<XmlDocument> {
    let bytes = entry.bytes.as_deref()?;
    let findings = inspect_xml(bytes, &XmlSecurityPolicy::default());
    if !findings.is_empty() {
        for finding in findings {
            diagnostics.push(
                Diagnostic::warning(PARSER, finding.code, finding.message)
                    .with_locator(member_locator(entry).expect("validated package member"))
                    .partial(),
            );
        }
        return None;
    }
    let text = String::from_utf8_lossy(bytes).into_owned();
    if std::str::from_utf8(bytes).is_err() {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                format!(
                    "XML member {} is not valid UTF-8; replacement text was retained",
                    entry.path
                ),
            )
            .with_locator(member_locator(entry).expect("validated package member"))
            .partial(),
        );
    }
    let line_index = LineIndex::new(&text);
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut nodes = Vec::<XmlElement>::new();
    let mut stack = Vec::<usize>::new();
    let mut sibling_counts = Vec::<HashMap<String, usize>>::new();
    let mut previous = 0usize;
    loop {
        let event = reader.read_event();
        let end = usize::try_from(reader.buffer_position())
            .unwrap_or(bytes.len())
            .min(bytes.len());
        match event {
            Ok(Event::Start(start)) => {
                if push_node(
                    &start,
                    false,
                    previous,
                    end,
                    &mut nodes,
                    &mut stack,
                    &mut sibling_counts,
                )
                .is_err()
                {
                    malformed_attributes(entry, previous, end, &line_index, diagnostics);
                }
            }
            Ok(Event::Empty(start)) => {
                if push_node(
                    &start,
                    true,
                    previous,
                    end,
                    &mut nodes,
                    &mut stack,
                    &mut sibling_counts,
                )
                .is_err()
                {
                    malformed_attributes(entry, previous, end, &line_index, diagnostics);
                }
            }
            Ok(Event::End(_)) => {
                if let Some(index) = stack.pop() {
                    nodes[index].end = end;
                }
                sibling_counts.pop();
            }
            Ok(Event::Text(value)) => {
                if let Some(index) = stack.last().copied() {
                    nodes[index].content.push(XmlContent::Text {
                        value: decode_xml(&String::from_utf8_lossy(value.as_ref())),
                        start: previous,
                        end,
                    });
                }
            }
            Ok(Event::CData(value)) => {
                if let Some(index) = stack.last().copied() {
                    nodes[index].content.push(XmlContent::Text {
                        value: String::from_utf8_lossy(value.as_ref()).into_owned(),
                        start: previous,
                        end,
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                diagnostics.push(
                    Diagnostic::malformed(
                        PARSER,
                        format!("malformed XML in {}: {error}", entry.path),
                    )
                    .with_locator(
                        range_locator(entry, previous, end.max(previous), &line_index)
                            .expect("valid XML error locator"),
                    )
                    .partial(),
                );
                return None;
            }
            _ => {}
        }
        previous = end;
    }
    if !stack.is_empty() {
        diagnostics.push(
            Diagnostic::malformed(PARSER, format!("unclosed XML elements in {}", entry.path))
                .with_locator(member_locator(entry).expect("validated package member"))
                .partial(),
        );
        return None;
    }
    Some(XmlDocument {
        nodes,
        text,
        line_index,
    })
}

fn push_node(
    start: &BytesStart<'_>,
    empty: bool,
    begin: usize,
    end: usize,
    nodes: &mut Vec<XmlElement>,
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
    let parent = stack.last().copied();
    let path = parent.map_or_else(
        || format!("/{name}[{count}]"),
        |parent| format!("{}/{name}[{count}]", nodes[parent].path),
    );
    let index = nodes.len();
    nodes.push(XmlElement {
        name,
        attributes: attributes(start)?,
        content: Vec::new(),
        start: begin,
        end,
        path,
    });
    if let Some(parent) = parent {
        nodes[parent].content.push(XmlContent::Child(index));
    }
    if !empty {
        stack.push(index);
        sibling_counts.push(HashMap::new());
    }
    Ok(())
}

fn attributes(start: &BytesStart<'_>) -> Result<BTreeMap<String, String>, ()> {
    let mut output = BTreeMap::new();
    for attribute in start.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| ())?;
        output.insert(
            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
            decode_xml(&String::from_utf8_lossy(attribute.value.as_ref())),
        );
    }
    Ok(output)
}

fn malformed_attributes(
    entry: &PackageEntry,
    start: usize,
    end: usize,
    index: &LineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(
        Diagnostic::malformed(
            PARSER,
            format!("malformed XML attributes in {}", entry.path),
        )
        .with_locator(
            range_locator(entry, start, end.max(start), index)
                .expect("valid XML attribute locator"),
        )
        .partial(),
    );
}

pub(super) fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub(super) fn attr<'a>(node: &'a XmlElement, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(key, _)| {
            key.eq_ignore_ascii_case(name) || local_name(key).eq_ignore_ascii_case(name)
        })
        .map(|(_, value)| value.as_str())
}

pub(super) fn descendant_text(document: &XmlDocument, index: usize) -> String {
    let mut output = String::new();
    for content in &document.nodes[index].content {
        match content {
            XmlContent::Text { value, .. } => output.push_str(value),
            XmlContent::Child(child) => output.push_str(&descendant_text(document, *child)),
        }
    }
    output
}

pub(super) fn element_locator(
    entry: &PackageEntry,
    document: &XmlDocument,
    node: &XmlElement,
) -> SourceLocator {
    range_locator(
        entry,
        node.start,
        node.end.max(node.start),
        &document.line_index,
    )
    .and_then(|locator| {
        locator.nested(LocationComponent::XmlPath {
            path: node.path.clone(),
        })
    })
    .expect("valid package XML locator")
}

pub(super) fn text_locator(
    entry: &PackageEntry,
    document: &XmlDocument,
    start: usize,
    end: usize,
) -> SourceLocator {
    range_locator(entry, start, end.max(start), &document.line_index)
        .expect("valid package XML text locator")
}

fn range_locator(
    entry: &PackageEntry,
    start: usize,
    end: usize,
    index: &LineIndex,
) -> Result<SourceLocator, crate::core::SourceLocatorError> {
    member_locator(entry)?.nested(LocationComponent::from(SourceRange::new(start, end, index)))
}

pub(super) fn raw_xml<'a>(document: &'a XmlDocument, node: &XmlElement) -> &'a str {
    document
        .text
        .get(node.start.min(document.text.len())..node.end.min(document.text.len()))
        .unwrap_or_default()
}

fn decode_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", r#"""#)
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
