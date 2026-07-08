---
ldgr_doc: 1
kind: ticket
id: ticket.document-graph-contract
schema: ldgr.ticket.v1
status: ready
produces:
- work:document-graph-contract
tags:
- grist
- document-graph
- epoch-7
---

# 0701 — Define the DocumentGraph public contract

**Slug:** `document-graph-contract`

**Epoch:** Epoch 7 — DocumentGraph foundation

## Objective

Add the central DocumentGraph IR types that can represent prose, code, semantic facts, and provenance while preserving existing parser-specific payloads.

```ldgr-contract yaml
title: >-
  Define the DocumentGraph public contract
description: >-
  Add the central DocumentGraph IR types that can represent prose, code, semantic facts, and provenance while preserving existing parser-specific payloads.
requirements:
- id: req.01
  text: >-
    ntGraph public types exist and serialize/deserialze correctly.
  evidence_required: true
- id: req.02
  text: >-
    rose/semantic node and relation families are represented.
  evidence_required: true
- id: req.03
  text: >-
    ng parser payloads continue to compile unchanged.
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
    cargo test --features "cli basin" document_graph_contract
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

- none

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] DocumentGraph public types exist and serialize/deserialze correctly.
- [ ] Code/prose/semantic node and relation families are represented.
- [ ] Existing parser payloads continue to compile unchanged.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- No parser conversion implementation beyond minimal construction tests.
- No LaTeX parser implementation.
