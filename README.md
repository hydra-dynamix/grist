# Grist

Grist is a Rust library and optional JSON-only CLI for interpretation tasks shared across Hydra Dynamix projects. It parses and normalizes documents, code, repository files, serializations, CSV data, and model outputs while leaving downstream policy, orchestration, trust, validation, and canonical-state decisions to consumers.

## What Grist provides

- Typed Rust output models with `serde` support.
- Versioned JSON envelopes for CLI and cross-project integrations.
- Checked-in JSON Schemas for public output contracts.
- Parsers for Markdown, LaTeX, HTML fragments/documents, CSV, Rust, Python, TypeScript/TSX/JSX, JSON/JSONL/YAML/TOML, model outputs, plain text, and LDGR Markdown Projection documents.
- A normalized `DocumentGraph` projection for cross-format transforms, code graph consumers, and semantic obligation extraction.
- Safe file and repository ingestion that honors ignore rules by default.

Grist does not execute code, approve evidence, mutate downstream ledgers, or decide what parsed information means.

## Install the CLI from GitHub

The binary is behind the `cli` feature:

```sh
cargo install --git https://github.com/hydra-dynamix/grist grist --features cli
```

From a source checkout:

```sh
git clone https://github.com/hydra-dynamix/grist
cd grist
cargo install --path . --features cli
```

The library can be used without the CLI feature.

## CLI quick start

```sh
grist parse markdown README.md
grist parse html fragment.html --mode fragment
grist parse csv data.csv
grist parse rust src/lib.rs
grist parse python script.py
grist parse typescript app.ts --dialect typescript
grist parse latex paper.tex
grist parse json config.toml --format toml
grist parse model-output response.txt --json-value
grist parse ldgr-projection ticket.md --strict
grist transform README.md --to graph
grist transform README.md --to latex
grist transform paper.tex --to markdown
grist transform requirements.md --to graph --extract-obligations
grist ingest file README.md
grist ingest repo . --exclude target/**
grist schema list
grist schema emit markdown-envelope
```

All successful CLI commands print JSON. CLI errors are emitted as structured diagnostics.

## Rust usage

```rust
use grist::core::SourceInfo;

let source = SourceInfo::inline("README.md");
let envelope = grist::markdown::parse_markdown("# Title\n", source);
println!("{}", envelope.schema_version);
```

Enable only the parser features you need, or use default features for the current full parser set.

## Features

Default features enable Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, DocumentGraph, schema, and LDGR projection support used by downstream LDGR tooling. The `latex` feature enables LaTeX parsing and LaTeX graph transforms; it is also included by the `cli` feature.

The `cli` feature enables the `grist` binary and pulls in the parser features needed by the command surface.

## Schemas and specs

- `docs/spec.md` describes the public interpretation contract.
- `docs/ldgr-projection-module-spec.md` describes the LDGR Markdown Projection parser/renderer contract.
- `docs/document-graph.md` describes the normalized graph IR, transforms, LaTeX support, basin integration, and semantic obligation policy.
- `schemas/` contains checked-in JSON Schema files for public envelopes and payloads.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
cargo test --all-features
```

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
