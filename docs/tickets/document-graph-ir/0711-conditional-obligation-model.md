---
ldgr_doc: 1
kind: ticket
id: ticket.conditional-obligation-model
schema: ldgr.ticket.v1
status: ready
produces:
- work:conditional-obligation-model
tags:
- grist
- document-graph
- epoch-10
- semantics
---

# 0711 — Model conditional obligations in DocumentGraph

**Slug:** `conditional-obligation-model`

**Epoch:** Epoch 10 — Semantic obligations

## Objective

Add first-class semantic node/relation coverage for obligations, conditions, requirements, permissions, prohibitions, satisfaction, violation, and provenance.

```ldgr-contract yaml
title: >-
  Model conditional obligations in DocumentGraph
description: >-
  Add first-class semantic node/relation coverage for obligations, conditions, requirements, permissions, prohibitions, satisfaction, violation, and provenance.
requirements:
- id: req.01
  text: >-
    ic obligation graph fixtures serialize and validate.
  evidence_required: true
- id: req.02
  text: >-
    ed metadata is present.
  evidence_required: true
- id: req.03
  text: >-
    dge-only representation is avoided.
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
    cargo test --features "cli basin" conditional_obligation_model
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

- ticket.document-graph-contract
- ticket.document-graph-schemas-cli

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Semantic obligation graph fixtures serialize and validate.
- [ ] Required metadata is present.
- [ ] Bare-edge-only representation is avoided.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- NLP/LLM extraction.
- Satisfaction checking against code/tests.
