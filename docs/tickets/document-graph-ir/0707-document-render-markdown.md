---
ldgr_doc: 1
kind: ticket
id: ticket.document-render-markdown
schema: ldgr.ticket.v1
status: ready
produces:
- work:document-render-markdown
tags:
- grist
- document-graph
- epoch-8
- markdown
- render
---

# 0707 — Render DocumentGraph back to Markdown

**Slug:** `document-render-markdown`

**Epoch:** Epoch 8 — Existing parser projections

## Objective

Implement a production Markdown renderer for supported prose graph nodes so graph-mediated Markdown transforms have a concrete output path.

```ldgr-contract yaml
title: >-
  Render DocumentGraph back to Markdown
description: >-
  Implement a production Markdown renderer for supported prose graph nodes so graph-mediated Markdown transforms have a concrete output path.
requirements:
- id: req.01
  text: >-
    entative Markdown documents render back to valid Markdown.
  evidence_required: true
- id: req.02
  text: >-
    orted nodes are reported.
  evidence_required: true
- id: req.03
  text: >-
    trip fixture expectations are checked in.
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
    cargo test --features "cli basin" document_render_markdown
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

- ticket.markdown-to-document-graph

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Representative Markdown documents render back to valid Markdown.
- [ ] Unsupported nodes are reported.
- [ ] Round-trip fixture expectations are checked in.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- LaTeX rendering.
- HTML rendering unless needed by fixtures.
