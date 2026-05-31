use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, LineIndex, ParserInfo, SchemaVersion, SourceInfo,
    SourceRange,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvDocument {
    pub schema_version: String,
    pub delimiter: CsvDelimiter,
    pub has_headers: bool,
    pub headers: Vec<CsvHeader>,
    pub rows: Vec<CsvRow>,
    pub record_count: usize,
    pub column_count: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CsvDelimiter {
    Comma,
    Tab,
    Semicolon,
    Pipe,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvHeader {
    pub column_index: usize,
    pub name: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvRow {
    pub index: usize,
    pub source_line: Option<usize>,
    pub range: Option<SourceRange>,
    pub width: usize,
    pub cells: Vec<CsvCell>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CsvCell {
    pub column_index: usize,
    pub header: Option<String>,
    pub raw: String,
    pub value: Value,
    pub empty: bool,
}

#[derive(Debug, Clone)]
pub struct CsvOptions {
    pub delimiter: CsvDelimiter,
    pub has_headers: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: CsvDelimiter::Comma,
            has_headers: true,
        }
    }
}

pub type CsvEnvelope = Envelope<CsvDocument>;

pub fn parse_csv(text: &str, source: SourceInfo, options: &CsvOptions) -> CsvEnvelope {
    let line_index = LineIndex::new(text);
    let mut diagnostics = Vec::new();
    let mut reader = ::csv::ReaderBuilder::new()
        .delimiter(options.delimiter.byte())
        .has_headers(options.has_headers)
        .flexible(true)
        .from_reader(text.as_bytes());

    let headers = if options.has_headers {
        match reader.headers() {
            Ok(record) => headers_from_record(record),
            Err(err) => {
                diagnostics.push(Diagnostic::error(
                    "grist.csv",
                    "csv.headers_parse",
                    format!("CSV header parse failed: {err}"),
                ));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    let mut rows = Vec::new();
    for record_result in reader.records() {
        match record_result {
            Ok(record) => {
                let range = record
                    .position()
                    .and_then(|position| source_range_for_position(text, position, &line_index));
                let source_line = record
                    .position()
                    .map(|position| usize::try_from(position.line()).unwrap_or(usize::MAX));
                rows.push(row_from_record(
                    rows.len(),
                    &record,
                    &headers,
                    source_line,
                    range,
                ));
            }
            Err(err) => {
                let diagnostic = Diagnostic::error(
                    "grist.csv",
                    "csv.record_parse",
                    format!("CSV record parse failed: {err}"),
                )
                .partial();
                diagnostics.push(diagnostic);
            }
        }
    }

    let column_count = rows
        .iter()
        .map(|row| row.width)
        .chain(std::iter::once(headers.len()))
        .max()
        .unwrap_or(0);
    let record_count = rows.len();
    let payload = CsvDocument {
        schema_version: SchemaVersion::CSV_V1.to_string(),
        delimiter: options.delimiter.clone(),
        has_headers: options.has_headers,
        headers,
        rows,
        record_count,
        column_count,
    };

    Envelope::new(
        ArtifactKind::Csv,
        source,
        ParserInfo::new("grist.csv"),
        SchemaVersion::CSV_V1,
        payload,
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(diagnostics)
}

fn headers_from_record(record: &::csv::StringRecord) -> Vec<CsvHeader> {
    record
        .iter()
        .enumerate()
        .map(|(column_index, name)| CsvHeader {
            column_index,
            name: name.to_string(),
        })
        .collect()
}

fn row_from_record(
    row_index: usize,
    record: &::csv::StringRecord,
    headers: &[CsvHeader],
    source_line: Option<usize>,
    range: Option<SourceRange>,
) -> CsvRow {
    let cells = record
        .iter()
        .enumerate()
        .map(|(column_index, raw)| CsvCell {
            column_index,
            header: headers
                .get(column_index)
                .map(|header| header.name.clone())
                .filter(|header| !header.is_empty()),
            raw: raw.to_string(),
            value: infer_cell_value(raw),
            empty: raw.is_empty(),
        })
        .collect::<Vec<_>>();

    CsvRow {
        index: row_index,
        source_line,
        range,
        width: record.len(),
        cells,
    }
}

fn infer_cell_value(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if should_parse_integer(trimmed) {
        if let Ok(number) = trimmed.parse::<i64>() {
            return Value::Number(number.into());
        }
    }
    if should_parse_float(trimmed) {
        if let Ok(number) = trimmed.parse::<f64>() {
            if let Some(value) = serde_json::Number::from_f64(number) {
                return Value::Number(value);
            }
        }
    }
    Value::String(raw.to_string())
}

fn should_parse_integer(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty()
        && digits.chars().all(|character| character.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
}

fn should_parse_float(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    digits.contains('.')
        && digits.chars().filter(|character| *character == '.').count() == 1
        && digits
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
}

fn source_range_for_position(
    text: &str,
    position: &::csv::Position,
    line_index: &LineIndex,
) -> Option<SourceRange> {
    let byte_start = usize::try_from(position.byte()).ok()?;
    if byte_start > text.len() {
        return None;
    }
    let byte_end = text[byte_start..]
        .find('\n')
        .map(|relative_end| byte_start + relative_end)
        .unwrap_or(text.len());
    Some(SourceRange::new(byte_start, byte_end, line_index))
}

impl CsvDelimiter {
    pub fn byte(&self) -> u8 {
        match self {
            Self::Comma => b',',
            Self::Tab => b'\t',
            Self::Semicolon => b';',
            Self::Pipe => b'|',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_csv_headers_rows_and_values() {
        let report = parse_csv(
            "name,score,ok\nalpha,1,true\nbeta,,false\n",
            SourceInfo::stdin("data.csv"),
            &CsvOptions::default(),
        );
        assert_eq!(report.kind, ArtifactKind::Csv);
        assert_eq!(report.payload.headers[0].name, "name");
        assert_eq!(report.payload.record_count, 2);
        assert_eq!(report.payload.rows[0].cells[1].value, serde_json::json!(1));
        assert_eq!(
            report.payload.rows[0].cells[2].value,
            serde_json::json!(true)
        );
        assert_eq!(report.payload.rows[1].cells[1].value, Value::Null);
    }

    #[test]
    fn parses_csv_without_headers() {
        let report = parse_csv(
            "alpha,1\nbeta,2\n",
            SourceInfo::stdin("data.csv"),
            &CsvOptions {
                has_headers: false,
                ..Default::default()
            },
        );
        assert!(report.payload.headers.is_empty());
        assert_eq!(report.payload.record_count, 2);
    }
}
