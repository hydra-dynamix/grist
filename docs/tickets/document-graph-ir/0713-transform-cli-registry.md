---
ldgr_doc: 1
kind: ticket
id: ticket.transform-cli-registry
schema: ldgr.ticket.v1
status: ready
produces:
- work:transform-cli-registry
tags:
- grist
- document-graph
- epoch-11
- cli
---

# 0713 — Add transform registry and CLI commands

**Slug:** `transform-cli-registry`

**Epoch:** Epoch 11 — Integration and operator surface

## Objective

Expose graph-mediated document transforms through production CLI/API surfaces with explicit format selection, diagnostics, and failure behavior.

```ldgr-contract yaml
title: >-
  Add transform registry and CLI commands
description: >-
  Expose graph-mediated document transforms through production CLI/API surfaces with explicit format selection, diagnostics, and failure behavior.
requirements:
- id: req.01
  text: >-
    ors can run graph emission and transforms from CLI.
  evidence_required: true
- id: req.02
  text: >-
    and warnings are observable.
  evidence_required: true
- id: req.03
  text: >-
    e-gated command availability is tested.
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
    cargo test --features "cli basin" transform_cli_registry
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

- ticket.document-render-latex
- ticket.conditional-obligation-extraction
- ticket.basin-document-graph-adapter

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Operators can run graph emission and transforms from CLI.
- [ ] Errors and warnings are observable.
- [ ] Feature-gated command availability is tested.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- PDF generation.
- Interactive editor integrations.
