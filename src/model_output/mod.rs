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
    pub value: Option<Value>,
    pub raw_text: Option<String>,
    pub raw_range: Option<SourceRange>,
    pub status: CandidateStatus,
    pub confidence: f32,
    pub normalizations: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub validation: Option<SchemaValidationResult>,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelOutputOptions {
    pub accepted_commands: Vec<String>,
    pub accepted_arguments: Vec<String>,
    pub aliases: AliasRules,
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
        extract_fenced_blocks(&working, &index, &mut candidates, &mut failures);
        extract_nested_json_fences(&working, &index, &mut candidates);
        extract_json_candidates(&working, &index, &mut candidates);
        extract_xml_tool_calls(&working, &index, &mut candidates);
        if options.parse_python_style_commands {
            extract_python_style_commands(&working, &index, options, &mut candidates);
        }
    }

    candidates = expand_compound_json_candidates(candidates, text, &index);
    for (id, candidate) in candidates.iter_mut().enumerate() {
        candidate.id = format!("candidate-{id}");
    }

    for candidate in candidates.iter_mut() {
        candidate
            .normalizations
            .extend(global_normalizations.clone());
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
                if let Ok(parsed) = parse_jsonish_value_with_repairs(body) {
                    let mut candidate = base_candidate(
                        candidates.len(),
                        CandidateGrammar::FencedJson,
                        Some(SourceRange::new(start, range_end, index)),
                    );
                    candidate.value = Some(parsed.value);
                    candidate.normalizations = vec!["extracted_nested_markdown_fence".into()];
                    candidate.normalizations.extend(parsed.normalizations);
                    if candidate.normalizations.len() > 1 {
                        candidate.status = CandidateStatus::Recovered;
                    }
                    classify_json_tool_shape(&mut candidate);
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
            let body = text[body_start..].trim();
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
                if let Ok(parsed) = parse_jsonish_value_with_repairs(body) {
                    candidate.value = Some(parsed.value);
                    candidate.normalizations.extend(parsed.normalizations);
                    classify_json_tool_shape(&mut candidate);
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
        let body = text[body_start..end].trim();
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
                match parse_jsonish_value_with_repairs(body) {
                    Ok(parsed) => {
                        candidate.value = Some(parsed.value);
                        candidate.normalizations.extend(parsed.normalizations);
                        if candidate.normalizations.len() > 1 {
                            candidate.status = CandidateStatus::Recovered;
                        }
                        classify_json_tool_shape(&mut candidate);
                    }
                    Err(err) => {
                        candidate.status = CandidateStatus::Malformed;
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
                match parse_jsonish_value_with_repairs(body) {
                    Ok(parsed) => {
                        candidate.value = Some(parsed.value);
                        candidate.normalizations.extend(parsed.normalizations);
                        if candidate.normalizations.len() > 1 {
                            candidate.status = CandidateStatus::Recovered;
                        }
                        classify_json_tool_shape(&mut candidate);
                    }
                    Err(err) => {
                        candidate.status = CandidateStatus::Malformed;
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
                    if let Ok((value, arg_name, normalizations)) = parse_command_arg(call, arg) {
                        let mut candidate = base_candidate(
                            candidates.len(),
                            CandidateGrammar::PythonStyleCommand,
                            Some(SourceRange::new(start, close + 1, index)),
                        );
                        candidate.command_name = Some(command.clone());
                        candidate.argument_name = Some(arg_name);
                        candidate.value = Some(value);
                        candidate.normalizations = normalizations;
                        candidates.push(candidate);
                        break;
                    }
                }
                if !candidates.iter().any(|candidate| {
                    candidate.raw_range.as_ref().is_some_and(|range| {
                        range.byte_start == start && range.byte_end == close + 1
                    })
                }) {
                    if let Ok((value, arg_name, normalizations)) =
                        parse_positional_command_arg(call)
                    {
                        let mut candidate = base_candidate(
                            candidates.len(),
                            CandidateGrammar::PythonStyleCommand,
                            Some(SourceRange::new(start, close + 1, index)),
                        );
                        candidate.command_name = Some(command.clone());
                        candidate.argument_name = Some(arg_name);
                        candidate.value = Some(value);
                        candidate.normalizations = normalizations;
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
    candidates: &mut Vec<ModelOutputCandidate>,
) {
    let trimmed = text.trim();
    if let Ok(parsed) = parse_jsonish_value_with_repairs(trimmed) {
        let start = text.find(trimmed).unwrap_or(0);
        let mut candidate = json_candidate(
            candidates.len(),
            CandidateGrammar::RawJson,
            parsed.value,
            SourceRange::new(start, start + trimmed.len(), index),
        );
        candidate.normalizations.extend(parsed.normalizations);
        if !candidate.normalizations.is_empty() {
            candidate.status = CandidateStatus::Recovered;
        }
        classify_json_tool_shape(&mut candidate);
        candidates.push(candidate);
        return;
    }
    let initial_candidate_count = candidates.len();
    let mut covered_until = 0usize;
    for (absolute_start, absolute_end) in balanced_json_spans(text) {
        if absolute_start < covered_until {
            continue;
        }
        let slice = &text[absolute_start..absolute_end];
        if let Ok(parsed) = parse_jsonish_value_with_repairs(slice) {
            if is_duplicate_of_existing_candidate(
                absolute_start,
                absolute_end,
                &parsed.value,
                candidates,
            ) {
                covered_until = absolute_end;
                continue;
            }
            let mut candidate = json_candidate(
                candidates.len(),
                CandidateGrammar::JsonObjectInText,
                parsed.value,
                SourceRange::new(absolute_start, absolute_end, index),
            );
            candidate
                .normalizations
                .push("extracted_balanced_json".into());
            candidate.normalizations.extend(parsed.normalizations);
            if candidate.normalizations.len() > 1 {
                candidate.status = CandidateStatus::Recovered;
            }
            classify_json_tool_shape(&mut candidate);
            candidates.push(candidate);
            // Match the previous outermost-candidate behavior: once a complete
            // container parses, do not also emit each nested container.
            covered_until = absolute_end;
        }
    }
    if candidates.len() == initial_candidate_count
        && (trimmed.starts_with('{') || trimmed.starts_with('['))
    {
        let start = text.find(trimmed).unwrap_or(0);
        let mut candidate = base_candidate(
            candidates.len(),
            CandidateGrammar::RawJson,
            Some(SourceRange::new(start, start + trimmed.len(), index)),
        );
        candidate.status = CandidateStatus::Incomplete;
        candidate.confidence = 0.4;
        candidate
            .normalizations
            .push("preserved_incomplete_raw_json".into());
        candidate.diagnostics.push(
            Diagnostic::warning(
                "grist.model_output.json",
                "json.incomplete",
                "raw JSON-like output started but did not parse as a complete value",
            )
            .partial(),
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

fn classify_json_tool_shape(candidate: &mut ModelOutputCandidate) {
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
        apply_openai_call(candidate, function, "parsed_stringified_arguments");
    } else if let Some(tool_calls) = openai_tool_calls(&value) {
        candidate.grammar = CandidateGrammar::OpenAiToolCall;
        if tool_calls.len() == 1 {
            if let Some(first_call) = tool_calls.first() {
                let function = first_call.get("function").unwrap_or(first_call);
                apply_openai_call(candidate, function, "parsed_first_tool_call_arguments");
            }
        }
    } else if let Some(content) = openai_chat_content(&value).map(str::to_string) {
        candidate.grammar = CandidateGrammar::OpenAiChatContent;
        candidate.argument_name = Some("content".into());
        if let Ok(parsed) = parse_nested_jsonish_value(&content) {
            candidate.value = Some(parsed);
            candidate
                .normalizations
                .push("parsed_openai_chat_message_content".into());
        } else if let Some((start, end)) = first_balanced_json_value(&content) {
            if let Ok(parsed) = parse_jsonish_value(&content[start..end]) {
                candidate.value = Some(parsed);
                candidate
                    .normalizations
                    .push("extracted_json_from_openai_chat_message_content".into());
            }
        }
    } else if value.get("name").is_some() && value.get("arguments").is_some() {
        apply_openai_call(candidate, &value, "parsed_root_stringified_arguments");
    } else {
        unwrap_stringified_json(candidate);
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
) {
    candidate.grammar = CandidateGrammar::OpenAiToolCall;
    candidate.command_name = call.get("name").and_then(Value::as_str).map(str::to_string);
    if let Some(arguments) = call.get("arguments") {
        candidate.argument_name = Some("arguments".into());
        candidate.value = Some(arguments.clone());
        if let Some(arguments_str) = arguments.as_str() {
            if let Ok(parsed) = parse_nested_jsonish_value(arguments_str) {
                candidate.value = Some(parsed);
                candidate
                    .normalizations
                    .push(stringified_normalization.into());
            }
        }
    }
}

fn expand_compound_json_candidates(
    candidates: Vec<ModelOutputCandidate>,
    source_text: &str,
    index: &LineIndex,
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
                    apply_openai_call(&mut item, function, "parsed_stringified_arguments");
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

fn parse_nested_jsonish_value(text: &str) -> Result<Value, serde_json::Error> {
    let mut value = parse_jsonish_value(text)?;
    for _ in 0..8 {
        let Value::String(nested) = &value else {
            break;
        };
        let trimmed = nested.trim();
        if !(trimmed.starts_with('{') || trimmed.starts_with('[') || trimmed.starts_with('"')) {
            break;
        }
        value = parse_jsonish_value(trimmed)?;
    }
    Ok(value)
}

fn unwrap_stringified_json(candidate: &mut ModelOutputCandidate) {
    let Some(Value::String(text)) = candidate.value.as_ref() else {
        return;
    };
    let Ok(value) = parse_nested_jsonish_value(text) else {
        return;
    };
    if matches!(value, Value::String(_)) {
        return;
    }
    candidate.value = Some(value);
    candidate
        .normalizations
        .push("parsed_stringified_json".into());
    candidate.status = CandidateStatus::Recovered;
    classify_json_tool_shape(candidate);
}
fn parse_command_arg(call: &str, arg_name: &str) -> Result<(Value, String, Vec<String>), String> {
    let Some(start) = find_argument_value_start(call, arg_name) else {
        return Err("missing arg".into());
    };
    let after = &call[start..];
    let Some((object_start, object_end)) = first_balanced_json_value(after) else {
        return Err("missing balanced JSON argument".into());
    };
    let object = &after[object_start..object_end];
    let parsed = parse_jsonish_value_with_repairs(object).map_err(|err| err.to_string())?;
    let mut normalizations = vec!["parsed_python_style_command".into()];
    normalizations.extend(parsed.normalizations);
    Ok((parsed.value, arg_name.to_string(), normalizations))
}

fn parse_positional_command_arg(call: &str) -> Result<(Value, String, Vec<String>), String> {
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
    let parsed = parse_jsonish_value_with_repairs(&args[object_start..object_end])
        .map_err(|err| err.to_string())?;
    let mut normalizations = vec!["parsed_python_style_positional_command".into()];
    normalizations.extend(parsed.normalizations);
    Ok((parsed.value, "positional".to_string(), normalizations))
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
            candidate.argument_name = Some(alias.to.clone());
            candidate
                .normalizations
                .push(format!("argument_alias:{}->{}", alias.from, alias.to));
        }
    }
    if let Some(Value::Object(map)) = candidate.value.as_mut() {
        for alias in &aliases.field_aliases {
            if let Some(value) = map.remove(&alias.from) {
                map.entry(alias.to.clone()).or_insert(value);
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
        value: None,
        raw_text: None,
        raw_range: range,
        status: CandidateStatus::Complete,
        confidence: 0.8,
        normalizations: Vec::new(),
        diagnostics: Vec::new(),
        validation: None,
    }
}

fn json_candidate(
    id: usize,
    grammar: CandidateGrammar,
    value: Value,
    range: SourceRange,
) -> ModelOutputCandidate {
    let mut candidate = base_candidate(id, grammar, Some(range));
    candidate.value = Some(value);
    candidate.confidence = 0.9;
    candidate
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
    normalizations: Vec<String>,
}

fn parse_jsonish_value(text: &str) -> Result<Value, serde_json::Error> {
    parse_jsonish_value_with_repairs(text).map(|parsed| parsed.value)
}

fn parse_jsonish_value_with_repairs(text: &str) -> Result<JsonishParse, serde_json::Error> {
    match serde_json::from_str(text) {
        Ok(value) => Ok(JsonishParse {
            value,
            normalizations: Vec::new(),
        }),
        Err(original_err) => {
            let (fixed, normalizations) = fix_jsonish_with_normalizations(text);
            serde_json::from_str(&fixed)
                .map(|value| JsonishParse {
                    value,
                    normalizations,
                })
                .map_err(|_| original_err)
        }
    }
}

fn fix_jsonish_with_normalizations(input: &str) -> (String, Vec<String>) {
    let mut normalizations = Vec::new();
    let mut out = input.trim().to_string();
    for (from, to, name) in [
        ("None", "null", "replaced_python_none"),
        ("True", "true", "replaced_python_true"),
        ("False", "false", "replaced_python_false"),
    ] {
        let next = out.replace(from, to);
        if next != out {
            normalizations.push(name.to_string());
            out = next;
        }
    }
    apply_repair(
        &mut out,
        &mut normalizations,
        remove_redundant_object_openers,
        "removed_redundant_object_opener",
    );
    apply_repair(
        &mut out,
        &mut normalizations,
        quote_single_quoted_strings,
        "quoted_single_quoted_strings",
    );
    apply_repair(
        &mut out,
        &mut normalizations,
        quote_unquoted_object_keys,
        "quoted_unquoted_object_keys",
    );
    apply_repair(
        &mut out,
        &mut normalizations,
        preserve_backslash_commands_in_strings,
        "preserved_backslash_command",
    );
    apply_repair(
        &mut out,
        &mut normalizations,
        escape_unescaped_string_boundary_quotes,
        "escaped_unescaped_string_quote",
    );
    apply_repair(
        &mut out,
        &mut normalizations,
        insert_missing_commas_between_members,
        "inserted_missing_comma",
    );
    apply_repair(
        &mut out,
        &mut normalizations,
        remove_trailing_commas,
        "removed_trailing_commas",
    );
    apply_repair(
        &mut out,
        &mut normalizations,
        complete_unterminated_json_containers,
        "closed_unterminated_object",
    );
    (out, normalizations)
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
    out: &mut String,
    normalizations: &mut Vec<String>,
    repair: fn(&str) -> String,
    name: &str,
) {
    let next = repair(out);
    if next != *out {
        normalizations.push(name.to_string());
        *out = next;
    }
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
