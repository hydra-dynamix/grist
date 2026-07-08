---
ldgr_doc: 1
kind: ticket
id: ticket.code-to-document-graph
schema: ldgr.ticket.v1
status: ready
produces:
- work:code-to-document-graph
tags:
- grist
- document-graph
- epoch-8
- code
---

# 0705 — Project code parser payloads into DocumentGraph

**Slug:** `code-to-document-graph`

**Epoch:** Epoch 8 — Existing parser projections

## Objective

Implement Python, Rust, and TypeScript payload projections into DocumentGraph using shared code-symbol, import/export, containment, call, inheritance, and reference relations.

```ldgr-contract yaml
title: >-
  Project code parser payloads into DocumentGraph
description: >-
  Implement Python, Rust, and TypeScript payload projections into DocumentGraph using shared code-symbol, import/export, containment, call, inheritance, and reference relations.
requirements:
- id: req.01
  text: >-
    ree supported code parsers produce graph projections.
  evidence_required: true
- id: req.02
  text: >-
    IMPORTS/CONTAINS/INHERITS coverage is tested where input supports it.
  evidence_required: true
- id: req.03
  text: >-
    e-gate combinations compile.
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
    cargo test --features "cli basin" code_to_document_graph
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

- [ ] All three supported code parsers produce graph projections.
- [ ] CALLS/IMPORTS/CONTAINS/INHERITS coverage is tested where input supports it.
- [ ] Feature-gate combinations compile.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- Adding new parsers.
- Changing source-language parser extraction semantics except for bugs discovered in tests.
