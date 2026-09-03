# AsciiDoc

The `asciidoc` feature exposes `grist::formats::asciidoc` and the compatible
`grist::asciidoc` path. The authoritative payload is
`grist/asciidoc/v1`; parse results use `grist/envelope/v2`. Registry and CLI
selectors accept `asciidoc`, `adoc`, and `asc`. Recognized extensions are
`.adoc`, `.asciidoc`, and `.asc`; recognized media types are `text/asciidoc`
and `text/x-asciidoc`.

`parse_asciidoc_bytes` is the authoritative entry point. It retains original
bytes, decoded text and encoding diagnostics, exact decoded/raw ranges, source
locators, deterministic node IDs, and content identity.

## Supported syntax

The parser recognizes `=` document/section headings, document attributes and
unsets, block attribute lists and roles, block macros, anchors, `xref:` and
`<<...>>` cross-references, links, attribute references, footnotes, lists,
comments, horizontal transitions, pipe tables, source/listing/literal and other
delimited blocks, inline roles, emphasis, strong text, and inline code.

`include::target[options]` is a typed reference. Known inert media macros such
as `image::`, `audio::`, and `video::` are typed directives. Unknown block and
inline macros survive as raw nodes with their exact syntax and an informational
diagnostic. Malformed attributes, tables, and unclosed blocks survive as raw
nodes and make the envelope partial. Code, passthrough blocks, macros, and
attributes are data only: the parser never executes or expands them.

The `DocumentGraph` projection uses the `grist.asciidoc` extension namespace.
It preserves typed payload nodes and raw syntax, emits link/reference relations,
projects tables and code structurally, and connects resolved include documents
with `resolves_to`. Segmentation consumes this graph while retaining node IDs
and locators.

## Explicit local includes

Includes remain references unless the caller supplies
`AsciiDocOptions::project_root` and leaves `resolve_includes` enabled. The
resolver canonicalizes the root and candidate, requires a regular file below
the root, detects recursion cycles, and enforces depth and per-file byte limits.
Parent traversal, canonical paths outside the root, invalid roots, missing
files, non-files, decode failures, and budget violations remain inert references
with stable diagnostics.

HTTP, HTTPS, protocol-relative, `file:`, `data:`, and other URI-like targets are
never fetched. Representative codes include `include.project_root_required`,
`include.remote_disabled`, `include.outside_project_root`, `include.cycle`, and
`include.budget_exceeded`.

## Verification

`tests/asciidoc_universal_contract.rs` covers typed parsing, exact locators,
detection, decoding, budgets, graph/segment/schema agreement, and bounded
includes. `tests/asciidoc_promotion.rs` applies the shared eleven-gate harness.
Deterministic maximum-complexity, malformed/adversarial, and downstream
regression fixtures live under `fixtures/generated/asciidoc/` with builder and
corpus provenance.