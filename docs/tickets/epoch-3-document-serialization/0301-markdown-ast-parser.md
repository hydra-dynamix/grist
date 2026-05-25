# 0301 — Implement Markdown AST parser

## Goal
Parse Markdown into Grist's AST-like public model with source ranges.

## Dependencies
- 0203

## Scope
- Select/confirm Markdown parser with source-position support.
- Emit document root, headings, paragraphs/text blocks, and code fences.
- Attach ranges to every node where available.
- Preserve parser diagnostics for malformed Markdown structures.

## Acceptance criteria
- Markdown parse output is typed, schema-versioned, and serializable.
- Headings/paragraphs/fences include source ranges.
- Malformed fences produce diagnostics.
