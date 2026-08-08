#![cfg(feature = "model-output")]

use grist::core::SourceInfo;
use grist::model_output::{
    AliasRule, AliasRules, CandidateGrammar, ModelOutputOptions, ModelOutputStatus,
    parse_model_output,
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
    let wire = serde_json::to_value(&report).unwrap();
    assert!(wire["candidates"].is_array());
    assert!(wire["selected_candidate_id"].is_string());
    assert!(wire["candidates"][0]["validation"]["diagnostics"].is_array());
}
