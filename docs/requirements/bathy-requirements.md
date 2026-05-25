# Grist Requirements

Grist is the proposed reusable interpretation utility library that Bathysphere depends on for code ingestion, source parsing, and eventually structured LLM response parsing.

Grist should be independently useful outside Bathysphere. It must not depend on Bathysphere concepts such as operation contracts, autonomy states, drift events, or provenance ledgers.

## Purpose

Grist provides reliable interpretation primitives for systems that need to ingest, parse, and reason over code or structured model output.

For Bathysphere, Grist must support:

- repository file inventory,
- language detection,
- Rust source parsing,
- symbol extraction,
- symbol source ranges,
- parse diagnostics,
- unsupported-file reporting,
- stable structured output suitable for adaptation into Bathysphere orientation artifacts.

Later, Grist may also provide:

- reusable LLM response parsing,
- structured block extraction,
- patch proposal parsing,
- code fence parsing,
- model-output diagnostics.

## Non-Goals

Grist should not own:

- Bathysphere operation contracts,
- mutation policies,
- autonomy states,
- trust bands,
- drift tripwires,
- validation execution,
- provenance ledger storage,
- rollback metadata,
- agent orchestration.

Grist interprets inputs. Bathysphere decides what those interpretations mean operationally.

## Core Design Principles

### Reusable Interpretation Boundary

Grist should expose general code and text interpretation models that multiple systems can consume.

Bathysphere-specific meaning should live in Bathysphere adapters.

### Real Parsing Only

If Grist claims symbol knowledge, it must come from real parsing or deterministic analysis.

No simulated symbol graphs, heuristic-only fake ASTs, or unverified semantic claims.

### Honest Unsupported States

Unsupported languages, binary files, parse failures, and ambiguous model output must be represented explicitly.

Grist should prefer:

```text
unsupported / partial / parse_error
```

over pretending success.

### Stable Source Locations

All code interpretation output must use stable source locations:

- relative path,
- byte range where available,
- line and column range,
- UTF-8 assumptions clearly documented.

### No Operational Policy

Grist may report facts like:

```text
symbol is public
file is a manifest
parse failed
code fence contains Rust
```

It must not decide:

```text
operation should be quarantined
mutation violates scope
agent autonomy should decrease
```

Those decisions belong to Bathysphere or other consuming systems.

## First Required Capability: Code Ingestion

### Repository Input

Grist must accept:

- explicit repository root,
- optional ignore rules,
- optional max file size,
- optional allowed path patterns,
- optional denied path patterns.

### File Inventory Output

Grist should emit:

```json
{
  "root": "/workspace/project",
  "files": [
    {
      "path": "src/lib.rs",
      "kind": "source",
      "language": "rust",
      "size_bytes": 1024,
      "content_hash": "sha256:..."
    }
  ],
  "ignored": [],
  "unsupported": []
}
```

### File Kind Classification

At minimum, classify:

- source,
- test,
- manifest,
- lockfile,
- generated,
- binary,
- documentation,
- unknown.

Classification can be conservative.

## First Required Language: Rust

### Parser

Rust parsing should be backed by tree-sitter:

- `tree-sitter`,
- `tree-sitter-rust`,
- a stable wrapper API owned by Grist.

### Required Rust Symbols

Grist must extract:

- functions,
- structs,
- enums,
- traits,
- impl blocks where practical,
- modules where practical,
- constants,
- statics,
- type aliases,
- test functions where practical.

### Rust Symbol Fields

Each symbol should include:

```json
{
  "id": "stable-or-derived-symbol-id",
  "name": "parse_input",
  "kind": "function",
  "language": "rust",
  "path": "src/parser.rs",
  "range": {
    "start_line": 10,
    "start_column": 1,
    "end_line": 42,
    "end_column": 2
  },
  "byte_range": {
    "start": 240,
    "end": 980
  },
  "visibility": "public",
  "parent": null,
  "attributes": []
}
```

### Rust Visibility

Grist should report conservative visibility facts:

- public,
- crate,
- restricted,
- private,
- unknown.

Visibility is a fact for consumers. It is not a public API policy decision.

### Parse Diagnostics

Parse diagnostics should include:

- path,
- severity,
- parser,
- message,
- optional range,
- whether output is partial.

Example:

```json
{
  "path": "src/lib.rs",
  "severity": "error",
  "parser": "tree-sitter-rust",
  "message": "Parse contained ERROR nodes.",
  "range": {
    "start_line": 12,
    "start_column": 5,
    "end_line": 12,
    "end_column": 9
  },
  "partial": true
}
```

## Bathysphere Integration Requirements

Bathysphere needs Grist to provide enough data to build orientation artifacts.

Required Grist output must map cleanly into Bathysphere:

- repository root,
- file inventory,
- detected languages,
- manifest paths,
- lockfile paths,
- test hints,
- symbols,
- unsupported files,
- parse diagnostics.

Bathysphere will adapt Grist output into its own orientation model.

Grist should not emit Bathysphere ledger events or drift events.

## Future Capability: LLM Response Parsing

Grist should eventually absorb the robust LLM response parser from `graph-composer`.

Initial LLM parser requirements:

- extract fenced code blocks,
- preserve declared language,
- preserve raw text ranges,
- detect malformed fences,
- extract structured sections where possible,
- parse model-proposed patches where explicitly supported,
- report ambiguous or partial parses.

Example output:

```json
{
  "blocks": [
    {
      "kind": "code_fence",
      "language": "rust",
      "content": "fn main() {}",
      "range": {
        "start_line": 4,
        "end_line": 6
      }
    }
  ],
  "diagnostics": []
}
```

## API Requirements

Grist should expose library APIs first.

Expected Rust crate shape:

```text
grist-core
grist-code
grist-llm
grist-cli
```

This crate split is provisional. A smaller initial layout is acceptable if module boundaries remain clean.

Required API properties:

- deterministic output,
- serializable data models,
- structured errors,
- no process-global mutable parser state exposed to callers,
- filesystem access behind explicit repository/input parameters,
- test fixtures for parser behavior.

## CLI Requirements

A CLI is useful but should remain thin over library APIs.

Candidate commands:

```text
grist scan --repo .
grist symbols --repo . --language rust
grist parse-llm response.md
```

CLI output should support:

- human-readable summaries,
- `--json` machine-readable output.

## Testing Requirements

Grist should include tests for:

- file inventory,
- ignore handling,
- Rust function extraction,
- Rust struct/enum/trait extraction,
- Rust impl extraction where supported,
- Rust visibility extraction,
- test function detection,
- parse diagnostics,
- unsupported file reporting,
- stable serialization,
- LLM code fence parsing once the LLM parser is extracted.

## Production Quality Bar

Grist is ready for Bathysphere integration when:

- Rust parsing uses tree-sitter or equivalent real parsing,
- symbol ranges are stable and tested,
- unsupported files are explicit,
- parse errors are explicit,
- output serializes deterministically,
- Bathysphere can build an orientation artifact without reaching into Grist internals,
- no Bathysphere operation concepts are required by Grist APIs.

