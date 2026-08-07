# Run 52: PDF interactive and embedded content

Implemented a bounded, inert PDF interaction and attachment inventory with stable nested artifact identities.

## Production behavior

- Preserves legacy and name-tree destinations, outline hierarchy, page links, annotations, threaded comments, AcroForm fields and widgets, signature dictionaries, optional-content groups, file specifications, and their typed relationships.
- Records actions as inert data only. URI, named, destination, file, and JavaScript action metadata is inventoried; script contents are represented by a digest and never executed.
- Retains page rectangles, PDF object references, JSON-pointer/object locators, destination views, form inheritance and flags, signature metadata and byte ranges, layer intent/visibility, attachment descriptions and AF relationships.
- Recursively inventories embedded PDF attachments under explicit count, byte, depth, and interactive-object limits. Every child uses the shared `EmbeddedArtifact` contract and names its immediate parent content identity.
- Keeps unsupported, encrypted, malformed, and budget-limited children as typed terminal inventory records instead of silently dropping them. Associated file specifications outside the name tree are included.
- Projects bookmarks, links, annotations, comments, forms, fields, signatures, layers, attachments, widget links, replies, destination resolution, outline hierarchy, and attachment containment into the shared document graph.

## Changed files

- `src/pdf/interactive.rs` (new bounded interactive/embedded extractor)
- `src/pdf/model.rs`, `src/pdf/parse.rs`, `src/pdf/mod.rs`
- `src/document_graph/mod.rs`
- `tests/pdf_interactive_embedded.rs` (interactive, widget, recursive, encrypted, and budget fixtures)
- `docs/pdf.md`
- `schemas/grist.pdf.v1.schema.json`, `schemas/grist.pdf-envelope.v2.schema.json`, `schemas/grist.pdf-options.v1.schema.json`
- `examples/schema-canonical-examples.v1.json`

## Validation

- Focused interactive/embedded tests pass, including nested parent identity, encrypted-child inventory, widget relationships, and budget-limited retention.
- The complete CLI unit, integration, schema-drift, security, and documentation test matrix passes.
- Supported all-target CLI Clippy passes with the repository's existing argument-count and dead-code allowances.
- Minimal PDF/document-graph/schema compilation and CLI documentation generation pass.
- Rust formatting and whitespace checks pass; Git reports only existing Windows line-ending conversion notices.

During implementation, schema/canonical-example drift and one new Clippy finding were observed, repaired, and revalidated. No interactive content is evaluated and no attachment is materialized.
