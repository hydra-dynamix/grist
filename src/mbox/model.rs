use crate::core::{Diagnostic, OperationStatus, SourceLocator};
use crate::email::{EmailDocument, EmailOptions};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MboxOptions {
    /// Maximum messages emitted from one mailbox. A limit hit is partial, never EOF.
    pub max_messages: usize,
    /// Maximum bytes accepted for one envelope separator line.
    pub max_separator_bytes: usize,
    /// Use a valid Content-Length header to protect unescaped body `From ` lines.
    pub honor_content_length: bool,
    /// Recover bytes before the first valid separator as a malformed message.
    pub recover_missing_initial_separator: bool,
    /// Remove one quote marker from mboxo/mboxrd `>From ` body lines.
    pub unescape_from_lines: bool,
    /// RFC 5322 and MIME limits shared by every message in the mailbox.
    pub email: EmailOptions,
}

impl Default for MboxOptions {
    fn default() -> Self {
        Self {
            max_messages: 1_000_000,
            max_separator_bytes: 8 * 1024,
            honor_content_length: true,
            recover_missing_initial_separator: true,
            unescape_from_lines: true,
            email: EmailOptions::default(),
        }
    }
}

impl crate::core::FormatOptions for MboxOptions {
    const FORMAT: &'static str = "mbox";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MboxVariant {
    Mboxo,
    Mboxrd,
    Mboxcl,
    Mboxcl2,
    Mixed,
    #[default]
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MboxLineEnding {
    CrLf,
    Lf,
    Cr,
    None,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MboxDocument {
    pub schema_version: String,
    pub variant: MboxVariant,
    pub messages: Vec<MboxMessage>,
    pub thread_evidence: MboxAggregateThreadEvidence,
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MboxMessage {
    /// Human-facing, one-based source order.
    pub ordinal: usize,
    /// Stable for the same decoded message bytes; exact duplicates add an occurrence suffix.
    pub stable_id: String,
    pub duplicate_occurrence: usize,
    pub raw_sha256: String,
    pub decoded_sha256: String,
    pub raw_bytes: usize,
    pub decoded_bytes: usize,
    pub separator: Option<MboxSeparator>,
    pub escaped_from_lines: Vec<MboxEscapedFromLine>,
    pub content_length: Option<MboxContentLengthEvidence>,
    pub locator: SourceLocator,
    pub status: OperationStatus,
    pub email: Option<EmailDocument>,
    pub diagnostics: Vec<Diagnostic>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MboxSeparator {
    pub raw: String,
    pub sender: String,
    pub date: String,
    pub line_ending: MboxLineEnding,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MboxEscapedFromLine {
    pub raw_prefix_length: usize,
    pub decoded_offset: usize,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MboxContentLengthEvidence {
    pub raw: String,
    pub declared_body_bytes: Option<usize>,
    pub observed_body_bytes: usize,
    pub honored: bool,
    pub exact_boundary: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MboxAggregateThreadEvidence {
    pub message_ids: Vec<MboxMessageIdEvidence>,
    pub links: Vec<MboxThreadLink>,
    pub subject_groups: Vec<MboxSubjectGroup>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MboxMessageIdEvidence {
    pub stable_id: String,
    pub message_id: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MboxThreadLink {
    pub source_stable_id: String,
    pub target_stable_id: Option<String>,
    pub referenced_message_id: String,
    pub relation: MboxThreadRelation,
    pub resolved: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MboxThreadRelation {
    InReplyTo,
    Reference,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MboxSubjectGroup {
    pub normalized_subject: String,
    pub message_stable_ids: Vec<String>,
}
