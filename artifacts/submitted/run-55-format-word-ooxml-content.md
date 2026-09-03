# Run 55 — WordprocessingML content

Work item: `format-word-ooxml-content`

## Implemented behavior

- Added producing-order WordprocessingML stories for the main body, footnotes,
  endnotes, headers, and footers while reusing the bounded, inert OPC package
  traversal from run 54.
- Preserved paragraphs, runs, run contents, visible text, named paragraph/run
  styles, style inheritance, direct and effective formatting, headings,
  line/page/column breaks, section breaks, columns, page geometry, and
  header/footer references.
- Preserved original abstract numbering, levels, instances, level overrides,
  and paragraph `numId`/level references, and emitted deterministic resolved
  labels and ordinals without replacing the original definitions.
- Preserved table grids, widths, layouts, header rows, grid spans,
  vertical/horizontal merge origins and continuations, and recursively nested
  tables.
- Preserved relationship-backed hyperlinks, anchors, bookmarks, simple and
  complex fields, result runs, native citation fields, cross-references,
  footnote/endnote references, notes, headers, and footers.
- Added exact nested `OoxmlPart` plus `XmlPath` locators with one-based
  paragraph, run, table, row, and column coordinates and stable object IDs.
- Extended DocumentGraph projection with headings, paragraphs, text runs,
  lists/items, tables/rows/cells, links, bookmarks, fields, citations,
  references, sections, notes, headers, and footers. Explicit link/reference
  edges, segmentation locators, normalized rendering, and source maps use the
  same producing hierarchy.
- Regenerated Word payload/envelope schemas and the full-feature canonical
  example manifest, and updated Word format and gap-matrix documentation.

## Focused coverage

The synthetic complex DOCX fixture covers inherited and direct formatting,
heading recognition, external links, bookmarks, simple citation and complex
cross-reference fields, numbering start overrides, page breaks, note
references, two-column sections, merged/header rows, a nested table, footnotes,
endnotes, headers, footers, exact semantic locators, graph vocabulary,
segmentation, rendering, schemas, determinism, and CLI parity. Existing package,
macro, hostile XML/ZIP, encrypted, budget, and variant tests remain passing.

## Validation evidence

- Isolated Word/content/graph/schema compile: pass.
- Focused Word test without default features: pass.
- CLI Word and module topology tests: pass (6 and 15 tests).
- Full generated schema/canonical-example drift check: pass.
- Clippy across all CLI-feature targets with warnings denied: pass with only
  the established graph-helper argument-count allowance.
- Complete `cargo test --offline --features cli` matrix and doctests: pass.
- Minimal no-feature and isolated Word-feature checks: pass with only the
  existing unrelated `summary::profile_name` warning.
- Documentation build: pass with only three existing unrelated graph rustdoc
  invalid-HTML-tag warnings.
- Rust formatting, scoped trailing-whitespace scan, and temporary-generator
  cleanup: pass.

LDGR records the diagnostic development failures and their corrected reruns:
Windows patch argument quote stripping, a minimal-feature canonical manifest,
Clippy type-size findings, and a reversible documentation encoding rewrite.
