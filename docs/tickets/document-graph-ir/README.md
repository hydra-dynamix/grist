# DocumentGraph IR Ticket Plan

Source specification: conversational architecture decision in this run: introduce a central DocumentGraph IR for parser-to-parser transforms, future LaTeX support, code document transitions, and conditional-obligation semantics.

## Summary

This decomposition schedules a production implementation of a normalized DocumentGraph projection while preserving existing parser-specific payloads. It covers the core IR, schema/CLI exposure, conversion traits, Markdown/code projections, basin refactor, Markdown/LaTeX rendering, LaTeX parsing/projection/rendering, conditional-obligation modeling/extraction, transform CLI integration, golden validation, and operator documentation.

## Non-negotiable constraints

- Existing parser-specific outputs remain authoritative and source-compatible unless explicitly migrated.
- DocumentGraph is a normalized projection, not a claim of lossless universal AST coverage.
- Cross-format transforms must expose unsupported-node and lossy-transform diagnostics instead of silently dropping content.
- Conditional obligations must be modeled as semantic nodes/relations with provenance and attrs, not as bare edges.
- LaTeX support must preserve unknown commands/environments as raw nodes and must not require external TeX executables.

## Ticket count by epoch

- Epoch 7 — DocumentGraph foundation: 3 tickets
- Epoch 8 — Existing parser projections: 4 tickets
- Epoch 9 — LaTeX support: 3 tickets
- Epoch 10 — Semantic obligations: 2 tickets
- Epoch 11 — Integration and operator surface: 3 tickets
- Total: 15 tickets

## Coverage map

- Central IR: 0701, 0702, 0703
- Markdown transforms: 0704, 0707, 0713, 0714
- Code document transitions: 0705, 0706, 0714
- Basin integration: 0706
- LaTeX parser and transforms: 0708, 0709, 0710, 0713, 0714
- Conditional obligations: 0711, 0712, 0714, 0715
- CLI/API/operator surface: 0702, 0713, 0715
- Validation and regressions: all tickets include local validation; 0714 provides the integration matrix
- Documentation/migration: 0715

## Dependency/orchestration notes

The first executable slice is 0701, followed by schema exposure and transform traits. Markdown/code projections can then proceed in parallel. LaTeX and semantic-obligation extraction depend on the shared graph model. CLI integration and golden validation close the architecture.

## Validation strategy

Use targeted cargo tests for each module during implementation. Promotion gates should run `cargo test --features "cli basin"`; after LaTeX lands, run `cargo test --features "cli latex basin"`.

## Explicit exclusions

- PDF generation from LaTeX.
- LLM/NLP-driven obligation extraction in the initial semantic pass.
- Byte-identical Markdown/LaTeX round trips.
- Removing or replacing existing parser-specific payloads.

## Known ambiguities

- Exact module name can be `document_graph` or `document_ir`; tickets use `DocumentGraph` as the public concept.
- Whether DocumentGraph is default-enabled or feature-gated should be decided in 0701 based on compatibility impact.
