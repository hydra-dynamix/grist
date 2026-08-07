# Parser promotion harness

`grist::promotion` is the one policy owner for format completion. A format may
be described as promotion-eligible only when `ParserPromotionHarness::evaluate`
returns a `ParserConformanceReport` with `eligible_for_promotion: true`.
Callers must not infer promotion from registration, feature availability, or a
subset of passing format tests.

## Adapter boundary

Each format implements `ParserPromotionAdapter` in its format-owned test or
verification module. The adapter calls the real public detection, parse,
`DocumentGraph`, segmentation, schema, Rust payload, and CLI surfaces. The
`ParserPromotionSuite` supplies only inputs and expected format-specific facts;
all pass/fail policy remains in the harness.

The suite must include:

- valid, mislabeled, extensionless, malformed, and ambiguous detection cases;
- parse cases distinguishing complete, partial, failed, encrypted,
  unsupported, and budget-limited outcomes;
- explicit accounting for meaningful constructs as typed, raw/unknown, or a
  specific diagnostic, with locators for every emitted fact;
- deterministic replay and projection cases;
- one CLI/library/schema/Rust-API parity case;
- hostile cases with explicit side-effect instrumentation. Converting an
  envelope directly into `ParseExecution` does not provide security evidence;
  hostile cases must use `ParseExecution::with_security_observation`;
- governed format fixtures and non-empty, repository-relative fuzz corpora;
- one evidence record for every required verification type and CI control.

The harness catches adapter panics and records them as failed checks. Missing
cases, missing schemas, missing canonical examples, absent instrumentation,
duplicate evidence, skipped mandatory tests, unsafe fixture paths, and failed
CI commands all fail closed.

## Eleven gates

The report always contains exactly these ordered gates:

1. detection;
2. typed/raw/diagnostic construct preservation;
3. exact, approximate, or derived source locators;
4. operation statuses and explicit budget diagnostics;
5. byte-stable canonical parse, graph, and segment output;
6. valid, source-located `DocumentGraph` projection;
7. source-identity- and locator-preserving segmentation;
8. schema, canonical-example, Rust API, and CLI agreement;
9. hostile-input panic, execution, network, path, expansion, and temporary-file safety;
10. governed real-world, malformed, adversarial, raw/unknown, malicious, regression, provenance, and fuzz fixtures;
11. unit, golden, property, fuzz, differential, round-trip, source-map,
    concurrency/cancellation, compatibility, budget, security, benchmark, and
    enabled/minimal-feature CI evidence.

Only differential and round-trip verification may be marked not applicable,
and then only with a non-empty rationale. Every other verification kind must
pass.

## Machine-readable output

`ParserConformanceReport` is deterministic and contains no input bytes or
timestamps. Its checked schema is
`schemas/grist.parser-conformance-report.v1.schema.json`; the schema catalog
name is `parser-conformance-report`. The suite digest binds a report to the
exact cases and evidence metadata used for evaluation.

Recommended item-scoped checks are:

```text
cargo test --test parser_promotion_harness --features schemas
cargo run --example schema_codegen --features ldgr-projection,latex,basin -- --check
cargo test --no-default-features --test parser_promotion_harness
cargo clippy --all-targets --features schemas -- -D warnings
cargo doc --no-deps --features schemas
cargo fmt --all -- --check
```
