---
ldgr_doc: 1
kind: ticket
id: ticket.document-graph-docs-migration
schema: ldgr.ticket.v1
status: ready
produces:
- work:document-graph-docs-migration
tags:
- grist
- document-graph
- epoch-11
- docs
---

# 0715 — Document DocumentGraph architecture and migration path

**Slug:** `document-graph-docs-migration`

**Epoch:** Epoch 11 — Integration and operator surface

## Objective

Document the normalized DocumentGraph architecture, transform guarantees, feature flags, semantic obligation policy, and migration guidance for downstream consumers.

```ldgr-contract yaml
title: >-
  Document DocumentGraph architecture and migration path
description: >-
  Document the normalized DocumentGraph architecture, transform guarantees, feature flags, semantic obligation policy, and migration guidance for downstream consumers.
requirements:
- id: req.01
  text: >-
    ecture docs are present.
  evidence_required: true
- id: req.02
  text: >-
    amples are accurate.
  evidence_required: true
- id: req.03
  text: >-
    ream compatibility guidance is updated.
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
    cargo test --features "cli basin" document_graph_docs_migration
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

- ticket.cross-format-golden-matrix

## Shared Context

Grist already has parser-specific payloads for Markdown, HTML, CSV, Rust, Python, TypeScript, serialization, model-output, text, repo ingestion, LDGR projection, and a new feature-gated `basin` module. This work adds a normalized central DocumentGraph projection without deleting or replacing existing public parser payloads. The graph must support structural document transforms, code semantic graphs, future LaTeX support, and conditional-obligation semantics with provenance.

## Acceptance Criteria

- [ ] Architecture docs are present.
- [ ] CLI examples are accurate.
- [ ] Downstream compatibility guidance is updated.

## Validation Guidance

- Prefer targeted unit tests for the module under change.
- Run `cargo test --features "cli basin"` before closing integration tickets.
- If checked-in schemas change, regenerate and validate schema drift tests.

## Out of Scope

- Paper updates.
- External website/docs deployment.
