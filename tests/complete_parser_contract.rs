use std::collections::BTreeSet;

const CONTRACT: &str = include_str!("../docs/complete-parser-contract.md");

#[test]
fn release_audit_covers_every_spec_section_without_retained_gaps() {
    let mut sections = BTreeSet::new();
    for line in CONTRACT.lines().filter(|line| line.starts_with("| §")) {
        let cells = line
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        assert_eq!(cells.len(), 3, "malformed audit row: {line}");
        let section = cells[0]
            .trim_start_matches('§')
            .split_whitespace()
            .next()
            .expect("section number")
            .parse::<u8>()
            .expect("numeric section");
        sections.insert(section);
        assert!(
            matches!(cells[1], "verified" | "verified with exclusions"),
            "unresolved disposition: {line}"
        );
    }
    assert_eq!(sections, (1_u8..=19).collect());
    for stale in ["**partial**", "**absent**", "remains queued"] {
        assert!(!CONTRACT.contains(stale), "stale gap marker: {stale}");
    }
}

#[test]
fn exclusions_are_exact_and_retained_contract_is_not_weakened() {
    for excluded in [
        "Legacy DOC",
        "WordProcessingML 2003",
        "Flat OPC",
        "Legacy PPT",
        "Legacy XLS",
        "XLSB",
        "SpreadsheetML 2003",
        "PST",
        "OST",
    ] {
        assert!(CONTRACT.contains(excluded), "missing exclusion: {excluded}");
    }
    assert!(CONTRACT.contains("not permission to weaken any other requirement"));
    assert!(CONTRACT.contains("92 selectors"));
    assert!(CONTRACT.contains("`model_output` and `ldgr_projection`"));
}

#[test]
fn all_universal_gates_and_release_deliverables_are_explicit() {
    for gate in 1..=11 {
        assert!(
            CONTRACT.contains(&format!("| {gate}.")),
            "gate {gate} missing"
        );
    }
    for token in [
        "fuzz/fuzz_targets/universal_bytes.rs",
        "benches/representative_parsers.rs",
        "tests/loader_replacement_e2e.rs",
        "cargo test --locked --all-features",
        "--example schema_codegen -- --check",
        "graph-transform-envelope",
    ] {
        assert!(
            CONTRACT.contains(token),
            "release evidence missing: {token}"
        );
    }
}
