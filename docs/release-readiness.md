# Grist release-readiness report

Date: 2026-08-09 (America/Vancouver)

Branch: `codex/grist-parser-schedule`

LDGR run: 216, `grist-release-readiness`

## Outcome

The retained Grist Complete Parser Specification is implemented and verified.
The historical work graph was consolidated into six direct LDGR items and
completed without Conduct orchestration. The final queue contains no pending
or held work.

User-approved scope exclusions are limited to legacy binary Office formats and
Outlook stores: DOC, WordProcessingML 2003, Flat OPC, PPT, XLS, XLSB,
SpreadsheetML 2003, PST and OST. All other specification sections and retained
format families are covered by the final audit in
`docs/complete-parser-contract.md`.

## Final changes

- Replaced the stale pre-implementation gap matrix with a Sections 1–19
  retained-spec audit and exact exclusion record.
- Added code/repository, media and model-output family documentation.
- Added a standard cargo-fuzz target, checked regression corpus and ordinary-CI
  no-panic regression test.
- Added a representative parser performance smoke/benchmark.
- Added a loader-replacement end-to-end scenario spanning Markdown, CSV, EML
  and Python through graph transform, schema validation and segmentation.
- Repaired `transform` source inference to use registry auto-detection instead
  of a duplicated extension allowlist. This closed the EML transform failure
  and prevents equivalent drift for other registered formats.

## Verification evidence

All commands ran serially with `CARGO_BUILD_JOBS=1`,
`CARGO_INCREMENTAL=0`, `--locked` where applicable, and the dedicated target
directory `D:\apps\.grist-cargo-target-consolidated-1`.

| Gate | Result |
|---|---|
| `cargo test --locked --all-features -- --test-threads=1` | pass: 94 library tests, every integration test, and doc tests; zero failures |
| focused final contract/loader/performance/fuzz tests | pass |
| `cargo test --locked --no-default-features --lib -- --test-threads=1` | pass: 12 tests |
| `cargo run --locked --all-features --example schema_codegen -- --check` | pass, no drift |
| CLI `capabilities`, `parse auto README.md`, `transform README.md --to graph` | pass |
| `cargo check --manifest-path fuzz/Cargo.toml --bin universal_bytes` | pass |
| `cargo clippy --locked --all-targets --all-features` | pass with the documented advisory baseline |
| root and fuzz formatting plus `git diff --check` | pass |

The advisory Clippy baseline contains 38 library style/API lints and several
test-style lints. The exact project gate succeeds; no correctness lint or build
failure is hidden. A non-project `-D warnings` policy would require unrelated
API/style refactoring and is outside this completed parser scope.
