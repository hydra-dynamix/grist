# Presentation OOXML package parsing

The presentation-ooxml feature provides a bounded, read-only parser for PPTX,
PPTM, POTX, and PPSX OPC packages. It is included by the presentations, full,
and cli feature gates.

The authoritative grist/presentation-ooxml/v1 payload preserves:

- the content-type manifest, every ZIP member in package order, compression
  metadata, part hashes, and exact OOXML-part locators;
- package and part relationships with root-bounded target resolution,
  dangling-target state, external targets as inert strings, and duplicate-ID
  diagnostics;
- core, extended, and typed custom document properties;
- presentation-native slide IDs and producing order, including hidden-slide
  state and stable content identities for resolved slide parts;
- slide masters, layouts, and themes with their native or relationship IDs,
  source parts, resolved parts, hashes, and locators;
- click, hover, action, and hyperlink inventory without navigation, launch, or
  network behavior;
- VBA projects and embedded media/objects as EmbeddedArtifact values.

Each resolved slide additionally has an authoritative `slide_contents` entry.
It preserves the nested shape tree and source z-order; raw EMU transforms,
rotation, flips, preset/custom geometry, placeholders, names, hidden and
decorative state; alternative text; text paragraphs and runs with language,
font, emphasis, color, fields, breaks, and hyperlinks; tables with grid sizes,
row heights, spans, and merge state; cached chart titles, types, series,
categories, and values; Office Math; pictures and crop metadata; notes and
classic or modern comments; transitions and the complete timing subtree as
inert metadata; and linked embedded objects.

Reading order is a distinct per-slide projection. The deterministic v1 rule
places title placeholders first, then uses top-to-bottom and left-to-right
geometry with shape-tree order as a tie-breaker. Every entry records the
evidence and a bounded confidence. Shapes without usable geometry remain in
source order with lower confidence; they are never presented as source-native
reading order.

VBA bytes are inventory-only by default. When explicitly requested they remain
classified as macros and their extraction state is quarantined; they are never
loaded or executed. Embedded executable-looking objects are likewise
quarantined. External relationships are never fetched.

## Failure and safety behavior

OLE encrypted OOXML and encrypted ZIP members return encrypted without a
payload. Missing main parts, invalid root relationships, and unreadable
manifests fail explicitly. Malformed secondary XML, dangling or unsafe
relationships, duplicate paths, and rejected archive members produce partial
payloads with machine-readable diagnostics.

Traversal uses the shared archive member, expansion ratio, memory, child
artifact, node, cancellation, and time budgets. Member paths are checked for
absolute paths, traversal, symbolic links, cross-platform collisions, and
duplicate ambiguity before bytes are read. XML DTDs, entities, and other active
constructs are rejected by the shared XML security policy.

## Graph, schema, and CLI

The graph projection emits a document root, all package parts, ordered slide
nodes, shapes, paragraphs and text runs, tables/rows/cells, charts, equations,
images, notes, comments, links, timing metadata, embedded objects, properties,
attachments, and relationship edges. Reading-order `precedes` edges are
explicitly marked inferred, include the rule and confidence, and cite both
shape locators. Stable IDs use native slide and shape IDs or part paths together
with source identity and exact locators. Normalized rendering and structural
segmentation therefore retain slide boundaries and citation-ready locators.

Public schemas are registered as presentation-ooxml,
presentation-ooxml-envelope, and presentation-ooxml-options. The CLI routes
grist parse pptx|pptm|potx|ppsx PATH through the same registry parser and
supports normal graph, rendering, and segmentation operations.
