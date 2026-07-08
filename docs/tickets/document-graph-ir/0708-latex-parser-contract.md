---
ldgr_doc: 1
kind: ticket
id: ticket.latex-parser-contract
schema: ldgr.ticket.v1
status: ready
produces:
- work:latex-parser-contract
tags:
- grist
- document-graph
- epoch-9
- latex
---

# 0708 — Add LaTeX parser contract and feature baseline

**Slug:** `latex-parser-contract`

**Epoch:** Epoch 9 — LaTeX support

## Objective

Introduce the LaTeX module contract, feature flag, dependency baseline, parser options, and typed LaTeXDocument payload without implementing full transformation semantics.

```ldgr-contract yaml
title: >-
  Add LaTeX parser contract and feature baseline
description: >-
  Introduce the LaTeX module contract, feature flag, dependency baseline, parser options, and typed LaTeXDocument payload without implementing full transformation semantics.
requirements:
- id: req.01
  text: >-
    payload structs exist and serialize.
  evidence_required: true
- id: req.02
  text: >-
    arsing works through CLI/file ingestion.
  evidence_required: true
- id: req.03
  text: >-
    n commands are preserved.
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
    cargo test --features "cli latex basin" latex_parser_contract
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

- [ ] LaTeX payload structs exist and serialize.
- [ ] .tex parsing works through CLI/file ingestion.
- [ ] Unknown commands are preserved.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- DocumentGraph projection.
- LaTeX rendering from DocumentGraph.
