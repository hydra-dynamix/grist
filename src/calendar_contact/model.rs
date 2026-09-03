use crate::core::{Diagnostic, SourceLocator};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ICalendarOptions {
    pub max_unfolded_line_bytes: usize,
    pub max_components: usize,
    pub max_properties: usize,
    pub max_nesting_depth: usize,
    pub max_decoded_attachment_bytes: usize,
}

impl Default for ICalendarOptions {
    fn default() -> Self {
        Self {
            max_unfolded_line_bytes: 1024 * 1024,
            max_components: 100_000,
            max_properties: 1_000_000,
            max_nesting_depth: 64,
            max_decoded_attachment_bytes: 64 * 1024 * 1024,
        }
    }
}

impl crate::core::FormatOptions for ICalendarOptions {
    const FORMAT: &'static str = "icalendar";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VCardOptions {
    pub max_unfolded_line_bytes: usize,
    pub max_cards: usize,
    pub max_properties: usize,
    pub max_decoded_attachment_bytes: usize,
}

impl Default for VCardOptions {
    fn default() -> Self {
        Self {
            max_unfolded_line_bytes: 1024 * 1024,
            max_cards: 100_000,
            max_properties: 1_000_000,
            max_decoded_attachment_bytes: 64 * 1024 * 1024,
        }
    }
}

impl crate::core::FormatOptions for VCardOptions {
    const FORMAT: &'static str = "vcard";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordParameter {
    pub ordinal: usize,
    pub name: String,
    pub raw_name: String,
    pub raw_value: String,
    pub values: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordProperty {
    pub ordinal: usize,
    pub group: Option<String>,
    pub name: String,
    pub raw_name: String,
    pub raw: String,
    pub raw_value: String,
    pub value: String,
    pub parameters: Vec<RecordParameter>,
    pub valid: bool,
    pub locator: SourceLocator,
}

impl RecordProperty {
    pub fn parameter_values(&self, name: &str) -> Vec<&str> {
        self.parameters
            .iter()
            .filter(|parameter| parameter.name.eq_ignore_ascii_case(name))
            .flat_map(|parameter| parameter.values.iter().map(String::as_str))
            .collect()
    }

    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameter_values(name).into_iter().next()
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocatedText {
    pub raw: String,
    pub value: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemporalValueKind {
    Date,
    DateTime,
    Time,
    Period,
    Duration,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemporalValue {
    pub raw: String,
    pub kind: TemporalValueKind,
    pub timezone_id: Option<String>,
    pub utc: bool,
    pub floating: bool,
    pub date: Option<String>,
    pub time: Option<String>,
    pub period_end: Option<String>,
    pub valid: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecurrenceRule {
    pub raw: String,
    pub frequency: Option<String>,
    pub until: Option<String>,
    pub count: Option<u64>,
    pub interval: Option<u64>,
    pub by_second: Vec<String>,
    pub by_minute: Vec<String>,
    pub by_hour: Vec<String>,
    pub by_day: Vec<String>,
    pub by_month_day: Vec<String>,
    pub by_year_day: Vec<String>,
    pub by_week_number: Vec<String>,
    pub by_month: Vec<String>,
    pub by_set_position: Vec<String>,
    pub week_start: Option<String>,
    pub recurrence_scale: Option<String>,
    pub skip: Option<String>,
    pub unknown_parts: Vec<RecurrencePart>,
    pub valid: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecurrencePart {
    pub name: String,
    pub value: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarAttendee {
    pub uri: String,
    pub common_name: Option<String>,
    pub calendar_user_type: Option<String>,
    pub role: Option<String>,
    pub participation_status: Option<String>,
    pub rsvp: Option<bool>,
    pub member: Vec<String>,
    pub delegated_to: Vec<String>,
    pub delegated_from: Vec<String>,
    pub sent_by: Option<String>,
    pub directory: Option<String>,
    pub language: Option<String>,
    pub schedule_agent: Option<String>,
    pub schedule_status: Vec<String>,
    pub schedule_force_send: Option<String>,
    pub parameters: Vec<RecordParameter>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentValueKind {
    Uri,
    Binary,
    ContentId,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InertAttachment {
    pub property_name: String,
    pub value_kind: AttachmentValueKind,
    pub raw_value: String,
    pub uri: Option<String>,
    pub media_type: Option<String>,
    pub encoding: Option<String>,
    pub decoded_sha256: Option<String>,
    pub decoded_bytes: Option<usize>,
    pub inline_bytes_base64: Option<String>,
    pub resolved: bool,
    pub safety_classification: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarRelationship {
    pub relation_type: Option<String>,
    pub target_uid: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarComponent {
    pub ordinal: usize,
    pub kind: String,
    pub properties: Vec<RecordProperty>,
    pub children: Vec<CalendarComponent>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarEvent {
    pub ordinal: usize,
    pub component_kind: String,
    pub uid: Option<LocatedText>,
    pub recurrence_id: Option<TemporalValue>,
    pub summary: Option<LocatedText>,
    pub description: Option<LocatedText>,
    pub location: Option<LocatedText>,
    pub status: Option<LocatedText>,
    pub classification: Option<LocatedText>,
    pub transparency: Option<LocatedText>,
    pub sequence: Option<i64>,
    pub organizer: Option<CalendarAttendee>,
    pub attendees: Vec<CalendarAttendee>,
    pub start: Option<TemporalValue>,
    pub end: Option<TemporalValue>,
    pub due: Option<TemporalValue>,
    pub duration: Option<TemporalValue>,
    pub created: Option<TemporalValue>,
    pub last_modified: Option<TemporalValue>,
    pub timestamp: Option<TemporalValue>,
    pub recurrence_rules: Vec<RecurrenceRule>,
    pub recurrence_dates: Vec<TemporalValue>,
    pub exception_dates: Vec<TemporalValue>,
    pub categories: Vec<String>,
    pub url: Option<LocatedText>,
    pub attachments: Vec<InertAttachment>,
    pub relationships: Vec<CalendarRelationship>,
    pub alarms: Vec<CalendarComponent>,
    pub properties: Vec<RecordProperty>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeZoneObservance {
    pub kind: String,
    pub start: Option<TemporalValue>,
    pub offset_from: Option<String>,
    pub offset_to: Option<String>,
    pub names: Vec<String>,
    pub recurrence_rules: Vec<RecurrenceRule>,
    pub recurrence_dates: Vec<TemporalValue>,
    pub properties: Vec<RecordProperty>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarTimeZone {
    pub ordinal: usize,
    pub timezone_id: Option<LocatedText>,
    pub last_modified: Option<TemporalValue>,
    pub url: Option<LocatedText>,
    pub observances: Vec<TimeZoneObservance>,
    pub properties: Vec<RecordProperty>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InertExternalReference {
    pub uri: String,
    pub source_property: String,
    pub owner: Option<String>,
    pub resolved: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ICalendarDocument {
    pub schema_version: String,
    pub version: Option<LocatedText>,
    pub product_id: Option<LocatedText>,
    pub calendar_scale: Option<LocatedText>,
    pub method: Option<LocatedText>,
    pub mime_media_type: Option<String>,
    pub mime_method: Option<String>,
    pub properties: Vec<RecordProperty>,
    pub components: Vec<CalendarComponent>,
    pub events: Vec<CalendarEvent>,
    pub time_zones: Vec<CalendarTimeZone>,
    pub external_references: Vec<InertExternalReference>,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredName {
    pub family: Vec<String>,
    pub given: Vec<String>,
    pub additional: Vec<String>,
    pub prefixes: Vec<String>,
    pub suffixes: Vec<String>,
    pub sort_as: Vec<String>,
    pub language: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VCardAddress {
    pub post_office_box: Vec<String>,
    pub extended: Vec<String>,
    pub street: Vec<String>,
    pub locality: Vec<String>,
    pub region: Vec<String>,
    pub postal_code: Vec<String>,
    pub country: Vec<String>,
    pub types: Vec<String>,
    pub preference: Option<u16>,
    pub label: Option<String>,
    pub language: Option<String>,
    pub geo: Option<String>,
    pub timezone: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VCardCommunication {
    pub kind: String,
    pub value: String,
    pub types: Vec<String>,
    pub preference: Option<u16>,
    pub alternative_id: Option<String>,
    pub media_type: Option<String>,
    pub language: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VCardRelated {
    pub value: String,
    pub value_kind: String,
    pub relation_types: Vec<String>,
    pub resolved: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VCard {
    pub ordinal: usize,
    pub version: Option<LocatedText>,
    pub formatted_names: Vec<LocatedText>,
    pub name: Option<StructuredName>,
    pub nicknames: Vec<String>,
    pub organizations: Vec<Vec<String>>,
    pub titles: Vec<LocatedText>,
    pub roles: Vec<LocatedText>,
    pub birthday: Option<TemporalValue>,
    pub anniversary: Option<TemporalValue>,
    pub gender: Option<LocatedText>,
    pub addresses: Vec<VCardAddress>,
    pub communications: Vec<VCardCommunication>,
    pub related: Vec<VCardRelated>,
    pub categories: Vec<String>,
    pub notes: Vec<LocatedText>,
    pub uid: Option<LocatedText>,
    pub kind: Option<LocatedText>,
    pub timezone: Option<LocatedText>,
    pub geo: Option<LocatedText>,
    pub attachments: Vec<InertAttachment>,
    pub properties: Vec<RecordProperty>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VCardDocument {
    pub schema_version: String,
    pub cards: Vec<VCard>,
    pub external_references: Vec<InertExternalReference>,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
    pub locator: SourceLocator,
}
