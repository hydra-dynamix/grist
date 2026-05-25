#![cfg(feature = "cli")]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn grist() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_grist"))
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dir.push(format!("grist-e2e-{name}-{nonce}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_stdin(args: &[&str], input: &str) -> serde_json::Value {
    let mut child = Command::new(grist())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn run(args: &[&str]) -> serde_json::Value {
    let output = Command::new(grist()).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn validate_with_schema(value: &serde_json::Value, schema_name: &str) {
    let schema = run(&["schema", "emit", schema_name]);
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<_> = validator.iter_errors(value).collect();
    assert!(
        errors.is_empty(),
        "schema errors for {schema_name}: {errors:?}"
    );
}

#[test]
fn cli_parses_serialization_with_schema_validation_end_to_end() {
    let dir = temp_dir("json-schema");
    let schema_path = dir.join("schema.json");
    fs::write(
        &schema_path,
        r#"{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}"#,
    )
    .unwrap();
    let output = run_stdin(
        &[
            "parse",
            "json",
            "-",
            "--schema",
            schema_path.to_str().unwrap(),
        ],
        r#"{"name":42}"#,
    );
    validate_with_schema(&output, "serialization-envelope");
    assert_eq!(output["payload"]["validation"]["valid"], false);
    assert!(
        output["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diag| { diag["code"] == "schema.validation" })
    );
}

#[test]
fn cli_parses_model_output_rules_schema_and_think_blocks_end_to_end() {
    let dir = temp_dir("model-output");
    let schema_path = dir.join("schema.json");
    let rules_path = dir.join("rules.json");
    fs::write(&schema_path, r#"{"type":"object","required":["leaves"]}"#).unwrap();
    fs::write(
        &rules_path,
        r#"{"field_aliases":[{"from":"nodes","to":"leaves"}],"command_aliases":[{"from":"Old.Tool","to":"New.Tool"}],"argument_aliases":[]}"#,
    )
    .unwrap();
    let output = run_stdin(
        &[
            "parse",
            "model-output",
            "-",
            "--schema",
            schema_path.to_str().unwrap(),
            "--rules",
            rules_path.to_str().unwrap(),
            "--strip-think-blocks",
        ],
        "<think>private reasoning</think>Old.Tool(arg={nodes:[1,], ok: True})",
    );
    validate_with_schema(&output, "model-output-envelope");
    let candidates = output["payload"]["candidates"].as_array().unwrap();
    let command = candidates
        .iter()
        .find(|candidate| candidate["grammar"] == "python_style_command")
        .unwrap();
    assert_eq!(command["command_name"], "New.Tool");
    assert_eq!(command["validation"]["valid"], true);
}

#[test]
fn cli_ingests_repo_with_filters_and_external_artifacts() {
    let repo = temp_dir("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("src/lib.rs"),
        "#[test]\nfn it_works() {}\npub struct Thing;\n",
    )
    .unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(repo.join("Cargo.lock"), "# lock\n").unwrap();
    fs::write(repo.join("README.md"), "# Repo\n").unwrap();
    fs::write(repo.join("skip.json"), "{\"skip\":true}\n").unwrap();
    let artifact_dir = repo.join("artifacts");
    let output = run(&[
        "ingest",
        "repo",
        repo.to_str().unwrap(),
        "--include",
        "src/**",
        "--include",
        "Cargo.*",
        "--external-artifact-dir",
        artifact_dir.to_str().unwrap(),
    ]);
    assert_eq!(output["kind"], "repo_ingest");
    assert_eq!(output["payload"]["detected_languages"][0], "rust");
    assert!(
        output["payload"]["manifest_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "Cargo.toml")
    );
    assert!(
        output["payload"]["lockfile_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "Cargo.lock")
    );
    assert!(
        output["payload"]["test_hints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hint| hint["name"] == "it_works")
    );
    assert_eq!(output["payload"]["artifacts"].as_array().unwrap().len(), 3);
    let artifact_ref = output["payload"]["artifacts"][0]["artifact_ref"]
        .as_str()
        .unwrap();
    assert!(Path::new(artifact_ref).exists());
    assert!(output["payload"]["artifacts"][0]["artifact"].is_null());
}

#[test]
fn cli_ingests_plain_text_blocks_for_research_chain_documents() {
    let output = run_stdin(
        &["ingest", "file", "-", "--filename", "notes.txt"],
        "first block\n\nsecond block\n",
    );
    assert_eq!(output["kind"], "file_ingest");
    let artifact = &output["payload"]["artifact"];
    assert_eq!(artifact["kind"], "text");
    assert_eq!(artifact["payload"]["blocks"].as_array().unwrap().len(), 2);
}

#[test]
fn cli_rust_detail_mode_exposes_syntax_debug() {
    let output = run_stdin(
        &["parse", "rust", "-", "--detail", "syntax-debug"],
        "pub fn run() {}",
    );
    validate_with_schema(&output, "rust-code-envelope");
    assert_eq!(output["payload"]["detail"]["root_kind"], "source_file");
}
