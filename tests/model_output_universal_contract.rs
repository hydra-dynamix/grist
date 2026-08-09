#![cfg(feature = "model-output")]

use grist::core::{
    BudgetSelection, CancellationToken, OperationControl, OperationStatus, ResourceBudget,
    SourceInfo,
};
use grist::model_output::{
    AliasRule, AliasRules, CandidateGrammar, CandidateStatus, ModelOutputEvent, ModelOutputOptions,
    ModelOutputStatus, ModelOutputStreamEventV2, RepairKind, RepairLimits,
    StreamingModelOutputParser, StreamingModelOutputParserV2, StreamingStateV2, parse_model_output,
};
use serde_json::json;

fn parse(text: &str, options: &ModelOutputOptions) -> grist::model_output::ModelOutputReport {
    parse_model_output(text, SourceInfo::stdin("model-output.txt"), options)
        .payload
        .expect("model-output parsing always returns a report")
}

#[test]
fn raw_fenced_prose_yaml_toml_and_xml_candidates_are_retained() {
    let raw = parse(r#"{"raw":true}"#, &ModelOutputOptions::default());
    assert_eq!(raw.candidates[0].grammar, CandidateGrammar::RawJson);
    let input = concat!(
        "preface {\"prose\":true}\n",
        "```json\n{\"fenced\":true}\n```\n",
        "```yaml\nyaml: true\n```\n",
        "```toml\ntoml = true\n```\n",
        "<tool_call><name>Xml.Run</name><arguments>{\"xml\":true}</arguments></tool_call>"
    );
    let report = parse(input, &ModelOutputOptions::default());
    for grammar in [
        CandidateGrammar::JsonObjectInText,
        CandidateGrammar::FencedJson,
        CandidateGrammar::YamlBlock,
        CandidateGrammar::TomlBlock,
        CandidateGrammar::XmlToolCall,
    ] {
        assert!(
            report
                .candidates
                .iter()
                .any(|candidate| candidate.grammar == grammar)
        );
    }
    assert_eq!(report.status, ModelOutputStatus::Ambiguous);
    assert_eq!(report.selected_candidate_id, None);
    for candidate in &report.candidates {
        let range = candidate.raw_range.as_ref().expect("exact candidate range");
        assert_eq!(
            &input[range.byte_start..range.byte_end],
            candidate.raw_text.as_deref().unwrap()
        );
    }
}

#[test]
fn nested_stringified_json_is_recovered_without_losing_its_range() {
    let input = r#""{\"nested\":{\"ok\":true}}""#;
    let report = parse(input, &ModelOutputOptions::default());
    let candidate = &report.candidates[0];
    assert_eq!(candidate.value.as_ref().unwrap()["nested"]["ok"], true);
    assert!(
        candidate
            .normalizations
            .contains(&"parsed_stringified_json".to_string())
    );
    assert_eq!(candidate.raw_text.as_deref(), Some(input));
    assert_eq!(candidate.raw_range.as_ref().unwrap().byte_end, input.len());
}

#[test]
fn all_openai_tool_calls_and_mcp_batch_items_are_retained() {
    let openai = json!({"choices":[{"message":{"tool_calls":[
        {"function":{"name":"First", "arguments":"{\"n\":1}"}},
        {"function":{"name":"Second", "arguments":"{\"n\":2}"}}
    ]}}]})
    .to_string();
    let report = parse(&openai, &ModelOutputOptions::default());
    assert_eq!(report.candidates.len(), 2);
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.grammar == CandidateGrammar::OpenAiToolCall)
    );
    assert_eq!(report.candidates[0].command_name.as_deref(), Some("First"));
    assert_eq!(report.candidates[1].value.as_ref().unwrap()["n"], 2);
    let openai_ranges = report
        .candidates
        .iter()
        .map(|candidate| candidate.raw_range.as_ref().unwrap())
        .collect::<Vec<_>>();
    assert_ne!(openai_ranges[0], openai_ranges[1]);
    for candidate in &report.candidates {
        let range = candidate.raw_range.as_ref().unwrap();
        assert_eq!(
            &openai[range.byte_start..range.byte_end],
            candidate.raw_text.as_deref().unwrap()
        );
        assert!(candidate.raw_text.as_deref().unwrap().contains("function"));
    }
    assert_eq!(report.status, ModelOutputStatus::Ambiguous);
    assert!(report.selected_candidate_id.is_none());

    let mcp = json!([
        {"jsonrpc":"2.0", "id":1, "method":"tools/first", "params":{"n":1}},
        {"jsonrpc":"2.0", "id":2, "method":"tools/second", "params":{"n":2}}
    ])
    .to_string();
    let report = parse(&mcp, &ModelOutputOptions::default());
    assert_eq!(report.candidates.len(), 2);
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.grammar == CandidateGrammar::McpJsonRpc)
    );
    assert_eq!(
        report.candidates[0].argument_name.as_deref(),
        Some("params")
    );
    assert_eq!(report.candidates[1].value.as_ref().unwrap()["n"], 2);
    let mcp_ranges = report
        .candidates
        .iter()
        .map(|candidate| candidate.raw_range.as_ref().unwrap())
        .collect::<Vec<_>>();
    assert_ne!(mcp_ranges[0], mcp_ranges[1]);
    for candidate in &report.candidates {
        let range = candidate.raw_range.as_ref().unwrap();
        let raw = candidate.raw_text.as_deref().unwrap();
        assert_eq!(&mcp[range.byte_start..range.byte_end], raw);
        // The semantic value projects `params`, while the exact source slice
        // preserves the JSON-RPC envelope metadata for audit and round-tripping.
        assert!(raw.contains("jsonrpc"));
        assert!(raw.contains("id"));
    }
    assert_eq!(report.status, ModelOutputStatus::Ambiguous);
}

#[test]
fn python_calls_are_opt_in_and_aliases_are_runtime_only() {
    let input = "Old.Tool(payload={\"nodes\":[1]})";
    let disabled = parse(input, &ModelOutputOptions::default());
    assert!(
        disabled
            .candidates
            .iter()
            .all(|candidate| candidate.grammar != CandidateGrammar::PythonStyleCommand)
    );
    assert_eq!(
        disabled.candidates[0].value.as_ref().unwrap()["nodes"][0],
        1
    );
    let enabled = parse(
        input,
        &ModelOutputOptions {
            parse_python_style_commands: true,
            accepted_commands: vec!["Old.Tool".into()],
            accepted_arguments: vec!["payload".into()],
            aliases: AliasRules {
                command_aliases: vec![AliasRule {
                    from: "Old.Tool".into(),
                    to: "New.Tool".into(),
                }],
                argument_aliases: vec![AliasRule {
                    from: "payload".into(),
                    to: "request".into(),
                }],
                field_aliases: vec![AliasRule {
                    from: "nodes".into(),
                    to: "leaves".into(),
                }],
            },
            ..Default::default()
        },
    );
    let command = enabled
        .candidates
        .iter()
        .find(|candidate| candidate.grammar == CandidateGrammar::PythonStyleCommand)
        .unwrap();
    assert_eq!(command.command_name.as_deref(), Some("New.Tool"));
    assert_eq!(command.argument_name.as_deref(), Some("request"));
    assert_eq!(command.value.as_ref().unwrap()["leaves"][0], 1);
    assert_eq!(command.original_value.as_ref().unwrap()["nodes"][0], 1);
    assert_eq!(command.alias_applications.len(), 3);
    assert!(!command.trusted);
    assert!(
        disabled.candidates[0]
            .value
            .as_ref()
            .unwrap()
            .get("leaves")
            .is_none()
    );
}

#[test]
fn selection_requires_one_semantic_candidate() {
    let distinct = parse(
        "first {\"choice\":1} second {\"choice\":2}",
        &ModelOutputOptions::default(),
    );
    assert_eq!(distinct.candidates.len(), 2);
    assert_eq!(distinct.status, ModelOutputStatus::Ambiguous);
    assert!(distinct.selected_candidate_id.is_none());
    let duplicate = parse(
        "Tool.Run(arg={\"choice\":1})",
        &ModelOutputOptions {
            parse_python_style_commands: true,
            ..Default::default()
        },
    );
    assert_eq!(duplicate.candidates.len(), 2);
    assert_eq!(duplicate.status, ModelOutputStatus::Parsed);
    assert!(duplicate.selected_candidate_id.is_some());
}

#[test]
fn think_stripping_preserves_original_unicode_offsets_and_hash() {
    let input = "<think>priv?\nreasoning</think> prefix {\"ok\":true}";
    let report = parse(
        input,
        &ModelOutputOptions {
            strip_think_blocks: true,
            ..Default::default()
        },
    );
    let candidate = &report.candidates[0];
    let expected = "{\"ok\":true}";
    let start = input.find(expected).unwrap();
    let range = candidate.raw_range.as_ref().unwrap();
    assert_eq!(
        (range.byte_start, range.byte_end),
        (start, start + expected.len())
    );
    assert_eq!(candidate.raw_text.as_deref(), Some(expected));
    assert_eq!(range.start_line, 2);
    assert_eq!(
        report.raw_text_sha256,
        grist::core::sha256_hex(input.as_bytes())
    );
}

#[test]
fn hostile_prefixes_and_large_prose_do_not_hide_later_candidates() {
    let hostile = format!("{} tail {{\"retained\":true}}", "{".repeat(10_000));
    let report = parse(&hostile, &ModelOutputOptions::default());
    assert!(report.candidates.iter().any(|candidate| {
        candidate
            .value
            .as_ref()
            .is_some_and(|value| value["retained"] == true)
    }));
    let large = format!("{}{{\"large\":true}}", "x".repeat(1_000_000));
    let report = parse(&large, &ModelOutputOptions::default());
    let candidate = report
        .candidates
        .iter()
        .find(|candidate| {
            candidate
                .value
                .as_ref()
                .is_some_and(|value| value["large"] == true)
        })
        .unwrap();
    assert_eq!(candidate.raw_range.as_ref().unwrap().byte_start, 1_000_000);
}

#[test]
fn schema_validation_is_advisory_and_wire_shape_stays_stable() {
    let report = parse(
        "{\"retained\":true}",
        &ModelOutputOptions {
            schema: Some(json!({"type":"object", "required":["missing"]})),
            ..Default::default()
        },
    );
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(report.status, ModelOutputStatus::Parsed);
    assert!(!report.candidates[0].validation.as_ref().unwrap().valid);
    assert!(!report.candidates[0].trusted);
    assert!(
        report
            .failures
            .iter()
            .any(|failure| failure.failure_mode == "schema_validation_failed")
    );
    let wire = serde_json::to_value(&report).unwrap();
    assert!(wire["candidates"].is_array());
    assert!(wire["selected_candidate_id"].is_string());
    assert!(wire["candidates"][0]["validation"]["diagnostics"].is_array());
}

#[test]
fn surplus_tool_call_brace_golden_records_exact_bounded_provenance() {
    let input =
        r#"{"name":"apply_beat_text_edits","arguments":{"edits":[]}},"call_id":"call_min"}"#;
    let report = parse(input, &ModelOutputOptions::default());
    assert_eq!(report.status, ModelOutputStatus::Parsed);
    let candidate = &report.candidates[0];
    assert_eq!(candidate.status, CandidateStatus::Recovered);
    assert_eq!(candidate.raw_text.as_deref(), Some(input));
    assert_eq!(candidate.repairs.len(), 1);
    let repair = &candidate.repairs[0];
    assert_eq!(repair.kind, RepairKind::RemovePrematureClosingBrace);
    assert_eq!(
        (
            repair.original_text.as_str(),
            repair.replacement_text.as_str()
        ),
        ("}", "")
    );
    let expected_offset = input.find(r#"}},"call_id""#).unwrap() + 1;
    assert_eq!(
        (repair.input_byte_start, repair.input_byte_end),
        (expected_offset, expected_offset + 1)
    );
    let source_range = repair.source_range.as_ref().unwrap();
    assert_eq!(
        (source_range.byte_start, source_range.byte_end),
        (expected_offset, expected_offset + 1)
    );
    assert_eq!(&input[source_range.byte_start..source_range.byte_end], "}");
    assert_eq!(
        candidate.repaired_text.as_ref().unwrap().len(),
        input.len() - 1
    );
    assert!(candidate.json_error.is_some());
    assert!(!candidate.trusted);
    assert!(
        candidate
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "json.repaired")
    );
}

#[test]
fn synthetic_reported_2544_byte_response_and_nested_variants_recover() {
    let prefix = r#"{"name":"apply_beat_text_edits","arguments":{"edits":[],"padding":""#;
    let suffix = r#""}},"call_id":"call_2544"}"#;
    let input = format!(
        "{prefix}{}{suffix}",
        "x".repeat(2544 - prefix.len() - suffix.len())
    );
    assert_eq!(input.len(), 2544);
    let report = parse(&input, &ModelOutputOptions::default());
    assert_eq!(report.candidates[0].status, CandidateStatus::Recovered);
    assert_eq!(report.candidates[0].repairs.len(), 1);

    for nested in [
        r#"[{"name":"apply_beat_text_edits","arguments":{"edits":[]}},"call_id":"array"}]"#,
        r#"{"tool_calls":[{"function":{"name":"apply_beat_text_edits","arguments":{"edits":[]}}},"call_id":"nested"}]}"#,
    ] {
        let report = parse(nested, &ModelOutputOptions::default());
        assert!(
            report.candidates.iter().any(|candidate| {
                candidate.status == CandidateStatus::Recovered
                    && candidate
                        .repairs
                        .iter()
                        .any(|repair| repair.kind == RepairKind::RemovePrematureClosingBrace)
            }),
            "nested tool call was not recovered: {nested}"
        );
    }
}

#[test]
fn ambiguous_and_non_tool_surplus_braces_fail_closed_and_preserve_raw_candidate() {
    for input in [
        r#"{"a":1},"b":2}"#,
        r#"{"example":{"name":"not_a_call","arguments":{}}},"extra":1}"#,
        r#"[{"name":"a","arguments":{}},"call_id":"1"},{"name":"b","arguments":{}},"call_id":"2"}]"#,
    ] {
        let report = parse(input, &ModelOutputOptions::default());
        let raw = report
            .candidates
            .iter()
            .find(|candidate| {
                candidate
                    .raw_range
                    .as_ref()
                    .is_some_and(|range| range.byte_start == 0 && range.byte_end == input.len())
            })
            .expect("the complete malformed candidate is retained");
        assert_eq!(raw.status, CandidateStatus::Malformed);
        assert!(raw.repairs.is_empty());
        assert_eq!(raw.raw_text.as_deref(), Some(input));
        assert!(raw.json_error.is_some());
    }
}

#[test]
fn valid_controls_are_byte_stable_and_repair_bounds_fail_closed() {
    let valid = r#"{"name":"apply_beat_text_edits","arguments":{"edits":[]},"call_id":"valid"}"#;
    let report = parse(valid, &ModelOutputOptions::default());
    let candidate = &report.candidates[0];
    assert_eq!(candidate.status, CandidateStatus::Complete);
    assert!(candidate.repairs.is_empty());
    assert!(candidate.repaired_text.is_none());
    assert!(candidate.original_value.is_some());

    let malformed =
        r#"{"name":"apply_beat_text_edits","arguments":{"edits":[]}},"call_id":"bounded"}"#;
    let report = parse(
        malformed,
        &ModelOutputOptions {
            repair_limits: RepairLimits {
                max_operations: 0,
                max_changed_bytes: 0,
            },
            ..Default::default()
        },
    );
    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.raw_range.as_ref().unwrap().byte_end == malformed.len())
        .unwrap();
    assert_eq!(candidate.status, CandidateStatus::Malformed);
    assert!(candidate.repairs.is_empty());
    assert_eq!(candidate.raw_text.as_deref(), Some(malformed));
    assert!(
        candidate
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "json.repair_bounds_exceeded")
    );
}

#[test]
fn completed_surplus_brace_repair_does_not_rewrite_valid_string_data() {
    let input = r#"{"name":"tool","arguments":{"note":"None"}},"call_id":"x"}"#;
    let report = parse(input, &ModelOutputOptions::default());
    let candidate = &report.candidates[0];
    assert_eq!(candidate.status, CandidateStatus::Recovered);
    assert_eq!(candidate.value.as_ref().unwrap()["note"], "None");
    assert_eq!(candidate.repairs.len(), 1);
    assert_eq!(
        candidate.repairs[0].kind,
        RepairKind::RemovePrematureClosingBrace
    );
    assert!(
        !candidate
            .normalizations
            .contains(&"replaced_python_literals".to_string())
    );
}

#[test]
fn stringified_argument_repairs_honor_limits_and_retain_provenance() {
    let input = r#"{"name":"tool","arguments":"{\"value\":1,}"}"#;
    let bounded = parse(
        input,
        &ModelOutputOptions {
            repair_limits: RepairLimits {
                max_operations: 0,
                max_changed_bytes: 0,
            },
            ..Default::default()
        },
    );
    let bounded_candidate = &bounded.candidates[0];
    assert_eq!(bounded_candidate.value.as_ref().unwrap(), "{\"value\":1,}");
    assert!(bounded_candidate.repairs.is_empty());
    assert!(
        !bounded_candidate
            .normalizations
            .contains(&"parsed_root_stringified_arguments".to_string())
    );

    let recovered = parse(input, &ModelOutputOptions::default());
    let candidate = &recovered.candidates[0];
    assert_eq!(candidate.value.as_ref().unwrap()["value"], 1);
    assert_eq!(candidate.status, CandidateStatus::Recovered);
    assert_eq!(candidate.repairs.len(), 1);
    assert_eq!(candidate.repairs[0].kind, RepairKind::RemoveTrailingComma);
    assert!(candidate.repairs[0].source_range.is_some());
    assert!(candidate.repaired_text.is_some());
    assert!(candidate.json_error.is_some());
}

#[test]
fn selected_incomplete_malformed_no_candidate_and_ambiguity_remain_distinct() {
    let incomplete_input = r#"{"name":"apply_beat_text_edits","arguments":"#;
    let incomplete = parse(incomplete_input, &ModelOutputOptions::default());
    assert_eq!(incomplete.status, ModelOutputStatus::Incomplete);
    assert!(incomplete.selected_candidate_id.is_some());
    let candidate = &incomplete.candidates[0];
    assert_eq!(candidate.status, CandidateStatus::Incomplete);
    assert_eq!(
        candidate.json_error.as_ref().unwrap().byte_offset,
        incomplete_input.len() - 1
    );

    let malformed = parse(
        "{\n\"name\":\"apply_beat_text_edits\",\n\"arguments\":{\"edits\":[]}},\"call_id\":???}",
        &ModelOutputOptions::default(),
    );
    assert_eq!(malformed.status, ModelOutputStatus::Malformed);
    assert!(malformed.selected_candidate_id.is_some());
    let error = malformed.candidates[0].json_error.as_ref().unwrap();
    assert!(error.byte_offset > 0);
    assert!(error.line >= 3);
    assert!(error.column > 0);

    let none = parse("plain prose", &ModelOutputOptions::default());
    assert_eq!(none.status, ModelOutputStatus::Unparsed);
    assert!(none.candidates.is_empty());

    let ambiguous = parse(
        r#"first {"a":1} second {"b":2}"#,
        &ModelOutputOptions::default(),
    );
    assert_eq!(ambiguous.status, ModelOutputStatus::Ambiguous);
    assert!(ambiguous.selected_candidate_id.is_none());
}

#[test]
fn bounded_repair_property_matrix_never_changes_valid_controls() {
    for size in 0..64 {
        let note = "x".repeat(size);
        let valid = format!(
            r#"{{"name":"apply_beat_text_edits","arguments":{{"edits":[],"note":"{note}"}},"call_id":"ok"}}"#
        );
        let valid_report = parse(&valid, &ModelOutputOptions::default());
        assert!(valid_report.candidates[0].repairs.is_empty());
        assert_eq!(
            valid_report.candidates[0].raw_text.as_deref(),
            Some(valid.as_str())
        );

        let malformed = format!(
            r#"{{"name":"apply_beat_text_edits","arguments":{{"edits":[],"note":"{note}"}}}},"call_id":"fixed"}}"#
        );
        let repaired = parse(&malformed, &ModelOutputOptions::default());
        let candidate = &repaired.candidates[0];
        assert_eq!(candidate.repairs.len(), 1);
        assert_eq!(
            candidate.repairs[0].kind,
            RepairKind::RemovePrematureClosingBrace
        );
        assert_eq!(candidate.raw_text.as_deref(), Some(malformed.as_str()));
        serde_json::from_str::<serde_json::Value>(candidate.repaired_text.as_ref().unwrap())
            .expect("every repaired property case is valid JSON");
    }
}

#[test]
fn every_byte_chunking_converges_to_batch_candidates() {
    let fixture = include_bytes!("../fixtures/generated/model_output/streaming-tool-call.txt");
    let text = std::str::from_utf8(fixture).unwrap();
    let options = ModelOutputOptions::default();
    let batch = parse_model_output(text, SourceInfo::stdin("stream-fixture"), &options);

    for split in 0..=fixture.len() {
        let mut parser =
            StreamingModelOutputParserV2::new(SourceInfo::stdin("stream-fixture"), options.clone());
        let mut events = parser.push_bytes(&fixture[..split]);
        events.extend(parser.push_bytes(&fixture[split..]));
        let (finish_events, streamed) = parser.finish();
        events.extend(finish_events);
        assert_eq!(
            streamed.payload.as_ref().unwrap().candidates,
            batch.payload.as_ref().unwrap().candidates,
            "candidate mismatch at byte split {split}"
        );
        assert_eq!(
            streamed, batch,
            "batch envelope mismatch at byte split {split}"
        );
        assert_eq!(
            events.last().map(event_name),
            Some("terminal"),
            "terminal event mismatch at byte split {split}"
        );
        assert_stable_candidate_lifecycle(&events, split);
    }

    let mut bytewise =
        StreamingModelOutputParserV2::new(SourceInfo::stdin("stream-fixture"), options);
    for byte in fixture {
        bytewise.push_bytes(std::slice::from_ref(byte));
    }
    let (_, streamed) = bytewise.finish();
    assert_eq!(streamed, batch);
}

#[test]
fn streaming_events_are_ordered_and_repairs_are_deferred_until_finish() {
    let mut parser = StreamingModelOutputParserV2::new(
        SourceInfo::stdin("ordered-stream"),
        ModelOutputOptions::default(),
    );
    let first = parser.push_chunk(r#"{"name":"tool","arguments":{"value":1}}"#);
    assert!(matches!(
        first.as_slice(),
        [ModelOutputStreamEventV2::ParserStateChanged {
            state: StreamingStateV2::Accumulating
        }]
    ));
    assert!(
        !first
            .iter()
            .any(|event| matches!(event, ModelOutputStreamEventV2::CandidateCompleted { .. }))
    );

    let update = parser.push_chunk(r#", "call_id":"stream"}"#);
    assert!(update.is_empty());
    let (finish_events, envelope) = parser.finish();
    assert!(matches!(
        finish_events.last(),
        Some(ModelOutputStreamEventV2::Terminal { terminal })
            if terminal.status == OperationStatus::Complete && terminal.emitted_items == 1
    ));
    let (completed_id, completed) = finish_events
        .iter()
        .find_map(|event| match event {
            ModelOutputStreamEventV2::CandidateCompleted {
                candidate_id,
                candidate,
            } => Some((candidate_id, candidate)),
            _ => None,
        })
        .unwrap();
    assert_eq!(completed.status, CandidateStatus::Recovered);
    assert_eq!(
        completed.repairs[0].kind,
        RepairKind::RemovePrematureClosingBrace
    );
    assert_eq!(completed, &envelope.payload.as_ref().unwrap().candidates[0]);
    let started = finish_events.iter().find_map(|event| match event {
        ModelOutputStreamEventV2::CandidateStarted {
            candidate_id,
            grammar,
        } => Some((candidate_id, grammar)),
        _ => None,
    });
    assert_eq!(started, Some((completed_id, &completed.grammar)));
}

#[test]
fn closed_prefix_candidate_emits_incremental_stable_updates_before_finish() {
    let mut parser = StreamingModelOutputParserV2::new(
        SourceInfo::stdin("incremental-prefix"),
        ModelOutputOptions::default(),
    );
    let mut events = parser.push_chunk("```json\n{\"value\":1}\n```");
    let started = events
        .iter()
        .find_map(|event| match event {
            ModelOutputStreamEventV2::CandidateStarted {
                candidate_id,
                grammar,
            } => Some((candidate_id.clone(), grammar.clone())),
            _ => None,
        })
        .expect("closed fenced candidate starts before finish");
    assert_eq!(started.1, CandidateGrammar::FencedJson);
    assert!(events.iter().any(|event| matches!(
        event,
        ModelOutputStreamEventV2::CandidateUpdated { candidate_id, .. }
            if candidate_id == &started.0
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ModelOutputStreamEventV2::CandidateCompleted { .. }))
    );

    let update = parser.push_chunk("\ntrailing prose");
    assert!(update.iter().any(|event| matches!(
        event,
        ModelOutputStreamEventV2::CandidateUpdated { candidate_id, .. }
            if candidate_id == &started.0
    )));
    events.extend(update);
    let (finish_events, _) = parser.finish();
    events.extend(finish_events);
    assert_stable_candidate_lifecycle(&events, 0);
}

#[test]
fn incomplete_cancelled_bounded_and_invalid_streams_are_explicit() {
    let mut incomplete = StreamingModelOutputParserV2::new(
        SourceInfo::stdin("incomplete"),
        ModelOutputOptions::default(),
    );
    incomplete.push_chunk(r#"{"value":"#);
    let (events, envelope) = incomplete.finish();
    assert_eq!(
        envelope.payload.as_ref().unwrap().status,
        ModelOutputStatus::Incomplete
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ModelOutputStreamEventV2::ParserStateChanged {
            state: StreamingStateV2::Incomplete
        }
    )));
    assert!(matches!(
        events.last(),
        Some(ModelOutputStreamEventV2::Terminal { terminal })
            if terminal.status == OperationStatus::Partial
    ));

    let cancellation = CancellationToken::new();
    let mut cancelled = StreamingModelOutputParserV2::new_with_cancellation(
        SourceInfo::stdin("cancelled"),
        ModelOutputOptions::default(),
        cancellation.clone(),
    );
    cancelled.push_chunk(r#"{"retained":true"#);
    cancellation.cancel();
    let (events, envelope) = cancelled.finish();
    assert_eq!(envelope.status, OperationStatus::Cancelled);
    assert!(envelope.payload.is_none());
    assert!(matches!(
        events.as_slice(),
        [
            ModelOutputStreamEventV2::Diagnostic { .. },
            ModelOutputStreamEventV2::ParserStateChanged {
                state: StreamingStateV2::Cancelled
            },
            ModelOutputStreamEventV2::Terminal { terminal }
        ] if terminal.status == OperationStatus::Cancelled
    ));

    let mut bounded = StreamingModelOutputParserV2::new_with_limit(
        SourceInfo::stdin("bounded"),
        ModelOutputOptions::default(),
        7,
    );
    bounded.push_bytes(b"{\"a\":1}");
    let events = bounded.push_bytes(b"x");
    assert_eq!(bounded.buffered_bytes(), 7);
    assert_eq!(bounded.max_buffer_bytes(), 7);
    assert!(bounded.is_terminated());
    assert!(matches!(
        events.last(),
        Some(ModelOutputStreamEventV2::Terminal { terminal })
            if terminal.status == OperationStatus::Failed
    ));
    assert!(bounded.push_bytes(b"ignored").is_empty());
    let (finish_events, envelope) = bounded.finish();
    assert!(
        finish_events.is_empty(),
        "no events may follow the terminal"
    );
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert!(envelope.payload.is_none());
    assert_eq!(envelope.hashes.as_ref().unwrap().size_bytes, 7);

    let mut invalid = StreamingModelOutputParserV2::new(
        SourceInfo::stdin("invalid-utf8"),
        ModelOutputOptions::default(),
    );
    invalid.push_bytes(b"{\"value\":\"");
    invalid.push_bytes(&[0xf0, 0x9f]);
    let (events, envelope) = invalid.finish();
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert!(events.iter().any(|event| matches!(
        event,
        ModelOutputStreamEventV2::Diagnostic { diagnostic }
            if diagnostic.code.as_str() == "stream.invalid_utf8"
    )));
    assert!(matches!(
        events.last(),
        Some(ModelOutputStreamEventV2::Terminal { terminal })
            if terminal.status == OperationStatus::Failed
    ));
    let diagnostic = events
        .iter()
        .find_map(|event| match event {
            ModelOutputStreamEventV2::Diagnostic { diagnostic }
                if diagnostic.code.as_str() == "stream.invalid_utf8" =>
            {
                Some(diagnostic)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(diagnostic.range.as_ref().unwrap().byte_start, 10);
}

#[test]
fn bytewise_utf8_chunks_converge_to_batch_candidates() {
    for unicode in ["{\"note\":\"Δ\"}", "{\"note\":\"€\"}", "{\"note\":\"😀\"}"] {
        let unicode_batch = parse_model_output(
            unicode,
            SourceInfo::stdin("unicode-stream"),
            &ModelOutputOptions::default(),
        );
        let mut unicode_stream = StreamingModelOutputParserV2::new(
            SourceInfo::stdin("unicode-stream"),
            ModelOutputOptions::default(),
        );
        for byte in unicode.as_bytes() {
            unicode_stream.push_bytes(std::slice::from_ref(byte));
        }
        let (_, unicode_result) = unicode_stream.finish();
        assert_eq!(unicode_result, unicode_batch);
    }
}

#[test]
fn multi_candidate_events_keep_batch_ids_and_grammars_at_every_split() {
    let input = br#"prefix {"a":1} middle {"b":2} suffix"#;
    let options = ModelOutputOptions::default();
    let batch = parse_model_output(
        std::str::from_utf8(input).unwrap(),
        SourceInfo::stdin("multi"),
        &options,
    );
    assert!(batch.payload.as_ref().unwrap().candidates.len() >= 2);

    for split in 0..=input.len() {
        let mut parser =
            StreamingModelOutputParserV2::new(SourceInfo::stdin("multi"), options.clone());
        let mut events = parser.push_bytes(&input[..split]);
        events.extend(parser.push_bytes(&input[split..]));
        let (finish_events, streamed) = parser.finish();
        events.extend(finish_events);
        assert_eq!(streamed, batch, "batch mismatch at split {split}");

        let starts = events
            .iter()
            .filter_map(|event| match event {
                ModelOutputStreamEventV2::CandidateStarted {
                    candidate_id,
                    grammar,
                } => Some((candidate_id.clone(), grammar.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        let completed = events
            .iter()
            .filter_map(|event| match event {
                ModelOutputStreamEventV2::CandidateCompleted {
                    candidate_id,
                    candidate,
                } => Some((candidate_id.clone(), candidate.grammar.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            starts, completed,
            "identity/grammar mismatch at split {split}"
        );
        assert_stable_candidate_lifecycle(&events, split);
    }
}

#[test]
fn successful_terminals_match_authoritative_batch_status() {
    let cases = [
        ("", StreamingStateV2::Empty),
        (r#"{"a":"#, StreamingStateV2::Incomplete),
        (r#"{"a":}"#, StreamingStateV2::Malformed),
        (
            r#"first {"a":1} second {"b":2}"#,
            StreamingStateV2::Ambiguous,
        ),
        ("plain prose", StreamingStateV2::Unparsed),
        (r#"{"a":1}"#, StreamingStateV2::Complete),
    ];
    for (input, expected_state) in cases {
        let source = SourceInfo::stdin("terminal-state");
        let batch = parse_model_output(input, source.clone(), &ModelOutputOptions::default());
        let mut parser = StreamingModelOutputParserV2::new(source, ModelOutputOptions::default());
        parser.push_chunk(input);
        let (events, streamed) = parser.finish();
        assert_eq!(streamed, batch);
        assert!(
            events.iter().any(|event| matches!(
                event,
                ModelOutputStreamEventV2::ParserStateChanged { state } if state == &expected_state
            )),
            "missing state {expected_state:?} for {input:?}: {events:?}"
        );
        assert!(matches!(
            events.last(),
            Some(ModelOutputStreamEventV2::Terminal { terminal }) if terminal.status == batch.status
        ));
    }
}

#[test]
fn v1_streaming_parser_source_contract_remains_available() {
    let mut parser = StreamingModelOutputParser::new(
        SourceInfo::stdin("v1-source-compatibility"),
        ModelOutputOptions::default(),
    );
    let _: Vec<ModelOutputEvent> = parser.push_chunk(r#"{"a":1}"#);
    let (_events, envelope): (Vec<ModelOutputEvent>, _) = parser.finish();
    assert_eq!(envelope.status, OperationStatus::Complete);
}

#[test]
fn limited_constructor_retains_untrusted_caps() {
    let parser = StreamingModelOutputParserV2::new_with_limit(
        SourceInfo::stdin("limited-untrusted"),
        ModelOutputOptions::default(),
        1024,
    );
    let budget = parser.resource_budget();
    assert_eq!(budget.max_input_bytes, Some(1024));
    assert_eq!(budget.max_decoded_characters, Some(1024));
    assert_eq!(budget.max_pages, Some(10_000));
    assert_eq!(budget.max_records, Some(1_000_000));
    assert_eq!(budget.max_nodes, Some(2_000_000));
    assert_eq!(budget.max_parse_millis, Some(120_000));
    assert_eq!(budget.max_output_bytes, Some(268_435_456));
}

#[test]
fn decoded_character_rejection_precedes_retention_and_reallocation() {
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_decoded_characters = Some(0);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), CancellationToken::new()).unwrap();
    let mut parser = StreamingModelOutputParserV2::new_with_control(
        SourceInfo::stdin("decoded-preflight"),
        ModelOutputOptions::default(),
        1024,
        control,
    );
    assert!(parser.push_bytes(&[0xf0, 0x9f]).is_empty());
    let bytes_before = parser.buffered_bytes();
    let capacity_before = parser.buffered_capacity();
    let usage_before = parser.budget_usage();

    let events = parser.push_bytes(&[0x98, 0x80]);
    assert!(matches!(
        events.last(),
        Some(ModelOutputStreamEventV2::Terminal { terminal })
            if terminal.status == OperationStatus::Failed
    ));
    assert_eq!(parser.buffered_bytes(), bytes_before);
    assert_eq!(parser.buffered_capacity(), capacity_before);
    let usage_after = parser.budget_usage();
    assert_eq!(usage_after.input_bytes, usage_before.input_bytes);
    assert_eq!(usage_after.memory_bytes, usage_before.memory_bytes);
    assert_eq!(usage_after.decoded_characters, 1);
}

#[test]
fn shared_operation_control_enforces_memory_before_retention() {
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_memory_bytes = Some(1);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), CancellationToken::new()).unwrap();
    let mut parser = StreamingModelOutputParserV2::new_with_control(
        SourceInfo::stdin("budgeted"),
        ModelOutputOptions::default(),
        1024,
        control,
    );
    let events = parser.push_bytes(b"{");
    assert_eq!(parser.buffered_bytes(), 0);
    assert!(matches!(
        events.last(),
        Some(ModelOutputStreamEventV2::Terminal { terminal })
            if terminal.status == OperationStatus::Failed
                && terminal.budget_usage.input_bytes == 1
                && terminal.budget_usage.memory_bytes > 1
    ));
    assert!(parser.push_bytes(b"ignored").is_empty());
    let (finish_events, _) = parser.finish();
    assert!(finish_events.is_empty());
}

fn event_name(event: &ModelOutputStreamEventV2) -> &'static str {
    match event {
        ModelOutputStreamEventV2::CandidateStarted { .. } => "candidate_started",
        ModelOutputStreamEventV2::CandidateUpdated { .. } => "candidate_updated",
        ModelOutputStreamEventV2::CandidateCompleted { .. } => "candidate_completed",
        ModelOutputStreamEventV2::Diagnostic { .. } => "diagnostic",
        ModelOutputStreamEventV2::ParserStateChanged { .. } => "parser_state_changed",
        ModelOutputStreamEventV2::Terminal { .. } => "terminal",
    }
}

fn assert_stable_candidate_lifecycle(events: &[ModelOutputStreamEventV2], split: usize) {
    let starts = events
        .iter()
        .filter_map(|event| match event {
            ModelOutputStreamEventV2::CandidateStarted {
                candidate_id,
                grammar,
            } => Some((candidate_id.clone(), grammar.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let completed = events
        .iter()
        .filter_map(|event| match event {
            ModelOutputStreamEventV2::CandidateCompleted {
                candidate_id,
                candidate,
            } => Some((candidate_id.clone(), candidate.grammar.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        starts, completed,
        "candidate lifecycle mismatch at split {split}"
    );
    for (candidate_id, grammar) in starts {
        assert!(
            events.iter().any(|event| matches!(
                event,
                ModelOutputStreamEventV2::CandidateUpdated {
                    candidate_id: updated_id,
                    ..
                } if updated_id == &candidate_id
            )),
            "missing update for {candidate_id} ({grammar:?}) at split {split}"
        );
    }
}
