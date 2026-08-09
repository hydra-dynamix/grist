# Complete parser release contract

This is the release audit for the retained **Grist Complete Parser
Specification**. It replaces the historical gap matrix that drove the
implementation schedule. The source specification remains the normative
design; this document records its final disposition and executable evidence.

## Scope decision

The retained target is the specification with one user-approved reduction:
legacy binary Office formats and Outlook stores are excluded. Specifically,
Grist does not claim support for Legacy DOC, WordProcessingML 2003, Flat OPC,
Legacy PPT, Legacy XLS, XLSB, SpreadsheetML 2003, PST, or OST. This exclusion
is not permission to weaken any other requirement or retained format.

The all-feature registry exposes 92 selectors. Every available selector has a
real parser, authoritative payload/options schema identities, capability
metadata, detection and CLI routing, and a `DocumentGraph` projection where a
normalized graph is meaningful. `model_output` and `ldgr_projection` are the
two deliberate payload-only formats. The exact selector-to-schema mapping is
maintained in [cross-format-integration.md](cross-format-integration.md).

## Sections 1–19 audit

`verified` means the retained requirement is represented by public code and a
release gate. `verified with exclusions` is used only for the format decision
above.

| Section | Disposition | Principal evidence |
|---:|---|---|
| §1 Purpose | verified | `README.md`, `docs/spec.md`, public crate and CLI boundaries |
| §2 Normative principles | verified | typed envelopes, identity, locators, diagnostics, provenance, budgets, provider and security contracts |
| §3 Public architecture | verified | `Cargo.toml`, `src/lib.rs`, minimal/default/full feature-matrix tests |
| §4 Common public contract | verified | `src/core`, canonical examples, schema compatibility and universal-contract tests |
| §5 Detection and decoding | verified | ranked detection, charset evidence, ambiguity and extensionless/mislabeled fixtures |
| §6 Complete format surface | verified with exclusions | 92-selector registry and the explicit exclusions above |
| §7 Payloads and `DocumentGraph` | verified | typed payloads remain authoritative; retained meaningful projections are registry-tested |
| §8 Embedded artifacts | verified | shared bounded container/artifact traversal, nested identity and locator tests |
| §9 Provider contract | verified | explicit provider selection, recording/replay, separated native/derived identity and provenance |
| §10 Deterministic segmentation | verified | structural graph segmentation, stable IDs, locators and citation anchors |
| §11 Transforms and renderers | verified | graph transform, fidelity/source-map manifests and renderer tests |
| §12 Resources, streaming, cancellation | verified | budget profiles, partial/cancelled states, event streams and cache behavior |
| §13 Security | verified | inert parsing, active-content inventory, root containment, bomb limits and fuzz regressions |
| §14 CLI | verified | detect/parse/ingest/segment/render/transform/validate/schema/capabilities integration tests |
| §15 Schema and compatibility | verified | all-feature schema generation, checked-in drift and canonical-example compatibility gates |
| §16 Test and verification | verified | fixtures, promotion gates, goldens, fuzz corpus, performance smoke and end-to-end loader test |
| §17 Observability and performance | verified | metrics/cache/merge contracts plus `representative_parsers` benchmark |
| §18 Repository deliverables | verified | `src/`, `schemas/`, `fixtures/`, `tests/`, `fuzz/`, `benches/`, `docs/` |
| §19 Definition of done | verified | final locked all-feature suite, schema/CLI gates and clean committed tree |

## Universal parser acceptance gates

Every retained byte-facing parser is promoted only when the common harness
demonstrates all eleven gates:

| Gate | Required evidence |
|---:|---|
| 1. Detection | valid, mislabeled, extensionless, malformed and ambiguous inputs |
| 2. Fidelity | source facts are retained or loss is explicit |
| 3. Locators | payload, graph and segment facts trace to source evidence |
| 4. Diagnostics | malformed/unsupported/loss/provider/security/budget outcomes are typed |
| 5. Identity | raw, decoded, canonical and derived identities are deterministic |
| 6. Graph | every meaningful retained payload projects without replacing authority |
| 7. Segmentation | stable ordering, identity, provenance and citation anchors |
| 8. Security | no execution or implicit network; paths and recursion are bounded |
| 9. Resources | budgets, partial results, cancellation and streaming are explicit |
| 10. Schemas and CLI | generated public contracts and registry routing do not drift |
| 11. Verification | fixtures, goldens/properties, fuzz regressions and performance smoke |

The checked fuzz surface lives at `fuzz/fuzz_targets/universal_bytes.rs`; its
seed corpus also runs as an ordinary integration test so regressions are
covered without requiring libFuzzer in normal CI. The representative benchmark
is `benches/representative_parsers.rs` and the loader-replacement scenario is
`tests/loader_replacement_e2e.rs`.

## Release gates

Release readiness is established serially with one Cargo worker:

```text
cargo fmt --all -- --check
cargo test --locked --all-features -- --test-threads=1
cargo clippy --locked --all-targets --all-features
cargo test --locked --no-default-features --lib
cargo run --locked --all-features --example schema_codegen -- --check
grist capabilities
grist parse auto <retained fixture>
grist transform <retained fixture> --to graph
grist validate <graph envelope> --schema graph-transform-envelope
grist segment <graph envelope> --graph
```

The repository currently carries advisory Clippy style/deprecation lints, but
the project's exact CI Clippy command succeeds. They are not parser correctness
failures and were deliberately not expanded into unrelated API refactoring.
