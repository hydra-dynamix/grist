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
    pub raw_range: Option<SourceRange>,
    pub status: CandidateStatus,
    pub confidence: f32,
    pub normalizations: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub validation: Option<SchemaValidationResult>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateGrammar {
    PythonStyleCommand,
    OpenAiToolCall,
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

#[derive(Debug, Clone, Default)]
pub struct ModelOutputOptions {
    pub accepted_commands: Vec<String>,
    pub accepted_arguments: Vec<String>,
    pub aliases: AliasRules,
    pub strip_think_blocks: bool,
    pub schema: Option<Value>,
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

    let index = LineIndex::new(&working);
    let mut candidates = Vec::new();
    let mut diagnostics = Vec::new();

    if working.trim().is_empty() {
        diagnostics.push(Diagnostic::error(
            "grist.model_output",
            "model_output.empty",
            "empty model output",
        ));
    } else {
        extract_fenced_blocks(&working, &index, &mut candidates);
        extract_xml_tool_calls(&working, &index, &mut candidates);
        extract_python_style_commands(&working, &index, options, &mut candidates);
        extract_json_candidates(&working, &index, &mut candidates);
    }

    for candidate in candidates.iter_mut() {
        candidate
            .normalizations
            .extend(global_normalizations.clone());
        apply_aliases(candidate, &options.aliases);
        if let (Some(value), Some(schema)) = (candidate.value.as_ref(), options.schema.as_ref()) {
            let validation_diagnostics = validate_json_schema(value, schema);
            candidate.validation = Some(SchemaValidationResult {
                valid: validation_diagnostics.is_empty(),
                diagnostics: validation_diagnostics.clone(),
            });
            candidate.diagnostics.extend(validation_diagnostics);
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
        ModelOutputStatus::Unparsed
    } else if candidates.len() == 1 {
        ModelOutputStatus::Parsed
    } else {
        ModelOutputStatus::Ambiguous
    };

    let selected_candidate_id = (candidates.len() == 1).then(|| candidates[0].id.clone());
    let raw_text_sha256 = crate::core::sha256_hex(working.as_bytes());
    Envelope::new(
        ArtifactKind::ModelOutput,
        source,
        ParserInfo::new("grist.model_output"),
        SchemaVersion::MODEL_OUTPUT_V1,
        ModelOutputReport {
            schema_version: SchemaVersion::MODEL_OUTPUT_V1.to_string(),
            candidates,
            selected_candidate_id,
            raw_text_sha256,
            status,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(diagnostics)
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
        let mut events = Vec::new();
        if report.payload.candidates.is_empty() && !self.buffer.trim().is_empty() {
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
            for candidate in &report.payload.candidates {
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
        let Some(end_rel) = text[body_start..].find("```") else {
            break;
        };
        let end = body_start + end_rel;
        let body = text[body_start..end].trim();
        let language = info
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
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
                match parse_jsonish_value(body) {
                    Ok(value) => candidate.value = Some(value),
                    Err(err) => {
                        candidate.status = CandidateStatus::Malformed;
                        candidate.diagnostics.push(Diagnostic::error(
                            "grist.model_output.fence",
                            "fence.json_parse",
                            format!("fenced JSON parse failed: {err}"),
                        ));
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
                        candidate.diagnostics.push(Diagnostic::error(
                            "grist.model_output.fence",
                            "fence.yaml_to_json",
                            format!("fenced YAML conversion failed: {err}"),
                        ));
                    }
                },
                Err(err) => {
                    candidate.status = CandidateStatus::Malformed;
                    candidate.diagnostics.push(Diagnostic::error(
                        "grist.model_output.fence",
                        "fence.yaml_parse",
                        format!("fenced YAML parse failed: {err}"),
                    ));
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
                        candidate.diagnostics.push(Diagnostic::error(
                            "grist.model_output.fence",
                            "fence.toml_to_json",
                            format!("fenced TOML conversion failed: {err}"),
                        ));
                    }
                },
                Err(err) => {
                    candidate.status = CandidateStatus::Malformed;
                    candidate.diagnostics.push(Diagnostic::error(
                        "grist.model_output.fence",
                        "fence.toml_parse",
                        format!("fenced TOML parse failed: {err}"),
                    ));
                }
            },
            _ if body.starts_with('{') || body.starts_with('[') => {
                match parse_jsonish_value(body) {
                    Ok(value) => candidate.value = Some(value),
                    Err(err) => {
                        candidate.status = CandidateStatus::Malformed;
                        candidate.diagnostics.push(Diagnostic::error(
                            "grist.model_output.fence",
                            "fence.jsonish_parse",
                            format!("fenced JSON-like parse failed: {err}"),
                        ));
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
    if let Ok(value) = parse_jsonish_value(trimmed) {
        let start = text.find(trimmed).unwrap_or(0);
        let mut candidate = json_candidate(
            candidates.len(),
            CandidateGrammar::RawJson,
            value,
            SourceRange::new(start, start + trimmed.len(), index),
        );
        classify_json_tool_shape(&mut candidate);
        candidates.push(candidate);
        return;
    }
    let mut search = 0;
    while let Some((start, end)) = first_balanced_json_value(&text[search..]) {
        let absolute_start = search + start;
        let absolute_end = search + end;
        let slice = &text[absolute_start..absolute_end];
        if let Ok(value) = parse_jsonish_value(slice) {
            let mut candidate = json_candidate(
                candidates.len(),
                CandidateGrammar::JsonObjectInText,
                value,
                SourceRange::new(absolute_start, absolute_end, index),
            );
            candidate
                .normalizations
                .push("extracted_balanced_json".into());
            classify_json_tool_shape(&mut candidate);
            candidates.push(candidate);
        }
        search = absolute_end;
    }
}

fn classify_json_tool_shape(candidate: &mut ModelOutputCandidate) {
    let Some(value) = candidate.value.as_ref() else {
        return;
    };
    if value.get("jsonrpc").is_some() && value.get("method").is_some() {
        candidate.grammar = CandidateGrammar::McpJsonRpc;
        candidate.command_name = value
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
    } else if let Some(function) = value.get("function") {
        candidate.grammar = CandidateGrammar::OpenAiToolCall;
        candidate.command_name = function
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(arguments) = function.get("arguments") {
            candidate.argument_name = Some("arguments".into());
            if let Some(arguments_str) = arguments.as_str() {
                if let Ok(parsed) = parse_jsonish_value(arguments_str) {
                    candidate.value = Some(parsed);
                    candidate
                        .normalizations
                        .push("parsed_stringified_arguments".into());
                }
            }
        }
    } else if let Some(tool_calls) = value.get("tool_calls").and_then(Value::as_array) {
        candidate.grammar = CandidateGrammar::OpenAiToolCall;
        if let Some(first_call) = tool_calls.first() {
            let function = first_call.get("function").unwrap_or(first_call);
            candidate.command_name = function
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(arguments) = function.get("arguments") {
                candidate.argument_name = Some("arguments".into());
                if let Some(arguments_str) = arguments.as_str() {
                    if let Ok(parsed) = parse_jsonish_value(arguments_str) {
                        candidate.value = Some(parsed);
                        candidate
                            .normalizations
                            .push("parsed_first_tool_call_arguments".into());
                    }
                }
            }
        }
    }
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
    let value = parse_jsonish_value(object).map_err(|err| err.to_string())?;
    Ok((
        value,
        arg_name.to_string(),
        vec!["parsed_python_style_command".into()],
    ))
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

fn parse_jsonish_value(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(text).or_else(|_| serde_json::from_str(&fix_jsonish(text)))
}

fn fix_jsonish(input: &str) -> String {
    let mut out = input
        .trim()
        .replace("None", "null")
        .replace("True", "true")
        .replace("False", "false");
    out = quote_single_quoted_strings(&out);
    out = quote_unquoted_object_keys(&out);
    out = remove_trailing_commas(&out);
    out
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
    for token in text.split(|ch: char| ch.is_whitespace() || ch == '\n') {
        if let Some(open) = token.find('(') {
            let name = &token[..open];
            if name.chars().any(|ch| ch == '.')
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.'))
            {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn strip_think_blocks(text: &str) -> String {
    let mut output = String::new();
    let mut rest = text;
    loop {
        let Some(start) = rest.find("<think>") else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..start]);
        let after = &rest[start + "<think>".len()..];
        if let Some(end) = after.find("</think>") {
            rest = &after[end + "</think>".len()..];
        } else {
            break;
        }
    }
    output
}

fn looks_like_candidate_start(text: &str) -> bool {
    text.contains("```")
        || text.contains("function")
        || text.contains("jsonrpc")
        || text.contains("{")
        || text.contains("(")
}

fn infer_streaming_grammar(text: &str) -> CandidateGrammar {
    if text.contains("```") {
        CandidateGrammar::FencedCode
    } else if text.contains("jsonrpc") {
        CandidateGrammar::McpJsonRpc
    } else if text.contains("function") {
        CandidateGrammar::OpenAiToolCall
    } else if text.contains('(') {
        CandidateGrammar::PythonStyleCommand
    } else {
        CandidateGrammar::RawJson
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_python_style_command() {
        let options = ModelOutputOptions {
            accepted_commands: vec!["Agent.Run".into()],
            accepted_arguments: vec!["arg".into()],
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
                .candidates
                .iter()
                .any(|candidate| candidate.grammar == CandidateGrammar::PythonStyleCommand)
        );
    }

    #[test]
    fn parses_openai_style_arguments() {
        let report = parse_model_output(
            "{\"function\":{\"name\":\"x\",\"arguments\":\"{\\\"a\\\":1}\"}}",
            SourceInfo::stdin("model.txt"),
            &ModelOutputOptions::default(),
        );
        assert_eq!(
            report.payload.candidates[0].grammar,
            CandidateGrammar::OpenAiToolCall
        );
        assert_eq!(report.payload.candidates[0].value.as_ref().unwrap()["a"], 1);
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
            ..Default::default()
        };
        let report = parse_model_output(
            "Old.Tool(arg={nodes: [1, 2,], ok: True})",
            SourceInfo::stdin("model.txt"),
            &options,
        );
        let candidate = report
            .payload
            .candidates
            .iter()
            .find(|candidate| candidate.grammar == CandidateGrammar::PythonStyleCommand)
            .unwrap();
        assert_eq!(candidate.command_name.as_deref(), Some("New.Tool"));
        assert!(candidate.value.as_ref().unwrap().get("leaves").is_some());
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
            report.payload.status,
            ModelOutputStatus::Parsed | ModelOutputStatus::Ambiguous | ModelOutputStatus::Unparsed
        ));
    }
}
