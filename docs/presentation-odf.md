# OpenDocument presentation parsing

The presentation-odf feature provides inert, bounded parsing for ODP presentations and OTP presentation templates. The authoritative payload schema is grist/presentation-odf/v1; both formats use the common envelope v2.

## Package and detection contract

Detection combines the ZIP signature with the required uncompressed mimetype member. The exact ODP and OTP media types route independently through the built-in registry, including for extensionless inputs. A contradictory requested selector and package mimetype fails explicitly.

ZIP traversal uses the shared archive security policy and resource budget. Every package member retains its zero-based archive-member locator, sizes, CRC, compression, content identity when readable, manifest media type, encryption state, and rejection code. Traversal, absolute paths, links, normalized collisions, expansion limits, and archive-member limits cannot escape or masquerade as complete output.

content.xml and mimetype encryption produce encrypted without a payload. Other encrypted members remain inventoried and make the result partial. Malformed recoverable XML, rejected members, and unsupported secured XML are diagnosed as partial; a missing safe core member or invalid ZIP fails.

## Authoritative content

The parser preserves:

- metadata, settings, named/default/automatic styles, style properties, page layouts, presentation placeholders, master pages, and master shapes;
- source-ordered slides with native IDs, names, master/layout/style references, visibility, exact XML/package locators, and one-based slide-region locators;
- nested drawing shapes, source z-order, parent groups, raw attributes/XML, style/class/layer references, SVG-compatible geometry, transforms, paths, and point-normalized bounding boxes;
- paragraphs and runs, whitespace controls, spans, headings, styling, text links, titles, descriptions, and speaker notes;
- annotations/comments, repeated and merged table cells, typed/stored values, formulas as inert source text, embedded chart metadata and series references, images, and accessibility text;
- hyperlinks and actions without resolving them, transitions, SMIL/animation trees as metadata, and unknown slide/root elements as raw XML;
- embedded package members as content-addressed artifact identities with passive/active/executable safety classification and inventory-only bytes by default.

inline_embedded_artifact_bytes opts into inline capture. It does not execute, render, activate, fetch, or materialize any embedded object.

## Reading order and graph projection

Slide order is source-explicit. Within each slide, title and subtitle placeholders are prioritized, then positioned shapes sort top-to-bottom and left-to-right, with source z-order as fallback and tie-break. Every inferred entry and Precedes edge records confidence, evidence, and exact shape locators.

The DocumentGraph projection retains slides, nested shapes, paragraphs/runs, notes, comments, tables/rows/cells, charts, images, links, transitions, animations, attachments, metadata, and package parts. Parser-specific details remain under grist.presentation_odf. Normalized renderers and the deterministic segmenter therefore preserve slide text and locators without replacing the authoritative package model.

## Diagnostics

Stable parser diagnostics include:

- presentation_odf.encrypted.core_member
- presentation_odf.encrypted.content
- presentation_odf.encrypted.member
- presentation_odf.styles.empty
- shared malformed XML, XML security, archive security, cancellation, and budget diagnostics

External links, animation commands, plugins, applets, OLE objects, executable bytes, and embedded documents always remain inert data. The parser performs no network access and invokes no provider.
