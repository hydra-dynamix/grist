use super::{EmailDocument, MimePart};
use crate::core::{SchemaVersion, SourceLocator};
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, ToDocumentGraph,
    TransformError,
};

impl ToDocumentGraph for EmailDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::EMAIL_V1, "email")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Email).with_projection(
            "email",
            SchemaVersion::EMAIL_V1,
            "grist.email.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "thread".into(),
            serde_json::to_value(&self.thread).map_err(error)?,
        );
        graph.attrs.insert(
            "external_references".into(),
            serde_json::to_value(&self.external_references).map_err(error)?,
        );
        let root = node_id(
            &ids,
            &["message"],
            "message",
            Some(self.mime.locator.clone()),
        )?;
        let mut root_node = DocumentNode::new(&root, DocumentNodeKind::Email)
            .with_locator(self.mime.locator.clone())
            .with_ordinal(0);
        if let Some(subject) = &self.subject {
            root_node = root_node
                .with_name(subject.decoded.clone())
                .with_attr("raw_subject", subject.raw.clone());
        }
        graph.add_node(root_node);
        for header in &self.headers {
            let header_id = node_id(
                &ids,
                &["message", "headers", &header.ordinal.to_string()],
                &format!("header:{}", header.ordinal),
                Some(header.locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&header_id, DocumentNodeKind::Metadata)
                    .with_name(
                        header
                            .name
                            .clone()
                            .unwrap_or_else(|| "malformed-header".into()),
                    )
                    .with_text(header.decoded_value.clone())
                    .with_locator(header.locator.clone())
                    .with_ordinal(header.ordinal)
                    .with_attr("raw", header.raw.clone())
                    .with_attr("valid", header.valid),
            );
            graph.add_contains(&root, &header_id);
        }
        project_part(&mut graph, &ids, &root, &self.mime)?;
        for (ordinal, reply) in self.thread.in_reply_to.iter().enumerate() {
            let id = node_id(
                &ids,
                &["message", "in_reply_to", &ordinal.to_string()],
                &reply.value,
                Some(reply.locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&id, DocumentNodeKind::Reference)
                    .with_text(reply.value.clone())
                    .with_locator(reply.locator.clone())
                    .with_ordinal(ordinal),
            );
            graph.add_edge(DocumentEdge::explicit(
                root.clone(),
                DocumentRelation::ReplyTo,
                id,
                reply.locator.clone(),
            ));
        }
        graph.diagnostics = self.diagnostics.clone();
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn project_part(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    parent: &str,
    part: &MimePart,
) -> Result<String, TransformError> {
    let path = display_path(&part.path);
    let id = node_id(
        ids,
        &["message", "mime", &path],
        &format!("mime:{path}"),
        Some(part.locator.clone()),
    )?;
    graph.add_node(
        DocumentNode::new(&id, DocumentNodeKind::MimePart)
            .with_name(part.content_type.essence.clone())
            .with_locator(part.locator.clone())
            .with_ordinal(part.path.last().copied().unwrap_or(1).saturating_sub(1))
            .with_attr("mime_path", part.path.clone())
            .with_attr(
                "content_type",
                serde_json::to_value(&part.content_type).map_err(error)?,
            )
            .with_attr(
                "content_disposition",
                serde_json::to_value(&part.content_disposition).map_err(error)?,
            )
            .with_attr(
                "content_id",
                serde_json::to_value(&part.content_id).map_err(error)?,
            )
            .with_attr(
                "content_location",
                serde_json::to_value(&part.content_location).map_err(error)?,
            )
            .with_attr("transfer_encoding", part.transfer_encoding.clone())
            .with_attr("smime", serde_json::to_value(&part.smime).map_err(error)?)
            .with_attr("tnef", serde_json::to_value(&part.tnef).map_err(error)?)
            .with_attr("encrypted", part.encrypted)
            .with_attr("signed", part.signed),
    );
    graph.add_contains(parent, &id);
    if let Some(text) = &part.text {
        let body_id = node_id(
            ids,
            &["message", "mime", &path, "body"],
            &format!("body:{path}"),
            Some(text.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(&body_id, DocumentNodeKind::MessageBody)
                .with_text(text.text.clone())
                .with_locator(text.locator.clone())
                .with_ordinal(0)
                .with_attr("charset", text.charset.clone())
                .with_attr("lossy", text.lossy),
        );
        graph.add_contains(&id, &body_id);
    }
    if let Some(attachment) = &part.attachment {
        let attachment_id = node_id(
            ids,
            &["message", "mime", &path, "attachment"],
            &attachment.artifact.identity.artifact_id,
            Some(part.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(&attachment_id, DocumentNodeKind::Attachment)
                .with_name(
                    attachment
                        .filename
                        .clone()
                        .unwrap_or_else(|| part.content_type.essence.clone()),
                )
                .with_locator(part.locator.clone())
                .with_ordinal(0)
                .with_attr(
                    "artifact",
                    serde_json::to_value(&attachment.artifact).map_err(error)?,
                )
                .with_attr(
                    "nested",
                    serde_json::to_value(&attachment.nested).map_err(error)?,
                )
                .with_attr("inline_resource", attachment.inline_resource),
        );
        graph.add_contains(&id, &attachment_id);
        graph.add_edge(DocumentEdge::explicit(
            attachment_id,
            if attachment.inline_resource {
                DocumentRelation::EmbeddedIn
            } else {
                DocumentRelation::AttachmentOf
            },
            parent.to_string(),
            part.locator.clone(),
        ));
    }
    let mut child_ids = Vec::new();
    for child in &part.children {
        child_ids.push(project_part(graph, ids, &id, child)?);
    }
    if part.content_type.essence == "multipart/alternative"
        && let Some(primary) = child_ids.first()
    {
        for (child, child_part) in child_ids.iter().skip(1).zip(part.children.iter().skip(1)) {
            graph.add_edge(DocumentEdge::explicit(
                child.clone(),
                DocumentRelation::AlternativeRepresentationOf,
                primary.clone(),
                child_part.locator.clone(),
            ));
        }
    }
    Ok(id)
}

fn display_path(path: &[usize]) -> String {
    if path.is_empty() {
        "root".into()
    } else {
        path.iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(".")
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
