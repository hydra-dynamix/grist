//! Strictly inert SQLite file-format inspection.
mod btree;
mod graph;
mod model;
pub use model::*;

use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, Diagnostic, DiagnosticCode, Envelope, Hashes,
    IndexBase, IndexRange, LocationComponent, OperationControl, OperationKind, OperationStatus,
    ParserInfo, SchemaVersion, SourceInfo, SourceLocator,
};
use sha2::{Digest, Sha256};
use std::path::Path;

const PARSER: &str = "grist.sqlite";
pub type SqliteEnvelope = Envelope<SqliteDocument>;

pub fn parse_sqlite(bytes: &[u8], source: SourceInfo, options: &SqliteOptions) -> SqliteEnvelope {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Default::default(),
    )
    .expect("trusted budget is valid");
    parse_sqlite_with_operation_control(bytes, source, options, &control)
}

pub fn parse_sqlite_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &SqliteOptions,
    control: &OperationControl,
) -> SqliteEnvelope {
    let digest = crate::core::options_digest(options).expect("SQLite options serialize");
    if let Err(error) = control.budget().consume_input_bytes(bytes.len() as u64) {
        return terminal(
            bytes,
            source,
            digest,
            error.operation_status(0),
            error.diagnostic(PARSER),
        );
    }
    if let Err(error) = control.checkpoint() {
        return terminal(
            bytes,
            source,
            digest,
            error.operation_status(0),
            error.diagnostic(PARSER),
        );
    }
    if let Some(selection) = &options.record_selection
        && (selection.tables.is_empty()
            || selection.max_tables == 0
            || selection.max_rows_per_table == 0)
    {
        return terminal(
            bytes,
            source,
            digest,
            OperationStatus::Failed,
            diagnostic(
                "sqlite.options.record_selection",
                "record selection requires non-empty table names and positive table/row budgets",
                0,
            ),
        );
    }
    let database = match btree::Database::open(bytes, Some(control)) {
        Ok(database) => database,
        Err(error) => {
            return terminal(
                bytes,
                source,
                digest,
                OperationStatus::Failed,
                decode_diagnostic(error),
            );
        }
    };
    let schema_rows = match database.table_records(1, usize::MAX - 1) {
        Ok((rows, _)) => rows,
        Err(error) => {
            let status = decode_status(&error);
            return terminal(bytes, source, digest, status, decode_diagnostic(error));
        }
    };
    let mut diagnostics = Vec::new();
    let mut schema = Vec::new();
    for (ordinal, row) in schema_rows.into_iter().enumerate() {
        if let Err(error) = control.budget().consume_nodes(1) {
            diagnostics.push(error.diagnostic(PARSER).partial());
            break;
        }
        match schema_object(&row, ordinal + 1) {
            Ok(object) => schema.push(object),
            Err(message) => diagnostics.push(
                Diagnostic::malformed(PARSER, message)
                    .with_locator(byte_locator(row.byte_start, row.byte_end))
                    .partial(),
            ),
        }
    }
    if !options.include_internal_schema {
        schema.retain(|object| !object.name.starts_with("sqlite_"));
    }
    let tables = schema
        .iter()
        .filter_map(table_from_schema)
        .collect::<Vec<_>>();
    let views = schema
        .iter()
        .filter_map(view_from_schema)
        .collect::<Vec<_>>();
    let indexes = schema
        .iter()
        .filter_map(index_from_schema)
        .collect::<Vec<_>>();
    let record_sets = extract_selected(
        &database,
        &tables,
        options.record_selection.as_ref(),
        control,
        &mut diagnostics,
    );
    let complete = diagnostics.iter().all(|item| !item.partial);
    let document = SqliteDocument {
        schema_version: SchemaVersion::SQLITE_V1.into(),
        header: database.header.clone(),
        schema,
        tables,
        views,
        indexes,
        record_sets,
        diagnostics: diagnostics.clone(),
        complete,
    };
    let envelope = if complete {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Sqlite,
            source,
            parser_info(),
            digest,
            SchemaVersion::SQLITE_V1,
            document,
        )
    } else {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Sqlite,
            source,
            parser_info(),
            digest,
            SchemaVersion::SQLITE_V1,
            Some(document),
        )
    };
    envelope
        .with_hashes(Hashes::for_bytes(bytes, None))
        .with_diagnostics(diagnostics)
        .with_canonical_payload_identity()
        .expect("SQLite payload serializes")
}

/// Opens only the main database file for reading. Sidecar journals are
/// inventoried but never replayed because recovery could write to the source.
pub fn inspect_sqlite_path(path: &Path, options: &SqliteOptions) -> SqliteEnvelope {
    let source = SourceInfo::from_path(path);
    let digest = crate::core::options_digest(options).expect("SQLite options serialize");
    let bytes = match std::fs::OpenOptions::new()
        .read(true)
        .write(false)
        .create(false)
        .open(path)
        .and_then(|mut file| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut bytes)?;
            Ok(bytes)
        }) {
        Ok(bytes) => bytes,
        Err(error) => {
            let diagnostic = Diagnostic::error(
                PARSER,
                "sqlite.io.locked_or_unreadable",
                format!("database could not be opened read-only: {error}"),
            );
            return Envelope::without_payload(
                OperationKind::Parse,
                ArtifactKind::Sqlite,
                OperationStatus::Failed,
                source,
                parser_info(),
                digest,
                SchemaVersion::SQLITE_V1,
            )
            .expect("failed envelope is valid")
            .with_diagnostics(vec![diagnostic]);
        }
    };
    let mut envelope = parse_sqlite(&bytes, source, options);
    let mut sidecar_present = false;
    for suffix in ["-wal", "-journal"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        if std::fs::metadata(&sidecar).is_ok_and(|metadata| metadata.len() != 0) {
            sidecar_present = true;
            let diagnostic = Diagnostic::warning(
                PARSER,
                "sqlite.sidecar.not_replayed",
                format!(
                    "{} sidecar is present; it was not replayed in strictly read-only mode",
                    &suffix[1..]
                ),
            )
            .partial();
            envelope.status = OperationStatus::Partial;
            envelope.diagnostics.push(diagnostic.clone());
            if let Some(document) = envelope.payload.as_mut() {
                document.complete = false;
                document.diagnostics.push(diagnostic);
            }
        }
    }
    if sidecar_present {
        envelope
            .with_canonical_payload_identity()
            .expect("SQLite payload serializes")
    } else {
        envelope
    }
}

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("raw-file-format-reader", env!("CARGO_PKG_VERSION"))
        .with_specification_version("SQLite format 3")
        .with_feature("sqlite")
}

fn terminal(
    bytes: &[u8],
    source: SourceInfo,
    digest: String,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> SqliteEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Sqlite,
        status,
        source,
        parser_info(),
        digest,
        SchemaVersion::SQLITE_V1,
    )
    .expect("terminal SQLite envelope is valid")
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(vec![diagnostic])
}

fn decode_diagnostic(error: btree::DecodeError) -> Diagnostic {
    let mut item = if error.code.contains("budget") {
        Diagnostic::budget_exhausted(PARSER, error.message)
    } else {
        Diagnostic::malformed(PARSER, error.message)
    };
    item.code = DiagnosticCode::new(error.code);
    item.with_locator(byte_locator(error.offset, error.offset.saturating_add(1)))
}

fn decode_status(error: &btree::DecodeError) -> OperationStatus {
    if error.code == "sqlite.operation.interrupted"
        && error.message.to_ascii_lowercase().contains("cancel")
    {
        OperationStatus::Cancelled
    } else {
        OperationStatus::Failed
    }
}

fn schema_object(row: &btree::RawRecord, ordinal: usize) -> Result<SqliteSchemaObject, String> {
    if row.values.len() < 5 {
        return Err("sqlite_schema record has fewer than five fields".into());
    }
    let source_type = required_text(&row.values[0], "type")?;
    let name = required_text(&row.values[1], "name")?;
    let table_name = required_text(&row.values[2], "tbl_name")?;
    let root_page = match &row.values[3] {
        SqliteValue::Integer { value } if *value > 0 => u32::try_from(*value).ok(),
        SqliteValue::Integer { .. } | SqliteValue::Null => None,
        _ => return Err("sqlite_schema rootpage is not an integer".into()),
    };
    let definition = match &row.values[4] {
        SqliteValue::Text { value, .. } => Some(value.clone()),
        SqliteValue::Null => None,
        _ => return Err("sqlite_schema sql field is not text or null".into()),
    };
    let kind = match source_type.to_ascii_lowercase().as_str() {
        "table" => SqliteSchemaObjectKind::Table,
        "index" => SqliteSchemaObjectKind::Index,
        "view" => SqliteSchemaObjectKind::View,
        "trigger" => SqliteSchemaObjectKind::Trigger,
        _ => SqliteSchemaObjectKind::Unknown,
    };
    Ok(SqliteSchemaObject {
        kind,
        source_type,
        name,
        table_name,
        root_page,
        definition,
        locator: record_locator(
            "sqlite_schema",
            &row.rowid.unwrap_or(ordinal as i64).to_string(),
            ordinal,
            None,
            row.byte_start,
            row.byte_end,
        ),
    })
}

fn required_text(value: &SqliteValue, field: &str) -> Result<String, String> {
    match value {
        SqliteValue::Text { value, .. } => Ok(value.clone()),
        _ => Err(format!("sqlite_schema {field} field is not text")),
    }
}

fn table_from_schema(object: &SqliteSchemaObject) -> Option<SqliteTable> {
    if object.kind != SqliteSchemaObjectKind::Table {
        return None;
    }
    let definition = object.definition.clone()?;
    Some(SqliteTable {
        name: object.name.clone(),
        root_page: object.root_page.unwrap_or(0),
        columns: parse_columns(&definition),
        without_rowid: contains_words(&definition, "without rowid"),
        strict: definition
            .trim_end_matches(';')
            .trim_end()
            .to_ascii_lowercase()
            .ends_with(" strict"),
        definition,
        locator: object.locator.clone(),
    })
}

fn view_from_schema(object: &SqliteSchemaObject) -> Option<SqliteView> {
    (object.kind == SqliteSchemaObjectKind::View).then(|| SqliteView {
        name: object.name.clone(),
        definition: object.definition.clone().unwrap_or_default(),
        locator: object.locator.clone(),
    })
}

fn index_from_schema(object: &SqliteSchemaObject) -> Option<SqliteIndex> {
    if object.kind != SqliteSchemaObjectKind::Index {
        return None;
    }
    let lower = object
        .definition
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Some(SqliteIndex {
        name: object.name.clone(),
        table_name: object.table_name.clone(),
        root_page: object.root_page,
        definition: object.definition.clone(),
        unique: lower.starts_with("create unique index"),
        partial: contains_words(&lower, " where "),
        locator: object.locator.clone(),
    })
}

fn parse_columns(sql: &str) -> Vec<SqliteColumn> {
    let Some(open) = find_unquoted(sql, '(') else {
        return Vec::new();
    };
    let Some(close) = matching_paren(sql, open) else {
        return Vec::new();
    };
    split_top_level(&sql[open + 1..close])
        .into_iter()
        .filter_map(|declaration| {
            let declaration = declaration.trim();
            if declaration.is_empty() || is_table_constraint(declaration) {
                return None;
            }
            let (name, rest) = identifier(declaration)?;
            let declared_type = rest
                .split_whitespace()
                .take_while(|word| {
                    !matches!(
                        word.to_ascii_lowercase().as_str(),
                        "constraint"
                            | "primary"
                            | "not"
                            | "unique"
                            | "check"
                            | "default"
                            | "collate"
                            | "references"
                            | "generated"
                            | "as"
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            Some(SqliteColumn {
                ordinal: 0,
                name,
                declaration: declaration.to_string(),
                declared_type: (!declared_type.is_empty()).then_some(declared_type),
                primary_key: contains_words(declaration, "primary key"),
            })
        })
        .enumerate()
        .map(|(ordinal, mut column)| {
            column.ordinal = ordinal;
            column
        })
        .collect()
}

fn is_table_constraint(text: &str) -> bool {
    let first = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        first.as_str(),
        "constraint" | "primary" | "unique" | "check" | "foreign"
    )
}

fn contains_words(text: &str, words: &str) -> bool {
    text.to_ascii_lowercase()
        .contains(&words.to_ascii_lowercase())
}

fn find_unquoted(text: &str, target: char) -> Option<usize> {
    let mut quote = None;
    for (index, ch) in text.char_indices() {
        match (quote, ch) {
            (None, '"' | '\'' | '`' | '[') => quote = Some(ch),
            (Some('['), ']') => quote = None,
            (Some(active), value) if active == value => quote = None,
            (None, value) if value == target => return Some(index),
            _ => {}
        }
    }
    None
}

fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    for (relative, ch) in text[open..].char_indices() {
        match (quote, ch) {
            (None, '"' | '\'' | '`' | '[') => quote = Some(ch),
            (Some('['), ']') => quote = None,
            (Some(active), value) if active == value => quote = None,
            (None, '(') => depth += 1,
            (None, ')') => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + relative);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(text: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quote = None;
    for (index, ch) in text.char_indices() {
        match (quote, ch) {
            (None, '"' | '\'' | '`' | '[') => quote = Some(ch),
            (Some('['), ']') => quote = None,
            (Some(active), value) if active == value => quote = None,
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, ',') if depth == 0 => {
                output.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    output.push(&text[start..]);
    output
}

fn identifier(text: &str) -> Option<(String, &str)> {
    let text = text.trim_start();
    let first = text.chars().next()?;
    if matches!(first, '"' | '\'' | '`' | '[') {
        let close = if first == '[' { ']' } else { first };
        let end = text[1..].find(close)? + 1;
        return Some((
            text[1..end].to_string(),
            text[end + close.len_utf8()..].trim_start(),
        ));
    }
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    Some((text[..end].to_string(), text[end..].trim_start()))
}

fn extract_selected(
    database: &btree::Database<'_>,
    tables: &[SqliteTable],
    selection: Option<&SqliteRecordSelection>,
    control: &OperationControl,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SqliteRecordSet> {
    let Some(selection) = selection else {
        return Vec::new();
    };
    let selected = selection.tables.iter().take(selection.max_tables);
    if selection.tables.len() > selection.max_tables {
        diagnostics.push(
            Diagnostic::budget_exhausted(
                PARSER,
                format!(
                    "selected {} tables but table budget is {}",
                    selection.tables.len(),
                    selection.max_tables
                ),
            )
            .with_explanation_key("sqlite.budget.tables")
            .partial(),
        );
    }
    let mut output = Vec::new();
    for requested in selected {
        let Some(table) = tables
            .iter()
            .find(|table| table.name.eq_ignore_ascii_case(requested))
        else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "sqlite.selection.table_not_found",
                    format!("selected table {requested:?} is not present"),
                )
                .partial(),
            );
            continue;
        };
        output.push(extract_table(
            database,
            table,
            selection.max_rows_per_table,
            control,
            diagnostics,
        ));
    }
    output
}

fn extract_table(
    database: &btree::Database<'_>,
    table: &SqliteTable,
    row_budget: usize,
    control: &OperationControl,
    diagnostics: &mut Vec<Diagnostic>,
) -> SqliteRecordSet {
    let columns = if table.without_rowid {
        Vec::new()
    } else {
        table
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect()
    };
    if table.without_rowid {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "sqlite.without_rowid.physical_order",
                format!(
                    "table {:?} uses WITHOUT ROWID; fields are exposed in physical key order",
                    table.name
                ),
            )
            .partial(),
        );
    }
    let (raw_records, truncated) = match database.table_records(table.root_page, row_budget) {
        Ok(result) => result,
        Err(error) => {
            diagnostics.push(decode_diagnostic(error).partial());
            return SqliteRecordSet {
                table: table.name.clone(),
                root_page: table.root_page,
                columns,
                records: Vec::new(),
                row_budget,
                truncated: false,
                complete: false,
            };
        }
    };
    let mut records = Vec::new();
    let mut complete = true;
    for (index, raw) in raw_records.into_iter().enumerate() {
        let cells = raw.values.len() as u64;
        if let Err(error) = control
            .budget()
            .consume_records(1)
            .and_then(|_| control.budget().consume_cells(cells))
            .and_then(|_| control.budget().consume_nodes(cells + 1))
        {
            diagnostics.push(error.diagnostic(PARSER).partial());
            complete = false;
            break;
        }
        let stable_key = raw
            .rowid
            .map(|rowid| format!("rowid:{rowid}"))
            .unwrap_or_else(|| value_key(&raw.values));
        let locator = record_locator(
            &table.name,
            &stable_key,
            index + 1,
            None,
            raw.byte_start,
            raw.byte_end,
        );
        let fields = raw
            .values
            .into_iter()
            .enumerate()
            .map(|(field_index, value)| {
                if matches!(&value, SqliteValue::Text { lossy: true, .. }) {
                    diagnostics.push(
                        Diagnostic::lossy(
                            PARSER,
                            format!(
                                "invalid text encoding in table {:?}, record {}, field {}",
                                table.name,
                                index + 1,
                                field_index + 1
                            ),
                        )
                        .with_locator(locator.clone()),
                    );
                }
                let name = table
                    .columns
                    .get(field_index)
                    .map(|column| column.name.clone())
                    .unwrap_or_else(|| format!("physical_{}", field_index + 1));
                SqliteField {
                    ordinal: field_index,
                    locator: record_locator(
                        &table.name,
                        &stable_key,
                        index + 1,
                        Some(&name),
                        raw.byte_start,
                        raw.byte_end,
                    ),
                    name,
                    value,
                }
            })
            .collect();
        records.push(SqliteRecord {
            ordinal: index + 1,
            stable_key,
            rowid: raw.rowid,
            fields,
            page: raw.page,
            cell: raw.cell,
            locator,
        });
    }
    if truncated {
        complete = false;
        diagnostics.push(
            Diagnostic::budget_exhausted(
                PARSER,
                format!(
                    "table {:?} contains more than the selected {} row budget",
                    table.name, row_budget
                ),
            )
            .with_explanation_key("sqlite.budget.rows")
            .partial(),
        );
    }
    SqliteRecordSet {
        table: table.name.clone(),
        root_page: table.root_page,
        columns,
        complete,
        records,
        row_budget,
        truncated,
    }
}

fn value_key(values: &[SqliteValue]) -> String {
    let canonical = serde_json::to_vec(values).expect("SQLite values serialize");
    let digest = Sha256::digest(canonical);
    format!("key:{}", hex(&digest))
}

fn byte_locator(start: usize, end: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ByteRange {
        byte_start: start,
        byte_end: end,
    })
    .expect("ordered byte range")
}

fn record_locator(
    table: &str,
    key: &str,
    ordinal: usize,
    field: Option<&str>,
    start: usize,
    end: usize,
) -> SourceLocator {
    byte_locator(start, end)
        .nested(LocationComponent::RecordRange {
            collection: format!("sqlite.table/{table}/{key}"),
            records: IndexRange::new(ordinal as u64, ordinal as u64 + 1, IndexBase::One)
                .expect("one-based record range"),
            field: field.map(str::to_string),
        })
        .expect("valid SQLite record locator")
}

fn diagnostic(code: &'static str, message: &'static str, offset: usize) -> Diagnostic {
    let mut diagnostic = Diagnostic::malformed(PARSER, message);
    diagnostic.code = DiagnosticCode::new(code);
    diagnostic
        .with_explanation_key(code)
        .with_locator(byte_locator(offset, offset))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}
