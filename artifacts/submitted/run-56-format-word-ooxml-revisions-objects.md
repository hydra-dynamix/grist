# Run 56 — Word revisions and rich objects

Work item: `format-word-ooxml-revisions-objects`

## Implemented behavior

- Added an authoritative source-ordered revision graph covering content,
  move/range, property, table/grid/row/cell, section, and numbering revisions.
  Native IDs, authors, dates, nesting, normalized namespace-preserving XML,
  move/range edges, exact object/XML locators, and malformed-pair diagnostics
  are retained.
- Added three explicit deterministic story projections. `original` retains all
  source alternatives, `accepted` removes deletion/move-source content, and
  `rejected` removes insertion/move-destination content. No implicit view is
  selected; all three remain attached to the graph root.
- Preserved classic and modern comments with content, authors, dates, initials,
  durable IDs, resolved state, range/reference anchors, parent replies, and
  reply edges. Orphan anchors, replies, and extension parents are named losses.
- Preserved nested content controls and data bindings, OMML equations,
  DrawingML/VML objects, image/chart relationships, chart XML/type/title/text,
  captions, text boxes, DrawingML and VML alt text, and OLE metadata.
- Linked OLE and package relationships to content-addressed child artifacts.
  Embedded packages remain inventory-only by default and are quarantined when
  inline bytes are requested, ready for explicit shared-budget recursion rather
  than execution or automatic materialization.
- Added normalized `DocumentGraph` nodes and explicit/inferred relations for
  revisions, replies, controls, equations, figures, charts, captions, text
  boxes, and embedded objects, all with exact locators.
- Regenerated Word payload/envelope schemas and canonical examples, and updated
  the Word format contract.

## Focused coverage

The synthetic rich DOCX fixture combines insert/delete/move/property/range
revisions, original/accepted/rejected views, comment anchors and a modern reply,
a tagged/data-bound control, OMML, a chart and caption, DrawingML alt text, a
VML text box, and a quarantined embedded workbook. A malformed variant verifies
that an unpaired move returns `partial` with a stable diagnostic while retaining
the authoritative source node.

## Validation evidence

- Focused Word package/rich-content/graph/schema tests: pass (7 tests without
  default features; 8 tests in the CLI matrix).
- Generated schema and canonical-example drift tests: pass.
- Complete `cargo test --offline --features cli` matrix and doctests: pass.
- Clippy across all CLI-feature targets with warnings denied: pass.
- Minimal no-feature and isolated `word-ooxml` checks: pass; only the existing
  unrelated summary dead-code warning remains.
- Documentation build: pass; only the three existing unrelated graph rustdoc
  invalid-HTML-tag warnings remain.
- Rust formatting and scoped trailing-whitespace checks: pass.

Development-only validation failures were corrected and recorded in LDGR: an
invalid feature-name invocation, the expected quarantine status for an embedded
package, inaccessible default Cargo registry extraction, command-line patch
quoting, and one new plus one pre-existing Clippy finding.
