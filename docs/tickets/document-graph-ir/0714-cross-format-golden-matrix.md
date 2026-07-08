---
ldgr_doc: 1
kind: ticket
id: ticket.cross-format-golden-matrix
schema: ldgr.ticket.v1
status: ready
produces:
- work:cross-format-golden-matrix
tags:
- grist
- document-graph
- epoch-11
- validation
---

# 0714 — Build cross-format golden fixture matrix

**Slug:** `cross-format-golden-matrix`

**Epoch:** Epoch 11 — Integration and operator surface

## Objective

Create a regression matrix that validates parser-specific payloads, DocumentGraph projections, cross-format transforms, and semantic obligation extraction together.

```ldgr-contract yaml
title: >-
  Build cross-format golden fixture matrix
description: >-
  Create a regression matrix that validates parser-specific payloads, DocumentGraph projections, cross-format transforms, and semantic obligation extraction together.
requirements:
- id: req.01
  text: >-
    format golden tests are checked in.
  evidence_required: true
- id: req.02
  text: >-
    e/edge fixtures are included.
  evidence_required: true
- id: req.03
  text: >-
    e-set test commands are documented.
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
    cargo test --features "cli basin" cross_format_golden_matrix
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

- ticket.transform-cli-registry

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Cross-format golden tests are checked in.
- [ ] Failure/edge fixtures are included.
- [ ] Feature-set test commands are documented.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- Performance benchmarking.
- CodeSearchNet experiment harness.
