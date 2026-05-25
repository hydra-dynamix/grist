# 0502 — Implement batch model-output grammars

## Goal
Parse complete model responses into one or more candidates.

## Dependencies
- 0501

## Scope
- Support Python-style command calls.
- Support OpenAI-style tool/function calls.
- Support raw JSON, fenced JSON/code, and MCP JSON-RPC.
- Support YAML/TOML/XML-ish extracted tool blocks where useful.
- Preserve raw ranges and diagnostics.

## Acceptance criteria
- Fixtures cover each grammar.
- Multiple candidates are returned rather than discarded.
- Parsing success does not imply schema or trust validity.
