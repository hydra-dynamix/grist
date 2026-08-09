//! Incremental, bounded model-output interpretation.

use super::{
    CandidateGrammar, CandidateStatus, ModelOutputCandidate, ModelOutputEnvelope,
    ModelOutputOptions, ModelOutputStatus, ModelOutputStreamEventV2, StreamingStateV2,
    parse_model_output,
};
use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, CancellationToken, Diagnostic, Envelope, Hashes,
    LineIndex, OperationControl, OperationControlError, OperationKind, OperationStatus, ParserInfo,
    ResourceBudget, SchemaVersion, SourceInfo, SourceRange, StreamTerminal,
};
use std::collections::BTreeMap;

const PARSER: &str = "grist.model_output.streaming";

/// Default hard bound for bytes retained by an incremental model-output parser.
pub const DEFAULT_STREAM_BUFFER_BYTES: usize = 128 * 1024 * 1024;

/// Conservative allowance for the batch extractor's owned working string,
/// candidate strings, JSON values, repair projections, and allocator overhead.
/// The control is charged for this peak before the corresponding input is
/// retained, so a finite memory budget fails before finish-time scratch exists.
const FINISH_MEMORY_FACTOR: usize = 32;
const FINISH_MEMORY_BASE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
enum StreamTermination {
    Controlled(OperationControlError),
    Explicit {
        status: OperationStatus,
        state: StreamingStateV2,
        diagnostic: Diagnostic,
    },
}

#[derive(Debug, Clone)]
struct CandidateProgress {
    grammar: CandidateGrammar,
    bytes_seen: usize,
}

/// Incremental model-output parser with exact byte retention, shared operation
/// control, deferred repair, and an exactly-once terminal event.
pub struct StreamingModelOutputParserV2 {
    source: SourceInfo,
    options: ModelOutputOptions,
    buffer: Vec<u8>,
    max_buffer_bytes: usize,
    control: OperationControl,
    decoded_characters: usize,
    state: StreamingStateV2,
    emitted_items: u64,
    candidates: BTreeMap<String, CandidateProgress>,
    termination: Option<StreamTermination>,
    terminal_event_emitted: bool,
}

impl StreamingModelOutputParserV2 {
    pub fn new(source: SourceInfo, options: ModelOutputOptions) -> Self {
        let control = OperationControl::new(
            &BudgetSelection::Profile(BudgetProfile::UntrustedServiceV1),
            CancellationToken::new(),
        )
        .expect("built-in model-output budget is valid");
        Self::new_with_control(source, options, DEFAULT_STREAM_BUFFER_BYTES, control)
    }

    pub fn new_with_limit(
        source: SourceInfo,
        options: ModelOutputOptions,
        max_buffer_bytes: usize,
    ) -> Self {
        let control = control_for_limit(max_buffer_bytes, &options, CancellationToken::new());
        Self::new_with_control(source, options, max_buffer_bytes, control)
    }

    pub fn new_with_cancellation(
        source: SourceInfo,
        options: ModelOutputOptions,
        cancellation: CancellationToken,
    ) -> Self {
        let control = OperationControl::new(
            &BudgetSelection::Profile(BudgetProfile::UntrustedServiceV1),
            cancellation,
        )
        .expect("built-in model-output budget is valid");
        Self::new_with_control(source, options, DEFAULT_STREAM_BUFFER_BYTES, control)
    }

    /// Construct a stream over the caller's shared cancellation and budget tree.
    pub fn new_with_control(
        source: SourceInfo,
        options: ModelOutputOptions,
        max_buffer_bytes: usize,
        control: OperationControl,
    ) -> Self {
        Self {
            source,
            options,
            buffer: Vec::new(),
            max_buffer_bytes,
            control,
            decoded_characters: 0,
            state: StreamingStateV2::Empty,
            emitted_items: 0,
            candidates: BTreeMap::new(),
            termination: None,
            terminal_event_emitted: false,
        }
    }

    pub fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }

    pub fn buffered_capacity(&self) -> usize {
        self.buffer.capacity()
    }

    pub fn max_buffer_bytes(&self) -> usize {
        self.max_buffer_bytes
    }

    pub fn is_terminated(&self) -> bool {
        self.termination.is_some() || self.terminal_event_emitted
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.control.cancellation().clone()
    }

    pub fn budget_usage(&self) -> crate::core::BudgetUsage {
        self.control.usage()
    }

    pub fn resource_budget(&self) -> &ResourceBudget {
        self.control.budget().budget()
    }

    pub fn push_chunk(&mut self, chunk: &str) -> Vec<ModelOutputStreamEventV2> {
        self.push_bytes(chunk.as_bytes())
    }

    /// Accept provider bytes without requiring UTF-8 code points to align with
    /// chunk boundaries. Candidate identity is deliberately not speculated from
    /// a prefix: `candidate-N` is assigned by the global batch extractor, so the
    /// stable start/update/completion sequence is emitted at finish.
    pub fn push_bytes(&mut self, chunk: &[u8]) -> Vec<ModelOutputStreamEventV2> {
        if self.is_terminated() {
            return Vec::new();
        }
        if let Err(error) = self.control.checkpoint() {
            self.stop_controlled(error);
            return self.take_termination_events();
        }
        let Some(bytes_after_push) = self.buffer.len().checked_add(chunk.len()) else {
            self.stop_for_buffer_limit(usize::MAX);
            return self.take_termination_events();
        };
        if bytes_after_push > self.max_buffer_bytes {
            self.stop_for_buffer_limit(bytes_after_push);
            return self.take_termination_events();
        }

        // Preflight complete scalar values without appending or reallocating.
        // A decoded-character rejection leaves retained bytes and capacity
        // untouched, including when a scalar is split across chunks.
        let newly_decoded = newly_completed_utf8_characters(&self.buffer, chunk);
        if let Err(error) = self
            .control
            .budget()
            .consume_decoded_characters(u64_from_usize(newly_decoded))
        {
            self.stop_controlled(error.into());
            return self.take_termination_events();
        }
        if let Err(error) = self
            .control
            .budget()
            .consume_input_bytes(u64_from_usize(chunk.len()))
        {
            self.stop_controlled(error.into());
            return self.take_termination_events();
        }
        if let Err(error) = self
            .control
            .budget()
            .observe_memory_bytes(estimated_finish_peak(bytes_after_push, &self.options))
        {
            self.stop_controlled(error.into());
            return self.take_termination_events();
        }
        if self.buffer.try_reserve_exact(chunk.len()).is_err() {
            self.stop_explicit(
                OperationStatus::Failed,
                StreamingStateV2::Failed,
                Diagnostic::error(
                    PARSER,
                    "stream.allocation_failed",
                    "unable to retain model-output stream bytes",
                ),
            );
            return self.take_termination_events();
        }
        self.buffer.extend_from_slice(chunk);

        // Account only characters from the complete UTF-8 prefix. A scalar split
        // between chunks is charged exactly once when its final byte arrives.
        let valid_text = valid_utf8_prefix(&self.buffer);
        let decoded = valid_text.chars().count();
        let valid_len = valid_text.len();
        let valid_is_empty = valid_text.is_empty();
        debug_assert_eq!(
            decoded,
            self.decoded_characters.saturating_add(newly_decoded),
            "incremental UTF-8 accounting must match the retained valid prefix"
        );
        self.decoded_characters = self.decoded_characters.saturating_add(newly_decoded);

        let new_state = if !self.candidates.is_empty() {
            StreamingStateV2::CandidateDetected
        } else if valid_text.trim().is_empty() {
            StreamingStateV2::Empty
        } else {
            StreamingStateV2::Accumulating
        };
        let mut events = Vec::new();
        if self.state != new_state {
            self.state = new_state.clone();
            if !self.emit(
                &mut events,
                ModelOutputStreamEventV2::ParserStateChanged { state: new_state },
                false,
            ) {
                events.extend(self.take_termination_events());
            }
        }
        if self.termination.is_none()
            && !valid_is_empty
            && valid_len == self.buffer.len()
            && chunk_may_complete_candidate(chunk)
        {
            self.emit_incremental_candidates(&mut events);
            if self.termination.is_some() {
                events.extend(self.take_termination_events());
            }
        } else if self.termination.is_none() && !self.candidates.is_empty() {
            self.emit_candidate_updates(&mut events);
            if self.termination.is_some() {
                events.extend(self.take_termination_events());
            }
        }
        events
    }

    /// Close an active stream after an external I/O failure while retaining the
    /// exactly-once, terminal-last v2 protocol.
    pub fn fail(
        mut self,
        diagnostic: Diagnostic,
    ) -> (Vec<ModelOutputStreamEventV2>, ModelOutputEnvelope) {
        self.stop_explicit(
            OperationStatus::Failed,
            StreamingStateV2::Failed,
            diagnostic,
        );
        let events = self.take_termination_events();
        let envelope = self.terminal_envelope();
        (events, envelope)
    }

    pub fn finish(mut self) -> (Vec<ModelOutputStreamEventV2>, ModelOutputEnvelope) {
        if self.termination.is_none() {
            if let Err(error) = self.control.checkpoint() {
                self.stop_controlled(error);
            }
        }
        if self.termination.is_none() {
            if let Err(error) = std::str::from_utf8(&self.buffer) {
                let valid_prefix = &self.buffer[..error.valid_up_to()];
                let invalid_len = error
                    .error_len()
                    .unwrap_or_else(|| self.buffer.len().saturating_sub(error.valid_up_to()));
                let range_end = error.valid_up_to().saturating_add(invalid_len);
                let index = LineIndex::new(
                    std::str::from_utf8(valid_prefix)
                        .expect("UTF-8 validator reports a valid prefix"),
                );
                let mut range = SourceRange::new(error.valid_up_to(), range_end, &index);
                if range.end_column == range.start_column {
                    range.end_column = range.start_column.saturating_add(1);
                }
                self.stop_explicit(
                    OperationStatus::Failed,
                    StreamingStateV2::Failed,
                    Diagnostic::error(
                        PARSER,
                        "stream.invalid_utf8",
                        "model-output stream ended with invalid or incomplete UTF-8",
                    )
                    .with_range(range)
                    .partial(),
                );
            }
        }
        if self.termination.is_some() {
            let events = self.take_termination_events();
            let envelope = self.terminal_envelope();
            return (events, envelope);
        }

        // This is the same authoritative parse used by batch mode. Its
        // conservative peak was charged before accepting the retained bytes.
        let text = std::str::from_utf8(&self.buffer).expect("UTF-8 checked above");
        let report = parse_model_output(text, self.source.clone(), &self.options);
        let candidates = report
            .payload
            .as_ref()
            .expect("model-output parse payload")
            .candidates
            .clone();
        if let Err(error) = self
            .control
            .budget()
            .consume_records(u64_from_usize(candidates.len()))
        {
            self.stop_controlled(error.into());
        }
        if self.termination.is_none() {
            if let Err(error) = self
                .control
                .budget()
                .consume_nodes(u64_from_usize(candidates.len()))
            {
                self.stop_controlled(error.into());
            }
        }

        let mut events = Vec::new();
        if self.termination.is_none() {
            for candidate in &candidates {
                let candidate_id = stable_stream_candidate_id(candidate);
                let progress = self.candidates.remove(&candidate_id);
                if progress.is_none() {
                    if !self.emit(
                        &mut events,
                        ModelOutputStreamEventV2::CandidateStarted {
                            candidate_id: candidate_id.clone(),
                            grammar: candidate.grammar.clone(),
                        },
                        false,
                    ) {
                        break;
                    }
                }
                if self.state != StreamingStateV2::CandidateDetected {
                    self.state = StreamingStateV2::CandidateDetected;
                    if !self.emit(
                        &mut events,
                        ModelOutputStreamEventV2::ParserStateChanged {
                            state: StreamingStateV2::CandidateDetected,
                        },
                        false,
                    ) {
                        break;
                    }
                }
                if progress.as_ref().map(|item| item.bytes_seen) != Some(self.buffer.len()) {
                    if !self.emit(
                        &mut events,
                        ModelOutputStreamEventV2::CandidateUpdated {
                            candidate_id: candidate_id.clone(),
                            bytes_seen: self.buffer.len(),
                        },
                        false,
                    ) {
                        break;
                    }
                }
                if !self.emit(
                    &mut events,
                    ModelOutputStreamEventV2::CandidateCompleted {
                        candidate_id,
                        candidate: candidate.clone(),
                    },
                    true,
                ) {
                    break;
                }
            }
        }

        if self.termination.is_none() && !self.candidates.is_empty() {
            self.stop_explicit(
                OperationStatus::Partial,
                StreamingStateV2::Failed,
                Diagnostic::parser_defect(
                    PARSER,
                    "an incrementally emitted candidate was absent from the authoritative final parse",
                )
                .partial(),
            );
        }

        if self.termination.is_some() {
            events.extend(self.take_termination_events());
            let envelope = self.terminal_envelope();
            return (events, envelope);
        }

        let operation_status = report.status;
        let payload = report.payload.as_ref().expect("model-output parse payload");
        let state = streaming_state(&payload.status);
        self.state = state.clone();
        if !self.emit(
            &mut events,
            ModelOutputStreamEventV2::ParserStateChanged { state },
            false,
        ) {
            events.extend(self.take_termination_events());
            let envelope = self.terminal_envelope();
            return (events, envelope);
        }

        let terminal = StreamTerminal {
            status: operation_status,
            emitted_items: self.emitted_items,
            diagnostics: report.diagnostics.clone(),
            budget_usage: self.control.usage(),
        };
        events.push(ModelOutputStreamEventV2::Terminal { terminal });
        self.terminal_event_emitted = true;
        (events, report)
    }

    fn emit(
        &mut self,
        events: &mut Vec<ModelOutputStreamEventV2>,
        event: ModelOutputStreamEventV2,
        completed_item: bool,
    ) -> bool {
        if let Err(error) = self.control.checkpoint() {
            self.stop_controlled(error);
            return false;
        }
        let output_bytes = serde_json::to_vec(&event)
            .expect("model-output events contain only JSON values")
            .len();
        if let Err(error) = self
            .control
            .budget()
            .consume_output_bytes(u64_from_usize(output_bytes))
        {
            self.stop_controlled(error.into());
            return false;
        }
        events.push(event);
        if completed_item {
            self.emitted_items = self.emitted_items.saturating_add(1);
        }
        true
    }

    fn emit_incremental_candidates(&mut self, events: &mut Vec<ModelOutputStreamEventV2>) {
        let text = std::str::from_utf8(&self.buffer).expect("caller checked complete UTF-8");
        let snapshot = parse_model_output(text, self.source.clone(), &self.options);
        let stable = snapshot
            .payload
            .as_ref()
            .expect("model-output parse payload")
            .candidates
            .iter()
            .filter(|candidate| candidate_is_prefix_stable(candidate, text))
            .map(|candidate| {
                (
                    stable_stream_candidate_id(candidate),
                    candidate.grammar.clone(),
                )
            })
            .collect::<Vec<_>>();
        if let Err(error) = self
            .control
            .budget()
            .consume_nodes(u64_from_usize(stable.len()))
        {
            self.stop_controlled(error.into());
            return;
        }

        for (candidate_id, grammar) in stable {
            if self.candidates.contains_key(&candidate_id) {
                continue;
            }
            if !self.emit(
                events,
                ModelOutputStreamEventV2::CandidateStarted {
                    candidate_id: candidate_id.clone(),
                    grammar: grammar.clone(),
                },
                false,
            ) {
                return;
            }
            if self.state != StreamingStateV2::CandidateDetected {
                self.state = StreamingStateV2::CandidateDetected;
                if !self.emit(
                    events,
                    ModelOutputStreamEventV2::ParserStateChanged {
                        state: StreamingStateV2::CandidateDetected,
                    },
                    false,
                ) {
                    return;
                }
            }
            if !self.emit(
                events,
                ModelOutputStreamEventV2::CandidateUpdated {
                    candidate_id: candidate_id.clone(),
                    bytes_seen: self.buffer.len(),
                },
                false,
            ) {
                return;
            }
            self.candidates.insert(
                candidate_id,
                CandidateProgress {
                    grammar,
                    bytes_seen: self.buffer.len(),
                },
            );
        }
        self.emit_candidate_updates(events);
    }

    fn emit_candidate_updates(&mut self, events: &mut Vec<ModelOutputStreamEventV2>) {
        let updates = self
            .candidates
            .iter()
            .filter(|(_, progress)| progress.bytes_seen != self.buffer.len())
            .map(|(candidate_id, progress)| (candidate_id.clone(), progress.grammar.clone()))
            .collect::<Vec<_>>();
        for (candidate_id, grammar) in updates {
            if !self.emit(
                events,
                ModelOutputStreamEventV2::CandidateUpdated {
                    candidate_id: candidate_id.clone(),
                    bytes_seen: self.buffer.len(),
                },
                false,
            ) {
                return;
            }
            self.candidates.insert(
                candidate_id,
                CandidateProgress {
                    grammar,
                    bytes_seen: self.buffer.len(),
                },
            );
        }
    }

    fn stop_controlled(&mut self, error: OperationControlError) {
        self.termination = Some(StreamTermination::Controlled(error));
    }

    fn stop_explicit(
        &mut self,
        status: OperationStatus,
        state: StreamingStateV2,
        diagnostic: Diagnostic,
    ) {
        self.termination = Some(StreamTermination::Explicit {
            status,
            state,
            diagnostic,
        });
    }

    fn stop_for_buffer_limit(&mut self, bytes_after_push: usize) {
        self.stop_explicit(
            OperationStatus::Failed,
            StreamingStateV2::Failed,
            Diagnostic::error(
                PARSER,
                "stream.buffer_limit_exceeded",
                format!(
                    "model-output stream would retain {bytes_after_push} bytes, exceeding the {} byte limit",
                    self.max_buffer_bytes
                ),
            )
            .partial(),
        );
    }

    fn termination_parts(&self) -> (OperationStatus, StreamingStateV2, Diagnostic) {
        match self.termination.as_ref().expect("stream termination") {
            StreamTermination::Controlled(error) => (
                error.operation_status(self.emitted_items),
                if matches!(error, OperationControlError::Cancelled(_)) {
                    StreamingStateV2::Cancelled
                } else {
                    StreamingStateV2::Failed
                },
                error.diagnostic(PARSER),
            ),
            StreamTermination::Explicit {
                status,
                state,
                diagnostic,
            } => (*status, state.clone(), diagnostic.clone()),
        }
    }

    fn take_termination_events(&mut self) -> Vec<ModelOutputStreamEventV2> {
        if self.terminal_event_emitted {
            return Vec::new();
        }
        let (status, state, diagnostic) = self.termination_parts();
        self.state = state.clone();
        self.terminal_event_emitted = true;
        vec![
            ModelOutputStreamEventV2::Diagnostic {
                diagnostic: diagnostic.clone(),
            },
            ModelOutputStreamEventV2::ParserStateChanged { state },
            ModelOutputStreamEventV2::Terminal {
                terminal: StreamTerminal {
                    status,
                    emitted_items: self.emitted_items,
                    diagnostics: vec![diagnostic],
                    budget_usage: self.control.usage(),
                },
            },
        ]
    }

    fn terminal_envelope(&self) -> ModelOutputEnvelope {
        let (status, _, diagnostic) = self.termination_parts();
        let digest = crate::core::options_digest(&self.options)
            .expect("model-output options must serialize");
        Envelope::without_payload(
            OperationKind::Parse,
            ArtifactKind::ModelOutput,
            status,
            self.source.clone(),
            ParserInfo::new("grist.model_output"),
            digest,
            SchemaVersion::MODEL_OUTPUT_V1,
        )
        .expect("stream terminal status is valid")
        .with_hashes(Hashes::for_bytes(&self.buffer, None))
        .with_diagnostics(vec![diagnostic])
    }
}

fn control_for_limit(
    max_buffer_bytes: usize,
    options: &ModelOutputOptions,
    cancellation: CancellationToken,
) -> OperationControl {
    let mut budget = ResourceBudget::untrusted_service_v1();
    budget.max_input_bytes = lower_limit(budget.max_input_bytes, u64_from_usize(max_buffer_bytes));
    budget.max_decoded_characters = lower_limit(
        budget.max_decoded_characters,
        u64_from_usize(max_buffer_bytes),
    );
    budget.max_memory_bytes = lower_limit(
        budget.max_memory_bytes,
        estimated_finish_peak(max_buffer_bytes, options),
    );
    OperationControl::new(&BudgetSelection::custom(budget), cancellation)
        .expect("model-output stream limit produces a valid budget")
}

fn estimated_finish_peak(input_bytes: usize, options: &ModelOutputOptions) -> u64 {
    let options_bytes = serde_json::to_vec(options)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    let peak = input_bytes
        .saturating_mul(FINISH_MEMORY_FACTOR)
        .saturating_add(options_bytes.saturating_mul(2))
        .saturating_add(options.repair_limits.max_changed_bytes)
        .saturating_add(FINISH_MEMORY_BASE_BYTES);
    u64_from_usize(peak)
}

fn lower_limit(existing: Option<u64>, requested: u64) -> Option<u64> {
    Some(existing.map_or(requested, |limit| limit.min(requested)))
}

fn streaming_state(status: &ModelOutputStatus) -> StreamingStateV2 {
    match status {
        ModelOutputStatus::Empty => StreamingStateV2::Empty,
        ModelOutputStatus::Incomplete => StreamingStateV2::Incomplete,
        ModelOutputStatus::Malformed => StreamingStateV2::Malformed,
        ModelOutputStatus::Parsed => StreamingStateV2::Complete,
        ModelOutputStatus::Ambiguous => StreamingStateV2::Ambiguous,
        ModelOutputStatus::Unparsed => StreamingStateV2::Unparsed,
    }
}

fn chunk_may_complete_candidate(chunk: &[u8]) -> bool {
    chunk
        .iter()
        .any(|byte| matches!(byte, b'}' | b']' | b'>' | b')' | b'`' | b'\n'))
}

fn candidate_is_prefix_stable(candidate: &ModelOutputCandidate, text: &str) -> bool {
    if candidate.status != CandidateStatus::Complete || !candidate.repairs.is_empty() {
        return false;
    }
    match candidate.grammar {
        CandidateGrammar::FencedJson
        | CandidateGrammar::FencedCode
        | CandidateGrammar::YamlBlock
        | CandidateGrammar::TomlBlock
        | CandidateGrammar::XmlToolCall => true,
        CandidateGrammar::PythonStyleCommand
        | CandidateGrammar::OpenAiToolCall
        | CandidateGrammar::OpenAiChatContent
        | CandidateGrammar::McpJsonRpc
        | CandidateGrammar::JsonObjectInText => candidate
            .raw_range
            .as_ref()
            .is_some_and(|range| !text[range.byte_end..].trim().is_empty()),
        // A top-level JSON value can become JSON-in-text if later bytes append
        // prose, so its grammar is authoritative only at end-of-stream.
        CandidateGrammar::RawJson => false,
    }
}

fn stable_stream_candidate_id(candidate: &ModelOutputCandidate) -> String {
    let grammar = serde_json::to_string(&candidate.grammar)
        .expect("candidate grammar serializes deterministically");
    let range = candidate
        .raw_range
        .as_ref()
        .map(|range| format!("{}:{}", range.byte_start, range.byte_end))
        .unwrap_or_else(|| {
            let raw = candidate.raw_text.as_deref().unwrap_or_default();
            format!("raw:{}", crate::core::sha256_hex(raw.as_bytes()))
        });
    let key = format!("{range}:{grammar}");
    format!(
        "stream-candidate-{}",
        crate::core::sha256_hex(key.as_bytes())
    )
}

fn u64_from_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn newly_completed_utf8_characters(buffer: &[u8], chunk: &[u8]) -> usize {
    let error = match std::str::from_utf8(buffer) {
        Ok(_) => return valid_utf8_prefix(chunk).chars().count(),
        Err(error) if error.error_len().is_some() => return 0,
        Err(error) => error,
    };
    let pending = &buffer[error.valid_up_to()..];
    let Some(width) = pending.first().and_then(|byte| utf8_sequence_width(*byte)) else {
        return 0;
    };
    let needed = width.saturating_sub(pending.len());
    if needed == 0 || chunk.len() < needed {
        return 0;
    }
    let mut scalar = [0_u8; 4];
    scalar[..pending.len()].copy_from_slice(pending);
    scalar[pending.len()..width].copy_from_slice(&chunk[..needed]);
    if std::str::from_utf8(&scalar[..width]).is_err() {
        return 0;
    }
    1 + valid_utf8_prefix(&chunk[needed..]).chars().count()
}

fn utf8_sequence_width(first: u8) -> Option<usize> {
    match first {
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

fn valid_utf8_prefix(bytes: &[u8]) -> &str {
    match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => std::str::from_utf8(&bytes[..error.valid_up_to()])
            .expect("UTF-8 validator reports a valid prefix"),
    }
}
