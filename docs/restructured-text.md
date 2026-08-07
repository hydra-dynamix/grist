# reStructuredText

The `restructured-text` feature exposes both
`grist::formats::restructured_text` and the compatible
`grist::restructured_text` path. The payload schema is
`grist/restructured-text/v1`; parse results use `grist/envelope/v2`.

`parse_restructured_text_bytes` is the authoritative entry point. It retains
the original bytes, decoded text, decoding report, content identity, exact raw
and decoded ranges, and source locators. The registry selector is
`restructured-text`; `rst`, `rest`, and `restructured_text` are aliases.

## Syntax and projections

The parser recognizes adornment headings, directives, interpreted roles,
include references, grid and simple tables, directive and literal code,
doctest blocks, footnotes, citations, hyperlink targets, named references,
substitutions, lists, fields, transitions, comments, and inline emphasis,
strong text, and literals. Unknown directives and roles remain typed, inert
nodes with their exact raw syntax; otherwise malformed explicit markup falls
back to `raw_block` or `raw_inline` with a partial diagnostic. Directive bodies
are data: Grist never invokes directive handlers or executes embedded code.

Every node contains its original syntax and exact locator. DocumentGraph v3
projection uses the `grist.restructured_text` namespace, preserves raw syntax,
emits reference/link relations, and connects resolved include documents with
`resolved_to` relations. Structural segmentation and normalized rendering use
that same graph, so typed payload, graph, segment, schema, and CLI views share
identities and locators.

## Local includes

Includes are references unless the caller explicitly supplies
`RestructuredTextOptions::project_root`. Resolution then canonicalizes both the
root and candidate, requires a regular file below that root, and applies depth
and byte limits. Recursive includes are cycle checked. Parent traversal,
canonical paths outside the root, missing files, invalid roots, and over-budget
files remain inert references with diagnostics.

HTTP, HTTPS, protocol-relative, `file:`, `data:`, and other URI-like targets
are never fetched. Representative diagnostic codes are
`include.project_root_required`, `include.remote_disabled`,
`include.outside_project_root`, `include.cycle`, and
`include.budget_exceeded`.

## Verification fixtures

`tests/restructured_text_universal_contract.rs` covers typed parsing, source
maps, graph/segment/schema agreement, detection, budgets, decoding, and bounded
includes. `tests/restructured_text_promotion.rs` applies the shared eleven-gate
promotion harness. Deterministic corpus cases live under
`fixtures/generated/restructured_text/` with recipes and provenance in the
corpus manifests.
