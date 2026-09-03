use super::{
    AttachmentValueKind, CalendarAttendee, CalendarComponent, CalendarEvent, CalendarRelationship,
    CalendarTimeZone, ICalendarDocument, ICalendarOptions, InertAttachment, InertExternalReference,
    LocatedText, RecordParameter, RecordProperty, RecurrencePart, RecurrenceRule, StructuredName,
    TemporalValue, TemporalValueKind, TimeZoneObservance, VCard, VCardAddress, VCardCommunication,
    VCardDocument, VCardOptions, VCardRelated,
};
use crate::core::{
    Diagnostic, IndexBase, IndexRange, LineIndex, LocationComponent, OperationControl, SourceInfo,
    SourceLocator, SourceRange,
};
use crate::decode::{DecodeOptions, decode_text};
use sha2::{Digest, Sha256};

const ICALENDAR_PARSER: &str = "grist.icalendar";
const VCARD_PARSER: &str = "grist.vcard";

#[derive(Debug, Clone)]
struct LogicalLine {
    ordinal: usize,
    start: usize,
    end: usize,
    raw: String,
    unfolded: String,
    too_long: bool,
}

#[derive(Debug)]
struct ComponentBuilder {
    ordinal: usize,
    kind: String,
    start: usize,
    properties: Vec<RecordProperty>,
    children: Vec<CalendarComponent>,
}

pub(super) fn parse_icalendar_document(
    bytes: &[u8],
    source: &SourceInfo,
    options: &ICalendarOptions,
    control: &OperationControl,
) -> Result<ICalendarDocument, Box<Diagnostic>> {
    let decoded = decode_text(
        bytes,
        &DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("icalendar")),
    )
    .map_err(|error| Box::new(error.diagnostic().with_parser(ICALENDAR_PARSER)))?;
    control
        .budget()
        .consume_decoded_characters(decoded.text.chars().count() as u64)
        .map_err(|error| Box::new(error.diagnostic(ICALENDAR_PARSER)))?;
    let text = decoded.text.as_str();
    let index = LineIndex::new(text);
    let mut diagnostics = decoded.report.diagnostics.clone();
    let lines = unfold_lines(
        text,
        options.max_unfolded_line_bytes,
        &mut diagnostics,
        ICALENDAR_PARSER,
    );
    let mut stack = Vec::<ComponentBuilder>::new();
    let mut roots = Vec::<CalendarComponent>::new();
    let mut component_ordinal = 0usize;
    let mut property_count = 0usize;

    for line in lines {
        control
            .checkpoint()
            .map_err(|error| Box::new(error.diagnostic(ICALENDAR_PARSER)))?;
        if line.unfolded.trim().is_empty() {
            continue;
        }
        let property = parse_property(
            &line,
            text,
            &index,
            "icalendar.content_lines",
            ICALENDAR_PARSER,
            &mut diagnostics,
        );
        property_count += 1;
        if property_count > options.max_properties {
            diagnostics.push(partial(
                ICALENDAR_PARSER,
                "icalendar.limit.properties",
                "iCalendar property limit was reached",
                Some(property.locator.clone()),
            ));
            break;
        }
        if let Err(error) = control.budget().consume_nodes(1) {
            diagnostics.push(error.diagnostic(ICALENDAR_PARSER).partial());
            break;
        }
        if property.name == "BEGIN" {
            component_ordinal += 1;
            if component_ordinal > options.max_components {
                diagnostics.push(partial(
                    ICALENDAR_PARSER,
                    "icalendar.limit.components",
                    "iCalendar component limit was reached",
                    Some(property.locator.clone()),
                ));
                break;
            }
            let depth = stack.len() + 1;
            if depth > options.max_nesting_depth {
                diagnostics.push(partial(
                    ICALENDAR_PARSER,
                    "icalendar.limit.nesting",
                    format!("component nesting exceeds {}", options.max_nesting_depth),
                    Some(property.locator.clone()),
                ));
                break;
            }
            if let Err(error) = control.budget().observe_nesting_depth(depth as u64) {
                diagnostics.push(error.diagnostic(ICALENDAR_PARSER).partial());
                break;
            }
            if let Err(error) = control.budget().consume_records(1) {
                diagnostics.push(error.diagnostic(ICALENDAR_PARSER).partial());
                break;
            }
            stack.push(ComponentBuilder {
                ordinal: component_ordinal,
                kind: property.value.trim().to_ascii_uppercase(),
                start: line.start,
                properties: Vec::new(),
                children: Vec::new(),
            });
            continue;
        }
        if property.name == "END" {
            let Some(builder) = stack.pop() else {
                diagnostics.push(partial(
                    ICALENDAR_PARSER,
                    "icalendar.structure.orphan_end",
                    format!("END:{} has no matching BEGIN", property.value),
                    Some(property.locator.clone()),
                ));
                continue;
            };
            if !builder.kind.eq_ignore_ascii_case(property.value.trim()) {
                diagnostics.push(partial(
                    ICALENDAR_PARSER,
                    "icalendar.structure.mismatched_end",
                    format!(
                        "BEGIN:{} was closed by END:{}",
                        builder.kind, property.value
                    ),
                    Some(property.locator.clone()),
                ));
            }
            let component = finish_component(builder, line.end, text, &index);
            if let Some(parent) = stack.last_mut() {
                parent.children.push(component);
            } else {
                roots.push(component);
            }
            continue;
        }
        if let Some(component) = stack.last_mut() {
            component.properties.push(property);
        } else {
            diagnostics.push(partial(
                ICALENDAR_PARSER,
                "icalendar.structure.property_outside_calendar",
                "content line occurs outside a component",
                Some(property.locator.clone()),
            ));
        }
    }
    while let Some(builder) = stack.pop() {
        diagnostics.push(partial(
            ICALENDAR_PARSER,
            "icalendar.structure.unclosed_component",
            format!("BEGIN:{} has no END", builder.kind),
            Some(component_locator(
                text,
                &index,
                builder.start,
                text.len(),
                builder.ordinal,
            )),
        ));
        let component = finish_component(builder, text.len(), text, &index);
        if let Some(parent) = stack.last_mut() {
            parent.children.push(component);
        } else {
            roots.push(component);
        }
    }

    let root_index = roots
        .iter()
        .position(|component| component.kind == "VCALENDAR");
    if root_index.is_none() {
        diagnostics.push(partial(
            ICALENDAR_PARSER,
            "icalendar.structure.missing_vcalendar",
            "no VCALENDAR record was found",
            Some(document_locator(text, &index, "icalendar.calendars")),
        ));
    }
    if roots.len() > 1 {
        diagnostics.push(partial(
            ICALENDAR_PARSER,
            "icalendar.structure.multiple_roots",
            "multiple top-level components were retained",
            Some(document_locator(text, &index, "icalendar.calendars")),
        ));
    }
    let root = root_index.and_then(|position| roots.get(position)).cloned();
    let properties = root
        .as_ref()
        .map(|component| component.properties.clone())
        .unwrap_or_default();
    let components = root
        .as_ref()
        .map(|component| component.children.clone())
        .unwrap_or_else(|| roots.clone());
    let mut events = Vec::new();
    let mut time_zones = Vec::new();
    for component in &components {
        collect_calendar_entities(
            component,
            options,
            &mut events,
            &mut time_zones,
            &mut diagnostics,
        );
    }
    account_for_attachments(
        events.iter().flat_map(|event| event.attachments.iter()),
        control,
        ICALENDAR_PARSER,
        &mut diagnostics,
    );
    validate_timezone_references(&events, &time_zones, &mut diagnostics);
    let mut external_references = Vec::new();
    collect_calendar_references(&events, &time_zones, &mut external_references);
    let complete = diagnostics.iter().all(|diagnostic| !diagnostic.partial);
    let mime_media_type = source.declared_mime_type.as_deref().map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or(value)
            .trim()
            .to_ascii_lowercase()
    });
    let mime_method = source
        .declared_mime_type
        .as_deref()
        .and_then(|value| media_parameter(value, "method"));
    Ok(ICalendarDocument {
        schema_version: crate::core::SchemaVersion::ICALENDAR_V1.into(),
        version: first_located(&properties, "VERSION"),
        product_id: first_located(&properties, "PRODID"),
        calendar_scale: first_located(&properties, "CALSCALE"),
        method: first_located(&properties, "METHOD"),
        mime_media_type,
        mime_method,
        properties,
        components,
        events,
        time_zones,
        external_references,
        diagnostics,
        complete,
        locator: root
            .map(|component| component.locator)
            .unwrap_or_else(|| document_locator(text, &index, "icalendar.calendars")),
    })
}

pub(super) fn parse_vcard_document(
    bytes: &[u8],
    source: &SourceInfo,
    options: &VCardOptions,
    control: &OperationControl,
) -> Result<VCardDocument, Box<Diagnostic>> {
    let decoded = decode_text(
        bytes,
        &DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("vcard")),
    )
    .map_err(|error| Box::new(error.diagnostic().with_parser(VCARD_PARSER)))?;
    control
        .budget()
        .consume_decoded_characters(decoded.text.chars().count() as u64)
        .map_err(|error| Box::new(error.diagnostic(VCARD_PARSER)))?;
    let text = decoded.text.as_str();
    let index = LineIndex::new(text);
    let mut diagnostics = decoded.report.diagnostics.clone();
    let lines = unfold_lines(
        text,
        options.max_unfolded_line_bytes,
        &mut diagnostics,
        VCARD_PARSER,
    );
    let mut cards = Vec::new();
    let mut current: Option<(usize, usize, Vec<RecordProperty>)> = None;
    let mut property_count = 0usize;

    for line in lines {
        control
            .checkpoint()
            .map_err(|error| Box::new(error.diagnostic(VCARD_PARSER)))?;
        if line.unfolded.trim().is_empty() {
            continue;
        }
        let property = parse_property(
            &line,
            text,
            &index,
            "vcard.content_lines",
            VCARD_PARSER,
            &mut diagnostics,
        );
        property_count += 1;
        if property_count > options.max_properties {
            diagnostics.push(partial(
                VCARD_PARSER,
                "vcard.limit.properties",
                "vCard property limit was reached",
                Some(property.locator.clone()),
            ));
            break;
        }
        if let Err(error) = control.budget().consume_nodes(1) {
            diagnostics.push(error.diagnostic(VCARD_PARSER).partial());
            break;
        }
        if property.name == "BEGIN" && property.value.eq_ignore_ascii_case("VCARD") {
            if let Some((ordinal, start, properties)) = current.take() {
                diagnostics.push(partial(
                    VCARD_PARSER,
                    "vcard.structure.nested_begin",
                    "a new VCARD began before the previous record ended",
                    Some(card_locator(text, &index, start, line.start, ordinal)),
                ));
                cards.push(build_card(
                    ordinal,
                    start,
                    line.start,
                    properties,
                    text,
                    &index,
                    options,
                    &mut diagnostics,
                ));
            }
            let ordinal = cards.len() + 1;
            if ordinal > options.max_cards {
                diagnostics.push(partial(
                    VCARD_PARSER,
                    "vcard.limit.cards",
                    "vCard record limit was reached",
                    Some(property.locator.clone()),
                ));
                break;
            }
            if let Err(error) = control.budget().consume_records(1) {
                diagnostics.push(error.diagnostic(VCARD_PARSER).partial());
                break;
            }
            current = Some((ordinal, line.start, Vec::new()));
            continue;
        }
        if property.name == "END" && property.value.eq_ignore_ascii_case("VCARD") {
            let Some((ordinal, start, properties)) = current.take() else {
                diagnostics.push(partial(
                    VCARD_PARSER,
                    "vcard.structure.orphan_end",
                    "END:VCARD has no matching BEGIN:VCARD",
                    Some(property.locator.clone()),
                ));
                continue;
            };
            cards.push(build_card(
                ordinal,
                start,
                line.end,
                properties,
                text,
                &index,
                options,
                &mut diagnostics,
            ));
            continue;
        }
        if let Some((_, _, properties)) = current.as_mut() {
            properties.push(property);
        } else {
            diagnostics.push(partial(
                VCARD_PARSER,
                "vcard.structure.property_outside_card",
                "content line occurs outside a VCARD record",
                Some(property.locator.clone()),
            ));
        }
    }
    if let Some((ordinal, start, properties)) = current {
        diagnostics.push(partial(
            VCARD_PARSER,
            "vcard.structure.unclosed_card",
            "BEGIN:VCARD has no END:VCARD",
            Some(card_locator(text, &index, start, text.len(), ordinal)),
        ));
        cards.push(build_card(
            ordinal,
            start,
            text.len(),
            properties,
            text,
            &index,
            options,
            &mut diagnostics,
        ));
    }
    if cards.is_empty() {
        diagnostics.push(partial(
            VCARD_PARSER,
            "vcard.structure.missing_card",
            "no VCARD record was found",
            Some(document_locator(text, &index, "vcard.cards")),
        ));
    }
    account_for_attachments(
        cards.iter().flat_map(|card| card.attachments.iter()),
        control,
        VCARD_PARSER,
        &mut diagnostics,
    );
    let mut external_references = Vec::new();
    for card in &cards {
        collect_vcard_references(card, &mut external_references);
    }
    let complete = diagnostics.iter().all(|diagnostic| !diagnostic.partial);
    Ok(VCardDocument {
        schema_version: crate::core::SchemaVersion::VCARD_V1.into(),
        cards,
        external_references,
        diagnostics,
        complete,
        locator: document_locator(text, &index, "vcard.cards"),
    })
}

fn unfold_lines(
    text: &str,
    max_bytes: usize,
    diagnostics: &mut Vec<Diagnostic>,
    parser: &'static str,
) -> Vec<LogicalLine> {
    let physical = physical_lines(text);
    let mut lines = Vec::<LogicalLine>::new();
    for (start, content_end, _) in physical {
        let content = &text[start..content_end];
        let folded = content.starts_with([' ', '\t']);
        let quoted_printable_continuation = lines.last().is_some_and(|line| {
            line.unfolded
                .to_ascii_uppercase()
                .contains("ENCODING=QUOTED-PRINTABLE")
                && line.unfolded.ends_with('=')
        });
        if folded || quoted_printable_continuation {
            if let Some(line) = lines.last_mut() {
                if quoted_printable_continuation {
                    line.unfolded.pop();
                }
                line.unfolded
                    .push_str(if folded { &content[1..] } else { content });
                line.end = content_end;
                line.raw = text[line.start..line.end].to_string();
                if line.unfolded.len() > max_bytes && !line.too_long {
                    line.too_long = true;
                    diagnostics.push(partial(
                        parser,
                        "content_line.limit.unfolded",
                        format!("unfolded content line exceeds {max_bytes} bytes"),
                        None,
                    ));
                }
            } else {
                lines.push(LogicalLine {
                    ordinal: 1,
                    start,
                    end: content_end,
                    raw: content.to_string(),
                    unfolded: content.trim_start_matches([' ', '\t']).to_string(),
                    too_long: false,
                });
            }
            continue;
        }
        lines.push(LogicalLine {
            ordinal: lines.len() + 1,
            start,
            end: content_end,
            raw: content.to_string(),
            unfolded: content.to_string(),
            too_long: content.len() > max_bytes,
        });
        if content.len() > max_bytes {
            diagnostics.push(partial(
                parser,
                "content_line.limit.unfolded",
                format!("content line exceeds {max_bytes} bytes"),
                None,
            ));
        }
    }
    lines
}

fn physical_lines(text: &str) -> Vec<(usize, usize, usize)> {
    let bytes = text.as_bytes();
    let mut output = Vec::new();
    let mut start = 0usize;
    while start < bytes.len() {
        let mut next = start;
        while next < bytes.len() && !matches!(bytes[next], b'\r' | b'\n') {
            next += 1;
        }
        let content_end = next;
        if next < bytes.len() && bytes[next] == b'\r' {
            next += 1;
            if next < bytes.len() && bytes[next] == b'\n' {
                next += 1;
            }
        } else if next < bytes.len() {
            next += 1;
        }
        output.push((start, content_end, next));
        start = next;
    }
    if bytes.is_empty() {
        output.push((0, 0, 0));
    }
    output
}

fn parse_property(
    line: &LogicalLine,
    text: &str,
    index: &LineIndex,
    collection: &str,
    parser: &'static str,
    diagnostics: &mut Vec<Diagnostic>,
) -> RecordProperty {
    let locator = property_locator(text, index, line, collection, None);
    let Some(colon) = delimiter_outside_quotes(&line.unfolded, ':') else {
        diagnostics.push(partial(
            parser,
            "content_line.malformed.missing_colon",
            "content line has no value delimiter",
            Some(locator.clone()),
        ));
        return RecordProperty {
            ordinal: line.ordinal,
            group: None,
            name: "UNKNOWN".into(),
            raw_name: line.unfolded.clone(),
            raw: line.raw.clone(),
            raw_value: String::new(),
            value: String::new(),
            parameters: Vec::new(),
            valid: false,
            locator,
        };
    };
    let left = &line.unfolded[..colon];
    let raw_value = line.unfolded[colon + 1..].to_string();
    let tokens = split_quoted(left, ';');
    let name_token = tokens.first().copied().unwrap_or_default();
    let (group, raw_name) = name_token
        .rsplit_once('.')
        .map(|(group, name)| (Some(group.to_string()), name.to_string()))
        .unwrap_or((None, name_token.to_string()));
    let name = raw_name.to_ascii_uppercase();
    let mut parameters = Vec::new();
    for (ordinal, token) in tokens.iter().skip(1).enumerate() {
        let (raw_parameter_name, raw_parameter_value) = token
            .split_once('=')
            .map(|(name, value)| (name, value))
            .unwrap_or(("TYPE", *token));
        let values = split_quoted(raw_parameter_value, ',')
            .into_iter()
            .map(|value| decode_parameter_value(value.trim_matches('"')))
            .collect();
        parameters.push(RecordParameter {
            ordinal: ordinal + 1,
            name: raw_parameter_name.to_ascii_uppercase(),
            raw_name: raw_parameter_name.to_string(),
            raw_value: raw_parameter_value.to_string(),
            values,
            locator: locator.clone(),
        });
    }
    let decoded_raw = if parameter_has(&parameters, "ENCODING", "QUOTED-PRINTABLE") {
        match decode_quoted_printable(&raw_value) {
            Ok(bytes) => {
                decode_property_charset(&bytes, parameter_first(&parameters, "CHARSET").as_deref())
            }
            Err(message) => {
                diagnostics.push(partial(
                    parser,
                    "content_line.quoted_printable.invalid",
                    message,
                    Some(locator.clone()),
                ));
                raw_value.clone()
            }
        }
    } else {
        raw_value.clone()
    };
    let value = unescape_text(&decoded_raw);
    let valid = !name.is_empty() && !line.too_long;
    if name.is_empty() {
        diagnostics.push(partial(
            parser,
            "content_line.malformed.missing_name",
            "content line property name is empty",
            Some(locator.clone()),
        ));
    }
    RecordProperty {
        ordinal: line.ordinal,
        group,
        name,
        raw_name,
        raw: line.raw.clone(),
        raw_value,
        value,
        parameters,
        valid,
        locator,
    }
}

fn finish_component(
    builder: ComponentBuilder,
    end: usize,
    text: &str,
    index: &LineIndex,
) -> CalendarComponent {
    CalendarComponent {
        ordinal: builder.ordinal,
        kind: builder.kind,
        properties: builder.properties,
        children: builder.children,
        locator: component_locator(text, index, builder.start, end, builder.ordinal),
    }
}

fn collect_calendar_entities(
    component: &CalendarComponent,
    options: &ICalendarOptions,
    events: &mut Vec<CalendarEvent>,
    time_zones: &mut Vec<CalendarTimeZone>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match component.kind.as_str() {
        "VEVENT" | "VTODO" | "VJOURNAL" | "VFREEBUSY" => {
            events.push(build_event(component, options, diagnostics));
        }
        "VTIMEZONE" => time_zones.push(build_timezone(component, diagnostics)),
        _ => {}
    }
    for child in &component.children {
        if component.kind != "VTIMEZONE" {
            collect_calendar_entities(child, options, events, time_zones, diagnostics);
        }
    }
}

fn build_event(
    component: &CalendarComponent,
    options: &ICalendarOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> CalendarEvent {
    let properties = &component.properties;
    let uid = first_located(properties, "UID");
    if uid.is_none() && matches!(component.kind.as_str(), "VEVENT" | "VTODO" | "VJOURNAL") {
        diagnostics.push(partial(
            ICALENDAR_PARSER,
            "icalendar.event.missing_uid",
            format!("{} has no UID", component.kind),
            Some(component.locator.clone()),
        ));
    }
    let recurrence_rules = named(properties, "RRULE")
        .map(|property| parse_recurrence(property, diagnostics, ICALENDAR_PARSER))
        .collect();
    let recurrence_dates = temporal_list(properties, "RDATE", diagnostics, ICALENDAR_PARSER);
    let exception_dates = temporal_list(properties, "EXDATE", diagnostics, ICALENDAR_PARSER);
    let attendees = named(properties, "ATTENDEE").map(parse_attendee).collect();
    let organizer = named(properties, "ORGANIZER").next().map(parse_attendee);
    let attachments = named(properties, "ATTACH")
        .map(|property| {
            parse_attachment(
                property,
                options.max_decoded_attachment_bytes,
                diagnostics,
                ICALENDAR_PARSER,
            )
        })
        .collect();
    let relationships = named(properties, "RELATED-TO")
        .map(|property| CalendarRelationship {
            relation_type: property.parameter("RELTYPE").map(str::to_string),
            target_uid: property.value.clone(),
            locator: property.locator.clone(),
        })
        .collect();
    CalendarEvent {
        ordinal: component.ordinal,
        component_kind: component.kind.clone(),
        uid,
        recurrence_id: first_temporal(properties, "RECURRENCE-ID", diagnostics, ICALENDAR_PARSER),
        summary: first_located(properties, "SUMMARY"),
        description: first_located(properties, "DESCRIPTION"),
        location: first_located(properties, "LOCATION"),
        status: first_located(properties, "STATUS"),
        classification: first_located(properties, "CLASS"),
        transparency: first_located(properties, "TRANSP"),
        sequence: named(properties, "SEQUENCE")
            .next()
            .and_then(|property| property.value.parse().ok()),
        organizer,
        attendees,
        start: first_temporal(properties, "DTSTART", diagnostics, ICALENDAR_PARSER),
        end: first_temporal(properties, "DTEND", diagnostics, ICALENDAR_PARSER),
        due: first_temporal(properties, "DUE", diagnostics, ICALENDAR_PARSER),
        duration: first_temporal(properties, "DURATION", diagnostics, ICALENDAR_PARSER),
        created: first_temporal(properties, "CREATED", diagnostics, ICALENDAR_PARSER),
        last_modified: first_temporal(properties, "LAST-MODIFIED", diagnostics, ICALENDAR_PARSER),
        timestamp: first_temporal(properties, "DTSTAMP", diagnostics, ICALENDAR_PARSER),
        recurrence_rules,
        recurrence_dates,
        exception_dates,
        categories: named(properties, "CATEGORIES")
            .flat_map(|property| split_escaped(&property.raw_value, ','))
            .map(|value| unescape_text(&value))
            .collect(),
        url: first_located(properties, "URL"),
        attachments,
        relationships,
        alarms: component
            .children
            .iter()
            .filter(|child| child.kind == "VALARM")
            .cloned()
            .collect(),
        properties: properties.clone(),
        locator: component.locator.clone(),
    }
}

fn build_timezone(
    component: &CalendarComponent,
    diagnostics: &mut Vec<Diagnostic>,
) -> CalendarTimeZone {
    let observances = component
        .children
        .iter()
        .filter(|child| matches!(child.kind.as_str(), "STANDARD" | "DAYLIGHT"))
        .map(|child| {
            for name in ["TZOFFSETFROM", "TZOFFSETTO"] {
                if let Some(property) = named(&child.properties, name).next()
                    && !valid_utc_offset(&property.value)
                {
                    diagnostics.push(partial(
                        ICALENDAR_PARSER,
                        "icalendar.timezone.invalid_offset",
                        format!("{name} has invalid UTC offset {}", property.value),
                        Some(property.locator.clone()),
                    ));
                }
            }
            TimeZoneObservance {
                kind: child.kind.clone(),
                start: first_temporal(&child.properties, "DTSTART", diagnostics, ICALENDAR_PARSER),
                offset_from: named(&child.properties, "TZOFFSETFROM")
                    .next()
                    .map(|property| property.value.clone()),
                offset_to: named(&child.properties, "TZOFFSETTO")
                    .next()
                    .map(|property| property.value.clone()),
                names: named(&child.properties, "TZNAME")
                    .map(|property| property.value.clone())
                    .collect(),
                recurrence_rules: named(&child.properties, "RRULE")
                    .map(|property| parse_recurrence(property, diagnostics, ICALENDAR_PARSER))
                    .collect(),
                recurrence_dates: temporal_list(
                    &child.properties,
                    "RDATE",
                    diagnostics,
                    ICALENDAR_PARSER,
                ),
                properties: child.properties.clone(),
                locator: child.locator.clone(),
            }
        })
        .collect();
    CalendarTimeZone {
        ordinal: component.ordinal,
        timezone_id: first_located(&component.properties, "TZID"),
        last_modified: first_temporal(
            &component.properties,
            "LAST-MODIFIED",
            diagnostics,
            ICALENDAR_PARSER,
        ),
        url: first_located(&component.properties, "TZURL"),
        observances,
        properties: component.properties.clone(),
        locator: component.locator.clone(),
    }
}

fn parse_attendee(property: &RecordProperty) -> CalendarAttendee {
    CalendarAttendee {
        uri: property.value.clone(),
        common_name: property.parameter("CN").map(str::to_string),
        calendar_user_type: property.parameter("CUTYPE").map(str::to_string),
        role: property.parameter("ROLE").map(str::to_string),
        participation_status: property.parameter("PARTSTAT").map(str::to_string),
        rsvp: property.parameter("RSVP").and_then(|value| {
            match value.to_ascii_uppercase().as_str() {
                "TRUE" | "YES" => Some(true),
                "FALSE" | "NO" => Some(false),
                _ => None,
            }
        }),
        member: property
            .parameter_values("MEMBER")
            .into_iter()
            .map(str::to_string)
            .collect(),
        delegated_to: property
            .parameter_values("DELEGATED-TO")
            .into_iter()
            .map(str::to_string)
            .collect(),
        delegated_from: property
            .parameter_values("DELEGATED-FROM")
            .into_iter()
            .map(str::to_string)
            .collect(),
        sent_by: property.parameter("SENT-BY").map(str::to_string),
        directory: property.parameter("DIR").map(str::to_string),
        language: property.parameter("LANGUAGE").map(str::to_string),
        schedule_agent: property.parameter("SCHEDULE-AGENT").map(str::to_string),
        schedule_status: property
            .parameter_values("SCHEDULE-STATUS")
            .into_iter()
            .map(str::to_string)
            .collect(),
        schedule_force_send: property
            .parameter("SCHEDULE-FORCE-SEND")
            .map(str::to_string),
        parameters: property.parameters.clone(),
        locator: property.locator.clone(),
    }
}

fn parse_recurrence(
    property: &RecordProperty,
    diagnostics: &mut Vec<Diagnostic>,
    parser: &'static str,
) -> RecurrenceRule {
    let mut rule = RecurrenceRule {
        raw: property.raw_value.clone(),
        frequency: None,
        until: None,
        count: None,
        interval: None,
        by_second: Vec::new(),
        by_minute: Vec::new(),
        by_hour: Vec::new(),
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_year_day: Vec::new(),
        by_week_number: Vec::new(),
        by_month: Vec::new(),
        by_set_position: Vec::new(),
        week_start: None,
        recurrence_scale: None,
        skip: None,
        unknown_parts: Vec::new(),
        valid: true,
        locator: property.locator.clone(),
    };
    for part in split_escaped(&property.raw_value, ';') {
        let Some((name, value)) = part.split_once('=') else {
            rule.valid = false;
            rule.unknown_parts.push(RecurrencePart {
                name: String::new(),
                value: part,
            });
            continue;
        };
        let name = name.to_ascii_uppercase();
        match name.as_str() {
            "FREQ" => rule.frequency = Some(value.to_ascii_uppercase()),
            "UNTIL" => rule.until = Some(value.to_string()),
            "COUNT" => rule.count = value.parse().ok(),
            "INTERVAL" => rule.interval = value.parse().ok(),
            "BYSECOND" => rule.by_second = comma_values(value),
            "BYMINUTE" => rule.by_minute = comma_values(value),
            "BYHOUR" => rule.by_hour = comma_values(value),
            "BYDAY" => rule.by_day = comma_values(value),
            "BYMONTHDAY" => rule.by_month_day = comma_values(value),
            "BYYEARDAY" => rule.by_year_day = comma_values(value),
            "BYWEEKNO" => rule.by_week_number = comma_values(value),
            "BYMONTH" => rule.by_month = comma_values(value),
            "BYSETPOS" => rule.by_set_position = comma_values(value),
            "WKST" => rule.week_start = Some(value.to_ascii_uppercase()),
            "RSCALE" => rule.recurrence_scale = Some(value.to_ascii_uppercase()),
            "SKIP" => rule.skip = Some(value.to_ascii_uppercase()),
            _ => rule.unknown_parts.push(RecurrencePart {
                name,
                value: value.to_string(),
            }),
        }
    }
    if !rule.frequency.as_deref().is_some_and(|frequency| {
        matches!(
            frequency,
            "SECONDLY" | "MINUTELY" | "HOURLY" | "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY"
        )
    }) {
        rule.valid = false;
    }
    if property.raw_value.contains("COUNT=") && rule.count.is_none()
        || property.raw_value.contains("INTERVAL=") && rule.interval.is_none()
    {
        rule.valid = false;
    }
    if !rule.valid {
        diagnostics.push(partial(
            parser,
            "icalendar.recurrence.invalid_rule",
            format!("invalid recurrence rule {}", property.raw_value),
            Some(property.locator.clone()),
        ));
    }
    rule
}

fn temporal_list(
    properties: &[RecordProperty],
    name: &str,
    diagnostics: &mut Vec<Diagnostic>,
    parser: &'static str,
) -> Vec<TemporalValue> {
    named(properties, name)
        .flat_map(|property| {
            split_escaped(&property.raw_value, ',')
                .into_iter()
                .map(move |raw| temporal(property, raw))
        })
        .inspect(|value| {
            if !value.valid {
                diagnostics.push(partial(
                    parser,
                    "calendar.temporal.invalid",
                    format!("invalid temporal value {}", value.raw),
                    Some(value.locator.clone()),
                ));
            }
        })
        .collect()
}

fn first_temporal(
    properties: &[RecordProperty],
    name: &str,
    diagnostics: &mut Vec<Diagnostic>,
    parser: &'static str,
) -> Option<TemporalValue> {
    let property = named(properties, name).next()?;
    let value = temporal(property, property.raw_value.clone());
    if !value.valid {
        diagnostics.push(partial(
            parser,
            "calendar.temporal.invalid",
            format!("{name} has invalid value {}", property.raw_value),
            Some(property.locator.clone()),
        ));
    }
    Some(value)
}

fn temporal(property: &RecordProperty, raw: String) -> TemporalValue {
    let declared = property.parameter("VALUE").map(str::to_ascii_uppercase);
    let timezone_id = property.parameter("TZID").map(str::to_string);
    let (kind, valid, date, time, period_end) = if property.name == "DURATION"
        || declared.as_deref() == Some("DURATION")
    {
        (
            TemporalValueKind::Duration,
            valid_duration(&raw),
            None,
            None,
            None,
        )
    } else if declared.as_deref() == Some("PERIOD") || raw.contains('/') {
        let (start, end) = raw.split_once('/').unwrap_or((&raw, ""));
        (
            TemporalValueKind::Period,
            valid_date_or_datetime(start) && (valid_date_or_datetime(end) || valid_duration(end)),
            Some(start.to_string()),
            None,
            Some(end.to_string()),
        )
    } else if declared.as_deref() == Some("DATE")
        || valid_date(&raw)
        || matches!(property.name.as_str(), "BDAY" | "ANNIVERSARY") && valid_partial_date(&raw)
    {
        (
            TemporalValueKind::Date,
            valid_date(&raw) || valid_partial_date(&raw),
            Some(raw.clone()),
            None,
            None,
        )
    } else if declared.as_deref() == Some("TIME") {
        (
            TemporalValueKind::Time,
            valid_time(&raw),
            None,
            Some(raw.clone()),
            None,
        )
    } else {
        let normalized = raw.strip_suffix('Z').unwrap_or(&raw);
        let (date, time) = normalized
            .split_once('T')
            .map(|(date, time)| (Some(date.to_string()), Some(time.to_string())))
            .unwrap_or((None, None));
        (
            TemporalValueKind::DateTime,
            valid_datetime(&raw),
            date,
            time,
            None,
        )
    };
    TemporalValue {
        raw: raw.clone(),
        kind,
        timezone_id,
        utc: raw.ends_with('Z'),
        floating: !raw.ends_with('Z') && property.parameter("TZID").is_none(),
        date,
        time,
        period_end,
        valid,
        locator: property.locator.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_card(
    ordinal: usize,
    start: usize,
    end: usize,
    properties: Vec<RecordProperty>,
    text: &str,
    index: &LineIndex,
    options: &VCardOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> VCard {
    let version = first_located(&properties, "VERSION");
    if version.is_none() {
        diagnostics.push(partial(
            VCARD_PARSER,
            "vcard.version.missing",
            "VCARD record has no VERSION property",
            Some(card_locator(text, index, start, end, ordinal)),
        ));
    }
    let formatted_names = named(&properties, "FN").map(located).collect::<Vec<_>>();
    if formatted_names.is_empty() {
        diagnostics.push(partial(
            VCARD_PARSER,
            "vcard.name.missing_formatted",
            "VCARD record has no FN property",
            Some(card_locator(text, index, start, end, ordinal)),
        ));
    }
    let name = named(&properties, "N").next().map(|property| {
        let parts = structured_values(structured_source(property), 5);
        StructuredName {
            family: parts.first().cloned().unwrap_or_default(),
            given: parts.get(1).cloned().unwrap_or_default(),
            additional: parts.get(2).cloned().unwrap_or_default(),
            prefixes: parts.get(3).cloned().unwrap_or_default(),
            suffixes: parts.get(4).cloned().unwrap_or_default(),
            sort_as: property
                .parameter_values("SORT-AS")
                .into_iter()
                .map(str::to_string)
                .collect(),
            language: property.parameter("LANGUAGE").map(str::to_string),
            locator: property.locator.clone(),
        }
    });
    let addresses = named(&properties, "ADR").map(parse_address).collect();
    let communications = properties
        .iter()
        .filter(|property| {
            matches!(
                property.name.as_str(),
                "TEL" | "EMAIL" | "IMPP" | "URL" | "SOURCE"
            )
        })
        .map(|property| VCardCommunication {
            kind: property.name.clone(),
            value: property.value.clone(),
            types: property
                .parameter_values("TYPE")
                .into_iter()
                .map(str::to_string)
                .collect(),
            preference: property
                .parameter("PREF")
                .and_then(|value| value.parse().ok()),
            alternative_id: property.parameter("ALTID").map(str::to_string),
            media_type: property.parameter("MEDIATYPE").map(str::to_string),
            language: property.parameter("LANGUAGE").map(str::to_string),
            locator: property.locator.clone(),
        })
        .collect();
    let related = named(&properties, "RELATED")
        .map(|property| VCardRelated {
            value: property.value.clone(),
            value_kind: property
                .parameter("VALUE")
                .unwrap_or("uri")
                .to_ascii_lowercase(),
            relation_types: property
                .parameter_values("TYPE")
                .into_iter()
                .map(str::to_string)
                .collect(),
            resolved: false,
            locator: property.locator.clone(),
        })
        .collect();
    let attachments = properties
        .iter()
        .filter(|property| matches!(property.name.as_str(), "PHOTO" | "LOGO" | "SOUND" | "KEY"))
        .map(|property| {
            parse_attachment(
                property,
                options.max_decoded_attachment_bytes,
                diagnostics,
                VCARD_PARSER,
            )
        })
        .collect();
    VCard {
        ordinal,
        version,
        formatted_names,
        name,
        nicknames: named(&properties, "NICKNAME")
            .flat_map(|property| split_escaped(&property.raw_value, ','))
            .map(|value| unescape_text(&value))
            .collect(),
        organizations: named(&properties, "ORG")
            .map(|property| {
                split_escaped(structured_source(property), ';')
                    .into_iter()
                    .map(|value| unescape_text(&value))
                    .collect()
            })
            .collect(),
        titles: named(&properties, "TITLE").map(located).collect(),
        roles: named(&properties, "ROLE").map(located).collect(),
        birthday: first_temporal(&properties, "BDAY", diagnostics, VCARD_PARSER),
        anniversary: first_temporal(&properties, "ANNIVERSARY", diagnostics, VCARD_PARSER),
        gender: first_located(&properties, "GENDER"),
        addresses,
        communications,
        related,
        categories: named(&properties, "CATEGORIES")
            .flat_map(|property| split_escaped(&property.raw_value, ','))
            .map(|value| unescape_text(&value))
            .collect(),
        notes: named(&properties, "NOTE").map(located).collect(),
        uid: first_located(&properties, "UID"),
        kind: first_located(&properties, "KIND"),
        timezone: first_located(&properties, "TZ"),
        geo: first_located(&properties, "GEO"),
        attachments,
        properties,
        locator: card_locator(text, index, start, end, ordinal),
    }
}

fn parse_address(property: &RecordProperty) -> VCardAddress {
    let parts = structured_values(structured_source(property), 7);
    VCardAddress {
        post_office_box: parts.first().cloned().unwrap_or_default(),
        extended: parts.get(1).cloned().unwrap_or_default(),
        street: parts.get(2).cloned().unwrap_or_default(),
        locality: parts.get(3).cloned().unwrap_or_default(),
        region: parts.get(4).cloned().unwrap_or_default(),
        postal_code: parts.get(5).cloned().unwrap_or_default(),
        country: parts.get(6).cloned().unwrap_or_default(),
        types: property
            .parameter_values("TYPE")
            .into_iter()
            .map(str::to_string)
            .collect(),
        preference: property
            .parameter("PREF")
            .and_then(|value| value.parse().ok()),
        label: property.parameter("LABEL").map(str::to_string),
        language: property.parameter("LANGUAGE").map(str::to_string),
        geo: property.parameter("GEO").map(str::to_string),
        timezone: property.parameter("TZ").map(str::to_string),
        locator: property.locator.clone(),
    }
}

fn parse_attachment(
    property: &RecordProperty,
    max_decoded_bytes: usize,
    diagnostics: &mut Vec<Diagnostic>,
    parser: &'static str,
) -> InertAttachment {
    let encoding = property.parameter("ENCODING").map(str::to_string);
    let declared_binary = property
        .parameter("VALUE")
        .is_some_and(|value| value.eq_ignore_ascii_case("BINARY"))
        || encoding
            .as_deref()
            .is_some_and(|value| matches!(value.to_ascii_uppercase().as_str(), "B" | "BASE64"));
    let raw = property.raw_value.trim();
    let uri = (!declared_binary).then(|| unescape_text(raw));
    let value_kind = if declared_binary {
        AttachmentValueKind::Binary
    } else if raw.to_ascii_lowercase().starts_with("cid:") {
        AttachmentValueKind::ContentId
    } else if looks_like_uri(raw) {
        AttachmentValueKind::Uri
    } else {
        AttachmentValueKind::Unknown
    };
    let (decoded_sha256, decoded_bytes) = if declared_binary {
        if raw.len() > max_decoded_bytes.saturating_mul(2).saturating_add(16) {
            diagnostics.push(partial(
                parser,
                "calendar.attachment.limit",
                "encoded attachment exceeds the configured decoded-byte limit",
                Some(property.locator.clone()),
            ));
            (None, None)
        } else {
            match decode_base64(raw) {
                Ok(bytes) if bytes.len() <= max_decoded_bytes => {
                    let mut hasher = Sha256::new();
                    hasher.update(&bytes);
                    (Some(format!("{:x}", hasher.finalize())), Some(bytes.len()))
                }
                Ok(_) => {
                    diagnostics.push(partial(
                        parser,
                        "calendar.attachment.limit",
                        "decoded attachment exceeds the configured byte limit",
                        Some(property.locator.clone()),
                    ));
                    (None, None)
                }
                Err(message) => {
                    diagnostics.push(partial(
                        parser,
                        "calendar.attachment.invalid_base64",
                        message,
                        Some(property.locator.clone()),
                    ));
                    (None, None)
                }
            }
        }
    } else {
        (None, None)
    };
    InertAttachment {
        property_name: property.name.clone(),
        value_kind,
        raw_value: property.raw_value.clone(),
        uri,
        media_type: property
            .parameter("FMTTYPE")
            .or_else(|| property.parameter("MEDIATYPE"))
            .or_else(|| property.parameter("TYPE"))
            .map(str::to_string),
        encoding,
        decoded_sha256,
        decoded_bytes,
        inline_bytes_base64: None,
        resolved: false,
        safety_classification: if declared_binary {
            "inert_embedded_bytes".into()
        } else {
            "unresolved_reference".into()
        },
        locator: property.locator.clone(),
    }
}

fn collect_calendar_references(
    events: &[CalendarEvent],
    time_zones: &[CalendarTimeZone],
    output: &mut Vec<InertExternalReference>,
) {
    for event in events {
        let owner = event.uid.as_ref().map(|uid| uid.value.clone());
        if let Some(url) = &event.url {
            output.push(reference("URL", &url.value, owner.clone(), &url.locator));
        }
        for attachment in &event.attachments {
            if let Some(uri) = &attachment.uri {
                output.push(reference("ATTACH", uri, owner.clone(), &attachment.locator));
            }
        }
    }
    for timezone in time_zones {
        if let Some(url) = &timezone.url {
            output.push(reference(
                "TZURL",
                &url.value,
                timezone
                    .timezone_id
                    .as_ref()
                    .map(|value| value.value.clone()),
                &url.locator,
            ));
        }
    }
}

fn collect_vcard_references(card: &VCard, output: &mut Vec<InertExternalReference>) {
    let owner = card.uid.as_ref().map(|uid| uid.value.clone());
    for communication in &card.communications {
        if matches!(communication.kind.as_str(), "URL" | "SOURCE" | "IMPP")
            || looks_like_uri(&communication.value)
        {
            output.push(reference(
                &communication.kind,
                &communication.value,
                owner.clone(),
                &communication.locator,
            ));
        }
    }
    for related in &card.related {
        if looks_like_uri(&related.value) {
            output.push(reference(
                "RELATED",
                &related.value,
                owner.clone(),
                &related.locator,
            ));
        }
    }
    for attachment in &card.attachments {
        if let Some(uri) = &attachment.uri {
            output.push(reference(
                &attachment.property_name,
                uri,
                owner.clone(),
                &attachment.locator,
            ));
        }
    }
}

fn reference(
    source_property: &str,
    uri: &str,
    owner: Option<String>,
    locator: &SourceLocator,
) -> InertExternalReference {
    InertExternalReference {
        uri: uri.to_string(),
        source_property: source_property.to_string(),
        owner,
        resolved: false,
        locator: locator.clone(),
    }
}

fn validate_timezone_references(
    events: &[CalendarEvent],
    time_zones: &[CalendarTimeZone],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let known = time_zones
        .iter()
        .filter_map(|timezone| timezone.timezone_id.as_ref())
        .map(|timezone| timezone.value.as_str())
        .collect::<Vec<_>>();
    for event in events {
        for value in [
            event.start.as_ref(),
            event.end.as_ref(),
            event.due.as_ref(),
            event.recurrence_id.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if value.utc && value.timezone_id.is_some() {
                diagnostics.push(partial(
                    ICALENDAR_PARSER,
                    "icalendar.timezone.utc_with_tzid",
                    "UTC temporal value also declares TZID",
                    Some(value.locator.clone()),
                ));
            }
            if let Some(timezone_id) = &value.timezone_id
                && !known
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(timezone_id))
            {
                diagnostics.push(partial(
                    ICALENDAR_PARSER,
                    "icalendar.timezone.unresolved",
                    format!("TZID {timezone_id} has no matching VTIMEZONE"),
                    Some(value.locator.clone()),
                ));
            }
        }
    }
}

fn account_for_attachments<'a>(
    attachments: impl Iterator<Item = &'a InertAttachment>,
    control: &OperationControl,
    parser: &'static str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut count = 0u64;
    let mut decoded_bytes = 0u64;
    for attachment in attachments {
        count = count.saturating_add(1);
        decoded_bytes =
            decoded_bytes.saturating_add(attachment.decoded_bytes.unwrap_or_default() as u64);
    }
    if count > 0
        && let Err(error) = control.budget().consume_child_artifacts(count)
    {
        diagnostics.push(error.diagnostic(parser).partial());
    }
    if decoded_bytes > 0
        && let Err(error) = control.budget().observe_memory_bytes(decoded_bytes)
    {
        diagnostics.push(error.diagnostic(parser).partial());
    }
}

fn named<'a>(
    properties: &'a [RecordProperty],
    name: &'a str,
) -> impl Iterator<Item = &'a RecordProperty> {
    properties
        .iter()
        .filter(move |property| property.name.eq_ignore_ascii_case(name))
}

fn first_located(properties: &[RecordProperty], name: &str) -> Option<LocatedText> {
    named(properties, name).next().map(located)
}

fn located(property: &RecordProperty) -> LocatedText {
    LocatedText {
        raw: property.raw_value.clone(),
        value: property.value.clone(),
        locator: property.locator.clone(),
    }
}

fn parameter_has(parameters: &[RecordParameter], name: &str, value: &str) -> bool {
    parameters.iter().any(|parameter| {
        parameter.name.eq_ignore_ascii_case(name)
            && parameter
                .values
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(value))
    })
}

fn parameter_first(parameters: &[RecordParameter], name: &str) -> Option<String> {
    parameters
        .iter()
        .find(|parameter| parameter.name.eq_ignore_ascii_case(name))
        .and_then(|parameter| parameter.values.first())
        .cloned()
}

fn structured_values(raw: &str, width: usize) -> Vec<Vec<String>> {
    let mut output = split_escaped(raw, ';')
        .into_iter()
        .map(|part| {
            split_escaped(&part, ',')
                .into_iter()
                .map(|value| unescape_text(&value))
                .collect()
        })
        .collect::<Vec<_>>();
    output.resize_with(width, Vec::new);
    output
}

fn structured_source(property: &RecordProperty) -> &str {
    if property
        .parameter("ENCODING")
        .is_some_and(|value| value.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
    {
        &property.value
    } else {
        &property.raw_value
    }
}

fn comma_values(value: &str) -> Vec<String> {
    value.split(',').map(str::to_string).collect()
}

fn split_quoted(value: &str, delimiter: char) -> Vec<&str> {
    let mut output = Vec::new();
    let mut start = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == '"' {
            quoted = !quoted;
        } else if character == delimiter && !quoted {
            output.push(&value[start..offset]);
            start = offset + character.len_utf8();
        }
    }
    output.push(&value[start..]);
    output
}

fn split_escaped(value: &str, delimiter: char) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            current.push('\\');
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == delimiter {
            output.push(current);
            current = String::new();
        } else {
            current.push(character);
        }
    }
    if escaped {
        current.push('\\');
    }
    output.push(current);
    output
}

fn delimiter_outside_quotes(value: &str, delimiter: char) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == delimiter && !quoted {
            return Some(offset);
        }
    }
    None
}

fn decode_parameter_value(value: &str) -> String {
    let mut output = String::new();
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '^' {
            output.push(character);
            continue;
        }
        match characters.next() {
            Some('n' | 'N') => output.push('\n'),
            Some('^') => output.push('^'),
            Some('\'') => output.push('"'),
            Some(other) => {
                output.push('^');
                output.push(other);
            }
            None => output.push('^'),
        }
    }
    output
}

fn unescape_text(value: &str) -> String {
    let mut output = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            match character {
                'n' | 'N' => output.push('\n'),
                '\\' | ',' | ';' | ':' => output.push(character),
                other => {
                    output.push('\\');
                    output.push(other);
                }
            }
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            output.push(character);
        }
    }
    if escaped {
        output.push('\\');
    }
    output
}

fn decode_quoted_printable(value: &str) -> Result<Vec<u8>, String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor] != b'=' {
            output.push(bytes[cursor]);
            cursor += 1;
            continue;
        }
        if cursor + 2 >= bytes.len() {
            return Err("truncated quoted-printable escape".into());
        }
        let Some(high) = hex(bytes[cursor + 1]) else {
            return Err("invalid quoted-printable hexadecimal escape".into());
        };
        let Some(low) = hex(bytes[cursor + 2]) else {
            return Err("invalid quoted-printable hexadecimal escape".into());
        };
        output.push((high << 4) | low);
        cursor += 3;
    }
    Ok(output)
}

fn decode_property_charset(bytes: &[u8], charset: Option<&str>) -> String {
    let charset = charset.unwrap_or("utf-8").to_ascii_lowercase();
    match charset.as_str() {
        "us-ascii" | "ascii" | "utf-8" | "utf8" => String::from_utf8_lossy(bytes).into_owned(),
        "iso-8859-1" | "latin1" | "latin-1" => bytes.iter().map(|byte| char::from(*byte)).collect(),
        "windows-1252" | "cp1252" => bytes.iter().map(|byte| cp1252(*byte)).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn cp1252(byte: u8) -> char {
    match byte {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8a => 'Š',
        0x8b => '‹',
        0x8c => 'Œ',
        0x8e => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9a => 'š',
        0x9b => '›',
        0x9c => 'œ',
        0x9e => 'ž',
        0x9f => 'Ÿ',
        _ => char::from(byte),
    }
}

fn decode_base64(value: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    let mut quartet = [0u8; 4];
    let mut count = 0usize;
    let mut padding = 0usize;
    for byte in value.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let decoded = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                0
            }
            _ => return Err("attachment contains invalid base64 character".into()),
        };
        if padding > 0 && byte != b'=' {
            return Err("attachment has data after base64 padding".into());
        }
        quartet[count] = decoded;
        count += 1;
        if count == 4 {
            output.push((quartet[0] << 2) | (quartet[1] >> 4));
            if padding < 2 {
                output.push((quartet[1] << 4) | (quartet[2] >> 2));
            }
            if padding == 0 {
                output.push((quartet[2] << 6) | quartet[3]);
            }
            count = 0;
        }
    }
    if count != 0 || padding > 2 {
        return Err("attachment has invalid base64 length or padding".into());
    }
    Ok(output)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn valid_date(value: &str) -> bool {
    value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_partial_date(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
        && value.bytes().any(|byte| byte.is_ascii_digit())
}

fn valid_time(value: &str) -> bool {
    let value = value.strip_suffix('Z').unwrap_or(value);
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_datetime(value: &str) -> bool {
    let value = value.strip_suffix('Z').unwrap_or(value);
    value.len() == 15
        && value.as_bytes().get(8) == Some(&b'T')
        && valid_date(&value[..8])
        && valid_time(&value[9..])
}

fn valid_date_or_datetime(value: &str) -> bool {
    valid_date(value) || valid_datetime(value)
}

fn valid_duration(value: &str) -> bool {
    let value = value.strip_prefix(['+', '-']).unwrap_or(value);
    value.starts_with('P')
        && value.len() > 1
        && value.bytes().skip(1).all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'W' | b'D' | b'T' | b'H' | b'M' | b'S')
        })
}

fn valid_utc_offset(value: &str) -> bool {
    let Some(rest) = value.strip_prefix(['+', '-']) else {
        return false;
    };
    matches!(rest.len(), 4 | 6) && rest.bytes().all(|byte| byte.is_ascii_digit())
}

fn looks_like_uri(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && scheme.bytes().enumerate().all(|(index, byte)| {
            if index == 0 {
                byte.is_ascii_alphabetic()
            } else {
                byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
            }
        })
}

fn media_parameter(media_type: &str, name: &str) -> Option<String> {
    media_type.split(';').skip(1).find_map(|part| {
        let (candidate, value) = part.trim().split_once('=')?;
        candidate
            .trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().trim_matches('"').to_string())
    })
}

fn property_locator(
    _text: &str,
    index: &LineIndex,
    line: &LogicalLine,
    collection: &str,
    field: Option<String>,
) -> SourceLocator {
    let records = IndexRange::new(line.ordinal as u64, line.ordinal as u64 + 1, IndexBase::One)
        .expect("content-line record ordinal is valid");
    SourceLocator::exact(SourceRange::new(line.start, line.end, index))
        .expect("content-line range is valid")
        .nested(LocationComponent::RecordRange {
            collection: collection.into(),
            records,
            field,
        })
        .expect("content-line record locator is valid")
}

fn component_locator(
    _text: &str,
    index: &LineIndex,
    start: usize,
    end: usize,
    ordinal: usize,
) -> SourceLocator {
    let records = IndexRange::new(ordinal as u64, ordinal as u64 + 1, IndexBase::One)
        .expect("component ordinal is valid");
    SourceLocator::exact(SourceRange::new(start, end, index))
        .expect("component source range is valid")
        .nested(LocationComponent::RecordRange {
            collection: "icalendar.components".into(),
            records,
            field: None,
        })
        .expect("component locator is valid")
}

fn card_locator(
    _text: &str,
    index: &LineIndex,
    start: usize,
    end: usize,
    ordinal: usize,
) -> SourceLocator {
    let records = IndexRange::new(ordinal as u64, ordinal as u64 + 1, IndexBase::One)
        .expect("card ordinal is valid");
    SourceLocator::exact(SourceRange::new(start, end, index))
        .expect("card source range is valid")
        .nested(LocationComponent::RecordRange {
            collection: "vcard.cards".into(),
            records,
            field: None,
        })
        .expect("card locator is valid")
}

fn document_locator(text: &str, index: &LineIndex, collection: &str) -> SourceLocator {
    SourceLocator::exact(SourceRange::new(0, text.len(), index))
        .expect("document source range is valid")
        .nested(LocationComponent::RecordRange {
            collection: collection.into(),
            records: IndexRange::new(1, 2, IndexBase::One).expect("document record range is valid"),
            field: None,
        })
        .expect("document locator is valid")
}

fn partial(
    parser: &'static str,
    code: &'static str,
    message: impl Into<String>,
    locator: Option<SourceLocator>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::warning(parser, code, message.into())
        .with_module(parser)
        .with_parser(parser)
        .with_explanation_key(format!("diagnostic.{code}"))
        .partial();
    if let Some(locator) = locator {
        diagnostic = diagnostic.with_locator(locator);
    }
    diagnostic
}
