use super::{CalendarEvent, ICalendarDocument, VCard, VCardDocument};
use crate::core::{SchemaVersion, SourceLocator};
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, GraphIdGenerator, ProjectionAddress, ToDocumentGraph,
    TransformError,
};

impl ToDocumentGraph for ICalendarDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::ICALENDAR_V1, "icalendar")
            .map_err(error)?;
        let mut graph =
            DocumentGraph::new(context.graph_id, DocumentKind::Other("icalendar".into()))
                .with_projection(
                    "icalendar",
                    SchemaVersion::ICALENDAR_V1,
                    "grist.icalendar.to-document-graph.v1",
                );
        graph.source = context.source;
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "properties".into(),
            serde_json::to_value(&self.properties).map_err(error)?,
        );
        graph.attrs.insert(
            "components".into(),
            serde_json::to_value(&self.components).map_err(error)?,
        );
        graph.attrs.insert(
            "external_references".into(),
            serde_json::to_value(&self.external_references).map_err(error)?,
        );
        graph.attrs.insert(
            "method".into(),
            serde_json::to_value(&self.method).map_err(error)?,
        );
        let root = node_id(&ids, &["calendar"], "calendar", Some(self.locator.clone()))?;
        graph.add_node(
            DocumentNode::new(&root, DocumentNodeKind::Document)
                .with_name(
                    self.product_id
                        .as_ref()
                        .map(|value| value.value.clone())
                        .unwrap_or_else(|| "iCalendar".into()),
                )
                .with_locator(self.locator.clone())
                .with_ordinal(0),
        );
        for timezone in &self.time_zones {
            let timezone_name = timezone
                .timezone_id
                .as_ref()
                .map(|value| value.value.as_str())
                .unwrap_or("unnamed");
            let timezone_id = node_id(
                &ids,
                &["calendar", "timezones", &timezone.ordinal.to_string()],
                timezone_name,
                Some(timezone.locator.clone()),
            )?;
            graph.add_node(
                DocumentNode::new(&timezone_id, DocumentNodeKind::Other("time_zone".into()))
                    .with_name(timezone_name)
                    .with_locator(timezone.locator.clone())
                    .with_ordinal(timezone.ordinal)
                    .with_attr(
                        "observances",
                        serde_json::to_value(&timezone.observances).map_err(error)?,
                    )
                    .with_attr(
                        "properties",
                        serde_json::to_value(&timezone.properties).map_err(error)?,
                    ),
            );
            graph.add_contains(&root, &timezone_id);
        }
        let mut event_ids = Vec::new();
        for event in &self.events {
            let event_id = project_event(&mut graph, &ids, &root, event)?;
            event_ids.push((
                event.uid.as_ref().map(|uid| uid.value.to_ascii_lowercase()),
                event_id,
            ));
        }
        for (event, (_, event_id)) in self.events.iter().zip(event_ids.iter()) {
            for (ordinal, relationship) in event.relationships.iter().enumerate() {
                let target = event_ids
                    .iter()
                    .find(|(uid, _)| {
                        uid.as_deref() == Some(&relationship.target_uid.to_ascii_lowercase())
                    })
                    .map(|(_, id)| id.clone())
                    .unwrap_or_else(|| {
                        let id = node_id(
                            &ids,
                            &[
                                "calendar",
                                "events",
                                &event.ordinal.to_string(),
                                "relations",
                                &ordinal.to_string(),
                            ],
                            &relationship.target_uid,
                            Some(relationship.locator.clone()),
                        )
                        .expect("relationship reference identity is valid");
                        graph.add_node(
                            DocumentNode::new(&id, DocumentNodeKind::Reference)
                                .with_text(relationship.target_uid.clone())
                                .with_locator(relationship.locator.clone())
                                .with_ordinal(ordinal),
                        );
                        graph.add_contains(event_id, &id);
                        id
                    });
                graph.add_edge(
                    DocumentEdge::explicit(
                        event_id.clone(),
                        DocumentRelation::References,
                        target,
                        relationship.locator.clone(),
                    )
                    .with_attr(
                        "relation_type",
                        relationship
                            .relation_type
                            .clone()
                            .unwrap_or_else(|| "PARENT".into()),
                    ),
                );
            }
        }
        graph.diagnostics = self.diagnostics.clone();
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn project_event(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    root: &str,
    event: &CalendarEvent,
) -> Result<String, TransformError> {
    let native = event
        .uid
        .as_ref()
        .map(|uid| uid.value.as_str())
        .unwrap_or(event.component_kind.as_str());
    let id = node_id(
        ids,
        &["calendar", "events", &event.ordinal.to_string()],
        native,
        Some(event.locator.clone()),
    )?;
    let mut node = DocumentNode::new(&id, DocumentNodeKind::Other("calendar_event".into()))
        .with_locator(event.locator.clone())
        .with_ordinal(event.ordinal)
        .with_attr("component_kind", event.component_kind.clone())
        .with_attr(
            "temporal",
            serde_json::json!({
                "start": event.start,
                "end": event.end,
                "due": event.due,
                "duration": event.duration,
                "recurrence_id": event.recurrence_id,
            }),
        )
        .with_attr(
            "properties",
            serde_json::to_value(&event.properties).map_err(error)?,
        );
    if let Some(summary) = &event.summary {
        node = node.with_name(summary.value.clone());
    }
    if let Some(description) = &event.description {
        node = node.with_text(description.value.clone());
    }
    graph.add_node(node);
    graph.add_contains(root, &id);
    for (ordinal, attendee) in event.attendees.iter().enumerate() {
        let attendee_id = node_id(
            ids,
            &[
                "calendar",
                "events",
                &event.ordinal.to_string(),
                "attendees",
                &ordinal.to_string(),
            ],
            &attendee.uri,
            Some(attendee.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(
                &attendee_id,
                DocumentNodeKind::Other("calendar_attendee".into()),
            )
            .with_name(
                attendee
                    .common_name
                    .clone()
                    .unwrap_or_else(|| attendee.uri.clone()),
            )
            .with_text(attendee.uri.clone())
            .with_locator(attendee.locator.clone())
            .with_ordinal(ordinal)
            .with_attr(
                "participation_status",
                serde_json::to_value(&attendee.participation_status).map_err(error)?,
            )
            .with_attr("role", serde_json::to_value(&attendee.role).map_err(error)?),
        );
        graph.add_contains(&id, &attendee_id);
    }
    for (ordinal, recurrence) in event.recurrence_rules.iter().enumerate() {
        let recurrence_id = node_id(
            ids,
            &[
                "calendar",
                "events",
                &event.ordinal.to_string(),
                "recurrence",
                &ordinal.to_string(),
            ],
            &recurrence.raw,
            Some(recurrence.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(
                &recurrence_id,
                DocumentNodeKind::Other("recurrence_rule".into()),
            )
            .with_text(recurrence.raw.clone())
            .with_locator(recurrence.locator.clone())
            .with_ordinal(ordinal)
            .with_attr("rule", serde_json::to_value(recurrence).map_err(error)?),
        );
        graph.add_contains(&id, &recurrence_id);
    }
    for (ordinal, attachment) in event.attachments.iter().enumerate() {
        let attachment_id = node_id(
            ids,
            &[
                "calendar",
                "events",
                &event.ordinal.to_string(),
                "attachments",
                &ordinal.to_string(),
            ],
            &attachment.raw_value,
            Some(attachment.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(&attachment_id, DocumentNodeKind::Attachment)
                .with_name(
                    attachment
                        .media_type
                        .clone()
                        .unwrap_or_else(|| "calendar attachment".into()),
                )
                .with_text(attachment.uri.clone().unwrap_or_default())
                .with_locator(attachment.locator.clone())
                .with_ordinal(ordinal)
                .with_attr(
                    "attachment",
                    serde_json::to_value(attachment).map_err(error)?,
                ),
        );
        graph.add_contains(&id, &attachment_id);
        graph.add_edge(DocumentEdge::explicit(
            attachment_id,
            DocumentRelation::AttachmentOf,
            id.clone(),
            attachment.locator.clone(),
        ));
    }
    Ok(id)
}

impl ToDocumentGraph for VCardDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let ids = context
            .identity_generator(SchemaVersion::VCARD_V1, "vcard")
            .map_err(error)?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Other("vcard".into()))
            .with_projection(
                "vcard",
                SchemaVersion::VCARD_V1,
                "grist.vcard.to-document-graph.v1",
            );
        graph.source = context.source;
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "external_references".into(),
            serde_json::to_value(&self.external_references).map_err(error)?,
        );
        let root = node_id(&ids, &["contacts"], "contacts", Some(self.locator.clone()))?;
        graph.add_node(
            DocumentNode::new(&root, DocumentNodeKind::Document)
                .with_name("vCard contacts")
                .with_locator(self.locator.clone())
                .with_ordinal(0),
        );
        for card in &self.cards {
            project_card(&mut graph, &ids, &root, card)?;
        }
        graph.diagnostics = self.diagnostics.clone();
        graph.finalize_projection(&ids).map_err(error)?;
        Ok(graph)
    }
}

fn project_card(
    graph: &mut DocumentGraph,
    ids: &GraphIdGenerator,
    root: &str,
    card: &VCard,
) -> Result<(), TransformError> {
    let native = card
        .uid
        .as_ref()
        .map(|uid| uid.value.as_str())
        .or_else(|| card.formatted_names.first().map(|name| name.value.as_str()))
        .unwrap_or("contact");
    let id = node_id(
        ids,
        &["contacts", &card.ordinal.to_string()],
        native,
        Some(card.locator.clone()),
    )?;
    let mut node = DocumentNode::new(&id, DocumentNodeKind::Other("contact".into()))
        .with_locator(card.locator.clone())
        .with_ordinal(card.ordinal)
        .with_attr("card", serde_json::to_value(card).map_err(error)?);
    if let Some(name) = card.formatted_names.first() {
        node = node
            .with_name(name.value.clone())
            .with_text(name.value.clone());
    }
    graph.add_node(node);
    graph.add_contains(root, &id);
    for (ordinal, communication) in card.communications.iter().enumerate() {
        let communication_id = node_id(
            ids,
            &[
                "contacts",
                &card.ordinal.to_string(),
                "communications",
                &ordinal.to_string(),
            ],
            &communication.value,
            Some(communication.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(&communication_id, DocumentNodeKind::Metadata)
                .with_name(communication.kind.clone())
                .with_text(communication.value.clone())
                .with_locator(communication.locator.clone())
                .with_ordinal(ordinal)
                .with_attr(
                    "communication",
                    serde_json::to_value(communication).map_err(error)?,
                ),
        );
        graph.add_contains(&id, &communication_id);
    }
    for (ordinal, attachment) in card.attachments.iter().enumerate() {
        let attachment_id = node_id(
            ids,
            &[
                "contacts",
                &card.ordinal.to_string(),
                "attachments",
                &ordinal.to_string(),
            ],
            &attachment.raw_value,
            Some(attachment.locator.clone()),
        )?;
        graph.add_node(
            DocumentNode::new(&attachment_id, DocumentNodeKind::Attachment)
                .with_name(attachment.property_name.clone())
                .with_text(attachment.uri.clone().unwrap_or_default())
                .with_locator(attachment.locator.clone())
                .with_ordinal(ordinal)
                .with_attr(
                    "attachment",
                    serde_json::to_value(attachment).map_err(error)?,
                ),
        );
        graph.add_contains(&id, &attachment_id);
        graph.add_edge(DocumentEdge::explicit(
            attachment_id,
            DocumentRelation::AttachmentOf,
            id.clone(),
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
