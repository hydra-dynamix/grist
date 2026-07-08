---
ldgr_doc: 1
kind: ticket
id: ticket.document-render-latex
schema: ldgr.ticket.v1
status: ready
produces:
- work:document-render-latex
tags:
- grist
- document-graph
- epoch-9
- latex
- render
---

# 0710 — Render DocumentGraph to LaTeX

**Slug:** `document-render-latex`

**Epoch:** Epoch 9 — LaTeX support

## Objective

Implement a LaTeX renderer from DocumentGraph for supported prose, math, citation, reference, and raw passthrough nodes.

```ldgr-contract yaml
title: >-
  Render DocumentGraph to LaTeX
description: >-
  Implement a LaTeX renderer from DocumentGraph for supported prose, math, citation, reference, and raw passthrough nodes.
requirements:
- id: req.01
  text: >-
    entative Markdown transforms render to LaTeX.
  evidence_required: true
- id: req.02
  text: >-
    entative LaTeX graph renders back to valid LaTeX.
  evidence_required: true
- id: req.03
  text: >-
    orted nodes are reported.
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
    cargo test --features "cli latex basin" document_render_latex
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

- ticket.latex-to-document-graph
- ticket.document-render-markdown

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Representative Markdown transforms render to LaTeX.
- [ ] Representative LaTeX graph renders back to valid LaTeX.
- [ ] Unsupported nodes are reported.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- PDF generation.
- Full macro package compatibility.
