# 0602 — Build golden fixture and regression test matrix

## Goal
Create the fixture/golden test suite that locks Grist's public behavior.

## Dependencies
- 0303
- 0304
- 0404
- 0506

## Scope
- Add valid, malformed, adversarial, and oversized fixtures.
- Add golden outputs for Markdown, Rust, serialization, model-output, file ingest, and repo ingest.
- Add schema validation for CLI JSON outputs.
- Add fuzz/property tests where practical for model-output repair.

## Acceptance criteria
- `dev test` covers all initial parser families.
- Golden outputs are stable and intentionally reviewed.
- Snapshot scope avoids huge end-to-end artifacts.
