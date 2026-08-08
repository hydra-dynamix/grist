#![cfg(feature = "email-message")]

use grist::calendar_contact::{
    AttachmentValueKind, ICalendarOptions, VCardOptions, parse_icalendar, parse_vcard,
};
use grist::core::{ContentIdentity, LocationComponent, OperationStatus, SourceInfo};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::email::{EmailOptions, parse_email};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::segment::{SegmentOptions, segment_document_graph};

const CALENDAR: &str = "BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Grist Tests//EN
METHOD:REQUEST
BEGIN:VTIMEZONE
TZID:America/Vancouver
TZURL:https://example.test/timezone.ics
BEGIN:STANDARD
DTSTART:19701101T020000
TZOFFSETFROM:-0700
TZOFFSETTO:-0800
RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU
END:STANDARD
END:VTIMEZONE
BEGIN:VEVENT
UID:event-1@example.test
DTSTAMP:20260808T120000Z
DTSTART;TZID=America/Vancouver:20260810T090000
DTEND;TZID=America/Vancouver:20260810T100000
SUMMARY:Calendar parser 
 continuation
DESCRIPTION:Inert event
ATTENDEE;CN=Reader;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:reader@example.test
RRULE:FREQ=WEEKLY;COUNT=4;BYDAY=MO,WE
RDATE;TZID=America/Vancouver:20260901T090000,20260902T090000
EXDATE;TZID=America/Vancouver:20260817T090000
ATTACH;FMTTYPE=application/pdf:https://example.test/agenda.pdf
ATTACH;VALUE=BINARY;ENCODING=BASE64;FMTTYPE=text/plain:SGVsbG8=
URL:https://example.test/event
RELATED-TO;RELTYPE=PARENT:event-parent@example.test
END:VEVENT
END:VCALENDAR
";

#[test]
fn icalendar_preserves_recurrence_attendees_timezones_attachments_and_locators() {
    let source =
        SourceInfo::new("invite.ics").with_declared_mime_type("text/calendar; method=REQUEST");
    let envelope = parse_icalendar(CALENDAR.as_bytes(), source, &ICalendarOptions::default());
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.mime_method.as_deref(), Some("REQUEST"));
    assert_eq!(document.events.len(), 1);
    assert_eq!(document.time_zones.len(), 1);
    let event = &document.events[0];
    assert_eq!(
        event.summary.as_ref().unwrap().value,
        "Calendar parser continuation"
    );
    assert_eq!(event.attendees.len(), 1);
    assert_eq!(event.attendees[0].rsvp, Some(true));
    assert_eq!(
        event.recurrence_rules[0].frequency.as_deref(),
        Some("WEEKLY")
    );
    assert_eq!(event.recurrence_rules[0].by_day, ["MO", "WE"]);
    assert_eq!(event.recurrence_dates.len(), 2);
    assert_eq!(event.attachments.len(), 2);
    assert_eq!(event.attachments[0].value_kind, AttachmentValueKind::Uri);
    assert!(!event.attachments[0].resolved);
    assert_eq!(event.attachments[1].decoded_bytes, Some(5));
    assert!(event.attachments[1].decoded_sha256.is_some());
    assert!(event.locator.components().iter().any(|component| matches!(
        component,
        LocationComponent::RecordRange { collection, .. }
            if collection == "icalendar.components"
    )));
    let summary_range = event.summary.as_ref().unwrap().locator.components()[0]
        .as_text_range()
        .unwrap();
    assert_eq!(
        &CALENDAR.as_bytes()[summary_range.byte_start..summary_range.byte_end],
        b"SUMMARY:Calendar parser \n continuation"
    );
    assert!(
        document
            .external_references
            .iter()
            .all(|reference| !reference.resolved)
    );

    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:calendar"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Other("calendar_event".into()))
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::AttachmentOf)
    );
    let payload_identity = envelope.identity.as_ref().unwrap();
    let graph_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        payload_identity,
        &graph_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| !segment.locators.is_empty())
    );
}

#[test]
fn vcard_variants_groups_structured_fields_binary_and_relationships_survive() {
    let cards = concat!(
        "BEGIN:VCARD\r\n",
        "VERSION:2.1\r\n",
        "item1.N;CHARSET=ISO-8859-1;ENCODING=QUOTED-PRINTABLE:Doe;Andr=E9;;Dr.;Jr.\r\n",
        "item1.FN:Dr. Andre Doe Jr.\r\n",
        "ADR;HOME;PREF:;;123 Main St.;Vancouver;BC;V5K 0A1;Canada\r\n",
        "TEL;TYPE=voice,cell;PREF=1:tel:+1-555-0100\r\n",
        "EMAIL;TYPE=internet:andre@example.test\r\n",
        "URL:https://example.test/contact\r\n",
        "PHOTO;ENCODING=BASE64;TYPE=JPEG:SGVsbG8=\r\n",
        "RELATED;TYPE=friend:urn:uuid:friend-1\r\n",
        "UID:urn:uuid:contact-1\r\n",
        "END:VCARD\r\n",
        "BEGIN:VCARD\n",
        "VERSION:4.0\n",
        "FN:Second Contact\n",
        "N:Contact;Second;;;\n",
        "IMPP:xmpp:second@example.test\n",
        "END:VCARD\n",
    );
    let envelope = parse_vcard(
        cards.as_bytes(),
        SourceInfo::new("contacts.vcf"),
        &VCardOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.cards.len(), 2);
    assert_eq!(
        document.cards[0].name.as_ref().unwrap().given,
        ["Andr\u{e9}"]
    );
    assert_eq!(document.cards[0].addresses[0].types, ["HOME", "PREF"]);
    assert_eq!(document.cards[0].communications.len(), 3);
    assert_eq!(document.cards[0].attachments[0].decoded_bytes, Some(5));
    assert!(!document.cards[0].related[0].resolved);
    assert!(
        document
            .external_references
            .iter()
            .all(|item| !item.resolved)
    );
    assert!(matches!(
        document.cards[0].locator.components().last().unwrap(),
        LocationComponent::RecordRange { collection, .. } if collection == "vcard.cards"
    ));
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:vcard"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Other("contact".into()))
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::AttachmentOf)
    );
}

#[test]
fn extensionless_detection_registry_and_malformed_limits_are_explicit() {
    let registry = builtin_parser_registry().unwrap();
    for (bytes, expected_kind, expected_format) in [
        (CALENDAR.as_bytes(), ContentKind::ICalendar, "icalendar"),
        (
            b"BEGIN:VCARD\r\nVERSION:4.0\r\nFN:A\r\nEND:VCARD\r\n".as_slice(),
            ContentKind::VCard,
            "vcard",
        ),
    ] {
        let detected = detect_with_registry(
            std::path::Path::new("extensionless"),
            bytes,
            None,
            None,
            &grist::core::Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detected.status, DetectionStatus::Selected);
        assert_eq!(detected.content_kind, expected_kind);
        assert_eq!(detected.candidates[0].identity.format, expected_format);
        assert!(matches!(
            registry.select_format(expected_format),
            ParserSelection::Available(_)
        ));
    }

    let malformed =
        b"BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nRRULE:COUNT=nope\r\nEND:VTODO\r\n";
    let envelope = parse_icalendar(
        malformed,
        SourceInfo::new("broken.ics"),
        &ICalendarOptions {
            max_components: 2,
            ..ICalendarOptions::default()
        },
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(envelope.payload().is_some());
    assert!(envelope.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "icalendar.structure.mismatched_end"
            || diagnostic.code == "icalendar.structure.unclosed_component"
    }));

    let limited = parse_vcard(
        b"BEGIN:VCARD\r\nVERSION:4.0\r\nFN:A\r\nNOTE:B\r\nEND:VCARD\r\n",
        SourceInfo::new("limited.vcf"),
        &VCardOptions {
            max_properties: 2,
            ..VCardOptions::default()
        },
    );
    assert_eq!(limited.status, OperationStatus::Partial);
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "vcard.limit.properties")
    );
}

#[test]
fn calendar_mime_part_is_an_inert_nested_attachment_and_never_answers() {
    let message = format!(
        "From: owner@example.test\r\nTo: reader@example.test\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\r\n{CALENDAR}\r\n--x--\r\n"
    );
    let envelope = parse_email(
        message.as_bytes(),
        SourceInfo::new("invite.eml"),
        &EmailOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let calendar_part = &envelope.payload().unwrap().mime.children[0];
    let attachment = calendar_part.attachment.as_ref().unwrap();
    assert_eq!(attachment.nested.as_ref().unwrap().format, "icalendar");
    assert_eq!(
        attachment.nested.as_ref().unwrap().status,
        OperationStatus::Complete
    );
    assert_eq!(
        attachment.nested.as_ref().unwrap().envelope["payload"]["method"]["value"],
        "REQUEST"
    );
    assert_eq!(
        attachment.nested.as_ref().unwrap().envelope["payload"]["external_references"][0]["resolved"],
        false
    );
}

#[cfg(feature = "schemas")]
#[test]
fn calendar_and_contact_schemas_are_registered() {
    for name in [
        "icalendar",
        "icalendar-envelope",
        "icalendar-options",
        "vcard",
        "vcard-envelope",
        "vcard-options",
    ] {
        assert!(grist::schema::schema_json(name).is_some(), "missing {name}");
    }
}
