//! Typed, projection-aware Apache Arrow IPC and Parquet parsing.
mod arrow;
mod binary;
mod flatbuffer;
mod graph;
mod model;
mod parquet;
mod thrift;
pub use model::*;

use crate::core::{
    ArtifactKind, Diagnostic, DiagnosticCode, Envelope, Hashes, OperationControl, OperationKind,
    OperationStatus, ParserInfo, SchemaVersion, SourceInfo,
};
use crate::detect::ContentKind;
use std::collections::BTreeSet;

const PARSER: &str = "grist.columnar";
pub type ColumnarEnvelope = Envelope<ColumnarDocument>;

pub fn parse_arrow_ipc(
    bytes: &[u8],
    source: SourceInfo,
    options: &ColumnarOptions,
) -> ColumnarEnvelope {
    parse(bytes, None, source, options, None)
}
pub fn parse_parquet(
    bytes: &[u8],
    source: SourceInfo,
    options: &ColumnarOptions,
) -> ColumnarEnvelope {
    parse(bytes, Some(ColumnarFormat::Parquet), source, options, None)
}
pub fn parse_columnar(
    bytes: &[u8],
    source: SourceInfo,
    options: &ColumnarOptions,
) -> ColumnarEnvelope {
    parse(bytes, None, source, options, None)
}
pub fn parse_columnar_with_operation_control(
    bytes: &[u8],
    format: Option<ColumnarFormat>,
    source: SourceInfo,
    options: &ColumnarOptions,
    control: &OperationControl,
) -> ColumnarEnvelope {
    parse(bytes, format, source, options, Some(control))
}

fn parse(
    bytes: &[u8],
    hint: Option<ColumnarFormat>,
    source: SourceInfo,
    options: &ColumnarOptions,
    control: Option<&OperationControl>,
) -> ColumnarEnvelope {
    let digest = crate::core::options_digest(options).expect("columnar options serialize");
    let format = hint.unwrap_or_else(|| {
        if bytes.starts_with(b"PAR1") {
            ColumnarFormat::Parquet
        } else if bytes.starts_with(b"ARROW1") {
            ColumnarFormat::ArrowIpcFile
        } else {
            ColumnarFormat::ArrowIpcStream
        }
    });
    let parser = parser_info(format);
    if let Some(control) = control {
        if let Err(error) = control.budget().consume_input_bytes(bytes.len() as u64) {
            return terminal(
                bytes,
                source,
                parser,
                digest,
                error.operation_status(0),
                error.diagnostic(PARSER),
            );
        }
    }
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
        ColumnarFormat::Parquet => parquet::parse(bytes, options)
            .map(|p| (p.version, p.schema, p.metadata, p.batches, p.dictionaries)),
        ColumnarFormat::ArrowIpcFile | ColumnarFormat::ArrowIpcStream => {
            arrow::parse(bytes, options).and_then(|p| {
                if p.format == format || hint.is_none() {
                    Ok((p.version, p.schema, p.metadata, p.batches, p.dictionaries))
                } else {
                    Err(binary::DecodeError::new(
                        "arrow.container_mismatch",
                        format!("expected {format:?}, decoded {:?}", p.format),
                        0,
                    ))
                }
            })
        }
    };
    let (version, schema, metadata, batches, dictionaries) = match parsed {
        Ok(value) => value,
        Err(error) => {
            let mut diagnostic = if error.code.contains("unsupported") {
                Diagnostic::unsupported(PARSER, error.message)
            } else {
                Diagnostic::malformed(PARSER, error.message)
            };
            diagnostic.code = DiagnosticCode::new(error.code);
            diagnostic.locator = Some(Box::new(binary::byte_locator(
                error.offset.min(bytes.len()),
                error.offset.saturating_add(1).min(bytes.len()),
            )));
            return terminal(
                bytes,
                source,
                parser,
                digest,
                OperationStatus::Failed,
                diagnostic,
            );
        }
    };
    let mut diagnostics = Vec::new();
    let projected_rows = batches
        .iter()
        .flat_map(|batch| batch.columns.iter())
        .flat_map(|column| column.values.iter())
        .map(|value| value.row)
        .collect::<BTreeSet<_>>()
        .len();
    if let Some(control) = control {
        let cells = batches
            .iter()
            .flat_map(|batch| &batch.columns)
            .map(|column| column.values.len() as u64)
            .sum();
        let result = control
            .budget()
            .consume_records(projected_rows as u64)
            .and_then(|_| control.budget().consume_cells(cells))
            .and_then(|_| {
                control
                    .budget()
                    .consume_nodes(cells + projected_rows as u64 + batches.len() as u64)
            })
            .and_then(|_| {
                control
                    .budget()
                    .observe_memory_bytes(estimated_memory(&batches, &dictionaries) as u64)
            });
        if let Err(error) = result {
            diagnostics.push(error.diagnostic(PARSER).partial());
        }
    }
    let complete = diagnostics.is_empty();
    let document = ColumnarDocument {
        schema_version: SchemaVersion::COLUMNAR_V1.into(),
        format,
        format_version: version,
        schema,
        metadata,
        batches,
        dictionaries,
        diagnostics: diagnostics.clone(),
        complete,
        projected_rows,
    };
    let envelope = if complete {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Columnar,
            source,
            parser,
            digest,
            SchemaVersion::COLUMNAR_V1,
            document,
        )
    } else {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Columnar,
            source,
            parser,
            digest,
            SchemaVersion::COLUMNAR_V1,
            Some(document),
        )
    }
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(diagnostics);
    envelope
        .with_canonical_payload_identity()
        .expect("columnar payload serializes")
}
fn terminal(
    bytes: &[u8],
    source: SourceInfo,
    parser: ParserInfo,
    digest: String,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> ColumnarEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Columnar,
        status,
        source,
        parser,
        digest,
        SchemaVersion::COLUMNAR_V1,
    )
    .expect("valid terminal envelope")
    .with_hashes(Hashes::for_bytes(bytes, None))
    .with_diagnostics(vec![diagnostic])
}
fn estimated_memory(batches: &[ColumnarBatch], dictionaries: &[ColumnarDictionary]) -> usize {
    batches
        .iter()
        .flat_map(|batch| &batch.columns)
        .map(|column| column.values.len() * std::mem::size_of::<ColumnarCell>())
        .sum::<usize>()
        + dictionaries
            .iter()
            .map(|dictionary| dictionary.values.len() * std::mem::size_of::<ColumnarValue>())
            .sum::<usize>()
}
pub fn parser_info(format: ColumnarFormat) -> ParserInfo {
    let identity = match format {
        ColumnarFormat::ArrowIpcFile => "arrow-ipc-file",
        ColumnarFormat::ArrowIpcStream => "arrow-ipc-stream",
        ColumnarFormat::Parquet => "parquet",
    };
    ParserInfo::new(PARSER)
        .with_implementation(identity, env!("CARGO_PKG_VERSION"))
        .with_specification_version(match format {
            ColumnarFormat::ArrowIpcFile | ColumnarFormat::ArrowIpcStream => "Arrow IPC",
            ColumnarFormat::Parquet => "Parquet",
        })
        .with_feature("columnar")
}
pub fn probe(bytes: &[u8]) -> Vec<ColumnarFormat> {
    let mut output = Vec::new();
    if bytes.len() >= 10 && bytes.starts_with(b"ARROW1") && bytes.ends_with(b"ARROW1") {
        output.push(ColumnarFormat::ArrowIpcFile);
    } else if arrow::is_arrow(bytes) {
        output.push(ColumnarFormat::ArrowIpcStream);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"PAR1") && bytes.ends_with(b"PAR1") {
        output.push(ColumnarFormat::Parquet);
    }
    output
}
pub fn content_kind(format: ColumnarFormat) -> ContentKind {
    match format {
        ColumnarFormat::ArrowIpcFile | ColumnarFormat::ArrowIpcStream => ContentKind::Arrow,
        ColumnarFormat::Parquet => ContentKind::Parquet,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[cfg_attr(feature = "schemas", derive(schemars::JsonSchema))]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ColumnarStreamEvent {
    Schema {
        format: ColumnarFormat,
        schema: ColumnarSchema,
    },
    Dictionary {
        dictionary: ColumnarDictionary,
    },
    Batch {
        batch: ColumnarBatch,
    },
    Diagnostic {
        diagnostic: Box<Diagnostic>,
    },
    End {
        complete: bool,
    },
}
pub fn parse_columnar_events(
    bytes: &[u8],
    source: SourceInfo,
    options: &ColumnarOptions,
) -> std::vec::IntoIter<ColumnarStreamEvent> {
    let envelope = parse_columnar(bytes, source, options);
    let mut events = Vec::new();
    if let Some(document) = envelope.payload {
        events.push(ColumnarStreamEvent::Schema {
            format: document.format,
            schema: document.schema,
        });
        events.extend(
            document
                .dictionaries
                .into_iter()
                .map(|dictionary| ColumnarStreamEvent::Dictionary { dictionary }),
        );
        events.extend(
            document
                .batches
                .into_iter()
                .map(|batch| ColumnarStreamEvent::Batch { batch }),
        );
        events.extend(document.diagnostics.into_iter().map(|diagnostic| {
            ColumnarStreamEvent::Diagnostic {
                diagnostic: Box::new(diagnostic),
            }
        }));
        events.push(ColumnarStreamEvent::End {
            complete: document.complete,
        });
    } else {
        events.extend(envelope.diagnostics.into_iter().map(|diagnostic| {
            ColumnarStreamEvent::Diagnostic {
                diagnostic: Box::new(diagnostic),
            }
        }));
        events.push(ColumnarStreamEvent::End { complete: false });
    }
    events.into_iter()
}
