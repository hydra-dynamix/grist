//! Typed, loss-aware CBOR, MessagePack, and descriptor-driven Protocol Buffers.

mod cbor;
mod descriptor;
mod graph;
mod messagepack;
mod model;
mod protobuf;
mod wire;

pub use model::*;

use crate::core::{
    ArtifactKind, Diagnostic, DiagnosticCode, Envelope, FormatIdentity, Hashes, OperationControl,
    OperationKind, OperationStatus, ParserInfo, SchemaVersion, SourceInfo,
};
use crate::detect::ContentKind;
use model::hex;
use wire::{DecodeError, DecodeState, byte_locator, locator};

const PARSER: &str = "grist.structured-binary";

pub type StructuredBinaryEnvelope = Envelope<StructuredBinaryDocument>;

pub fn parse_cbor(bytes: &[u8], source: SourceInfo) -> StructuredBinaryEnvelope {
    parse_structured_binary(
        bytes,
        StructuredBinaryFormat::Cbor,
        source,
        &StructuredBinaryOptions::default(),
    )
}

pub fn parse_messagepack(bytes: &[u8], source: SourceInfo) -> StructuredBinaryEnvelope {
    parse_structured_binary(
        bytes,
        StructuredBinaryFormat::MessagePack,
        source,
        &StructuredBinaryOptions::default(),
    )
}

pub fn parse_protobuf(
    bytes: &[u8],
    source: SourceInfo,
    protobuf: ProtobufDecodeOptions,
) -> StructuredBinaryEnvelope {
    parse_structured_binary(
        bytes,
        StructuredBinaryFormat::Protobuf,
        source,
        &StructuredBinaryOptions {
            protobuf: Some(protobuf),
            ..Default::default()
        },
    )
}

pub fn parse_structured_binary(
    bytes: &[u8],
    format: StructuredBinaryFormat,
    source: SourceInfo,
    options: &StructuredBinaryOptions,
) -> StructuredBinaryEnvelope {
    parse_structured_binary_with_control(bytes, format, source, options, None)
}

pub fn parse_structured_binary_with_operation_control(
    bytes: &[u8],
    format: StructuredBinaryFormat,
    source: SourceInfo,
    options: &StructuredBinaryOptions,
    control: &OperationControl,
) -> StructuredBinaryEnvelope {
    parse_structured_binary_with_control(bytes, format, source, options, Some(control))
}

fn parse_structured_binary_with_control(
    bytes: &[u8],
    format: StructuredBinaryFormat,
    source: SourceInfo,
    options: &StructuredBinaryOptions,
    control: Option<&OperationControl>,
) -> StructuredBinaryEnvelope {
    let digest = crate::core::options_digest(options).expect("binary options serialize");
    let parser = parser_info(format);
    let mut state = DecodeState::new(format_name(format), bytes.len(), options, control);
    if let Some(control) = control
        && let Err(error) = control.checkpoint()
    {
        return terminal(
            bytes,
            source,
            parser,
            digest,
            error.operation_status(0),
            error.diagnostic(PARSER),
        );
    }

    let parsed = match format {
        StructuredBinaryFormat::Cbor => {
            cbor::parse_records(bytes, options, &mut state).map(|records| {
                let self_described = state.tags.iter().any(|tag| tag.tag == 55_799);
                (
                    records,
                    BinarySchemaIdentity::Cbor {
                        specification: "RFC 8949".to_string(),
                        self_described,
                    },
                )
            })
        }
        StructuredBinaryFormat::MessagePack => {
            messagepack::parse_records(bytes, options, &mut state).map(|records| {
                (
                    records,
                    BinarySchemaIdentity::MessagePack {
                        specification: "MessagePack specification v5".to_string(),
                    },
                )
            })
        }
        StructuredBinaryFormat::Protobuf => parse_protobuf_records(bytes, options, &mut state),
    };
    let (records, schema_identity) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            if let Some(status) = state.terminal_status {
                let diagnostic = state
                    .diagnostics
                    .pop()
                    .unwrap_or_else(|| malformed_diagnostic(format, &error, bytes.len(), false));
                return terminal(bytes, source, parser, digest, status, diagnostic);
            }
            let descriptor_error = error.code.starts_with("protobuf.descriptor")
                || matches!(
                    error.code,
                    "protobuf.message_name_required" | "protobuf.message_not_found"
                );
            if options.malformed_recovery == BinaryMalformedRecovery::Strict || descriptor_error {
                return terminal(
                    bytes,
                    source,
                    parser,
                    digest,
                    OperationStatus::Failed,
                    malformed_diagnostic(format, &error, bytes.len(), false),
                );
            }
            state
                .diagnostics
                .push(malformed_diagnostic(format, &error, bytes.len(), true));
            (
                vec![raw_record(bytes, format, &error)],
                fallback_schema_identity(format),
            )
        }
    };

    if let Some(control) = control {
        let budget_result = control
            .budget()
            .consume_records(records.len() as u64)
            .and_then(|_| control.budget().consume_nodes(state.value_count as u64))
            .and_then(|_| {
                control
                    .budget()
                    .observe_nesting_depth(state.max_depth as u64)
            })
            .and_then(|_| {
                control
                    .budget()
                    .observe_memory_bytes(estimated_memory(&records) as u64)
            });
        if let Err(error) = budget_result {
            state.diagnostics.push(error.diagnostic(PARSER).partial());
        }
    }
    let mut diagnostics = state.diagnostics;
    if !state.unknown_fields.is_empty() {
        diagnostics.push(Diagnostic::warning(
            PARSER,
            "protobuf.unknown_fields_preserved",
            format!(
                "{} unknown Protobuf field occurrence(s) were preserved losslessly",
                state.unknown_fields.len()
            ),
        ));
    }
    let partial = diagnostics.iter().any(|diagnostic| diagnostic.partial);
    let json_projection = match records.as_slice() {
        [] => None,
        [record] => Some(record.value.json_projection()),
        records => Some(serde_json::Value::Array(
            records
                .iter()
                .map(|record| record.value.json_projection())
                .collect(),
        )),
    };
    let document = StructuredBinaryDocument {
        schema_version: SchemaVersion::STRUCTURED_BINARY_V1.to_string(),
        format,
        schema_identity,
        records,
        tags: state.tags,
        extensions: state.extensions,
        unknown_fields: state.unknown_fields,
        diagnostics: diagnostics.clone(),
        complete: !partial,
        json_projection,
    };
    let envelope = if partial {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::StructuredBinary,
            source,
            parser,
            digest,
            SchemaVersion::STRUCTURED_BINARY_V1,
            Some(document),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::StructuredBinary,
            source,
            parser,
            digest,
            SchemaVersion::STRUCTURED_BINARY_V1,
            document,
        )
    }
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(diagnostics);
    envelope
        .with_canonical_payload_identity()
        .expect("structured-binary payload serializes")
}

fn parse_protobuf_records(
    bytes: &[u8],
    options: &StructuredBinaryOptions,
    state: &mut DecodeState<'_>,
) -> Result<(Vec<BinaryRecord>, BinarySchemaIdentity), DecodeError> {
    let options = options.protobuf.as_ref().ok_or_else(|| {
        DecodeError::new(
            "protobuf.descriptor_required",
            "Protobuf decoding requires descriptor_set bytes and message_name",
            0,
        )
    })?;
    let pool = descriptor::parse_descriptor_set(&options.descriptor_set)?;
    let normalized = options.message_name.trim().trim_start_matches('.');
    let message = pool.messages.get(normalized).ok_or_else(|| {
        DecodeError::new(
            "protobuf.message_not_found",
            format!("message {normalized} is not present in the descriptor set"),
            0,
        )
    })?;
    let record = protobuf::parse_record(
        bytes,
        &pool,
        normalized,
        options.preserve_unknown_fields,
        state,
    )?;
    Ok((
        vec![record],
        BinarySchemaIdentity::ProtobufDescriptor {
            descriptor_sha256: crate::core::sha256_hex(&options.descriptor_set),
            descriptor_size: options.descriptor_set.len(),
            message_name: normalized.to_string(),
            syntax: message.syntax,
            files: pool.files,
        },
    ))
}

fn malformed_diagnostic(
    format: StructuredBinaryFormat,
    error: &DecodeError,
    input_len: usize,
    partial: bool,
) -> Diagnostic {
    let is_limit = error.code.ends_with("_limit");
    let mut diagnostic = if is_limit {
        Diagnostic::budget_exhausted(PARSER, error.message.clone())
    } else {
        Diagnostic::malformed(PARSER, error.message.clone())
    };
    diagnostic.code = DiagnosticCode::new(error.code);
    diagnostic.module = format!("grist.structured-binary.{}", format_name(format));
    diagnostic.locator = Some(Box::new(byte_locator(
        error.offset.min(input_len),
        error.offset.saturating_add(1).min(input_len),
    )));
    if partial {
        diagnostic = diagnostic.partial();
    }
    diagnostic
}

fn terminal(
    bytes: &[u8],
    source: SourceInfo,
    parser: ParserInfo,
    digest: String,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> StructuredBinaryEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::StructuredBinary,
        status,
        source,
        parser,
        digest,
        SchemaVersion::STRUCTURED_BINARY_V1,
    )
    .expect("terminal structured-binary status is valid")
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(vec![diagnostic])
}

fn raw_record(bytes: &[u8], format: StructuredBinaryFormat, error: &DecodeError) -> BinaryRecord {
    let collection = format!("{}-sequence", format_name(format));
    let value = BinaryValue {
        id: format!("{}:/raw@0", format_name(format)),
        kind: BinaryValueKind::Unknown,
        path: "/raw".to_string(),
        byte_start: 0,
        byte_end: bytes.len(),
        locator: locator(&collection, 1, 0, 0, bytes.len(), Some("/raw".to_string())),
        scalar: Some(BinaryScalar::Bytes {
            hex: hex(bytes),
            length: bytes.len(),
        }),
        entries: Vec::new(),
        items: Vec::new(),
        cbor_tag: None,
        messagepack_extension: None,
        protobuf_field: None,
        indefinite: false,
        recovered: true,
    };
    let _ = error;
    BinaryRecord {
        index: 1,
        byte_start: 0,
        byte_end: bytes.len(),
        locator: locator(&collection, 1, 0, 0, bytes.len(), None),
        value,
    }
}

fn fallback_schema_identity(format: StructuredBinaryFormat) -> BinarySchemaIdentity {
    match format {
        StructuredBinaryFormat::Cbor => BinarySchemaIdentity::Cbor {
            specification: "RFC 8949".to_string(),
            self_described: false,
        },
        StructuredBinaryFormat::MessagePack => BinarySchemaIdentity::MessagePack {
            specification: "MessagePack specification v5".to_string(),
        },
        StructuredBinaryFormat::Protobuf => BinarySchemaIdentity::ProtobufDescriptor {
            descriptor_sha256: "sha256:unavailable".to_string(),
            descriptor_size: 0,
            message_name: "<unavailable>".to_string(),
            syntax: ProtobufSyntax::Unknown,
            files: Vec::new(),
        },
    }
}

fn estimated_memory(records: &[BinaryRecord]) -> usize {
    records
        .iter()
        .map(|record| record.byte_end.saturating_sub(record.byte_start))
        .sum::<usize>()
        .saturating_mul(4)
}

pub fn parser_info(format: StructuredBinaryFormat) -> ParserInfo {
    let (implementation, specification) = match format {
        StructuredBinaryFormat::Cbor => ("grist-cbor", "RFC 8949"),
        StructuredBinaryFormat::MessagePack => ("grist-messagepack", "MessagePack v5"),
        StructuredBinaryFormat::Protobuf => (
            "grist-protobuf-descriptor",
            "Protocol Buffers descriptor.proto / wire format",
        ),
    };
    ParserInfo::new(PARSER)
        .with_implementation(implementation, env!("CARGO_PKG_VERSION"))
        .with_specification_version(specification)
        .with_feature("structured-binary")
}

pub const fn format_name(format: StructuredBinaryFormat) -> &'static str {
    match format {
        StructuredBinaryFormat::Cbor => "cbor",
        StructuredBinaryFormat::MessagePack => "messagepack",
        StructuredBinaryFormat::Protobuf => "protobuf",
    }
}

pub const fn media_type(format: StructuredBinaryFormat) -> &'static str {
    match format {
        StructuredBinaryFormat::Cbor => "application/cbor",
        StructuredBinaryFormat::MessagePack => "application/msgpack",
        StructuredBinaryFormat::Protobuf => "application/x-protobuf",
    }
}

pub fn format_from_content_kind(kind: &ContentKind) -> Option<StructuredBinaryFormat> {
    match kind {
        ContentKind::Cbor => Some(StructuredBinaryFormat::Cbor),
        ContentKind::MessagePack => Some(StructuredBinaryFormat::MessagePack),
        ContentKind::Protobuf => Some(StructuredBinaryFormat::Protobuf),
        _ => None,
    }
}

pub fn identity_for_format(format: StructuredBinaryFormat) -> FormatIdentity {
    FormatIdentity::new(format_name(format), Some(media_type(format)))
}

pub(crate) fn probe_formats(bytes: &[u8]) -> Vec<StructuredBinaryFormat> {
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return Vec::new();
    }
    let options = StructuredBinaryOptions {
        max_nesting_depth: 32,
        max_values: 4096,
        max_collection_items: 4096,
        max_blob_bytes: 1024 * 1024,
        allow_sequence: false,
        malformed_recovery: BinaryMalformedRecovery::Strict,
        protobuf: None,
    };
    let mut cbor_state = DecodeState::new("cbor", bytes.len(), &options, None);
    let cbor = cbor::parse_records(bytes, &options, &mut cbor_state).is_ok();
    let mut messagepack_state = DecodeState::new("messagepack", bytes.len(), &options, None);
    let messagepack = messagepack::parse_records(bytes, &options, &mut messagepack_state).is_ok();
    let mut formats = Vec::new();
    match (cbor, messagepack) {
        (true, false) => formats.push(StructuredBinaryFormat::Cbor),
        (false, true) => formats.push(StructuredBinaryFormat::MessagePack),
        _ => {}
    }
    if !cbor && !messagepack && probe_protobuf_wire(bytes) {
        formats.push(StructuredBinaryFormat::Protobuf);
    }
    formats
}

fn probe_protobuf_wire(bytes: &[u8]) -> bool {
    fn skip(cursor: &mut wire::Cursor<'_>, wire_type: u8, field: u32, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        match wire_type {
            0 => cursor.read_varint("probe").is_ok(),
            1 => cursor.take(8, "probe").is_ok(),
            2 => {
                let Ok(length) = cursor.read_varint("probe") else {
                    return false;
                };
                usize::try_from(length)
                    .ok()
                    .is_some_and(|length| cursor.take(length, "probe").is_ok())
            }
            3 => loop {
                let Ok(key) = cursor.read_varint("probe") else {
                    return false;
                };
                let Ok(number) = u32::try_from(key >> 3) else {
                    return false;
                };
                let nested_wire = (key & 7) as u8;
                if nested_wire == 4 {
                    break number == field;
                }
                if number == 0 || !skip(cursor, nested_wire, number, depth + 1) {
                    break false;
                }
            },
            5 => cursor.take(4, "probe").is_ok(),
            _ => false,
        }
    }
    let mut cursor = wire::Cursor::new(bytes);
    let mut fields = 0usize;
    while !cursor.is_empty() {
        let Ok(key) = cursor.read_varint("probe") else {
            return false;
        };
        let Ok(number) = u32::try_from(key >> 3) else {
            return false;
        };
        let wire_type = (key & 7) as u8;
        if number == 0
            || number > 536_870_911
            || (19_000..=19_999).contains(&number)
            || wire_type == 4
            || !skip(&mut cursor, wire_type, number, 1)
        {
            return false;
        }
        fields += 1;
        if fields > 4096 {
            return false;
        }
    }
    fields > 0
}
