---
ldgr_doc: 1
kind: ticket
id: ticket.document-transform-traits
schema: ldgr.ticket.v1
status: ready
produces:
- work:document-transform-traits
tags:
- grist
- document-graph
- epoch-7
- transforms
---

# 0703 — Add transform traits and error model

**Slug:** `document-transform-traits`

**Epoch:** Epoch 7 — DocumentGraph foundation

## Objective

Define production traits and typed errors for converting parser payloads into DocumentGraph and rendering DocumentGraph into target document forms.

```ldgr-contract yaml
title: >-
  Add transform traits and error model
description: >-
  Define production traits and typed errors for converting parser payloads into DocumentGraph and rendering DocumentGraph into target document forms.
requirements:
- id: req.01
  text: >-
    sion and rendering traits compile under relevant feature combinations.
  evidence_required: true
- id: req.02
  text: >-
    cases have unit coverage.
  evidence_required: true
- id: req.03
  text: >-
    ture leakage forces unused parser dependencies.
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
    cargo test --features "cli basin" document_transform_traits
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

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Conversion and rendering traits compile under relevant feature combinations.
- [ ] Error cases have unit coverage.
- [ ] No feature leakage forces unused parser dependencies.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- No concrete Markdown/LaTeX/code converters beyond small test fixtures.
- No CLI transform command.
