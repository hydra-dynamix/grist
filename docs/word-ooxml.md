# Word OOXML package and content parsing

The `word-ooxml` feature implements bounded, inert OPC and WordprocessingML
parsing for DOCX, DOCM, DOTX, and DOTM. Package structure remains authoritative
alongside the producing-order content model and an authoritative rich-content
model. No tracked-change decision is made while parsing.

## Detection and dispatch

The registry exposes four exact selectors:

| Selector | Main-part content type |
|---|---|
| `docx` | `application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml` |
| `docm` | `application/vnd.ms-word.document.macroEnabled.main+xml` |
| `dotx` | `application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml` |
| `dotm` | `application/vnd.ms-word.template.macroEnabledTemplate.main+xml` |

Detection reads the bounded `[Content_Types].xml` member, so extensionless
packages and macro/template variants retain their exact identity. A selector
whose declared kind does not match the package fails explicitly.

## Authoritative payload

`WordOoxmlDocument` records:

- every ZIP member in package order, including compression, sizes, CRC-32,
  content type, identity, availability or rejection status;
- all content-type defaults and overrides;
- every root and per-part relationship, its raw target, internal/external mode,
  root-bounded resolved target, and whether the target exists;
- core, extended, and typed custom properties with namespaces, attributes, and
  exact source locations;
- a VBA-project inventory and embedded/media child-artifact inventory.
- paragraph and run order, paragraph/run styles, direct formatting, resolved
  style inheritance, headings, line/page/column breaks, and section columns;
- original abstract numbering, instances and overrides plus deterministic
  resolved labels and ordinals on each numbered paragraph;
- table grids, widths, layouts, header rows, grid spans, vertical/horizontal
  merge origins and continuations, and recursively nested tables;
- hyperlinks, bookmarks, simple and complex fields, cross-references, native
  citation fields, footnotes, endnotes, headers, and footers.
- a source-ordered revision graph for insertions, deletions, moves, range
  markers, and run/paragraph/section/table/row/cell/numbering property changes;
- classic and modern threaded comments, durable IDs, reply/resolution state,
  range/reference anchors, content controls and data bindings;
- namespace-preserving OMML equations, DrawingML and VML objects, images,
  charts, inferred caption associations, text boxes, alternative text, and OLE
  metadata linked to content-addressed child artifacts.

## Revision projections

The revision graph, including normalized XML subtrees, authors, dates, native
IDs, nesting, move pairs, range pairs, and exact locators, is authoritative.
All text projections are named and emitted together; there is no default
accepted or rejected view:

- `original` follows XML source order and includes every tracked alternative;
- `accepted` removes deletion and move-source subtrees;
- `rejected` removes insertion and move-destination subtrees.

Each story projection names its part and the revision node IDs from which it
was derived. Malformed missing IDs, unpaired moves/ranges, orphan comment
anchors/replies, and unresolved chart parts produce stable partial-result
diagnostics while retaining every structurally available subtree.

Every package member uses an exact `OoxmlPart` locator. XML declarations and
WordprocessingML constructs add an exact `XmlPath` component. Paragraph,
run, table, row, column, and native object coordinates are one-based and
retained in the OOXML component. Rejected names that cannot be represented
safely receive a stable package-index fallback locator.

## Security and budgets

ZIP traversal is read-only and charged to the shared archive-member,
expansion-ratio, memory, child-artifact, node, input, output, and time budgets.
Absolute, traversal, device, control-character, duplicate-normalized, link, and
special-file members are rejected without extraction. Internal relationship
targets are percent-decoded and normalized within the package root. External
relationships are retained but never fetched; unknown target modes are treated
as external.

Metadata XML is inspected before parsing. DTDs, entity declarations, XInclude,
and remote schema locations are rejected and reported. Encrypted ZIP members
and OLE-wrapped encrypted OOXML return `encrypted`; malformed required package
structure returns `failed`; rejected optional metadata or hostile optional
members produce `partial` with named diagnostics.

Macros are never executed or interpreted. By default, VBA projects are
inventory-only. Setting `extract_macro_bytes` retains their bytes only as
quarantined embedded artifacts. This applies even when macro content is hidden
inside a nominally macro-free package.

Child media and embedded objects are inventory-only by default. Setting
`inline_child_artifact_bytes` embeds bytes subject to the shared artifact
safety classification; nested packages and unsafe children remain quarantined
for caller-controlled recursive parsing under the shared container budget.

## Public integration

The package payload, options, and envelope use
`grist/word-ooxml/v1` schemas. The CLI supports
`grist parse docx|docm|dotx|dotm PATH`. Its DocumentGraph projection includes
package parts and metadata together with headings, paragraphs, runs, lists,
tables, links, bookmarks, fields/citations, notes, sections, headers, and
footers, revisions, comments/replies, controls, equations, figures, charts,
captions, text boxes, and embedded objects. Explicit relationships and exact
source locators survive common segmentation and normalized rendering. The
graph root retains all three named revision projections so graph conversion
cannot silently select one.

Focused fixtures cover all four variants, selector mismatch, exact locations,
metadata, external and unsafe relationships, macro quarantine, embedded child
artifacts, malformed and encrypted packages, active XML, deterministic output,
budget exhaustion, complex style inheritance, numbering overrides, fields,
citations, merged and nested tables, sections, notes, headers/footers,
insert/delete/move and property/range revisions, original/accepted/rejected
views, threaded comments, controls/data bindings, OMML, DrawingML, charts,
captions, text boxes, alt text, OLE/package artifacts, explicit loss
diagnostics, graph/segment/render/schema projection, and CLI parity.
