use crate::container::EmbeddedArtifact;
use crate::core::{Diagnostic, OperationStatus, SourceLocator};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmailOptions {
    /// Maximum RFC 5322 header block accepted as structured headers.
    pub max_header_bytes: usize,
    /// Maximum MIME entity depth, with the root entity at depth zero.
    pub max_mime_depth: usize,
    /// Maximum number of MIME entities emitted for one message.
    pub max_mime_parts: usize,
    /// Maximum decoded bytes retained or recursively parsed for one MIME entity.
    pub max_decoded_part_bytes: usize,
    /// Capture decoded attachment bytes inline. Otherwise exact identities and
    /// source locators are retained without duplicating source bytes.
    pub inline_attachment_bytes: bool,
    /// Dispatch locally supplied attachment bytes to enabled Grist parsers.
    pub parse_nested_attachments: bool,
}

impl Default for EmailOptions {
    fn default() -> Self {
        Self {
            max_header_bytes: 256 * 1024,
            max_mime_depth: 64,
            max_mime_parts: 10_000,
            max_decoded_part_bytes: 64 * 1024 * 1024,
            inline_attachment_bytes: false,
            parse_nested_attachments: true,
        }
    }
}

impl crate::core::FormatOptions for EmailOptions {
    const FORMAT: &'static str = "eml";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailDocument {
    pub schema_version: String,
    pub headers: Vec<EmailHeader>,
    pub address_fields: Vec<EmailAddressField>,
    pub date_fields: Vec<EmailDateField>,
    pub subject: Option<EmailTextFact>,
    pub thread: EmailThreadEvidence,
    pub authentication: Vec<EmailAuthenticationEvidence>,
    pub mime: MimePart,
    pub external_references: Vec<EmailExternalReference>,
    pub encrypted_parts: Vec<Vec<usize>>,
    pub signed_parts: Vec<Vec<usize>>,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailHeader {
    pub ordinal: usize,
    pub name: Option<String>,
    pub normalized_name: Option<String>,
    pub raw: String,
    pub raw_value: String,
    pub unfolded_value: String,
    pub decoded_value: String,
    pub valid: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailTextFact {
    pub raw: String,
    pub decoded: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailAddressField {
    pub header_name: String,
    pub raw: String,
    pub decoded: String,
    pub addresses: Vec<EmailAddress>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmailAddress {
    pub raw: String,
    pub display_name: Option<String>,
    pub address: Option<String>,
    pub group: Option<String>,
    pub comments: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailDateField {
    pub header_name: String,
    pub raw: String,
    pub decoded: String,
    pub timezone_text: Option<String>,
    pub timezone_offset_minutes: Option<i32>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EmailThreadEvidence {
    pub message_id: Option<EmailTextFact>,
    pub in_reply_to: Vec<EmailMessageId>,
    pub references: Vec<EmailMessageId>,
    pub received: Vec<EmailTextFact>,
    pub thread_index: Option<EmailTextFact>,
    pub thread_topic: Option<EmailTextFact>,
    pub normalized_subject_hint: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailMessageId {
    pub value: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailAuthenticationEvidence {
    pub kind: String,
    pub raw: String,
    pub decoded: String,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MimePart {
    /// Human-facing one-based MIME path. The root entity has an empty path.
    pub path: Vec<usize>,
    pub headers: Vec<EmailHeader>,
    pub content_type: MimeValue,
    pub content_disposition: Option<MimeValue>,
    pub transfer_encoding: String,
    pub content_id: Option<String>,
    pub content_location: Option<String>,
    pub encoded_body_sha256: String,
    pub encoded_body_bytes: usize,
    pub decoded_body_sha256: Option<String>,
    pub decoded_body_bytes: Option<usize>,
    pub text: Option<MimeTextBody>,
    pub preamble: Option<String>,
    pub epilogue: Option<String>,
    pub children: Vec<MimePart>,
    pub attachment: Option<EmailAttachment>,
    pub encrypted: bool,
    pub signed: bool,
    pub locator: SourceLocator,
    pub body_locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MimeValue {
    pub raw: String,
    pub essence: String,
    pub parameters: Vec<MimeParameter>,
}

impl MimeValue {
    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|parameter| parameter.name.eq_ignore_ascii_case(name))
            .map(|parameter| parameter.value.as_str())
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MimeParameter {
    pub name: String,
    pub raw_name: String,
    pub raw_value: String,
    pub value: String,
    pub extended: bool,
    pub charset: Option<String>,
    pub language: Option<String>,
    pub segments: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MimeTextBody {
    pub text: String,
    pub charset: String,
    pub lossy: bool,
    pub format_flowed: bool,
    pub delsp: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailAttachment {
    pub filename: Option<String>,
    pub disposition: String,
    pub inline_resource: bool,
    pub artifact: EmbeddedArtifact,
    pub nested: Option<NestedEmailParse>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NestedEmailParse {
    pub format: String,
    pub status: OperationStatus,
    pub envelope: serde_json::Value,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmailExternalReference {
    pub uri: String,
    pub source: String,
    pub mime_path: Vec<usize>,
    pub resolved: bool,
    pub locator: SourceLocator,
}
