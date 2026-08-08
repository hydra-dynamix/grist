# Grist

Grist is a Rust library and optional JSON-only CLI for interpretation tasks shared across Hydra Dynamix projects. It parses and normalizes documents, code, repository files, serializations, CSV data, and model outputs while leaving downstream policy, orchestration, trust, validation, and canonical-state decisions to consumers.

## What Grist provides

- Typed Rust output models with `serde` support.
- Versioned JSON envelopes for CLI and cross-project integrations.
- Checked-in JSON Schemas for public output contracts.
- Parsers for Markdown, LaTeX, BibTeX/BibLaTeX, HTML fragments/documents, XML/JATS, CSV, EML/RFC 5322 and MIME, MBOX mailboxes, Outlook MSG, iCalendar, vCard, Rust, Python, TypeScript/TSX/JSX, JSON/JSONL/YAML/TOML, model outputs, plain text, and LDGR Markdown Projection documents.
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

See [`docs/cli.md`](docs/cli.md) for the full command menu and examples.

```sh
grist parse markdown README.md
grist parse html fragment.html --mode fragment
grist parse xml article.nxml --dialect jats
grist parse csv data.csv
grist parse rust src/lib.rs
grist parse python script.py
grist parse typescript app.ts --dialect typescript
grist parse latex paper.tex
grist parse bibtex references.bib
grist parse json config.toml --format toml
grist parse model-output response.txt --json-value
grist parse ldgr-projection ticket.md --strict
grist render json-summary data.json
grist render json-summary data.json --profile dynamic-event-dataset
grist validate json dataset.json --schema schemas/dynamic-event-explorer.dataset.v1.schema.json
grist transform README.md --to graph
grist transform README.md --to latex
grist transform paper.tex --to markdown
grist transform requirements.md --to graph --extract-obligations
grist ingest file README.md
grist ingest repo . --exclude target/**
grist schema list
grist schema emit markdown-envelope
```

HTML5 document/fragment and XHTML parsing uses standards-based tree recovery,
retains an exact lexical stream and declared encodings, and inventories scripts,
forms, remote references, and event handlers without executing or fetching
them. See [docs/html.md](docs/html.md). XML/JATS parsing preserves namespaces, XML paths, metadata, links, tables, media, and unknown elements while rejecting external resolution; see [docs/xml-jats.md](docs/xml-jats.md).

All successful parse, ingest, schema, render, and validate CLI commands print JSON. Transform renderers may emit Markdown or LaTeX text when requested. CLI errors are emitted as structured diagnostics.

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

The `bibliography` feature enables lossless BibTeX/BibLaTeX parsing, bounded string/crossref resolution, citation lookup, and graph projection. It is included in `scholarly`, `full`, and `cli`.

The `pdf` feature enables bounded inert PDF object/xref, catalog, page-tree, label, metadata, filter, encryption, repair, native glyph/text, font/style, geometry, and reading-order parsing. It is included in `full` and `cli`.

The `word-ooxml` feature enables bounded inert DOCX, DOCM, DOTX, and DOTM package parsing, including content types, relationships, properties, quarantined macro inventory, and embedded child artifacts. It is included in `word-processing`, `full`, and `cli`.

The `presentation-ooxml` feature enables bounded inert PPTX, PPTM, POTX, and PPSX package parsing, including slide order, masters/layouts/themes, properties, action inventory, macro quarantine, and embedded child artifacts. It is included in `presentations`, `full`, and `cli`.

The `odf-word` feature enables bounded inert ODT and OTT package parsing, including metadata, styles, document structure, tracked revisions and named views, rich objects, and quarantined embedded artifacts. It is included in `word-processing`, `full`, and `cli`.

The `rtf` feature enables byte-precise, inert Rich Text Format parsing with nested-group recovery, destinations, formatting and style tables, Unicode and ANSI code pages, fields, lists, tables, revisions/comments, and quarantined picture/object payloads. It is included in `word-processing`, `full`, and `cli`.

The `cli` feature enables the `grist` binary and pulls in the parser features needed by the command surface.

## Schemas and specs

- `docs/spec.md` describes the public interpretation contract.
- `docs/cli.md` describes the CLI command menu, parse/ingest/schema/render/validate/transform commands, and examples.
- `docs/ldgr-projection-module-spec.md` describes the LDGR Markdown Projection parser/renderer contract.
- `docs/markdown.md` describes the CommonMark/GFM v2 payload, extensions, diagnostics, projections, and security behavior.
- `docs/restructured-text.md` describes the source-preserving reStructuredText payload, inert syntax handling, and root-bounded include policy.
- docs/asciidoc.md describes the typed inert AsciiDoc payload, raw syntax retention, and explicit root-bounded includes.
- docs/latex.md describes safe project-root detection, bounded includes and macro expansion, rich scholarly syntax, and inert active commands.
- `docs/bibliography.md` describes BibTeX/BibLaTeX provenance, bounded value and crossref resolution, duplicate-key citation semantics, and graph projection.
- docs/pdf.md describes the bounded inert PDF container plus native font, glyph, token, line, block, geometry, reading-order, diagnostic, and locator contract.
- docs/word-ooxml.md describes safe Word OPC traversal, package metadata, relationships, macro quarantine, child artifacts, and exact locators.
- docs/presentation-ooxml.md describes safe PresentationML OPC traversal, slide order, masters/layouts/themes, inert actions, macro quarantine, child artifacts, and exact locators.
- docs/presentation-odf.md describes inert ODP/OTP package parsing, masters/styles, complete slide content, confidence-bearing reading order, embedded artifacts, security, graph, segment, schema, and CLI behavior.
- `docs/odf-word.md` describes ODT/OTT package traversal, semantic and structural views, revisions, rich objects, child artifacts, and exact locators.
- `docs/rtf.md` describes RTF group recovery, decoding, retained controls, semantic projections, embedded artifacts, and security behavior.
- `docs/document-graph.md` describes the normalized graph IR, transforms, LaTeX support, basin integration, and semantic obligation policy.
- `schemas/` contains checked-in JSON Schema files for public envelopes and payloads.
- `fixtures/` contains the licensed/synthetic corpus registry, deterministic builders,
  provider recordings, downstream regression intake policy, and canonical golden rules.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
cargo test --all-features
```

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
