use super::json::locator;
use super::model::{
    DuplicateKey, DuplicateKeyDisposition, StructuredAlias, StructuredEntry, StructuredScalar,
    StructuredValue, StructuredValueKind, pointer_escape,
};
use crate::core::{LineIndex, SourceRange};
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::mem::MaybeUninit;

#[derive(Debug, Clone)]
pub(crate) struct YamlParseFailure {
    pub offset: usize,
    pub message: String,
}

pub(crate) struct ParsedYaml {
    pub documents: Vec<StructuredValue>,
    pub duplicate_keys: Vec<DuplicateKey>,
    pub aliases: Vec<StructuredAlias>,
    pub max_depth: usize,
    pub node_count: usize,
}

#[derive(Debug, Clone)]
enum EventKind {
    StreamStart,
    StreamEnd,
    DocumentStart,
    DocumentEnd,
    Alias {
        name: String,
    },
    Scalar {
        value: String,
        anchor: Option<String>,
        tag: Option<String>,
        plain: bool,
    },
    SequenceStart {
        anchor: Option<String>,
        tag: Option<String>,
    },
    SequenceEnd,
    MappingStart {
        anchor: Option<String>,
        tag: Option<String>,
    },
    MappingEnd,
}

#[derive(Debug, Clone)]
struct Event {
    kind: EventKind,
    start: usize,
    end: usize,
}

pub(crate) fn parse(text: &str) -> Result<ParsedYaml, YamlParseFailure> {
    let events = scan(text)?;
    let mut builder = Builder {
        text,
        lines: LineIndex::new(text),
        events: &events,
        cursor: 0,
        duplicates: Vec::new(),
        anchors: BTreeMap::new(),
        max_depth: 0,
        node_count: 0,
    };
    builder.expect(
        |kind| matches!(kind, EventKind::StreamStart),
        "YAML stream start",
    )?;
    let mut documents = Vec::new();
    while !builder.peek_is(|kind| matches!(kind, EventKind::StreamEnd)) {
        builder.expect(
            |kind| matches!(kind, EventKind::DocumentStart),
            "YAML document start",
        )?;
        if builder.peek_is(|kind| matches!(kind, EventKind::DocumentEnd)) {
            let event = builder.events[builder.cursor].clone();
            let range = SourceRange::new(event.start, event.end, &builder.lines);
            documents.push(StructuredValue {
                id: format!("yaml:/@{}", event.start),
                kind: StructuredValueKind::Null,
                path: String::new(),
                range: range.clone(),
                locator: locator(range, "", None),
                raw: String::new(),
                scalar: Some(StructuredScalar::Null),
                entries: vec![],
                items: vec![],
                anchor: None,
                tag: None,
                alias: None,
                alias_target_id: None,
                recovered: false,
            });
        } else {
            documents.push(builder.node("", 1)?);
        }
        builder.expect(
            |kind| matches!(kind, EventKind::DocumentEnd),
            "YAML document end",
        )?;
    }
    builder.expect(
        |kind| matches!(kind, EventKind::StreamEnd),
        "YAML stream end",
    )?;

    let anchors = builder.anchors.clone();
    let mut aliases = Vec::new();
    for document in &mut documents {
        resolve_aliases(document, &anchors, &mut aliases);
    }
    Ok(ParsedYaml {
        documents,
        duplicate_keys: builder.duplicates,
        aliases,
        max_depth: builder.max_depth,
        node_count: builder.node_count,
    })
}

struct Builder<'a> {
    text: &'a str,
    lines: LineIndex,
    events: &'a [Event],
    cursor: usize,
    duplicates: Vec<DuplicateKey>,
    anchors: BTreeMap<String, String>,
    max_depth: usize,
    node_count: usize,
}

impl Builder<'_> {
    fn node(&mut self, path: &str, depth: usize) -> Result<StructuredValue, YamlParseFailure> {
        self.max_depth = self.max_depth.max(depth);
        self.node_count += 1;
        let event = self
            .events
            .get(self.cursor)
            .cloned()
            .ok_or_else(|| YamlParseFailure {
                offset: self.text.len(),
                message: "unexpected end of YAML event stream".into(),
            })?;
        self.cursor += 1;
        match event.kind {
            EventKind::Scalar {
                value,
                anchor,
                tag,
                plain,
            } => {
                let (kind, scalar) = yaml_scalar(&value, tag.as_deref(), plain);
                Ok(self.finish_node(
                    event.start,
                    event.end,
                    path,
                    kind,
                    Some(scalar),
                    vec![],
                    vec![],
                    anchor,
                    tag,
                    None,
                ))
            }
            EventKind::Alias { name } => Ok(self.finish_node(
                event.start,
                event.end,
                path,
                StructuredValueKind::Alias,
                None,
                vec![],
                vec![],
                None,
                None,
                Some(name),
            )),
            EventKind::SequenceStart { anchor, tag } => {
                let mut items = Vec::new();
                while !self.peek_is(|kind| matches!(kind, EventKind::SequenceEnd)) {
                    let child = format!("{path}/{}", items.len());
                    items.push(self.node(&child, depth + 1)?);
                }
                let end = self.events[self.cursor].end;
                self.cursor += 1;
                Ok(self.finish_node(
                    event.start,
                    end,
                    path,
                    StructuredValueKind::Array,
                    None,
                    vec![],
                    items,
                    anchor,
                    tag,
                    None,
                ))
            }
            EventKind::MappingStart { anchor, tag } => {
                let mut entries = Vec::new();
                let mut seen = BTreeMap::<String, (usize, crate::core::SourceLocator)>::new();
                while !self.peek_is(|kind| matches!(kind, EventKind::MappingEnd)) {
                    let key_path = format!("{path}/$key/{}", entries.len());
                    let key = self.node(&key_path, depth + 1)?;
                    let key_text = scalar_text(&key);
                    let identity = key_text.clone().unwrap_or_else(|| {
                        serde_json::to_string(&key).unwrap_or_else(|_| key.raw.clone())
                    });
                    let value_path = key_text.as_ref().map_or_else(
                        || format!("{path}/{}", entries.len()),
                        |key| format!("{path}/{}", pointer_escape(key)),
                    );
                    let value = self.node(&value_path, depth + 1)?;
                    let occurrence = seen.get(&identity).map_or(1, |(count, _)| count + 1);
                    if let Some((_, first)) = seen.get(&identity) {
                        self.duplicates.push(DuplicateKey {
                            path: path.to_string(),
                            key: identity.clone(),
                            occurrence,
                            first_locator: first.clone(),
                            duplicate_locator: key.locator.clone(),
                            disposition: DuplicateKeyDisposition::Preserved,
                        });
                    }
                    seen.insert(identity, (occurrence, key.locator.clone()));
                    entries.push(StructuredEntry {
                        index: entries.len(),
                        key: Box::new(key),
                        value: Box::new(value),
                        key_text,
                        duplicate_ordinal: occurrence,
                    });
                }
                let end = self.events[self.cursor].end;
                self.cursor += 1;
                Ok(self.finish_node(
                    event.start,
                    end,
                    path,
                    StructuredValueKind::Object,
                    None,
                    entries,
                    vec![],
                    anchor,
                    tag,
                    None,
                ))
            }
            _ => Err(YamlParseFailure {
                offset: event.start,
                message: "unexpected YAML structural event".into(),
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_node(
        &mut self,
        start: usize,
        end: usize,
        path: &str,
        kind: StructuredValueKind,
        scalar: Option<StructuredScalar>,
        entries: Vec<StructuredEntry>,
        items: Vec<StructuredValue>,
        anchor: Option<String>,
        tag: Option<String>,
        alias: Option<String>,
    ) -> StructuredValue {
        let end = end.min(self.text.len()).max(start);
        let range = SourceRange::new(start, end, &self.lines);
        let id = format!("yaml:{}@{start}", if path.is_empty() { "/" } else { path });
        if let Some(anchor) = &anchor {
            self.anchors.insert(anchor.clone(), id.clone());
        }
        StructuredValue {
            id,
            kind,
            path: path.to_string(),
            range: range.clone(),
            locator: locator(range, path, None),
            raw: self.text[start..end].to_string(),
            scalar,
            entries,
            items,
            anchor,
            tag,
            alias,
            alias_target_id: None,
            recovered: false,
        }
    }

    fn peek_is(&self, test: impl FnOnce(&EventKind) -> bool) -> bool {
        self.events
            .get(self.cursor)
            .is_some_and(|event| test(&event.kind))
    }

    fn expect(
        &mut self,
        test: impl FnOnce(&EventKind) -> bool,
        expected: &str,
    ) -> Result<(), YamlParseFailure> {
        let event = self
            .events
            .get(self.cursor)
            .ok_or_else(|| YamlParseFailure {
                offset: self.text.len(),
                message: format!("expected {expected}"),
            })?;
        if !test(&event.kind) {
            return Err(YamlParseFailure {
                offset: event.start,
                message: format!("expected {expected}"),
            });
        }
        self.cursor += 1;
        Ok(())
    }
}

fn scalar_text(value: &StructuredValue) -> Option<String> {
    match value.scalar.as_ref()? {
        StructuredScalar::Null => Some("null".into()),
        StructuredScalar::Boolean { value } => Some(value.to_string()),
        StructuredScalar::Integer { canonical } | StructuredScalar::Float { canonical, .. } => {
            Some(canonical.clone())
        }
        StructuredScalar::String { value }
        | StructuredScalar::Date { value }
        | StructuredScalar::Time { value }
        | StructuredScalar::DateTime { value } => Some(value.clone()),
    }
}

fn yaml_scalar(
    value: &str,
    tag: Option<&str>,
    plain: bool,
) -> (StructuredValueKind, StructuredScalar) {
    if tag.is_some_and(|tag| tag.ends_with(":str")) || !plain {
        return (
            StructuredValueKind::String,
            StructuredScalar::String {
                value: value.into(),
            },
        );
    }
    let lower = value.to_ascii_lowercase();
    if tag.is_some_and(|tag| tag.ends_with(":null")) || matches!(lower.as_str(), "" | "null" | "~")
    {
        return (StructuredValueKind::Null, StructuredScalar::Null);
    }
    if tag.is_some_and(|tag| tag.ends_with(":bool")) || matches!(lower.as_str(), "true" | "false") {
        return (
            StructuredValueKind::Boolean,
            StructuredScalar::Boolean {
                value: lower == "true",
            },
        );
    }
    let normalized = value.replace('_', "");
    if tag.is_some_and(|tag| tag.ends_with(":int")) || yaml_integer(&normalized) {
        return (
            StructuredValueKind::Integer,
            StructuredScalar::Integer {
                canonical: normalized,
            },
        );
    }
    if tag.is_some_and(|tag| tag.ends_with(":float")) || yaml_float(&lower) {
        let finite = !matches!(lower.as_str(), ".inf" | "+.inf" | "-.inf" | ".nan");
        return (
            StructuredValueKind::Float,
            StructuredScalar::Float {
                canonical: normalized,
                finite,
            },
        );
    }
    if looks_datetime(value) {
        return (
            StructuredValueKind::DateTime,
            StructuredScalar::DateTime {
                value: value.into(),
            },
        );
    }
    (
        StructuredValueKind::String,
        StructuredScalar::String {
            value: value.into(),
        },
    )
}

fn yaml_integer(value: &str) -> bool {
    let value = value.strip_prefix(['+', '-']).unwrap_or(value);
    if let Some(digits) = value.strip_prefix("0x") {
        return !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_hexdigit());
    }
    if let Some(digits) = value.strip_prefix("0o") {
        return !digits.is_empty() && digits.chars().all(|ch| matches!(ch, '0'..='7'));
    }
    if let Some(digits) = value.strip_prefix("0b") {
        return !digits.is_empty() && digits.chars().all(|ch| matches!(ch, '0' | '1'));
    }
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())
}

fn yaml_float(value: &str) -> bool {
    matches!(value, ".inf" | "+.inf" | "-.inf" | ".nan")
        || (value.contains(['.', 'e', 'E']) && value.parse::<f64>().is_ok())
}

fn looks_datetime(value: &str) -> bool {
    value.len() >= 10
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value[..4].chars().all(|ch| ch.is_ascii_digit())
}

fn resolve_aliases(
    value: &mut StructuredValue,
    anchors: &BTreeMap<String, String>,
    aliases: &mut Vec<StructuredAlias>,
) {
    if let Some(name) = &value.alias {
        value.alias_target_id = anchors.get(name).cloned();
        aliases.push(StructuredAlias {
            name: name.clone(),
            path: value.path.clone(),
            locator: value.locator.clone(),
            target_id: value.alias_target_id.clone(),
            resolved: value.alias_target_id.is_some(),
            expanded: false,
        });
    }
    for entry in &mut value.entries {
        resolve_aliases(&mut entry.key, anchors, aliases);
        resolve_aliases(&mut entry.value, anchors, aliases);
    }
    for item in &mut value.items {
        resolve_aliases(item, anchors, aliases);
    }
}

fn scan(text: &str) -> Result<Vec<Event>, YamlParseFailure> {
    unsafe {
        let mut parser = MaybeUninit::<unsafe_libyaml::yaml_parser_t>::uninit();
        if unsafe_libyaml::yaml_parser_initialize(parser.as_mut_ptr()).fail {
            return Err(YamlParseFailure {
                offset: 0,
                message: "YAML parser allocation failed".into(),
            });
        }
        let mut parser = ParserGuard(parser.assume_init());
        unsafe_libyaml::yaml_parser_set_encoding(&mut parser.0, unsafe_libyaml::YAML_UTF8_ENCODING);
        unsafe_libyaml::yaml_parser_set_input_string(
            &mut parser.0,
            text.as_ptr(),
            text.len() as u64,
        );
        let mut events = Vec::new();
        loop {
            let mut raw = MaybeUninit::<unsafe_libyaml::yaml_event_t>::uninit();
            if unsafe_libyaml::yaml_parser_parse(&mut parser.0, raw.as_mut_ptr()).fail {
                let (offset, message) = serde_yaml_error(text);
                return Err(YamlParseFailure { offset, message });
            }
            let mut raw = raw.assume_init();
            let event = copy_event(&raw);
            let done = matches!(event.kind, EventKind::StreamEnd);
            unsafe_libyaml::yaml_event_delete(&mut raw);
            events.push(event);
            if done {
                break;
            }
        }
        Ok(events)
    }
}

struct ParserGuard(unsafe_libyaml::yaml_parser_t);

impl Drop for ParserGuard {
    fn drop(&mut self) {
        unsafe { unsafe_libyaml::yaml_parser_delete(&mut self.0) }
    }
}

unsafe fn copy_event(raw: &unsafe_libyaml::yaml_event_t) -> Event {
    use unsafe_libyaml::*;
    let kind = match raw.type_ {
        YAML_STREAM_START_EVENT => EventKind::StreamStart,
        YAML_STREAM_END_EVENT => EventKind::StreamEnd,
        YAML_DOCUMENT_START_EVENT => EventKind::DocumentStart,
        YAML_DOCUMENT_END_EVENT => EventKind::DocumentEnd,
        YAML_ALIAS_EVENT => EventKind::Alias {
            name: c_string(unsafe { raw.data.alias.anchor }).unwrap_or_default(),
        },
        YAML_SCALAR_EVENT => {
            let data = unsafe { raw.data.scalar };
            let bytes = unsafe { std::slice::from_raw_parts(data.value, data.length as usize) };
            EventKind::Scalar {
                value: String::from_utf8_lossy(bytes).into_owned(),
                anchor: c_string(data.anchor),
                tag: c_string(data.tag),
                plain: data.style == YAML_PLAIN_SCALAR_STYLE,
            }
        }
        YAML_SEQUENCE_START_EVENT => {
            let data = unsafe { raw.data.sequence_start };
            EventKind::SequenceStart {
                anchor: c_string(data.anchor),
                tag: c_string(data.tag),
            }
        }
        YAML_SEQUENCE_END_EVENT => EventKind::SequenceEnd,
        YAML_MAPPING_START_EVENT => {
            let data = unsafe { raw.data.mapping_start };
            EventKind::MappingStart {
                anchor: c_string(data.anchor),
                tag: c_string(data.tag),
            }
        }
        YAML_MAPPING_END_EVENT => EventKind::MappingEnd,
        YAML_NO_EVENT | _ => unreachable!("libyaml returned an unsupported event"),
    };
    Event {
        kind,
        start: raw.start_mark.index as usize,
        end: raw.end_mark.index as usize,
    }
}

fn c_string(pointer: *const u8) -> Option<String> {
    if pointer.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(pointer.cast()) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

fn serde_yaml_error(text: &str) -> (usize, String) {
    match serde_yaml::from_str::<serde_yaml::Value>(text) {
        Err(error) => {
            let offset = error.location().map_or(0, |location| {
                let line = location.line().saturating_sub(1);
                let column = location.column().saturating_sub(1);
                text.split_inclusive('\n')
                    .take(line)
                    .map(str::len)
                    .sum::<usize>()
                    + column
            });
            (offset.min(text.len()), error.to_string())
        }
        Ok(_) => (0, "YAML event parsing failed".into()),
    }
}
