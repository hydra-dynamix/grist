use super::*;
use crate::core::SchemaVersion;
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, ProjectionAddress, ToDocumentGraph, TransformError,
};
use serde_json::json;
use std::collections::HashMap;

impl ToDocumentGraph for SpreadsheetOoxmlDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::SPREADSHEET_OOXML_V1, "spreadsheet_ooxml")
            .map_err(transform_error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::SpreadsheetOoxml)
            .with_projection(
                "spreadsheet_ooxml",
                SchemaVersion::SPREADSHEET_OOXML_V1,
                "grist.spreadsheet_ooxml.to-document-graph.v1",
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
            Some(self.workbook_part.clone()),
            Some(self.workbook_locator.clone()),
        )?;
        let mut root = DocumentNode::new(&root_id, DocumentNodeKind::Document)
            .with_name(&self.workbook_part)
            .with_locator(self.workbook_locator.clone())
            .with_ordinal(0);
        root.extensions.insert(
            "grist.spreadsheet_ooxml".into(),
            json!({
                "package_kind": self.package_kind, "date_system": self.date_system,
                "calculation": self.calculation, "macro_project_count": self.macro_projects.len()
            }),
        );
        graph.add_node(root);
        let mut cell_ids = HashMap::<(String, String), String>::new();
        for sheet in &self.sheets {
            let sheet_id = make_id(
                vec!["sheets".into(), sheet.order.to_string()],
                Some(format!("sheet:{}", sheet.sheet_id)),
                Some(sheet.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&sheet_id, DocumentNodeKind::Sheet)
                .with_name(&sheet.name)
                .with_locator(sheet.locator.clone())
                .with_ordinal(sheet.order)
                .with_attr(
                    "visibility",
                    serde_json::to_value(sheet.visibility).map_err(transform_error)?,
                )
                .with_attr("dimension", sheet.dimension.clone().unwrap_or_default());
            node.extensions.insert(
                "grist.spreadsheet_ooxml".into(),
                json!({
                    "part": sheet.part, "pane": sheet.pane, "selections": sheet.selections,
                    "columns": sheet.columns, "merges": sheet.merges
                }),
            );
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
                    .with_attr("hidden", row.hidden);
                row_node.extensions.insert(
                    "grist.spreadsheet_ooxml".into(),
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
                        "grist.spreadsheet_ooxml".into(),
                        serde_json::to_value(cell).map_err(transform_error)?,
                    );
                    graph.add_node(cell_node);
                    graph.add_contains(&row_id, &cell_id);
                    cell_ids.insert((sheet.name.clone(), cell.reference.clone()), cell_id);
                }
            }
            project_sheet_objects(sheet, &sheet_id, &make_id, &mut graph)?;
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
            let node_id = make_id(
                vec!["named_ranges".into(), index.to_string()],
                Some(format!("defined-name:{}", range.name)),
                Some(range.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::StructuredValue)
                .with_name(&range.name)
                .with_text(&range.formula)
                .with_locator(range.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.spreadsheet_ooxml".into(),
                serde_json::to_value(range).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
        }
        for (index, property) in self.properties.iter().enumerate() {
            let node_id = make_id(
                vec!["properties".into(), index.to_string()],
                None,
                Some(property.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Metadata)
                .with_name(&property.name)
                .with_text(&property.value)
                .with_locator(property.locator.clone())
                .with_ordinal(index);
            node.extensions.insert(
                "grist.spreadsheet_ooxml".into(),
                serde_json::to_value(property).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
        }
        for (index, project) in self.macro_projects.iter().enumerate() {
            let node_id = make_id(
                vec!["macro_projects".into(), index.to_string()],
                Some(project.part.clone()),
                Some(project.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&node_id, DocumentNodeKind::Attachment)
                .with_name(&project.part)
                .with_locator(project.locator.clone())
                .with_ordinal(index)
                .with_attr("quarantined", true)
                .with_attr("executable", false);
            node.extensions.insert(
                "grist.spreadsheet_ooxml".into(),
                serde_json::to_value(project).map_err(transform_error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root_id, &node_id);
        }
        graph
            .finalize_projection(&identities)
            .map_err(transform_error)?;
        Ok(graph)
    }
}

fn project_sheet_objects<F>(
    sheet: &SpreadsheetSheet,
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
        .tables
        .iter()
        .enumerate()
        .map(|(index, item)| {
            (
                "tables",
                index,
                DocumentNodeKind::Table,
                item.display_name.clone().or_else(|| item.name.clone()),
                item.locator.clone(),
                serde_json::to_value(item),
            )
        })
        .chain(sheet.comments.iter().enumerate().map(|(index, item)| {
            (
                "comments",
                index,
                DocumentNodeKind::Comment,
                Some(item.text.clone()),
                item.locator.clone(),
                serde_json::to_value(item),
            )
        }))
        .chain(sheet.hyperlinks.iter().enumerate().map(|(index, item)| {
            (
                "links",
                index,
                DocumentNodeKind::Link,
                item.target.clone().or_else(|| item.location.clone()),
                item.locator.clone(),
                serde_json::to_value(item),
            )
        }))
        .chain(sheet.objects.iter().enumerate().map(|(index, item)| {
            (
                "objects",
                index,
                match item.kind {
                    SpreadsheetObjectKind::Chart => DocumentNodeKind::Chart,
                    SpreadsheetObjectKind::Image => DocumentNodeKind::Image,
                    SpreadsheetObjectKind::EmbeddedObject => DocumentNodeKind::Attachment,
                    _ => DocumentNodeKind::Figure,
                },
                item.title.clone().or_else(|| item.name.clone()),
                item.locator.clone(),
                serde_json::to_value(item),
            )
        }))
    {
        let node_id = make_id(
            vec![
                "sheets".into(),
                sheet.order.to_string(),
                collection.into(),
                index.to_string(),
            ],
            None,
            Some(locator.clone()),
        )?;
        let mut node = DocumentNode::new(&node_id, kind)
            .with_locator(locator)
            .with_ordinal(index);
        node.text = text;
        node.extensions.insert(
            "grist.spreadsheet_ooxml".into(),
            value.map_err(transform_error)?,
        );
        graph.add_node(node);
        graph.add_contains(sheet_id, &node_id);
    }
    Ok(())
}

fn formula_references(formula: &str) -> Vec<String> {
    let bytes = formula.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        while index < bytes.len() && (bytes[index] == b'$' || bytes[index].is_ascii_alphabetic()) {
            index += 1;
        }
        let digit_start = index;
        while index < bytes.len() && (bytes[index] == b'$' || bytes[index].is_ascii_digit()) {
            index += 1;
        }
        if digit_start > start
            && index > digit_start
            && bytes[digit_start..index].iter().any(u8::is_ascii_digit)
        {
            output.push(formula[start..index].replace('$', ""));
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
