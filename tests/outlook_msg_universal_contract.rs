#![cfg(feature = "email-message")]

use grist::container::{ArtifactExtractionStatus, ArtifactSafetyClassification};
use grist::core::{
    BudgetSelection, ContentIdentity, LocationComponent, OperationControl, OperationStatus,
    ResourceBudget, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::outlook::{
    MapiValue, MsgBodyKind, OutlookMsgOptions, parse_outlook_msg,
    parse_outlook_msg_with_operation_control,
};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::segment::{SegmentOptions, segment_document_graph};

const FREE: u32 = 0xffff_ffff;
const END: u32 = 0xffff_fffe;
const FAT: u32 = 0xffff_fffd;
const SECTOR: usize = 512;
const MINI_SECTOR: usize = 64;

#[derive(Clone)]
enum Node {
    Storage(&'static str, Vec<Node>),
    Stream(&'static str, Vec<u8>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FlatKind {
    Root,
    Storage,
    Stream,
}

struct FlatNode {
    name: String,
    kind: FlatKind,
    children: Vec<usize>,
    data: Vec<u8>,
    mini_start: u32,
}

fn flatten(nodes: &[Node], parent: usize, flat: &mut Vec<FlatNode>) {
    for node in nodes {
        let id = flat.len();
        match node {
            Node::Storage(name, _) => flat.push(FlatNode {
                name: (*name).to_string(),
                kind: FlatKind::Storage,
                children: Vec::new(),
                data: Vec::new(),
                mini_start: END,
            }),
            Node::Stream(name, data) => flat.push(FlatNode {
                name: (*name).to_string(),
                kind: FlatKind::Stream,
                children: Vec::new(),
                data: data.clone(),
                mini_start: END,
            }),
        }
        flat[parent].children.push(id);
        if let Node::Storage(_, children) = node {
            flatten(children, id, flat);
        }
    }
}

fn cfb(nodes: Vec<Node>) -> Vec<u8> {
    let mut flat = vec![FlatNode {
        name: "Root Entry".to_string(),
        kind: FlatKind::Root,
        children: Vec::new(),
        data: Vec::new(),
        mini_start: END,
    }];
    flatten(&nodes, 0, &mut flat);
    let mut mini_stream = Vec::new();
    let mut mini_fat = Vec::<u32>::new();
    for node in &mut flat {
        if node.kind != FlatKind::Stream || node.data.is_empty() {
            continue;
        }
        let count = node.data.len().div_ceil(MINI_SECTOR);
        node.mini_start = mini_fat.len() as u32;
        for index in 0..count {
            let start = index * MINI_SECTOR;
            let end = (start + MINI_SECTOR).min(node.data.len());
            mini_stream.extend_from_slice(&node.data[start..end]);
            mini_stream.resize(mini_stream.len().next_multiple_of(MINI_SECTOR), 0);
            mini_fat.push(if index + 1 == count {
                END
            } else {
                node.mini_start + index as u32 + 1
            });
        }
    }
    let directory_sector_count = (flat.len() * 128).div_ceil(SECTOR);
    let mini_stream_sector_count = mini_stream.len().div_ceil(SECTOR);
    let mini_fat_sector_count = (mini_fat.len() * 4).div_ceil(SECTOR).max(1);
    let directory_start = 0u32;
    let mini_stream_start = directory_sector_count as u32;
    let mini_fat_start = mini_stream_start + mini_stream_sector_count as u32;
    let fat_sector = mini_fat_start + mini_fat_sector_count as u32;
    assert!(fat_sector < 128, "test CFB needs one FAT sector");

    let mut directory = vec![0u8; directory_sector_count * SECTOR];
    for (id, node) in flat.iter().enumerate() {
        let entry = &mut directory[id * 128..id * 128 + 128];
        let mut name = node.name.encode_utf16().collect::<Vec<_>>();
        name.truncate(31);
        for (index, unit) in name.iter().chain(std::iter::once(&0)).enumerate() {
            entry[index * 2..index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        put_u16(entry, 64, ((name.len() + 1) * 2) as u16);
        entry[66] = match node.kind {
            FlatKind::Root => 5,
            FlatKind::Storage => 1,
            FlatKind::Stream => 2,
        };
        entry[67] = 1;
        put_u32(entry, 68, FREE);
        let next_sibling = flat
            .iter()
            .enumerate()
            .find_map(|(parent_id, parent)| {
                parent
                    .children
                    .iter()
                    .position(|child| *child == id)
                    .and_then(|position| {
                        parent
                            .children
                            .get(position + 1)
                            .copied()
                            .map(|next| (parent_id, next))
                    })
            })
            .map(|(_, next)| next as u32)
            .unwrap_or(FREE);
        put_u32(entry, 72, next_sibling);
        put_u32(
            entry,
            76,
            node.children
                .first()
                .copied()
                .map(|id| id as u32)
                .unwrap_or(FREE),
        );
        match node.kind {
            FlatKind::Root => {
                put_u32(
                    entry,
                    116,
                    if mini_stream.is_empty() {
                        END
                    } else {
                        mini_stream_start
                    },
                );
                put_u64(entry, 120, mini_stream.len() as u64);
            }
            FlatKind::Stream => {
                put_u32(entry, 116, node.mini_start);
                put_u64(entry, 120, node.data.len() as u64);
            }
            FlatKind::Storage => put_u32(entry, 116, END),
        }
    }

    let mut sectors = vec![vec![0u8; SECTOR]; fat_sector as usize + 1];
    for index in 0..directory_sector_count {
        sectors[index].copy_from_slice(&directory[index * SECTOR..(index + 1) * SECTOR]);
    }
    for index in 0..mini_stream_sector_count {
        let start = index * SECTOR;
        let end = (start + SECTOR).min(mini_stream.len());
        sectors[mini_stream_start as usize + index][..end - start]
            .copy_from_slice(&mini_stream[start..end]);
    }
    let mut mini_fat_bytes = vec![0xff; mini_fat_sector_count * SECTOR];
    for (index, value) in mini_fat.iter().enumerate() {
        put_u32(&mut mini_fat_bytes, index * 4, *value);
    }
    for index in 0..mini_fat_sector_count {
        sectors[mini_fat_start as usize + index]
            .copy_from_slice(&mini_fat_bytes[index * SECTOR..(index + 1) * SECTOR]);
    }
    let mut fat = vec![FREE; SECTOR / 4];
    chain(&mut fat, directory_start, directory_sector_count);
    chain(&mut fat, mini_stream_start, mini_stream_sector_count);
    chain(&mut fat, mini_fat_start, mini_fat_sector_count);
    fat[fat_sector as usize] = FAT;
    for (index, value) in fat.into_iter().enumerate() {
        put_u32(&mut sectors[fat_sector as usize], index * 4, value);
    }

    let mut header = vec![0u8; SECTOR];
    header[..8].copy_from_slice(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1");
    put_u16(&mut header, 24, 0x003e);
    put_u16(&mut header, 26, 3);
    put_u16(&mut header, 28, 0xfffe);
    put_u16(&mut header, 30, 9);
    put_u16(&mut header, 32, 6);
    put_u32(&mut header, 44, 1);
    put_u32(&mut header, 48, directory_start);
    put_u32(&mut header, 56, 4096);
    put_u32(&mut header, 60, mini_fat_start);
    put_u32(&mut header, 64, mini_fat_sector_count as u32);
    put_u32(&mut header, 68, END);
    for index in 0..109 {
        put_u32(
            &mut header,
            76 + index * 4,
            if index == 0 { fat_sector } else { FREE },
        );
    }
    let mut output = header;
    for sector in sectors {
        output.extend_from_slice(&sector);
    }
    output
}

fn chain(fat: &mut [u32], start: u32, count: usize) {
    for index in 0..count {
        fat[start as usize + index] = if index + 1 == count {
            END
        } else {
            start + index as u32 + 1
        };
    }
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn unicode(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn property_table(message: bool, entries: Vec<(u32, [u8; 8])>) -> Vec<u8> {
    let mut output = vec![0u8; if message { 32 } else { 8 }];
    for (tag, value) in entries {
        output.extend_from_slice(&tag.to_le_bytes());
        output.extend_from_slice(&6u32.to_le_bytes());
        output.extend_from_slice(&value);
    }
    output
}

fn variable(tag: u32, bytes: &[u8]) -> (u32, [u8; 8]) {
    let mut value = [0u8; 8];
    value[..4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    (tag, value)
}

fn fixed_i32(tag: u32, value: i32) -> (u32, [u8; 8]) {
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(&value.to_le_bytes());
    (tag, bytes)
}

fn fixed_u64(tag: u32, value: u64) -> (u32, [u8; 8]) {
    (tag, value.to_le_bytes())
}

fn rtf_mela(value: &[u8], corrupt_crc: bool) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(&((value.len() + 12) as u32).to_le_bytes());
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(&0x414c_454du32.to_le_bytes());
    let crc = crc32(value) ^ u32::from(corrupt_crc);
    output.extend_from_slice(&crc.to_le_bytes());
    output.extend_from_slice(value);
    output
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn recipient_node() -> Node {
    let name = unicode("Recipient");
    let email = unicode("reader@example.test");
    let properties = property_table(
        false,
        vec![
            variable(0x3001_001f, &name),
            variable(0x39fe_001f, &email),
            fixed_i32(0x0c15_0003, 1),
        ],
    );
    Node::Storage(
        "__recip_version1.0_#00000000",
        vec![
            Node::Stream("__properties_version1.0", properties),
            Node::Stream("__substg1.0_3001001F", name),
            Node::Stream("__substg1.0_39FE001F", email),
        ],
    )
}

fn by_value_attachment() -> Node {
    let filename = unicode("payload.exe");
    let mime = unicode("application/x-msdownload");
    let data = vec![b'M', b'Z', 0x90, 0, b'i', b'n', b'e', b'r', b't'];
    let properties = property_table(
        false,
        vec![
            fixed_i32(0x3705_0003, 1),
            variable(0x3707_001f, &filename),
            variable(0x370e_001f, &mime),
            variable(0x3701_0102, &data),
        ],
    );
    Node::Storage(
        "__attach_version1.0_#00000000",
        vec![
            Node::Stream("__properties_version1.0", properties),
            Node::Stream("__substg1.0_3707001F", filename),
            Node::Stream("__substg1.0_370E001F", mime),
            Node::Stream("__substg1.0_37010102", data),
        ],
    )
}

fn embedded_attachment() -> Node {
    let subject = unicode("Embedded subject");
    let body = unicode("Embedded body");
    let embedded_properties = property_table(
        true,
        vec![
            variable(0x0037_001f, &subject),
            variable(0x1000_001f, &body),
        ],
    );
    let embedded_properties = embedded_properties[8..].to_vec();
    let attachment_properties = property_table(
        false,
        vec![fixed_i32(0x3705_0003, 5), (0x3701_000d, [0; 8])],
    );
    Node::Storage(
        "__attach_version1.0_#00000001",
        vec![
            Node::Stream("__properties_version1.0", attachment_properties),
            Node::Storage(
                "__substg1.0_3701000D",
                vec![
                    Node::Stream("__properties_version1.0", embedded_properties),
                    Node::Stream("__substg1.0_0037001F", subject),
                    Node::Stream("__substg1.0_1000001F", body),
                ],
            ),
        ],
    )
}

fn named_property_node() -> Node {
    let name = unicode("CustomName");
    let mut string_stream = Vec::new();
    string_stream.extend_from_slice(&(name.len() as u32).to_le_bytes());
    string_stream.extend_from_slice(&name);
    let mut entry = Vec::new();
    entry.extend_from_slice(&0u32.to_le_bytes());
    entry.extend_from_slice(&3u16.to_le_bytes());
    entry.extend_from_slice(&0u16.to_le_bytes());
    Node::Storage(
        "__nameid_version1.0",
        vec![
            Node::Stream("__substg1.0_00030102", entry),
            Node::Stream("__substg1.0_00040102", string_stream),
        ],
    )
}

fn fixture(encrypted: bool, corrupt_rtf_crc: bool) -> Vec<u8> {
    let subject = unicode("Quarterly update");
    let message_class = unicode(if encrypted {
        "IPM.Note.SMIME"
    } else {
        "IPM.Note"
    });
    let plain = unicode("Plain body");
    let html = b"<html><img src=https://example.test/pixel>HTML body</html>".to_vec();
    let rtf = rtf_mela(b"{\\rtf1 inert RTF body}", corrupt_rtf_crc);
    let message_id = unicode("<message@example.test>");
    let reply = unicode("<parent@example.test>");
    let named_value = unicode("named value");
    let properties = property_table(
        true,
        vec![
            variable(0x0037_001f, &subject),
            variable(0x001a_001f, &message_class),
            variable(0x1000_001f, &plain),
            variable(0x1013_0102, &html),
            variable(0x1009_0102, &rtf),
            variable(0x1035_001f, &message_id),
            variable(0x1042_001f, &reply),
            variable(0x8000_001f, &named_value),
            fixed_i32(0x3ffd_0003, 1252),
            fixed_u64(0x0039_0040, 132_537_600_000_000_000),
            (0x6666_0777, *b"UNKNOWN!"),
        ],
    );
    cfb(vec![
        Node::Stream("__properties_version1.0", properties),
        Node::Stream("__substg1.0_0037001F", subject),
        Node::Stream("__substg1.0_001A001F", message_class),
        Node::Stream("__substg1.0_1000001F", plain),
        Node::Stream("__substg1.0_10130102", html),
        Node::Stream("__substg1.0_10090102", rtf),
        Node::Stream("__substg1.0_1035001F", message_id),
        Node::Stream("__substg1.0_1042001F", reply),
        Node::Stream("__substg1.0_8000001F", named_value),
        named_property_node(),
        recipient_node(),
        by_value_attachment(),
        embedded_attachment(),
        Node::Stream("__custom_mapi_unknown", b"opaque vendor bytes".to_vec()),
    ])
}

#[test]
fn msg_detection_properties_recipients_bodies_attachments_graph_segments_and_schemas() {
    let bytes = fixture(false, false);
    let registry = builtin_parser_registry().unwrap();
    let detection = detect_with_registry(
        std::path::Path::new("extensionless"),
        &bytes,
        None,
        None,
        &grist::core::Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Msg);
    assert_eq!(detection.candidates[0].identity.format, "msg");
    assert!(matches!(
        registry.select_format("msg"),
        ParserSelection::Available(_)
    ));

    let envelope = parse_outlook_msg(
        &bytes,
        SourceInfo::new("message.msg"),
        &OutlookMsgOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let identity = envelope.identity.as_ref().unwrap();
    let document = envelope.payload.as_ref().unwrap();
    assert_eq!(document.subject.as_ref().unwrap().text, "Quarterly update");
    assert_eq!(document.recipients.len(), 1);
    assert_eq!(
        document.recipients[0].smtp_address.as_ref().unwrap().text,
        "reader@example.test"
    );
    assert_eq!(document.bodies.len(), 3);
    assert!(
        document
            .bodies
            .iter()
            .any(|body| body.kind == MsgBodyKind::Html
                && body.text.contains("https://example.test/pixel")
                && body.active_content_inert)
    );
    assert!(document.bodies.iter().any(|body| {
        body.kind == MsgBodyKind::Rtf
            && body
                .rtf_compression
                .as_ref()
                .is_some_and(|compression| compression.crc_matches)
    }));
    assert_eq!(document.attachments.len(), 2);
    let executable = document.attachments[0].artifact.as_ref().unwrap();
    assert_eq!(
        executable.safety.classification,
        ArtifactSafetyClassification::Executable
    );
    assert_eq!(
        executable.extraction.status,
        ArtifactExtractionStatus::InventoryOnly
    );
    assert!(executable.content.is_none());
    assert_eq!(
        document.attachments[1]
            .embedded_message
            .as_ref()
            .unwrap()
            .subject
            .as_ref()
            .unwrap()
            .text,
        "Embedded subject"
    );
    assert_eq!(document.named_properties.len(), 1);
    assert!(document.properties.iter().any(|property| {
        property.property_tag == "0x66660777" && matches!(property.value, MapiValue::Unknown { .. })
    }));
    assert_eq!(document.unknown_objects.len(), 1);
    let subject_locator = &document.subject.as_ref().unwrap().locator;
    assert!(matches!(
        &subject_locator.components()[0],
        LocationComponent::ArchiveMember { member_path, .. }
            if member_path == "__substg1.0_0037001F"
    ));
    assert!(matches!(
        &subject_locator.components()[1],
        LocationComponent::ByteRange { byte_start: 0, .. }
    ));

    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:msg"))
        .unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Email)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Attachment)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| { edge.relation == DocumentRelation::AlternativeRepresentationOf })
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::ReplyTo)
    );
    assert!(graph.nodes.iter().any(|node| {
        node.kind == DocumentNodeKind::Metadata && node.attrs.contains_key("table_value")
    }));
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| { !segment.node_ids.is_empty() && !segment.locators.is_empty() })
    );
    for name in ["outlook-msg", "outlook-msg-envelope", "outlook-msg-options"] {
        assert!(grist::schema::schema_json(name).is_some(), "missing {name}");
    }
}

#[test]
fn malformed_rtf_encrypted_budget_and_determinism_are_explicit() {
    let corrupt = fixture(false, true);
    let partial = parse_outlook_msg(
        &corrupt,
        SourceInfo::new("corrupt-rtf.msg"),
        &OutlookMsgOptions::default(),
    );
    assert_eq!(partial.status, OperationStatus::Partial);
    assert!(
        partial
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "outlook.msg.rtf.crc_mismatch" })
    );

    let encrypted = parse_outlook_msg(
        &fixture(true, false),
        SourceInfo::new("encrypted.msg"),
        &OutlookMsgOptions::default(),
    );
    assert_eq!(encrypted.status, OperationStatus::Partial);
    let encrypted_document = encrypted.payload.expect("encrypted metadata payload");
    assert!(encrypted_document.encrypted);
    assert_eq!(
        encrypted_document
            .subject
            .as_ref()
            .map(|subject| subject.text.as_str()),
        Some("Quarterly update")
    );

    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_child_artifacts = Some(1);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let budgeted = parse_outlook_msg_with_operation_control(
        &fixture(false, false),
        SourceInfo::new("budget.msg"),
        &OutlookMsgOptions::default(),
        &control,
    );
    assert_eq!(budgeted.status, OperationStatus::Partial);
    assert_eq!(control.usage().child_artifacts, 2);

    let malformed = parse_outlook_msg(
        &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1],
        SourceInfo::new("truncated.msg"),
        &OutlookMsgOptions::default(),
    );
    assert_eq!(malformed.status, OperationStatus::Failed);

    let first = parse_outlook_msg(
        &fixture(false, false),
        SourceInfo::new("stable.msg"),
        &OutlookMsgOptions::default(),
    );
    let second = parse_outlook_msg(
        &fixture(false, false),
        SourceInfo::new("stable.msg"),
        &OutlookMsgOptions::default(),
    );
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_parses_msg_through_the_registered_surface() {
    use std::fs;
    use std::process::Command;

    let path = std::env::temp_dir().join(format!("grist-outlook-msg-{}.msg", std::process::id()));
    fs::write(&path, fixture(false, false)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "msg", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["kind"], "outlook_msg");
    assert_eq!(value["payload"]["subject"]["text"], "Quarterly update");
    assert_eq!(value["payload"]["attachments"].as_array().unwrap().len(), 2);
}
