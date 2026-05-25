# 0505 — Add model-output schema validation integration

## Goal
Validate parsed model-output candidates against caller-provided JSON Schemas after repair/alias normalization.

## Dependencies
- 0503
- 0304

## Scope
- Apply schema validation per candidate.
- Report validation diagnostics without discarding raw candidates.
- Preserve distinction between parse success and semantic/schema validity.

## Acceptance criteria
- Valid/invalid candidate fixtures produce clear validation results.
- Validation can be enabled by library options and CLI flags.
- Candidate raw data and repair metadata remain available after validation failure.
