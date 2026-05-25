# 0404 — Add Rust parser detail modes

## Goal
Expose optional syntax/parser detail without making tree-sitter's raw model the default contract.

## Dependencies
- 0402

## Scope
- Add Rust-command/API-specific detail options.
- Default to semantic Grist-owned model.
- Provide selected syntax metadata and a syntax/debug mode for parser investigation.
- Clearly mark stable semantic fields vs debug/detail fields.

## Acceptance criteria
- CLI/API can request Rust detail modes.
- Default output remains stable and semantic.
- Debug detail helps trace symbols back to tree-sitter node kinds/ranges.
