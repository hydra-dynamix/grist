//! Lossless, bounded BibTeX and BibLaTeX parsing and resolution.
//!
//! The parser never invokes TeX, follows file references, or fetches network
//! resources. Source spelling and ranges remain authoritative; expanded values
//! and inherited fields are explicit derived views with provenance.

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, Envelope, FormatIdentity, LineIndex, OperationKind,
    OperationStatus, ParserInfo, SchemaVersion, SourceInfo, SourceLocator, SourceRange,
    options_digest,
};
use crate::decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeReport, DecodedText, TextEncoding, decode_text,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BibliographyDialect {
    #[default]
    Auto,
    Bibtex,
    Biblatex,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct BibliographyOptions {
    pub dialect: BibliographyDialect,
    pub encoding: Option<String>,
    pub retain_comments: bool,
    pub max_entries: u32,
    pub max_fields_per_entry: u32,
    pub max_value_parts: u32,
    pub max_string_depth: u16,
    pub max_string_expansions: u32,
    pub max_expanded_characters: u64,
    pub max_crossref_depth: u16,
}

impl Default for BibliographyOptions {
    fn default() -> Self {
        Self {
            dialect: BibliographyDialect::Auto,
            encoding: None,
            retain_comments: true,
            max_entries: 100_000,
            max_fields_per_entry: 4_096,
            max_value_parts: 4_096,
            max_string_depth: 32,
            max_string_expansions: 100_000,
            max_expanded_characters: 16 * 1024 * 1024,
            max_crossref_depth: 32,
        }
    }
}

impl crate::core::FormatOptions for BibliographyOptions {
    const FORMAT: &'static str = "bibtex";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BibliographyDocument {
    pub schema_version: String,
    #[serde(default)]
    pub raw_bytes: Vec<u8>,
    #[serde(default)]
    pub decoded_text: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub source: SourceInfo,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub dialect: BibliographyDialect,
    #[serde(default)]
    pub constructs: Vec<BibliographyConstruct>,
    #[serde(default)]
    pub strings: Vec<BibliographyString>,
    #[serde(default)]
    pub preambles: Vec<BibliographyValue>,
    #[serde(default)]
    pub entries: Vec<BibliographyEntry>,
    #[serde(default)]
    pub duplicate_keys: Vec<DuplicateBibliographyKey>,
    #[serde(default)]
    pub crossrefs: Vec<CrossrefResolution>,
    #[serde(default)]
    pub parse_errors: Vec<BibliographyParseError>,
}

pub type BibliographyEnvelope = Envelope<BibliographyDocument>;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BibliographyConstructKind {
    Entry,
    String,
    Preamble,
    Comment,
    LineComment,
    Raw,
    Malformed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BibliographyConstruct {
    pub kind: BibliographyConstructKind,
    pub raw: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub string_index: Option<usize>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BibliographyEntry {
    pub index: usize,
    pub entry_type: String,
    pub raw_entry_type: String,
    pub key: String,
    pub key_range: SourceRange,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub source: SourceInfo,
    pub raw: String,
    #[serde(default)]
    pub fields: Vec<BibliographyField>,
    #[serde(default)]
    pub effective_fields: BTreeMap<String, ResolvedBibliographyField>,
}

impl BibliographyEntry {
    pub fn field(&self, name: &str) -> Option<&BibliographyField> {
        self.fields
            .iter()
            .rev()
            .find(|field| field.name.eq_ignore_ascii_case(name))
    }

    pub fn effective_field(&self, name: &str) -> Option<&ResolvedBibliographyField> {
        self.effective_fields.get(&name.to_ascii_lowercase())
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BibliographyField {
    pub name: String,
    pub raw_name: String,
    pub name_range: SourceRange,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub source: SourceInfo,
    pub raw: String,
    pub value: BibliographyValue,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BibliographyString {
    pub index: usize,
    pub name: String,
    pub raw_name: String,
    pub name_range: SourceRange,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub source: SourceInfo,
    pub raw: String,
    pub value: BibliographyValue,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BibliographyValue {
    pub raw: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub source: SourceInfo,
    #[serde(default)]
    pub parts: Vec<BibliographyValuePart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    pub expansion_status: ValueExpansionStatus,
    #[serde(default)]
    pub provenance: Vec<ValueProvenance>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BibliographyValuePartKind {
    Braced,
    Quoted,
    Number,
    StringIdentifier,
    Malformed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BibliographyValuePart {
    pub kind: BibliographyValuePartKind,
    pub raw: String,
    pub text: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub source: SourceInfo,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValueExpansionStatus {
    #[default]
    Literal,
    Expanded,
    UnresolvedIdentifier,
    AmbiguousIdentifier,
    Cycle,
    DepthExceeded,
    ExpansionLimit,
    OutputLimit,
    Malformed,
}

impl ValueExpansionStatus {
    fn is_resolved(self) -> bool {
        matches!(self, Self::Literal | Self::Expanded)
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValueProvenanceKind {
    Literal,
    BuiltInString,
    StringDefinition,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValueProvenance {
    pub kind: ValueProvenanceKind,
    pub name: Option<String>,
    pub string_index: Option<usize>,
    pub range: Option<SourceRange>,
    pub locator: Option<SourceLocator>,
    pub source: Option<SourceInfo>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedBibliographyField {
    pub name: String,
    pub source_entry_index: usize,
    pub source_entry_key: String,
    pub inherited: bool,
    pub value: BibliographyValue,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuplicateBibliographyKey {
    pub key: String,
    pub entry_indices: Vec<usize>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrossrefStatus {
    Resolved,
    Unresolved,
    Ambiguous,
    Cycle,
    DepthExceeded,
    ValueUnresolved,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrossrefResolution {
    pub source_entry_index: usize,
    pub source_key: String,
    pub field: String,
    pub target_key: String,
    pub status: CrossrefStatus,
    #[serde(default)]
    pub target_entry_indices: Vec<usize>,
    #[serde(default)]
    pub inherited_fields: Vec<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BibliographyParseError {
    pub code: String,
    pub message: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CitationResolutionStatus {
    Resolved,
    Missing,
    Ambiguous,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CitationResolution {
    pub requested_key: String,
    pub status: CitationResolutionStatus,
    #[serde(default)]
    pub candidate_entry_indices: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<BibliographyEntry>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CitationResolutionReport {
    pub schema_version: String,
    #[serde(default)]
    pub resolutions: Vec<CitationResolution>,
}

impl BibliographyDocument {
    pub fn entries_for_key(&self, key: &str) -> Vec<&BibliographyEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.key == key)
            .collect()
    }

    pub fn resolve_citation(&self, key: &str) -> CitationResolution {
        let candidates = self
            .entries
            .iter()
            .filter(|entry| entry.key == key)
            .collect::<Vec<_>>();
        let candidate_entry_indices = candidates.iter().map(|entry| entry.index).collect();
        match candidates.as_slice() {
            [] => CitationResolution {
                requested_key: key.to_string(),
                status: CitationResolutionStatus::Missing,
                candidate_entry_indices,
                entry: None,
            },
            [entry] => CitationResolution {
                requested_key: key.to_string(),
                status: CitationResolutionStatus::Resolved,
                candidate_entry_indices,
                entry: Some((*entry).clone()),
            },
            _ => CitationResolution {
                requested_key: key.to_string(),
                status: CitationResolutionStatus::Ambiguous,
                candidate_entry_indices,
                entry: None,
            },
        }
    }

    pub fn resolve_citations<I, S>(&self, keys: I) -> CitationResolutionReport
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        CitationResolutionReport {
            schema_version: SchemaVersion::BIBLIOGRAPHY_CITATION_RESOLUTION_V1.to_string(),
            resolutions: keys
                .into_iter()
                .map(|key| self.resolve_citation(key.as_ref()))
                .collect(),
        }
    }
}

pub fn resolve_citations<I, S>(document: &BibliographyDocument, keys: I) -> CitationResolutionReport
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    document.resolve_citations(keys)
}

pub fn parse_bibliography(
    text: &str,
    source: SourceInfo,
    options: &BibliographyOptions,
) -> BibliographyEnvelope {
    parse_bibliography_bytes(text.as_bytes(), source, options)
}

pub fn parse_bibliography_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &BibliographyOptions,
) -> BibliographyEnvelope {
    let mut decode_options =
        DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("bibtex"));
    decode_options.context = DecodeContext::PlainText;
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    match decode_text(bytes, &decode_options) {
        Ok(decoded) => envelope_from_decoded(&decoded, source, options),
        Err(error) => failed_decode_envelope(bytes, source, options, error),
    }
}

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new("grist.bibliography")
        .with_implementation("grist-safe-bibliography", env!("CARGO_PKG_VERSION"))
        .with_feature("bibliography")
}

fn failed_decode_envelope(
    bytes: &[u8],
    source: SourceInfo,
    options: &BibliographyOptions,
    error: DecodeError,
) -> BibliographyEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Bibliography,
        OperationStatus::Failed,
        source,
        parser_info(),
        options_digest(options).expect("bibliography options serialize"),
        SchemaVersion::BIBLIOGRAPHY_V1,
    )
    .expect("valid failed bibliography envelope")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes)
            .with_format(FormatIdentity::new("bibtex", Some("application/x-bibtex"))),
    )
    .with_diagnostics(vec![error.diagnostic().with_parser("grist.bibliography")])
}

fn envelope_from_decoded(
    decoded: &DecodedText,
    source: SourceInfo,
    options: &BibliographyOptions,
) -> BibliographyEnvelope {
    let mut parser = BibliographyParser::new(&decoded.text, source.clone(), options);
    parser.parse();
    expand_all_values(
        &mut parser.entries,
        &mut parser.strings,
        &mut parser.preambles,
        options,
        &mut parser.diagnostics,
    );
    let duplicate_keys = find_duplicate_keys(&parser.entries, &mut parser.diagnostics);
    let crossrefs = resolve_all_crossrefs(&mut parser.entries, options, &mut parser.diagnostics);
    let dialect = detect_dialect(options.dialect, &parser.entries);
    let whole = parser.range(0, decoded.text.len());
    let payload = BibliographyDocument {
        schema_version: SchemaVersion::BIBLIOGRAPHY_V1.to_string(),
        raw_bytes: decoded.raw_bytes().to_vec(),
        decoded_text: decoded.text.clone(),
        range: whole.clone(),
        locator: exact_locator(whole),
        source: source.clone(),
        encoding: decoded.report.encoding.clone(),
        decoding: decoded.report.clone(),
        dialect,
        constructs: parser.constructs,
        strings: parser.strings,
        preambles: parser.preambles,
        entries: parser.entries,
        duplicate_keys,
        crossrefs,
        parse_errors: parser.parse_errors,
    };
    let partial = decoded.report.makes_operation_partial()
        || parser
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.partial);
    let digest = options_digest(options).expect("bibliography options serialize");
    let mut diagnostics = decoded.report.diagnostics.clone();
    diagnostics.append(&mut parser.diagnostics);
    let mut envelope = if partial {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Bibliography,
            source,
            parser_info(),
            digest,
            SchemaVersion::BIBLIOGRAPHY_V1,
            Some(payload),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Bibliography,
            source,
            parser_info(),
            digest,
            SchemaVersion::BIBLIOGRAPHY_V1,
            payload,
        )
    };
    envelope.diagnostics = diagnostics;
    envelope
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    envelope
        .with_identity(
            ContentIdentity::for_raw_bytes(decoded.raw_bytes())
                .with_decoded(
                    &decoded.text,
                    decoded.report.encoding.label(),
                    decoded.report.is_lossy(),
                )
                .with_format(FormatIdentity::new("bibtex", Some("application/x-bibtex"))),
        )
        .with_canonical_payload_identity()
        .expect("canonical bibliography payload")
}

struct BibliographyParser<'a> {
    text: &'a str,
    index: LineIndex,
    source: SourceInfo,
    options: &'a BibliographyOptions,
    constructs: Vec<BibliographyConstruct>,
    strings: Vec<BibliographyString>,
    preambles: Vec<BibliographyValue>,
    entries: Vec<BibliographyEntry>,
    parse_errors: Vec<BibliographyParseError>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> BibliographyParser<'a> {
    fn new(text: &'a str, source: SourceInfo, options: &'a BibliographyOptions) -> Self {
        Self {
            text,
            index: LineIndex::new(text),
            source,
            options,
            constructs: Vec::new(),
            strings: Vec::new(),
            preambles: Vec::new(),
            entries: Vec::new(),
            parse_errors: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn range(&self, start: usize, end: usize) -> SourceRange {
        SourceRange::new(start, end, &self.index)
    }

    fn locator(&self, start: usize, end: usize) -> SourceLocator {
        exact_locator(self.range(start, end))
    }

    fn parse(&mut self) {
        let mut cursor = 0;
        while cursor < self.text.len() {
            match self.text.as_bytes()[cursor] {
                b'@' => cursor = self.parse_at_construct(cursor),
                b'%' => cursor = self.parse_line_comment(cursor),
                _ => cursor = self.parse_raw(cursor),
            }
        }
    }

    fn parse_line_comment(&mut self, start: usize) -> usize {
        let end = self.text[start..]
            .find('\n')
            .map(|offset| start + offset + 1)
            .unwrap_or(self.text.len());
        if self.options.retain_comments {
            self.push_construct(
                BibliographyConstructKind::LineComment,
                start,
                end,
                None,
                None,
            );
        }
        end
    }

    fn parse_raw(&mut self, start: usize) -> usize {
        let mut end = start;
        while end < self.text.len() && !matches!(self.text.as_bytes()[end], b'@' | b'%') {
            end += 1;
        }
        if !self.text[start..end].trim().is_empty() {
            self.push_construct(BibliographyConstructKind::Raw, start, end, None, None);
        }
        end
    }

    fn parse_at_construct(&mut self, start: usize) -> usize {
        let mut cursor = start + 1;
        cursor = skip_whitespace(self.text, cursor, self.text.len());
        let type_start = cursor;
        while cursor < self.text.len() && is_name_byte(self.text.as_bytes()[cursor]) {
            cursor += 1;
        }
        if cursor == type_start {
            let end = line_end(self.text, start);
            self.push_error(
                "bibliography.construct.missing_type",
                "`@` is not followed by a construct type",
                start,
                end,
            );
            self.push_construct(BibliographyConstructKind::Malformed, start, end, None, None);
            return end;
        }
        let raw_type = &self.text[type_start..cursor];
        let construct_type = raw_type.to_ascii_lowercase();
        cursor = skip_whitespace(self.text, cursor, self.text.len());
        if cursor >= self.text.len() || !matches!(self.text.as_bytes()[cursor], b'{' | b'(') {
            let end = line_end(self.text, start);
            self.push_error(
                "bibliography.construct.missing_delimiter",
                format!("@{raw_type} is missing an opening brace or parenthesis"),
                start,
                end,
            );
            self.push_construct(BibliographyConstructKind::Malformed, start, end, None, None);
            return end;
        }
        let open = self.text.as_bytes()[cursor];
        let body_start = cursor + 1;
        let (end, terminated) = scan_construct_end(self.text, cursor, open);
        let body_end = if terminated { end - 1 } else { end };
        if !terminated {
            self.push_error(
                "bibliography.construct.unterminated",
                format!("@{raw_type} is not terminated"),
                start,
                end,
            );
        }
        match construct_type.as_str() {
            "comment" => {
                if self.options.retain_comments {
                    self.push_construct(BibliographyConstructKind::Comment, start, end, None, None);
                }
            }
            "string" => self.parse_string_construct(start, end, body_start, body_end),
            "preamble" => self.parse_preamble_construct(start, end, body_start, body_end),
            _ => self.parse_entry_construct(
                start, end, type_start, cursor, body_start, body_end, raw_type,
            ),
        }
        end.max(start + 1)
    }

    fn parse_string_construct(
        &mut self,
        start: usize,
        end: usize,
        body_start: usize,
        body_end: usize,
    ) {
        let fields = self.parse_fields(body_start, body_end, 1);
        let Some(field) = fields.into_iter().next() else {
            self.push_error(
                "bibliography.string.malformed",
                "@string must contain a name and value",
                body_start,
                body_end,
            );
            self.push_construct(BibliographyConstructKind::Malformed, start, end, None, None);
            return;
        };
        let string_index = self.strings.len();
        self.strings.push(BibliographyString {
            index: string_index,
            name: field.name,
            raw_name: field.raw_name,
            name_range: field.name_range,
            range: self.range(start, end),
            locator: self.locator(start, end),
            source: self.source.clone(),
            raw: self.text[start..end].to_string(),
            value: field.value,
        });
        self.push_construct(
            BibliographyConstructKind::String,
            start,
            end,
            None,
            Some(string_index),
        );
    }

    fn parse_preamble_construct(
        &mut self,
        start: usize,
        end: usize,
        body_start: usize,
        body_end: usize,
    ) {
        let value_start = skip_whitespace_and_comments(self.text, body_start, body_end);
        let (value, _) = self.parse_value(value_start, body_end);
        self.preambles.push(value);
        self.push_construct(BibliographyConstructKind::Preamble, start, end, None, None);
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_entry_construct(
        &mut self,
        start: usize,
        end: usize,
        type_start: usize,
        open_offset: usize,
        body_start: usize,
        body_end: usize,
        raw_type: &str,
    ) {
        if self.entries.len() >= self.options.max_entries as usize {
            self.push_error(
                "bibliography.limit.entries",
                format!("entry limit {} reached", self.options.max_entries),
                start,
                end,
            );
            self.push_construct(BibliographyConstructKind::Raw, start, end, None, None);
            return;
        }
        let comma = find_top_level_comma(self.text, body_start, body_end);
        let key_end = comma.unwrap_or(body_end);
        let (key_start, key_end) = trim_range(self.text, body_start, key_end);
        let key = self.text[key_start..key_end].to_string();
        if key.is_empty() {
            self.push_error(
                "bibliography.entry.missing_key",
                format!("@{raw_type} entry has an empty citation key"),
                body_start,
                key_end.max(body_start),
            );
        }
        let fields = comma
            .map(|comma| self.parse_fields(comma + 1, body_end, self.options.max_fields_per_entry))
            .unwrap_or_default();
        self.report_duplicate_fields(&fields);
        let entry_index = self.entries.len();
        self.entries.push(BibliographyEntry {
            index: entry_index,
            entry_type: raw_type.to_ascii_lowercase(),
            raw_entry_type: raw_type.to_string(),
            key,
            key_range: self.range(key_start, key_end),
            range: self.range(start, end),
            locator: self.locator(start, end),
            source: self.source.clone(),
            raw: self.text[start..end].to_string(),
            fields,
            effective_fields: BTreeMap::new(),
        });
        let _type_range = self.range(type_start, open_offset);
        self.push_construct(
            BibliographyConstructKind::Entry,
            start,
            end,
            Some(entry_index),
            None,
        );
    }

    fn report_duplicate_fields(&mut self, fields: &[BibliographyField]) {
        let mut seen = BTreeSet::new();
        for field in fields {
            if !seen.insert(field.name.clone()) {
                self.push_error(
                    "bibliography.entry.duplicate_field",
                    format!("duplicate field `{}` is preserved", field.raw_name),
                    field.name_range.byte_start,
                    field.name_range.byte_end,
                );
            }
        }
    }

    fn push_construct(
        &mut self,
        kind: BibliographyConstructKind,
        start: usize,
        end: usize,
        entry_index: Option<usize>,
        string_index: Option<usize>,
    ) {
        self.constructs.push(BibliographyConstruct {
            kind,
            raw: self.text[start..end].to_string(),
            range: self.range(start, end),
            locator: self.locator(start, end),
            entry_index,
            string_index,
        });
    }

    fn push_error(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        start: usize,
        end: usize,
    ) {
        let code = code.into();
        let message = message.into();
        let range = self.range(start, end);
        let locator = exact_locator(range.clone());
        self.parse_errors.push(BibliographyParseError {
            code: code.clone(),
            message: message.clone(),
            range: range.clone(),
            locator,
        });
        self.diagnostics.push(
            Diagnostic::error("grist.bibliography", code, message)
                .with_parser("grist.bibliography")
                .with_range(range)
                .partial(),
        );
    }
}

impl BibliographyParser<'_> {
    fn parse_fields(&mut self, start: usize, end: usize, limit: u32) -> Vec<BibliographyField> {
        let mut fields = Vec::new();
        let mut cursor = start;
        while cursor < end {
            cursor = skip_field_separators(self.text, cursor, end);
            if cursor >= end {
                break;
            }
            if fields.len() >= limit as usize {
                self.push_error(
                    "bibliography.limit.fields",
                    format!("field limit {limit} reached"),
                    cursor,
                    end,
                );
                break;
            }
            let name_start = cursor;
            while cursor < end && is_name_byte(self.text.as_bytes()[cursor]) {
                cursor += 1;
            }
            let name_end = cursor;
            if name_start == name_end {
                let next = next_char_offset(self.text, cursor).min(end);
                self.push_error(
                    "bibliography.field.malformed_name",
                    "field name is malformed",
                    cursor,
                    next,
                );
                cursor = next;
                continue;
            }
            let raw_name = self.text[name_start..name_end].to_string();
            cursor = skip_whitespace_and_comments(self.text, cursor, end);
            if cursor >= end || self.text.as_bytes()[cursor] != b'=' {
                let next = find_top_level_comma(self.text, cursor, end)
                    .map(|comma| comma + 1)
                    .unwrap_or(end);
                self.push_error(
                    "bibliography.field.missing_equals",
                    format!("field `{raw_name}` is missing `=`"),
                    name_start,
                    next,
                );
                cursor = next;
                continue;
            }
            cursor += 1;
            let value_start = skip_whitespace_and_comments(self.text, cursor, end);
            let (value, value_end) = self.parse_value(value_start, end);
            let field_end = value_end.max(value_start);
            fields.push(BibliographyField {
                name: raw_name.to_ascii_lowercase(),
                raw_name,
                name_range: self.range(name_start, name_end),
                range: self.range(name_start, field_end),
                locator: self.locator(name_start, field_end),
                source: self.source.clone(),
                raw: self.text[name_start..field_end].to_string(),
                value,
            });
            cursor = skip_whitespace_and_comments(self.text, field_end, end);
            if cursor < end && self.text.as_bytes()[cursor] == b',' {
                cursor += 1;
            } else if cursor < end {
                let next = find_top_level_comma(self.text, cursor, end)
                    .map(|comma| comma + 1)
                    .unwrap_or(end);
                self.push_error(
                    "bibliography.field.trailing_syntax",
                    "unexpected syntax after field value is retained in the entry raw text",
                    cursor,
                    next,
                );
                cursor = next;
            }
        }
        fields
    }

    fn parse_value(&mut self, start: usize, end: usize) -> (BibliographyValue, usize) {
        let mut cursor = start;
        let mut parts = Vec::new();
        let mut malformed = false;
        let mut last_end = start;
        loop {
            cursor = skip_whitespace_and_comments(self.text, cursor, end);
            if cursor >= end || self.text.as_bytes()[cursor] == b',' {
                break;
            }
            if parts.len() >= self.options.max_value_parts as usize {
                self.push_error(
                    "bibliography.limit.value_parts",
                    format!("value-part limit {} reached", self.options.max_value_parts),
                    start,
                    cursor,
                );
                malformed = true;
                last_end = find_top_level_comma(self.text, cursor, end).unwrap_or(end);
                break;
            }
            let part_start = cursor;
            let (part_end, content_start, content_end, kind, terminated) =
                match self.text.as_bytes()[cursor] {
                    b'{' => {
                        let (part_end, terminated) = scan_braced_value(self.text, cursor, end);
                        let content_end = if terminated { part_end - 1 } else { part_end };
                        (
                            part_end,
                            cursor + 1,
                            content_end,
                            BibliographyValuePartKind::Braced,
                            terminated,
                        )
                    }
                    b'"' => {
                        let (part_end, terminated) = scan_quoted_value(self.text, cursor, end);
                        let content_end = if terminated { part_end - 1 } else { part_end };
                        (
                            part_end,
                            cursor + 1,
                            content_end,
                            BibliographyValuePartKind::Quoted,
                            terminated,
                        )
                    }
                    _ => {
                        while cursor < end
                            && !matches!(
                                self.text.as_bytes()[cursor],
                                b'#' | b',' | b'%' | b' ' | b'\t' | b'\r' | b'\n'
                            )
                        {
                            cursor += 1;
                        }
                        let kind = if self.text[part_start..cursor]
                            .bytes()
                            .all(|byte| byte.is_ascii_digit())
                        {
                            BibliographyValuePartKind::Number
                        } else {
                            BibliographyValuePartKind::StringIdentifier
                        };
                        (cursor, part_start, cursor, kind, cursor > part_start)
                    }
                };
            if !terminated {
                malformed = true;
                self.push_error(
                    "bibliography.value.unterminated",
                    "quoted or braced bibliography value is not terminated",
                    part_start,
                    part_end,
                );
            }
            let actual_kind = if terminated {
                kind
            } else {
                BibliographyValuePartKind::Malformed
            };
            parts.push(BibliographyValuePart {
                kind: actual_kind,
                raw: self.text[part_start..part_end].to_string(),
                text: self.text[content_start..content_end].to_string(),
                range: self.range(part_start, part_end),
                locator: self.locator(part_start, part_end),
                source: self.source.clone(),
            });
            cursor = part_end;
            last_end = part_end;
            cursor = skip_whitespace_and_comments(self.text, cursor, end);
            if cursor < end && self.text.as_bytes()[cursor] == b'#' {
                cursor += 1;
                if skip_whitespace_and_comments(self.text, cursor, end) >= end {
                    malformed = true;
                    self.push_error(
                        "bibliography.value.missing_part",
                        "value concatenation is missing its right-hand part",
                        part_start,
                        cursor,
                    );
                    last_end = cursor;
                    break;
                }
                continue;
            }
            break;
        }
        if parts.is_empty() {
            malformed = true;
            self.push_error(
                "bibliography.value.empty",
                "field value is empty",
                start,
                start,
            );
        }
        let range = self.range(start, last_end);
        (
            BibliographyValue {
                raw: self.text[start..last_end].to_string(),
                range: range.clone(),
                locator: exact_locator(range),
                source: self.source.clone(),
                parts,
                resolved: None,
                expansion_status: if malformed {
                    ValueExpansionStatus::Malformed
                } else {
                    ValueExpansionStatus::Literal
                },
                provenance: Vec::new(),
            },
            last_end,
        )
    }
}

fn exact_locator(range: SourceRange) -> SourceLocator {
    SourceLocator::exact(range).expect("bibliography ranges are valid UTF-8 ranges")
}

fn next_char_offset(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    offset
        + text[offset..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(0)
}

fn skip_whitespace(text: &str, mut cursor: usize, end: usize) -> usize {
    while cursor < end {
        let Some(value) = text[cursor..end].chars().next() else {
            break;
        };
        if !value.is_whitespace() {
            break;
        }
        cursor += value.len_utf8();
    }
    cursor
}

fn skip_whitespace_and_comments(text: &str, mut cursor: usize, end: usize) -> usize {
    loop {
        cursor = skip_whitespace(text, cursor, end);
        if cursor >= end || text.as_bytes()[cursor] != b'%' {
            return cursor;
        }
        cursor = text[cursor..end]
            .find('\n')
            .map(|offset| cursor + offset + 1)
            .unwrap_or(end);
    }
}

fn skip_field_separators(text: &str, mut cursor: usize, end: usize) -> usize {
    loop {
        cursor = skip_whitespace_and_comments(text, cursor, end);
        if cursor < end && text.as_bytes()[cursor] == b',' {
            cursor += 1;
        } else {
            return cursor;
        }
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.' | b'/')
}

fn line_end(text: &str, start: usize) -> usize {
    text[start..]
        .find('\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(text.len())
}

fn trim_range(text: &str, mut start: usize, mut end: usize) -> (usize, usize) {
    start = skip_whitespace(text, start, end);
    while end > start {
        let Some(value) = text[start..end].chars().next_back() else {
            break;
        };
        if !value.is_whitespace() {
            break;
        }
        end -= value.len_utf8();
    }
    (start, end)
}

fn scan_construct_end(text: &str, open_offset: usize, open: u8) -> (usize, bool) {
    let close = if open == b'{' { b'}' } else { b')' };
    let mut cursor = open_offset + 1;
    let mut outer_depth = 1usize;
    let mut brace_depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    while cursor < text.len() {
        let byte = text.as_bytes()[cursor];
        if quoted {
            if byte == b'"' && brace_depth == 0 && !escaped {
                quoted = false;
            } else if byte == b'{' && !escaped {
                brace_depth += 1;
            } else if byte == b'}' && brace_depth > 0 && !escaped {
                brace_depth -= 1;
            }
            escaped = byte == b'\\' && !escaped;
            if byte != b'\\' {
                escaped = false;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' if open == b'{' => outer_depth += 1,
            b'}' if open == b'{' => {
                outer_depth -= 1;
                if outer_depth == 0 {
                    return (cursor + 1, true);
                }
            }
            b'{' => brace_depth += 1,
            b'}' if brace_depth > 0 => brace_depth -= 1,
            value if value == close && brace_depth == 0 => {
                outer_depth -= 1;
                if outer_depth == 0 {
                    return (cursor + 1, true);
                }
            }
            value if value == open && brace_depth == 0 => outer_depth += 1,
            _ => {}
        }
        cursor += 1;
    }
    (text.len(), false)
}

fn scan_braced_value(text: &str, start: usize, end: usize) -> (usize, bool) {
    let mut cursor = start + 1;
    let mut depth = 1usize;
    while cursor < end {
        match text.as_bytes()[cursor] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (cursor + 1, true);
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    (end, false)
}

fn scan_quoted_value(text: &str, start: usize, end: usize) -> (usize, bool) {
    let mut cursor = start + 1;
    let mut brace_depth = 0usize;
    let mut escaped = false;
    while cursor < end {
        let byte = text.as_bytes()[cursor];
        if byte == b'"' && brace_depth == 0 && !escaped {
            return (cursor + 1, true);
        }
        if byte == b'{' && !escaped {
            brace_depth += 1;
        } else if byte == b'}' && brace_depth > 0 && !escaped {
            brace_depth -= 1;
        }
        escaped = byte == b'\\' && !escaped;
        if byte != b'\\' {
            escaped = false;
        }
        cursor += 1;
    }
    (end, false)
}

fn find_top_level_comma(text: &str, start: usize, end: usize) -> Option<usize> {
    let mut cursor = start;
    let mut braces = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    while cursor < end {
        let byte = text.as_bytes()[cursor];
        if quoted {
            if byte == b'"' && braces == 0 && !escaped {
                quoted = false;
            } else if byte == b'{' && !escaped {
                braces += 1;
            } else if byte == b'}' && braces > 0 && !escaped {
                braces -= 1;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'{' => braces += 1,
                b'}' if braces > 0 => braces -= 1,
                b',' if braces == 0 => return Some(cursor),
                _ => {}
            }
        }
        escaped = byte == b'\\' && !escaped;
        if byte != b'\\' {
            escaped = false;
        }
        cursor += 1;
    }
    None
}

struct ExpansionResult {
    text: String,
    status: ValueExpansionStatus,
    provenance: Vec<ValueProvenance>,
    used_identifier: bool,
}

struct StringEvaluator<'a> {
    strings: &'a [BibliographyString],
    definitions: BTreeMap<String, Vec<usize>>,
    options: &'a BibliographyOptions,
    expansions: u32,
    expanded_characters: u64,
}

impl<'a> StringEvaluator<'a> {
    fn new(strings: &'a [BibliographyString], options: &'a BibliographyOptions) -> Self {
        let mut definitions = BTreeMap::<String, Vec<usize>>::new();
        for definition in strings {
            definitions
                .entry(definition.name.to_ascii_lowercase())
                .or_default()
                .push(definition.index);
        }
        Self {
            strings,
            definitions,
            options,
            expansions: 0,
            expanded_characters: 0,
        }
    }

    fn expand_value(&mut self, value: &mut BibliographyValue) {
        if value.expansion_status == ValueExpansionStatus::Malformed {
            return;
        }
        let mut stack = Vec::new();
        let mut result = self.expand_parts(&value.parts, 0, &mut stack);
        if result.status.is_resolved() {
            let characters = result.text.chars().count() as u64;
            if self.expanded_characters.saturating_add(characters)
                > self.options.max_expanded_characters
            {
                result.status = ValueExpansionStatus::OutputLimit;
                result.text.clear();
            } else {
                self.expanded_characters += characters;
            }
        }
        value.expansion_status = if result.status.is_resolved() && result.used_identifier {
            ValueExpansionStatus::Expanded
        } else {
            result.status
        };
        value.resolved = value.expansion_status.is_resolved().then_some(result.text);
        value.provenance = result.provenance;
    }

    fn expand_parts(
        &mut self,
        parts: &[BibliographyValuePart],
        depth: u16,
        stack: &mut Vec<String>,
    ) -> ExpansionResult {
        let mut output = String::new();
        let mut provenance = Vec::new();
        let mut used_identifier = false;
        for part in parts {
            let result = match part.kind {
                BibliographyValuePartKind::Braced
                | BibliographyValuePartKind::Quoted
                | BibliographyValuePartKind::Number => ExpansionResult {
                    text: part.text.clone(),
                    status: ValueExpansionStatus::Literal,
                    provenance: vec![ValueProvenance {
                        kind: ValueProvenanceKind::Literal,
                        name: None,
                        string_index: None,
                        range: Some(part.range.clone()),
                        locator: Some(part.locator.clone()),
                        source: Some(part.source.clone()),
                    }],
                    used_identifier: false,
                },
                BibliographyValuePartKind::StringIdentifier => {
                    used_identifier = true;
                    self.expand_identifier(&part.text, depth, stack)
                }
                BibliographyValuePartKind::Malformed => ExpansionResult {
                    text: String::new(),
                    status: ValueExpansionStatus::Malformed,
                    provenance: Vec::new(),
                    used_identifier: false,
                },
            };
            provenance.extend(result.provenance);
            used_identifier |= result.used_identifier;
            if !result.status.is_resolved() {
                return ExpansionResult {
                    text: String::new(),
                    status: result.status,
                    provenance,
                    used_identifier,
                };
            }
            output.push_str(&result.text);
            if output.chars().count() as u64 > self.options.max_expanded_characters {
                return ExpansionResult {
                    text: String::new(),
                    status: ValueExpansionStatus::OutputLimit,
                    provenance,
                    used_identifier,
                };
            }
        }
        ExpansionResult {
            text: output,
            status: ValueExpansionStatus::Literal,
            provenance,
            used_identifier,
        }
    }

    fn expand_identifier(
        &mut self,
        identifier: &str,
        depth: u16,
        stack: &mut Vec<String>,
    ) -> ExpansionResult {
        let normalized = identifier.to_ascii_lowercase();
        if let Some(value) = builtin_string(&normalized) {
            return ExpansionResult {
                text: value.to_string(),
                status: ValueExpansionStatus::Expanded,
                provenance: vec![ValueProvenance {
                    kind: ValueProvenanceKind::BuiltInString,
                    name: Some(identifier.to_string()),
                    string_index: None,
                    range: None,
                    locator: None,
                    source: None,
                }],
                used_identifier: true,
            };
        }
        let Some(indices) = self.definitions.get(&normalized) else {
            return failed_expansion(ValueExpansionStatus::UnresolvedIdentifier);
        };
        if indices.len() != 1 {
            return failed_expansion(ValueExpansionStatus::AmbiguousIdentifier);
        }
        if stack.contains(&normalized) {
            return failed_expansion(ValueExpansionStatus::Cycle);
        }
        if depth >= self.options.max_string_depth {
            return failed_expansion(ValueExpansionStatus::DepthExceeded);
        }
        if self.expansions >= self.options.max_string_expansions {
            return failed_expansion(ValueExpansionStatus::ExpansionLimit);
        }
        self.expansions += 1;
        let definition = &self.strings[indices[0]];
        stack.push(normalized);
        let mut result = self.expand_parts(&definition.value.parts, depth + 1, stack);
        stack.pop();
        result.provenance.insert(
            0,
            ValueProvenance {
                kind: ValueProvenanceKind::StringDefinition,
                name: Some(definition.name.clone()),
                string_index: Some(definition.index),
                range: Some(definition.range.clone()),
                locator: Some(definition.locator.clone()),
                source: Some(definition.source.clone()),
            },
        );
        result.used_identifier = true;
        result
    }
}

fn failed_expansion(status: ValueExpansionStatus) -> ExpansionResult {
    ExpansionResult {
        text: String::new(),
        status,
        provenance: Vec::new(),
        used_identifier: true,
    }
}

fn builtin_string(name: &str) -> Option<&'static str> {
    Some(match name {
        "jan" => "January",
        "feb" => "February",
        "mar" => "March",
        "apr" => "April",
        "may" => "May",
        "jun" => "June",
        "jul" => "July",
        "aug" => "August",
        "sep" => "September",
        "oct" => "October",
        "nov" => "November",
        "dec" => "December",
        _ => return None,
    })
}

fn expand_all_values(
    entries: &mut [BibliographyEntry],
    strings: &mut [BibliographyString],
    preambles: &mut [BibliographyValue],
    options: &BibliographyOptions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let originals = strings.to_vec();
    let mut names = BTreeMap::<String, Vec<&BibliographyString>>::new();
    for definition in &originals {
        names
            .entry(definition.name.to_ascii_lowercase())
            .or_default()
            .push(definition);
    }
    for (name, definitions) in names.iter().filter(|(_, values)| values.len() > 1) {
        let range = definitions[1].name_range.clone();
        diagnostics.push(
            Diagnostic::error(
                "grist.bibliography",
                "bibliography.string.duplicate",
                format!("duplicate @string definition `{name}` makes expansion ambiguous"),
            )
            .with_parser("grist.bibliography")
            .with_range(range)
            .partial(),
        );
    }
    let mut evaluator = StringEvaluator::new(&originals, options);
    for definition in strings {
        evaluator.expand_value(&mut definition.value);
        report_expansion_failure(&definition.value, diagnostics);
    }
    for preamble in preambles {
        evaluator.expand_value(preamble);
        report_expansion_failure(preamble, diagnostics);
    }
    for entry in entries {
        for field in &mut entry.fields {
            evaluator.expand_value(&mut field.value);
            report_expansion_failure(&field.value, diagnostics);
        }
    }
}

fn report_expansion_failure(value: &BibliographyValue, diagnostics: &mut Vec<Diagnostic>) {
    if value.expansion_status.is_resolved()
        || value.expansion_status == ValueExpansionStatus::Malformed
    {
        return;
    }
    diagnostics.push(
        Diagnostic::error(
            "grist.bibliography",
            "bibliography.value.expansion_failed",
            format!(
                "bibliography value expansion ended with {:?}",
                value.expansion_status
            ),
        )
        .with_parser("grist.bibliography")
        .with_range(value.range.clone())
        .partial(),
    );
}

fn find_duplicate_keys(
    entries: &[BibliographyEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<DuplicateBibliographyKey> {
    let mut by_key = BTreeMap::<String, Vec<usize>>::new();
    for entry in entries {
        by_key
            .entry(entry.key.clone())
            .or_default()
            .push(entry.index);
    }
    by_key
        .into_iter()
        .filter_map(|(key, entry_indices)| {
            if entry_indices.len() < 2 {
                return None;
            }
            let range = entries[entry_indices[1]].key_range.clone();
            diagnostics.push(
                Diagnostic::error(
                    "grist.bibliography",
                    "bibliography.entry.duplicate_key",
                    format!(
                        "citation key `{key}` occurs {} times and resolves ambiguously",
                        entry_indices.len()
                    ),
                )
                .with_parser("grist.bibliography")
                .with_range(range)
                .partial(),
            );
            Some(DuplicateBibliographyKey { key, entry_indices })
        })
        .collect()
}

struct EffectiveResult {
    fields: BTreeMap<String, ResolvedBibliographyField>,
    failure: Option<CrossrefStatus>,
}

fn resolve_all_crossrefs(
    entries: &mut [BibliographyEntry],
    options: &BibliographyOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<CrossrefResolution> {
    let snapshot = entries.to_vec();
    let mut key_index = BTreeMap::<String, Vec<usize>>::new();
    for entry in &snapshot {
        key_index
            .entry(entry.key.clone())
            .or_default()
            .push(entry.index);
    }
    let mut resolutions = Vec::new();
    for (entry_index, entry) in entries.iter_mut().enumerate() {
        let mut visiting = BTreeSet::new();
        let result = effective_fields_for(
            entry_index,
            &snapshot,
            &key_index,
            options,
            0,
            &mut visiting,
            true,
            &mut resolutions,
            diagnostics,
        );
        entry.effective_fields = result.fields;
    }
    resolutions
}

#[allow(clippy::too_many_arguments)]
fn effective_fields_for(
    entry_index: usize,
    entries: &[BibliographyEntry],
    key_index: &BTreeMap<String, Vec<usize>>,
    options: &BibliographyOptions,
    depth: u16,
    visiting: &mut BTreeSet<usize>,
    record: bool,
    resolutions: &mut Vec<CrossrefResolution>,
    diagnostics: &mut Vec<Diagnostic>,
) -> EffectiveResult {
    let entry = &entries[entry_index];
    let mut fields = direct_fields(entry);
    if !visiting.insert(entry_index) {
        return EffectiveResult {
            fields,
            failure: Some(CrossrefStatus::Cycle),
        };
    }
    let mut failure = None;
    for field in entry
        .fields
        .iter()
        .filter(|field| matches!(field.name.as_str(), "crossref" | "xdata"))
    {
        let Some(resolved) = field.value.resolved.as_deref() else {
            let status = CrossrefStatus::ValueUnresolved;
            if record {
                push_crossref(
                    entry,
                    field,
                    field.value.raw.clone(),
                    status,
                    Vec::new(),
                    Vec::new(),
                    resolutions,
                    diagnostics,
                );
            }
            failure.get_or_insert(status);
            continue;
        };
        let targets = if field.name == "xdata" {
            resolved
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        } else {
            vec![resolved.trim()]
        };
        for target_key in targets {
            let candidates = key_index.get(target_key).cloned().unwrap_or_default();
            let mut status = match candidates.len() {
                0 => CrossrefStatus::Unresolved,
                1 => CrossrefStatus::Resolved,
                _ => CrossrefStatus::Ambiguous,
            };
            let mut inherited_fields = Vec::new();
            if status == CrossrefStatus::Resolved {
                let target = candidates[0];
                if visiting.contains(&target) {
                    status = CrossrefStatus::Cycle;
                } else if depth >= options.max_crossref_depth {
                    status = CrossrefStatus::DepthExceeded;
                } else {
                    let parent = effective_fields_for(
                        target,
                        entries,
                        key_index,
                        options,
                        depth + 1,
                        visiting,
                        false,
                        resolutions,
                        diagnostics,
                    );
                    if let Some(parent_failure) = parent.failure {
                        status = parent_failure;
                    } else {
                        for (name, mut inherited) in parent.fields {
                            if matches!(name.as_str(), "crossref" | "xdata")
                                || fields.contains_key(&name)
                            {
                                continue;
                            }
                            inherited.inherited = true;
                            inherited_fields.push(name.clone());
                            fields.insert(name, inherited);
                        }
                    }
                }
            }
            inherited_fields.sort();
            if status != CrossrefStatus::Resolved {
                failure.get_or_insert(status);
            }
            if record {
                push_crossref(
                    entry,
                    field,
                    target_key.to_string(),
                    status,
                    candidates,
                    inherited_fields,
                    resolutions,
                    diagnostics,
                );
            }
        }
    }
    visiting.remove(&entry_index);
    EffectiveResult { fields, failure }
}

fn direct_fields(entry: &BibliographyEntry) -> BTreeMap<String, ResolvedBibliographyField> {
    let mut fields = BTreeMap::new();
    for field in &entry.fields {
        fields.insert(
            field.name.clone(),
            ResolvedBibliographyField {
                name: field.name.clone(),
                source_entry_index: entry.index,
                source_entry_key: entry.key.clone(),
                inherited: false,
                value: field.value.clone(),
            },
        );
    }
    fields
}

#[allow(clippy::too_many_arguments)]
fn push_crossref(
    entry: &BibliographyEntry,
    field: &BibliographyField,
    target_key: String,
    status: CrossrefStatus,
    target_entry_indices: Vec<usize>,
    inherited_fields: Vec<String>,
    resolutions: &mut Vec<CrossrefResolution>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    resolutions.push(CrossrefResolution {
        source_entry_index: entry.index,
        source_key: entry.key.clone(),
        field: field.name.clone(),
        target_key: target_key.clone(),
        status,
        target_entry_indices,
        inherited_fields,
        locator: field.value.locator.clone(),
    });
    if status != CrossrefStatus::Resolved {
        diagnostics.push(
            Diagnostic::error(
                "grist.bibliography",
                "bibliography.crossref.unresolved",
                format!(
                    "{} `{target_key}` from `{}` resolved as {:?}",
                    field.name, entry.key, status
                ),
            )
            .with_parser("grist.bibliography")
            .with_range(field.value.range.clone())
            .partial(),
        );
    }
}

fn detect_dialect(
    requested: BibliographyDialect,
    entries: &[BibliographyEntry],
) -> BibliographyDialect {
    if requested != BibliographyDialect::Auto {
        return requested;
    }
    let biblatex = entries.iter().any(|entry| {
        matches!(
            entry.entry_type.as_str(),
            "online" | "software" | "dataset" | "xdata" | "mvbook" | "mvcollection"
        ) || entry.fields.iter().any(|field| {
            matches!(
                field.name.as_str(),
                "date"
                    | "urldate"
                    | "journaltitle"
                    | "location"
                    | "langid"
                    | "eprinttype"
                    | "xdata"
                    | "related"
                    | "ids"
            )
        })
    });
    if biblatex {
        BibliographyDialect::Biblatex
    } else {
        BibliographyDialect::Bibtex
    }
}
