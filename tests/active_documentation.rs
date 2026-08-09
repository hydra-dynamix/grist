const README: &str = include_str!("../README.md");
const CLI: &str = include_str!("../docs/cli.md");
const CONTRACT: &str = include_str!("../docs/complete-parser-contract.md");
const SPEC: &str = include_str!("../docs/spec.md");

#[test]
fn readme_describes_actual_feature_defaults_and_transform_outputs() {
    for feature in [
        "text-publishing",
        "structured-data",
        "code",
        "model-output",
        "document-graph",
        "schemas",
    ] {
        assert!(README.contains(&format!("`{feature}`")));
    }
    assert!(README.contains("LDGR projection is not\nenabled by default"));
    assert!(README.contains("Markdown, LaTeX, HTML, and\nplain-text transform targets"));
    assert!(!README.contains("current full parser set"));
}

#[test]
fn cli_reference_tracks_registry_routing_and_graph_envelopes() {
    for token in [
        "grist parse FORMAT INPUT",
        "grist parse FORMAT --help",
        "grist capabilities",
        "graph-transform-envelope",
        "--to markdown|latex|html|text",
    ] {
        assert!(CLI.contains(token), "CLI reference is missing `{token}`");
    }
    assert!(!CLI.contains("Supported source extensions:"));
    assert!(!CLI.contains("`--to graph` emits JSON `DocumentGraph`"));
}

#[test]
fn retained_contract_and_spec_do_not_reintroduce_known_stale_claims() {
    assert!(CONTRACT.contains("grist segment <graph envelope> --graph --config <segment options>"));
    for stale in [
        "future parsers such as TSV, notebooks, PDFs, or DOCX",
        "does not attempt useful binary ingestion or reconstruction",
        "TSV remains deferred",
        "All CLI output is JSON. Human-readable or pretty rendering is deferred.",
    ] {
        assert!(!SPEC.contains(stale), "stale spec claim returned: {stale}");
    }
    assert!(SPEC.contains("CSV and TSV are first-class delimited-data selectors"));
    assert!(SPEC.contains("Transform can explicitly emit normalized Markdown, LaTeX, HTML"));
}
