//! Batch and streaming model-output interpretation.

use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, LineIndex, ParserInfo, SchemaVersion, SourceInfo,
    SourceRange,
};
use crate::serialization::{SchemaValidationResult, validate_json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

mod streaming;
pub use streaming::{DEFAULT_STREAM_BUFFER_BYTES, StreamingModelOutputParserV2};

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelOutputReport {
    pub schema_version: String,
    pub candidates: Vec<ModelOutputCandidate>,
    pub selected_candidate_id: Option<String>,
    pub raw_text_sha256: String,
    pub status: ModelOutputStatus,
    pub failures: Vec<ModelOutputFailure>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelOutputStatus {
    Empty,
    Incomplete,
    Malformed,
    Parsed,
    Ambiguous,
    Unparsed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelOutputCandidate {
    pub id: String,
    pub grammar: CandidateGrammar,
    pub command_name: Option<String>,
    pub argument_name: Option<String>,
    /// The value before repair projection and runtime alias normalization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_value: Option<Value>,
    /// The repaired and alias-normalized value. Parsing does not make it trusted.
    pub value: Option<Value>,
    pub raw_text: Option<String>,
    pub raw_range: Option<SourceRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repaired_text: Option<String>,
    #[serde(default)]
    pub repairs: Vec<RepairOperation>,
    #[serde(default)]
    pub alias_applications: Vec<AliasApplication>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_error: Option<JsonErrorPosition>,
    pub status: CandidateStatus,
    pub confidence: f32,
    pub normalizations: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub validation: Option<SchemaValidationResult>,
    /// Grist parses and validates structure but never establishes downstream trust.
    #[serde(default)]
    pub trusted: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepairOperation {
    pub kind: RepairKind,
    /// Byte range in the input to this operation (half-open and zero-based).
    pub input_byte_start: usize,
    pub input_byte_end: usize,
    /// Exact bytes replaced by this operation.
    pub original_text: String,
    /// Exact replacement bytes; empty for a bounded deletion.
    pub replacement_text: String,
    /// Exact source range when the operation applies directly to original candidate bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_range: Option<SourceRange>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepairKind {
    RemovePrematureClosingBrace,
    ReplacePythonLiteral,
    RemoveRedundantObjectOpener,
    QuoteSingleQuotedString,
    QuoteUnquotedObjectKey,
    PreserveBackslashCommand,
    EscapeUnescapedStringQuote,
    InsertMissingComma,
    RemoveTrailingComma,
    CloseUnterminatedContainer,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AliasApplication {
    pub kind: AliasKind,
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_value: Option<Value>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AliasKind {
    Field,
    Command,
    Argument,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonErrorPosition {
    /// Zero-based byte offset in the original complete model-output source.
    pub byte_offset: usize,
    /// One-based line and column reported by the JSON parser.
    pub line: usize,
    pub column: usize,
    pub message: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelOutputFailure {
    pub id: String,
    pub failure_mode: String,
    pub grammar: Option<CandidateGrammar>,
    pub raw_text: String,
    pub raw_text_sha256: String,
    pub raw_range: Option<SourceRange>,
    pub diagnostics: Vec<Diagnostic>,
    pub recoverable: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateGrammar {
    PythonStyleCommand,
    OpenAiToolCall,
    OpenAiChatContent,
    McpJsonRpc,
    FencedJson,
    FencedCode,
    RawJson,
    JsonObjectInText,
    YamlBlock,
    TomlBlock,
    XmlToolCall,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Complete,
    Recovered,
    Incomplete,
    Malformed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AliasRules {
    pub field_aliases: Vec<AliasRule>,
    pub command_aliases: Vec<AliasRule>,
    pub argument_aliases: Vec<AliasRule>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AliasRule {
    pub from: String,
    pub to: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RepairLimits {
    pub max_operations: usize,
    pub max_changed_bytes: usize,
}

impl Default for RepairLimits {
    fn default() -> Self {
        Self {
            max_operations: 32,
            max_changed_bytes: 64 * 1024,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelOutputOptions {
    pub accepted_commands: Vec<String>,
    pub accepted_arguments: Vec<String>,
    pub aliases: AliasRules,
    pub repair_limits: RepairLimits,
    pub strip_think_blocks: bool,
    pub schema: Option<Value>,
    /// Enable legacy Python-style command calls such as `Namespace.Command(arg={...})`.
    ///
    /// JSON-oriented output is the default primary model-output path. Callers that still
    /// need Python-style command parsing can opt in with this flag and, preferably,
    /// explicit accepted command/argument names.
    pub parse_python_style_commands: bool,
}

impl crate::core::FormatOptions for ModelOutputOptions {
    const FORMAT: &'static str = "model_output";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ModelOutputEvent {
    CandidateStarted {
        candidate_id: String,
        grammar: CandidateGrammar,
    },
    CandidateUpdated {
        candidate_id: String,
        bytes_seen: usize,
    },
    CandidateCompleted {
        candidate: ModelOutputCandidate,
    },
    Diagnostic {
        diagnostic: Diagnostic,
    },
    ParserStateChanged {
        state: StreamingState,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamingState {
    Empty,
    Accumulating,
    CandidateDetected,
    Complete,
    Incomplete,
}

/// Version 2 streaming contract. V1 remains unchanged for consumers that use
/// the original non-terminal event schema.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ModelOutputStreamEventV2 {
    CandidateStarted {
        candidate_id: String,
        grammar: CandidateGrammar,
    },
    CandidateUpdated {
        candidate_id: String,
        bytes_seen: usize,
    },
    CandidateCompleted {
        candidate_id: String,
        candidate: ModelOutputCandidate,
    },
    Diagnostic {
        diagnostic: Diagnostic,
    },
    ParserStateChanged {
        state: StreamingStateV2,
    },
    /// Exactly one terminal event closes the stream. It is always the last
    /// event, including for cancellation and resource exhaustion.
    Terminal {
        terminal: crate::core::StreamTerminal,
    },
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamingStateV2 {
    Empty,
    Accumulating,
    CandidateDetected,
    Complete,
    Incomplete,
    Malformed,
    Ambiguous,
    Unparsed,
    Failed,
    Cancelled,
}

pub type ModelOutputEnvelope = Envelope<ModelOutputReport>;

pub fn parse_model_output(
    text: &str,
    source: SourceInfo,
    options: &ModelOutputOptions,
) -> ModelOutputEnvelope {
    let mut working = text.to_string();
    let mut global_normalizations = Vec::new();
    if options.strip_think_blocks {
        let stripped = strip_think_blocks(&working);
        if stripped != working {
            global_normalizations.push("stripped_think_blocks".to_string());
            working = stripped;
        }
    }

    // Think-block stripping deliberately preserves byte offsets, so all candidate
    // ranges and raw text continue to address the caller's original input.
    let index = LineIndex::new(text);
    let mut candidates = Vec::new();
    let mut diagnostics = Vec::new();
    let mut failures = Vec::new();

    if working.trim().is_empty() {
        diagnostics.push(Diagnostic::error(
            "grist.model_output",
            "model_output.empty",
            "empty model output",
        ));
    } else {
        extract_fenced_blocks(
            &working,
            &index,
            &mut candidates,
            &options.repair_limits,
            &mut failures,
        );
        extract_nested_json_fences(&working, &index, &mut candidates, &options.repair_limits);
        extract_json_candidates(&working, &index, &options.repair_limits, &mut candidates);
        extract_xml_tool_calls(&working, &index, &mut candidates);
        if options.parse_python_style_commands {
            extract_python_style_commands(&working, &index, options, &mut candidates);
        }
    }

    candidates = expand_compound_json_candidates(candidates, text, &index, &options.repair_limits);
    for (id, candidate) in candidates.iter_mut().enumerate() {
        candidate.id = format!("candidate-{id}");
    }

    for candidate in candidates.iter_mut() {
        candidate
            .normalizations
            .extend(global_normalizations.clone());
        if candidate.original_value.is_none() {
            candidate.original_value = candidate.value.clone();
        }
        apply_aliases(candidate, &options.aliases);
        if candidate.raw_text.is_none() {
            candidate.raw_text = candidate
                .raw_range
                .as_ref()
                .and_then(|range| text.get(range.byte_start..range.byte_end))
                .map(str::to_string);
        }
        if let (Some(value), Some(schema)) = (candidate.value.as_ref(), options.schema.as_ref()) {
            let validation_diagnostics = validate_json_schema(value, schema);
            candidate.validation = Some(SchemaValidationResult {
                valid: validation_diagnostics.is_empty(),
                diagnostics: validation_diagnostics.clone(),
            });
            candidate.diagnostics.extend(validation_diagnostics);
        }
        for mut failure in failures_from_candidate(candidate, text) {
            failure.id = format!("failure-{}", failures.len());
            failures.push(failure);
        }
    }

    let status = if working.trim().is_empty() {
        ModelOutputStatus::Empty
    } else if candidates.is_empty() {
        diagnostics.push(Diagnostic::error(
            "grist.model_output",
            "model_output.unparsed",
            "no supported model-output candidate was detected",
        ));
        failures.push(failure_record(
            failures.len(),
            "unparsed_model_output",
            None,
            working.trim(),
            None,
            diagnostics.clone(),
            true,
        ));
        ModelOutputStatus::Unparsed
    } else if candidates
        .iter()
        .all(|candidate| candidate.status == CandidateStatus::Incomplete)
    {
        ModelOutputStatus::Incomplete
    } else if candidates.iter().all(|candidate| {
        matches!(
            candidate.status,
            CandidateStatus::Incomplete | CandidateStatus::Malformed
        )
    }) && candidates
        .iter()
        .any(|candidate| candidate.status == CandidateStatus::Malformed)
    {
        ModelOutputStatus::Malformed
    } else if select_candidate_id(&candidates, options).is_some() {
        ModelOutputStatus::Parsed
    } else {
        ModelOutputStatus::Ambiguous
    };

    let selected_candidate_id = select_candidate_id(&candidates, options);
    let raw_text_sha256 = crate::core::sha256_hex(text.as_bytes());
    let report = ModelOutputReport {
        schema_version: SchemaVersion::MODEL_OUTPUT_V1.to_string(),
        candidates,
        selected_candidate_id,
        raw_text_sha256,
        status: status.clone(),
        failures,
    };
    let options_digest =
        crate::core::options_digest(options).expect("model-output options must serialize");
    let envelope = if status == ModelOutputStatus::Parsed {
        Envelope::complete(
            crate::core::OperationKind::Parse,
            ArtifactKind::ModelOutput,
            source,
            ParserInfo::new("grist.model_output"),
            options_digest,
            SchemaVersion::MODEL_OUTPUT_V1,
            report,
        )
    } else {
        Envelope::partial(
            crate::core::OperationKind::Parse,
            ArtifactKind::ModelOutput,
            source,
            ParserInfo::new("grist.model_output"),
            options_digest,
            SchemaVersion::MODEL_OUTPUT_V1,
            Some(report),
        )
    };
    envelope
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(diagnostics)
}

fn extract_nested_json_fences(
    text: &str,
    index: &LineIndex,
    candidates: &mut Vec<ModelOutputCandidate>,
    limits: &RepairLimits,
) {
    for marker in ["```json\n", "```JSON\n", "```json\r\n", "```JSON\r\n"] {
        let mut search = 0;
        while let Some(relative_start) = text[search..].find(marker) {
            let start = search + relative_start;
            let body_start = start + marker.len();
            let Some(relative_end) = text[body_start..].find("```") else {
                break;
            };
            let end = body_start + relative_end;
            let range_end = end + 3;
            if !candidates.iter().any(|candidate| {
                candidate
                    .raw_range
                    .as_ref()
                    .is_some_and(|range| range.byte_start == start && range.byte_end == range_end)
            }) {
                let body = text[body_start..end].trim();
                if let Ok(parsed) = parse_jsonish_value_with_limits(body, limits) {
                    let mut candidate = base_candidate(
                        candidates.len(),
                        CandidateGrammar::FencedJson,
                        Some(SourceRange::new(start, range_end, index)),
                    );
                    candidate.normalizations = vec!["extracted_nested_markdown_fence".into()];
                    let parsed_start = body_start + text[body_start..end].find(body).unwrap_or(0);
                    adopt_jsonish_parse(&mut candidate, parsed, parsed_start, index);
                    if candidate.normalizations.len() > 1 {
                        candidate.status = CandidateStatus::Recovered;
                    }
                    classify_json_tool_shape(&mut candidate, limits, text, index);
                    candidates.push(candidate);
                }
            }
            search = range_end;
        }
    }
}

pub struct StreamingModelOutputParser {
    source: SourceInfo,
    options: ModelOutputOptions,
    buffer: String,
    state: StreamingState,
    emitted_started: bool,
}

impl StreamingModelOutputParser {
    pub fn new(source: SourceInfo, options: ModelOutputOptions) -> Self {
        Self {
            source,
            options,
            buffer: String::new(),
            state: StreamingState::Empty,
            emitted_started: false,
        }
    }

    pub fn push_chunk(&mut self, chunk: &str) -> Vec<ModelOutputEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();
        let new_state = if self.buffer.trim().is_empty() {
            StreamingState::Empty
        } else {
            StreamingState::Accumulating
        };
        if new_state != self.state {
            self.state = new_state.clone();
            events.push(ModelOutputEvent::ParserStateChanged { state: new_state });
        }
        if !self.emitted_started && looks_like_candidate_start(&self.buffer) {
            self.emitted_started = true;
            events.push(ModelOutputEvent::CandidateStarted {
                candidate_id: "stream-candidate-0".into(),
                grammar: infer_streaming_grammar(&self.buffer),
            });
            events.push(ModelOutputEvent::ParserStateChanged {
                state: StreamingState::CandidateDetected,
            });
        } else if self.emitted_started {
            events.push(ModelOutputEvent::CandidateUpdated {
                candidate_id: "stream-candidate-0".into(),
                bytes_seen: self.buffer.len(),
            });
        }
        events
    }

    pub fn finish(self) -> (Vec<ModelOutputEvent>, ModelOutputEnvelope) {
        let report = parse_model_output(&self.buffer, self.source, &self.options);
        let payload = report
            .payload
            .as_ref()
            .expect("complete model-output parse envelope");
        let mut events = Vec::new();
        if payload.candidates.is_empty() && !self.buffer.trim().is_empty() {
            events.push(ModelOutputEvent::Diagnostic {
                diagnostic: Diagnostic::error(
                    "grist.model_output.streaming",
                    "stream.unparsed",
                    "stream finished without a complete supported candidate",
                ),
            });
            events.push(ModelOutputEvent::ParserStateChanged {
                state: StreamingState::Incomplete,
            });
        } else {
            for candidate in &payload.candidates {
                events.push(ModelOutputEvent::CandidateCompleted {
                    candidate: candidate.clone(),
                });
            }
            events.push(ModelOutputEvent::ParserStateChanged {
                state: StreamingState::Complete,
            });
        }
        (events, report)
    }
}

fn extract_fenced_blocks(
    text: &str,
    index: &LineIndex,
    candidates: &mut Vec<ModelOutputCandidate>,
    limits: &RepairLimits,
    failures: &mut Vec<ModelOutputFailure>,
) {
    let mut search = 0;
    while let Some(start_rel) = text[search..].find("```") {
        let start = search + start_rel;
        let info_start = start + 3;
        let Some(line_end_rel) = text[info_start..].find('\n') else {
            break;
        };
        let line_end = info_start + line_end_rel;
        let info = text[info_start..line_end].trim().to_string();
        let body_start = line_end + 1;
        let language = info
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some(end_rel) = text[body_start..].find("```") else {
            let untrimmed_body = &text[body_start..];
            let body = untrimmed_body.trim();
            let parsed_start = body_start + untrimmed_body.find(body).unwrap_or(0);
            let mut candidate = base_candidate(
                candidates.len(),
                if language == "json" {
                    CandidateGrammar::FencedJson
                } else {
                    CandidateGrammar::FencedCode
                },
                Some(SourceRange::new(start, text.len(), index)),
            );
            candidate.status = CandidateStatus::Incomplete;
            candidate
                .normalizations
                .push("extracted_incomplete_markdown_fence".into());
            if language == "json" || body.starts_with('{') || body.starts_with('[') {
                if let Ok(parsed) = parse_jsonish_value_with_limits(body, limits) {
                    adopt_jsonish_parse(&mut candidate, parsed, parsed_start, index);
                    classify_json_tool_shape(&mut candidate, limits, text, index);
                    candidate.status = CandidateStatus::Recovered;
                    candidate
                        .normalizations
                        .push("parsed_unclosed_fence_body".into());
                }
            } else if !body.is_empty() {
                candidate.value = Some(Value::String(body.to_string()));
            }
            let diagnostic = Diagnostic::warning(
                "grist.model_output.fence",
                "fence.unclosed",
                "markdown fence was opened but not closed",
            )
            .partial();
            failures.push(failure_record(
                failures.len(),
                "unclosed_markdown_fence",
                Some(candidate.grammar.clone()),
                body,
                Some(SourceRange::new(body_start, text.len(), index)),
                vec![diagnostic.clone()],
                true,
            ));
            candidate.diagnostics.push(diagnostic);
            candidates.push(candidate);
            break;
        };
        let end = body_start + end_rel;
        let untrimmed_body = &text[body_start..end];
        let body = untrimmed_body.trim();
        let parsed_start = body_start + untrimmed_body.find(body).unwrap_or(0);
        let mut candidate = base_candidate(
            candidates.len(),
            if language == "json" {
                CandidateGrammar::FencedJson
            } else {
                CandidateGrammar::FencedCode
            },
            Some(SourceRange::new(start, end + 3, index)),
        );
        candidate
            .normalizations
            .push("extracted_markdown_fence".into());
        match language.as_str() {
            "json" if body.starts_with('{') || body.starts_with('[') || !body.is_empty() => {
                match parse_jsonish_value_with_limits(body, limits) {
                    Ok(parsed) => {
                        adopt_jsonish_parse(&mut candidate, parsed, parsed_start, index);
                        if candidate.normalizations.len() > 1 {
                            candidate.status = CandidateStatus::Recovered;
                        }
                        classify_json_tool_shape(&mut candidate, limits, text, index);
                    }
                    Err(err) => {
                        retain_json_failure(&mut candidate, &err, body, parsed_start, index);
                        let diagnostic = Diagnostic::error(
                            "grist.model_output.fence",
                            "fence.json_parse",
                            format!("fenced JSON parse failed: {err}"),
                        );
                        failures.push(failure_record(
                            failures.len(),
                            "fenced_json_parse_failed",
                            Some(CandidateGrammar::FencedJson),
                            body,
                            Some(SourceRange::new(body_start, end, index)),
                            vec![diagnostic.clone()],
                            true,
                        ));
                        candidate.diagnostics.push(diagnostic);
                    }
                }
            }
            "yaml" | "yml" => match serde_yaml::from_str::<serde_yaml::Value>(body) {
                Ok(value) => match serde_json::to_value(value) {
                    Ok(value) => {
                        candidate.grammar = CandidateGrammar::YamlBlock;
                        candidate.value = Some(value);
                        candidate.normalizations.push("parsed_yaml_block".into());
                    }
                    Err(err) => {
                        candidate.status = CandidateStatus::Malformed;
                        let diagnostic = Diagnostic::error(
                            "grist.model_output.fence",
                            "fence.yaml_to_json",
                            format!("fenced YAML conversion failed: {err}"),
                        );
                        failures.push(failure_record(
                            failures.len(),
                            "fenced_yaml_conversion_failed",
                            Some(CandidateGrammar::YamlBlock),
                            body,
                            Some(SourceRange::new(body_start, end, index)),
                            vec![diagnostic.clone()],
                            true,
                        ));
                        candidate.diagnostics.push(diagnostic);
                    }
                },
                Err(err) => {
                    candidate.status = CandidateStatus::Malformed;
                    let diagnostic = Diagnostic::error(
                        "grist.model_output.fence",
                        "fence.yaml_parse",
                        format!("fenced YAML parse failed: {err}"),
                    );
                    failures.push(failure_record(
                        failures.len(),
                        "fenced_yaml_parse_failed",
                        Some(CandidateGrammar::YamlBlock),
                        body,
                        Some(SourceRange::new(body_start, end, index)),
                        vec![diagnostic.clone()],
                        true,
                    ));
                    candidate.diagnostics.push(diagnostic);
                }
            },
            "toml" => match body.parse::<toml::Value>() {
                Ok(value) => match serde_json::to_value(value) {
                    Ok(value) => {
                        candidate.grammar = CandidateGrammar::TomlBlock;
                        candidate.value = Some(value);
                        candidate.normalizations.push("parsed_toml_block".into());
                    }
                    Err(err) => {
                        candidate.status = CandidateStatus::Malformed;
                        let diagnostic = Diagnostic::error(
                            "grist.model_output.fence",
                            "fence.toml_to_json",
                            format!("fenced TOML conversion failed: {err}"),
                        );
                        failures.push(failure_record(
                            failures.len(),
                            "fenced_toml_conversion_failed",
                            Some(CandidateGrammar::TomlBlock),
                            body,
                            Some(SourceRange::new(body_start, end, index)),
                            vec![diagnostic.clone()],
                            true,
                        ));
                        candidate.diagnostics.push(diagnostic);
                    }
                },
                Err(err) => {
                    candidate.status = CandidateStatus::Malformed;
                    let diagnostic = Diagnostic::error(
                        "grist.model_output.fence",
                        "fence.toml_parse",
                        format!("fenced TOML parse failed: {err}"),
                    );
                    failures.push(failure_record(
                        failures.len(),
                        "fenced_toml_parse_failed",
                        Some(CandidateGrammar::TomlBlock),
                        body,
                        Some(SourceRange::new(body_start, end, index)),
                        vec![diagnostic.clone()],
                        true,
                    ));
                    candidate.diagnostics.push(diagnostic);
                }
            },
            _ if body.starts_with('{') || body.starts_with('[') => {
                match parse_jsonish_value_with_limits(body, limits) {
                    Ok(parsed) => {
                        adopt_jsonish_parse(&mut candidate, parsed, parsed_start, index);
                        if candidate.normalizations.len() > 1 {
                            candidate.status = CandidateStatus::Recovered;
                        }
                        classify_json_tool_shape(&mut candidate, limits, text, index);
                    }
                    Err(err) => {
                        retain_json_failure(&mut candidate, &err, body, parsed_start, index);
                        let diagnostic = Diagnostic::error(
                            "grist.model_output.fence",
                            "fence.jsonish_parse",
                            format!("fenced JSON-like parse failed: {err}"),
                        );
                        failures.push(failure_record(
                            failures.len(),
                            "fenced_jsonish_parse_failed",
                            Some(candidate.grammar.clone()),
                            body,
                            Some(SourceRange::new(body_start, end, index)),
                            vec![diagnostic.clone()],
                            true,
                        ));
                        candidate.diagnostics.push(diagnostic);
                    }
                }
            }
            _ => candidate.value = Some(Value::String(body.to_string())),
        }
        candidates.push(candidate);
        search = end + 3;
    }
}

fn extract_xml_tool_calls(
    text: &str,
    index: &LineIndex,
    candidates: &mut Vec<ModelOutputCandidate>,
) {
    let mut search = 0;
    while let Some(start_rel) = text[search..].find("<tool_call>") {
        let start = search + start_rel;
        let body_start = start + "<tool_call>".len();
        let Some(end_rel) = text[body_start..].find("</tool_call>") else {
            break;
        };
        let end = body_start + end_rel;
        let body = &text[body_start..end];
        let mut object = serde_json::Map::new();
        for tag in ["name", "arguments", "method", "params"] {
            let open = format!("<{tag}>");
            let close = format!("</{tag}>");
            if let Some(value_start_rel) = body.find(&open) {
                let value_start = value_start_rel + open.len();
                if let Some(value_end_rel) = body[value_start..].find(&close) {
                    let raw = body[value_start..value_start + value_end_rel].trim();
                    let value =
                        parse_jsonish_value(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
                    object.insert(tag.to_string(), value);
                }
            }
        }
        let mut candidate = base_candidate(
            candidates.len(),
            CandidateGrammar::XmlToolCall,
            Some(SourceRange::new(start, end + "</tool_call>".len(), index)),
        );
        candidate.command_name = object
            .get("name")
            .or_else(|| object.get("method"))
            .and_then(Value::as_str)
            .map(str::to_string);
        candidate.argument_name = object
            .contains_key("arguments")
            .then_some("arguments".to_string())
            .or_else(|| {
                object
                    .contains_key("params")
                    .then_some("params".to_string())
            });
        candidate.value = Some(Value::Object(object));
        candidate.normalizations.push("parsed_xml_tool_call".into());
        candidates.push(candidate);
        search = end + "</tool_call>".len();
    }
}

fn extract_python_style_commands(
    text: &str,
    index: &LineIndex,
    options: &ModelOutputOptions,
    candidates: &mut Vec<ModelOutputCandidate>,
) {
    let commands: Vec<String> = if options.accepted_commands.is_empty() {
        discover_command_names(text)
    } else {
        options.accepted_commands.clone()
    };
    let args: Vec<String> = if options.accepted_arguments.is_empty() {
        vec![
            "arg".into(),
            "args".into(),
            "input".into(),
            "request".into(),
            "payload".into(),
        ]
    } else {
        options.accepted_arguments.clone()
    };
    for command in commands {
        let needle = format!("{command}(");
        let mut search = 0;
        while let Some(start_rel) = text[search..].find(&needle) {
            let start = search + start_rel;
            let open = start + command.len();
            if let Some(close) = find_matching(text, open, '(', ')') {
                let call = &text[start..=close];
                for arg in &args {
                    if let Ok((parsed, arg_name, normalizations, relative_start)) =
                        parse_command_arg(call, arg, &options.repair_limits)
                    {
                        let mut candidate = base_candidate(
                            candidates.len(),
                            CandidateGrammar::PythonStyleCommand,
                            Some(SourceRange::new(start, close + 1, index)),
                        );
                        candidate.command_name = Some(command.clone());
                        candidate.argument_name = Some(arg_name);
                        candidate.normalizations = normalizations;
                        adopt_jsonish_parse(&mut candidate, parsed, start + relative_start, index);
                        if !candidate.repairs.is_empty() {
                            candidate.status = CandidateStatus::Recovered;
                        }
                        candidates.push(candidate);
                        break;
                    }
                }
                if !candidates.iter().any(|candidate| {
                    candidate.raw_range.as_ref().is_some_and(|range| {
                        range.byte_start == start && range.byte_end == close + 1
                    })
                }) {
                    if let Ok((parsed, arg_name, normalizations, relative_start)) =
                        parse_positional_command_arg(call, &options.repair_limits)
                    {
                        let mut candidate = base_candidate(
                            candidates.len(),
                            CandidateGrammar::PythonStyleCommand,
                            Some(SourceRange::new(start, close + 1, index)),
                        );
                        candidate.command_name = Some(command.clone());
                        candidate.argument_name = Some(arg_name);
                        candidate.normalizations = normalizations;
                        adopt_jsonish_parse(&mut candidate, parsed, start + relative_start, index);
                        if !candidate.repairs.is_empty() {
                            candidate.status = CandidateStatus::Recovered;
                        }
                        candidates.push(candidate);
                    }
                }
                search = close + 1;
            } else {
                break;
            }
        }
    }
}

fn extract_json_candidates(
    text: &str,
    index: &LineIndex,
    limits: &RepairLimits,
    candidates: &mut Vec<ModelOutputCandidate>,
) {
    let trimmed = text.trim();
    let start = text.find(trimmed).unwrap_or(0);
    let raw_failure = match parse_jsonish_value_with_limits(trimmed, limits) {
        Ok(parsed) => {
            let mut candidate = base_candidate(
                candidates.len(),
                CandidateGrammar::RawJson,
                Some(SourceRange::new(start, start + trimmed.len(), index)),
            );
            adopt_jsonish_parse(&mut candidate, parsed, start, index);
            if !candidate.repairs.is_empty() {
                candidate.status = CandidateStatus::Recovered;
            }
            classify_json_tool_shape(&mut candidate, limits, text, index);
            candidates.push(candidate);
            return;
        }
        Err(failure) => failure,
    };
    let raw_error_absolute = start + pending_json_error(trimmed, &raw_failure.error).byte_offset;
    let mut covered_until = 0usize;
    for (absolute_start, absolute_end) in balanced_json_spans(text) {
        if absolute_start < covered_until
            || ((trimmed.starts_with('{') || trimmed.starts_with('['))
                && absolute_start < raw_error_absolute)
        {
            continue;
        }
        let slice = &text[absolute_start..absolute_end];
        if let Ok(parsed) = parse_jsonish_value_with_limits(slice, limits) {
            if is_duplicate_of_existing_candidate(
                absolute_start,
                absolute_end,
                &parsed.value,
                candidates,
            ) {
                covered_until = absolute_end;
                continue;
            }
            let mut candidate = base_candidate(
                candidates.len(),
                CandidateGrammar::JsonObjectInText,
                Some(SourceRange::new(absolute_start, absolute_end, index)),
            );
            candidate
                .normalizations
                .push("extracted_balanced_json".into());
            adopt_jsonish_parse(&mut candidate, parsed, absolute_start, index);
            if !candidate.repairs.is_empty() {
                candidate.status = CandidateStatus::Recovered;
            }
            classify_json_tool_shape(&mut candidate, limits, text, index);
            candidates.push(candidate);
            // Match the previous outermost-candidate behavior: once a complete
            // container parses, do not also emit each nested container.
            covered_until = absolute_end;
        }
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let mut candidate = base_candidate(
            candidates.len(),
            CandidateGrammar::RawJson,
            Some(SourceRange::new(start, start + trimmed.len(), index)),
        );
        retain_json_failure(&mut candidate, &raw_failure, trimmed, start, index);
        candidate.normalizations.push(
            if candidate.status == CandidateStatus::Incomplete {
                "preserved_incomplete_raw_json"
            } else {
                "preserved_malformed_raw_json"
            }
            .into(),
        );
        candidates.push(candidate);
    }
}

fn is_duplicate_of_existing_candidate(
    start: usize,
    end: usize,
    value: &Value,
    candidates: &[ModelOutputCandidate],
) -> bool {
    candidates.iter().any(|candidate| {
        candidate.value.as_ref() == Some(value)
            && candidate.raw_range.as_ref().is_some_and(|range| {
                range.byte_start <= start
                    && end <= range.byte_end
                    && (range.byte_start != start || range.byte_end != end)
            })
    })
}

fn classify_json_tool_shape(
    candidate: &mut ModelOutputCandidate,
    limits: &RepairLimits,
    source_text: &str,
    source_index: &LineIndex,
) {
    let Some(value) = candidate.value.clone() else {
        return;
    };
    if value.get("jsonrpc").is_some() && value.get("method").is_some() {
        candidate.grammar = CandidateGrammar::McpJsonRpc;
        candidate.command_name = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(params) = value.get("params").cloned() {
            candidate.argument_name = Some("params".into());
            candidate.value = Some(params);
        }
    } else if let Some(function) = value.get("function") {
        apply_openai_call(
            candidate,
            function,
            "parsed_stringified_arguments",
            limits,
            source_text,
            source_index,
        );
    } else if let Some(tool_calls) = openai_tool_calls(&value) {
        candidate.grammar = CandidateGrammar::OpenAiToolCall;
        if tool_calls.len() == 1 {
            if let Some(first_call) = tool_calls.first() {
                let function = first_call.get("function").unwrap_or(first_call);
                apply_openai_call(
                    candidate,
                    function,
                    "parsed_first_tool_call_arguments",
                    limits,
                    source_text,
                    source_index,
                );
            }
        }
    } else if let Some(content) = openai_chat_content(&value).map(str::to_string) {
        candidate.grammar = CandidateGrammar::OpenAiChatContent;
        candidate.argument_name = Some("content".into());
        if let Ok(parsed) = parse_nested_jsonish_value_with_limits(&content, limits) {
            adopt_nested_jsonish_parse(
                candidate,
                parsed,
                "parsed_openai_chat_message_content",
                &content,
                source_text,
                source_index,
            );
        } else if let Some((start, end)) = first_balanced_json_value(&content) {
            if let Ok(parsed) = parse_jsonish_value(&content[start..end]) {
                candidate.value = Some(parsed);
                candidate
                    .normalizations
                    .push("extracted_json_from_openai_chat_message_content".into());
            }
        }
    } else if value.get("name").is_some() && value.get("arguments").is_some() {
        apply_openai_call(
            candidate,
            &value,
            "parsed_root_stringified_arguments",
            limits,
            source_text,
            source_index,
        );
    } else {
        unwrap_stringified_json(candidate, limits, source_text, source_index);
    }
}

fn openai_tool_calls(value: &Value) -> Option<&Vec<Value>> {
    value
        .get("tool_calls")
        .and_then(Value::as_array)
        .or_else(|| {
            value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message").or_else(|| choice.get("delta")))
                .and_then(|message| message.get("tool_calls"))
                .and_then(Value::as_array)
        })
}

fn apply_openai_call(
    candidate: &mut ModelOutputCandidate,
    call: &Value,
    stringified_normalization: &str,
    limits: &RepairLimits,
    source_text: &str,
    source_index: &LineIndex,
) {
    candidate.grammar = CandidateGrammar::OpenAiToolCall;
    candidate.command_name = call.get("name").and_then(Value::as_str).map(str::to_string);
    if let Some(arguments) = call.get("arguments") {
        candidate.argument_name = Some("arguments".into());
        candidate.value = Some(arguments.clone());
        if let Some(arguments_str) = arguments.as_str() {
            if let Ok(parsed) = parse_nested_jsonish_value_with_limits(arguments_str, limits) {
                adopt_nested_jsonish_parse(
                    candidate,
                    parsed,
                    stringified_normalization,
                    arguments_str,
                    source_text,
                    source_index,
                );
            }
        }
    }
}

fn expand_compound_json_candidates(
    candidates: Vec<ModelOutputCandidate>,
    source_text: &str,
    index: &LineIndex,
    limits: &RepairLimits,
) -> Vec<ModelOutputCandidate> {
    let mut expanded = Vec::new();
    for candidate in candidates {
        let Some(value) = candidate.value.as_ref() else {
            expanded.push(candidate);
            continue;
        };
        let calls = compound_json_calls(value);
        if calls.is_empty() {
            expanded.push(candidate);
            continue;
        }
        let parent_start = candidate
            .raw_range
            .as_ref()
            .map_or(0, |range| range.byte_start);
        let parent_text = candidate
            .raw_range
            .as_ref()
            .and_then(|range| source_text.get(range.byte_start..range.byte_end));
        let mut call_search_start = 0usize;
        for (grammar, call) in calls {
            let mut item = candidate.clone();
            if let Some((start, end)) =
                parent_text.and_then(|text| find_json_value_span(text, call, call_search_start))
            {
                let absolute_start = parent_start + start;
                let absolute_end = parent_start + end;
                item.raw_range = Some(SourceRange::new(absolute_start, absolute_end, index));
                item.raw_text = source_text
                    .get(absolute_start..absolute_end)
                    .map(str::to_string);
                call_search_start = end;
            } else {
                item.diagnostics.push(
                    Diagnostic::warning(
                        "grist.model_output.json",
                        "json.compound_candidate_range",
                        "compound JSON candidate retained with its enclosing source range",
                    )
                    .partial(),
                );
            }
            match grammar {
                CandidateGrammar::McpJsonRpc => {
                    item.grammar = CandidateGrammar::McpJsonRpc;
                    item.command_name = call
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(params) = call.get("params") {
                        item.argument_name = Some("params".into());
                        item.value = Some(params.clone());
                    } else {
                        item.value = Some(call.clone());
                    }
                    item.normalizations.push("extracted_mcp_batch_item".into());
                }
                CandidateGrammar::OpenAiToolCall => {
                    let function = call.get("function").unwrap_or(call);
                    apply_openai_call(
                        &mut item,
                        function,
                        "parsed_stringified_arguments",
                        limits,
                        source_text,
                        index,
                    );
                    item.normalizations
                        .push("extracted_openai_tool_call".into());
                }
                _ => unreachable!("compound call grammars are closed"),
            }
            expanded.push(item);
        }
    }
    expanded
}

fn find_json_value_span(text: &str, target: &Value, search_start: usize) -> Option<(usize, usize)> {
    balanced_json_spans(text)
        .into_iter()
        .filter(|(start, _)| *start >= search_start)
        .find(|(start, end)| {
            parse_jsonish_value_with_repairs(&text[*start..*end])
                .map(|parsed| parsed.value == *target)
                .unwrap_or(false)
        })
}

/// Return every balanced JSON object/array span in source order, with enclosing
/// containers ordered before nested containers that begin at the same byte.
///
/// This is a single pass over hostile input. In particular, an arbitrarily long
/// prefix of unmatched opening delimiters cannot hide a later balanced value or
/// force the quadratic rescanning that a fixed probe limit was preventing.
fn balanced_json_spans(text: &str) -> Vec<(usize, usize)> {
    let mut stack = Vec::<(u8, usize)>::new();
    let mut spans = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (offset, byte) in text.bytes().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => stack.push((byte, offset)),
            b'}' | b']' => {
                let expected = if byte == b'}' { b'{' } else { b'[' };
                if let Some(position) = stack.iter().rposition(|(open, _)| *open == expected) {
                    let start = stack[position].1;
                    stack.truncate(position);
                    spans.push((start, offset + 1));
                }
            }
            _ => {}
        }
    }

    spans.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));
    spans
}

fn compound_json_calls(value: &Value) -> Vec<(CandidateGrammar, &Value)> {
    if let Some(items) = value.as_array() {
        let calls = items
            .iter()
            .filter_map(|item| {
                if item.get("jsonrpc").is_some() && item.get("method").is_some() {
                    Some((CandidateGrammar::McpJsonRpc, item))
                } else if item.get("function").is_some()
                    || (item.get("name").is_some() && item.get("arguments").is_some())
                {
                    Some((CandidateGrammar::OpenAiToolCall, item))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !calls.is_empty() && calls.len() == items.len() {
            return calls;
        }
    }
    if let Some(items) = value.get("output").and_then(Value::as_array) {
        let calls = items
            .iter()
            .filter(|item| {
                item.get("type").and_then(Value::as_str) == Some("function_call")
                    || (item.get("name").is_some() && item.get("arguments").is_some())
            })
            .map(|item| (CandidateGrammar::OpenAiToolCall, item))
            .collect::<Vec<_>>();
        if !calls.is_empty() {
            return calls;
        }
    }
    openai_tool_calls(value)
        .filter(|calls| !calls.is_empty())
        .map(|calls| {
            calls
                .iter()
                .map(|call| (CandidateGrammar::OpenAiToolCall, call))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_nested_jsonish_value_with_limits(
    text: &str,
    limits: &RepairLimits,
) -> Result<JsonishParse, JsonishParseFailure> {
    let mut parsed = parse_jsonish_value_with_limits(text, limits)?;
    for _ in 0..8 {
        let Value::String(nested) = &parsed.value else {
            break;
        };
        let trimmed = nested.trim();
        if !(trimmed.starts_with('{') || trimmed.starts_with('[') || trimmed.starts_with('"')) {
            break;
        }
        parsed = parse_jsonish_value_with_limits(trimmed, limits)?;
    }
    Ok(parsed)
}

fn unwrap_stringified_json(
    candidate: &mut ModelOutputCandidate,
    limits: &RepairLimits,
    source_text: &str,
    source_index: &LineIndex,
) {
    let Some(Value::String(text)) = candidate.value.as_ref() else {
        return;
    };
    let nested_text = text.clone();
    let Ok(parsed) = parse_nested_jsonish_value_with_limits(&nested_text, limits) else {
        return;
    };
    if matches!(parsed.value, Value::String(_)) {
        return;
    }
    adopt_nested_jsonish_parse(
        candidate,
        parsed,
        "parsed_stringified_json",
        &nested_text,
        source_text,
        source_index,
    );
    classify_json_tool_shape(candidate, limits, source_text, source_index);
}
fn parse_command_arg(
    call: &str,
    arg_name: &str,
    limits: &RepairLimits,
) -> Result<(JsonishParse, String, Vec<String>, usize), String> {
    let Some(start) = find_argument_value_start(call, arg_name) else {
        return Err("missing arg".into());
    };
    let after = &call[start..];
    let Some((object_start, object_end)) = first_balanced_json_value(after) else {
        return Err("missing balanced JSON argument".into());
    };
    let object = &after[object_start..object_end];
    let parsed = parse_jsonish_value_with_limits(object, limits).map_err(|err| err.to_string())?;
    Ok((
        parsed,
        arg_name.to_string(),
        vec!["parsed_python_style_command".into()],
        start + object_start,
    ))
}

fn parse_positional_command_arg(
    call: &str,
    limits: &RepairLimits,
) -> Result<(JsonishParse, String, Vec<String>, usize), String> {
    let Some(open) = call.find('(') else {
        return Err("missing open paren".into());
    };
    let Some(close) = call.rfind(')') else {
        return Err("missing close paren".into());
    };
    if close <= open {
        return Err("empty call".into());
    }
    let args = &call[open + 1..close];
    let Some((object_start, object_end)) = first_balanced_json_value(args) else {
        return Err("missing balanced positional JSON argument".into());
    };
    let parsed = parse_jsonish_value_with_limits(&args[object_start..object_end], limits)
        .map_err(|err| err.to_string())?;
    Ok((
        parsed,
        "positional".to_string(),
        vec!["parsed_python_style_positional_command".into()],
        open + 1 + object_start,
    ))
}

fn openai_chat_content(value: &Value) -> Option<&str> {
    let choices = value.get("choices")?.as_array()?;
    let first_choice = choices.first()?;
    first_choice
        .get("message")
        .and_then(|message| message.get("content"))
        .or_else(|| {
            first_choice
                .get("delta")
                .and_then(|delta| delta.get("content"))
        })?
        .as_str()
}

fn select_candidate_id(
    candidates: &[ModelOutputCandidate],
    options: &ModelOutputOptions,
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    if options.schema.is_some() {
        let valid = candidates
            .iter()
            .filter(|candidate| {
                candidate.value.is_some()
                    && candidate
                        .validation
                        .as_ref()
                        .map(|validation| validation.valid)
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        match valid.as_slice() {
            [candidate] => return Some(candidate.id.clone()),
            [] => {}
            _ if candidates_are_duplicate_interpretations(&valid) => {
                return select_highest_priority_candidate(&valid)
                    .map(|candidate| candidate.id.clone());
            }
            _ => return None,
        }
    }
    if candidates.len() == 1 {
        return Some(candidates[0].id.clone());
    }
    let complete = candidates
        .iter()
        .filter(|candidate| {
            candidate.value.is_some()
                && matches!(
                    candidate.status,
                    CandidateStatus::Complete | CandidateStatus::Recovered
                )
        })
        .collect::<Vec<_>>();
    match complete.as_slice() {
        [candidate] => Some(candidate.id.clone()),
        _ if candidates_are_duplicate_interpretations(&complete) => {
            select_highest_priority_candidate(&complete).map(|candidate| candidate.id.clone())
        }
        _ => None,
    }
}

fn candidates_are_duplicate_interpretations(candidates: &[&ModelOutputCandidate]) -> bool {
    let Some(first) = candidates.first() else {
        return false;
    };
    let Some(first_range) = first.raw_range.as_ref() else {
        return false;
    };
    candidates.iter().skip(1).all(|candidate| {
        candidate.value == first.value
            && candidate.raw_range.as_ref().is_some_and(|range| {
                range.byte_start < first_range.byte_end && first_range.byte_start < range.byte_end
            })
    })
}
fn select_highest_priority_candidate<'a>(
    candidates: &[&'a ModelOutputCandidate],
) -> Option<&'a ModelOutputCandidate> {
    let max_priority = candidates
        .iter()
        .map(|candidate| candidate_grammar_priority(&candidate.grammar))
        .max()?;
    let mut top = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate_grammar_priority(&candidate.grammar) == max_priority);
    let selected = top.next()?;
    top.next().is_none().then_some(selected)
}

fn candidate_grammar_priority(grammar: &CandidateGrammar) -> u8 {
    match grammar {
        CandidateGrammar::RawJson => 100,
        CandidateGrammar::FencedJson => 95,
        CandidateGrammar::JsonObjectInText => 90,
        CandidateGrammar::OpenAiChatContent => 85,
        CandidateGrammar::OpenAiToolCall => 80,
        CandidateGrammar::McpJsonRpc => 80,
        CandidateGrammar::FencedCode => 60,
        CandidateGrammar::YamlBlock => 55,
        CandidateGrammar::TomlBlock => 55,
        CandidateGrammar::XmlToolCall => 45,
        CandidateGrammar::PythonStyleCommand => 20,
    }
}

fn apply_aliases(candidate: &mut ModelOutputCandidate, aliases: &AliasRules) {
    if let Some(command) = &candidate.command_name {
        if let Some(alias) = aliases
            .command_aliases
            .iter()
            .find(|rule| rule.from == *command)
        {
            candidate.alias_applications.push(AliasApplication {
                kind: AliasKind::Command,
                from: alias.from.clone(),
                to: alias.to.clone(),
                original_value: Some(Value::String(command.clone())),
                normalized_value: Some(Value::String(alias.to.clone())),
            });
            candidate.command_name = Some(alias.to.clone());
            candidate
                .normalizations
                .push(format!("command_alias:{}->{}", alias.from, alias.to));
        }
    }
    if let Some(argument) = &candidate.argument_name {
        if let Some(alias) = aliases
            .argument_aliases
            .iter()
            .find(|rule| rule.from == *argument)
        {
            candidate.alias_applications.push(AliasApplication {
                kind: AliasKind::Argument,
                from: alias.from.clone(),
                to: alias.to.clone(),
                original_value: Some(Value::String(argument.clone())),
                normalized_value: Some(Value::String(alias.to.clone())),
            });
            candidate.argument_name = Some(alias.to.clone());
            candidate
                .normalizations
                .push(format!("argument_alias:{}->{}", alias.from, alias.to));
        }
    }
    if let Some(Value::Object(map)) = candidate.value.as_mut() {
        for alias in &aliases.field_aliases {
            if let Some(value) = map.remove(&alias.from) {
                let original_value = value.clone();
                let normalized_value = map.entry(alias.to.clone()).or_insert(value).clone();
                candidate.alias_applications.push(AliasApplication {
                    kind: AliasKind::Field,
                    from: alias.from.clone(),
                    to: alias.to.clone(),
                    original_value: Some(original_value),
                    normalized_value: Some(normalized_value),
                });
                candidate
                    .normalizations
                    .push(format!("field_alias:{}->{}", alias.from, alias.to));
            }
        }
    }
}

fn base_candidate(
    id: usize,
    grammar: CandidateGrammar,
    range: Option<SourceRange>,
) -> ModelOutputCandidate {
    ModelOutputCandidate {
        id: format!("candidate-{id}"),
        grammar,
        command_name: None,
        argument_name: None,
        original_value: None,
        value: None,
        raw_text: None,
        raw_range: range,
        repaired_text: None,
        repairs: Vec::new(),
        alias_applications: Vec::new(),
        json_error: None,
        status: CandidateStatus::Complete,
        confidence: 0.8,
        normalizations: Vec::new(),
        diagnostics: Vec::new(),
        validation: None,
        trusted: false,
    }
}

fn adopt_jsonish_parse(
    candidate: &mut ModelOutputCandidate,
    parsed: JsonishParse,
    source_start: usize,
    source_index: &LineIndex,
) {
    candidate.original_value = parsed.original_value;
    candidate.value = Some(parsed.value);
    candidate.repaired_text = parsed.repaired_text;
    candidate.normalizations.extend(parsed.normalizations);
    candidate.repairs = parsed
        .repairs
        .into_iter()
        .map(|mut pending| {
            if pending.source_relative {
                let start = source_start + pending.operation.input_byte_start;
                let end = source_start + pending.operation.input_byte_end;
                pending.operation.source_range = Some(SourceRange::new(start, end, source_index));
            }
            pending.operation
        })
        .collect();
    if let Some(error) = parsed.original_error {
        let absolute = source_start + error.byte_offset;
        let location = source_index.line_column(absolute);
        candidate.json_error = Some(JsonErrorPosition {
            byte_offset: absolute,
            line: location.line,
            column: location.column,
            message: error.message,
        });
    }
    for repair in &candidate.repairs {
        let mut diagnostic = Diagnostic::warning(
            "grist.model_output.json",
            "json.repaired",
            format!("applied bounded JSON repair {:?}", repair.kind),
        )
        .partial();
        if let Some(range) = repair.source_range.clone() {
            diagnostic = diagnostic.with_range(range);
        }
        candidate.diagnostics.push(diagnostic);
    }
}

fn adopt_nested_jsonish_parse(
    candidate: &mut ModelOutputCandidate,
    parsed: JsonishParse,
    normalization: &str,
    nested_text: &str,
    source_text: &str,
    source_index: &LineIndex,
) {
    candidate.value = Some(parsed.value);
    if candidate.repaired_text.is_none() {
        candidate.repaired_text = parsed.repaired_text;
    }
    candidate.normalizations.extend(parsed.normalizations);
    candidate.normalizations.push(normalization.into());

    let repair_start = candidate.repairs.len();
    let mut appended_repairs = Vec::new();
    for mut pending in parsed.repairs {
        if pending.source_relative
            && let (Some(start), Some(end)) = (
                nested_literal_source_offset(
                    candidate,
                    source_text,
                    nested_text,
                    pending.operation.input_byte_start,
                ),
                nested_literal_source_offset(
                    candidate,
                    source_text,
                    nested_text,
                    pending.operation.input_byte_end,
                ),
            )
        {
            pending.operation.source_range = Some(SourceRange::new(start, end, source_index));
        }
        appended_repairs.push(pending.operation);
    }
    candidate.repairs.extend(appended_repairs);

    if candidate.json_error.is_none()
        && let Some(error) = parsed.original_error
        && let Some(absolute) =
            nested_literal_source_offset(candidate, source_text, nested_text, error.byte_offset)
    {
        let location = source_index.line_column(absolute);
        candidate.json_error = Some(JsonErrorPosition {
            byte_offset: absolute,
            line: location.line,
            column: location.column,
            message: error.message,
        });
    }

    for repair in &candidate.repairs[repair_start..] {
        let mut diagnostic = Diagnostic::warning(
            "grist.model_output.json",
            "json.repaired",
            format!("applied bounded nested JSON repair {:?}", repair.kind),
        )
        .partial();
        if let Some(range) = repair.source_range.clone() {
            diagnostic = diagnostic.with_range(range);
        }
        candidate.diagnostics.push(diagnostic);
    }
    candidate.status = CandidateStatus::Recovered;
}

fn nested_literal_source_offset(
    candidate: &ModelOutputCandidate,
    source_text: &str,
    nested_text: &str,
    decoded_offset: usize,
) -> Option<usize> {
    let range = candidate.raw_range.as_ref()?;
    let raw = source_text.get(range.byte_start..range.byte_end)?;
    let literal = serde_json::to_string(nested_text).ok()?;
    let relative_start = raw.find(&literal)?;
    let literal_offset = decoded_offset_to_json_literal_offset(&literal, decoded_offset)?;
    Some(range.byte_start + relative_start + literal_offset)
}

fn decoded_offset_to_json_literal_offset(literal: &str, target: usize) -> Option<usize> {
    let bytes = literal.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut encoded = 1usize;
    let mut decoded = 0usize;
    while decoded < target {
        match bytes.get(encoded).copied()? {
            b'\\' => {
                let escape = bytes.get(encoded + 1).copied()?;
                if escape == b'u' {
                    // Mapping UTF-16 escape pairs to UTF-8 byte offsets is not
                    // safely one-to-one. Keep the repair but omit its range.
                    return None;
                }
                encoded += 2;
                decoded += 1;
            }
            b'"' => return None,
            _ => {
                let ch = literal.get(encoded..)?.chars().next()?;
                encoded += ch.len_utf8();
                decoded += ch.len_utf8();
            }
        }
    }
    (decoded == target).then_some(encoded)
}

fn retain_json_failure(
    candidate: &mut ModelOutputCandidate,
    failure: &JsonishParseFailure,
    source_text: &str,
    source_start: usize,
    source_index: &LineIndex,
) {
    let pending = pending_json_error(source_text, &failure.error);
    let absolute = source_start + pending.byte_offset;
    let location = source_index.line_column(absolute);
    let error = JsonErrorPosition {
        byte_offset: absolute,
        line: location.line,
        column: location.column,
        message: pending.message,
    };
    candidate.status = if pending.incomplete {
        CandidateStatus::Incomplete
    } else {
        CandidateStatus::Malformed
    };
    candidate.confidence = 0.4;
    candidate.json_error = Some(error.clone());
    let code = if failure.repair_limit_exceeded {
        "json.repair_bounds_exceeded"
    } else if pending.incomplete {
        "json.incomplete"
    } else {
        "json.malformed"
    };
    let message = format!(
        "JSON candidate is {} at byte {} (line {}, column {}): {}",
        if pending.incomplete {
            "incomplete"
        } else {
            "malformed"
        },
        error.byte_offset,
        error.line,
        error.column,
        error.message
    );
    let range_end = absolute
        .saturating_add(1)
        .min(source_start + source_text.len());
    let mut diagnostic = if pending.incomplete {
        Diagnostic::warning("grist.model_output.json", code, message).partial()
    } else {
        Diagnostic::error("grist.model_output.json", code, message)
    };
    diagnostic = diagnostic.with_range(SourceRange::new(absolute, range_end, source_index));
    candidate.diagnostics.push(diagnostic);
}

fn failures_from_candidate(
    candidate: &ModelOutputCandidate,
    source_text: &str,
) -> Vec<ModelOutputFailure> {
    let error_diagnostics: Vec<Diagnostic> = candidate
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == crate::core::Severity::Error)
        .cloned()
        .collect();
    if error_diagnostics.is_empty() && candidate.status != CandidateStatus::Malformed {
        return Vec::new();
    }
    let raw_text = candidate
        .raw_range
        .as_ref()
        .and_then(|range| source_text.get(range.byte_start..range.byte_end))
        .unwrap_or("")
        .to_string();
    vec![failure_record(
        0,
        if candidate.status == CandidateStatus::Malformed {
            "malformed_candidate"
        } else if candidate
            .validation
            .as_ref()
            .is_some_and(|validation| !validation.valid)
        {
            "schema_validation_failed"
        } else {
            "candidate_error"
        },
        Some(candidate.grammar.clone()),
        &raw_text,
        candidate.raw_range.clone(),
        error_diagnostics,
        true,
    )]
}

fn failure_record(
    id: usize,
    failure_mode: impl Into<String>,
    grammar: Option<CandidateGrammar>,
    raw_text: &str,
    raw_range: Option<SourceRange>,
    diagnostics: Vec<Diagnostic>,
    recoverable: bool,
) -> ModelOutputFailure {
    ModelOutputFailure {
        id: format!("failure-{id}"),
        failure_mode: failure_mode.into(),
        grammar,
        raw_text: raw_text.to_string(),
        raw_text_sha256: crate::core::sha256_hex(raw_text.as_bytes()),
        raw_range,
        diagnostics,
        recoverable,
    }
}

struct JsonishParse {
    value: Value,
    original_value: Option<Value>,
    repaired_text: Option<String>,
    normalizations: Vec<String>,
    repairs: Vec<PendingRepair>,
    original_error: Option<PendingJsonError>,
}

struct PendingRepair {
    operation: RepairOperation,
    source_relative: bool,
}

struct PendingJsonError {
    byte_offset: usize,
    message: String,
    incomplete: bool,
}

#[derive(Debug)]
struct JsonishParseFailure {
    error: serde_json::Error,
    repair_limit_exceeded: bool,
}

impl std::fmt::Display for JsonishParseFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.repair_limit_exceeded {
            write!(formatter, "{} (repair bounds exceeded)", self.error)
        } else {
            self.error.fmt(formatter)
        }
    }
}

fn parse_jsonish_value(text: &str) -> Result<Value, serde_json::Error> {
    parse_jsonish_value_with_repairs(text)
        .map(|parsed| parsed.value)
        .map_err(|failure| failure.error)
}

fn parse_jsonish_value_with_repairs(text: &str) -> Result<JsonishParse, JsonishParseFailure> {
    parse_jsonish_value_with_limits(text, &RepairLimits::default())
}

fn parse_jsonish_value_with_limits(
    text: &str,
    limits: &RepairLimits,
) -> Result<JsonishParse, JsonishParseFailure> {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => Ok(JsonishParse {
            original_value: Some(value.clone()),
            value,
            repaired_text: None,
            normalizations: Vec::new(),
            repairs: Vec::new(),
            original_error: None,
        }),
        Err(original_err) => {
            let original_error = pending_json_error(text, &original_err);
            let repair = fix_jsonish_with_normalizations(text, limits);
            if repair.limit_exceeded {
                return Err(JsonishParseFailure {
                    error: original_err,
                    repair_limit_exceeded: true,
                });
            }
            serde_json::from_str(&repair.text)
                .map(|value| JsonishParse {
                    value,
                    original_value: None,
                    repaired_text: Some(repair.text),
                    normalizations: repair.normalizations,
                    repairs: repair.repairs,
                    original_error: Some(original_error),
                })
                .map_err(|_| JsonishParseFailure {
                    error: original_err,
                    repair_limit_exceeded: false,
                })
        }
    }
}

struct RepairAttempt {
    text: String,
    normalizations: Vec<String>,
    repairs: Vec<PendingRepair>,
    limit_exceeded: bool,
}

fn fix_jsonish_with_normalizations(input: &str, limits: &RepairLimits) -> RepairAttempt {
    let mut attempt = RepairAttempt {
        text: input.trim().to_string(),
        normalizations: Vec::new(),
        repairs: Vec::new(),
        limit_exceeded: false,
    };
    let mut changed_bytes = 0usize;
    let mut source_pristine = attempt.text == input;

    let stages: &[(RepairKind, &str, fn(&str) -> String)] = &[
        (
            RepairKind::RemovePrematureClosingBrace,
            "removed_premature_closing_brace",
            remove_unambiguous_premature_closing_brace,
        ),
        (
            RepairKind::ReplacePythonLiteral,
            "replaced_python_literals",
            replace_python_literals,
        ),
        (
            RepairKind::RemoveRedundantObjectOpener,
            "removed_redundant_object_opener",
            remove_redundant_object_openers,
        ),
        (
            RepairKind::QuoteSingleQuotedString,
            "quoted_single_quoted_strings",
            quote_single_quoted_strings,
        ),
        (
            RepairKind::QuoteUnquotedObjectKey,
            "quoted_unquoted_object_keys",
            quote_unquoted_object_keys,
        ),
        (
            RepairKind::PreserveBackslashCommand,
            "preserved_backslash_command",
            preserve_backslash_commands_in_strings,
        ),
        (
            RepairKind::EscapeUnescapedStringQuote,
            "escaped_unescaped_string_quote",
            escape_unescaped_string_boundary_quotes,
        ),
        (
            RepairKind::InsertMissingComma,
            "inserted_missing_comma",
            insert_missing_commas_between_members,
        ),
        (
            RepairKind::RemoveTrailingComma,
            "removed_trailing_commas",
            remove_trailing_commas,
        ),
        (
            RepairKind::CloseUnterminatedContainer,
            "closed_unterminated_object",
            complete_unterminated_json_containers,
        ),
    ];

    for (kind, normalization, repair) in stages {
        // Each stage is a fallback for still-invalid JSON. Once a bounded edit
        // produces valid JSON, later heuristics must not rewrite valid data.
        if serde_json::from_str::<Value>(&attempt.text).is_ok() {
            break;
        }
        if !apply_repair(
            &mut attempt,
            &mut changed_bytes,
            &mut source_pristine,
            limits,
            kind.clone(),
            normalization,
            *repair,
        ) {
            attempt.limit_exceeded = true;
            break;
        }
    }
    attempt
}

fn pending_json_error(text: &str, error: &serde_json::Error) -> PendingJsonError {
    let line_start = text
        .split_inclusive('\n')
        .take(error.line().saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    let byte_offset = line_start
        .saturating_add(error.column().saturating_sub(1))
        .min(text.len());
    PendingJsonError {
        byte_offset,
        message: error.to_string(),
        incomplete: error.is_eof(),
    }
}

fn replace_python_literals(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut offset = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    while offset < bytes.len() {
        let byte = bytes[offset];
        if in_string {
            output.push(byte as char);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            offset += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            output.push('"');
            offset += 1;
            continue;
        }

        let replacement = [("None", "null"), ("True", "true"), ("False", "false")]
            .into_iter()
            .find(|(token, _)| {
                input[offset..].starts_with(token)
                    && !bytes
                        .get(offset.wrapping_sub(1))
                        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    && !bytes
                        .get(offset + token.len())
                        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            });
        if let Some((token, replacement)) = replacement {
            output.push_str(replacement);
            offset += token.len();
        } else {
            let ch = input[offset..]
                .chars()
                .next()
                .expect("offset remains on a UTF-8 boundary");
            output.push(ch);
            offset += ch.len_utf8();
        }
    }
    output
}

/// Delete exactly one premature tool-call object closer only when that one-byte
/// edit makes the complete candidate valid JSON. Multiple possible edits fail
/// closed, as do ordinary objects without a recognizable tool-call shape.
fn remove_unambiguous_premature_closing_brace(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut stack = Vec::<(u8, usize)>::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut successful = Vec::new();

    for (offset, byte) in bytes.iter().copied().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => stack.push((byte, offset)),
            b'}' => {
                if let Some((b'{', object_start)) = stack.last().copied()
                    && follows_comma_and_quoted_member(bytes, offset + 1)
                    && looks_like_tool_call_prefix(&input[object_start..offset])
                {
                    let mut repaired = String::with_capacity(input.len() - 1);
                    repaired.push_str(&input[..offset]);
                    repaired.push_str(&input[offset + 1..]);
                    if serde_json::from_str::<Value>(&repaired).is_ok() {
                        successful.push(repaired);
                    }
                }
                if stack.last().is_some_and(|(open, _)| *open == b'{') {
                    stack.pop();
                }
            }
            b']' => {
                if stack.last().is_some_and(|(open, _)| *open == b'[') {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    match successful.as_slice() {
        [repaired] => repaired.clone(),
        _ => input.to_string(),
    }
}

fn follows_comma_and_quoted_member(bytes: &[u8], mut offset: usize) -> bool {
    while bytes.get(offset).is_some_and(u8::is_ascii_whitespace) {
        offset += 1;
    }
    if bytes.get(offset) != Some(&b',') {
        return false;
    }
    offset += 1;
    while bytes.get(offset).is_some_and(u8::is_ascii_whitespace) {
        offset += 1;
    }
    if bytes.get(offset) != Some(&b'"') {
        return false;
    }
    offset += 1;
    let mut escaped = false;
    while let Some(byte) = bytes.get(offset).copied() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            offset += 1;
            break;
        }
        offset += 1;
    }
    while bytes.get(offset).is_some_and(u8::is_ascii_whitespace) {
        offset += 1;
    }
    bytes.get(offset) == Some(&b':')
}

fn looks_like_tool_call_prefix(prefix: &str) -> bool {
    let mut closed = String::with_capacity(prefix.len() + 1);
    closed.push_str(prefix);
    closed.push('}');
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(&closed) else {
        return false;
    };
    let direct_call = object.contains_key("name") && object.contains_key("arguments");
    let direct_rpc = object.contains_key("method") && object.contains_key("params");
    let wrapped_call = object
        .get("function")
        .and_then(Value::as_object)
        .is_some_and(|function| {
            function.contains_key("name") && function.contains_key("arguments")
        });
    direct_call || direct_rpc || wrapped_call
}

/// Turns model-emitted command text such as `\lambda`, `\Delta`, and `\frac`
/// into valid JSON string content. A multi-letter ASCII command is treated as
/// literal backslash text even when its first letter (`b`, `f`, `n`, `r`, or
/// `t`) is also a valid one-character JSON escape.
fn preserve_backslash_commands_in_strings(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            in_string = !in_string;
            out.push(ch);
            i += 1;
            continue;
        }
        if in_string && ch == '\\' {
            let Some(&next) = chars.get(i + 1) else {
                out.push(ch);
                i += 1;
                continue;
            };
            if next == '\\' || next == '"' || next == '/' {
                out.push(ch);
                out.push(next);
                i += 2;
                continue;
            }
            let command_len = chars[i + 1..]
                .iter()
                .take_while(|candidate| candidate.is_ascii_alphabetic())
                .count();
            let valid_unicode_escape = next == 'u'
                && chars
                    .get(i + 2..i + 6)
                    .is_some_and(|digits| digits.iter().all(|digit| digit.is_ascii_hexdigit()));
            let invalid_single_escape = !matches!(next, 'b' | 'f' | 'n' | 'r' | 't' | 'u');
            if !valid_unicode_escape && (command_len > 1 || invalid_single_escape) {
                out.push('\\');
            }
            out.push(ch);
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Removes the common model slip `{ { "key": ... }` while leaving braces in
/// strings alone. The repair is deliberately narrow: the second object must
/// begin with a quoted or identifier-like key.
fn remove_redundant_object_openers(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == '{' {
            let mut second = i + 1;
            while second < bytes.len() && (bytes[second] as char).is_whitespace() {
                second += 1;
            }
            if second < bytes.len() && bytes[second] == b'{' {
                let mut key = second + 1;
                while key < bytes.len() && (bytes[key] as char).is_whitespace() {
                    key += 1;
                }
                if key < bytes.len()
                    && (bytes[key] == b'"'
                        || bytes[key] == b'\''
                        || (bytes[key] as char).is_ascii_alphabetic()
                        || bytes[key] == b'_')
                {
                    // Drop the first opener and its intervening whitespace.
                    i = second;
                    continue;
                }
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn apply_repair(
    attempt: &mut RepairAttempt,
    changed_bytes: &mut usize,
    source_pristine: &mut bool,
    limits: &RepairLimits,
    kind: RepairKind,
    normalization: &str,
    repair: fn(&str) -> String,
) -> bool {
    let next = repair(&attempt.text);
    if next == attempt.text {
        return true;
    }
    let (start, end, replacement_end) = changed_span(&attempt.text, &next);
    let original_text = attempt.text[start..end].to_string();
    let replacement_text = next[start..replacement_end].to_string();
    let operation_bytes = original_text.len().max(replacement_text.len());
    if attempt.repairs.len() >= limits.max_operations
        || changed_bytes.saturating_add(operation_bytes) > limits.max_changed_bytes
    {
        return false;
    }
    attempt.normalizations.push(normalization.to_string());
    attempt.repairs.push(PendingRepair {
        operation: RepairOperation {
            kind,
            input_byte_start: start,
            input_byte_end: end,
            original_text,
            replacement_text,
            source_range: None,
        },
        source_relative: *source_pristine,
    });
    *changed_bytes += operation_bytes;
    *source_pristine = false;
    attempt.text = next;
    true
}

fn changed_span(before: &str, after: &str) -> (usize, usize, usize) {
    let mut prefix = 0usize;
    for ((before_offset, before_char), (after_offset, after_char)) in
        before.char_indices().zip(after.char_indices())
    {
        if before_char != after_char {
            break;
        }
        prefix = (before_offset + before_char.len_utf8()).min(after_offset + after_char.len_utf8());
    }

    let before_tail = &before[prefix..];
    let after_tail = &after[prefix..];
    let mut suffix = 0usize;
    for (before_char, after_char) in before_tail.chars().rev().zip(after_tail.chars().rev()) {
        if before_char != after_char {
            break;
        }
        suffix += before_char.len_utf8();
    }
    (
        prefix,
        before.len().saturating_sub(suffix),
        after.len().saturating_sub(suffix),
    )
}

fn quote_single_quoted_strings(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_single = false;
    for ch in input.chars() {
        if ch == '\'' {
            in_single = !in_single;
            out.push('"');
        } else {
            out.push(ch);
        }
    }
    out
}

fn quote_unquoted_object_keys(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();
    let mut in_string = false;
    let mut escape = false;
    while let Some((_, ch)) = chars.next() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            out.push(ch);
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }
        if ch == '{' || ch == ',' {
            out.push(ch);
            let mut whitespace = String::new();
            while let Some((_, next)) = chars.peek().copied() {
                if next.is_whitespace() {
                    chars.next();
                    whitespace.push(next);
                } else {
                    break;
                }
            }
            let mut key = String::new();
            while let Some((_, next)) = chars.peek().copied() {
                if next.is_ascii_alphanumeric() || next == '_' || next == '-' {
                    chars.next();
                    key.push(next);
                } else {
                    break;
                }
            }
            if !key.is_empty() {
                let mut post = String::new();
                while let Some((_, next)) = chars.peek().copied() {
                    if next.is_whitespace() {
                        chars.next();
                        post.push(next);
                    } else {
                        break;
                    }
                }
                if matches!(chars.peek(), Some((_, ':'))) {
                    chars.next();
                    out.push_str(&whitespace);
                    out.push('"');
                    out.push_str(&key);
                    out.push('"');
                    out.push_str(&post);
                    out.push(':');
                } else {
                    out.push_str(&whitespace);
                    out.push_str(&key);
                    out.push_str(&post);
                }
            } else {
                out.push_str(&whitespace);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn remove_trailing_commas(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == ',' {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some(c) if c.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }
        out.push(ch);
    }
    out
}

fn insert_missing_commas_between_members(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();
    let mut in_string = false;
    let mut escape = false;
    while let Some((_, ch)) = chars.next() {
        out.push(ch);
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
                let mut lookahead = chars.clone();
                while matches!(lookahead.peek(), Some((_, c)) if c.is_whitespace()) {
                    lookahead.next();
                }
                if matches!(lookahead.peek(), Some((_, '"')))
                    && lookahead_quoted_key_colon(lookahead)
                {
                    out.push(',');
                }
            }
        } else if ch == '"' {
            in_string = true;
        } else if ch == '}' || ch == ']' || ch.is_ascii_digit() || matches!(ch, 'e' | 'E') {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some((_, c)) if c.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some((_, '"'))) && lookahead_quoted_key_colon(lookahead) {
                out.push(',');
            }
        }
    }
    out
}

fn lookahead_quoted_key_colon<I>(mut chars: I) -> bool
where
    I: Iterator<Item = (usize, char)> + Clone,
{
    if !matches!(chars.next(), Some((_, '"'))) {
        return false;
    }
    let mut escape = false;
    for (_, ch) in chars.by_ref() {
        if escape {
            escape = false;
        } else if ch == '\\' {
            escape = true;
        } else if ch == '"' {
            break;
        }
    }
    while matches!(chars.clone().next(), Some((_, c)) if c.is_whitespace()) {
        chars.next();
    }
    matches!(chars.next(), Some((_, ':')))
}

fn escape_unescaped_string_boundary_quotes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();
    let mut in_string = false;
    let mut escape = false;
    while let Some((_, ch)) = chars.next() {
        if in_string {
            if escape {
                escape = false;
                out.push(ch);
            } else if ch == '\\' {
                escape = true;
                out.push(ch);
            } else if ch == '"' {
                let mut lookahead = chars.clone();
                while matches!(lookahead.peek(), Some((_, c)) if c.is_whitespace()) {
                    lookahead.next();
                }
                if matches!(lookahead.peek(), Some((_, ',' | '}' | ']' | ':')))
                    || lookahead_quoted_key_colon(lookahead.clone())
                    || lookahead.peek().is_none()
                {
                    in_string = false;
                    out.push(ch);
                } else {
                    out.push('\\');
                    out.push(ch);
                }
            } else {
                out.push(ch);
            }
        } else {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
        }
    }
    out
}

fn complete_unterminated_json_containers(input: &str) -> String {
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escape = false;
    for ch in input.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' if stack.last().copied() == Some(ch) => {
                stack.pop();
            }
            _ => {}
        }
    }
    if in_string || stack.is_empty() {
        return input.to_string();
    }
    let mut out = input.trim_end().to_string();
    while let Some(ch) = stack.pop() {
        out.push(ch);
    }
    out
}

fn first_balanced_json_value(text: &str) -> Option<(usize, usize)> {
    let start = text.find(['{', '['])?;
    let open = text.as_bytes()[start] as char;
    let close = if open == '{' { '}' } else { ']' };
    find_matching(text, start, open, close).map(|end| (start, end + 1))
}

fn find_matching(text: &str, open_index: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    for (idx, ch) in text[open_index..].char_indices() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            c if c == open => depth += 1,
            c if c == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open_index + idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_argument_value_start(args: &str, arg_name: &str) -> Option<usize> {
    let bytes = args.as_bytes();
    let mut search_from = 0;
    while let Some(relative) = args[search_from..].find(arg_name) {
        let name_start = search_from + relative;
        let name_end = name_start + arg_name.len();
        let before_ok = name_start == 0
            || !bytes[name_start - 1].is_ascii_alphanumeric() && bytes[name_start - 1] != b'_';
        let after_name = args[name_end..]
            .char_indices()
            .find_map(|(idx, ch)| (!ch.is_whitespace()).then_some((name_end + idx, ch)));
        if before_ok && matches!(after_name, Some((_, '='))) {
            let eq_idx = after_name.unwrap().0;
            return args[eq_idx + 1..]
                .char_indices()
                .find_map(|(idx, ch)| (!ch.is_whitespace()).then_some(eq_idx + 1 + idx));
        }
        search_from = name_end;
    }
    None
}

fn discover_command_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for (idx, ch) in text.char_indices() {
        if ch != '(' {
            continue;
        }
        let name_end = idx;
        let name_start = text[..name_end]
            .char_indices()
            .rev()
            .find_map(|(pos, candidate)| {
                (!is_command_name_char(candidate)).then_some(pos + candidate.len_utf8())
            })
            .unwrap_or(0);
        let name = text[name_start..name_end].trim();
        if is_plausible_command_name(name)
            && find_matching(text, idx, '(', ')')
                .map(|close| {
                    text[idx + 1..close].contains('{') || text[idx + 1..close].contains('[')
                })
                .unwrap_or(false)
        {
            names.push(name.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

fn is_command_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.')
}

fn is_plausible_command_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_') && chars.all(is_command_name_char)
}

fn strip_think_blocks(text: &str) -> String {
    let mut output = text.as_bytes().to_vec();
    let mut search = 0;
    while let Some(relative_start) = text[search..].find("<think>") {
        let start = search + relative_start;
        let after_start = start + "<think>".len();
        let end = text[after_start..]
            .find("</think>")
            .map(|relative_end| after_start + relative_end + "</think>".len())
            .unwrap_or(text.len());
        for byte in &mut output[start..end] {
            if !matches!(*byte, b'\r' | b'\n') {
                *byte = b' ';
            }
        }
        search = end;
    }
    String::from_utf8(output).expect("masking UTF-8 with ASCII spaces stays valid UTF-8")
}

fn looks_like_candidate_start(text: &str) -> bool {
    text.contains("```")
        || text.contains("function")
        || text.contains("jsonrpc")
        || text.contains("{")
}

fn infer_streaming_grammar(text: &str) -> CandidateGrammar {
    if text.contains("```") {
        CandidateGrammar::FencedCode
    } else if text.contains("jsonrpc") {
        CandidateGrammar::McpJsonRpc
    } else if text.contains("function") {
        CandidateGrammar::OpenAiToolCall
    } else {
        CandidateGrammar::RawJson
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_output_parser_is_json_primary() {
        let report = parse_model_output(
            "Submit.Result(arg={\"a\":1})",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates
                .iter()
                .all(|candidate| candidate.grammar != CandidateGrammar::PythonStyleCommand)
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .grammar,
            CandidateGrammar::JsonObjectInText
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .value
                .as_ref()
                .unwrap()["a"],
            1
        );
    }

    #[test]
    fn parses_python_style_command_when_enabled() {
        let options = ModelOutputOptions {
            accepted_commands: vec!["Agent.Run".into()],
            accepted_arguments: vec!["arg".into()],
            parse_python_style_commands: true,
            ..Default::default()
        };
        let report = parse_model_output(
            "Agent.Run(arg={\"command\":\"ls\"})",
            SourceInfo::stdin("model.txt"),
            &options,
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates
                .iter()
                .any(|candidate| candidate.grammar == CandidateGrammar::PythonStyleCommand)
        );
    }

    #[test]
    fn json_candidate_stays_primary_when_python_style_is_enabled() {
        let report = parse_model_output(
            "Submit.Result(arg={\"a\":1})",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions {
                parse_python_style_commands: true,
                ..Default::default()
            },
        );
        let selected_id = report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .selected_candidate_id
            .as_deref()
            .unwrap();
        let selected = report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates
            .iter()
            .find(|candidate| candidate.id == selected_id)
            .unwrap();
        assert_eq!(selected.grammar, CandidateGrammar::JsonObjectInText);
    }

    #[test]
    fn parses_openai_style_arguments() {
        let report = parse_model_output(
            "{\"function\":{\"name\":\"x\",\"arguments\":\"{\\\"a\\\":1}\"}}",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .grammar,
            CandidateGrammar::OpenAiToolCall
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .value
                .as_ref()
                .unwrap()["a"],
            1
        );
    }

    #[test]
    fn parses_openai_chat_completion_content() {
        let report = parse_model_output(
            "{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"{\\\"a\\\":1}\"}}]}",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .grammar,
            CandidateGrammar::OpenAiChatContent
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .value
                .as_ref()
                .unwrap()["a"],
            1
        );
    }

    #[test]
    fn parses_root_name_arguments_tool_call() {
        let report = parse_model_output(
            "{\"name\":\"Submit.Result\",\"arguments\":\"{\\\"a\\\":1}\"}",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .grammar,
            CandidateGrammar::OpenAiToolCall
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .command_name
                .as_deref(),
            Some("Submit.Result")
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .value
                .as_ref()
                .unwrap()["a"],
            1
        );
    }

    #[test]
    fn parses_plain_identifier_positional_command() {
        let report = parse_model_output(
            "submit({\"a\":1})",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions {
                parse_python_style_commands: true,
                ..Default::default()
            },
        );
        let command = report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates
            .iter()
            .find(|candidate| candidate.grammar == CandidateGrammar::PythonStyleCommand)
            .unwrap();
        assert_eq!(command.command_name.as_deref(), Some("submit"));
        assert_eq!(command.argument_name.as_deref(), Some("positional"));
    }

    #[test]
    fn records_unparsed_failure_with_raw_text() {
        let report = parse_model_output(
            "there is no structured output here",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .status,
            ModelOutputStatus::Unparsed
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .failures
                .len(),
            1
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .failures[0]
                .failure_mode,
            "unparsed_model_output"
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .failures[0]
                .raw_text,
            "there is no structured output here"
        );
    }

    #[test]
    fn records_incomplete_unclosed_fence_as_candidate() {
        let report = parse_model_output(
            "```json\n{\"a\":",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .status,
            ModelOutputStatus::Incomplete
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .grammar,
            CandidateGrammar::FencedJson
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .candidates[0]
                .status,
            CandidateStatus::Incomplete
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .failures[0]
                .failure_mode,
            "unclosed_markdown_fence"
        );
    }

    #[test]
    fn repairs_aliases_and_validates_schema() {
        let options = ModelOutputOptions {
            aliases: AliasRules {
                field_aliases: vec![AliasRule {
                    from: "nodes".into(),
                    to: "leaves".into(),
                }],
                command_aliases: vec![AliasRule {
                    from: "Old.Tool".into(),
                    to: "New.Tool".into(),
                }],
                argument_aliases: Vec::new(),
            },
            schema: Some(serde_json::json!({"type":"object", "required":["leaves"]})),
            parse_python_style_commands: true,
            ..Default::default()
        };
        let report = parse_model_output(
            "Old.Tool(arg={nodes: [1, 2,], ok: True})",
            SourceInfo::stdin("model.txt"),
            &options,
        );
        let candidate = report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates
            .iter()
            .find(|candidate| candidate.grammar == CandidateGrammar::PythonStyleCommand)
            .unwrap();
        assert_eq!(candidate.command_name.as_deref(), Some("New.Tool"));
        assert!(candidate.value.as_ref().unwrap().get("leaves").is_some());
        assert!(candidate.validation.as_ref().unwrap().valid);
    }

    #[test]
    fn repairs_missing_comma_and_records_candidate_raw_text() {
        let report = parse_model_output(
            "{\"narration_text\": \"Aim for sixty seconds.\" \"target_words\": 130}",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        let candidate = &report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates[0];
        assert_eq!(candidate.status, CandidateStatus::Recovered);
        assert_eq!(candidate.value.as_ref().unwrap()["target_words"], 130);
        assert!(
            candidate
                .normalizations
                .contains(&"inserted_missing_comma".to_string())
        );
        assert_eq!(
            candidate.raw_text.as_deref(),
            Some("{\"narration_text\": \"Aim for sixty seconds.\" \"target_words\": 130}")
        );
    }

    #[test]
    fn repairs_redundant_object_opener_in_array() {
        let report = parse_model_output(
            r#"[{"title":"The Speed Limit Contradiction"}, {{"title":"Fusion of Dimensions"}]"#,
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        let candidate = &report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates[0];
        assert_eq!(candidate.status, CandidateStatus::Recovered);
        assert_eq!(
            candidate.value.as_ref().unwrap()[1]["title"],
            "Fusion of Dimensions"
        );
        assert!(
            candidate
                .normalizations
                .contains(&"removed_redundant_object_opener".to_string())
        );
    }

    #[test]
    fn preserves_latex_commands_as_literal_backslashes() {
        let report = parse_model_output(
            r#"["$\Delta \lambda$", "$\frac{a(t_{obs})}{a(t_{emit})}$"]"#,
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        let candidate = &report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates[0];
        assert_eq!(candidate.status, CandidateStatus::Recovered);
        assert_eq!(candidate.value.as_ref().unwrap()[0], "$\\Delta \\lambda$");
        assert_eq!(
            candidate.value.as_ref().unwrap()[1],
            "$\\frac{a(t_{obs})}{a(t_{emit})}$"
        );
        assert!(
            candidate
                .normalizations
                .contains(&"preserved_backslash_command".to_string())
        );
    }

    #[test]
    fn leaves_real_json_escapes_and_unicode_escapes_unchanged() {
        let parsed = parse_jsonish_value_with_repairs(r#""line\nfeed \u0394""#).unwrap();
        assert_eq!(parsed.value, "line\nfeed Δ");
        assert!(parsed.normalizations.is_empty());
    }

    #[test]
    fn extracts_json_fence_nested_in_markdown_fence() {
        let report = parse_model_output(
            "````markdown\nresult:\n```json\n{\"ok\": true}\n```\n````",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        let candidate = report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates
            .iter()
            .find(|candidate| {
                candidate
                    .normalizations
                    .contains(&"extracted_nested_markdown_fence".to_string())
            })
            .unwrap();
        assert_eq!(candidate.value.as_ref().unwrap()["ok"], true);
    }

    #[test]
    fn repairs_unescaped_quote_inside_long_string() {
        let report = parse_model_output(
            "{\"narration_text\":\"This has an \"internal\" quote.\" \"target_words\":130}",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        let candidate = &report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates[0];
        assert_eq!(candidate.status, CandidateStatus::Recovered);
        assert_eq!(
            candidate.value.as_ref().unwrap()["narration_text"],
            "This has an \"internal\" quote."
        );
        assert!(
            candidate
                .normalizations
                .contains(&"escaped_unescaped_string_quote".to_string())
        );
    }

    #[test]
    fn completes_unterminated_root_object() {
        let report = parse_model_output(
            "{\"narrative_contract\": {\"target_words\": 130}",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        let candidate = &report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates[0];
        assert_eq!(candidate.status, CandidateStatus::Recovered);
        assert_eq!(
            candidate.value.as_ref().unwrap()["narrative_contract"]["target_words"],
            130
        );
        assert!(
            candidate
                .normalizations
                .contains(&"closed_unterminated_object".to_string())
        );
    }

    #[test]
    fn schema_validation_selects_matching_candidate() {
        let options = ModelOutputOptions {
            schema: Some(serde_json::json!({
                "type": "object",
                "required": ["section_chunks"]
            })),
            ..Default::default()
        };
        let report = parse_model_output(
            "first {\"narrative_contract\": {}} second {\"section_chunks\": []}",
            SourceInfo::stdin("model.txt"),
            &options,
        );
        let selected = report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .selected_candidate_id
            .as_deref()
            .unwrap();
        let candidate = report
            .payload
            .as_ref()
            .expect("complete operation payload")
            .candidates
            .iter()
            .find(|candidate| candidate.id == selected)
            .unwrap();
        assert!(
            candidate
                .value
                .as_ref()
                .unwrap()
                .get("section_chunks")
                .is_some()
        );
        assert!(candidate.validation.as_ref().unwrap().valid);
    }

    #[test]
    fn streaming_finishes_with_candidate() {
        let mut parser = StreamingModelOutputParser::new(
            SourceInfo::stdin("stream"),
            ModelOutputOptions::default(),
        );
        let events = parser.push_chunk("```json\n{\"a\":");
        assert!(!events.is_empty());
        let (_events, report) = parser.finish();
        assert!(matches!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .status,
            ModelOutputStatus::Parsed
                | ModelOutputStatus::Ambiguous
                | ModelOutputStatus::Incomplete
                | ModelOutputStatus::Unparsed
        ));
    }
}
