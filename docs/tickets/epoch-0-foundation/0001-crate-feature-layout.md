# 0001 — Establish crate module and feature layout

## Goal
Create the top-level crate/module organization and Cargo feature strategy described in `docs/spec.md`.

## Dependencies
None.

## Scope
- Add module skeletons for `core`, `detect`, `ingest`, `markdown`, `rust`, `serialization`, `model_output`, `schema`, and CLI support.
- Define Cargo features for `markdown`, `rust`, `serialization`, `model-output`, `schemas`, and `cli`.
- Set default features to the initial core set.
- Keep modules empty/minimal where implementation belongs to later tickets.

## Acceptance criteria
- `cargo check` passes with default features.
- `cargo check --no-default-features` passes or has documented exclusions.
- Public module layout matches the spec.
