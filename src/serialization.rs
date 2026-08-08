//! Loss-aware typed parsing for JSON, JSONL/NDJSON, YAML, TOML, and generic XML.

mod json;
mod model;
mod toml;
mod xml;
mod yaml;

pub use model::*;

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, DiagnosticCode, Envelope, FormatIdentity, Hashes,
    LineIndex, OperationControl, OperationKind, OperationStatus, ParserInfo, RecoveryAction,
    RecoveryKind, RequestId, SchemaVersion, SourceInfo, SourceLocator, SourceRange, StreamEvent,
    StreamItem, StreamTerminal,
};
use crate::detect::ContentKind;
use serde_json::Value;

const PARSER: &str = "grist.structured-text";

pub type SerializationEnvelope = Envelope<SerializationPayload>;
pub type StructuredRecordStreamEvent = StreamEvent<StructuredRecord>;

pub fn parse_serialization(
    text: &str,
    format: SerializationFormat,
    source: SourceInfo,
) -> SerializationEnvelope {
    parse_serialization_with_options(text, format, source, &SerializationOptions::default())
}

pub fn parse_serialization_with_options(
    text: &str,
    format: SerializationFormat,
    source: SourceInfo,
    options: &SerializationOptions,
) -> SerializationEnvelope {
    parse_impl(text, format, source, options, None)
}

pub fn parse_serialization_with_control(
    text: &str,
    format: SerializationFormat,
    source: SourceInfo,
    options: &SerializationOptions,
    control: &OperationControl,
) -> SerializationEnvelope {
    parse_impl(text, format, source, options, Some(control))
}

fn parse_impl(
    text: &str,
    format: SerializationFormat,
    source: SourceInfo,
    options: &SerializationOptions,
    control: Option<&OperationControl>,
) -> SerializationEnvelope {
    let operation = if options.schema.is_some() {
        OperationKind::Validate
    } else {
        OperationKind::Parse
    };
    let digest = crate::core::options_digest(options).expect("structured-text options serialize");
    let parser = parser_info(format);
    if let Some(control) = control
        && let Err(error) = control.checkpoint()
    {
        return Envelope::without_payload(
            operation,
            ArtifactKind::Serialization,
            error.operation_status(0),
            source,
            parser,
            digest,
            SchemaVersion::STRUCTURED_TEXT_V2,
        )
        .expect("controlled terminal status is valid")
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(vec![error.diagnostic(PARSER)]);
    }

    let observed_depth = lexical_nesting_depth(text, format);
    let shared_depth_limit = control
        .and_then(|control| control.budget().budget().max_nesting_depth)
        .and_then(|value| usize::try_from(value).ok());
    let depth_limit = shared_depth_limit.map_or(options.max_nesting_depth, |limit| {
        limit.min(options.max_nesting_depth)
    });
    if observed_depth > depth_limit {
        if let Some(control) = control {
            let _ = control
                .budget()
                .observe_nesting_depth(observed_depth as u64);
        }
        let mut diagnostic = Diagnostic::budget_exhausted(
            PARSER,
            format!(
                "structured-text nesting depth {observed_depth} exceeds the explicit limit {depth_limit}"
            ),
        );
        diagnostic.code = DiagnosticCode::new("structured.nesting_limit");
        return Envelope::without_payload(
            operation,
            ArtifactKind::Serialization,
            OperationStatus::Failed,
            source,
            parser,
            digest,
            SchemaVersion::STRUCTURED_TEXT_V2,
        )
        .expect("failed envelope status is valid")
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(vec![diagnostic]);
    }

    let mut diagnostics = Vec::new();
    let mut documents = Vec::new();
    let mut records = Vec::new();
    let mut duplicates = Vec::new();
    let mut aliases = Vec::new();
    let mut raw_unknowns = Vec::new();
    let mut node_count = 0usize;
    let mut max_depth = 0usize;
    let mut forced_status = None;

    match format {
        SerializationFormat::Json => match json::parse(text, 0, text.len(), None) {
            Ok(parsed) => {
                documents.push(parsed.value);
                duplicates = parsed.duplicate_keys;
                node_count = parsed.node_count;
                max_depth = parsed.max_depth;
            }
            Err(error) => recover_or_fail(
                text,
                "json.parse",
                error.offset,
                error.message,
                options,
                &mut documents,
                &mut raw_unknowns,
                &mut diagnostics,
                &mut forced_status,
            ),
        },
        SerializationFormat::Jsonl => {
            let parsed = parse_jsonl_records(text, options, control);
            records = parsed.records;
            duplicates = parsed.duplicates;
            raw_unknowns = parsed.raw_unknowns;
            diagnostics = parsed.diagnostics;
            node_count = parsed.node_count;
            max_depth = parsed.max_depth;
            forced_status = parsed.status;
            documents.extend(records.iter().filter_map(|record| record.value.clone()));
        }
        SerializationFormat::Yaml => match yaml::parse(text) {
            Ok(parsed) => {
                documents = parsed.documents;
                duplicates = parsed.duplicate_keys;
                aliases = parsed.aliases;
                node_count = parsed.node_count;
                max_depth = parsed.max_depth;
            }
            Err(error) => recover_or_fail(
                text,
                "yaml.parse",
                error.offset,
                error.message,
                options,
                &mut documents,
                &mut raw_unknowns,
                &mut diagnostics,
                &mut forced_status,
            ),
        },
        SerializationFormat::Toml => match toml::parse(text) {
            Ok(parsed) => {
                documents.push(parsed.value);
                node_count = parsed.node_count;
                max_depth = parsed.max_depth;
            }
            Err(error) => recover_or_fail_range(
                text,
                "toml.parse",
                error.range,
                error.message,
                options,
                &mut documents,
                &mut raw_unknowns,
                &mut diagnostics,
                &mut forced_status,
            ),
        },
        SerializationFormat::Xml => {
            let xml_envelope =
                crate::xml::parse_xml(text, source.clone(), &crate::xml::XmlOptions::default());
            diagnostics.extend(xml_envelope.diagnostics);
            forced_status = Some(xml_envelope.status);
            if let Some(xml_document) = xml_envelope.payload {
                let projected = xml::project(&xml_document);
                documents.push(projected.value);
                raw_unknowns = projected.raw_unknowns;
                node_count = projected.node_count;
                max_depth = projected.max_depth;
            }
        }
    }

    if max_depth > depth_limit {
        let mut diagnostic = Diagnostic::budget_exhausted(
            PARSER,
            format!("parsed nesting depth {max_depth} exceeds the explicit limit {depth_limit}"),
        );
        diagnostic.code = DiagnosticCode::new("structured.nesting_limit");
        diagnostics.push(diagnostic);
        forced_status = Some(if documents.is_empty() && records.is_empty() {
            OperationStatus::Failed
        } else {
            OperationStatus::Partial
        });
    }

    for duplicate in &duplicates {
        let partial = format == SerializationFormat::Yaml;
        let mut diagnostic = Diagnostic::warning(
            PARSER,
            "structured.duplicate_key",
            format!(
                "duplicate key `{}` occurrence {} was retained in source order",
                duplicate.key, duplicate.occurrence
            ),
        )
        .with_locator(duplicate.duplicate_locator.clone())
        .with_explanation_key("structured.duplicate_key");
        if partial {
            diagnostic = diagnostic.partial();
        }
        diagnostics.push(diagnostic);
    }
    for alias in &aliases {
        if !alias.resolved {
            diagnostics.push(
                malformed(
                    "yaml.alias.unresolved",
                    format!("YAML alias `*{}` has no matching anchor", alias.name),
                    alias.locator.clone(),
                )
                .partial(),
            );
        }
    }

    if let Some(control) = control {
        let budget_result = control
            .budget()
            .consume_nodes(node_count as u64)
            .and_then(|_| control.budget().observe_nesting_depth(max_depth as u64));
        if let Err(error) = budget_result {
            diagnostics.push(error.diagnostic(PARSER));
            forced_status = Some(if documents.is_empty() && records.is_empty() {
                OperationStatus::Failed
            } else {
                OperationStatus::Partial
            });
        }
    }

    if forced_status == Some(OperationStatus::Failed) && documents.is_empty() && records.is_empty()
    {
        return Envelope::without_payload(
            operation,
            ArtifactKind::Serialization,
            OperationStatus::Failed,
            source,
            parser,
            digest,
            SchemaVersion::STRUCTURED_TEXT_V2,
        )
        .expect("failed envelope status is valid")
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(diagnostics);
    }

    let projected_values = documents
        .iter()
        .filter_map(|document| document.json_projection(options.duplicate_projection))
        .collect::<Vec<_>>();
    let value = match (format, projected_values.len()) {
        (_, 0) => None,
        (SerializationFormat::Jsonl, _) => Some(Value::Array(projected_values)),
        (_, 1) => projected_values.into_iter().next(),
        (_, _) => Some(Value::Array(projected_values)),
    };
    let jsonl_records = records
        .iter()
        .map(|record| JsonlRecord {
            line: record.source_line,
            value: record
                .value
                .as_ref()
                .and_then(|value| value.json_projection(options.duplicate_projection)),
            diagnostic: record.diagnostic.clone(),
        })
        .collect::<Vec<_>>();
    let mut validation = None;
    if let Some(schema) = &options.schema {
        let validation_diagnostics = value.as_ref().map_or_else(
            || {
                vec![Diagnostic::error(
                    "grist.structured-text.schema",
                    "schema.projection_unavailable",
                    "schema validation requires a JSON-compatible projection",
                )]
            },
            |value| validate_json_schema(value, schema),
        );
        validation = Some(SchemaValidationResult {
            valid: validation_diagnostics.is_empty(),
            diagnostics: validation_diagnostics.clone(),
        });
        diagnostics.extend(validation_diagnostics);
    }
    let mut status = forced_status.unwrap_or(OperationStatus::Complete);
    if diagnostics.iter().any(|diagnostic| diagnostic.partial)
        && status == OperationStatus::Complete
    {
        status = OperationStatus::Partial;
    }
    let payload = StructuredTextDocument {
        schema_version: SchemaVersion::STRUCTURED_TEXT_V2.into(),
        format,
        documents,
        records,
        ordering: StructuredOrdering::Source,
        duplicate_keys: duplicates,
        aliases,
        raw_unknowns,
        validation,
        complete: status == OperationStatus::Complete,
        value,
        jsonl_records,
    };
    let envelope = match status {
        OperationStatus::Complete => Envelope::complete(
            operation,
            ArtifactKind::Serialization,
            source,
            parser,
            digest,
            SchemaVersion::STRUCTURED_TEXT_V2,
            payload,
        ),
        OperationStatus::Partial => Envelope::partial(
            operation,
            ArtifactKind::Serialization,
            source,
            parser,
            digest,
            SchemaVersion::STRUCTURED_TEXT_V2,
            Some(payload),
        ),
        terminal => Envelope::without_payload(
            operation,
            ArtifactKind::Serialization,
            terminal,
            source,
            parser,
            digest,
            SchemaVersion::STRUCTURED_TEXT_V2,
        )
        .expect("terminal status is valid"),
    };
    envelope
        .with_identity(
            ContentIdentity::for_raw_bytes(text.as_bytes())
                .with_decoded(text, "utf-8", false)
                .with_format(FormatIdentity::new(format_name(format), media_type(format))),
        )
        .with_canonical_payload_identity()
        .expect("structured-text payload canonicalizes")
        .with_diagnostics(diagnostics)
}

struct ParsedRecords {
    records: Vec<StructuredRecord>,
    duplicates: Vec<DuplicateKey>,
    raw_unknowns: Vec<RawStructuredUnknown>,
    diagnostics: Vec<Diagnostic>,
    node_count: usize,
    max_depth: usize,
    status: Option<OperationStatus>,
}

fn parse_jsonl_records(
    text: &str,
    options: &SerializationOptions,
    control: Option<&OperationControl>,
) -> ParsedRecords {
    let lines = LineIndex::new(text);
    let mut records = Vec::new();
    let mut duplicates = Vec::new();
    let mut raw_unknowns = Vec::new();
    let mut diagnostics = Vec::new();
    let mut node_count = 0;
    let mut max_depth = 0;
    let mut offset = 0;
    let mut source_line = 1;
    let mut status = None;
    while offset < text.len() {
        let line_end = text[offset..]
            .find('\n')
            .map_or(text.len(), |relative| offset + relative + 1);
        let content_end = text[offset..line_end].trim_end_matches(['\r', '\n']).len() + offset;
        let raw = &text[offset..content_end];
        if !raw.trim().is_empty() {
            if let Some(control) = control {
                if let Err(error) = control.checkpoint().and_then(|_| {
                    control
                        .budget()
                        .consume_records(1)
                        .map_err(crate::core::OperationControlError::from)
                }) {
                    diagnostics.push(error.diagnostic(PARSER));
                    status = Some(if records.is_empty() {
                        OperationStatus::Failed
                    } else {
                        OperationStatus::Partial
                    });
                    break;
                }
            }
            let record_index = records.len() + 1;
            let range = SourceRange::new(offset, content_end, &lines);
            let record_locator = json::locator(range.clone(), "", Some(record_index));
            match json::parse(text, offset, content_end, Some(record_index)) {
                Ok(parsed) => {
                    node_count += parsed.node_count;
                    max_depth = max_depth.max(parsed.max_depth);
                    duplicates.extend(parsed.duplicate_keys);
                    records.push(StructuredRecord {
                        index: record_index,
                        source_line,
                        range,
                        locator: record_locator,
                        raw: raw.into(),
                        value: Some(parsed.value),
                        diagnostic: None,
                    });
                }
                Err(error) => {
                    let error_range =
                        SourceRange::new(error.offset, (error.offset + 1).min(content_end), &lines);
                    let diagnostic = malformed(
                        "jsonl.record_parse",
                        format!(
                            "JSONL record {record_index} is malformed: {}",
                            error.message
                        ),
                        json::locator(error_range, "", Some(record_index)),
                    )
                    .partial();
                    raw_unknowns.push(RawStructuredUnknown {
                        path: format!("/records/{record_index}"),
                        raw: raw.into(),
                        reason: error.message,
                        range: range.clone(),
                        locator: record_locator.clone(),
                    });
                    records.push(StructuredRecord {
                        index: record_index,
                        source_line,
                        range,
                        locator: record_locator,
                        raw: raw.into(),
                        value: None,
                        diagnostic: Some(diagnostic.clone()),
                    });
                    diagnostics.push(diagnostic);
                    status = Some(OperationStatus::Partial);
                }
            }
        }
        source_line += 1;
        offset = line_end;
    }
    if text.is_empty() && options.malformed_recovery == MalformedRecoveryPolicy::PreserveRaw {
        status = None;
    }
    ParsedRecords {
        records,
        duplicates,
        raw_unknowns,
        diagnostics,
        node_count,
        max_depth,
        status,
    }
}

pub fn stream_jsonl<'a>(
    text: &'a str,
    _options: &SerializationOptions,
    request_id: RequestId,
    control: OperationControl,
) -> JsonlStream<'a> {
    JsonlStream {
        text,
        lines: LineIndex::new(text),
        request_id,
        control,
        offset: 0,
        source_line: 1,
        record_index: 0,
        sequence: 0,
        diagnostics: Vec::new(),
        finished: false,
    }
}

pub struct JsonlStream<'a> {
    text: &'a str,
    lines: LineIndex,
    request_id: RequestId,
    control: OperationControl,
    offset: usize,
    source_line: usize,
    record_index: usize,
    sequence: u64,
    diagnostics: Vec<Diagnostic>,
    finished: bool,
}

impl Iterator for JsonlStream<'_> {
    type Item = StructuredRecordStreamEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            if let Err(error) = self.control.checkpoint() {
                self.finished = true;
                return Some(StreamEvent::terminal(StreamTerminal::from_control(
                    PARSER,
                    self.sequence,
                    &self.control,
                    error,
                )));
            }
            if self.offset >= self.text.len() {
                self.finished = true;
                let status = if self.diagnostics.iter().any(|diagnostic| diagnostic.partial) {
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
            }
            let start = self.offset;
            let line_end = self.text[start..]
                .find('\n')
                .map_or(self.text.len(), |relative| start + relative + 1);
            let content_end = self.text[start..line_end]
                .trim_end_matches(['\r', '\n'])
                .len()
                + start;
            self.offset = line_end;
            let source_line = self.source_line;
            self.source_line += 1;
            let raw = &self.text[start..content_end];
            if raw.trim().is_empty() {
                continue;
            }
            if let Err(error) = self.control.budget().consume_records(1) {
                self.finished = true;
                return Some(StreamEvent::terminal(StreamTerminal::from_control(
                    PARSER,
                    self.sequence,
                    &self.control,
                    error.into(),
                )));
            }
            self.record_index += 1;
            let range = SourceRange::new(start, content_end, &self.lines);
            let record_locator = json::locator(range.clone(), "", Some(self.record_index));
            let (value, diagnostic) =
                match json::parse(self.text, start, content_end, Some(self.record_index)) {
                    Ok(parsed) => {
                        let charged = self
                            .control
                            .budget()
                            .consume_nodes(parsed.node_count as u64)
                            .and_then(|_| {
                                self.control
                                    .budget()
                                    .observe_nesting_depth(parsed.max_depth as u64)
                            });
                        if let Err(error) = charged {
                            self.finished = true;
                            return Some(StreamEvent::terminal(StreamTerminal::from_control(
                                PARSER,
                                self.sequence,
                                &self.control,
                                error.into(),
                            )));
                        }
                        (Some(parsed.value), None)
                    }
                    Err(error) => {
                        let error_range = SourceRange::new(
                            error.offset,
                            (error.offset + 1).min(content_end),
                            &self.lines,
                        );
                        let diagnostic = malformed(
                            "jsonl.record_parse",
                            format!(
                                "JSONL record {} is malformed: {}",
                                self.record_index, error.message
                            ),
                            json::locator(error_range, "", Some(self.record_index)),
                        )
                        .partial();
                        self.diagnostics.push(diagnostic.clone());
                        (None, Some(diagnostic))
                    }
                };
            let record = StructuredRecord {
                index: self.record_index,
                source_line,
                range,
                locator: record_locator,
                raw: raw.into(),
                value,
                diagnostic,
            };
            let identity = ContentIdentity::for_raw_bytes(record.raw.as_bytes()).with_format(
                FormatIdentity::new("jsonl-record", Some("application/json")),
            );
            let event = StreamEvent::item(StreamItem::new(
                self.sequence,
                self.request_id.clone(),
                identity,
                record,
            ));
            self.sequence += 1;
            return Some(event);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn recover_or_fail(
    text: &str,
    code: &str,
    offset: usize,
    message: String,
    options: &SerializationOptions,
    documents: &mut Vec<StructuredValue>,
    raw_unknowns: &mut Vec<RawStructuredUnknown>,
    diagnostics: &mut Vec<Diagnostic>,
    status: &mut Option<OperationStatus>,
) {
    recover_or_fail_range(
        text,
        code,
        offset..(offset + 1).min(text.len()),
        message,
        options,
        documents,
        raw_unknowns,
        diagnostics,
        status,
    );
}

#[allow(clippy::too_many_arguments)]
fn recover_or_fail_range(
    text: &str,
    code: &str,
    error_span: std::ops::Range<usize>,
    message: String,
    options: &SerializationOptions,
    documents: &mut Vec<StructuredValue>,
    raw_unknowns: &mut Vec<RawStructuredUnknown>,
    diagnostics: &mut Vec<Diagnostic>,
    status: &mut Option<OperationStatus>,
) {
    let lines = LineIndex::new(text);
    let start = error_span.start.min(text.len());
    let end = error_span.end.min(text.len()).max(start);
    let error_range = SourceRange::new(start, end, &lines);
    let error_locator = json::locator(error_range, "", None);
    let mut diagnostic = malformed(code, message.clone(), error_locator);
    diagnostic.recovery = Some(RecoveryAction::new(
        RecoveryKind::InspectInput,
        "correct the malformed source or select preserve_raw recovery",
        true,
    ));
    if options.malformed_recovery == MalformedRecoveryPolicy::Strict {
        diagnostics.push(diagnostic);
        *status = Some(OperationStatus::Failed);
        return;
    }
    let range = SourceRange::new(0, text.len(), &lines);
    let locator = json::locator(range.clone(), "", None);
    let unknown = RawStructuredUnknown {
        path: String::new(),
        raw: text.into(),
        reason: message,
        range: range.clone(),
        locator: locator.clone(),
    };
    raw_unknowns.push(unknown);
    documents.push(StructuredValue {
        id: "structured:raw@0".into(),
        kind: StructuredValueKind::RawUnknown,
        path: String::new(),
        range,
        locator,
        raw: text.into(),
        scalar: None,
        entries: vec![],
        items: vec![],
        anchor: None,
        tag: None,
        alias: None,
        alias_target_id: None,
        recovered: true,
    });
    diagnostics.push(diagnostic.partial());
    *status = Some(OperationStatus::Partial);
}

fn malformed(code: &str, message: impl Into<String>, locator: SourceLocator) -> Diagnostic {
    let mut diagnostic = Diagnostic::malformed(PARSER, message)
        .with_locator(locator)
        .with_module(PARSER)
        .with_explanation_key(code);
    diagnostic.code = DiagnosticCode::new(code);
    diagnostic
}

fn parser_info(format: SerializationFormat) -> ParserInfo {
    let (implementation, version, specification) = match format {
        SerializationFormat::Json | SerializationFormat::Jsonl => (
            "grist-json-cst",
            env!("CARGO_PKG_VERSION"),
            "RFC 8259 / NDJSON 1.0",
        ),
        SerializationFormat::Yaml => ("libyaml-events", "0.2.11", "YAML 1.2 core schema"),
        SerializationFormat::Toml => ("toml_edit", "0.22.27", "TOML 1.0"),
        SerializationFormat::Xml => ("grist.xml/quick-xml", "0.37.5", "XML 1.0/1.1"),
    };
    ParserInfo::new(PARSER)
        .with_implementation(implementation, version)
        .with_specification_version(specification)
        .with_feature("serialization")
}

const fn format_name(format: SerializationFormat) -> &'static str {
    match format {
        SerializationFormat::Json => "json",
        SerializationFormat::Jsonl => "jsonl",
        SerializationFormat::Yaml => "yaml",
        SerializationFormat::Toml => "toml",
        SerializationFormat::Xml => "xml",
    }
}

const fn media_type(format: SerializationFormat) -> Option<&'static str> {
    match format {
        SerializationFormat::Json => Some("application/json"),
        SerializationFormat::Jsonl => Some("application/x-ndjson"),
        SerializationFormat::Yaml => Some("application/yaml"),
        SerializationFormat::Toml => Some("application/toml"),
        SerializationFormat::Xml => Some("application/xml"),
    }
}

pub fn format_from_content_kind(kind: &ContentKind) -> Option<SerializationFormat> {
    match kind {
        ContentKind::Json => Some(SerializationFormat::Json),
        ContentKind::Jsonl => Some(SerializationFormat::Jsonl),
        ContentKind::Yaml => Some(SerializationFormat::Yaml),
        ContentKind::Toml => Some(SerializationFormat::Toml),
        ContentKind::Xml => Some(SerializationFormat::Xml),
        _ => None,
    }
}

pub fn validate_json_schema(value: &Value, schema: &Value) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    match jsonschema::validator_for(schema) {
        Ok(validator) => {
            for error in validator.iter_errors(value) {
                diagnostics.push(Diagnostic::error(
                    "grist.structured-text.schema",
                    "schema.validation",
                    format!(
                        "JSON Schema validation failed at {}: {}",
                        error.instance_path, error
                    ),
                ));
            }
        }
        Err(error) => diagnostics.push(Diagnostic::error(
            "grist.structured-text.schema",
            "schema.invalid",
            format!("JSON Schema is invalid: {error}"),
        )),
    }
    diagnostics
}

fn lexical_nesting_depth(text: &str, format: SerializationFormat) -> usize {
    let mut depth = 0usize;
    let mut maximum = 0usize;
    let mut quoted = None;
    let mut escaped = false;
    for byte in text.bytes() {
        if let Some(quote) = quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' && quote == b'"' {
                escaped = true;
            } else if byte == quote {
                quoted = None;
            }
            continue;
        }
        if matches!(byte, b'"' | b'\'') {
            quoted = Some(byte);
            continue;
        }
        match byte {
            b'{' | b'[' => {
                depth += 1;
                maximum = maximum.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if matches!(
        format,
        SerializationFormat::Yaml | SerializationFormat::Toml
    ) {
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let indentation = line.len() - line.trim_start().len();
            maximum = maximum.max(indentation / 2 + 1);
            if format == SerializationFormat::Toml {
                maximum = maximum.max(line.trim_start_matches('[').split('.').count());
            }
        }
    }
    maximum.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_duplicates_and_large_integer_are_lossless() {
        let envelope = parse_serialization(
            r#"{"a":1,"a":184467440737095516160}"#,
            SerializationFormat::Json,
            SourceInfo::stdin("input.json"),
        );
        let payload = envelope.payload().unwrap();
        assert_eq!(payload.duplicate_keys.len(), 1);
        assert_eq!(payload.documents[0].entries.len(), 2);
        assert_eq!(
            payload.documents[0].entries[1].value.scalar,
            Some(StructuredScalar::Integer {
                canonical: "184467440737095516160".into()
            })
        );
    }

    #[test]
    fn jsonl_retains_bad_records() {
        let envelope = parse_serialization(
            "{\"a\":1}\nnot-json\n{\"b\":2}",
            SerializationFormat::Jsonl,
            SourceInfo::stdin("input.ndjson"),
        );
        assert_eq!(envelope.status, OperationStatus::Partial);
        assert_eq!(envelope.payload().unwrap().records.len(), 3);
        assert!(envelope.payload().unwrap().records[1].value.is_none());
    }

    #[test]
    fn yaml_alias_is_a_reference_not_an_expansion() {
        let envelope = parse_serialization(
            "base: &base {x: 1}\ncopy: *base\n",
            SerializationFormat::Yaml,
            SourceInfo::stdin("input.yaml"),
        );
        let alias = &envelope.payload().unwrap().aliases[0];
        assert!(alias.resolved);
        assert!(!alias.expanded);
    }
}
