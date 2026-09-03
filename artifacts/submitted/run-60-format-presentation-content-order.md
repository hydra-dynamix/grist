# Run 60: OOXML presentation content and reading order

Implemented the complete `format-presentation-content-order` work item on top of the bounded OOXML package parser from run 59.

## Production behavior

- Added typed, serde/schema-compatible per-slide content for nested shapes, exact EMU geometry and transforms, rich text paragraphs/runs/fields/breaks, notes, comments/authors, tables, charts, Office Math equations, images, alt text, links, transitions, animations, and embedded/content-part objects.
- Retained source identities, relationship IDs and targets, content types, raw XML metadata, one-based slide locators, parent/z-order information, and related-part paths. External relationships remain inert and are never fetched; actions, animation, media, and embedded objects are never executed.
- Added deterministic per-slide reading-order inference. Title placeholders sort first, positioned shapes then sort top-to-bottom/left-to-right, and source z-order is the fallback/tiebreaker. Every entry and the aggregate carry bounded confidence plus explicit evidence.
- Projected semantic content into stable document-graph nodes and inferred `Precedes` edges with confidence/evidence locators. Rendering and segmentation consume the same deterministic graph.
- Extended generated payload/envelope schemas and CLI JSON coverage without removing the run-59 package contracts.

## Changed files

- `src/presentation_ooxml/content.rs`
- `src/presentation_ooxml/model.rs`
- `src/presentation_ooxml/mod.rs`
- `src/presentation_ooxml/graph.rs`
- `tests/presentation_ooxml_package.rs`
- `docs/presentation-ooxml.md`
- `schemas/grist.presentation-ooxml.v1.schema.json`
- `schemas/grist.presentation-ooxml-envelope.v2.schema.json`

## Validation

- PASS: `cargo test --offline --features "presentation-ooxml,document-graph,schemas" --test presentation_ooxml_package` (4 focused tests).
- PASS: `cargo test --all-features --offline --test presentation_ooxml_package` (5 tests, including CLI JSON).
- PASS: `cargo check --all-features --offline`.
- PASS: `cargo clippy --offline --features presentation-ooxml --all-targets -- -D warnings`.
- PASS: `cargo run --all-features --offline --example schema_codegen -- --check`.
- PASS: `cargo fmt --all -- --check` and scoped trailing-whitespace scan.
- FAIL (unrelated, preserved): `cargo clippy --all-features --offline --all-targets -- -D warnings` reaches pre-existing `clippy::int_plus_one` and `clippy::filter_map_bool_then` findings in `src/basin.rs`.

The all-feature commands used a temporary local unpack of the cached `encoding_rs 0.8.35` crate because the global Cargo registry is read-only. The temporary Cargo patch was removed, the lockfile was restored to the registry dependency, and the rebuildable target cache is cleaned after evidence capture.
