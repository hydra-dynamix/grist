---
ldgr_doc: 1
kind: ticket
id: ticket.basin-document-graph-adapter
schema: ldgr.ticket.v1
status: ready
produces:
- work:basin-document-graph-adapter
tags:
- grist
- document-graph
- epoch-8
- basin
---

# 0706 — Refactor basin to consume DocumentGraph

**Slug:** `basin-document-graph-adapter`

**Epoch:** Epoch 8 — Existing parser projections

## Objective

Move basin graph construction to the central DocumentGraph so basin retrieval becomes language-neutral across current and future code parsers.

```ldgr-contract yaml
title: >-
  Refactor basin to consume DocumentGraph
description: >-
  Move basin graph construction to the central DocumentGraph so basin retrieval becomes language-neutral across current and future code parsers.
requirements:
- id: req.01
  text: >-
    tests pass using DocumentGraph.
  evidence_required: true
- id: req.02
  text: >-
    helper remains available.
  evidence_required: true
- id: req.03
  text: >-
    apter can accept non-Python graph inputs for future code languages.
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
    cargo test --features "cli basin" basin_document_graph_adapter
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

- ticket.code-to-document-graph

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Basin tests pass using DocumentGraph.
- [ ] Python helper remains available.
- [ ] The adapter can accept non-Python graph inputs for future code languages.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- Fresh-domain 12.7x reproduction harness.
- Embedding integration.
