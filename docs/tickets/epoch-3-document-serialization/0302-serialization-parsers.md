# 0302 — Implement JSON, JSONL, YAML, and TOML parsers

## Goal
Add initial common serialization support.

## Dependencies
- 0203

## Scope
- Parse JSON, JSONL, YAML, and TOML into typed Grist serialization payloads.
- Preserve raw source metadata and parser diagnostics.
- Report partial/malformed inputs clearly.
- Keep TSV out of scope. CSV is now a first-class parser/ingestion artifact with its own payload schema.

## Acceptance criteria
- Each supported format has valid and malformed fixtures.
- JSONL reports per-line diagnostics where useful.
- CLI `parse json` works for JSON and reports diagnostics for invalid input.
