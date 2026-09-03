use super::*;
use crate::core::SchemaVersion;
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, ProjectionAddress, ToDocumentGraph, TransformError,
};
use serde_json::json;
use std::collections::HashMap;

impl ToDocumentGraph for SpreadsheetOdfDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::SPREADSHEET_ODF_V1, "spreadsheet_odf")
            .map_err(transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::SpreadsheetOdf)
            .with_projection(
                "spreadsheet_odf",
                SchemaVersion::SPREADSHEET_ODF_V1,
                "grist.spreadsheet_odf.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = context.language;
        graph.dialect = Some(
            context
                .dialect
                .unwrap_or_else(|| self.package_kind.format_id().into()),
        );
        graph.attrs = context.attrs;
        let make_id = |path: Vec<String>,
                       native_id: Option<String>,
                       locator: Option<crate::core::SourceLocator>| {
            identities
                .node_id(&ProjectionAddress {
                    structural_path: path,
                    native_id,
                    locator,
                })
                .map_err(transform_error)
        };
        let root_id = make_id(
            vec!["workbook".into()],
            Some("content.xml".into()),
            Some(self.workbook_locator.clone()),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name("content.xml")
            .with_locator(self.workbook_locator.clone())
            .with_ordinal(0);
        root.extensions.insert("grist.spreadsheet_odf".into(), json!({"package_kind": self.package_kind, "version": self.version, "calculation": self.calculation, "active_table": self.active_table, "panes": self.panes}));
        graph.add_node(root);
        let mut cell_ids = HashMap::<(String, String), String>::new();
        for sheet in &self.sheets {
            let sheet_id = make_id(
                vec!["sheets".into(), sheet.order.to_string()],
                Some(format!("sheet:{}", sheet.name)),
                Some(sheet.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&sheet_id, DocumentNodeKind::Sheet)
                .with_name(&sheet.name)
                .with_locator(sheet.locator.clone())
                .with_ordinal(sheet.order)
                .with_attr(
                    "visibility",
                    serde_json::to_value(sheet.visibility).map_err(transform_error)?,
                );
            node.extensions.insert("grist.spreadsheet_odf".into(), json!({"style_name": sheet.style_name, "protected": sheet.protected, "print_ranges": sheet.print_ranges, "columns": sheet.columns, "merges": sheet.merges}));
            graph.add_node(node);
            graph.add_contains(&root_id, &sheet_id);
            for row in &sheet.rows {
                let row_id = make_id(
                    vec![
                        "sheets".into(),
                        sheet.order.to_string(),
                        "rows".into(),
                        row.row.to_string(),
                    ],
                    None,
                    Some(row.locator.clone()),
                )?;
                let mut row_node = DocumentNode::new(&row_id, DocumentNodeKind::Row)
                    .with_locator(row.locator.clone())
                    .with_ordinal(row.row as usize)
                    .with_attr("repeated", row.repeated);
                row_node.extensions.insert(
                    "grist.spreadsheet_odf".into(),
                    serde_json::to_value(row).map_err(transform_error)?,
                );
                graph.add_node(row_node);
                graph.add_contains(&sheet_id, &row_id);
                for cell in &row.cells {
                    let cell_id = make_id(
                        vec![
                            "sheets".into(),
                            sheet.order.to_string(),
                            "cells".into(),
                            cell.reference.clone(),
                        ],
                        Some(format!("cell:{}!{}", sheet.name, cell.reference)),
                        Some(cell.locator.clone()),
                    )?;
                    let mut cell_node = DocumentNode::new(&cell_id, DocumentNodeKind::Cell)
                        .with_name(&cell.reference)
                        .with_locator(cell.locator.clone())
                        .with_ordinal(cell.column as usize);
                    cell_node.text = cell
                        .displayed_value
                        .clone()
                        .or_else(|| cell.stored_value.clone());
                    cell_node.extensions.insert(
                        "grist.spreadsheet_odf".into(),
                        serde_json::to_value(cell).map_err(transform_error)?,
                    );
                    graph.add_node(cell_node);
                    graph.add_contains(&row_id, &cell_id);
                    cell_ids.insert((sheet.name.clone(), cell.reference.clone()), cell_id);
                }
            }
            project_sheet_items(sheet, &sheet_id, &make_id, &mut graph)?;
        }
        for sheet in &self.sheets {
            for cell in sheet.rows.iter().flat_map(|row| &row.cells) {
                let (Some(formula), Some(source)) = (
                    &cell.formula,
                    cell_ids.get(&(sheet.name.clone(), cell.reference.clone())),
                ) else {
                    continue;
                };
                for reference in formula_references(&formula.source) {
                    if let Some(target) = cell_ids.get(&(sheet.name.clone(), reference)) {
                        graph.add_edge(DocumentEdge::explicit(
                            source.clone(),
                            DocumentRelation::FormulaDependsOn,
                            target.clone(),
                            formula.locator.clone(),
                        ));
                    }
                }
            }
        }
        for (index, range) in self.named_ranges.iter().enumerate() {
            let id = make_id(
                vec!["named_ranges".into(), index.to_string()],
                Some(format!("named-range:{}", range.name)),
                Some(range.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::StructuredValue)
                .with_name(&range.name)
                .with_text(&range.expression)
                .with_locator(range.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.spreadsheet_odf".into(),
                serde_json::to_value(range).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
        }
        for (index, metadata) in self.metadata.iter().enumerate() {
            let id = make_id(
                vec!["metadata".into(), index.to_string()],
                None,
                Some(metadata.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Metadata)
                .with_name(&metadata.name)
                .with_text(&metadata.value)
                .with_locator(metadata.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.spreadsheet_odf".into(),
                serde_json::to_value(metadata).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
        }
        for (index, raw) in self.raw_elements.iter().enumerate() {
            let id = make_id(
                vec!["raw_elements".into(), index.to_string()],
                None,
                Some(raw.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Raw)
                .with_name(&raw.name)
                .with_text(&raw.raw_xml)
                .with_locator(raw.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.spreadsheet_odf".into(),
                serde_json::to_value(raw).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &id);
        }
        graph
            .finalize_projection(&identities)
            .map_err(transform_error)?;
        Ok(graph)
    }
}

fn project_sheet_items<F>(
    sheet: &SpreadsheetOdfSheet,
    sheet_id: &str,
    make_id: &F,
    graph: &mut DocumentGraph,
) -> Result<(), TransformError>
where
    F: Fn(
        Vec<String>,
        Option<String>,
        Option<crate::core::SourceLocator>,
    ) -> Result<String, TransformError>,
{
    for (collection, index, kind, text, locator, value) in sheet
        .comments
        .iter()
        .enumerate()
        .map(|(index, item)| {
            (
                "comments",
                index,
                DocumentNodeKind::Comment,
                Some(item.text.clone()),
                item.locator.clone(),
                serde_json::to_value(item),
            )
        })
        .chain(sheet.links.iter().enumerate().map(|(index, item)| {
            (
                "links",
                index,
                DocumentNodeKind::Link,
                Some(item.target.clone()),
                item.locator.clone(),
                serde_json::to_value(item),
            )
        }))
        .chain(sheet.objects.iter().enumerate().map(|(index, item)| {
            (
                "objects",
                index,
                match item.kind {
                    SpreadsheetOdfObjectKind::Chart => DocumentNodeKind::Chart,
                    SpreadsheetOdfObjectKind::Image => DocumentNodeKind::Image,
                    SpreadsheetOdfObjectKind::EmbeddedObject => DocumentNodeKind::Attachment,
                    SpreadsheetOdfObjectKind::Unknown => DocumentNodeKind::Figure,
                },
                item.title.clone().or_else(|| item.name.clone()),
                item.locator.clone(),
                serde_json::to_value(item),
            )
        }))
    {
        let id = make_id(
            vec![
                "sheets".into(),
                sheet.order.to_string(),
                collection.into(),
                index.to_string(),
            ],
            None,
            Some(locator.clone()),
        )?;
        let mut node = DocumentNode::new(&id, kind)
            .with_locator(locator)
            .with_ordinal(index);
        node.text = text;
        node.extensions.insert(
            "grist.spreadsheet_odf".into(),
            value.map_err(transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(sheet_id, &id);
    }
    for (index, item) in sheet.raw_elements.iter().enumerate() {
        let id = make_id(
            vec![
                "sheets".into(),
                sheet.order.to_string(),
                "raw_elements".into(),
                index.to_string(),
            ],
            None,
            Some(item.locator.clone()),
        )?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::Raw)
            .with_name(&item.name)
            .with_text(&item.raw_xml)
            .with_locator(item.locator.clone())
            .with_ordinal(index);
        node.extensions.insert(
            "grist.spreadsheet_odf".into(),
            serde_json::to_value(item).map_err(transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(sheet_id, &id);
    }
    Ok(())
}

fn formula_references(formula: &str) -> Vec<String> {
    let mut output = Vec::new();
    let bytes = formula.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len()
            && !(bytes[index] == b'$' || bytes[index] == b'.' || bytes[index].is_ascii_alphabetic())
        {
            index += 1;
        }
        let start = index;
        while index < bytes.len()
            && (bytes[index] == b'$' || bytes[index] == b'.' || bytes[index].is_ascii_alphabetic())
        {
            index += 1;
        }
        let digit = index;
        while index < bytes.len() && (bytes[index] == b'$' || bytes[index].is_ascii_digit()) {
            index += 1;
        }
        if digit > start && index > digit && bytes[digit..index].iter().any(u8::is_ascii_digit) {
            output.push(
                formula[start..index]
                    .trim_start_matches('.')
                    .replace('$', ""),
            );
        }
        if index == start {
            index += 1;
        }
    }
    output.sort();
    output.dedup();
    output
}

fn transform_error(error: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: error.to_string(),
    }
}
