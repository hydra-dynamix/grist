#![cfg(feature = "sqlite")]

use grist::core::{
    BudgetSelection, CancellationToken, OperationControl, OperationStatus, ResourceBudget,
    SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::registry::builtin_parser_registry;
use grist::sqlite::{
    SqliteOptions, SqliteRecordSelection, SqliteSchemaObjectKind, SqliteValue, inspect_sqlite_path,
    parse_sqlite, parse_sqlite_with_operation_control,
};

#[test]
fn schema_views_indexes_records_locators_and_graph_are_preserved() {
    let bytes = database();
    let envelope = parse_sqlite(
        &bytes,
        SourceInfo::stdin("records.sqlite"),
        &SqliteOptions {
            record_selection: Some(SqliteRecordSelection {
                tables: vec!["users".into()],
                max_tables: 1,
                max_rows_per_table: 2,
            }),
            include_internal_schema: false,
        },
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.tables[0].name, "users");
    assert_eq!(document.tables[0].columns[1].name, "name");
    assert_eq!(document.views[0].name, "names");
    assert_eq!(document.indexes[0].name, "idx_users_name");
    assert!(
        document
            .schema
            .iter()
            .any(|item| item.kind == SqliteSchemaObjectKind::Trigger)
    );
    let records = &document.record_sets[0].records;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].stable_key, "rowid:1");
    assert!(matches!(
        records[0].fields[1].value,
        SqliteValue::Text { ref value, lossy: false, .. } if value == "Ada"
    ));
    assert_eq!(records[0].locator.components().len(), 2);
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:sqlite"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Table)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Row)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Cell)
    );
}

#[test]
fn extensionless_sqlite_magic_selects_the_available_parser() {
    let detected = detect_with_registry(
        std::path::Path::new("extensionless"),
        &database(),
        None,
        None,
        &grist::core::Limits::default(),
        &builtin_parser_registry().unwrap(),
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detected.status, DetectionStatus::Selected);
    assert_eq!(detected.content_kind, ContentKind::Sqlite);
    assert_eq!(detected.candidates[0].identity.format, "sqlite");
    assert_eq!(
        detected.candidates[0].parser_availability,
        grist::core::ParserAvailability::Available
    );
}

#[test]
fn caller_selection_and_table_row_budgets_are_mandatory_and_explicit() {
    let bytes = database();
    let invalid = parse_sqlite(
        &bytes,
        SourceInfo::stdin("invalid.sqlite"),
        &SqliteOptions {
            record_selection: Some(SqliteRecordSelection {
                tables: vec!["users".into()],
                max_tables: 1,
                max_rows_per_table: 0,
            }),
            ..Default::default()
        },
    );
    assert_eq!(invalid.status, OperationStatus::Failed);
    assert_eq!(
        invalid.diagnostics[0].explanation_key.as_deref(),
        Some("sqlite.options.record_selection")
    );

    let bounded = parse_sqlite(
        &bytes,
        SourceInfo::stdin("bounded.sqlite"),
        &SqliteOptions {
            record_selection: Some(SqliteRecordSelection {
                tables: vec!["users".into()],
                max_tables: 1,
                max_rows_per_table: 1,
            }),
            ..Default::default()
        },
    );
    assert_eq!(bounded.status, OperationStatus::Partial);
    assert!(bounded.payload().unwrap().record_sets[0].truncated);
    assert_eq!(bounded.payload().unwrap().record_sets[0].records.len(), 1);

    let schema_only = parse_sqlite(
        &bytes,
        SourceInfo::stdin("schema.sqlite"),
        &SqliteOptions::default(),
    );
    assert_eq!(schema_only.status, OperationStatus::Complete);
    assert!(schema_only.payload().unwrap().record_sets.is_empty());
}

#[test]
fn malformed_large_and_unreadable_inputs_have_explicit_status() {
    let malformed = parse_sqlite(
        b"SQLite format 3",
        SourceInfo::stdin("truncated.sqlite"),
        &SqliteOptions::default(),
    );
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert_eq!(malformed.diagnostics[0].code, "sqlite.header.invalid");

    let bytes = database();
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_input_bytes = Some(16);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), CancellationToken::new()).unwrap();
    let large = parse_sqlite_with_operation_control(
        &bytes,
        SourceInfo::stdin("large.sqlite"),
        &SqliteOptions::default(),
        &control,
    );
    assert_eq!(large.status, OperationStatus::Failed);
    assert_eq!(
        large.diagnostics[0].code,
        "grist.budget.input_bytes.exhausted"
    );

    let missing = std::env::temp_dir().join(format!("grist-sqlite-missing-{}", std::process::id()));
    let unreadable = inspect_sqlite_path(&missing, &SqliteOptions::default());
    assert_eq!(unreadable.status, OperationStatus::Failed);
    assert_eq!(
        unreadable.diagnostics[0].code,
        "sqlite.io.locked_or_unreadable"
    );
}

#[test]
fn path_inspection_never_mutates_or_replays_active_content() {
    let directory =
        std::env::temp_dir().join(format!("grist-sqlite-readonly-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("hostile.sqlite");
    let bytes = database();
    std::fs::write(&path, &bytes).unwrap();
    std::fs::write(
        directory.join("hostile.sqlite-wal"),
        b"not a WAL; must never be replayed",
    )
    .unwrap();
    let before = std::fs::read(&path).unwrap();
    let envelope = inspect_sqlite_path(&path, &SqliteOptions::default());
    let after = std::fs::read(&path).unwrap();
    assert_eq!(before, after);
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|item| item.code == "sqlite.sidecar.not_replayed")
    );
    assert!(envelope.payload().unwrap().schema.iter().any(|item| {
        item.definition
            .as_deref()
            .is_some_and(|sql| sql.contains("load_extension"))
    }));
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(feature = "cli")]
#[test]
fn cli_routes_sqlite_with_explicit_selection_options() {
    let directory = std::env::temp_dir().join(format!("grist-sqlite-cli-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let database_path = directory.join("records.sqlite");
    let options_path = directory.join("options.json");
    std::fs::write(&database_path, database()).unwrap();
    std::fs::write(
        &options_path,
        br#"{"record_selection":{"tables":["users"],"max_tables":1,"max_rows_per_table":2},"include_internal_schema":false}"#,
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_grist"))
        .args([
            "parse",
            "sqlite",
            database_path.to_str().unwrap(),
            "--options",
            options_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["kind"], "sqlite");
    assert_eq!(envelope["status"], "complete");
    assert_eq!(
        envelope["payload"]["record_sets"][0]["records"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    std::fs::remove_dir_all(directory).unwrap();
}

fn database() -> Vec<u8> {
    const PAGE: usize = 1024;
    let mut bytes = vec![0u8; PAGE * 3];
    bytes[..16].copy_from_slice(b"SQLite format 3\0");
    put_u16(&mut bytes, 16, PAGE as u16);
    bytes[18] = 1;
    bytes[19] = 1;
    bytes[21] = 64;
    bytes[22] = 32;
    bytes[23] = 32;
    put_u32(&mut bytes, 24, 1);
    put_u32(&mut bytes, 28, 3);
    put_u32(&mut bytes, 40, 1);
    put_u32(&mut bytes, 44, 4);
    put_u32(&mut bytes, 56, 1);
    put_u32(&mut bytes, 92, 1);
    put_u32(&mut bytes, 96, 3_046_000);

    let schema = [
        vec![
            Value::Text("table"),
            Value::Text("users"),
            Value::Text("users"),
            Value::Integer(2),
            Value::Text("CREATE TABLE users(id INTEGER PRIMARY KEY, name TEXT)"),
        ],
        vec![
            Value::Text("view"),
            Value::Text("names"),
            Value::Text("names"),
            Value::Integer(0),
            Value::Text("CREATE VIEW names AS SELECT name FROM users"),
        ],
        vec![
            Value::Text("index"),
            Value::Text("idx_users_name"),
            Value::Text("users"),
            Value::Integer(3),
            Value::Text("CREATE INDEX idx_users_name ON users(name)"),
        ],
        vec![
            Value::Text("trigger"),
            Value::Text("hostile"),
            Value::Text("users"),
            Value::Integer(0),
            Value::Text(
                "CREATE TRIGGER hostile AFTER INSERT ON users BEGIN SELECT load_extension('pwn'); END",
            ),
        ],
    ];
    write_leaf_page(
        &mut bytes[..PAGE],
        100,
        schema
            .iter()
            .enumerate()
            .map(|(index, values)| table_cell((index + 1) as u64, &record(values))),
    );

    let rows = [
        vec![Value::Null, Value::Text("Ada")],
        vec![Value::Null, Value::Text("Lin")],
    ];
    write_leaf_page(
        &mut bytes[PAGE..PAGE * 2],
        0,
        rows.iter()
            .enumerate()
            .map(|(index, values)| table_cell((index + 1) as u64, &record(values))),
    );
    write_index_leaf_page(&mut bytes[PAGE * 2..]);
    bytes
}

#[derive(Clone, Copy)]
enum Value<'a> {
    Null,
    Integer(i64),
    Text(&'a str),
}

fn record(values: &[Value<'_>]) -> Vec<u8> {
    let mut serials = Vec::new();
    let mut body = Vec::new();
    for value in values {
        match value {
            Value::Null => serials.push(0),
            Value::Integer(0) => serials.push(8),
            Value::Integer(1) => serials.push(9),
            Value::Integer(value) => {
                serials.push(1);
                body.push(*value as u8);
            }
            Value::Text(text) => {
                serials.push(13 + text.len() as u64 * 2);
                body.extend_from_slice(text.as_bytes());
            }
        }
    }
    let encoded_serials = serials.into_iter().flat_map(varint).collect::<Vec<_>>();
    let mut output = varint((encoded_serials.len() + 1) as u64);
    output.extend(encoded_serials);
    output.extend(body);
    output
}

fn table_cell(rowid: u64, payload: &[u8]) -> Vec<u8> {
    let mut cell = varint(payload.len() as u64);
    cell.extend(varint(rowid));
    cell.extend_from_slice(payload);
    cell
}

fn write_leaf_page<I>(page: &mut [u8], header: usize, cells: I)
where
    I: IntoIterator<Item = Vec<u8>>,
{
    let cells = cells.into_iter().collect::<Vec<_>>();
    page[header] = 0x0d;
    put_u16(page, header + 3, cells.len() as u16);
    let mut cursor = page.len();
    for (index, cell) in cells.iter().enumerate() {
        cursor -= cell.len();
        page[cursor..cursor + cell.len()].copy_from_slice(cell);
        put_u16(page, header + 8 + index * 2, cursor as u16);
    }
    put_u16(page, header + 5, cursor as u16);
}

fn write_index_leaf_page(page: &mut [u8]) {
    page[0] = 0x0a;
    put_u16(page, 3, 0);
    put_u16(page, 5, page.len() as u16);
}

fn varint(mut value: u64) -> Vec<u8> {
    if value <= 0x7f {
        return vec![value as u8];
    }
    let mut groups = Vec::new();
    while value != 0 {
        groups.push((value & 0x7f) as u8);
        value >>= 7;
    }
    groups.reverse();
    let last = groups.len() - 1;
    for byte in &mut groups[..last] {
        *byte |= 0x80;
    }
    groups
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
