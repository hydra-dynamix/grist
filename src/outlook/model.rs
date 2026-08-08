use crate::container::EmbeddedArtifact;
use crate::core::{Diagnostic, SourceLocator};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutlookMsgOptions {
    pub max_properties_per_object: usize,
    pub max_recipients: usize,
    pub max_attachments: usize,
    pub max_embedded_depth: usize,
    pub max_chain_sectors: usize,
    pub max_rtf_output_bytes: usize,
    pub inline_property_binary: bool,
    pub inline_unknown_streams: bool,
    pub inline_attachment_bytes: bool,
    pub parse_embedded_messages: bool,
}

impl Default for OutlookMsgOptions {
    fn default() -> Self {
        Self {
            max_properties_per_object: 65_536,
            max_recipients: 10_000,
            max_attachments: 10_000,
            max_embedded_depth: 64,
            max_chain_sectors: 1_000_000,
            max_rtf_output_bytes: 64 * 1024 * 1024,
            inline_property_binary: false,
            inline_unknown_streams: false,
            inline_attachment_bytes: false,
            parse_embedded_messages: true,
        }
    }
}

impl crate::core::FormatOptions for OutlookMsgOptions {
    const FORMAT: &'static str = "msg";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutlookMsgDocument {
    pub schema_version: String,
    pub compound_file: MsgCompoundFile,
    pub properties: Vec<MapiProperty>,
    pub named_properties: Vec<MapiNamedProperty>,
    pub recipients: Vec<MsgRecipient>,
    pub bodies: Vec<MsgBodyAlternative>,
    pub attachments: Vec<MsgAttachment>,
    pub subject: Option<MsgTextFact>,
    pub sender: MsgSender,
    pub thread: MsgThreadEvidence,
    pub dates: Vec<MsgDateFact>,
    pub message_class: Option<MsgTextFact>,
    pub unknown_objects: Vec<MsgUnknownObject>,
    pub encrypted: bool,
    pub signed: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MsgCompoundFile {
    pub major_version: u16,
    pub minor_version: u16,
    pub sector_size: usize,
    pub mini_sector_size: usize,
    pub mini_stream_cutoff: u32,
    pub directory_entries: usize,
    pub storage_count: usize,
    pub stream_count: usize,
    pub root_clsid: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MapiProperty {
    pub ordinal: usize,
    pub property_tag: String,
    pub property_id: u16,
    pub property_type: MapiPropertyType,
    pub flags: u32,
    pub table_value: MsgBinary,
    pub canonical_name: Option<String>,
    pub named: Option<MapiNamedProperty>,
    pub value: MapiValue,
    pub stream_paths: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MapiPropertyType {
    pub code: u16,
    pub name: String,
    pub multi_valued: bool,
    pub known: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapiValue {
    Unspecified {
        raw: MsgBinary,
    },
    Null,
    Integer16 {
        value: i16,
    },
    Integer32 {
        value: i32,
    },
    Float32 {
        bits: u32,
        value: f32,
    },
    Float64 {
        bits: u64,
        value: f64,
    },
    Currency {
        scaled_value: i64,
    },
    FloatingTime {
        bits: u64,
        value: f64,
    },
    Error {
        code: u32,
    },
    Boolean {
        raw: u16,
        value: bool,
    },
    Integer64 {
        value: i64,
    },
    String {
        text: String,
        encoding: String,
        lossy: bool,
        raw: MsgBinary,
    },
    SystemTime {
        value: MsgDate,
    },
    Guid {
        value: String,
        raw: MsgBinary,
    },
    Binary {
        value: MsgBinary,
    },
    Object {
        storage_path: Option<String>,
        raw: MsgBinary,
    },
    MultiValue {
        values: Vec<MapiValue>,
        raw: MsgBinary,
    },
    Unknown {
        type_code: u16,
        raw: MsgBinary,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MsgBinary {
    pub byte_length: usize,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MapiNamedProperty {
    pub property_id: u16,
    pub property_set: String,
    pub kind: MapiNamedPropertyKind,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapiNamedPropertyKind {
    Numeric { id: u32 },
    String { name: String },
    Unknown { raw_name_or_id: u32 },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgRecipient {
    pub ordinal: usize,
    pub storage_path: String,
    pub recipient_type: MsgRecipientType,
    pub display_name: Option<MsgTextFact>,
    pub email_address: Option<MsgTextFact>,
    pub smtp_address: Option<MsgTextFact>,
    pub address_type: Option<MsgTextFact>,
    pub properties: Vec<MapiProperty>,
    pub unknown_objects: Vec<MsgUnknownObject>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MsgRecipientType {
    To,
    Cc,
    Bcc,
    Originator,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgBodyAlternative {
    pub ordinal: usize,
    pub kind: MsgBodyKind,
    pub text: String,
    pub charset: String,
    pub lossy: bool,
    pub source_property_tag: String,
    pub source_binary: MsgBinary,
    pub rtf_compression: Option<MsgRtfCompression>,
    pub active_content_inert: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MsgBodyKind {
    PlainText,
    Html,
    Rtf,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MsgRtfCompression {
    pub magic: String,
    pub declared_compressed_size: u32,
    pub declared_uncompressed_size: u32,
    pub declared_crc32: u32,
    pub actual_crc32: u32,
    pub crc_matches: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgAttachment {
    pub ordinal: usize,
    pub storage_path: String,
    pub method: MsgAttachmentMethod,
    pub filename: Option<MsgTextFact>,
    pub mime_type: Option<MsgTextFact>,
    pub content_id: Option<MsgTextFact>,
    pub content_location: Option<MsgTextFact>,
    pub rendering_position: Option<i32>,
    pub declared_size: Option<i32>,
    pub properties: Vec<MapiProperty>,
    pub artifact: Option<EmbeddedArtifact>,
    pub embedded_message: Option<Box<OutlookMsgDocument>>,
    pub unknown_objects: Vec<MsgUnknownObject>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MsgAttachmentMethod {
    None,
    ByValue,
    ByReference,
    ByReferenceResolve,
    ByReferenceOnly,
    EmbeddedMessage,
    OleObject,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgTextFact {
    pub property_tag: String,
    pub raw: MsgBinary,
    pub text: String,
    pub encoding: String,
    pub lossy: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MsgSender {
    pub display_name: Option<MsgTextFact>,
    pub email_address: Option<MsgTextFact>,
    pub smtp_address: Option<MsgTextFact>,
    pub address_type: Option<MsgTextFact>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MsgThreadEvidence {
    pub internet_message_id: Option<MsgTextFact>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub conversation_topic: Option<MsgTextFact>,
    pub conversation_index: Option<MsgBinary>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MsgDate {
    pub filetime_ticks: u64,
    pub unix_seconds: Option<i64>,
    pub rfc3339_utc: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgDateFact {
    pub role: String,
    pub property_tag: String,
    pub value: MsgDate,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MsgUnknownObjectKind {
    Storage,
    Stream,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MsgUnknownObject {
    pub path: String,
    pub kind: MsgUnknownObjectKind,
    pub clsid: Option<String>,
    pub data: Option<MsgBinary>,
    pub locator: SourceLocator,
}
