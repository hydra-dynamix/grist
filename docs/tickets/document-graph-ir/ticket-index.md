---
ldgr_doc: 1
kind: ticket_index
id: ticket_index.document-graph-ir.v1
schema: ldgr.ticket_index.v1
status: ready
tags:
- grist
- document-graph
- ticket-index
---

# DocumentGraph IR Ticket Index

Artifact refs use `artifact:pending-*` placeholders until these planning files are recorded as immutable artifact IDs. This index is planning-ready but not conduct-launch-ready without artifact id substitution.

```ldgr-ticket-index yaml
tickets:
  - id: ticket.document-graph-contract
    artifact: artifact:pending-document-graph-contract
    title: Define the DocumentGraph public contract
    work_item: work:document-graph-contract
  - id: ticket.document-graph-schemas-cli
    artifact: artifact:pending-document-graph-schemas-cli
    title: Expose DocumentGraph schemas and CLI emission
    work_item: work:document-graph-schemas-cli
  - id: ticket.document-transform-traits
    artifact: artifact:pending-document-transform-traits
    title: Add transform traits and error model
    work_item: work:document-transform-traits
  - id: ticket.markdown-to-document-graph
    artifact: artifact:pending-markdown-to-document-graph
    title: Project Markdown into DocumentGraph
    work_item: work:markdown-to-document-graph
  - id: ticket.code-to-document-graph
    artifact: artifact:pending-code-to-document-graph
    title: Project code parser payloads into DocumentGraph
    work_item: work:code-to-document-graph
  - id: ticket.basin-document-graph-adapter
    artifact: artifact:pending-basin-document-graph-adapter
    title: Refactor basin to consume DocumentGraph
    work_item: work:basin-document-graph-adapter
  - id: ticket.document-render-markdown
    artifact: artifact:pending-document-render-markdown
    title: Render DocumentGraph back to Markdown
    work_item: work:document-render-markdown
  - id: ticket.latex-parser-contract
    artifact: artifact:pending-latex-parser-contract
    title: Add LaTeX parser contract and feature baseline
    work_item: work:latex-parser-contract
  - id: ticket.latex-to-document-graph
    artifact: artifact:pending-latex-to-document-graph
    title: Project LaTeX into DocumentGraph
    work_item: work:latex-to-document-graph
  - id: ticket.document-render-latex
    artifact: artifact:pending-document-render-latex
    title: Render DocumentGraph to LaTeX
    work_item: work:document-render-latex
  - id: ticket.conditional-obligation-model
    artifact: artifact:pending-conditional-obligation-model
    title: Model conditional obligations in DocumentGraph
    work_item: work:conditional-obligation-model
  - id: ticket.conditional-obligation-extraction
    artifact: artifact:pending-conditional-obligation-extraction
    title: Extract conditional obligations from document prose
    work_item: work:conditional-obligation-extraction
  - id: ticket.transform-cli-registry
    artifact: artifact:pending-transform-cli-registry
    title: Add transform registry and CLI commands
    work_item: work:transform-cli-registry
  - id: ticket.cross-format-golden-matrix
    artifact: artifact:pending-cross-format-golden-matrix
    title: Build cross-format golden fixture matrix
    work_item: work:cross-format-golden-matrix
  - id: ticket.document-graph-docs-migration
    artifact: artifact:pending-document-graph-docs-migration
    title: Document DocumentGraph architecture and migration path
    work_item: work:document-graph-docs-migration
```
