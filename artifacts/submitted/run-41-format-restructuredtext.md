# Run 41 — reStructuredText parser

Work item: `format-restructuredtext`

## Result

Implemented a deterministic, source-preserving `grist/restructured-text/v1`
parser and its envelope, registry, detection, graph, segmentation, schema, CLI,
fixture, documentation, and promotion surfaces.

The parser covers adornment headings, directives and options, interpreted
roles, grid/simple tables, code/literal/doctest blocks, footnotes, citations,
targets, named cross-references, substitutions, lists, fields, transitions,
comments, and inline syntax. Unknown directives and roles remain inert with
their original syntax. Malformed syntax uses explicit raw nodes and partial
diagnostics; no directive or embedded code executes.

Local includes resolve only when the caller supplies a project root. Candidate
and root paths are canonicalized, containment is enforced, recursive resolution
tracks cycles, and depth/byte budgets apply. Outside, missing, remote, invalid,
or rootless includes remain references with stable diagnostics. No network
fetch is attempted.

## Changed surfaces

- Parser/API: `src/restructured_text.rs`, `src/formats/restructured_text/mod.rs`,
  `src/lib.rs`, `src/formats/mod.rs`, and Cargo feature topology.
- Shared contracts: core artifact/schema identity, built-in registry,
  capabilities, ranked detection, DocumentGraph projection, CLI graph dispatch,
  and schema catalog.
- Tests: focused universal contract, shared eleven-gate promotion adapter,
  module topology, and actual CLI end-to-end parsing.
- Data: three deterministic generated reStructuredText fixtures, corpus entries,
  builder recipes, three public schemas, and regenerated canonical schema data.
- Docs: `docs/restructured-text.md`, CLI/module topology, README, and the
  complete-parser conformance ledger.

## Validation

- Focused unit, minimal-feature universal, module topology, and default-feature
  eleven-gate promotion tests pass.
- The complete workspace test suite passes with reStructuredText, LaTeX,
  LDGR projection, and schemas enabled.
- Actual `grist parse restructured-text -` CLI output validates against the
  checked envelope schema.
- Full schema generation is reproducible under `--check`.
- Scoped all-target Clippy passes with `-D warnings` after excluding the known
  unrelated `summary.rs` dead-code class; strict output retains only that
  pre-existing finding.
- Workspace documentation, formatting, and `git diff --check` pass. Rustdoc
  retains three pre-existing invalid-HTML-tag warnings in DocumentGraph.
- The Rust fixture governance suite passes. The standalone Python fixture tool
  still reports the two pre-existing unregistered files
  `generated/reconstruction/minimal.gristpkg` and
  `generated/text/plain-universal.txt`; every run-41 fixture is registered,
  reproducible, hash-verified, and promotion-tested.

No unrelated worktree changes were reverted or overwritten.
