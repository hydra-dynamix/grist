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
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("grist-cli-surface-{name}-{nonce}"));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn run_json(args: &[&str]) -> serde_json::Value {
    let output = Command::new(grist())
        .args(args)
        .output()
        .expect("run grist");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

fn run_stdin(args: &[&str], input: &[u8]) -> serde_json::Value {
    let mut child = Command::new(grist())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn grist");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

fn run_json_lines(args: &[&str]) -> Vec<serde_json::Value> {
    let output = Command::new(grist())
        .args(args)
        .output()
        .expect("run grist");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 output")
        .lines()
        .map(|line| serde_json::from_str(line).expect("NDJSON event"))
        .collect()
}

fn validate_with_schema(value: &serde_json::Value, schema_name: &str) {
    let schema = run_json(&["schema", "emit", schema_name]);
    let validator = jsonschema::validator_for(&schema).expect("schema validator");
    let errors: Vec<_> = validator.iter_errors(value).collect();
    assert!(
        errors.is_empty(),
        "schema errors for {schema_name}: {errors:?}"
    );
}

fn write_graph_fixture(dir: &Path) -> PathBuf {
    let source = dir.join("source.md");
    let graph = dir.join("graph.json");
    fs::write(&source, "# Heading\n\nParagraph text.\n").expect("source");
    let output = Command::new(grist())
        .args([
            "transform",
            source.to_str().expect("source path"),
            "--to",
            "graph",
            "--output",
            graph.to_str().expect("graph path"),
        ])
        .output()
        .expect("transform");
    assert!(output.status.success());
    graph
}

#[test]
fn detection_and_auto_parse_honor_stdin_hints() {
    let detected = run_stdin(
        &[
            "detect",
            "-",
            "--filename",
            "README.md",
            "--mime",
            "text/markdown",
            "--request-id",
            "detect-1",
        ],
        b"# Heading\n",
    );
    assert_eq!(detected["request_id"], "detect-1");
    assert_eq!(detected["detection"]["status"], "selected");
    assert_eq!(detected["detection"]["content_kind"], "markdown");
    assert!(
        !detected["detection"]["candidates"]
            .as_array()
            .expect("candidates")
            .is_empty()
    );
    validate_with_schema(&detected, "cli-detection-report");

    let parsed = run_stdin(
        &["parse", "auto", "-", "--filename", "README.md"],
        b"# Heading\n",
    );
    assert_eq!(parsed["operation"], "parse");
    assert_eq!(parsed["kind"], "markdown");
    assert_eq!(parsed["status"], "complete");

    let jsx = run_stdin(
        &["parse", "jsx", "-"],
        b"export function View() { return <div>Hello</div>; }\n",
    );
    assert_eq!(jsx["kind"], "javascript_code");
    assert_eq!(jsx["payload"]["dialect"], "jsx");
}

#[test]
fn batch_collection_preserves_order_and_request_ids() {
    let dir = temp_dir("batch");
    let first = dir.join("first.md");
    let second = dir.join("second.rs");
    fs::write(&first, "# First\n").expect("first");
    fs::write(&second, "fn second() {}\n").expect("second");
    let result = run_json(&[
        "ingest",
        "batch",
        first.to_str().expect("first path"),
        second.to_str().expect("second path"),
        "--request-id",
        "alpha",
        "--request-id",
        "beta",
        "--collect",
    ]);
    assert_eq!(result["status"], "complete");
    assert_eq!(result["items"][0]["sequence"], 0);
    assert_eq!(result["items"][0]["request_id"], "alpha");
    assert_eq!(result["items"][1]["sequence"], 1);
    assert_eq!(result["items"][1]["request_id"], "beta");

    let events = run_json_lines(&[
        "ingest",
        "batch",
        first.to_str().expect("first path"),
        second.to_str().expect("second path"),
        "--request-id",
        "stream-alpha",
        "--request-id",
        "stream-beta",
    ]);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["event"], "item");
    assert_eq!(events[0]["item"]["sequence"], 0);
    assert_eq!(events[0]["item"]["request_id"], "stream-alpha");
    assert_eq!(events[1]["event"], "item");
    assert_eq!(events[1]["item"]["sequence"], 1);
    assert_eq!(events[1]["item"]["request_id"], "stream-beta");
    assert_eq!(events[2]["event"], "terminal");
    assert_eq!(events[2]["terminal"]["emitted_items"], 2);
}

#[test]
fn normalized_render_and_text_manifest_are_explicit() {
    let dir = temp_dir("render");
    let graph = write_graph_fixture(&dir);
    let rendered = run_json(&["render", graph.to_str().expect("graph"), "--to", "markdown"]);
    assert_eq!(rendered["format"], "markdown");
    assert!(
        rendered["content"]
            .as_str()
            .expect("content")
            .contains("Heading")
    );
    assert_eq!(
        rendered["source_map"]["generated_length"],
        rendered["content"].as_str().expect("content").len()
    );
    assert_eq!(
        rendered["fidelity"]["reconstruction_claim"],
        "normalized_not_byte_round_trip"
    );

    let text = dir.join("rendered.txt");
    let manifest = run_json(&[
        "render",
        graph.to_str().expect("graph"),
        "--to",
        "text",
        "--text-output",
        text.to_str().expect("text"),
        "--request-id",
        "render-1",
    ]);
    assert_eq!(manifest["request_id"], "render-1");
    assert_eq!(manifest["destination"]["kind"], "path");
    assert_eq!(
        manifest["byte_length"].as_u64().expect("byte length") as usize,
        fs::read(&text).expect("rendered text").len()
    );
    validate_with_schema(&manifest, "cli-text-output-manifest");
}

#[test]
fn segment_validate_archive_and_capabilities_are_structured() {
    let dir = temp_dir("remaining");
    let graph = write_graph_fixture(&dir);
    let config = dir.join("segments.json");
    fs::write(
        &config,
        serde_json::to_vec_pretty(&grist::segment::SegmentOptions {
            target_size: 8,
            maximum_size: 64,
            ..Default::default()
        })
        .expect("options"),
    )
    .expect("config");

    let segmented = run_json(&[
        "segment",
        graph.to_str().expect("graph"),
        "--graph",
        "--config",
        config.to_str().expect("config"),
    ]);
    assert_eq!(segmented["operation"], "segment");
    assert_eq!(segmented["kind"], "segment_collection");
    assert!(
        !segmented["payload"]["segments"]
            .as_array()
            .expect("segments")
            .is_empty()
    );

    let validation = run_json(&[
        "validate",
        graph.to_str().expect("graph"),
        "--schema",
        "graph-transform-envelope",
    ]);
    assert_eq!(validation["operation"], "validate");
    assert_eq!(validation["payload"]["valid"], true);

    let archive = dir.join("empty.zip");
    fs::write(&archive, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").expect("archive");
    let traversal = run_json(&[
        "ingest",
        "archive",
        archive.to_str().expect("archive"),
        "--request-id",
        "archive-1",
    ]);
    assert_eq!(traversal["request_id"], "archive-1");
    assert_eq!(traversal["container_format"], "zip");
    assert_eq!(traversal["status"], "unsupported");

    let capabilities = run_json(&["capabilities"]);
    assert_eq!(
        capabilities["schema_version"],
        grist::capabilities::CapabilityManifest::SCHEMA_VERSION
    );
    assert!(
        capabilities["operations"]
            .as_array()
            .expect("operations")
            .iter()
            .any(|operation| operation == "segment")
    );
    assert!(
        capabilities["registry"]["parsers"]
            .as_array()
            .expect("parsers")
            .iter()
            .any(|parser| parser["format"]["id"] == "markdown")
    );
    assert_eq!(
        capabilities,
        serde_json::to_value(grist::capabilities::discover().unwrap()).unwrap()
    );
    assert!(
        capabilities["unsupported_capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|capability| capability["id"] != "format.pdf")
    );
    validate_with_schema(&capabilities, "capability-manifest");
}
