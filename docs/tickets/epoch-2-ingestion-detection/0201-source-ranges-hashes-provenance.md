# 0201 — Implement source ranges, hashes, and provenance

## Goal
Provide reusable utilities for source ranges, hashing, and provenance metadata.

## Dependencies
- 0101

## Scope
- Compute raw byte SHA-256 and optional decoded text hash.
- Normalize source path/URI metadata.
- Provide line/column indexing utilities for UTF-8 text.
- Emit diagnostics for decode/range issues.

## Acceptance criteria
- Hashes are stable and tested.
- Byte offsets and line/column ranges can be derived consistently.
- Invalid UTF-8 is reported without panic.
