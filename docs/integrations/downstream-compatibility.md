# Downstream Compatibility

This document records the operational compatibility surfaces Grist exposes for the three immediate downstream consumers.

## Bathysphere

Bathysphere needs repo-orientation facts without importing Bathysphere policy concepts into Grist.

`grist ingest repo <root>` now emits the neutral fields Bathysphere needs directly in `RepoIngestReport`:

- `files[]` with path, kind, language/content kind, size, and raw SHA-256 hash;
- `detected_languages[]`;
- `manifest_paths[]`;
- `lockfile_paths[]`;
- `test_hints[]` for test files and Rust test functions;
- `unsupported[]`, `skipped[]`, and envelope diagnostics;
- path-stamped flattened diagnostics when file parser diagnostics are promoted to repo diagnostics.

Grist still does not emit Bathysphere operation contracts, ledgers, autonomy states, or drift decisions.

## Graph Composer

Graph Composer has two immediate Grist-facing needs.

### Model-output compatibility

`grist::compatibility::parse_strict_normalized_model_output` provides the legacy narrow `NormalizedModelOutput` shape:

- `ok`
- `grammar`
- `command_name`
- `argument_name`
- `value`
- `normalizations`
- `warnings`
- `error`

Unlike Grist's richer native candidate parser, this compatibility function fails closed unless explicit accepted command and argument allowlists are supplied.

`grist::compatibility::normalize_work_graph_value` applies the known work-graph aliases/defaults used by graph-composer-style work graph proposals, including:

- `graph`, `work_graph`, `workGraph` unwrapping;
- `nodes`/`tasks -> leaves`;
- `id`/`node_id -> leaf_id`;
- `source -> from`, `target -> to`, `type`/`label -> kind`;
- `outputs`/`output_refs`/`output_artifact -> expected_outputs`;
- default leaf `kind`, `title`, `instructions`, `task_ref`, `inputs`, and `expected_outputs` where safe.

### Rust code facts compatibility

`grist::compatibility::rust_code_facts` projects Grist's tree-sitter Rust parser into a graph-composer-compatible `CodeFacts` shape with:

- `path`
- `language`
- `symbols[]`
- `imports[]`
- `tests[]`
- `generated`
- `vendor`
- `metadata.input_truncated`
- `metadata.parse_diagnostics`

This lets downstream coding tools consume Grist without depending on tree-sitter-native details.

## Research Chain

Research Chain needs document/evidence ingestion and robust model-output normalization.

Grist supports the immediate local-first formats needed by the first integration path:

- Markdown AST-like ingestion;
- plain text block ingestion;
- JSON, JSONL, YAML, TOML parsing;
- model-output parsing with strict compatibility mode, candidate mode, schema validation, aliases, repairs, and streaming events;
- repo/file provenance with SHA-256 hashes and source ranges.

`grist::compatibility::parse_research_chain_model_output` intentionally maps to the same strict normalized shape as the graph-composer compatibility function so research-chain can replace its narrow parser without inheriting Grist's richer ambiguity semantics by accident.

Notebook/PDF/DOCX/HTML ingestion remain outside the initial Grist parser set and should be added as concrete parser modules when those downstream ingestion paths are activated. Plain text, Markdown, source, and JSON-like evidence paths are operational now and exercised by `dev ci`.

## Production validation

The checked-in `dev ci` pipeline exercises the compatibility path end-to-end:

- all-target clippy with the CLI feature enabled;
- no-default-features type check;
- unit tests for compatibility adapters and parser primitives;
- CLI e2e tests that validate emitted JSON against generated envelope schemas;
- repo ingestion e2e with filters, externalized artifacts, detected languages, manifests, lockfiles, and test hints;
- model-output e2e with alias rules, schema validation, JSON-ish repair, and think-block stripping;
- plain-text document ingestion e2e.
