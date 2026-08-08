#![cfg(feature = "columnar")]

use grist::columnar::{
    ColumnarFormat, ColumnarOptions, ColumnarValue, parse_columnar, parse_parquet,
};
use grist::core::{OperationStatus, SourceInfo};
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};

#[test]
fn arrow_stream_preserves_schema_batches_values_projection_and_graph() {
    let bytes = arrow_stream();
    let envelope = parse_columnar(
        &bytes,
        SourceInfo::stdin("values.arrow"),
        &ColumnarOptions {
            row_start: 1,
            row_limit: Some(2),
            columns: vec!["id".into()],
            ..Default::default()
        },
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.format, ColumnarFormat::ArrowIpcStream);
    assert_eq!(document.schema.fields[0].name, "id");
    assert_eq!(document.batches[0].source_row_count, 3);
    let values = &document.batches[0].columns[0].values;
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].row, 1);
    assert!(
        matches!(values[0].value, ColumnarValue::SignedInteger { ref canonical } if canonical == "20")
    );
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:arrow"))
        .unwrap();
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
    let events = grist::columnar::parse_columnar_events(
        &bytes,
        SourceInfo::stdin("events.arrow"),
        &ColumnarOptions::default(),
    )
    .collect::<Vec<_>>();
    assert!(matches!(
        events.last(),
        Some(grist::columnar::ColumnarStreamEvent::End { complete: true })
    ));
    let file = parse_columnar(
        &arrow_file(),
        SourceInfo::stdin("values.feather"),
        &ColumnarOptions::default(),
    );
    assert_eq!(
        file.status,
        OperationStatus::Complete,
        "{:?}",
        file.diagnostics
    );
    assert_eq!(file.payload().unwrap().format, ColumnarFormat::ArrowIpcFile);
}

#[test]
fn parquet_preserves_footer_schema_row_groups_columns_and_typed_values() {
    let bytes = parquet_file();
    let envelope = parse_parquet(
        &bytes,
        SourceInfo::stdin("values.parquet"),
        &ColumnarOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.format, ColumnarFormat::Parquet);
    assert_eq!(document.metadata["created_by"], "grist-test");
    assert_eq!(document.schema.fields[0].name, "id");
    assert_eq!(document.batches.len(), 1);
    assert_eq!(document.batches[0].source_row_count, 3);
    assert!(
        matches!(document.batches[0].columns[0].values[2].value, ColumnarValue::SignedInteger { ref canonical } if canonical == "30")
    );
}

#[test]
fn malformed_inputs_and_projection_limits_are_explicit() {
    let malformed = parse_columnar(
        b"ARROW1badARROW1",
        SourceInfo::stdin("bad.arrow"),
        &ColumnarOptions::default(),
    );
    assert_eq!(malformed.status, OperationStatus::Failed);
    assert!(!malformed.diagnostics.is_empty());

    let projected = parse_parquet(
        &parquet_file(),
        SourceInfo::stdin("selected.parquet"),
        &ColumnarOptions {
            row_start: 1,
            row_limit: Some(1),
            columns: vec!["id".into()],
            ..Default::default()
        },
    );
    let values = &projected.payload().unwrap().batches[0].columns[0].values;
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].row, 1);

    let mut budget = grist::core::ResourceBudget::trusted_unbounded();
    budget.max_input_bytes = Some(1);
    let control = grist::core::OperationControl::new(
        &grist::core::BudgetSelection::custom(budget),
        grist::core::CancellationToken::new(),
    )
    .unwrap();
    let limited = grist::columnar::parse_columnar_with_operation_control(
        &parquet_file(),
        None,
        SourceInfo::stdin("limited.parquet"),
        &ColumnarOptions::default(),
        &control,
    );
    assert_eq!(limited.status, OperationStatus::Failed);
    assert_eq!(
        limited.diagnostics[0].code,
        "grist.budget.input_bytes.exhausted"
    );
}

struct Fb(Vec<u8>);
impl Fb {
    fn new() -> Self {
        Self(vec![0; 4])
    }
    fn align(&mut self, n: usize, remainder: usize) {
        while self.0.len() % n != remainder {
            self.0.push(0);
        }
    }
    fn table(&mut self, fields: &[u16], object: usize) -> usize {
        let vlen = 4 + fields.len() * 2;
        while (self.0.len() + vlen) % 8 != 0 {
            self.0.push(0);
        }
        let vstart = self.0.len();
        self.u16(vlen as u16);
        self.u16(object as u16);
        for field in fields {
            self.u16(*field);
        }
        let table = self.0.len();
        self.i32((table - vstart) as i32);
        self.0.resize(table + object, 0);
        table
    }
    fn string(&mut self, value: &str) -> usize {
        self.align(4, 0);
        let pos = self.0.len();
        self.u32(value.len() as u32);
        self.0.extend_from_slice(value.as_bytes());
        self.0.push(0);
        pos
    }
    fn vector(&mut self, count: usize, width: usize, align: usize) -> usize {
        if align > 4 {
            self.align(align, align - 4);
        } else {
            self.align(4, 0);
        }
        let pos = self.0.len();
        self.u32(count as u32);
        self.0.resize(self.0.len() + count * width, 0);
        pos
    }
    fn patch_u32(&mut self, at: usize, value: usize) {
        self.0[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
    }
    fn patch_offset(&mut self, at: usize, target: usize) {
        self.patch_u32(at, target - at);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn finish(mut self, root: usize) -> Vec<u8> {
        self.patch_u32(0, root);
        self.0
    }
}
fn put_i64(bytes: &mut [u8], at: usize, value: i64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn put_i32(bytes: &mut [u8], at: usize, value: i32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn schema_message() -> Vec<u8> {
    let mut fb = Fb::new();
    let message = fb.table(&[4, 6, 8, 16], 24);
    fb.0[message + 4..message + 6].copy_from_slice(&4i16.to_le_bytes());
    fb.0[message + 6] = 1;
    let schema = fb.table(&[4, 8], 12);
    fb.patch_offset(message + 8, schema);
    let fields = fb.vector(1, 4, 4);
    fb.patch_offset(schema + 8, fields);
    let field = fb.table(&[4, 8, 9, 12], 16);
    fb.patch_offset(fields + 4, field);
    let name = fb.string("id");
    fb.patch_offset(field + 4, name);
    fb.0[field + 8] = 0;
    fb.0[field + 9] = 2;
    let int_type = fb.table(&[4, 8], 12);
    fb.patch_offset(field + 12, int_type);
    put_i32(&mut fb.0, int_type + 4, 32);
    fb.0[int_type + 8] = 1;
    fb.finish(message)
}
fn batch_message() -> Vec<u8> {
    let mut fb = Fb::new();
    let message = fb.table(&[4, 6, 8, 16], 24);
    fb.0[message + 4..message + 6].copy_from_slice(&4i16.to_le_bytes());
    fb.0[message + 6] = 3;
    put_i64(&mut fb.0, message + 16, 12);
    let batch = fb.table(&[8, 16, 20], 24);
    fb.patch_offset(message + 8, batch);
    put_i64(&mut fb.0, batch + 8, 3);
    let nodes = fb.vector(1, 16, 8);
    fb.patch_offset(batch + 16, nodes);
    put_i64(&mut fb.0, nodes + 4, 3);
    let buffers = fb.vector(2, 16, 8);
    fb.patch_offset(batch + 20, buffers);
    put_i64(&mut fb.0, buffers + 4, 0);
    put_i64(&mut fb.0, buffers + 12, 0);
    put_i64(&mut fb.0, buffers + 20, 0);
    put_i64(&mut fb.0, buffers + 28, 12);
    fb.finish(message)
}
fn frame(metadata: Vec<u8>, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    out.extend_from_slice(&metadata);
    while out.len() % 8 != 0 {
        out.push(0);
    }
    out.extend_from_slice(body);
    while out.len() % 8 != 0 {
        out.push(0);
    }
    out
}
fn arrow_stream() -> Vec<u8> {
    let mut out = frame(schema_message(), &[]);
    let mut body = Vec::new();
    for value in [10i32, 20, 30] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    out.extend(frame(batch_message(), &body));
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

fn arrow_file() -> Vec<u8> {
    let mut stream = arrow_stream();
    stream.truncate(stream.len() - 8);
    let mut file = b"ARROW1\0\0".to_vec();
    file.extend(stream);
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(b"ARROW1");
    file
}
#[derive(Default)]
struct Compact(Vec<u8>);
impl Compact {
    fn field(&mut self, id: i16, previous: i16, kind: u8) {
        let delta = id - previous;
        if (1..=15).contains(&delta) {
            self.0.push(((delta as u8) << 4) | kind);
        } else {
            self.0.push(kind);
            self.zigzag(id as i64);
        }
    }
    fn zigzag(&mut self, value: i64) {
        self.varint(((value << 1) ^ (value >> 63)) as u64);
    }
    fn varint(&mut self, mut value: u64) {
        loop {
            let mut byte = (value & 127) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 128;
            }
            self.0.push(byte);
            if value == 0 {
                break;
            }
        }
    }
    fn binary(&mut self, value: &[u8]) {
        self.varint(value.len() as u64);
        self.0.extend_from_slice(value);
    }
    fn stop(&mut self) {
        self.0.push(0);
    }
}
fn schema_elem(name: &str, physical: Option<i64>, children: Option<i64>) -> Vec<u8> {
    let mut c = Compact::default();
    let mut prev = 0;
    if let Some(value) = physical {
        c.field(1, prev, 5);
        c.zigzag(value);
        prev = 1;
    }
    c.field(3, prev, 5);
    c.zigzag(0);
    prev = 3;
    c.field(4, prev, 8);
    c.binary(name.as_bytes());
    prev = 4;
    if let Some(value) = children {
        c.field(5, prev, 5);
        c.zigzag(value);
    }
    c.stop();
    c.0
}
fn page_header() -> Vec<u8> {
    let mut data = Compact::default();
    data.field(1, 0, 5);
    data.zigzag(3);
    data.field(2, 1, 5);
    data.zigzag(0);
    data.field(3, 2, 5);
    data.zigzag(3);
    data.field(4, 3, 5);
    data.zigzag(3);
    data.stop();
    let mut c = Compact::default();
    c.field(1, 0, 5);
    c.zigzag(0);
    c.field(2, 1, 5);
    c.zigzag(12);
    c.field(3, 2, 5);
    c.zigzag(12);
    c.field(5, 3, 12);
    c.0.extend(data.0);
    c.stop();
    c.0
}
fn column_metadata(column_size: i64) -> Vec<u8> {
    let mut c = Compact::default();
    c.field(1, 0, 5);
    c.zigzag(1);
    c.field(2, 1, 9);
    c.0.push(0x15);
    c.zigzag(0);
    c.field(3, 2, 9);
    c.0.push(0x18);
    c.binary(b"id");
    c.field(4, 3, 5);
    c.zigzag(0);
    c.field(5, 4, 6);
    c.zigzag(3);
    c.field(6, 5, 6);
    c.zigzag(12);
    c.field(7, 6, 6);
    c.zigzag(column_size);
    c.field(9, 7, 6);
    c.zigzag(4);
    c.stop();
    c.0
}
fn row_group(column_size: i64) -> Vec<u8> {
    let meta = column_metadata(column_size);
    let mut chunk = Compact::default();
    chunk.field(2, 0, 6);
    chunk.zigzag(4);
    chunk.field(3, 2, 12);
    chunk.0.extend(meta);
    chunk.stop();
    let mut group = Compact::default();
    group.field(1, 0, 9);
    group.0.push(0x1c);
    group.0.extend(chunk.0);
    group.field(2, 1, 6);
    group.zigzag(12);
    group.field(3, 2, 6);
    group.zigzag(3);
    group.field(6, 3, 6);
    group.zigzag(column_size);
    group.stop();
    group.0
}
fn parquet_file() -> Vec<u8> {
    let header = page_header();
    let column_size = (header.len() + 12) as i64;
    let mut footer = Compact::default();
    footer.field(1, 0, 5);
    footer.zigzag(2);
    footer.field(2, 1, 9);
    footer.0.push(0x2c);
    footer.0.extend(schema_elem("schema", None, Some(1)));
    footer.0.extend(schema_elem("id", Some(1), None));
    footer.field(3, 2, 6);
    footer.zigzag(3);
    footer.field(4, 3, 9);
    footer.0.push(0x1c);
    footer.0.extend(row_group(column_size));
    footer.field(6, 4, 8);
    footer.binary(b"grist-test");
    footer.stop();
    let mut out = b"PAR1".to_vec();
    out.extend(header);
    for value in [10i32, 20, 30] {
        out.extend_from_slice(&value.to_le_bytes())
    }
    out.extend(&footer.0);
    out.extend_from_slice(&(footer.0.len() as u32).to_le_bytes());
    out.extend_from_slice(b"PAR1");
    out
}
