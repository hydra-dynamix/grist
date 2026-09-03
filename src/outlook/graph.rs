use super::{MapiProperty, MapiValue, MsgAttachment, OutlookMsgDocument};
use crate::core::{SchemaVersion, SourceLocator};
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, ToDocumentGraph,
    TransformError,
};

impl ToDocumentGraph for OutlookMsgDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::OUTLOOK_MSG_V1, "outlook-msg")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Email).with_projection(
            "outlook-msg",
            SchemaVersion::OUTLOOK_MSG_V1,
            "grist.outlook.msg.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "compound_file".into(),
            serde_json::to_value(&self.compound_file).map_err(error)?,
        );
        graph.attrs.insert(
            "named_properties".into(),
            serde_json::to_value(&self.named_properties).map_err(error)?,
        );
        let root_locator = self
            .properties
            .first()
            .map(|property| property.locator.clone())
            .or_else(|| self.bodies.first().map(|body| body.locator.clone()))
            .or_else(|| {
                self.unknown_objects
                    .first()
                    .map(|item| item.locator.clone())
            });
        let root = node_id(&ids, &["message"], "message", root_locator.clone())?;
        let mut root_node = DocumentNode::new(&root, DocumentNodeKind::Email)
            .with_ordinal(0)
            .with_attr(
                "message_class",
                serde_json::to_value(&self.message_class).map_err(error)?,
            )
            .with_attr("sender", serde_json::to_value(&self.sender).map_err(error)?)
            .with_attr("thread", serde_json::to_value(&self.thread).map_err(error)?)
            .with_attr("dates", serde_json::to_value(&self.dates).map_err(error)?)
            .with_attr("encrypted", self.encrypted)
            .with_attr("signed", self.signed);
        if let Some(locator) = root_locator {
            root_node = root_node.with_locator(locator);
        }
        if let Some(subject) = &self.subject {
            root_node = root_node.with_name(subject.text.clone());
        }
        graph.add_node(root_node);
        project_properties(&mut graph, &ids, &root, "message", &self.properties)?;

        let mut body_ids = Vec::new();
        for body in &self.bodies {
            let body_id = node_id(
                &ids,
                &["message", "body", &body.ordinal.to_string()],
                &body.source_property_tag,
                Some(body.locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&body_id, DocumentNodeKind::MessageBody)
                    .with_text(body.text.clone())
                    .with_locator(body.locator.clone())
                    .with_ordinal(body.ordinal)
                    .with_attr("body_kind", serde_json::to_value(body.kind).map_err(error)?)
                    .with_attr("charset", body.charset.clone())
                    .with_attr("lossy", body.lossy)
                    .with_attr(
                        "rtf_compression",
                        serde_json::to_value(&body.rtf_compression).map_err(error)?,
                    )
                    .with_attr("active_content_inert", body.active_content_inert),
            );
            graph.add_contains(&root, &body_id);
            body_ids.push((body_id, body.locator.clone()));
        }
        if let Some((primary, _)) = body_ids.first() {
            for (alternative, locator) in body_ids.iter().skip(1) {
                graph.add_edge(DocumentEdge::explicit(
                    alternative.clone(),
                    DocumentRelation::AlternativeRepresentationOf,
                    primary.clone(),
                    locator.clone(),
                ));
            }
        }
        for recipient in &self.recipients {
            let id = node_id(
                &ids,
                &["message", "recipient", &recipient.ordinal.to_string()],
                &recipient.storage_path,
                Some(recipient.locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&id, DocumentNodeKind::Record)
                    .with_name(
                        recipient
                            .display_name
                            .as_ref()
                            .map(|fact| fact.text.clone())
                            .or_else(|| {
                                recipient
                                    .smtp_address
                                    .as_ref()
                                    .map(|fact| fact.text.clone())
                            })
                            .unwrap_or_else(|| format!("recipient {}", recipient.ordinal + 1)),
                    )
                    .with_locator(recipient.locator.clone())
                    .with_ordinal(recipient.ordinal)
                    .with_attr(
                        "recipient_type",
                        serde_json::to_value(recipient.recipient_type).map_err(error)?,
                    )
                    .with_attr(
                        "address_type",
                        serde_json::to_value(&recipient.address_type).map_err(error)?,
                    )
                    .with_attr(
                        "email_address",
                        serde_json::to_value(&recipient.email_address).map_err(error)?,
                    )
                    .with_attr(
                        "smtp_address",
                        serde_json::to_value(&recipient.smtp_address).map_err(error)?,
                    )
                    .with_attr(
                        "unknown_objects",
                        serde_json::to_value(&recipient.unknown_objects).map_err(error)?,
                    ),
            );
            graph.add_contains(&root, &id);
            project_properties(
                &mut graph,
                &ids,
                &id,
                &format!("recipient/{}", recipient.ordinal),
                &recipient.properties,
            )?;
        }
        for attachment in &self.attachments {
            project_attachment(&mut graph, &ids, &root, attachment)?;
        }
        for (ordinal, unknown) in self.unknown_objects.iter().enumerate() {
            let id = node_id(
                &ids,
                &["message", "unknown", &ordinal.to_string()],
                &unknown.path,
                Some(unknown.locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&id, DocumentNodeKind::Unknown)
                    .with_name(unknown.path.clone())
                    .with_locator(unknown.locator.clone())
                    .with_ordinal(ordinal)
                    .with_attr(
                        "outlook_msg_unknown",
                        serde_json::to_value(unknown).map_err(error)?,
                    ),
            );
            graph.add_contains(&root, &id);
        }
        for (ordinal, reply) in self.thread.in_reply_to.iter().enumerate() {
            let locator = self
                .thread
                .internet_message_id
                .as_ref()
                .map(|fact| fact.locator.clone())
                .or_else(|| {
                    self.properties
                        .first()
                        .map(|property| property.locator.clone())
                });
            let id = node_id(
                &ids,
                &["message", "reply", &ordinal.to_string()],
                reply,
                locator.clone(),
            )?;
            let mut node = DocumentNode::new(&id, DocumentNodeKind::Reference)
                .with_text(reply.clone())
                .with_ordinal(ordinal);
            if let Some(locator) = &locator {
                node = node.with_locator(locator.clone());
            }
            graph.add_node(node);
            if let Some(locator) = locator {
                graph.add_edge(DocumentEdge::explicit(
                    root.clone(),
                    DocumentRelation::ReplyTo,
                    id,
                    locator,
                ));
            }
        }
        graph.diagnostics = self.diagnostics.clone();
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn project_properties(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    parent: &str,
    parent_path: &str,
    properties: &[MapiProperty],
) -> Result<(), TransformError> {
    for property in properties {
        let id = node_id(
            ids,
            &[parent_path, "property", &property.ordinal.to_string()],
            &property.property_tag,
            Some(property.locator.clone()),
        )?;
        let mut node = DocumentNode::new(&id, DocumentNodeKind::Metadata)
            .with_name(
                property
                    .canonical_name
                    .clone()
                    .unwrap_or_else(|| property.property_tag.clone()),
            )
            .with_locator(property.locator.clone())
            .with_ordinal(property.ordinal)
            .with_attr("property_tag", property.property_tag.clone())
            .with_attr(
                "property_type",
                serde_json::to_value(&property.property_type).map_err(error)?,
            )
            .with_attr("flags", property.flags)
            .with_attr(
                "table_value",
                serde_json::to_value(&property.table_value).map_err(error)?,
            )
            .with_attr(
                "named",
                serde_json::to_value(&property.named).map_err(error)?,
            )
            .with_attr(
                "value",
                serde_json::to_value(&property.value).map_err(error)?,
            )
            .with_attr("stream_paths", property.stream_paths.clone());
        if let MapiValue::String { text, .. } = &property.value {
            node = node.with_text(text.clone());
        }
        graph.add_node(node);
        graph.add_contains(parent, &id);
    }
    Ok(())
}

fn project_attachment(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    root: &str,
    attachment: &MsgAttachment,
) -> Result<(), TransformError> {
    let id = node_id(
        ids,
        &["message", "attachment", &attachment.ordinal.to_string()],
        &attachment.storage_path,
        Some(attachment.locator.clone()),
    )?;
    graph.add_node(
        DocumentNode::new(&id, DocumentNodeKind::Attachment)
            .with_name(
                attachment
                    .filename
                    .as_ref()
                    .map(|fact| fact.text.clone())
                    .unwrap_or_else(|| format!("attachment {}", attachment.ordinal + 1)),
            )
            .with_locator(attachment.locator.clone())
            .with_ordinal(attachment.ordinal)
            .with_attr(
                "method",
                serde_json::to_value(attachment.method).map_err(error)?,
            )
            .with_attr(
                "mime_type",
                serde_json::to_value(&attachment.mime_type).map_err(error)?,
            )
            .with_attr(
                "content_id",
                serde_json::to_value(&attachment.content_id).map_err(error)?,
            )
            .with_attr(
                "content_location",
                serde_json::to_value(&attachment.content_location).map_err(error)?,
            )
            .with_attr(
                "artifact",
                serde_json::to_value(&attachment.artifact).map_err(error)?,
            )
            .with_attr(
                "rendering_position",
                serde_json::to_value(attachment.rendering_position).map_err(error)?,
            )
            .with_attr(
                "declared_size",
                serde_json::to_value(attachment.declared_size).map_err(error)?,
            )
            .with_attr(
                "unknown_objects",
                serde_json::to_value(&attachment.unknown_objects).map_err(error)?,
            ),
    );
    graph.add_contains(root, &id);
    graph.add_edge(DocumentEdge::explicit(
        id.clone(),
        if attachment.content_id.is_some() {
            DocumentRelation::EmbeddedIn
        } else {
            DocumentRelation::AttachmentOf
        },
        root.to_string(),
        attachment.locator.clone(),
    ));
    project_properties(
        graph,
        ids,
        &id,
        &format!("attachment/{}", attachment.ordinal),
        &attachment.properties,
    )?;
    if let Some(embedded) = &attachment.embedded_message {
        let embedded_id = node_id(
            ids,
            &[
                "message",
                "attachment",
                &attachment.ordinal.to_string(),
                "embedded-message",
            ],
            &format!("embedded:{}", attachment.storage_path),
            embedded
                .properties
                .first()
                .map(|property| property.locator.clone())
                .or_else(|| Some(attachment.locator.clone())),
        )?;
        graph.add_node(
            DocumentNode::new(&embedded_id, DocumentNodeKind::Email)
                .with_name(
                    embedded
                        .subject
                        .as_ref()
                        .map(|fact| fact.text.clone())
                        .unwrap_or_else(|| "embedded message".to_string()),
                )
                .with_locator(attachment.locator.clone())
                .with_ordinal(0)
                .with_attr(
                    "outlook_msg",
                    serde_json::to_value(embedded).map_err(error)?,
                ),
        );
        graph.add_contains(&id, &embedded_id);
        graph.add_edge(DocumentEdge::explicit(
            embedded_id,
            DocumentRelation::EmbeddedIn,
            id,
            attachment.locator.clone(),
        ));
    }
    Ok(())
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
