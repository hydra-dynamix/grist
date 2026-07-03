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
fn cli_parses_markdown_rich_structures_end_to_end() {
    let output = run_stdin(
        &["parse", "markdown", "-"],
        "---\ntitle: Test\n---\n# Heading\n\nSee [Grist](https://example.test \"docs\").\n\n```rust\nfn main() {}\n```\n\n| name | score |\n| :--- | ---: |\n| alpha | 1 |\n",
    );
    validate_with_schema(&output, "markdown-envelope");
    assert_eq!(output["kind"], "markdown");
    assert_eq!(output["payload"]["frontmatter"]["value"]["title"], "Test");

    let nodes = output["payload"]["nodes"].as_array().unwrap();
    let heading = nodes.iter().find(|node| node["kind"] == "heading").unwrap();
    assert_eq!(heading["text"], "Heading");
    assert!(heading["range"].is_object());

    let paragraph = nodes
        .iter()
        .find(|node| node["kind"] == "paragraph")
        .unwrap();
    assert_eq!(paragraph["text"], "See Grist.");

    let link = nodes.iter().find(|node| node["kind"] == "link").unwrap();
    assert_eq!(link["text"], "Grist");
    assert_eq!(link["destination"], "https://example.test");
    assert_eq!(link["title"], "docs");
    assert!(link["range"].is_object());

    let fence = nodes
        .iter()
        .find(|node| node["kind"] == "code_fence")
        .unwrap();
    assert_eq!(fence["language"], "rust");
    assert!(fence["range"].is_object());

    let table = nodes.iter().find(|node| node["kind"] == "table").unwrap();
    assert_eq!(table["table"]["rows"][0][0], "name");
    assert_eq!(
        table["table"]["alignments"],
        serde_json::json!(["left", "right"])
    );
    assert_eq!(table["table"]["row_details"][0]["header"], true);
    assert!(table["table"]["row_details"][0]["cells"][0]["range"].is_object());
}

#[test]
fn cli_parses_ldgr_projection_ticket_end_to_end() {
    let output = run_stdin(
        &["parse", "ldgr-projection", "-"],
        "---\nldgr_doc: 1\nkind: ticket\nid: ticket.cli\nschema: ldgr.ticket.v1\n---\n# Context ignored\n\n```ldgr-contract yaml\ntitle: CLI Ticket\ndescription: Validate CLI projection parsing.\nrequirements:\n  - id: req.cli\n    text: CLI produces a typed ticket\nvalidation_instructions:\n  - run this e2e test\n```\n",
    );
    validate_with_schema(&output, "ldgr-projection-envelope");
    assert_eq!(output["kind"], "ldgr_projection");
    assert_eq!(output["payload"]["metadata"]["id"], "ticket.cli");
    assert_eq!(output["payload"]["typed"]["kind"], "ticket");
    assert_eq!(
        output["payload"]["typed"]["document"]["title"],
        "CLI Ticket"
    );
    assert!(output["diagnostics"].as_array().unwrap().is_empty());
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
            "--python-style",
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
fn cli_repairs_model_output_and_can_emit_selected_json_value() {
    let dir = temp_dir("model-output-json-value");
    let schema_path = dir.join("schema.json");
    fs::write(
        &schema_path,
        r#"{"type":"object","required":["section_chunks"]}"#,
    )
    .unwrap();
    let input = r#"{"narrative_contract": {}} {"section_chunks": [], "narration_text": "60 seconds" "target_words": 130}"#;
    let envelope = run_stdin(
        &[
            "parse",
            "model-output",
            "-",
            "--schema",
            schema_path.to_str().unwrap(),
        ],
        input,
    );
    validate_with_schema(&envelope, "model-output-envelope");
    let selected_id = envelope["payload"]["selected_candidate_id"]
        .as_str()
        .unwrap();
    let selected = envelope["payload"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == selected_id)
        .unwrap();
    assert_eq!(selected["value"]["target_words"], 130);
    assert!(
        selected["raw_text"]
            .as_str()
            .unwrap()
            .contains("section_chunks")
    );
    assert!(
        selected["normalizations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|normalization| normalization == "inserted_missing_comma")
    );

    let value = run_stdin(
        &[
            "parse",
            "model-output",
            "-",
            "--schema",
            schema_path.to_str().unwrap(),
            "--json-value",
        ],
        input,
    );
    assert_eq!(value["target_words"], 130);
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
fn cli_parses_html_fragment_with_htmx_attributes() {
    let output = run_stdin(
        &["parse", "html", "-", "--mode", "fragment"],
        r##"<button hx-post="/save" data-hx-target="#result">Save</button><div id="result"></div>"##,
    );
    validate_with_schema(&output, "html-envelope");
    assert_eq!(output["kind"], "html");
    assert_eq!(output["payload"]["mode"], "fragment");
    let htmx_attributes = output["payload"]["htmx_attributes"].as_array().unwrap();
    assert!(
        htmx_attributes
            .iter()
            .any(|attribute| attribute["normalized_name"] == "hx-post")
    );
    assert!(
        htmx_attributes
            .iter()
            .any(|attribute| attribute["normalized_name"] == "hx-target")
    );
}

#[test]
fn cli_parses_csv_with_headers_end_to_end() {
    let output = run_stdin(
        &["parse", "csv", "-"],
        "name,score,ok\nalpha,1,true\nbeta,,false\n",
    );
    validate_with_schema(&output, "csv-envelope");
    assert_eq!(output["kind"], "csv");
    assert_eq!(output["payload"]["record_count"], 2);
    assert_eq!(output["payload"]["headers"][0]["name"], "name");
    assert_eq!(output["payload"]["rows"][0]["cells"][1]["value"], 1);
    assert_eq!(
        output["payload"]["rows"][1]["cells"][1]["value"],
        serde_json::Value::Null
    );
}

#[test]
fn cli_ingests_csv_files_as_first_class_artifacts() {
    let output = run_stdin(
        &["ingest", "file", "-", "--filename", "metrics.csv"],
        "name,score\nalpha,1\n",
    );
    assert_eq!(output["kind"], "file_ingest");
    assert_eq!(output["payload"]["detection"]["content_kind"], "csv");
    let artifact = &output["payload"]["artifact"];
    assert_eq!(artifact["kind"], "csv");
    assert_eq!(artifact["payload"]["rows"][0]["cells"][0]["raw"], "alpha");
}

#[test]
fn cli_ingests_html_files_as_first_class_artifacts() {
    let output = run_stdin(
        &["ingest", "file", "-", "--filename", "template.htm"],
        r#"<section hx-get="/partial">Load</section>"#,
    );
    assert_eq!(output["kind"], "file_ingest");
    assert_eq!(output["payload"]["detection"]["content_kind"], "html");
    let artifact = &output["payload"]["artifact"];
    assert_eq!(artifact["kind"], "html");
    assert_eq!(
        artifact["payload"]["htmx_attributes"][0]["normalized_name"],
        "hx-get"
    );
}

#[test]
fn cli_parses_python_with_schema_validation_end_to_end() {
    let output = run_stdin(
        &["parse", "python", "-", "--detail", "semantic-with-syntax"],
        "import os\nclass Form:\n    @classmethod\n    def build(cls):\n        value = cls()\n        return value\n",
    );
    validate_with_schema(&output, "python-code-envelope");
    assert_eq!(output["kind"], "python_code");
    assert!(
        output["payload"]["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .any(|symbol| symbol["qualified_name"] == "Form.build")
    );
    assert_eq!(output["payload"]["assignments"][0]["lhs"], "value");
    assert_eq!(output["payload"]["returns"][0]["expression"], "value");
    assert_eq!(output["payload"]["calls"][0]["target"], "cls");
}

#[test]
fn cli_parses_typescript_with_schema_validation_end_to_end() {
    let output = run_stdin(
        &[
            "parse",
            "typescript",
            "-",
            "--detail",
            "semantic-with-syntax",
        ],
        "import { useMemo } from \"react\";\nexport class Form {\n  build() {\n    const value = useMemo(() => 1, []);\n    return value;\n  }\n}\n",
    );
    validate_with_schema(&output, "typescript-code-envelope");
    assert_eq!(output["kind"], "typescript_code");
    assert!(
        output["payload"]["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .any(|symbol| symbol["qualified_name"] == "Form.build")
    );
    assert_eq!(output["payload"]["imports"][0]["module"], "react");
    assert_eq!(output["payload"]["assignments"][0]["lhs"], "value");
    assert_eq!(output["payload"]["returns"][0]["expression"], "value");
    assert_eq!(output["payload"]["calls"][0]["target"], "useMemo");
}

#[test]
fn cli_ingests_repo_with_typescript_detection() {
    let repo = temp_dir("typescript-repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("src/form.test.ts"),
        "export function testBuild() { return 1; }\n",
    )
    .unwrap();
    let output = run(&["ingest", "repo", repo.to_str().unwrap()]);
    assert!(
        output["payload"]["detected_languages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|language| language == "typescript")
    );
    assert!(
        output["payload"]["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|artifact| artifact["kind"] == "typescript_code")
    );
    assert!(
        output["payload"]["test_hints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hint| hint["path"] == "src/form.test.ts")
    );
}

#[test]
fn cli_parses_jsx_dialect_with_schema_validation_end_to_end() {
    let output = run_stdin(
        &["parse", "typescript", "-", "--dialect", "jsx"],
        "export function View() { return <section data-id=\"ok\">Hello</section>; }\n",
    );
    validate_with_schema(&output, "typescript-code-envelope");
    assert_eq!(output["payload"]["dialect"], "jsx");
    assert!(
        output["payload"]["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .any(|symbol| symbol["name"] == "View" && symbol["language"] == "jsx")
    );
}

#[test]
fn cli_ingests_repo_with_jsx_detection() {
    let repo = temp_dir("jsx-repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("src/view.jsx"),
        "export function View() { return <div>Hello</div>; }\n",
    )
    .unwrap();
    let output = run(&["ingest", "repo", repo.to_str().unwrap()]);
    assert!(
        output["payload"]["detected_languages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|language| language == "jsx")
    );
    assert!(
        output["payload"]["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|artifact| artifact["kind"] == "typescript_code")
    );
}

#[test]
fn cli_promotes_typescript_test_calls_to_repo_test_hints() {
    let repo = temp_dir("typescript-test-calls");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(
        repo.join("src/form.spec.ts"),
        "import { test } from \"vitest\";\ntest(\"builds form\", () => {});\n",
    )
    .unwrap();
    let output = run(&["ingest", "repo", repo.to_str().unwrap()]);
    assert!(
        output["payload"]["test_hints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hint| hint["kind"] == "test_call" && hint["name"] == "builds form")
    );
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
