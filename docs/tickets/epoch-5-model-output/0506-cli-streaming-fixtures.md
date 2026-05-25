# 0506 — Add model-output CLI and streaming fixtures

## Goal
Expose model-output parsing through CLI and verify streaming behavior with fixtures.

## Dependencies
- 0504
- 0505

## Scope
- Implement `grist parse model-output <path|->`.
- Add flags for repair/alias rule config and schema validation.
- Add fixture tests for batch and streaming-equivalent inputs.
- Ensure CLI emits JSON-only reports.

## Acceptance criteria
- CLI handles stdin and files.
- Fixture matrix covers Python-style, OpenAI, MCP, fenced JSON, malformed JSON, multiple candidates, and streaming partials.
- CLI output validates against Grist schemas.
