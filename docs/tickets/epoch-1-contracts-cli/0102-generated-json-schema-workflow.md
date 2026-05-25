# 0102 — Implement generated JSON Schema workflow

## Goal
Generate and check in JSON Schemas for public Grist output contracts.

## Dependencies
- 0101

## Scope
- Add schema generation using Rust public types.
- Write schemas under `schemas/`.
- Add tests or a dev task that detects schema drift.
- Include schema versions in all relevant output types.

## Acceptance criteria
- Schema files are generated for core/envelope outputs.
- Drift test fails when Rust types and checked-in schemas diverge.
- `grist schema list` and `grist schema emit` can later use the same registry.
