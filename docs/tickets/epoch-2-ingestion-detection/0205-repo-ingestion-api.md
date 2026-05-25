# 0205 — Implement repo ingestion API

## Goal
Implement repository traversal and aggregated repo reports.

## Dependencies
- 0204

## Scope
- Traverse root using gitignore-style semantics.
- Honor repo ignore rules by default with override to include ignored files.
- Apply include/exclude globs and shared limits.
- Produce inventory, inline parsed artifacts, skipped/ignored/unsupported summaries, hashes, and diagnostics.

## Acceptance criteria
- CLI `ingest repo` emits a schema-versioned JSON report.
- Ignored-file behavior is tested with override.
- Binary files are skipped/reported without full artifact records.
