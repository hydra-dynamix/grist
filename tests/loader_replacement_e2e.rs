#![cfg(feature = "cli")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn grist() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_grist"))
}

fn temp_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("grist-loader-replacement-{nonce}"));
    fs::create_dir_all(&path).expect("temp directory");
    path
}

fn run_json(args: &[&str]) -> serde_json::Value {
    let output = Command::new(grist())
        .args(args)
        .output()
        .expect("run grist");
    assert!(
        output.status.success(),
        "grist {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents.as_bytes()).expect("fixture write");
}

#[test]
fn one_loader_boundary_projects_validates_and_segments_multiple_format_families() {
    let dir = temp_dir();
    let fixtures = [
        (
            "document.md",
            "# Loader replacement\n\nTraceable paragraph.\n",
        ),
        ("records.csv", "name,value\nalpha,1\nbeta,2\n"),
        (
            "message.eml",
            "From: sender@example.test\r\nTo: reader@example.test\r\nSubject: Loader replacement\r\nMessage-ID: <loader@example.test>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nTraceable message body.\r\n",
        ),
        ("module.py", "def loader(value):\n    return value\n"),
    ];
    let config = dir.join("segments.json");
    fs::write(
        &config,
        serde_json::to_vec_pretty(&grist::segment::SegmentOptions {
            target_size: 32,
            maximum_size: 256,
            ..Default::default()
        })
        .expect("segment options"),
    )
    .expect("segment config");

    for (name, contents) in fixtures {
        let input = dir.join(name);
        write(&input, contents);
        let input = input.to_str().expect("input path");
        let graph = run_json(&["transform", input, "--to", "graph"]);
        assert_eq!(graph["operation"], "transform", "{name}");
        assert_eq!(graph["kind"], "graph_transform_result", "{name}");
        assert!(
            graph["payload"]["graph"]["nodes"]
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty()),
            "{name} graph"
        );

        let graph_path = dir.join(format!("{name}.graph.json"));
        fs::write(
            &graph_path,
            serde_json::to_vec_pretty(&graph).expect("graph JSON"),
        )
        .expect("graph file");
        let graph_path = graph_path.to_str().expect("graph path");
        let validation = run_json(&[
            "validate",
            graph_path,
            "--schema",
            "graph-transform-envelope",
        ]);
        assert_eq!(validation["payload"]["valid"], true, "{name}");

        let segments = run_json(&[
            "segment",
            graph_path,
            "--graph",
            "--config",
            config.to_str().expect("config path"),
        ]);
        let segments = segments["payload"]["segments"]
            .as_array()
            .expect("segment array");
        assert!(!segments.is_empty(), "{name} segments");
        assert!(
            segments.iter().all(|segment| segment["locators"]
                .as_array()
                .is_some_and(|locators| !locators.is_empty())),
            "{name} traceable segment"
        );
    }
}
