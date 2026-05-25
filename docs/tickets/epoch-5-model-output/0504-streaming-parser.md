# 0504 — Implement streaming parser

## Goal
Support incremental model-output parsing from day one.

## Dependencies
- 0501

## Scope
- Accept chunks and emit parse events.
- Track partial JSON, fenced blocks, Python-style calls, OpenAI calls, and MCP JSON-RPC objects.
- Provide `finish()` or equivalent to emit final accumulated report.
- Distinguish incomplete, complete valid, recovered/malformed, and unrecoverable states.

## Acceptance criteria
- Streaming fixtures match batch final reports where applicable.
- Partial structures emit meaningful events and diagnostics.
- Long model outputs are supported by centralized generous limits.
