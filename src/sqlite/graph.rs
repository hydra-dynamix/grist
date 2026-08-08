use super::{SqliteDocument, SqliteValue};
use crate::core::SchemaVersion;
use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    GraphIdGenerator, ProjectionAddress, ToDocumentGraph, TransformError,
};

impl ToDocumentGraph for SqliteDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::SQLITE_V1, "sqlite")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Other("sqlite".into()))
            .with_projection(
                "sqlite",
                SchemaVersion::SQLITE_V1,
                "grist.sqlite.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "header".into(),
            serde_json::to_value(&self.header).map_err(error)?,
        );
        graph.attrs.insert(
            "schema".into(),
            serde_json::to_value(&self.schema).map_err(error)?,
        );
        let root = node_id(&ids, &["database"], "database", None)?;
        graph.add_node(DocumentNode::new(&root, DocumentNodeKind::Document).with_ordinal(0));
        for (table_index, table) in self.tables.iter().enumerate() {
            let table_id = node_id(
                &ids,
                &["database", "tables", &table.name],
                &format!("table:{}", table.name),
                Some(table.locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&table_id, DocumentNodeKind::Table)
                    .with_name(table.name.clone())
                    .with_text(table.definition.clone())
                    .with_locator(table.locator.clone())
                    .with_ordinal(table_index)
                    .with_attr("root_page", table.root_page)
                    .with_attr("without_rowid", table.without_rowid),
            );
            graph.add_contains(&root, &table_id);
            if let Some(records) = self.record_sets.iter().find(|set| set.table == table.name) {
                for record in &records.records {
                    let record_id = node_id(
                        &ids,
                        &[
                            "database",
                            "tables",
                            &table.name,
                            "records",
                            &record.stable_key,
                        ],
                        &record.stable_key,
                        Some(record.locator.clone()),
                    )?;
                    graph.add_node(
                        DocumentNode::new(&record_id, DocumentNodeKind::Row)
                            .with_locator(record.locator.clone())
                            .with_ordinal(record.ordinal - 1)
                            .with_attr("stable_key", record.stable_key.clone())
                            .with_attr("rowid", record.rowid),
                    );
                    graph.add_contains(&table_id, &record_id);
                    for field in &record.fields {
                        let field_id = node_id(
                            &ids,
                            &[
                                "database",
                                "tables",
                                &table.name,
                                "records",
                                &record.stable_key,
                                "fields",
                                &field.name,
                            ],
                            &format!("{}:{}", record.stable_key, field.name),
                            Some(field.locator.clone()),
                        )?;
                        let mut node = DocumentNode::new(&field_id, DocumentNodeKind::Cell)
                            .with_name(field.name.clone())
                            .with_locator(field.locator.clone())
                            .with_ordinal(field.ordinal)
                            .with_attr(
                                "typed_value",
                                serde_json::to_value(&field.value).map_err(error)?,
                            );
                        node.text = value_text(&field.value);
                        graph.add_node(node);
                        graph.add_contains(&record_id, &field_id);
                    }
                }
            }
        }
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn value_text(value: &SqliteValue) -> Option<String> {
    value.display_text()
}

fn node_id(
    ids: &GraphIdGenerator,
    path: &[&str],
    native: &str,
    locator: Option<crate::core::SourceLocator>,
) -> Result<String, TransformError> {
    let mut address = ProjectionAddress::native(path.iter().copied(), native);
    if let Some(locator) = locator {
        address = address.with_locator(locator);
    }
    ids.node_id(&address).map_err(error)
}

fn error(value: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: value.to_string(),
    }
}
