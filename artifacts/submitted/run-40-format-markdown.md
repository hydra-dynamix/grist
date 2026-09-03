# Run 40 — format-markdown

## Result

Implemented the `grist/markdown/v2` universal parser contract for CommonMark, GFM, and retained extended Markdown syntax. The parser preserves raw bytes, decode fidelity, exact decoded/raw ranges, locators, hierarchy, frontmatter, standard block and inline constructs, task lists, tables, footnotes, HTML, directives, extension syntax, and malformed/unknown raw nodes.

## Production changes

- Added a byte-authoritative Markdown v2 payload and typed options with deterministic decoding, source identities, diagnostics, budgets, and partial-result behavior.
- Added YAML/TOML frontmatter, GFM syntax, tables, footnotes, task items, strikethrough, definition lists, math, heading attributes, MyST/colon directives, wiki links, shortcodes, templates, raw HTML, and unknown syntax retention.
- Added deterministic DocumentGraph, segmentation, normalized-render, schema, CLI, and ingest projections with explicit link/reference edges and raw fallbacks.
- Added generated v2 schemas/canonical examples, governed adversarial/malformed/security/regression fixtures, contract documentation, focused universal tests, and the shared 11-gate promotion report.
- Made all registry input adapters use one raw-byte decoding path so equivalent inputs produce identical canonical payloads.

## Validation

Passed:

- `cargo test --offline --workspace --features cli`
- `cargo test --offline --features cli --test markdown_promotion`
- `cargo test --offline --features cli --test markdown_universal_contract`
- `cargo test --offline --no-default-features --features markdown,schemas --test markdown_universal_contract`
- `cargo check --offline --no-default-features --features markdown`
- `cargo check --offline --features cli`
- `cargo run --offline --example schema_codegen --features cli -- --check`
- `python scripts/fixture_corpus.py build --check` (14 deterministic fixtures)
- `cargo clippy --offline --workspace --all-targets --features cli -- -D warnings -A clippy::too_many_arguments -A clippy::unnecessary_lazy_evaluations`
- `cargo doc --offline --workspace --features cli --no-deps`
- `cargo fmt --all -- --check`
- `git diff --check`

Expected retained repository evidence:

- Strict Clippy without allowances reports the same three unrelated legacy findings: two `too_many_arguments` findings in DocumentGraph/LaTeX and one LaTeX `unnecessary_lazy_evaluations` finding. Allowing exactly those classes passes the complete matrix; Markdown introduces no lint finding.
- Full fixture validation reports pre-existing unregistered `generated/reconstruction/minimal.gristpkg` and `generated/text/plain-universal.txt` artifacts from earlier runs. Recipe reproducibility for every registered fixture passes.
- Rustdoc emits the same three pre-existing invalid-HTML-tag warnings in DocumentGraph and completes successfully.

## Regressions found and fixed

The first broad test exposed duplicate rendered obligation text in Markdown graph projection; leaf Markdown text now projects as spans under semantic block nodes. A second broad test exposed adapter-dependent canonical payloads; all registry adapters now share the byte-authoritative decoder. The final full workspace matrix passes.
