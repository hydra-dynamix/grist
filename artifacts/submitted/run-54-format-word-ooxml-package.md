# Run 54 — Word OOXML package parsing

Work item: `format-word-ooxml-package`

## Implemented behavior

- Added exact DOCX, DOCM, DOTX, and DOTM detection and registry dispatch from bounded `[Content_Types].xml` inspection, including extensionless compressed packages and selector/kind mismatch failures.
- Added bounded, read-only ZIP traversal with shared member, expansion, memory, child-artifact, node, input, output, and time budgets. Unsafe paths, links/special entries, cross-platform duplicate paths, inconsistent decompression lengths, unsupported packages, and encrypted members have explicit outcomes.
- Preserved every content-type default/override and package part with package order, compression facts, sizes, CRC-32, content identity, availability/rejection status, and exact `OoxmlPart` locators.
- Parsed root and per-part relationships with raw targets, source parts, exact XML locators, internal/external classification, percent decoding, root-bounded resolution, dangling/orphan/duplicate diagnostics, and no network access. Unknown target modes are conservatively retained as external.
- Parsed core, extended, and typed custom properties with namespaces, attributes, values, IDs, format IDs, link targets, exact XML paths, and malformed/duplicate diagnostics.
- Rejected DTD, entity declaration, XInclude, and remote-schema XML before metadata parsing.
- Inventoried VBA projects without execution. Requested macro bytes remain quarantined, including unexpected macro parts in nominally macro-free packages.
- Inventoried embedded packages, OLE objects, and media as identity-bearing child artifacts; requested inline capture remains subject to shared artifact safety policy.
- Added deterministic DocumentGraph projection, relationship edges, metadata nodes, stable identities, segment/render integration, public schemas, capability discovery, canonical/compatibility module paths, CLI routing, and documentation.

WordprocessingML body semantics are intentionally outside this package-layer work item and remain owned by the dependent Word content item.

## Changed implementation and contract files

- `src/word_ooxml/{mod,model,archive,xml_util,content_types,relationships,properties,artifacts}.rs`
- `src/formats/word_ooxml/mod.rs`, `src/formats/mod.rs`, and `src/lib.rs`
- `src/detect/{mod,packages}.rs`, `src/registry/builtins.rs`, and `src/capabilities.rs`
- `src/core/mod.rs`, `src/document_graph/mod.rs`, `src/cli/operations.rs`, and `src/schema/catalog.rs`
- `Cargo.toml`
- `tests/word_ooxml_package.rs`, `tests/detect_ranked_format.rs`, and `tests/module_feature_topology.rs`
- `docs/word-ooxml.md`, `README.md`, `docs/cli.md`, `docs/module-feature-topology.md`, and `docs/complete-parser-contract.md`
- generated Word OOXML payload/options/envelope schemas, shared enum-bearing schemas, schema catalog, and canonical examples under `schemas/` and `examples/`

## Validation evidence

- Isolated `word-ooxml,document-graph,schemas` check: pass.
- Focused no-default and CLI-feature package tests: pass (four and five tests respectively).
- Capability manifest and ranked-detection regression targets: pass.
- Module topology and compatibility-path target: pass.
- Complete `cargo test --offline --features cli` matrix and doctests: pass.
- Generated schema/canonical-example drift check: pass.
- Clippy across all CLI-feature targets with warnings denied and only the known unrelated graph-helper `too_many_arguments` lint allowed: pass.
- Minimal no-default-feature check: pass with only the existing unrelated `summary::profile_name` warning.
- Documentation build: pass with only three existing unrelated document-graph invalid-HTML-tag warnings.
- Formatting and whitespace checks: pass; Git reports existing Windows line-ending notices only.

LDGR also records the diagnostic development failures: an initial patch-transport syntax break, an initially unhandled compressed content-types manifest, capability feature ordering, the legacy unsupported-DOCX assertion, and local Clippy cleanups. Each was corrected and its focused regression gate passed before the final broad matrix.
