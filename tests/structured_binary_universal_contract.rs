#![cfg(feature = "structured-binary")]

use grist::core::{
    BudgetSelection, ContentIdentity, Limits, LocationComponent, OperationControl, OperationStatus,
    ResourceBudget, SourceInfo,
};
use grist::detect::{ContentKind, detect_path};
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::segment::{SegmentOptions, segment_document_graph};
use grist::structured_binary::{
    BinaryMalformedRecovery, BinaryScalar, BinarySchemaIdentity, BinaryValueKind,
    ProtobufDecodeOptions, StructuredBinaryFormat, StructuredBinaryOptions, parse_cbor,
    parse_messagepack, parse_structured_binary, parse_structured_binary_with_operation_control,
};
use std::path::Path;

#[test]
fn cbor_preserves_tags_duplicates_non_json_types_and_exact_record_bytes() {
    let bytes = [
        0xd9, 0xd9, 0xf7, 0xa2, 0x61, b'a', 0x01, 0x61, b'a', 0x82, 0xf5, 0x42, 0x00, 0xff,
    ];
    let envelope = parse_cbor(&bytes, SourceInfo::stdin("value.cbor"));
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document = envelope.payload().unwrap();
    assert!(matches!(
        document.schema_identity,
        BinarySchemaIdentity::Cbor {
            self_described: true,
            ..
        }
    ));
    assert_eq!(document.tags[0].tag, 55_799);
    let root = &document.records[0].value;
    assert_eq!(root.kind, BinaryValueKind::Map);
    assert_eq!(root.entries.len(), 2);
    assert_eq!(root.entries[1].duplicate_ordinal, 2);
    assert!(matches!(
        root.entries[1].value.items[1].scalar,
        Some(BinaryScalar::Bytes {
            ref hex,
            length: 2
        }) if hex == "00ff"
    ));
    assert!(matches!(
        root.locator.components(),
        [
            LocationComponent::RecordRange { .. },
            LocationComponent::ByteRange {
                byte_start: 0,
                byte_end: 14
            }
        ]
    ));
    assert!(
        document
            .json_projection
            .as_ref()
            .unwrap()
            .get("$tag")
            .is_some()
    );
}

#[test]
fn cbor_sequences_indefinite_items_and_limits_are_explicit() {
    let bytes = [0x9f, 0x01, 0x02, 0xff, 0x61, b'x'];
    let envelope = parse_cbor(&bytes, SourceInfo::stdin("sequence.cbor"));
    let document = envelope.payload().unwrap();
    assert_eq!(document.records.len(), 2);
    assert!(document.records[0].value.indefinite);
    assert!(matches!(
        document.records[1].locator.components()[0],
        LocationComponent::RecordRange { .. }
    ));
    assert!(matches!(
        document.records[1].value.locator.components()[1],
        LocationComponent::ByteRange {
            byte_start: 0,
            byte_end: 2
        }
    ));

    let deep = parse_structured_binary(
        &[0x81, 0x81, 0x81, 0x00],
        StructuredBinaryFormat::Cbor,
        SourceInfo::stdin("deep.cbor"),
        &StructuredBinaryOptions {
            max_nesting_depth: 2,
            ..Default::default()
        },
    );
    assert_eq!(deep.status, OperationStatus::Failed);
    assert_eq!(deep.diagnostics[0].code, "binary.nesting_limit");

    let oversized = parse_structured_binary(
        &[0x43, 1, 2, 3],
        StructuredBinaryFormat::Cbor,
        SourceInfo::stdin("blob.cbor"),
        &StructuredBinaryOptions {
            max_blob_bytes: 2,
            ..Default::default()
        },
    );
    assert_eq!(oversized.status, OperationStatus::Failed);
    assert_eq!(oversized.diagnostics[0].code, "binary.blob_limit");
}

#[test]
fn messagepack_preserves_extensions_timestamp_and_malformed_policy() {
    let bytes = [
        0x82, 0xa1, b'x', 0x01, 0xa1, b'e', 0xd6, 0xff, 0x00, 0x00, 0x00, 0x2a,
    ];
    let envelope = parse_messagepack(&bytes, SourceInfo::stdin("value.msgpack"));
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document = envelope.payload().unwrap();
    assert_eq!(document.extensions.len(), 1);
    let extension = &document.extensions[0];
    assert_eq!(extension.type_code, -1);
    assert_eq!(extension.data_hex, "0000002a");
    assert_eq!(extension.timestamp.as_ref().unwrap().seconds, 42);
    assert!(
        document.json_projection.as_ref().unwrap()["e"]
            .get("$extension")
            .is_some()
    );

    let strict = parse_messagepack(&[0xc1], SourceInfo::stdin("bad.msgpack"));
    assert_eq!(strict.status, OperationStatus::Failed);
    assert_eq!(strict.diagnostics[0].code, "messagepack.reserved_marker");
    let recovered = parse_structured_binary(
        &[0xc1],
        StructuredBinaryFormat::MessagePack,
        SourceInfo::stdin("bad.msgpack"),
        &StructuredBinaryOptions {
            malformed_recovery: BinaryMalformedRecovery::PreserveRaw,
            ..Default::default()
        },
    );
    assert_eq!(recovered.status, OperationStatus::Partial);
    let value = &recovered.payload().unwrap().records[0].value;
    assert_eq!(value.kind, BinaryValueKind::Unknown);
    assert!(value.recovered);
}

#[test]
fn protobuf_requires_descriptor_and_preserves_known_extensions_and_unknown_fields() {
    let descriptor = person_descriptor_set(5);
    let message = [
        0x08, 0x96, 0x01, // id = 150
        0x12, 0x03, b'A', b'd', b'a', // name = Ada
        0x1a, 0x03, 0x01, 0x02, 0x03, // packed repeated nums
        0xa2, 0x06, 0x03, b'e', b'x', b't', // extension 100
        0x48, 0x07, // unknown field 9
    ];
    let options = StructuredBinaryOptions {
        protobuf: Some(ProtobufDecodeOptions {
            descriptor_set: descriptor.clone(),
            message_name: ".example.Person".to_string(),
            preserve_unknown_fields: true,
        }),
        ..Default::default()
    };
    let first = parse_structured_binary(
        &message,
        StructuredBinaryFormat::Protobuf,
        SourceInfo::stdin("person.pb"),
        &options,
    );
    let second = parse_structured_binary(
        &message,
        StructuredBinaryFormat::Protobuf,
        SourceInfo::stdin("person.pb"),
        &options,
    );
    assert_eq!(first.status, OperationStatus::Complete);
    assert_eq!(first.identity, second.identity);
    let document = first.payload().unwrap();
    let BinarySchemaIdentity::ProtobufDescriptor {
        descriptor_sha256,
        message_name,
        syntax,
        ..
    } = &document.schema_identity
    else {
        panic!("expected descriptor identity")
    };
    assert!(descriptor_sha256.starts_with("sha256:"));
    assert_eq!(message_name, "example.Person");
    assert_eq!(*syntax, grist::structured_binary::ProtobufSyntax::Proto3);
    assert_eq!(document.unknown_fields.len(), 1);
    assert_eq!(document.unknown_fields[0].field_number, 9);
    assert_eq!(document.unknown_fields[0].raw_hex, "4807");
    let root = &document.records[0].value;
    assert_eq!(root.entries[0].field_name.as_deref(), Some("id"));
    assert_eq!(root.entries[1].field_name.as_deref(), Some("name"));
    assert_eq!(
        root.entries
            .iter()
            .filter(|entry| entry.field_name.as_deref() == Some("nums"))
            .count(),
        3
    );
    let extension = root
        .entries
        .iter()
        .find(|entry| entry.field_number == Some(100))
        .unwrap();
    assert!(extension.value.protobuf_field.as_ref().unwrap().extension);
    assert_eq!(document.json_projection.as_ref().unwrap()["name"], "Ada");
    assert_eq!(
        document.json_projection.as_ref().unwrap()["nums"],
        serde_json::json!([1, 2, 3])
    );

    let missing = parse_structured_binary(
        &message,
        StructuredBinaryFormat::Protobuf,
        SourceInfo::stdin("person.pb"),
        &StructuredBinaryOptions::default(),
    );
    assert_eq!(missing.status, OperationStatus::Failed);
    assert_eq!(missing.diagnostics[0].code, "protobuf.descriptor_required");
}

#[test]
fn protobuf_wrong_descriptor_and_required_fields_are_diagnostic() {
    let wrong = person_descriptor_set(9);
    let wire_mismatch = parse_structured_binary(
        &[0x08, 0x01],
        StructuredBinaryFormat::Protobuf,
        SourceInfo::stdin("wrong.pb"),
        &StructuredBinaryOptions {
            protobuf: Some(ProtobufDecodeOptions {
                descriptor_set: wrong,
                message_name: "example.Person".to_string(),
                preserve_unknown_fields: true,
            }),
            ..Default::default()
        },
    );
    assert_eq!(wire_mismatch.status, OperationStatus::Partial);
    assert!(
        wire_mismatch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "protobuf.wire_type_mismatch"),
        "{:#?}",
        wire_mismatch.diagnostics
    );
    assert_eq!(
        wire_mismatch.payload().unwrap().unknown_fields[0].reason,
        "wire type does not match the supplied descriptor"
    );

    let required_descriptor = required_descriptor_set();
    let missing_required = parse_structured_binary(
        &[],
        StructuredBinaryFormat::Protobuf,
        SourceInfo::stdin("required.pb"),
        &StructuredBinaryOptions {
            protobuf: Some(ProtobufDecodeOptions {
                descriptor_set: required_descriptor,
                message_name: "example.Required".to_string(),
                preserve_unknown_fields: true,
            }),
            ..Default::default()
        },
    );
    assert_eq!(missing_required.status, OperationStatus::Partial);
    assert!(
        missing_required
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("required proto2 field") })
    );
}

fn person_descriptor_set(id_type: u64) -> Vec<u8> {
    let id = field_descriptor("id", 1, 1, id_type, None, false);
    let name = field_descriptor("name", 2, 1, 9, None, false);
    let nums = field_descriptor("nums", 3, 3, 5, None, true);
    let message = message_descriptor("Person", &[id, name, nums]);
    let extension = extension_descriptor("note", 100, 9, ".example.Person");
    file_descriptor_set(
        "person.proto",
        "example",
        "proto3",
        &[message],
        &[extension],
    )
}

fn required_descriptor_set() -> Vec<u8> {
    let required = field_descriptor("must", 1, 2, 9, None, false);
    let message = message_descriptor("Required", &[required]);
    file_descriptor_set("required.proto", "example", "proto2", &[message], &[])
}

fn file_descriptor_set(
    name: &str,
    package: &str,
    syntax: &str,
    messages: &[Vec<u8>],
    extensions: &[Vec<u8>],
) -> Vec<u8> {
    let mut file = Vec::new();
    string_field(1, name, &mut file);
    string_field(2, package, &mut file);
    for message in messages {
        bytes_field(4, message, &mut file);
    }
    for extension in extensions {
        bytes_field(7, extension, &mut file);
    }
    string_field(12, syntax, &mut file);
    let mut set = Vec::new();
    bytes_field(1, &file, &mut set);
    set
}

fn message_descriptor(name: &str, fields: &[Vec<u8>]) -> Vec<u8> {
    let mut message = Vec::new();
    string_field(1, name, &mut message);
    for field in fields {
        bytes_field(2, field, &mut message);
    }
    message
}

fn field_descriptor(
    name: &str,
    number: u64,
    label: u64,
    kind: u64,
    type_name: Option<&str>,
    packed: bool,
) -> Vec<u8> {
    let mut field = Vec::new();
    string_field(1, name, &mut field);
    varint_field(3, number, &mut field);
    varint_field(4, label, &mut field);
    varint_field(5, kind, &mut field);
    if let Some(type_name) = type_name {
        string_field(6, type_name, &mut field);
    }
    if packed {
        let mut options = Vec::new();
        varint_field(2, 1, &mut options);
        bytes_field(8, &options, &mut field);
    }
    field
}

fn extension_descriptor(name: &str, number: u64, kind: u64, extendee: &str) -> Vec<u8> {
    let mut field = field_descriptor(name, number, 1, kind, None, false);
    let mut with_extendee = Vec::new();
    string_field(1, name, &mut with_extendee);
    string_field(2, extendee, &mut with_extendee);
    with_extendee.extend(field.drain(name.len() + 2..));
    with_extendee
}

fn string_field(number: u64, value: &str, output: &mut Vec<u8>) {
    bytes_field(number, value.as_bytes(), output);
}

fn bytes_field(number: u64, value: &[u8], output: &mut Vec<u8>) {
    encode_varint((number << 3) | 2, output);
    encode_varint(value.len() as u64, output);
    output.extend_from_slice(value);
}

fn varint_field(number: u64, value: u64, output: &mut Vec<u8>) {
    encode_varint(number << 3, output);
    encode_varint(value, output);
}

fn encode_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

#[test]
fn extensionless_detection_uses_complete_structural_probes_without_guessing_overlap() {
    let cbor = detect_path(
        Path::new("value"),
        &[0xa1, 0x61, b'a', 0x01],
        &Limits::default(),
    );
    assert_eq!(cbor.content_kind, ContentKind::Cbor, "{cbor:#?}");
    let messagepack = detect_path(
        Path::new("value"),
        &[0x81, 0xa1, b'a', 0x01],
        &Limits::default(),
    );
    assert_eq!(messagepack.content_kind, ContentKind::MessagePack);
    let protobuf = detect_path(Path::new("value"), &[0x08, 0x01], &Limits::default());
    assert_eq!(protobuf.content_kind, ContentKind::Protobuf);
}

#[test]
fn graph_segments_schema_and_shared_budget_are_deterministic() {
    let bytes = [0xa2, 0x61, b'a', 0x01, 0x61, b'b', 0x82, 0xf5, 0xf6];
    let first = parse_cbor(&bytes, SourceInfo::stdin("graph.cbor"));
    let second = parse_cbor(&bytes, SourceInfo::stdin("graph.cbor"));
    assert_eq!(first.identity, second.identity);
    let document = first.into_payload().unwrap();
    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:binary"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Field)
    );
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.kind,
                    DocumentNodeKind::Record
                        | DocumentNodeKind::Field
                        | DocumentNodeKind::StructuredValue
                )
            })
            .all(|node| node.locator.is_some())
    );
    let source_identity = ContentIdentity::for_raw_bytes(&bytes);
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        &source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(!segments.segments.is_empty());
    assert!(
        segments
            .segments
            .iter()
            .all(|segment| !segment.locators.is_empty())
    );

    let schema = grist::schema::schema_json("structured-binary").unwrap();
    let report = grist::schema::validate_against_schema(
        "structured-binary",
        grist::core::SchemaVersion::STRUCTURED_BINARY_V1,
        &serde_json::to_value(&document).unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nodes = Some(2);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let budgeted = parse_structured_binary_with_operation_control(
        &bytes,
        StructuredBinaryFormat::Cbor,
        SourceInfo::stdin("budget.cbor"),
        &StructuredBinaryOptions::default(),
        &control,
    );
    assert_eq!(budgeted.status, OperationStatus::Partial);
    assert!(
        budgeted
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str().contains("budget"))
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_routes_cbor_messagepack_and_descriptor_driven_protobuf() {
    use std::process::Command;

    let root = std::env::temp_dir().join(format!("grist-structured-binary-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let cbor_path = root.join("value.cbor");
    let messagepack_path = root.join("value.msgpack");
    let protobuf_path = root.join("person.pb");
    let descriptor_path = root.join("person.desc");
    std::fs::write(&cbor_path, [0xa1, 0x61, b'a', 0x01]).unwrap();
    std::fs::write(&messagepack_path, [0x81, 0xa1, b'a', 0x01]).unwrap();
    std::fs::write(&protobuf_path, [0x08, 0x2a]).unwrap();
    std::fs::write(&descriptor_path, person_descriptor_set(5)).unwrap();

    for arguments in [
        vec![
            "parse".to_string(),
            "cbor".to_string(),
            cbor_path.display().to_string(),
        ],
        vec![
            "parse".to_string(),
            "messagepack".to_string(),
            messagepack_path.display().to_string(),
        ],
        vec![
            "parse".to_string(),
            "protobuf".to_string(),
            protobuf_path.display().to_string(),
            "--descriptor".to_string(),
            descriptor_path.display().to_string(),
            "--message".to_string(),
            "example.Person".to_string(),
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_grist"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["kind"], "structured_binary");
        assert_eq!(envelope["status"], "complete");
    }
    std::fs::remove_dir_all(root).unwrap();
}
