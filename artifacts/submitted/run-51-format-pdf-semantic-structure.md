# Run 51: PDF semantic structure

Implemented confidence-bearing PDF semantic inference on top of the native layout layer.

## Production behavior

- Infers deterministic columns and reading order, headings, paragraphs, list items, lists, captions, repeated headers, repeated footers, and page numbers. Every inferred relationship carries evidence, confidence, and PDF locators.
- Validates and inventories `/StructTreeRoot`; malformed, missing, or cyclic tagged structure remains explicit rather than being treated as trustworthy structure.
- Infers tables from aligned native text and ruling geometry, including row and cell boxes, header candidates, row/column spans, captions, and reading relationships.
- Inventories inert vector paths, raster image XObjects, form XObjects, and inline images; groups figure candidates with nearby captions and preserves geometry and pixel dimensions where available.
- Bounds semantic graphics processing with `PdfOptions::max_semantic_graphics` and emits explicit partial diagnostics for budget exhaustion, malformed tagged structure, and unsupported graphics operations.
- Projects lists, tables, rows, cells, figures, images, captions, containment, caption, and precedence relationships into the shared document graph and segmentation surfaces with stable native-block identities.

## Changed files

- `src/pdf/semantic.rs` (new semantic inference implementation)
- `src/pdf/model.rs`, `src/pdf/parse.rs`, `src/pdf/layout.rs`, `src/pdf/mod.rs`
- `src/document_graph/mod.rs`
- `tests/pdf_semantic_structure.rs` (focused two-page semantic fixture)
- `docs/pdf.md`
- `schemas/grist.pdf.v1.schema.json`, `schemas/grist.pdf-envelope.v2.schema.json`, `schemas/grist.pdf-options.v1.schema.json`
- `examples/schema-canonical-examples.v1.json`

## Validation

- Minimal PDF and complete CLI feature graphs compile.
- Focused semantic, native-layout, universal-contract, and schema-drift tests pass.
- Supported CLI library Clippy is warning-clean with the repository's existing argument-count and dead-code allowances.
- The full `cargo test --offline --features cli` suite passes.
- Rust formatting and whitespace checks pass; Git reports only existing Windows line-ending conversion notices.

The PDF-only all-test-target Clippy attempt cannot compile two existing integration tests because they directly use the optional `jsonschema` crate without enabling the `schemas` feature. The supported complete CLI lint and test paths pass; no semantic acceptance gap remains.
