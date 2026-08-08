use super::ImageDocument;
use crate::core::{SchemaVersion, SourceLocator};
use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    GraphIdGenerator, ProjectionAddress, RawNodeContent, ToDocumentGraph, TransformError,
};

impl ToDocumentGraph for ImageDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::IMAGE_V1, "image")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Image).with_projection(
            "image",
            SchemaVersion::IMAGE_V1,
            "grist.image.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.dialect = Some(self.format.as_str().into());
        graph.attrs = context.attrs;
        graph
            .attrs
            .insert("width".into(), self.dimensions.width.into());
        graph
            .attrs
            .insert("height".into(), self.dimensions.height.into());
        graph
            .attrs
            .insert("frame_count".into(), (self.frames.len() as u64).into());
        let root_locator = self.frames.first().map(|frame| frame.locator.clone());
        let root = node_id(&ids, &["image"], self.format.as_str(), root_locator.clone())?;
        let mut root_node = DocumentNode::new(&root, DocumentNodeKind::Document)
            .with_name(format!("{} image", self.format.as_str()))
            .with_ordinal(0);
        if let Some(locator) = root_locator {
            root_node = root_node.with_locator(locator);
        }
        root_node.extensions.insert(
            "grist.image".into(),
            serde_json::json!({
                "dimensions": self.dimensions, "color": self.color, "orientation": self.orientation,
                "vector": self.vector, "complete": self.complete,
            }),
        );
        graph.add_node(root_node);
        let mut ordinal = 0usize;
        for frame in &self.frames {
            let id = node_id(
                &ids,
                &["image", "frames", &frame.index.to_string()],
                &frame.index.to_string(),
                Some(frame.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Image)
                .with_name(format!("frame {}", frame.index))
                .with_locator(frame.locator.clone())
                .with_ordinal(ordinal);
            ordinal = ordinal.saturating_add(1);
            node.extensions.insert(
                "grist.image".into(),
                serde_json::to_value(frame).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &id);
        }
        for (index, item) in self.embedded_text.iter().enumerate() {
            let id = node_id(
                &ids,
                &["image", "text", &index.to_string()],
                &format!("{:?}-{index}", item.kind),
                Some(item.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Text)
                .with_text(&item.text)
                .with_locator(item.locator.clone())
                .with_ordinal(ordinal);
            ordinal = ordinal.saturating_add(1);
            node.extensions.insert(
                "grist.image".into(),
                serde_json::to_value(item).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &id);
        }
        for (index, block) in self.metadata.blocks.iter().enumerate() {
            let id = node_id(
                &ids,
                &["image", "metadata", &index.to_string()],
                &format!("{:?}-{index}", block.kind),
                Some(block.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Metadata)
                .with_name(format!("{:?}", block.kind))
                .with_locator(block.locator.clone())
                .with_ordinal(ordinal);
            ordinal = ordinal.saturating_add(1);
            if let Some(text) = &block.text {
                node.text = Some(text.clone());
            }
            node.extensions.insert(
                "grist.image".into(),
                serde_json::to_value(block).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &id);
        }
        for (index, link) in self.links.iter().enumerate() {
            let id = node_id(
                &ids,
                &["image", "links", &index.to_string()],
                &link.target,
                Some(link.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Link)
                .with_text(&link.target)
                .with_locator(link.locator.clone())
                .with_ordinal(ordinal);
            ordinal = ordinal.saturating_add(1);
            node.extensions.insert(
                "grist.image".into(),
                serde_json::to_value(link).map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &id);
        }
        for (index, active) in self.active_content.iter().enumerate() {
            let id = node_id(
                &ids,
                &["image", "active_content", &index.to_string()],
                &format!("{}-{index}", active.kind),
                Some(active.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Unknown)
                .with_name(&active.kind)
                .with_locator(active.locator.clone())
                .with_ordinal(ordinal);
            ordinal = ordinal.saturating_add(1);
            node.raw = Some(
                RawNodeContent::new(
                    "grist.image",
                    "inert_active_content",
                    serde_json::to_value(active).map_err(error)?,
                )
                .map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &id);
        }
        for (index, chunk) in self
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, chunk)| !chunk.known)
        {
            let id = node_id(
                &ids,
                &["image", "unknown_chunks", &index.to_string()],
                &format!("{}-{index}", chunk.kind),
                Some(chunk.locator.clone()),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Raw)
                .with_name(&chunk.kind)
                .with_locator(chunk.locator.clone())
                .with_ordinal(ordinal);
            ordinal = ordinal.saturating_add(1);
            node.raw = Some(
                RawNodeContent::new(
                    "grist.image",
                    "unknown_chunk",
                    serde_json::to_value(chunk).map_err(error)?,
                )
                .map_err(error)?,
            );
            graph.add_node(node);
            graph.add_contains(&root, &id);
        }
        super::ocr_graph::project_text_representations(&mut graph, &ids, &root, self)?;
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn node_id(
    ids: &GraphIdGenerator,
    path: &[&str],
    native: &str,
    locator: Option<SourceLocator>,
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
