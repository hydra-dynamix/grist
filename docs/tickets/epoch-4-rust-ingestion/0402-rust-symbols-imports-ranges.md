# 0402 — Extract Rust symbols, imports, and ranges

## Goal
Emit core Rust semantic facts from the tree-sitter parse tree.

## Dependencies
- 0401

## Scope
- Extract modules, functions, structs, enums, traits, impl blocks, consts, statics, type aliases, and imports.
- Include symbol ids, names, kinds, visibility, parent relationships, attributes, docs where available, and source ranges.
- Parse use trees including aliases/groups.

## Acceptance criteria
- Symbol/import fixtures cover common Rust files.
- Extracted ranges map back to source slices.
- Output uses stable Grist-owned public models.
