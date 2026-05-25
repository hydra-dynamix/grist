# 0304 — Implement schema validation hooks

## Goal
Allow parsed JSON-like values to be validated against caller-provided JSON Schemas.

## Dependencies
- 0302

## Scope
- Add schema options to relevant parser APIs.
- Validate after parse/normalization.
- Return structured validation diagnostics.
- Keep repair/alias logic separate from validation.

## Acceptance criteria
- Valid and invalid JSON Schema fixtures pass/fail with clear diagnostics.
- Parsing remains useful without schemas.
- CLI can accept a schema path for supported commands.
