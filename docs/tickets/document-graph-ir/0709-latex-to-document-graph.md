---
ldgr_doc: 1
kind: ticket
id: ticket.latex-to-document-graph
schema: ldgr.ticket.v1
status: ready
produces:
- work:latex-to-document-graph
tags:
- grist
- document-graph
- epoch-9
- latex
---

# 0709 — Project LaTeX into DocumentGraph

**Slug:** `latex-to-document-graph`

**Epoch:** Epoch 9 — LaTeX support

## Objective

Convert LaTeXDocument into DocumentGraph with section, paragraph, formatting, math, label/ref/citation, environment, and raw node coverage.

```ldgr-contract yaml
title: >-
  Project LaTeX into DocumentGraph
description: >-
  Convert LaTeXDocument into DocumentGraph with section, paragraph, formatting, math, label/ref/citation, environment, and raw node coverage.
requirements:
- id: req.01
  text: >-
    fixtures produce expected graph structure.
  evidence_required: true
- id: req.02
  text: >-
    on/ref edges are represented.
  evidence_required: true
- id: req.03
  text: >-
    known content remains inspectable.
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
    cargo test --features "cli latex basin" latex_to_document_graph
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

- ticket.latex-parser-contract
- ticket.document-graph-schemas-cli

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] LaTeX fixtures produce expected graph structure.
- [ ] Citation/ref edges are represented.
- [ ] Raw unknown content remains inspectable.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- Graph-to-LaTeX rendering.
- Obligation extraction from LaTeX prose.
