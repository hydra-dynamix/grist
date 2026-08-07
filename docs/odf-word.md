# OpenDocument text parsing

The `odf-word` feature implements bounded, inert parsing for ODT documents and
OTT templates. It retains the ZIP package, XML structure, semantic document
model, revision alternatives, and named text projections together so no view
silently replaces the source representation.

## Detection and dispatch

The registry exposes `odt` for
`application/vnd.oasis.opendocument.text` and `ott` for
`application/vnd.oasis.opendocument.text-template`. Detection inspects the
bounded `mimetype` member, including for extensionless input, and an explicit
selector/package-kind mismatch fails. Both formats are enabled by
`word-processing`, `full`, and `cli`.

## Authoritative payload

`OdfWordDocument` records:

- package-order ZIP members, compression and size metadata, manifest media
  types, checksums, encryption declarations, rejected names, and exact member
  locators;
- Dublin Core and OpenDocument metadata plus arbitrary typed user properties,
  view settings, font faces, named and automatic styles, list definitions,
  master pages, headers, and footers;
- source-ordered paragraphs, headings, spans, sections, lists and list items,
  tables, rows and cells, repeated and covered cells, spans, nested tables,
  links, bookmarks, notes, annotations, and unknown XML subtrees;
- drawings, frames, images, text boxes, MathML equations, and embedded object
  references linked to content-addressed child artifacts; and
- tracked change regions, authorship and dates, deleted content, range anchors,
  and explicit visible, original, accepted, and rejected text projections.

The recursive structural tree is authoritative. Unknown elements retain their
qualified name, attributes, raw XML, ordered children, and exact XML path.
Revision projections are derived views: `visible` follows the current body,
`accepted` keeps insertions and omits deletions, while `original` and
`rejected` omit insertions and reinstate preserved deletion content. Malformed
or unpaired revision anchors are retained and produce stable partial-result
diagnostics.

## Security, budgets, and provenance

Package traversal uses the shared archive policy and never extracts members to
the filesystem. Absolute, traversal, device, control-character,
duplicate-normalized, link, and special-file entries are rejected. Every safe
member has an `ArchiveMember` locator; parsed XML constructs add exact byte
ranges and `XmlPath` components.

DTD, entity, XInclude, and remote-schema constructs are rejected before XML is
interpreted. External links are retained but never fetched, macros or embedded
content are never executed, and child artifacts are inventory-only unless the
caller explicitly requests bounded byte capture. Content-addressed artifact
records carry media type, safety classification, package locator, and optional
bytes for caller-controlled recursive parsing.

Encrypted `content.xml` or `mimetype` members return `encrypted`. Other
encrypted members remain inventoried and make the result explicitly `partial`.
Malformed required structure returns `failed`; rejected optional package
members and recoverable malformed optional XML remain visible through named
diagnostics. Input, output, archive-member, expansion-ratio, memory, node,
nesting, child-artifact, and time budgets use the shared request ledger.

## Public integration

The payload, envelope, and options schemas are `grist/odf-word/v1`,
`grist/envelope/v2`, and `grist/odf-word-options/v1`. The CLI supports
`grist parse odt|ott PATH`. DocumentGraph projection retains package,
metadata, style, structure, revision, link, annotation, drawing, equation, and
embedded-object information with explicit relations and source locators.
Shared segmentation and normalized rendering operate on the same deterministic
graph while all four named text views remain available on the graph root.

Focused tests cover rich ODT and OTT packages, exact detection and selector
matching, deterministic serialization, revisions and projections, tables,
notes, annotations, drawings, equations, embedded artifacts, graph/segment/
render/schema integration, encryption, hostile paths, malformed XML, and
budget exhaustion.
