---
ldgr_doc: 1
kind: ticket
id: ticket.document-graph-schemas-cli
schema: ldgr.ticket.v1
status: ready
produces:
- work:document-graph-schemas-cli
tags:
- grist
- document-graph
- epoch-7
- schemas
---

# 0702 — Expose DocumentGraph schemas and CLI emission

**Slug:** `document-graph-schemas-cli`

**Epoch:** Epoch 7 — DocumentGraph foundation

## Objective

Publish the DocumentGraph schema contract and add CLI/schema surfaces so operators can emit and validate the normalized graph projection.

```ldgr-contract yaml
title: >-
  Expose DocumentGraph schemas and CLI emission
description: >-
  Publish the DocumentGraph schema contract and add CLI/schema surfaces so operators can emit and validate the normalized graph projection.
requirements:
- id: req.01
  text: >-
    d-in schema artifacts include the new graph contract.
  evidence_required: true
- id: req.02
  text: >-
    n emit the schema by name.
  evidence_required: true
- id: req.03
  text: >-
    ng schema contract tests pass.
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
    cargo test --features cli schema::tests::checked_in_schemas_match_generated_public_contracts
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

- [ ] Checked-in schema artifacts include the new graph contract.
- [ ] CLI can emit the schema by name.
- [ ] Existing schema contract tests pass.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- No graph conversion logic.
- No transform CLI commands.
