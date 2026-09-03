use super::{ColumnarDocument, ColumnarValue};
use crate::core::SchemaVersion;
use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    GraphIdGenerator, ProjectionAddress, ToDocumentGraph, TransformError,
};
use std::collections::BTreeMap;

impl ToDocumentGraph for ColumnarDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::COLUMNAR_V1, "columnar")
            .map_err(err)?;
        let mut graph =
            DocumentGraph::new(context.graph_id, DocumentKind::Other("columnar".into()))
                .with_projection(
                    "columnar",
                    SchemaVersion::COLUMNAR_V1,
                    "grist.columnar.to-document-graph.v1",
                );
        graph.source = context.source;
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "format".into(),
            serde_json::to_value(self.format).map_err(err)?,
        );
        graph.attrs.insert(
            "schema".into(),
            serde_json::to_value(&self.schema).map_err(err)?,
        );
        let root = id(&ids, &["document"], "root", None)?;
        graph.add_node(DocumentNode::new(&root, DocumentNodeKind::Document).with_ordinal(0));
        for batch in &self.batches {
            let batch_id = id(
                &ids,
                &["document", "batches", &batch.index.to_string()],
                &format!("batch:{}", batch.index),
                Some(batch.locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&batch_id, DocumentNodeKind::Container)
                    .with_name(format!("{:?} {}", batch.kind, batch.index))
                    .with_locator(batch.locator.clone())
                    .with_ordinal(batch.index - 1)
                    .with_attr("source_row_count", batch.source_row_count),
            );
            graph.add_contains(&root, &batch_id);
            let mut rows: BTreeMap<u64, Vec<(&[String], &super::ColumnarCell)>> = BTreeMap::new();
            for column in &batch.columns {
                for cell in &column.values {
                    rows.entry(cell.row).or_default().push((&column.path, cell));
                }
            }
            for (row, cells) in rows {
                let row_locator = cells[0].1.locator.clone();
                let row_id = id(
                    &ids,
                    &[
                        "document",
                        "batches",
                        &batch.index.to_string(),
                        "rows",
                        &row.to_string(),
                    ],
                    &format!("row:{row}"),
                    Some(row_locator.clone()),
                )?;
                graph.add_node(
                    DocumentNode::new(&row_id, DocumentNodeKind::Row)
                        .with_locator(row_locator)
                        .with_ordinal(row as usize)
                        .with_attr("source_row", row),
                );
                graph.add_contains(&batch_id, &row_id);
                for (ordinal, (path, cell)) in cells.into_iter().enumerate() {
                    let name = path.join(".");
                    let cell_id = id(
                        &ids,
                        &[
                            "document",
                            "batches",
                            &batch.index.to_string(),
                            "rows",
                            &row.to_string(),
                            "fields",
                            &name,
                        ],
                        &format!("cell:{row}:{name}:{}", cell.repetition_index),
                        Some(cell.locator.clone()),
                    )?;
                    let mut node = DocumentNode::new(&cell_id, DocumentNodeKind::Cell)
                        .with_name(name)
                        .with_locator(cell.locator.clone())
                        .with_ordinal(ordinal)
                        .with_attr("definition_level", cell.definition_level)
                        .with_attr("repetition_level", cell.repetition_level)
                        .with_attr(
                            "typed_value",
                            serde_json::to_value(&cell.value).map_err(err)?,
                        );
                    node.text = value_text(&cell.value);
                    graph.add_node(node);
                    graph.add_contains(&row_id, &cell_id)
                }
            }
        }
        graph.finalize_projection(&ids).map_err(err)?;
        Ok(graph)
    }
}
fn value_text(value: &ColumnarValue) -> Option<String> {
    match value {
        ColumnarValue::Null => None,
        ColumnarValue::Boolean { value } => Some(value.to_string()),
        ColumnarValue::SignedInteger { canonical }
        | ColumnarValue::UnsignedInteger { canonical }
        | ColumnarValue::Float { canonical, .. }
        | ColumnarValue::Interval { canonical } => Some(canonical.clone()),
        ColumnarValue::Decimal {
            unscaled, scale, ..
        } => Some(format!("{unscaled}e-{scale}")),
        ColumnarValue::Utf8 { value } => Some(value.clone()),
        ColumnarValue::Date { value, unit }
        | ColumnarValue::Time { value, unit }
        | ColumnarValue::Duration { value, unit } => Some(format!("{value} {unit}")),
        ColumnarValue::Timestamp {
            value,
            unit,
            timezone,
        } => Some(format!(
            "{value} {unit} {}",
            timezone.as_deref().unwrap_or("")
        )),
        other => serde_json::to_string(other).ok(),
    }
}
fn id(
    ids: &GraphIdGenerator,
    path: &[&str],
    native: &str,
    locator: Option<crate::core::SourceLocator>,
) -> Result<String, TransformError> {
    let mut address = ProjectionAddress::native(path.iter().copied(), native);
    if let Some(locator) = locator {
        address = address.with_locator(locator)
    }
    ids.node_id(&address).map_err(err)
}
fn err(error: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: error.to_string(),
    }
}
