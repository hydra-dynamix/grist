//! Content identities and the single versioned canonical JSON implementation.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

use super::{Hashes, SchemaVersion, sha256_hex};

/// Canonical JSON compatibility boundary.
///
/// New algorithms require new enum variants. Changing `V1` would change
/// payload and aggregate identities and is an incompatible change.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(
    Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum CanonicalJsonVersion {
    #[default]
    #[serde(rename = "grist/canonical-json/v1")]
    V1,
}

impl CanonicalJsonVersion {
    pub const CURRENT: Self = Self::V1;

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "grist/canonical-json/v1",
        }
    }
}

/// Encode one value using Grist canonical JSON v1.
///
/// The hashed byte sequence is UTF-8 JSON with no whitespace; object keys are
/// sorted lexicographically by their UTF-8 bytes; arrays retain input order;
/// strings, booleans, null, and numbers use `serde_json`'s compact encoding.
pub fn canonical_json_bytes<T: Serialize + ?Sized>(
    value: &T,
) -> Result<Vec<u8>, serde_json::Error> {
    canonical_json_bytes_with_version(CanonicalJsonVersion::CURRENT, value)
}

pub fn canonical_json_bytes_with_version<T: Serialize + ?Sized>(
    version: CanonicalJsonVersion,
    value: &T,
) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    let mut output = Vec::new();
    match version {
        CanonicalJsonVersion::V1 => encode_v1(&value, &mut output)?,
    }
    Ok(output)
}

/// SHA-256 of the exact byte sequence returned by [`canonical_json_bytes`].
pub fn canonical_json_sha256<T: Serialize + ?Sized>(
    value: &T,
) -> Result<String, serde_json::Error> {
    canonical_json_bytes(value).map(|bytes| sha256_hex(&bytes))
}

fn encode_v1(value: &Value, output: &mut Vec<u8>) -> Result<(), serde_json::Error> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_writer(output, value)?;
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                encode_v1(value, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                encode_v1(value, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

/// SHA-256 of exactly the original input bytes, without a prefix or suffix.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawContentIdentity {
    pub byte_length: u64,
    pub sha256: String,
}

impl RawContentIdentity {
    pub fn new(bytes: &[u8]) -> Self {
        Self {
            byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: sha256_hex(bytes),
        }
    }
}

/// SHA-256 of decoded text's UTF-8 bytes, including replacements when lossy.
///
/// Encoding, loss state, and diagnostics are metadata outside the hashed bytes.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecodedContentIdentity {
    pub encoding: String,
    pub byte_length: u64,
    pub sha256: String,
    pub lossy: bool,
}

impl DecodedContentIdentity {
    pub fn new(text: &str, encoding: impl Into<String>, lossy: bool) -> Self {
        Self {
            encoding: encoding.into(),
            byte_length: u64::try_from(text.len()).unwrap_or(u64::MAX),
            sha256: sha256_hex(text.as_bytes()),
            lossy,
        }
    }
}

/// SHA-256 of the exact canonical JSON bytes of an authoritative payload.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalPayloadIdentity {
    pub canonicalization: CanonicalJsonVersion,
    pub payload_schema_version: SchemaVersion,
    pub byte_length: u64,
    pub sha256: String,
}

impl CanonicalPayloadIdentity {
    pub fn new<T: Serialize + ?Sized>(
        payload_schema_version: impl Into<SchemaVersion>,
        payload: &T,
    ) -> Result<Self, serde_json::Error> {
        let canonicalization = CanonicalJsonVersion::CURRENT;
        let bytes = canonical_json_bytes_with_version(canonicalization, payload)?;
        Ok(Self {
            canonicalization,
            payload_schema_version: payload_schema_version.into(),
            byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: sha256_hex(&bytes),
        })
    }
}

/// Detected parser-format name paired with an Internet media type when known.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct FormatIdentity {
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl FormatIdentity {
    pub fn new(format: impl Into<String>, media_type: Option<impl Into<String>>) -> Self {
        Self {
            format: format.into(),
            media_type: media_type.map(Into::into),
        }
    }
}

/// Signal family that contributed to a detection candidate.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DetectionEvidenceKind {
    MagicBytes,
    DeclaredMediaType,
    Extension,
    Filename,
    ContainerManifest,
    Charset,
    Structure,
    Shebang,
    GrammarProbe,
    Fallback,
}

/// One retained reason for a ranked detection candidate.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct DetectionEvidence {
    pub kind: DetectionEvidenceKind,
    pub description: String,
}

impl DetectionEvidence {
    pub fn new(kind: DetectionEvidenceKind, description: impl Into<String>) -> Self {
        Self {
            kind,
            description: description.into(),
        }
    }
}

/// A ranked format candidate with all evidence retained.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectionCandidate {
    /// One-based rank; lower ranks are preferred.
    pub rank: u32,
    pub identity: FormatIdentity,
    pub confidence: f32,
    pub evidence: Vec<DetectionEvidence>,
    #[serde(default)]
    pub parser_availability: ParserAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser_id: Option<String>,
}

impl DetectionCandidate {
    pub fn new(
        rank: u32,
        identity: FormatIdentity,
        confidence: f32,
        evidence: Vec<DetectionEvidence>,
    ) -> Self {
        Self {
            rank,
            identity,
            confidence,
            evidence,
            parser_availability: ParserAvailability::Unregistered,
            parser_id: None,
        }
    }
}

/// Whether a ranked candidate can be interpreted by the active registry.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ParserAvailability {
    Available,
    Unavailable,
    #[default]
    Unregistered,
}

/// Stable member summary used in a compound-input aggregate manifest.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AggregateMemberIdentity {
    pub member_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_index: Option<u64>,
    pub byte_length: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_payload_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<FormatIdentity>,
}

impl AggregateMemberIdentity {
    pub fn new(
        member_path: impl Into<String>,
        member_index: Option<u64>,
        identity: &ContentIdentity,
    ) -> Self {
        Self {
            member_path: member_path.into(),
            member_index,
            byte_length: identity.byte_length(),
            raw_sha256: identity.raw.as_ref().map(|raw| raw.sha256.clone()),
            decoded_sha256: identity
                .decoded
                .as_ref()
                .map(|decoded| decoded.sha256.clone()),
            canonical_payload_sha256: identity
                .canonical_payload
                .as_ref()
                .map(|canonical| canonical.sha256.clone()),
            aggregate_sha256: identity
                .aggregate
                .as_ref()
                .map(|aggregate| aggregate.sha256.clone()),
            format: identity.format.clone(),
        }
    }
}

/// Content identity for a compound input.
///
/// Members are sorted by member index, then path and identity fields. `sha256`
/// hashes canonical JSON v1 of `schema_version`, `canonicalization`, and the
/// sorted `members` array. Summary fields and the digest itself are excluded.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AggregateContentIdentity {
    pub schema_version: String,
    pub canonicalization: CanonicalJsonVersion,
    pub member_count: u64,
    pub total_byte_length: u64,
    pub manifest_byte_length: u64,
    pub sha256: String,
    pub members: Vec<AggregateMemberIdentity>,
}

#[derive(Serialize)]
struct AggregateManifest<'a> {
    schema_version: &'static str,
    canonicalization: CanonicalJsonVersion,
    members: &'a [AggregateMemberIdentity],
}

impl AggregateContentIdentity {
    pub const SCHEMA_VERSION: &'static str = "grist/aggregate-identity/v1";

    pub fn new(mut members: Vec<AggregateMemberIdentity>) -> Result<Self, serde_json::Error> {
        members.sort_by(compare_members);
        let total_byte_length = members.iter().try_fold(0_u64, |total, member| {
            total.checked_add(member.byte_length).ok_or_else(|| {
                serde_json::Error::io(std::io::Error::other(
                    "aggregate member byte length exceeds u64",
                ))
            })
        })?;
        let canonicalization = CanonicalJsonVersion::CURRENT;
        let manifest = AggregateManifest {
            schema_version: Self::SCHEMA_VERSION,
            canonicalization,
            members: &members,
        };
        let bytes = canonical_json_bytes_with_version(canonicalization, &manifest)?;
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            canonicalization,
            member_count: u64::try_from(members.len()).unwrap_or(u64::MAX),
            total_byte_length,
            manifest_byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: sha256_hex(&bytes),
            members,
        })
    }
}

fn compare_members(left: &AggregateMemberIdentity, right: &AggregateMemberIdentity) -> Ordering {
    left.member_index
        .is_none()
        .cmp(&right.member_index.is_none())
        .then_with(|| left.member_index.cmp(&right.member_index))
        .then_with(|| {
            left.member_path
                .as_bytes()
                .cmp(right.member_path.as_bytes())
        })
        .then_with(|| left.byte_length.cmp(&right.byte_length))
        .then_with(|| left.raw_sha256.cmp(&right.raw_sha256))
        .then_with(|| left.decoded_sha256.cmp(&right.decoded_sha256))
        .then_with(|| {
            left.canonical_payload_sha256
                .cmp(&right.canonical_payload_sha256)
        })
        .then_with(|| left.aggregate_sha256.cmp(&right.aggregate_sha256))
        .then_with(|| left.format.cmp(&right.format))
}

/// Complete singular or compound content identity retained by an envelope.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ContentIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<RawContentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded: Option<DecodedContentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_payload: Option<CanonicalPayloadIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<FormatIdentity>,
    #[serde(default)]
    pub detection_candidates: Vec<DetectionCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<AggregateContentIdentity>,
}

impl ContentIdentity {
    pub fn for_raw_bytes(bytes: &[u8]) -> Self {
        Self {
            raw: Some(RawContentIdentity::new(bytes)),
            ..Self::default()
        }
    }

    pub fn for_compound(members: Vec<AggregateMemberIdentity>) -> Result<Self, serde_json::Error> {
        Ok(Self {
            aggregate: Some(AggregateContentIdentity::new(members)?),
            ..Self::default()
        })
    }

    pub fn byte_length(&self) -> u64 {
        self.raw.as_ref().map_or_else(
            || {
                self.aggregate
                    .as_ref()
                    .map_or(0, |aggregate| aggregate.total_byte_length)
            },
            |raw| raw.byte_length,
        )
    }

    pub fn with_decoded(mut self, text: &str, encoding: impl Into<String>, lossy: bool) -> Self {
        self.decoded = Some(DecodedContentIdentity::new(text, encoding, lossy));
        self
    }

    pub fn with_canonical_payload<T: Serialize + ?Sized>(
        mut self,
        payload_schema_version: impl Into<SchemaVersion>,
        payload: &T,
    ) -> Result<Self, serde_json::Error> {
        self.canonical_payload = Some(CanonicalPayloadIdentity::new(
            payload_schema_version,
            payload,
        )?);
        Ok(self)
    }

    pub fn with_format(mut self, format: FormatIdentity) -> Self {
        self.format = Some(format);
        self
    }

    pub fn with_detection_candidates(mut self, mut candidates: Vec<DetectionCandidate>) -> Self {
        for candidate in &mut candidates {
            candidate.evidence.sort();
        }
        candidates.sort_by(|left, right| {
            left.rank
                .cmp(&right.rank)
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| left.identity.cmp(&right.identity))
                .then_with(|| left.evidence.cmp(&right.evidence))
        });
        self.detection_candidates = candidates;
        self
    }

    pub fn with_aggregate(mut self, aggregate: AggregateContentIdentity) -> Self {
        self.aggregate = Some(aggregate);
        self
    }
}

impl From<Hashes> for ContentIdentity {
    fn from(hashes: Hashes) -> Self {
        Self {
            raw: Some(RawContentIdentity {
                byte_length: u64::try_from(hashes.size_bytes).unwrap_or(u64::MAX),
                sha256: hashes.sha256,
            }),
            decoded: hashes.text_sha256.map(|sha256| DecodedContentIdentity {
                encoding: "utf-8".to_string(),
                byte_length: u64::try_from(hashes.size_bytes).unwrap_or(u64::MAX),
                sha256,
                lossy: false,
            }),
            ..Self::default()
        }
    }
}
