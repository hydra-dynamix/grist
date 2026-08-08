# Grist Specification

Grist is a Rust library and CLI for interpretation tasks shared across Hydra Dynamix projects. It provides reliable document/code ingestion and model-output normalization without owning downstream policy, orchestration, trust, validation, or canonical-state decisions.

This spec is grounded in the immediate integration requirements from:

- `docs/requirements/bathy-requirements.md`
- `docs/requirements/graph-composer-requirements.md`
- `docs/requirements/research-chain-requirements.md`

## Goals

- Provide reusable parsing and ingestion primitives for Markdown documents, HTML/HTMX documents and fragments, CSV datasets, Rust, Python, TypeScript/TSX/JSX code, common serializations, and model outputs.
- Expose stable, typed Rust models that serialize to versioned, verifiable JSON.
- Provide a JSON-only CLI that maps closely to the public library API for agents and end-to-end testing.
- Preserve provenance, source ranges, hashes, diagnostics, and parser metadata where useful.
- Be modular enough to add future parsers such as TSV, notebooks, PDFs, or DOCX without redesigning the core interface.

## Non-goals

Grist does not:

- execute code, notebooks, scripts, or model-requested tools;
- decide scientific truth or evidence acceptance;
- approve canonical mutations;
- own Bathysphere operation contracts, autonomy states, drift tripwires, or provenance ledgers;
- own graph-composer or research-chain policy decisions;
- silently hide malformed input behind best-effort success.

Grist interprets inputs. Consumers decide what those interpretations mean.

## Crate shape

Initial crate layout may remain a single crate with modules, but public module boundaries should be designed as if they could become separate crates later.

Expected modules:

- `grist::core` — shared source, range, diagnostic, envelope, option, hashing, and schema types.
- `grist::detect` — file kind, content type, language, and parser selection.
- `grist::ingest` — file and repo ingestion.
- `grist::markdown` — Markdown AST-like document parsing.
- `grist::html` — HTML document/fragment parsing with structured element, attribute, and HTMX attribute facts.
- `grist::rust` — tree-sitter-backed Rust parsing.
- `grist::python` — tree-sitter-backed Python parsing.
- `grist::typescript` — tree-sitter-backed TypeScript, TSX, and JSX parsing.
- `grist::serialization` — JSON, JSONL, YAML, and TOML parsing.
- `grist::structured_binary` — CBOR, MessagePack, and descriptor-driven Protocol Buffers.
- `grist::csv` — CSV row/cell parsing with headers, typed scalar inference, source metadata, and diagnostics.
- `grist::email` — inert RFC 5322/MIME parsing, attachments, and threading evidence.
- `grist::mbox` — streaming MBOX variants, separator provenance, and aggregate threads.
- `grist::outlook` — bounded, inert Outlook MSG compound-file and MAPI parsing.
- `grist::calendar_contact` - inert iCalendar events/time zones and vCard contacts.
- `grist::model_output` — model-output candidate extraction, normalization, repair, and streaming parsing.
- `grist::schema` — generated JSON Schema emission and validation helpers.
- `grist::cli` / binary `grist` — thin JSON-only CLI over the library API.

## Cargo features

Major functionality should be feature-gated while default features enable the initial core set.

Suggested features:

- `markdown`
- `html`
- `rust`
- `python`
- `typescript`
- `serialization`
- `structured-binary`
- `columnar`
- `sqlite`
- `email-message`
- `csv`
- `model-output`
- `schemas`
- `cli`
- `default = ["markdown", "html", "csv", "rust", "python", "typescript", "serialization", "model-output", "schemas"]`

The CLI binary should be enabled by the package binary target and may require the `cli` feature internally.

## Public output contract

### Typed Rust first

All public outputs are strongly typed Rust structs/enums with `serde` support. These types are the primary library contract.

### Versioned JSON

All public CLI/library JSON outputs include explicit schema versions, for example:

- `grist/envelope/v1`
- `grist/markdown/v1`
- `grist/html/v1`
- `grist/rust-code/v1`
- `grist/python-code/v1`
- `grist/typescript-code/v1`
- `grist/structured-text/v2`
- `grist/structured-binary/v1`
- `grist/columnar/v1`
- `grist/sqlite/v1`
- `grist/email/v1`
- `grist/mbox/v1`
- `grist/outlook-msg/v1`
- `grist/icalendar/v1`
- `grist/vcard/v1`
- `grist/csv/v1`
- `grist/model-output/v1`
- `grist/repo-ingest/v1`

### Generated JSON Schema

Grist should generate JSON Schema files from Rust public types, check them into the repo, and test for schema drift.

Suggested layout:

```text
schemas/grist.envelope.v1.schema.json
schemas/grist.markdown.v1.schema.json
schemas/grist.html.v1.schema.json
schemas/grist.rust-code.v1.schema.json
schemas/grist.python-code.v1.schema.json
schemas/grist.typescript-code.v1.schema.json
schemas/grist.serialization.v1.schema.json
schemas/grist.model-output.v1.schema.json
schemas/grist.repo-ingest.v1.schema.json
```

Use generated schemas for external verification of CLI output and cross-project integration.

## Shared envelope

Grist should not force every parser into one monolithic artifact schema. Instead, use a shared envelope with typed payloads.

Conceptual shape:

```json
{
  "schema_version": "grist/envelope/v1",
  "kind": "markdown|html|csv|rust_code|python_code|typescript_code|serialization|model_output|repo_ingest",
  "source": {},
  "hashes": {},
  "parser": {},
  "diagnostics": [],
  "payload_schema_version": "grist/markdown/v1",
  "payload": {}
}
```

The shared envelope handles source identity, diagnostics, hashes, parser metadata, and schema versioning. Each payload owns its domain-specific schema.

## Source model and ranges

Supported text parsers treat input as UTF-8. Decode failures produce diagnostics.

Source ranges include both:

- UTF-8 byte offsets for exact slicing, hashing, graph anchoring, and diff tools;
- 1-based line/column positions for humans and editors.

Ranges use half-open intervals: `start` inclusive, `end` exclusive.

Conceptual shape:

```json
{
  "byte_start": 10,
  "byte_end": 42,
  "start_line": 2,
  "start_column": 1,
  "end_line": 3,
  "end_column": 8
}
```

## Diagnostics

Use one shared diagnostic model across parsers, with parser-specific details attached as optional structured metadata.

Diagnostics must be root-cause-oriented and actionable.

Suggested fields:

- `severity`: `info | warning | error`
- `code`: stable diagnostic code
- `message`: human-readable root-cause message
- `parser`: parser/module that emitted it
- `source`: optional source/path context
- `range`: optional source range
- `partial`: whether output is partial because of this issue
- `cause`: optional nested cause/context chain
- `details`: optional parser-specific structured metadata

Avoid vague messages like `parse failed` without the underlying reason.

## Options and limits

Avoid one global options monstrosity. Each module has its own options struct while reusing shared option components.

Examples:

- `MarkdownOptions`
- `RustIngestOptions`
- `SerializationOptions`
- `ModelOutputOptions`
- `RepoIngestOptions`

Shared components:

- `Limits`
- `SourceOptions`
- `DiagnosticOptions`
- `SchemaOptions`

Resource limits should be centralized where possible, generous by default, and overrideable by callers. Limit failures must produce structured diagnostics that identify the specific limit hit.

Initial limits should not obstruct ordinary development or long LLM outputs.

## Parser registry and detection

Grist uses an internal parser registry from day one. Parsers implement a common trait/interface and are registered by enabled feature/module.

Dynamic third-party plugins are not required initially.

Detection should use:

- file extension;
- special filenames/manifests such as `Cargo.toml` and `README.md`;
- shebangs where useful;
- lightweight content sniffing for JSON/YAML/TOML/Markdown-like text.

Detection must not execute code or perform risky deep inspection. Detection should include confidence/reasoning metadata where practical.

## File and repo ingestion

### API layers

Grist exposes two ingestion layers:

1. Raw content APIs for already-loaded text/bytes.
2. Filesystem/repo APIs for safe traversal and path-based ingestion.

### Repo traversal

Repo ingestion accepts:

- explicit root path;
- include/exclude patterns using gitignore-style glob semantics;
- default ignore behavior honoring repository ignore rules;
- override to include ignored files when needed;
- max file size and max file count limits;
- allowed/denied path patterns;
- inline vs externalized artifact output mode.

Default traversal should honor repo ignore rules because ignored build outputs, caches, secrets, and generated files are usually not part of the meaningful source corpus. Callers can explicitly override this for graph/diff workflows that require fuller capture.

### Hashing and provenance

Every ingested text file gets stable identity/provenance metadata:

- relative path;
- size;
- raw byte SHA-256;
- decoded text hash when applicable;
- detected kind/language/content type;
- parser used;
- source ranges in parsed payloads.

Grist does not store raw bytes in normal JSON output by default.

### Binary files

Initial Grist does not attempt useful binary ingestion or reconstruction. Binary files should be skipped or reported in summaries/diagnostics, not converted into full artifact records.

### File kind classification

Classify conservatively:

- `source`
- `test`
- `manifest`
- `lockfile`
- `generated`
- `binary`
- `documentation`
- `unknown`

### Repo-level output

Repo ingestion returns aggregated reports as a first-class output.

Reports include:

- root/path metadata;
- traversal options summary;
- file inventory;
- inline parsed artifacts by default;
- skipped/ignored/unsupported summaries;
- diagnostics;
- hashes for ingested text files.

For large repositories, support externalized artifact mode where the repo report contains a manifest/index and parsed artifacts are written separately.

## Markdown parsing

Markdown parsing should produce an AST-like document model suitable for machine-readable instructions, skills, program execution instructions, model prompts, and human documentation.

Initial support includes:

- document root;
- headings with levels, hierarchy, and ranges;
- paragraphs/text blocks;
- fenced code blocks with info string/language and ranges;
- links;
- tables normalized into rows/cells and optionally CSV-compatible representation;
- YAML frontmatter as parsed metadata plus raw source/range;
- diagnostics for malformed fences, malformed frontmatter, unsupported table forms, and decode/source issues.

Every AST node should carry source location when available.

## Rust parsing

Rust ingestion is backed by real parsing from day one using `tree-sitter` and `tree-sitter-rust`. Regex or heuristic-only symbol extraction is not acceptable for the Rust parser.

### Public model

Grist exposes stable, Grist-owned Rust semantic models by default rather than raw tree-sitter structures. Parser-native/syntax detail remains available when requested through command/API-specific detail options.

Suggested detail modes for Rust-specific APIs:

- semantic default: Grist-owned stable semantic model;
- semantic plus selected syntax metadata;
- syntax/debug detail for investigating parser behavior and abstraction gaps.

Do not create one global detail system for all modules.

### Required extraction

The initial Rust parser should handle modern Rust codebases comprehensively, including:

- modules;
- structs, enums, unions;
- traits and trait items;
- impl blocks and methods;
- free functions;
- consts and statics;
- type aliases;
- imports/use trees including groups and aliases;
- visibility forms: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`, private, unknown;
- attributes;
- doc comments attached to symbols;
- macros and macro invocations where tree-sitter can represent them usefully;
- test/bench-like functions via attributes such as `#[test]` and `#[tokio::test]`;
- parse errors and partial parse diagnostics with ranges.

Missing modern Rust constructs are parser/spec gaps to fix when encountered, not accepted permanent limitations.

### Symbol model

Conceptual symbol shape:

```json
{
  "id": "stable-or-derived-symbol-id",
  "name": "parse_input",
  "kind": "function",
  "language": "rust",
  "path": "src/parser.rs",
  "range": {},
  "visibility": "public|crate|restricted|private|unknown",
  "parent": null,
  "attributes": [],
  "doc": null,
  "syntax": null
}
```

Visibility is a fact for consumers. It is not a public API policy decision.

## TypeScript, TSX, and JSX parsing

TypeScript-family ingestion is backed by real parsing from day one using `tree-sitter` and `tree-sitter-typescript`. Regex or heuristic-only symbol extraction is not acceptable for the TypeScript parser.

For Grist, an operational baseline for any first-class language service means functional parity with the existing language services:

- parser-backed source analysis, not simulated symbol graphs;
- deterministic Grist-owned public payload types;
- source ranges and hashes;
- parse errors represented as partial diagnostics;
- schema generation and schema drift tests;
- CLI parse support;
- file/repo ingest routing;
- repo-level language detection and test hints where the language has conventional test forms;
- syntax detail modes for parser investigation.

### Public model

Grist exposes one TypeScript-family payload for TypeScript, TSX, and JSX with an explicit dialect field. Parser-native/syntax detail remains optional and selected through TypeScript-specific detail options.

### Required extraction

The TypeScript-family parser should handle common real-world TypeScript, TSX, and JSX codebases, including:

- classes and methods;
- constructors where represented by tree-sitter;
- interfaces and interface methods;
- type aliases;
- enums;
- namespaces/modules where represented by tree-sitter;
- free functions;
- function-valued variables such as arrow functions and function expressions;
- top-level variables;
- class fields and property signatures;
- imports including default, namespace, named, side-effect, and type imports;
- exports and re-exports;
- visibility/modifier facts such as `public`, `protected`, `private`, `static`, `readonly`, `abstract`, `declare`, `export`, and `default`;
- decorators attached to symbols;
- JSDoc-style comments attached to symbols where practical;
- calls, constructor calls, returns, assignments, and branch/control-flow nodes;
- test-like calls such as `test(...)`, `it(...)`, and `describe(...)` for repo test hints;
- parse errors and partial parse diagnostics with ranges.

Missing modern TypeScript/TSX/JSX constructs are parser/spec gaps to fix when encountered, not accepted permanent limitations.

## Serialization parsing

Initial common serialization support includes:

- JSON;
- JSONL;
- YAML;
- TOML.

CSV is a first-class parser and ingestion artifact. It preserves headers, rows, cells, raw cell text, inferred scalar JSON values, source metadata where available, and parser diagnostics. TSV remains deferred until a concrete use case appears, though the CSV parser is delimiter-configurable enough to support tab-delimited input through explicit options.

Parsing should be modular so additional serialization parsers can be dropped in later.

Grist should support default normalization/validation behavior based on common developer conventions and allow custom caller schemas/rules for unusual cases.

Use JSON Schema as the initial external validation format for JSON-like outputs. Repair/alias normalization happens before schema validation. Schema validation returns structured diagnostics, not only pass/fail.

## Model-output parsing

Model-output parsing is a first-class module.

Initial grammars/candidate types include:

- raw JSON outputs;
- fenced JSON/code blocks;
- prose-wrapped or malformed JSON-like outputs where safe repair is possible;
- OpenAI-style tool/function calls;
- MCP-style JSON-RPC commands;
- Python-style command calls such as `Namespace.Command(arg={...})` when explicitly enabled;
- YAML/TOML/XML-ish extracted tool blocks where useful;
- `<think>...</think>` stripping as a normalization where configured.

JSON-oriented parsing is the primary/default model-output path. Python-style command calling remains a supported legacy/compatibility option for callers that explicitly enable it, preferably with accepted command and argument names.

### Candidates

When multiple parse candidates are present, Grist returns all detected candidates with diagnostics and normalization metadata. It may mark a selected/best candidate only when unambiguous, but should not discard alternatives.

Candidate fields should include:

- candidate id;
- grammar/type;
- command/tool/function name where applicable;
- argument name where applicable;
- parsed value where applicable;
- raw source range;
- confidence/status;
- normalizations applied;
- warnings/errors;
- schema validation result if requested.

### Repair and aliases

Schema repair and alias rules are runtime-configurable. Rules should be constructible by Rust callers and loadable by the CLI from JSON/TOML config.

Required repair/normalization capabilities include:

- unwrapping markdown fences;
- extracting balanced JSON objects;
- parsing stringified nested JSON arguments;
- common JSON-ish repairs such as single quotes, unquoted keys, trailing commas, Python booleans/nulls, and mislabeled formats;
- field aliases such as `nodes -> leaves` or `source -> from` for project-specific schemas;
- command/tool/function aliases;
- argument aliases.

Parsing success does not imply semantic validity or trust. Consumers still validate schemas and apply policy.

### Streaming

Streaming model-output parsing is required from day one.

The streaming parser provides both:

- incremental events for live agent/control-loop use;
- final accumulated reports for batch parsing and CLI/e2e validation.

Events may include:

- `candidate_started`;
- `candidate_updated`;
- `candidate_completed`;
- `diagnostic`;
- `parser_state_changed`.

Streaming must handle partial structures:

- incomplete JSON;
- incomplete fenced blocks;
- incomplete Python-style calls;
- incomplete OpenAI/MCP JSON-RPC objects.

Finalization distinguishes:

- incomplete;
- complete valid candidate;
- recovered/malformed candidate;
- unrecoverable parse failure.

The streaming event model should remain compatible with the full-response candidate/diagnostic model.

## CLI

The `grist` CLI is a first-class control surface for agents and end-to-end testing. It should map closely to public library APIs.

All CLI output is JSON. Human-readable or pretty rendering is deferred.

Initial commands:

```text
grist parse markdown <path|->
grist parse rust <path|->
grist parse json <path|->
grist parse model-output <path|->
grist ingest file <path|->
grist ingest repo <path>
grist schema list
grist schema emit <name>
```

Stdin support:

```text
grist parse markdown -
grist parse model-output -
grist ingest file - --filename README.md
grist ingest file - --kind markdown
```

Stdin without enough detection context should return a structured diagnostic rather than guessing wildly.

Repo/file commands should expose relevant include/exclude, ignore override, limit, schema, and artifact-output options in a consistent way.

## Dependency plan

Initial intended dependencies:

- `serde`, `serde_json` for typed JSON serialization;
- `schemars` for JSON Schema generation;
- `sha2` for hashing;
- `ignore` for repo traversal and gitignore-style matching;
- `tree-sitter`, `tree-sitter-rust` for Rust parsing;
- Markdown parser with strong source-position/table/frontmatter support, to be selected during implementation;
- `serde_yaml` for YAML;
- `toml` for TOML;
- JSON Schema validation crate, to be selected during implementation;
- `clap` for CLI;
- `thiserror` or equivalent for structured internal errors.

Dependency choices can be revised if investigation finds a better fit, especially for Markdown source ranges, tables, and frontmatter.

## Testing requirements

Grist should include:

- unit tests for parser primitives;
- golden tests for stable normalized output;
- schema drift tests for generated JSON Schemas;
- CLI end-to-end tests that validate JSON output;
- fixture matrix covering valid, malformed, adversarial, and oversized inputs;
- Rust fixtures for functions, traits, impls, macros, modules, visibility, tests, attributes, doc comments, syntax errors, and modern language constructs;
- Markdown fixtures for headings, nested sections, fences, malformed fences, links, tables, and frontmatter;
- serialization fixtures for JSON, JSONL, YAML, TOML, malformed inputs, and schema validation;
- CSV fixtures for header/no-header inputs, typed scalar inference, file ingest, and schema validation;
- model-output fixtures for Python-style calls, OpenAI calls, MCP JSON-RPC, fenced JSON, prose wrappers, multiple candidates, malformed JSON, nested stringified arguments, and streaming partials;
- fuzz/property tests for model-output parsing and JSON-ish repair where practical.

Snapshot/golden tests should focus on parser outputs and schemas, not huge end-to-end artifacts.

## Acceptance criteria for first useful integration

Grist is ready for initial integration when:

- CLI and library APIs produce matching JSON for the same operations;
- JSON outputs are typed, versioned, and schema-verifiable;
- repo ingestion produces inventory, hashes, parsed artifacts, skipped/unsupported summaries, and diagnostics;
- Markdown parser emits AST-like nodes with source ranges;
- Rust parser uses tree-sitter and emits comprehensive symbols/imports/diagnostics with ranges;
- serialization parser supports JSON, JSONL, YAML, and TOML;
- CSV parser supports header-aware rows/cells with raw values, inferred scalar values, hashes, ranges where available, and diagnostics;
- model-output parser supports batch and streaming parsing with candidates, repairs, aliases, diagnostics, and raw provenance;
- malformed/unsupported inputs produce explicit diagnostics rather than silent success;
- no network access or code execution occurs by default;
- Bathysphere, graph-composer, and research-chain can consume Grist outputs without reaching into Grist internals.
