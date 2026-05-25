# 0401 — Set up tree-sitter Rust parser

## Goal
Integrate tree-sitter Rust as the backing parser for Rust ingestion.

## Dependencies
- 0203

## Scope
- Add tree-sitter parser initialization behind the Rust feature.
- Parse Rust source into an internal tree.
- Detect ERROR/MISSING nodes and convert them to diagnostics.
- Return a minimal Rust payload with parser metadata.

## Acceptance criteria
- Valid Rust parses successfully.
- Invalid Rust returns partial diagnostics with ranges.
- No regex/heuristic-only Rust parser is used as the main implementation.
