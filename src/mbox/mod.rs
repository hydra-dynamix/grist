//! Ordered, streaming MBOX parsing over the inert RFC 5322/MIME parser.

mod graph;
mod model;
mod parser;

pub use model::*;
pub use parser::{MboxStream, MboxStreamEvent, stream_mbox};

use crate::core::{
    ArtifactKind, BatchResult, BudgetProfile, BudgetSelection, Envelope, Hashes, OperationControl,
    OperationKind, OperationStatus, ParserInfo, RequestId, SchemaVersion, SourceInfo,
};
use std::collections::BTreeMap;

const PARSER: &str = "grist.mbox";
pub type MboxEnvelope = Envelope<MboxDocument>;

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("grist-mbox", env!("CARGO_PKG_VERSION"))
        .with_feature("email-message")
        .with_specification_version("RFC 4155; mboxo; mboxrd; mboxcl; mboxcl2")
}

pub fn parse_mbox(bytes: &[u8], source: SourceInfo, options: &MboxOptions) -> MboxEnvelope {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .expect("trusted budget is valid");
    parse_mbox_with_operation_control(bytes, source, options, &control)
}

pub fn parse_mbox_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &MboxOptions,
    control: &OperationControl,
) -> MboxEnvelope {
    let digest = crate::core::options_digest(options).expect("mbox options serialize");
    let request_id = RequestId::new("mbox/batch").expect("static request ID is valid");
    let batch = BatchResult::collect(stream_mbox(
        bytes,
        source.clone(),
        options,
        request_id,
        control.clone(),
    ))
    .expect("mbox stream obeys the shared stream protocol");
    let messages = batch
        .items
        .into_iter()
        .map(|item| item.payload)
        .collect::<Vec<_>>();
    let payload = MboxDocument {
        schema_version: SchemaVersion::MBOX_V1.to_string(),
        variant: infer_variant(&messages),
        thread_evidence: aggregate_threads(&messages),
        messages,
        diagnostics: batch.diagnostics.clone(),
        complete: batch.status == OperationStatus::Complete,
    };
    let envelope = match batch.status {
        OperationStatus::Complete => Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Mbox,
            source,
            parser_info(),
            digest,
            SchemaVersion::MBOX_V1,
            payload,
        ),
        OperationStatus::Partial => Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Mbox,
            source,
            parser_info(),
            digest,
            SchemaVersion::MBOX_V1,
            Some(payload),
        ),
        status => Envelope::without_payload(
            OperationKind::Parse,
            ArtifactKind::Mbox,
            status,
            source,
            parser_info(),
            digest,
            SchemaVersion::MBOX_V1,
        )
        .expect("stream terminal status is valid"),
    };
    envelope
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_diagnostics(batch.diagnostics)
        .with_canonical_payload_identity()
        .expect("mbox payload serializes")
}

fn infer_variant(messages: &[MboxMessage]) -> MboxVariant {
    let with_length = messages
        .iter()
        .filter(|message| message.content_length.is_some())
        .count();
    let escaped = messages
        .iter()
        .flat_map(|message| &message.escaped_from_lines)
        .collect::<Vec<_>>();
    if with_length > 0 && with_length < messages.len() {
        return MboxVariant::Mixed;
    }
    if with_length > 0 {
        return if escaped.is_empty() {
            MboxVariant::Mboxcl2
        } else {
            MboxVariant::Mboxcl
        };
    }
    if escaped.iter().any(|line| line.raw_prefix_length > 1) {
        MboxVariant::Mboxrd
    } else if messages.iter().any(|message| message.separator.is_some()) {
        MboxVariant::Mboxo
    } else {
        MboxVariant::Unknown
    }
}

fn aggregate_threads(messages: &[MboxMessage]) -> MboxAggregateThreadEvidence {
    let mut evidence = MboxAggregateThreadEvidence::default();
    let mut by_message_id = BTreeMap::new();
    let mut subjects = BTreeMap::<String, Vec<String>>::new();
    for message in messages {
        let Some(email) = &message.email else {
            continue;
        };
        if let Some(message_id) = &email.thread.message_id {
            by_message_id
                .entry(message_id.decoded.clone())
                .or_insert_with(|| message.stable_id.clone());
            evidence.message_ids.push(MboxMessageIdEvidence {
                stable_id: message.stable_id.clone(),
                message_id: message_id.decoded.clone(),
            });
        }
        if let Some(subject) = &email.thread.normalized_subject_hint
            && !subject.is_empty()
        {
            subjects
                .entry(subject.clone())
                .or_default()
                .push(message.stable_id.clone());
        }
    }
    for message in messages {
        let Some(email) = &message.email else {
            continue;
        };
        for referenced in &email.thread.in_reply_to {
            let target = by_message_id.get(&referenced.value).cloned();
            evidence.links.push(MboxThreadLink {
                source_stable_id: message.stable_id.clone(),
                resolved: target.is_some(),
                target_stable_id: target,
                referenced_message_id: referenced.value.clone(),
                relation: MboxThreadRelation::InReplyTo,
            });
        }
        for referenced in &email.thread.references {
            let target = by_message_id.get(&referenced.value).cloned();
            evidence.links.push(MboxThreadLink {
                source_stable_id: message.stable_id.clone(),
                resolved: target.is_some(),
                target_stable_id: target,
                referenced_message_id: referenced.value.clone(),
                relation: MboxThreadRelation::Reference,
            });
        }
    }
    evidence.subject_groups = subjects
        .into_iter()
        .map(
            |(normalized_subject, message_stable_ids)| MboxSubjectGroup {
                normalized_subject,
                message_stable_ids,
            },
        )
        .collect();
    evidence
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{LocationComponent, StreamEvent};

    const TWO_MESSAGES: &str = concat!(
        "From alice@example.test Fri Jul  8 12:08:34 2022\n",
        "Message-ID: <one@example.test>\nSubject: Re: Topic\n\n",
        ">From body\n",
        "From bob@example.test Fri Jul  8 12:09:34 2022\n",
        "Message-ID: <two@example.test>\nIn-Reply-To: <one@example.test>\n",
        "References: <one@example.test>\nSubject: Topic\n\nsecond\n"
    );

    #[test]
    fn ordered_messages_escape_provenance_and_threads_survive() {
        let envelope = parse_mbox(
            TWO_MESSAGES.as_bytes(),
            SourceInfo::new("mailbox.mbox"),
            &MboxOptions::default(),
        );
        assert_eq!(envelope.status, OperationStatus::Complete);
        let document = envelope.payload.unwrap();
        assert_eq!(document.messages.len(), 2);
        assert_eq!(document.messages[0].ordinal, 1);
        assert_eq!(document.messages[0].escaped_from_lines.len(), 1);
        assert!(document.thread_evidence.links[0].resolved);
        assert_eq!(document.variant, MboxVariant::Mboxo);
        assert!(matches!(
            &document.messages[0]
                .email
                .as_ref()
                .unwrap()
                .mime
                .locator
                .components()[0],
            LocationComponent::TextRange { .. }
        ));
    }

    #[test]
    fn stream_and_batch_message_payloads_are_identical() {
        let control = OperationControl::new(
            &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
            Default::default(),
        )
        .unwrap();
        let streamed = stream_mbox(
            TWO_MESSAGES.as_bytes(),
            SourceInfo::new("mailbox.mbox"),
            &MboxOptions::default(),
            RequestId::new("test").unwrap(),
            control,
        )
        .filter_map(|event| match event {
            StreamEvent::Item { item } => Some(item.payload),
            StreamEvent::Terminal { .. } => None,
        })
        .collect::<Vec<_>>();
        let batch = parse_mbox(
            TWO_MESSAGES.as_bytes(),
            SourceInfo::new("mailbox.mbox"),
            &MboxOptions::default(),
        )
        .payload
        .unwrap();
        assert_eq!(streamed, batch.messages);
    }
}
