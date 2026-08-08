use super::{NotebookCell, NotebookDocument};
use crate::core::{LocationComponent, SchemaVersion, SourceLocator};
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, ToDocumentGraph,
    TransformError,
};

impl ToDocumentGraph for NotebookDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::IPYNB_V1, "grist.ipynb")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Notebook)
            .with_projection(
                "ipynb",
                SchemaVersion::IPYNB_V1,
                "grist.ipynb.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.language = context.language;
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert("nbformat".into(), self.nbformat.into());
        graph
            .attrs
            .insert("nbformat_minor".into(), self.nbformat_minor.into());
        graph.attrs.insert("metadata".into(), self.metadata.clone());
        graph
            .attrs
            .insert("stored_results_are_inert".into(), true.into());
        graph.attrs.insert(
            "worksheets".into(),
            serde_json::to_value(&self.worksheets).map_err(error)?,
        );

        let root = node_id(
            &ids,
            ProjectionAddress::located(["notebook"], self.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(&root, DocumentNodeKind::Notebook)
                .with_name("Jupyter Notebook")
                .with_locator(self.locator.clone())
                .with_ordinal(0),
        );
        for cell in &self.cells {
            project_cell(&mut graph, &ids, &root, cell)?;
        }
        if let Some(widgets) = &self.widgets {
            let locator = SourceLocator::exact(LocationComponent::JsonPointer {
                pointer: "/metadata/widgets".into(),
            })
            .expect("widget locator is valid");
            let widget_id = node_id(
                &ids,
                ProjectionAddress::located(["notebook", "widgets"], locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&widget_id, DocumentNodeKind::Metadata)
                    .with_name("Jupyter widgets")
                    .with_locator(locator)
                    .with_attr("state", widgets.clone()),
            );
            graph.add_contains(&root, &widget_id);
        }
        graph.diagnostics = self.diagnostics.clone();
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn project_cell(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    root: &str,
    cell: &NotebookCell,
) -> Result<(), TransformError> {
    let address = ProjectionAddress::native(
        ["notebook", "cells", &cell.ordinal.to_string()],
        cell.id.as_deref().unwrap_or(&cell.stable_id),
    )
    .with_locator(cell.locator.clone());
    let cell_id = node_id(ids, address)?;
    graph.add_node(
        DocumentNode::new(&cell_id, DocumentNodeKind::NotebookCell)
            .with_name(cell.cell_type.clone())
            .with_text(cell.source.clone())
            .with_locator(cell.locator.clone())
            .with_ordinal(cell.ordinal)
            .with_attr("native_id", serde_json::to_value(&cell.id).map_err(error)?)
            .with_attr("stable_id", cell.stable_id.clone())
            .with_attr("cell_type", cell.cell_type.clone())
            .with_attr("metadata", cell.metadata.clone())
            .with_attr(
                "execution_count",
                serde_json::to_value(&cell.execution_count).map_err(error)?,
            ),
    );
    graph.add_contains(root, &cell_id);

    for attachment in &cell.attachments {
        let attachment_id = node_id(
            ids,
            ProjectionAddress::native(
                [
                    "notebook",
                    "cells",
                    &cell.ordinal.to_string(),
                    "attachments",
                    &attachment.name,
                ],
                &attachment.name,
            )
            .with_locator(attachment.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(&attachment_id, DocumentNodeKind::Attachment)
                .with_name(attachment.name.clone())
                .with_locator(attachment.locator.clone())
                .with_attr(
                    "data",
                    serde_json::to_value(&attachment.data).map_err(error)?,
                ),
        );
        graph.add_contains(&cell_id, &attachment_id);
        graph.add_edge(DocumentEdge::explicit(
            attachment_id,
            DocumentRelation::AttachmentOf,
            cell_id.clone(),
            attachment.locator.clone(),
        ));
    }

    for output in &cell.outputs {
        let output_id = node_id(
            ids,
            ProjectionAddress::located(
                [
                    "notebook",
                    "cells",
                    &cell.ordinal.to_string(),
                    "outputs",
                    &output.ordinal.to_string(),
                ],
                output.locator.clone(),
            ),
        )?;
        let text = output.text.clone().or_else(|| {
            output.error_value.as_ref().map(|value| {
                format!(
                    "{}: {value}",
                    output.error_name.as_deref().unwrap_or("Error")
                )
            })
        });
        let mut node = DocumentNode::new(&output_id, DocumentNodeKind::CellOutput)
            .with_name(output.normalized_output_type.clone())
            .with_locator(output.locator.clone())
            .with_ordinal(output.ordinal)
            .with_attr("stored_result", true)
            .with_attr("executed_by_grist", false)
            .with_attr("output", serde_json::to_value(output).map_err(error)?);
        if let Some(text) = text {
            node = node.with_text(text);
        }
        graph.add_node(node);
        graph.add_contains(&cell_id, &output_id);
        graph.add_edge(DocumentEdge::explicit(
            output_id,
            DocumentRelation::DerivedFrom,
            cell_id.clone(),
            output.locator.clone(),
        ));
    }
    Ok(())
}

fn node_id(ids: &GraphIdGenerator, address: ProjectionAddress) -> Result<String, TransformError> {
    ids.node_id(&address).map_err(error)
}

fn error(value: impl std::fmt::Display) -> TransformError {
    TransformError::Other {
        message: value.to_string(),
    }
}
