# 0002 — Add initial dependency baseline

## Goal
Add the dependency set needed to implement Grist without repeated Cargo churn.

## Dependencies
- 0001

## Scope
- Add serialization/schema/hash/CLI/parser traversal dependencies.
- Add tree-sitter Rust dependencies behind the Rust feature.
- Add Markdown, YAML, TOML, JSON Schema, and error dependencies after confirming crate fit.
- Respect supply-chain age/security expectations before adding new crates.

## Acceptance criteria
- Dependencies are feature-gated where practical.
- `dev check` passes.
- Dependency choices are documented in code comments or a short docs note if non-obvious.
