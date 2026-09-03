# Run 43: HTML5 and XHTML parsing

## Outcome

Implemented the production `grist/html/v2` parser for HTML5 documents, HTML5
fragments, and the repository's inert XHTML profile. The parser combines a
standards-based html5ever recovery tree with an exact source-token layer so DOM
order and browser recovery do not erase original spelling, attributes, ranges,
unknown elements, declarations, comments, or malformed input.

## Delivered behavior

- Added byte-oriented HTML/XHTML entry points using the shared decoder, including
  BOM, HTML `meta`, and XML declaration encoding provenance and raw-byte maps.
- Added explicit document/fragment mode and HTML/XHTML syntax selection, namespace
  and DOM-path provenance, synthetic-node marking, quirks/doctype state, and stable
  malformed-recovery/XHTML well-formedness diagnostics.
- Preserved ordered DOM nodes, exact lexical tokens, source/raw ranges, attributes,
  metadata, links, tables, media, semantic sections, HTMX attributes, and raw
  unknown nodes.
- Classified scripts, forms, event handlers, frames, active URLs, media, and
  external XHTML identifiers as inert retained content. The parser performs no
  network access and executes no active content.
- Integrated HTML v2 through registry aliases/media types, detection, byte/file
  ingestion, budgets, document graph, segmentation, normalized rendering, schema
  catalog/codegen, and the CLI.
- Added deterministic HTML, malformed, XHTML, active-content fixtures and the
  eleven-gate promotion plus focused universal-contract tests.
- Added HTML support/security documentation and updated CLI, README, and normative
  conformance records.

## Primary files

- `src/html.rs`
- `src/registry/builtins.rs`
- `src/ingest/file.rs`
- `src/document_graph/mod.rs`
- `src/schema/catalog.rs`
- `src/cli/operations.rs`
- `src/core/mod.rs`
- `src/main.rs`
- `tests/html_promotion.rs`
- `tests/html_universal_contract.rs`
- `fixtures/generated/html/*`
- `fixtures/corpus.v1.json`
- `fixtures/recipes.v1.json`
- `schemas/grist.html.v2.schema.json`
- `schemas/grist.html-envelope.v2.schema.json`
- `schemas/grist.html-options.v1.schema.json`
- `docs/html.md`

## Validation

- PASS: focused HTML/XHTML unit, universal-contract, and deterministic eleven-gate
  promotion suites.
- PASS: complete CLI-enabled workspace test matrix.
- PASS: scoped warnings-denied Clippy.
- PASS: schema generation/check, full workspace docs, formatting, whitespace, and
  a real CLI parse of the maximum-complexity fixture.
- EXPECTED EXISTING FAILURE: standalone `python scripts/fixture_corpus.py validate`
  reports only `generated/text/plain-universal.txt` (missing recipe/unregistered)
  and `generated/reconstruction/minimal.gristpkg` (unregistered). All HTML fixture
  registrations, hashes, and recipes pass Rust fixture governance and the full
  workspace suite.

The temporary run-scoped offline Cargo vendor shim was removed after validation;
`Cargo.lock` resolves `encoding_rs` to the crates.io registry and contains no
temporary paths.
