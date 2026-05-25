# 0501 — Define model-output candidate and event model

## Goal
Create the typed public model for model-output candidates, diagnostics, normalizations, and streaming events.

## Dependencies
- 0101

## Scope
- Define candidate ids, grammar/type, command/tool/function names, argument names, parsed values, raw ranges, status/confidence, normalizations, diagnostics, and validation results.
- Define streaming events: candidate started/updated/completed, diagnostic, parser state changed.
- Ensure batch and streaming results share compatible structures.

## Acceptance criteria
- Types serialize to versioned JSON.
- Candidate model can represent multiple candidates.
- Event model can represent partial/incomplete structures.
