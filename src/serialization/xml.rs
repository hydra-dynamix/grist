use super::model::{
    RawStructuredUnknown, StructuredEntry, StructuredScalar, StructuredValue, StructuredValueKind,
};
use crate::core::{LineIndex, LocationComponent, SourceLocator, SourceRange};
use crate::xml::{XmlDocument, XmlNodeKind};
use std::collections::BTreeMap;

pub(crate) struct ProjectedXml {
    pub value: StructuredValue,
    pub raw_unknowns: Vec<RawStructuredUnknown>,
    pub max_depth: usize,
    pub node_count: usize,
}

pub(crate) fn project(document: &XmlDocument) -> ProjectedXml {
    let by_id = document
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut raw_unknowns = Vec::new();
    let items = document
        .root_element_ids
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .map(|index| node(document, index, &by_id, &mut raw_unknowns))
        .collect::<Vec<_>>();
    let lines = LineIndex::new(&document.decoded_text);
    let range = SourceRange::new(0, document.decoded_text.len(), &lines);
    let locator = SourceLocator::exact(range.clone())
        .expect("document range is valid")
        .nested(LocationComponent::XmlPath { path: "/".into() })
        .expect("root XML path is valid");
    let value = if items.len() == 1 {
        items.into_iter().next().expect("one XML root")
    } else {
        StructuredValue {
            id: "xml:/@0".into(),
            kind: StructuredValueKind::Array,
            path: "/".into(),
            range,
            locator,
            raw: document.decoded_text.clone(),
            scalar: None,
            entries: vec![],
            items,
            anchor: None,
            tag: None,
            alias: None,
            alias_target_id: None,
            recovered: !document.well_formed,
        }
    };
    ProjectedXml {
        value,
        raw_unknowns,
        max_depth: document
            .nodes
            .iter()
            .map(|node| node.depth + 1)
            .max()
            .unwrap_or(1),
        node_count: document.nodes.len()
            + document
                .nodes
                .iter()
                .map(|node| node.attributes.len())
                .sum::<usize>(),
    }
}

fn node(
    document: &XmlDocument,
    index: usize,
    by_id: &BTreeMap<&str, usize>,
    raw_unknowns: &mut Vec<RawStructuredUnknown>,
) -> StructuredValue {
    let source = &document.nodes[index];
    let kind = match source.kind {
        XmlNodeKind::Element => StructuredValueKind::XmlElement,
        XmlNodeKind::Text | XmlNodeKind::Cdata | XmlNodeKind::EntityReference => {
            StructuredValueKind::XmlText
        }
        XmlNodeKind::Comment => StructuredValueKind::XmlComment,
        XmlNodeKind::ProcessingInstruction | XmlNodeKind::Doctype => {
            StructuredValueKind::XmlProcessingInstruction
        }
        XmlNodeKind::RawUnknown => StructuredValueKind::RawUnknown,
    };
    let scalar_value = if source.kind == XmlNodeKind::Element {
        None
    } else {
        Some(StructuredScalar::String {
            value: source.text.clone().unwrap_or_else(|| source.raw.clone()),
        })
    };
    let mut entries = Vec::new();
    for attribute in &source.attributes {
        let path = format!("{}/@{}", source.xml_path, attribute.qualified_name);
        let key = scalar(
            format!("xml:{path}:key"),
            path.clone(),
            attribute.name_range.clone(),
            attribute.locator.clone(),
            attribute.qualified_name.clone(),
            attribute.qualified_name.clone(),
        );
        let value_range = attribute
            .value_range
            .clone()
            .unwrap_or_else(|| attribute.range.clone());
        let value = scalar(
            format!("xml:{path}:value"),
            path,
            value_range,
            attribute.locator.clone(),
            attribute.raw_value.clone(),
            attribute.value.clone(),
        );
        entries.push(StructuredEntry {
            index: entries.len(),
            key: Box::new(key),
            value: Box::new(value),
            key_text: Some(format!("@{}", attribute.qualified_name)),
            duplicate_ordinal: 1,
        });
    }
    let items = source
        .children
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .map(|index| node(document, index, by_id, raw_unknowns))
        .collect();
    let value = StructuredValue {
        id: format!("xml:{}", source.id),
        kind,
        path: source.xml_path.clone(),
        range: source.range.clone(),
        locator: source.locator.clone(),
        raw: source.raw.clone(),
        scalar: scalar_value,
        entries,
        items,
        anchor: None,
        tag: source.qualified_name.clone(),
        alias: None,
        alias_target_id: None,
        recovered: source.recovered,
    };
    if source.kind == XmlNodeKind::RawUnknown {
        raw_unknowns.push(RawStructuredUnknown {
            path: source.xml_path.clone(),
            raw: source.raw.clone(),
            reason: "malformed XML retained by the secure XML parser".into(),
            range: source.range.clone(),
            locator: source.locator.clone(),
        });
    }
    value
}

fn scalar(
    id: String,
    path: String,
    range: SourceRange,
    locator: SourceLocator,
    raw: String,
    value: String,
) -> StructuredValue {
    StructuredValue {
        id,
        kind: StructuredValueKind::String,
        path,
        range,
        locator,
        raw,
        scalar: Some(StructuredScalar::String { value }),
        entries: vec![],
        items: vec![],
        anchor: None,
        tag: None,
        alias: None,
        alias_target_id: None,
        recovered: false,
    }
}
