---
ldgr_doc: 1
kind: ticket
id: ticket.markdown-to-document-graph
schema: ldgr.ticket.v1
status: ready
produces:
- work:markdown-to-document-graph
tags:
- grist
- document-graph
- epoch-8
- markdown
---

# 0704 — Project Markdown into DocumentGraph

**Slug:** `markdown-to-document-graph`

**Epoch:** Epoch 8 — Existing parser projections

## Objective

Implement a complete MarkdownDocument to DocumentGraph projection for headings, paragraphs, inline text, links, code fences, tables, frontmatter, and diagnostics/provenance.

```ldgr-contract yaml
title: >-
  Project Markdown into DocumentGraph
description: >-
  Implement a complete MarkdownDocument to DocumentGraph projection for headings, paragraphs, inline text, links, code fences, tables, frontmatter, and diagnostics/provenance.
requirements:
- id: req.01
  text: >-
    wn fixtures produce expected graph nodes and edges.
  evidence_required: true
- id: req.02
  text: >-
    and links are represented.
  evidence_required: true
- id: req.03
  text: >-
    med fixture diagnostics are not lost.
  evidence_required: true
constraints:
- id: con.01
  text: >-
    Preserve existing parser-specific public payloads unless a later migration explicitly changes them.
- id: con.02
  text: >-
    Do not silently drop unsupported content; preserve it as raw data or report a structured diagnostic.
tests:
- id: test.01
  command: >-
    cargo test --features "cli basin" markdown_to_document_graph
  required: true
validation_instructions:
- >-
  Run the narrowest relevant cargo checks/tests for changed modules.
- >-
  Run schema contract tests when public serializable structs or schema outputs change.
- >-
  Record validation commands and results as LDGR observations before closure.
expected_artifacts:
- implementation changes
- tests or fixtures proving production behavior
- LDGR observation summarizing validation evidence
```

## Dependencies

- ticket.document-transform-traits

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Markdown fixtures produce expected graph nodes and edges.
- [ ] Tables and links are represented.
- [ ] Malformed fixture diagnostics are not lost.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- Graph-to-Markdown rendering.
- LaTeX conversion.
