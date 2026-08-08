use super::{MboxDocument, MboxMessage};
use crate::core::{SchemaVersion, SourceLocator};
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, ToDocumentGraph,
    TransformError,
};
use std::collections::BTreeMap;

impl ToDocumentGraph for MboxDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::MBOX_V1, "mbox")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Mbox).with_projection(
            "mbox",
            SchemaVersion::MBOX_V1,
            "grist.mbox.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.dialect = Some(format!("{:?}", self.variant).to_ascii_lowercase());
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "thread_evidence".into(),
            serde_json::to_value(&self.thread_evidence).map_err(error)?,
        );
        let root = node_id(&ids, &["mailbox"], "mailbox", None)?;
        graph.add_node(
            DocumentNode::new(&root, DocumentNodeKind::Container)
                .with_name("MBOX mailbox")
                .with_ordinal(0)
                .with_attr(
                    "variant",
                    serde_json::to_value(self.variant).map_err(error)?,
                ),
        );
        let mut message_nodes = BTreeMap::new();
        let mut previous = None;
        for message in &self.messages {
            let message_id = project_message(&mut graph, &ids, &root, message)?;
            if let Some(previous_id) = previous {
                graph.add_edge(DocumentEdge::explicit(
                    previous_id,
                    DocumentRelation::Precedes,
                    message_id.clone(),
                    message.locator.clone(),
                ));
            }
            previous = Some(message_id.clone());
            message_nodes.insert(message.stable_id.clone(), message_id);
        }
        for (ordinal, link) in self.thread_evidence.links.iter().enumerate() {
            let Some(source) = message_nodes.get(&link.source_stable_id) else {
                continue;
            };
            if let Some(target) = link
                .target_stable_id
                .as_ref()
                .and_then(|stable_id| message_nodes.get(stable_id))
            {
                let locator = self
                    .messages
                    .iter()
                    .find(|message| message.stable_id == link.source_stable_id)
                    .map(|message| message.locator.clone())
                    .expect("aggregate links name source messages");
                let relation = match link.relation {
                    crate::mbox::MboxThreadRelation::InReplyTo => DocumentRelation::ReplyTo,
                    crate::mbox::MboxThreadRelation::Reference => DocumentRelation::References,
                };
                graph.add_edge(DocumentEdge::explicit(
                    source.clone(),
                    relation,
                    target.clone(),
                    locator,
                ));
            } else {
                let reference_id = node_id(
                    &ids,
                    &["mailbox", "unresolved", &ordinal.to_string()],
                    &link.referenced_message_id,
                    None,
                )?;
                graph.add_node(
                    DocumentNode::new(&reference_id, DocumentNodeKind::Reference)
                        .with_text(link.referenced_message_id.clone())
                        .with_ordinal(ordinal),
                );
                graph.add_edge(DocumentEdge::inferred(
                    source.clone(),
                    DocumentRelation::ReplyTo,
                    reference_id,
                    "grist.mbox.unresolved-thread-reference.v1",
                    crate::core::LocatorConfidence::new(1.0).expect("one is valid"),
                ));
            }
        }
        graph.diagnostics = self.diagnostics.clone();
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn project_message(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    root: &str,
    message: &MboxMessage,
) -> Result<String, TransformError> {
    let base = ["mailbox", "messages", message.stable_id.as_str()];
    let id = node_id(
        ids,
        &base,
        &message.stable_id,
        Some(message.locator.clone()),
    )?;
    let mut node = DocumentNode::new(&id, DocumentNodeKind::Email)
        .with_locator(message.locator.clone())
        .with_ordinal(message.ordinal.saturating_sub(1))
        .with_attr("stable_id", message.stable_id.clone())
        .with_attr(
            "separator",
            serde_json::to_value(&message.separator).map_err(error)?,
        )
        .with_attr(
            "status",
            serde_json::to_value(message.status).map_err(error)?,
        );
    if let Some(subject) = message
        .email
        .as_ref()
        .and_then(|email| email.subject.as_ref())
    {
        node = node.with_name(subject.decoded.clone());
    }
    graph.add_node(node);
    graph.add_contains(root, &id);
    let Some(email) = &message.email else {
        return Ok(id);
    };
    for header in &email.headers {
        let path = [
            "mailbox",
            "messages",
            message.stable_id.as_str(),
            "headers",
            &header.ordinal.to_string(),
        ];
        let header_id = node_id(
            ids,
            &path,
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
        graph.add_contains(&id, &header_id);
    }
    project_part(graph, ids, &id, message, &email.mime)?;
    Ok(id)
}

fn project_part(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    parent: &str,
    message: &MboxMessage,
    part: &crate::email::MimePart,
) -> Result<String, TransformError> {
    let path = display_path(&part.path);
    let structural = [
        "mailbox",
        "messages",
        message.stable_id.as_str(),
        "mime",
        path.as_str(),
    ];
    let id = node_id(
        ids,
        &structural,
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
            .with_attr("encrypted", part.encrypted)
            .with_attr("signed", part.signed),
    );
    graph.add_contains(parent, &id);
    if let Some(text) = &part.text {
        let body_path = [
            "mailbox",
            "messages",
            message.stable_id.as_str(),
            "mime",
            path.as_str(),
            "body",
        ];
        let body_id = node_id(
            ids,
            &body_path,
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
        let attachment_path = [
            "mailbox",
            "messages",
            message.stable_id.as_str(),
            "mime",
            path.as_str(),
            "attachment",
        ];
        let attachment_id = node_id(
            ids,
            &attachment_path,
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
    let mut children = Vec::new();
    for child in &part.children {
        children.push(project_part(graph, ids, &id, message, child)?);
    }
    if part.content_type.essence == "multipart/alternative"
        && let Some(primary) = children.first()
    {
        for (child, child_part) in children.iter().skip(1).zip(part.children.iter().skip(1)) {
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
