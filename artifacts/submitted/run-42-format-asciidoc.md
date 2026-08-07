# Run 42 — Implement AsciiDoc parsing

## Result

Implemented the native `grist/asciidoc/v1` parser and its envelope, format registry/detection integration, graph projection, segmentation/schema/CLI surfaces, deterministic fixture corpus, documentation, and promotion coverage.

The parser preserves source order, raw text, byte/line/column ranges, stable node identities, diagnostics, and decoding provenance. It recognizes headings, document attributes and unsets, block attributes and roles, inert block macros/directives, delimited source/literal/raw blocks, pipe tables, anchors, cross-references, links, footnotes, attribute references, inline roles, emphasis/strong/code, and raw unknown syntax.

Includes remain bounded references by default. Opt-in local resolution requires an explicit canonical project root and enforces root containment, recursion depth, byte budget, cycle detection, and missing/unsafe-target diagnostics. Remote includes are never fetched and parsed content is never executed.

## Changed files

- Parser and public format surface: `src/asciidoc.rs`, `src/formats/asciidoc/mod.rs`, `src/formats/mod.rs`, `src/lib.rs`, `Cargo.toml`.
- Shared contracts: `src/core/mod.rs`, `src/registry/builtins.rs`, `src/detect/mod.rs`, `src/detect/text.rs`, `src/document_graph/mod.rs`, `src/cli/operations.rs`, `src/capabilities.rs`, `src/schema/catalog.rs`.
- Tests: `tests/asciidoc_universal_contract.rs`, `tests/asciidoc_promotion.rs`, plus contract/topology/CLI integration updates.
- Schemas/examples: `schemas/grist.asciidoc.v1.schema.json`, `schemas/grist.asciidoc-envelope.v2.schema.json`, `schemas/grist.asciidoc-options.v1.schema.json`, and regenerated registered schema/canonical-example outputs.
- Fixtures: three registered deterministic files under `fixtures/generated/asciidoc/`, with recipe and corpus manifest entries.
- Documentation: `docs/asciidoc.md`, README, CLI, module-topology, and complete-parser contract updates.

## Validation

Passed:

- Focused parser unit tests and the AsciiDoc universal contract.
- Deterministic eleven-gate `asciidoc_promotion` suite, including graph, segmentation, schemas, CLI, budgets, hostile input, provenance, unknown syntax, and root-bounded includes.
- Actual CLI end-to-end test and module/complete-parser/fixture-governance contracts.
- `cargo run --offline --example schema_codegen --features cli --config .tmp/cargo-config.toml -- --check` after regeneration.
- `cargo test --offline --workspace --features cli --config .tmp/cargo-config.toml`.
- `cargo clippy --offline --no-default-features --features asciidoc,document-graph,schemas --lib --tests -- -D warnings -A dead-code`.
- `cargo doc --offline --workspace --features cli --no-deps --config .tmp/cargo-config.toml` (only three pre-existing invalid-HTML-tag warnings in `DocumentGraph`).
- `cargo fmt --all -- --check` and `git diff --check` (only existing Windows line-ending notices).

The standalone `python scripts/fixture_corpus.py validate` remains non-zero solely for pre-existing `generated/reconstruction/minimal.gristpkg` and `generated/text/plain-universal.txt`; the latter also lacks a deterministic recipe. Rust fixture governance and all new AsciiDoc registrations pass.

A temporary workspace-local extraction of the already-cached `encoding_rs` crate was used only to run the full offline CLI matrix and was removed afterward. Pre-existing `.tmp/run22-vendor` and `.tmp/tmpmbi2wzbe` were preserved.