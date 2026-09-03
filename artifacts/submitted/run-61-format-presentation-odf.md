# Run 61: Implement ODP and OTP parsing

## Outcome

Implemented complete bounded, inert OpenDocument presentation parsing for ODP and OTP packages. The authoritative payload retains package identity and parts, manifest encryption state, metadata/settings, styles, page layouts, masters, slides, geometry, nested shapes, rich text, notes, annotations, tables, embedded charts and images, links, transitions, animations, embedded objects, raw unknown XML, deterministic reading order, exact locators, diagnostics, and operation status.

## Integration

- Added `src/presentation_odf/{archive,graph,model,parse,xml}.rs` and the public/parser entry modules.
- Added ODP/OTP detection, distinct package kinds, registry providers/capabilities, `presentations` feature wiring, CLI transform inference, graph projection, rendering/segmentation, schema catalog entries, and generated public schemas.
- Added contract/module documentation and focused package, detection, graph, CLI, schema, hostile input, encryption, malformed archive, and budget tests.
- External links, animations, media, scripts, and executable embedded artifacts remain inventory-only inert data.

## Validation

- PASS: `cargo check --offline --no-default-features --features presentation-odf`.
- PASS: `cargo check --all-features --offline` using a temporary local unpack of cached `encoding_rs`; no Cargo patch remains.
- PASS: `cargo test --offline --no-default-features --features presentation-odf,document-graph,schemas --test presentation_odf_package` (3 tests).
- PASS: `cargo test --all-features --offline`, including doc tests and all integration suites.
- PASS: `cargo run --all-features --offline --example schema_codegen -- --check` using the same temporary dependency workaround.
- PASS: scoped Clippy with `-D warnings -A dead-code`; the exception is the unrelated pre-existing `src/summary.rs` dead-code warning.
- PASS: `cargo fmt --all -- --check` and scoped trailing-whitespace checks.
- EXPECTED FAIL: repository-wide strict Clippy remains blocked only by the pre-existing `src/basin.rs` `int_plus_one` and `filter_map_bool_then` findings.

During broad validation, an accidental UTF-8 conversion in two previously existing section-marker strings was detected by tests and repaired before the final passing all-feature suite.