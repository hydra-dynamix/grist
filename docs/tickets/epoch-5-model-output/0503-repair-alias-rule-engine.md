# 0503 — Implement repair and alias rule engine

## Goal
Add runtime-configurable model-output repair and alias normalization.

## Dependencies
- 0502

## Scope
- Load/construct rules from Rust structs and CLI JSON/TOML configs.
- Support field, command/tool/function, and argument aliases.
- Support common JSON-ish repairs: fences, balanced object extraction, nested stringified JSON, single quotes, unquoted keys, trailing commas, Python booleans/nulls, and mislabeled outputs.
- Track normalizations and warnings.

## Acceptance criteria
- Repair fixtures include examples from graph-composer/research-chain/bash agent patterns.
- Alias rules are project-configurable at runtime.
- Unsafe or ambiguous repairs produce diagnostics.
