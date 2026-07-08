---
ldgr_doc: 1
kind: graph
id: graph.document-graph-ir.v1
schema: ldgr.graph.v1
status: ready
tags:
- grist
- document-graph
- dependency-graph
---

# DocumentGraph IR Dependency Graph

Artifact refs use `artifact:pending-*` placeholders until these planning files are recorded as immutable artifact IDs. This graph is dependency-accurate but not conduct-launch-ready without artifact id substitution.

```ldgr-graph yaml
nodes:
  - id: ticket.document-graph-contract
    artifact: artifact:pending-document-graph-contract
    work_item: work:document-graph-contract
  - id: ticket.document-graph-schemas-cli
    artifact: artifact:pending-document-graph-schemas-cli
    work_item: work:document-graph-schemas-cli
  - id: ticket.document-transform-traits
    artifact: artifact:pending-document-transform-traits
    work_item: work:document-transform-traits
  - id: ticket.markdown-to-document-graph
    artifact: artifact:pending-markdown-to-document-graph
    work_item: work:markdown-to-document-graph
  - id: ticket.code-to-document-graph
    artifact: artifact:pending-code-to-document-graph
    work_item: work:code-to-document-graph
  - id: ticket.basin-document-graph-adapter
    artifact: artifact:pending-basin-document-graph-adapter
    work_item: work:basin-document-graph-adapter
  - id: ticket.document-render-markdown
    artifact: artifact:pending-document-render-markdown
    work_item: work:document-render-markdown
  - id: ticket.latex-parser-contract
    artifact: artifact:pending-latex-parser-contract
    work_item: work:latex-parser-contract
  - id: ticket.latex-to-document-graph
    artifact: artifact:pending-latex-to-document-graph
    work_item: work:latex-to-document-graph
  - id: ticket.document-render-latex
    artifact: artifact:pending-document-render-latex
    work_item: work:document-render-latex
  - id: ticket.conditional-obligation-model
    artifact: artifact:pending-conditional-obligation-model
    work_item: work:conditional-obligation-model
  - id: ticket.conditional-obligation-extraction
    artifact: artifact:pending-conditional-obligation-extraction
    work_item: work:conditional-obligation-extraction
  - id: ticket.transform-cli-registry
    artifact: artifact:pending-transform-cli-registry
    work_item: work:transform-cli-registry
  - id: ticket.cross-format-golden-matrix
    artifact: artifact:pending-cross-format-golden-matrix
    work_item: work:cross-format-golden-matrix
  - id: ticket.document-graph-docs-migration
    artifact: artifact:pending-document-graph-docs-migration
    work_item: work:document-graph-docs-migration
edges:
  - dependency: ticket.document-graph-contract
    dependent: ticket.document-graph-schemas-cli
    kind: blocks
  - dependency: ticket.document-graph-contract
    dependent: ticket.document-transform-traits
    kind: blocks
  - dependency: ticket.document-transform-traits
    dependent: ticket.markdown-to-document-graph
    kind: blocks
  - dependency: ticket.document-transform-traits
    dependent: ticket.code-to-document-graph
    kind: blocks
  - dependency: ticket.code-to-document-graph
    dependent: ticket.basin-document-graph-adapter
    kind: blocks
  - dependency: ticket.markdown-to-document-graph
    dependent: ticket.document-render-markdown
    kind: blocks
  - dependency: ticket.document-transform-traits
    dependent: ticket.latex-parser-contract
    kind: blocks
  - dependency: ticket.latex-parser-contract
    dependent: ticket.latex-to-document-graph
    kind: blocks
  - dependency: ticket.document-graph-schemas-cli
    dependent: ticket.latex-to-document-graph
    kind: blocks
  - dependency: ticket.latex-to-document-graph
    dependent: ticket.document-render-latex
    kind: blocks
  - dependency: ticket.document-render-markdown
    dependent: ticket.document-render-latex
    kind: blocks
  - dependency: ticket.document-graph-contract
    dependent: ticket.conditional-obligation-model
    kind: blocks
  - dependency: ticket.document-graph-schemas-cli
    dependent: ticket.conditional-obligation-model
    kind: blocks
  - dependency: ticket.conditional-obligation-model
    dependent: ticket.conditional-obligation-extraction
    kind: blocks
  - dependency: ticket.markdown-to-document-graph
    dependent: ticket.conditional-obligation-extraction
    kind: blocks
  - dependency: ticket.latex-to-document-graph
    dependent: ticket.conditional-obligation-extraction
    kind: blocks
  - dependency: ticket.document-render-latex
    dependent: ticket.transform-cli-registry
    kind: blocks
  - dependency: ticket.conditional-obligation-extraction
    dependent: ticket.transform-cli-registry
    kind: blocks
  - dependency: ticket.basin-document-graph-adapter
    dependent: ticket.transform-cli-registry
    kind: blocks
  - dependency: ticket.transform-cli-registry
    dependent: ticket.cross-format-golden-matrix
    kind: blocks
  - dependency: ticket.cross-format-golden-matrix
    dependent: ticket.document-graph-docs-migration
    kind: blocks
```
