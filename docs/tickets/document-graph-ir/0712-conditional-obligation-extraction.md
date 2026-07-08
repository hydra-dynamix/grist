---
ldgr_doc: 1
kind: ticket
id: ticket.conditional-obligation-extraction
schema: ldgr.ticket.v1
status: ready
produces:
- work:conditional-obligation-extraction
tags:
- grist
- document-graph
- epoch-10
- semantics
---

# 0712 — Extract conditional obligations from document prose

**Slug:** `conditional-obligation-extraction`

**Epoch:** Epoch 10 — Semantic obligations

## Objective

Implement deterministic semantic passes that extract explicit conditional obligations from Markdown and LaTeX graph prose into obligation/condition subgraphs.

```ldgr-contract yaml
title: >-
  Extract conditional obligations from document prose
description: >-
  Implement deterministic semantic passes that extract explicit conditional obligations from Markdown and LaTeX graph prose into obligation/condition subgraphs.
requirements:
- id: req.01
  text: >-
    ve and negative fixtures pass.
  evidence_required: true
- id: req.02
  text: >-
    ted obligations preserve provenance.
  evidence_required: true
- id: req.03
  text: >-
    wn and LaTeX graph inputs are supported.
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
    cargo test --features "cli basin" conditional_obligation_extraction
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

- ticket.conditional-obligation-model
- ticket.markdown-to-document-graph
- ticket.latex-to-document-graph

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Positive and negative fixtures pass.
- [ ] Extracted obligations preserve provenance.
- [ ] Markdown and LaTeX graph inputs are supported.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- Checking whether obligations are satisfied by code.
- Advanced natural-language parsing beyond deterministic rules.
