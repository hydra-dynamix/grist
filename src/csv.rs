//! Dialect-aware, provenance-preserving CSV and TSV parsing.

use crate::core::{
    ArtifactKind, BatchResult, BudgetProfile, BudgetSelection, ContentIdentity, Diagnostic,
    Envelope, FormatIdentity, Hashes, IndexBase, IndexRange, LineIndex, LocationComponent,
    OperationControl, OperationKind, OperationStatus, ParserInfo, RequestId, SchemaVersion,
    SourceInfo, SourceLocator, SourceRange, StreamEvent, StreamItem, StreamTerminal,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

const PARSER: &str = "grist.csv";
const COLLECTION: &str = "records";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvDocument {
    pub schema_version: String,
    pub dialect: CsvDialect,
    pub has_headers: bool,
    pub headers: Vec<CsvHeader>,
    pub header_record: Option<CsvRow>,
    pub rows: Vec<CsvRow>,
    pub record_count: usize,
    pub source_record_count: usize,
    pub column_count: usize,
    pub newline_fidelity: CsvNewlineFidelity,
    pub complete: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvDialect {
    pub source: CsvDialectSource,
    pub delimiter: CsvDelimiter,
    pub delimiter_text: String,
    #[cfg_attr(feature = "schemas", schemars(with = "Option<String>"))]
    pub quote: Option<char>,
    #[cfg_attr(feature = "schemas", schemars(with = "Option<String>"))]
    pub escape: Option<char>,
    pub double_quote: bool,
    pub candidates: Vec<CsvDialectCandidate>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CsvDialectSource {
    Detected,
    Declared,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CsvDelimiter {
    Auto,
    Comma,
    Tab,
    Semicolon,
    Pipe,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvDialectCandidate {
    pub delimiter: CsvDelimiter,
    pub sampled_records: usize,
    pub modal_width: usize,
    pub consistent_records: usize,
    pub malformed_records: usize,
    pub score: f64,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvHeader {
    pub column_index: usize,
    pub name: String,
    pub raw: String,
    pub quoted: bool,
    pub range: SourceRange,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvRow {
    pub index: usize,
    /// One-based logical record index, including the header.
    pub source_record_index: usize,
    pub role: CsvRecordRole,
    pub source_line: usize,
    pub range: SourceRange,
    pub full_range: SourceRange,
    pub locator: SourceLocator,
    /// Exact logical-record text, excluding its terminator.
    pub raw: String,
    pub terminator: CsvRecordTerminator,
    pub width: usize,
    pub cells: Vec<CsvCell>,
    pub malformed: bool,
    pub issues: Vec<CsvParseIssue>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CsvRecordRole {
    Header,
    Data,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvCell {
    pub column_index: usize,
    pub header: Option<String>,
    /// Exact source lexeme, including quote and escape spelling.
    pub raw: String,
    /// Field text after dialect-level unquoting only.
    pub text: String,
    /// Compatibility-selected scalar; `typed_candidates` is authoritative.
    pub value: Value,
    pub typed_candidates: Vec<CsvScalarCandidate>,
    pub empty: bool,
    pub quoted: bool,
    pub quote_closed: bool,
    pub range: SourceRange,
    pub value_range: SourceRange,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvScalarCandidate {
    pub kind: CsvScalarKind,
    pub value: Value,
    pub confidence: f64,
    pub evidence: String,
    pub normalized: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CsvScalarKind {
    Null,
    Boolean,
    Integer,
    Decimal,
    Date,
    DateTime,
    Text,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CsvParseIssue {
    pub code: String,
    pub message: String,
    pub range: SourceRange,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CsvRecordTerminator {
    Lf,
    CrLf,
    Cr,
    EndOfInput,
}

impl CsvRecordTerminator {
    pub const fn byte_len(self) -> usize {
        match self {
            Self::Lf | Self::Cr => 1,
            Self::CrLf => 2,
            Self::EndOfInput => 0,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CsvNewlineFidelity {
    pub lf_count: usize,
    pub crlf_count: usize,
    pub cr_count: usize,
    pub final_record_terminated: bool,
    pub mixed: bool,
}

impl CsvNewlineFidelity {
    fn observe(&mut self, terminator: CsvRecordTerminator) {
        match terminator {
            CsvRecordTerminator::Lf => self.lf_count += 1,
            CsvRecordTerminator::CrLf => self.crlf_count += 1,
            CsvRecordTerminator::Cr => self.cr_count += 1,
            CsvRecordTerminator::EndOfInput => {}
        }
        self.final_record_terminated = terminator != CsvRecordTerminator::EndOfInput;
        self.mixed = [self.lf_count, self.crlf_count, self.cr_count]
            .into_iter()
            .filter(|count| *count != 0)
            .count()
            > 1;
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CsvOptions {
    pub delimiter: CsvDelimiter,
    pub has_headers: bool,
    #[cfg_attr(feature = "schemas", schemars(with = "Option<String>"))]
    pub quote: Option<char>,
    #[cfg_attr(feature = "schemas", schemars(with = "Option<String>"))]
    pub escape: Option<char>,
    pub double_quote: bool,
    pub dialect_sample_records: usize,
}

impl crate::core::FormatOptions for CsvOptions {
    const FORMAT: &'static str = "csv";
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: CsvDelimiter::Auto,
            has_headers: true,
            quote: Some(char::from(34)),
            escape: None,
            double_quote: true,
            dialect_sample_records: 32,
        }
    }
}

pub type CsvEnvelope = Envelope<CsvDocument>;
pub fn parse_csv(text: &str, source: SourceInfo, options: &CsvOptions) -> CsvEnvelope {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .unwrap();
    parse_csv_with_control(text, source, options, &control)
}

pub fn parse_csv_with_control(
    text: &str,
    source: SourceInfo,
    options: &CsvOptions,
    control: &OperationControl,
) -> CsvEnvelope {
    let digest = crate::core::options_digest(options).unwrap();
    let stream = stream_csv(
        text,
        options,
        RequestId::new("csv/batch").unwrap(),
        control.clone(),
    );
    let dialect = stream.dialect.clone();
    let batch = BatchResult::collect(stream).unwrap();
    let mut newline_fidelity = CsvNewlineFidelity::default();
    let mut header_record = None;
    let mut rows = Vec::new();
    for item in batch.items {
        match item.payload {
            CsvStreamRecord::Header(record) => {
                newline_fidelity.observe(record.terminator);
                header_record = Some(record);
            }
            CsvStreamRecord::Data(record) => {
                newline_fidelity.observe(record.terminator);
                rows.push(record);
            }
        }
    }
    let headers = header_record
        .as_ref()
        .map(|record| {
            record
                .cells
                .iter()
                .map(|cell| CsvHeader {
                    column_index: cell.column_index,
                    name: cell.text.clone(),
                    raw: cell.raw.clone(),
                    quoted: cell.quoted,
                    range: cell.range.clone(),
                    locator: cell.locator.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let source_record_count = rows.len() + usize::from(header_record.is_some());
    let column_count = rows
        .iter()
        .map(|row| row.width)
        .chain(header_record.iter().map(|row| row.width))
        .max()
        .unwrap_or(0);
    let record_count = rows.len();
    let payload = CsvDocument {
        schema_version: SchemaVersion::CSV_V1.to_string(),
        dialect,
        has_headers: options.has_headers,
        headers,
        header_record,
        rows,
        record_count,
        source_record_count,
        column_count,
        newline_fidelity,
        complete: batch.status == OperationStatus::Complete,
    };
    let parser = ParserInfo::new(PARSER).with_feature("csv");
    let envelope = if batch.status == OperationStatus::Complete {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Csv,
            source,
            parser,
            digest,
            SchemaVersion::CSV_V1,
            payload,
        )
    } else {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Csv,
            source,
            parser,
            digest,
            SchemaVersion::CSV_V1,
            Some(payload),
        )
    };
    envelope
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(batch.diagnostics)
}

pub fn stream_csv<'a>(
    text: &'a str,
    options: &CsvOptions,
    request_id: RequestId,
    control: OperationControl,
) -> CsvStream<'a> {
    let dialect = resolve_dialect(text, options);
    CsvStream {
        scanner: Scanner::new(text, &dialect),
        text,
        request_id,
        control,
        dialect,
        has_headers: options.has_headers,
        source_record_index: 0,
        data_index: 0,
        expected_width: None,
        headers: Vec::new(),
        sequence: 0,
        diagnostics: Vec::new(),
        finished: false,
    }
}

pub struct CsvStream<'a> {
    scanner: Scanner<'a>,
    text: &'a str,
    request_id: RequestId,
    control: OperationControl,
    pub dialect: CsvDialect,
    has_headers: bool,
    source_record_index: usize,
    data_index: usize,
    expected_width: Option<usize>,
    headers: Vec<String>,
    sequence: u64,
    diagnostics: Vec<Diagnostic>,
    finished: bool,
}

impl Iterator for CsvStream<'_> {
    type Item = CsvStreamEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if let Err(error) = self.control.checkpoint() {
            self.finished = true;
            return Some(StreamEvent::terminal(StreamTerminal::from_control(
                PARSER,
                self.sequence,
                &self.control,
                error,
            )));
        }
        let Some(raw) = self.scanner.next() else {
            self.finished = true;
            let status = if self.diagnostics.iter().any(|item| item.partial) {
                OperationStatus::Partial
            } else {
                OperationStatus::Complete
            };
            return Some(StreamEvent::terminal(StreamTerminal {
                status,
                emitted_items: self.sequence,
                diagnostics: std::mem::take(&mut self.diagnostics),
                budget_usage: self.control.usage(),
            }));
        };
        let count = u64::try_from(raw.cells.len()).unwrap_or(u64::MAX);
        let charged = self
            .control
            .budget()
            .consume_records(1)
            .and_then(|_| self.control.budget().consume_cells(count));
        if let Err(error) = charged {
            self.finished = true;
            return Some(StreamEvent::terminal(StreamTerminal::from_control(
                PARSER,
                self.sequence,
                &self.control,
                error.into(),
            )));
        }
        self.source_record_index += 1;
        let role = if self.has_headers && self.source_record_index == 1 {
            CsvRecordRole::Header
        } else {
            CsvRecordRole::Data
        };
        let mut row = materialize_row(
            self.text,
            raw,
            role,
            self.data_index,
            self.source_record_index,
            &self.headers,
        );
        if role == CsvRecordRole::Header {
            self.headers = row.cells.iter().map(|cell| cell.text.clone()).collect();
            self.expected_width = Some(row.width);
        } else {
            if self.expected_width.is_none() {
                self.expected_width = Some(row.width);
            }
            if self.expected_width == Some(row.width) {
            } else {
                row.malformed = true;
            }
            self.data_index += 1;
        }
        // Retain malformed rows and report them at stream termination.
        let malformed = row.malformed;
        if malformed {
            self.diagnostics
                .push(Diagnostic::malformed(PARSER, PARSER).partial());
        }
        let identity = ContentIdentity::for_raw_bytes(row.raw.as_bytes());
        let payload = match role {
            CsvRecordRole::Header => CsvStreamRecord::Header(row),
            CsvRecordRole::Data => CsvStreamRecord::Data(row),
        };
        let event = StreamEvent::item(StreamItem::new(
            self.sequence,
            self.request_id.clone(),
            identity,
            payload,
        ));
        self.sequence += 1;
        Some(event)
    }
}

fn materialize_row(
    text: &str,
    record: ScannedRecord,
    role: CsvRecordRole,
    data_index: usize,
    source_record_index: usize,
    headers: &[String],
) -> CsvRow {
    let line_index = LineIndex::new(text);
    let range = SourceRange::new(record.start, record.end, &line_index);
    let full_range = SourceRange::new(
        record.start,
        record.end + record.terminator.byte_len(),
        &line_index,
    );
    let locator = record_locator(source_record_index, None, &range);
    let cells = record
        .cells
        .into_iter()
        .enumerate()
        .map(|(column_index, cell)| {
            let cell_range = SourceRange::new(cell.start, cell.end, &line_index);
            let value_range = SourceRange::new(cell.value_start, cell.value_end, &line_index);
            let raw = text[cell.start..cell.end].to_string();
            let decoded = decode_cell(&raw, cell.quoted, cell.quote_closed, &record.dialect);
            let candidates = infer_scalar_candidates(&decoded);
            let value = candidates
                .iter()
                .find(|candidate| candidate.kind != CsvScalarKind::Text)
                .or_else(|| candidates.last())
                .map(|candidate| candidate.value.clone())
                .unwrap_or(Value::String(decoded.clone()));
            CsvCell {
                column_index,
                header: headers
                    .get(column_index)
                    .cloned()
                    .filter(|header| !header.is_empty()),
                raw,
                text: decoded.clone(),
                value,
                typed_candidates: candidates,
                empty: decoded.is_empty(),
                quoted: cell.quoted,
                quote_closed: cell.quote_closed,
                range: cell_range.clone(),
                value_range,
                locator: record_locator(source_record_index, Some(column_index + 1), &cell_range),
            }
        })
        .collect::<Vec<_>>();
    CsvRow {
        index: data_index,
        source_record_index,
        role,
        source_line: range.start_line,
        range,
        full_range,
        locator,
        raw: text[record.start..record.end].to_string(),
        terminator: record.terminator,
        width: cells.len(),
        cells,
        malformed: !record.issues.is_empty(),
        issues: record.issues,
    }
}

fn record_locator(record: usize, field: Option<usize>, range: &SourceRange) -> SourceLocator {
    let records = IndexRange::new(record as u64, record as u64 + 1, IndexBase::One).unwrap();
    let field = field.map(|column| column.to_string());
    SourceLocator::exact(LocationComponent::RecordRange {
        collection: COLLECTION.into(),
        records,
        field,
    })
    .and_then(|locator| locator.nested(range.clone()))
    .unwrap()
}

fn infer_scalar_candidates(text: &str) -> Vec<CsvScalarCandidate> {
    let trimmed = text.trim();
    let mut candidates = Vec::new();
    if trimmed.is_empty() {
        candidates.push(scalar(CsvScalarKind::Null, Value::Null, 0.8, PARSER, false));
    } else if trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("false") {
        candidates.push(scalar(
            CsvScalarKind::Boolean,
            Value::Bool(trimmed.eq_ignore_ascii_case("true")),
            1.0,
            PARSER,
            false,
        ));
    } else if should_parse_integer(trimmed) {
        if let Ok(number) = trimmed.parse::<i64>() {
            candidates.push(scalar(
                CsvScalarKind::Integer,
                Value::Number(number.into()),
                1.0,
                PARSER,
                false,
            ));
        }
    } else if trimmed.bytes().any(|byte| matches!(byte, 46 | 69 | 101)) {
        if let Ok(number) = trimmed.parse::<f64>() {
            if let Some(number) = serde_json::Number::from_f64(number) {
                candidates.push(scalar(
                    CsvScalarKind::Decimal,
                    Value::Number(number),
                    0.95,
                    PARSER,
                    false,
                ));
            }
        }
    }
    candidates.push(scalar(
        CsvScalarKind::Text,
        Value::String(text.into()),
        1.0,
        PARSER,
        false,
    ));
    let _ = trimmed;
    candidates
}

fn scalar(
    kind: CsvScalarKind,
    value: Value,
    confidence: f64,
    evidence: &str,
    normalized: bool,
) -> CsvScalarCandidate {
    CsvScalarCandidate {
        kind,
        value,
        confidence,
        evidence: evidence.into(),
        normalized,
    }
}

fn should_parse_integer(value: &str) -> bool {
    let digits = value.strip_prefix(char::from(45)).unwrap_or(value);
    digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with(char::from(48)))
}

fn decode_cell(raw: &str, quoted: bool, closed: bool, dialect: &CsvDialect) -> String {
    if !quoted {
        return raw.to_string();
    }
    let quote = dialect.quote.unwrap_or(char::from(34));
    let width = quote.len_utf8();
    let start = width.min(raw.len());
    let end = if closed && raw.ends_with(quote) {
        raw.len().saturating_sub(width)
    } else {
        raw.len()
    };
    if end < start {
        return raw.to_string();
    }
    let mut value = raw[start..end].to_string();
    if dialect.double_quote {
        let doubled = quote.to_string().repeat(2);
        value = value.replace(&doubled, &quote.to_string());
    }
    if let Some(escape) = dialect.escape {
        value = value.replace(
            &[escape, quote].iter().collect::<String>(),
            &quote.to_string(),
        );
    }
    value
}

fn resolve_dialect(text: &str, options: &CsvOptions) -> CsvDialect {
    let candidates = detect_dialect(text, options);
    let (source, delimiter) = if options.delimiter == CsvDelimiter::Auto {
        (
            CsvDialectSource::Detected,
            candidates
                .first()
                .map(|candidate| candidate.delimiter)
                .unwrap_or(CsvDelimiter::Comma),
        )
    } else {
        (CsvDialectSource::Declared, options.delimiter)
    };
    CsvDialect {
        source,
        delimiter,
        delimiter_text: delimiter.as_char().to_string(),
        quote: options.quote,
        escape: options.escape,
        double_quote: options.double_quote,
        candidates,
    }
}

impl CsvDelimiter {
    const DETECTABLE: [Self; 4] = [Self::Comma, Self::Tab, Self::Semicolon, Self::Pipe];
    pub fn byte(self) -> u8 {
        self.as_char() as u8
    }
    pub fn as_char(self) -> char {
        match self {
            Self::Auto | Self::Comma => char::from(44),
            Self::Tab => char::from(9),
            Self::Semicolon => char::from(59),
            Self::Pipe => char::from(124),
        }
    }
}

fn detect_dialect(text: &str, options: &CsvOptions) -> Vec<CsvDialectCandidate> {
    let mut candidates = CsvDelimiter::DETECTABLE
        .into_iter()
        .map(|delimiter| {
            let dialect = CsvDialect {
                source: CsvDialectSource::Detected,
                delimiter,
                delimiter_text: delimiter.as_char().to_string(),
                quote: options.quote,
                escape: options.escape,
                double_quote: options.double_quote,
                candidates: Vec::new(),
            };
            let records = Scanner::new(text, &dialect)
                .take(options.dialect_sample_records.max(1))
                .collect::<Vec<_>>();
            let mut widths = BTreeMap::<usize, usize>::new();
            for record in &records {
                *widths.entry(record.cells.len()).or_default() += 1;
            }
            let (modal_width, consistent_records) = widths
                .into_iter()
                .max_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)))
                .unwrap_or((0, 0));
            let malformed_records = records
                .iter()
                .filter(|record| !record.issues.is_empty())
                .count();
            let sampled_records = records.len();
            let score = if modal_width <= 1 || sampled_records == 0 {
                0.0
            } else {
                consistent_records as f64 / sampled_records as f64 * 1000.0 + modal_width as f64
                    - malformed_records as f64 * 100.0
            };
            CsvDialectCandidate {
                delimiter,
                sampled_records,
                modal_width,
                consistent_records,
                malformed_records,
                score,
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then(left.delimiter.cmp(&right.delimiter))
    });
    candidates
}

#[derive(Debug, Clone)]
struct ScannedCell {
    start: usize,
    end: usize,
    value_start: usize,
    value_end: usize,
    quoted: bool,
    quote_closed: bool,
}

#[derive(Debug, Clone)]
struct ScannedRecord {
    dialect: CsvDialect,
    start: usize,
    end: usize,
    terminator: CsvRecordTerminator,
    cells: Vec<ScannedCell>,
    issues: Vec<CsvParseIssue>,
}

struct Scanner<'a> {
    text: &'a str,
    offset: usize,
    dialect: CsvDialect,
    index: LineIndex,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str, dialect: &CsvDialect) -> Self {
        Self {
            text,
            offset: 0,
            dialect: dialect.clone(),
            index: LineIndex::new(text),
        }
    }
    fn issue(&self, code: &str, message: &str, start: usize, end: usize) -> CsvParseIssue {
        CsvParseIssue {
            code: code.into(),
            message: message.into(),
            range: SourceRange::new(start, end, &self.index),
        }
    }
}

impl Iterator for Scanner<'_> {
    type Item = ScannedRecord;
    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.text.len() {
            return None;
        }
        let bytes = self.text.as_bytes();
        let delimiter = self.dialect.delimiter.byte();
        let quote = self
            .dialect
            .quote
            .filter(char::is_ascii)
            .map(|value| value as u8);
        let escape = self
            .dialect
            .escape
            .filter(char::is_ascii)
            .map(|value| value as u8);
        let record_start = self.offset;
        let mut cursor = self.offset;
        let mut cell_start = cursor;
        let mut quoted = false;
        let mut quote_closed = false;
        let mut in_quotes = false;
        let mut after_quote = false;
        let mut cells = Vec::new();
        let mut issues = Vec::new();
        let mut terminator = CsvRecordTerminator::EndOfInput;
        let record_end;
        loop {
            if cursor >= bytes.len() {
                if in_quotes {
                    issues.push(self.issue(PARSER, PARSER, cell_start, cursor));
                }
                cells.push(scanned_cell(cell_start, cursor, quoted, quote_closed));
                record_end = cursor;
                break;
            }
            let byte = bytes[cursor];
            if in_quotes {
                if escape == Some(byte) && cursor + 1 < bytes.len() {
                    cursor += 2;
                    continue;
                }
                if quote == Some(byte) {
                    if self.dialect.double_quote && bytes.get(cursor + 1) == Some(&byte) {
                        cursor += 2;
                        continue;
                    }
                    in_quotes = false;
                    quote_closed = true;
                    after_quote = true;
                    cursor += 1;
                    continue;
                }
                cursor += 1;
                continue;
            }
            if byte == delimiter {
                cells.push(scanned_cell(cell_start, cursor, quoted, quote_closed));
                cursor += 1;
                cell_start = cursor;
                quoted = false;
                quote_closed = false;
                after_quote = false;
                continue;
            }
            if byte == 13 || byte == 10 {
                cells.push(scanned_cell(cell_start, cursor, quoted, quote_closed));
                record_end = cursor;
                terminator = if byte == 13 && bytes.get(cursor + 1) == Some(&10) {
                    CsvRecordTerminator::CrLf
                } else if byte == 13 {
                    CsvRecordTerminator::Cr
                } else {
                    CsvRecordTerminator::Lf
                };
                cursor += terminator.byte_len();
                break;
            }
            if quote == Some(byte) {
                if cursor == cell_start {
                    quoted = true;
                    in_quotes = true;
                    cursor += 1;
                    continue;
                }
                issues.push(self.issue(PARSER, PARSER, cursor, cursor + 1));
            } else if after_quote && byte.is_ascii_whitespace() == false {
                issues.push(self.issue(PARSER, PARSER, cursor, cursor + 1));
                after_quote = false;
            }
            cursor += 1;
        }
        self.offset = cursor;
        Some(ScannedRecord {
            dialect: self.dialect.clone(),
            start: record_start,
            end: record_end,
            terminator,
            cells,
            issues,
        })
    }
}

fn scanned_cell(start: usize, end: usize, quoted: bool, quote_closed: bool) -> ScannedCell {
    let value_start = if quoted {
        start.saturating_add(1).min(end)
    } else {
        start
    };
    let value_end = if quoted && quote_closed && end > value_start {
        end - 1
    } else {
        end
    };
    ScannedCell {
        start,
        end,
        value_start,
        value_end,
        quoted,
        quote_closed,
    }
}
pub type CsvStreamEvent = StreamEvent<CsvStreamRecord>;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "record_kind", content = "record", rename_all = "snake_case")]
pub enum CsvStreamRecord {
    Header(CsvRow),
    Data(CsvRow),
}

impl CsvStreamRecord {
    pub fn row(&self) -> &CsvRow {
        match self {
            Self::Header(row) | Self::Data(row) => row,
        }
    }
}
