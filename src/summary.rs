use crate::core::{Diagnostic, SchemaVersion};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub const RENDERED_SUMMARY_V1: &str = "grist/rendered-summary/v1";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderedSummary {
    pub schema_version: String,
    pub source_schema_version: String,
    pub title: String,
    pub profile: Option<String>,
    pub sections: Vec<SummarySection>,
    pub tables: Vec<SummaryTable>,
    pub diagnostics: Vec<Diagnostic>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SummarySection {
    pub heading: String,
    pub facts: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SummaryTable {
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SummaryProfile {
    DynamicEventDataset,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SummaryOptions {
    pub profile: Option<SummaryProfile>,
    pub source_diagnostics: Vec<Diagnostic>,
}

pub fn summarize_json_value(value: &Value, options: &SummaryOptions) -> RenderedSummary {
    let mut summary = summarize_generic_json(value, options.source_diagnostics.clone());
    if let Some(profile) = options.profile.as_ref() {
        match profile {
            SummaryProfile::DynamicEventDataset => apply_dynamic_event_profile(value, &mut summary),
        }
    }
    summary
}

#[cfg(feature = "serialization")]
pub fn summarize_serialization_payload(
    payload: &crate::serialization::SerializationPayload,
    source_diagnostics: Vec<Diagnostic>,
    profile: Option<SummaryProfile>,
) -> RenderedSummary {
    let mut diagnostics = source_diagnostics;
    match payload.value.as_ref() {
        Some(value) => summarize_json_value(
            value,
            &SummaryOptions {
                profile,
                source_diagnostics: diagnostics,
            },
        ),
        None => {
            diagnostics.push(Diagnostic::error(
                "grist.summary",
                "summary.no_value",
                "serialization payload did not contain a JSON value to summarize",
            ));
            RenderedSummary {
                schema_version: RENDERED_SUMMARY_V1.to_string(),
                source_schema_version: SchemaVersion::STRUCTURED_TEXT_V2.to_string(),
                title: "Serialization Summary".into(),
                profile: profile_name(profile.as_ref()).map(str::to_string),
                sections: vec![SummarySection {
                    heading: "Structure".into(),
                    facts: vec!["no parsed value".into()],
                }],
                tables: Vec::new(),
                diagnostics,
            }
        }
    }
}

fn summarize_generic_json(value: &Value, diagnostics: Vec<Diagnostic>) -> RenderedSummary {
    let stats = ValueStats::from(value);
    let mut sections = vec![SummarySection {
        heading: "Structure".into(),
        facts: structure_facts(value, &stats),
    }];

    if let Value::Object(map) = value {
        let keys: Vec<String> = map.keys().cloned().collect();
        sections.push(SummarySection {
            heading: "Top-level Keys".into(),
            facts: if keys.is_empty() {
                vec!["no top-level keys".into()]
            } else {
                vec![format!("{} keys: {}", keys.len(), keys.join(", "))]
            },
        });
    }

    let arrays = collect_array_summaries(value);
    if !arrays.is_empty() {
        sections.push(SummarySection {
            heading: "Arrays".into(),
            facts: arrays
                .iter()
                .map(|array| format!("{}: {} items", array.path, array.len))
                .collect(),
        });
    }

    let detected = detected_field_facts(value);
    if !detected.is_empty() {
        sections.push(SummarySection {
            heading: "Detected Fields".into(),
            facts: detected,
        });
    }

    let mut tables = Vec::new();
    for array in arrays.into_iter().filter(|array| !array.columns.is_empty()) {
        tables.push(SummaryTable {
            title: format!("Array shape at {}", array.path),
            columns: vec!["field".into(), "presence".into()],
            rows: array
                .columns
                .into_iter()
                .map(|(field, count)| vec![field, format!("{count}/{}", array.len)])
                .collect(),
        });
    }

    RenderedSummary {
        schema_version: RENDERED_SUMMARY_V1.to_string(),
        source_schema_version: SchemaVersion::STRUCTURED_TEXT_V2.to_string(),
        title: infer_title(value).unwrap_or_else(|| "Serialization Summary".into()),
        profile: None,
        sections,
        tables,
        diagnostics,
    }
}

fn structure_facts(value: &Value, stats: &ValueStats) -> Vec<String> {
    let mut facts = vec![format!("root type: {}", value_kind(value))];
    match value {
        Value::Object(map) => facts.push(format!("top-level object with {} keys", map.len())),
        Value::Array(items) => facts.push(format!("top-level array with {} items", items.len())),
        Value::String(text) => facts.push(format!("string length {}", text.chars().count())),
        Value::Number(_) => facts.push("numeric scalar".into()),
        Value::Bool(value) => facts.push(format!("boolean scalar: {value}")),
        Value::Null => facts.push("null scalar".into()),
    }
    facts.push(format!("max depth {}", stats.max_depth));
    facts.push(format!("{} objects", stats.objects));
    facts.push(format!("{} arrays", stats.arrays));
    facts.push(format!("{} scalar values", stats.scalars));
    facts
}

fn apply_dynamic_event_profile(value: &Value, summary: &mut RenderedSummary) {
    summary.profile = Some("dynamic-event-dataset".into());
    summary.title = infer_title(value).unwrap_or_else(|| "Dynamic Event Dataset".into());

    let mut facts = Vec::new();
    let object = match value.as_object() {
        Some(object) => object,
        None => {
            summary.diagnostics.push(Diagnostic::error(
                "grist.summary.dynamic_event_dataset",
                "profile.root_type",
                "dynamic-event-dataset profile expects a top-level object",
            ));
            return;
        }
    };

    let events = first_array(object, &["canonical_events", "canonicalEvents", "events"]);
    let signals = first_array(object, &["signals", "signal_events", "signalEvents"]);
    let participants_declared = first_array(object, &["participants", "actors", "entities"]);

    if let Some(events) = events {
        facts.push(format!("{} canonical events", events.len()));
    } else {
        facts.push("0 canonical events detected".into());
        summary.diagnostics.push(Diagnostic::warning(
            "grist.summary.dynamic_event_dataset",
            "profile.events_missing",
            "no canonical events array found at canonical_events, canonicalEvents, or events",
        ));
    }

    if let Some(signals) = signals {
        facts.push(format!("{} signals", signals.len()));
    }

    let participants = collect_participants(events, participants_declared);
    if !participants.is_empty() {
        facts.push(format!("{} participants", participants.len()));
    }

    if let Some((min, max)) = collect_time_extent(events) {
        facts.push(format!("time range {min}-{max}"));
    }

    if let Some(context) = object.get("context").and_then(Value::as_object) {
        let keys: Vec<_> = context.keys().cloned().collect();
        facts.push(format!("context keys: {}", keys.join(", ")));
    }
    if let Some(metadata) = object.get("metadata").and_then(Value::as_object) {
        let keys: Vec<_> = metadata.keys().cloned().collect();
        facts.push(format!("metadata keys: {}", keys.join(", ")));
    }

    if let Some(events) = events {
        for (idx, event) in events.iter().enumerate() {
            let Some(event) = event.as_object() else {
                summary.diagnostics.push(Diagnostic::warning(
                    "grist.summary.dynamic_event_dataset",
                    "profile.event_shape",
                    format!("event {} is not an object", idx + 1),
                ));
                continue;
            };
            if !has_any_key(event, &["id", "event_id", "eventId"]) {
                summary.diagnostics.push(Diagnostic::warning(
                    "grist.summary.dynamic_event_dataset",
                    "profile.event_missing_id",
                    format!("event {} has no id/event_id/eventId", idx + 1),
                ));
            }
            if !has_any_key(event, &["t", "time", "timestamp", "start", "tick"]) {
                summary.diagnostics.push(Diagnostic::warning(
                    "grist.summary.dynamic_event_dataset",
                    "profile.event_missing_time",
                    format!("event {} has no t/time/timestamp/start/tick", idx + 1),
                ));
            }
        }
    }

    if facts.is_empty() {
        facts.push("no dynamic event dataset facts detected".into());
    }
    summary.sections.insert(
        0,
        SummarySection {
            heading: "Canonical Events".into(),
            facts,
        },
    );

    if let Some(events) = events {
        let rows = events
            .iter()
            .take(5)
            .enumerate()
            .map(|(idx, event)| {
                vec![
                    (idx + 1).to_string(),
                    pick_field(event, &["id", "event_id", "eventId"]).unwrap_or_default(),
                    pick_field(event, &["t", "time", "timestamp", "start", "tick"])
                        .unwrap_or_default(),
                    pick_field(event, &["type", "kind", "label", "name"]).unwrap_or_default(),
                ]
            })
            .collect();
        summary.tables.insert(
            0,
            SummaryTable {
                title: "Representative Events".into(),
                columns: vec!["#".into(), "id".into(), "time".into(), "type".into()],
                rows,
            },
        );
    }
}

fn infer_title(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    for key in ["title", "name", "dataset_name", "datasetName"] {
        if let Some(text) = object.get(key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    object
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| {
            ["title", "name", "dataset_name", "datasetName"]
                .into_iter()
                .find_map(|key| metadata.get(key).and_then(Value::as_str))
        })
        .map(str::to_string)
}

fn first_array<'a>(
    object: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Vec<Value>> {
    keys.iter().find_map(|key| object.get(*key)?.as_array())
}

fn collect_participants(
    events: Option<&Vec<Value>>,
    declared: Option<&Vec<Value>>,
) -> BTreeSet<String> {
    let mut participants = BTreeSet::new();
    if let Some(declared) = declared {
        for value in declared {
            if let Some(text) = value.as_str() {
                participants.insert(text.to_string());
            } else if let Some(object) = value.as_object() {
                if let Some(text) = ["id", "name", "label"]
                    .into_iter()
                    .find_map(|key| object.get(key).and_then(Value::as_str))
                {
                    participants.insert(text.to_string());
                }
            }
        }
    }

    if let Some(events) = events {
        for event in events {
            if let Some(object) = event.as_object() {
                for key in [
                    "participant",
                    "participants",
                    "actor",
                    "actors",
                    "subject",
                    "object",
                    "source",
                    "target",
                    "from",
                    "to",
                ] {
                    collect_participant_value(object.get(key), &mut participants);
                }
            }
        }
    }
    participants
}

fn collect_participant_value(value: Option<&Value>, participants: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(text)) => {
            participants.insert(text.to_string());
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_participant_value(Some(item), participants);
            }
        }
        Some(Value::Object(object)) => {
            if let Some(text) = ["id", "name", "label"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(Value::as_str))
            {
                participants.insert(text.to_string());
            }
        }
        _ => {}
    }
}

fn collect_time_extent(events: Option<&Vec<Value>>) -> Option<(String, String)> {
    let mut numeric = Vec::new();
    let mut text = Vec::new();
    for event in events? {
        let object = event.as_object()?;
        let value = ["t", "time", "timestamp", "start", "tick"]
            .into_iter()
            .find_map(|key| object.get(key));
        match value {
            Some(Value::Number(number)) => {
                if let Some(v) = number.as_f64() {
                    numeric.push(v);
                }
            }
            Some(Value::String(value)) => text.push(value.clone()),
            _ => {}
        }
    }
    if !numeric.is_empty() {
        numeric.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some((format_number(numeric[0]), format_number(*numeric.last()?)))
    } else if !text.is_empty() {
        text.sort();
        Some((text.first()?.clone(), text.last()?.clone()))
    } else {
        None
    }
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn has_any_key(object: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|key| object.contains_key(*key))
}

fn pick_field(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key).map(render_cell))
}

fn render_cell(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(items) => format!("array[{}]", items.len()),
        Value::Object(map) => format!("object{{{}}}", map.len()),
    }
}

#[derive(Debug)]
struct ArraySummary {
    path: String,
    len: usize,
    columns: BTreeMap<String, usize>,
}

fn collect_array_summaries(value: &Value) -> Vec<ArraySummary> {
    let mut arrays = Vec::new();
    collect_array_summaries_at(value, "$", &mut arrays);
    arrays
}

fn collect_array_summaries_at(value: &Value, path: &str, arrays: &mut Vec<ArraySummary>) {
    match value {
        Value::Array(items) => {
            let mut columns = BTreeMap::new();
            for item in items {
                if let Some(object) = item.as_object() {
                    for key in object.keys() {
                        *columns.entry(key.clone()).or_insert(0) += 1;
                    }
                }
            }
            arrays.push(ArraySummary {
                path: path.into(),
                len: items.len(),
                columns,
            });
            for (idx, item) in items.iter().take(20).enumerate() {
                collect_array_summaries_at(item, &format!("{path}[{idx}]"), arrays);
            }
        }
        Value::Object(map) => {
            for (key, child) in map {
                collect_array_summaries_at(child, &format!("{path}.{key}"), arrays);
            }
        }
        _ => {}
    }
}

fn detected_field_facts(value: &Value) -> Vec<String> {
    let mut ids = BTreeSet::new();
    let mut times = BTreeSet::new();
    let mut names = BTreeSet::new();
    collect_detected_fields(value, "$", &mut ids, &mut times, &mut names);
    let mut facts = Vec::new();
    if !ids.is_empty() {
        facts.push(format!("id-like fields: {}", join_limited(ids)));
    }
    if !times.is_empty() {
        facts.push(format!("time-like fields: {}", join_limited(times)));
    }
    if !names.is_empty() {
        facts.push(format!("name-like fields: {}", join_limited(names)));
    }
    facts
}

fn collect_detected_fields(
    value: &Value,
    path: &str,
    ids: &mut BTreeSet<String>,
    times: &mut BTreeSet<String>,
    names: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let lower = key.to_ascii_lowercase();
                let field_path = format!("{path}.{key}");
                if lower == "id" || lower.ends_with("_id") || lower.ends_with("id") {
                    ids.insert(field_path.clone());
                }
                if matches!(
                    lower.as_str(),
                    "t" | "time" | "timestamp" | "start" | "end" | "tick"
                ) || lower.contains("time")
                {
                    times.insert(field_path.clone());
                }
                if matches!(lower.as_str(), "name" | "title" | "label") || lower.ends_with("name") {
                    names.insert(field_path.clone());
                }
                collect_detected_fields(child, &field_path, ids, times, names);
            }
        }
        Value::Array(items) => {
            for (idx, item) in items.iter().take(20).enumerate() {
                collect_detected_fields(item, &format!("{path}[{idx}]"), ids, times, names);
            }
        }
        _ => {}
    }
}

fn join_limited(values: BTreeSet<String>) -> String {
    let mut values: Vec<_> = values.into_iter().collect();
    let suffix = if values.len() > 12 {
        format!(" (+{} more)", values.len() - 12)
    } else {
        String::new()
    };
    values.truncate(12);
    format!("{}{}", values.join(", "), suffix)
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[derive(Default)]
struct ValueStats {
    objects: usize,
    arrays: usize,
    scalars: usize,
    max_depth: usize,
}

impl ValueStats {
    fn from(value: &Value) -> Self {
        let mut stats = Self::default();
        stats.visit(value, 1);
        stats
    }

    fn visit(&mut self, value: &Value, depth: usize) {
        self.max_depth = self.max_depth.max(depth);
        match value {
            Value::Object(map) => {
                self.objects += 1;
                for child in map.values() {
                    self.visit(child, depth + 1);
                }
            }
            Value::Array(items) => {
                self.arrays += 1;
                for child in items {
                    self.visit(child, depth + 1);
                }
            }
            _ => self.scalars += 1,
        }
    }
}

fn profile_name(profile: Option<&SummaryProfile>) -> Option<&'static str> {
    match profile {
        Some(SummaryProfile::DynamicEventDataset) => Some("dynamic-event-dataset"),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_generic_json_structure() {
        let value = serde_json::json!({
            "name": "Dataset",
            "items": [
                {"id": "a", "time": 1, "value": true},
                {"id": "b", "time": 2}
            ]
        });
        let summary = summarize_json_value(&value, &SummaryOptions::default());
        assert_eq!(summary.schema_version, RENDERED_SUMMARY_V1);
        assert_eq!(summary.title, "Dataset");
        assert!(
            summary
                .sections
                .iter()
                .any(|section| section.heading == "Arrays")
        );
        assert!(
            summary
                .tables
                .iter()
                .any(|table| table.title == "Array shape at $.items")
        );
    }

    #[test]
    fn dynamic_event_profile_extracts_domain_neutral_facts() {
        let value = serde_json::json!({
            "metadata": {"name": "Dynamic Event Dataset", "source": "test"},
            "context": {"run": "demo"},
            "canonical_events": [
                {"id": "e1", "t": 0, "type": "start", "participants": ["a", "b"]},
                {"id": "e2", "t": 45, "type": "stop", "actor": "a"}
            ],
            "signals": [{"id": "s1"}]
        });
        let summary = summarize_json_value(
            &value,
            &SummaryOptions {
                profile: Some(SummaryProfile::DynamicEventDataset),
                ..Default::default()
            },
        );
        let facts = &summary.sections[0].facts;
        assert!(facts.iter().any(|fact| fact == "2 canonical events"));
        assert!(facts.iter().any(|fact| fact == "1 signals"));
        assert!(facts.iter().any(|fact| fact == "2 participants"));
        assert!(facts.iter().any(|fact| fact == "time range 0-45"));
        assert!(
            summary
                .tables
                .iter()
                .any(|table| table.title == "Representative Events")
        );
    }
}
