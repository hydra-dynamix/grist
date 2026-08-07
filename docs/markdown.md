# Markdown parser contract

The `markdown` feature exposes `grist::formats::markdown` and the compatible
`grist::markdown` path. Its authoritative payload schema is
`grist/markdown/v2`; the checked envelope uses `grist/envelope/v2`.

## Input and options

`parse_markdown_bytes` is the primary API. It uses the shared decoder, retains
the exact input bytes, decoded text, encoding evidence, decoded-to-raw byte
mapping, identity hashes, and decoding diagnostics. `parse_markdown` remains a
source-compatible UTF-8 convenience API.

`MarkdownOptions` independently controls the caller-supplied encoding label,
GFM behavior, tables, footnotes, task lists, strikethrough, math, heading
attributes, definition lists, frontmatter, and extension retention. Defaults
enable the full deterministic built-in surface. Unknown option fields are
rejected.

## Preserved syntax

The typed node tree preserves source/preorder hierarchy and exact decoded and
raw-byte ranges for:

- headings and attributes, paragraphs, text, soft/hard breaks, and thematic breaks;
- emphasis, strong, strikethrough, inline code, inline/display math, and HTML;
- links, reference-link metadata, images, and titles;
- ordered/unordered lists, start numbers, items, and checked/unchecked task markers;
- block quotes, fenced/indented code, language and full info strings;
- GFM tables with alignments and located rows/cells;
- footnote definitions/references and definition lists;
- YAML (`---`/`...`) and TOML (`+++`) frontmatter as exact raw plus structured values;
- MyST-style fenced directives, colon-fenced directives, wiki links, shortcodes,
  templates, and other retained raw block/inline extensions.

Directives, HTML, code, links, and extension bodies are inert data. Grist never
executes them, fetches a link or image, expands a shortcode, or performs network
access.

## Status and diagnostics

Malformed but recoverable input returns `partial` with the payload retained.
Stable Markdown diagnostic codes include:

- `frontmatter.parse` and `frontmatter.unclosed`;
- `fence.unclosed`;
- `directive.unclosed`;
- shared `decode.*` and resource-budget diagnostics.

Every diagnostic tied to source syntax has an exact locator. A decode failure
or exhausted budget returns `failed` rather than an empty successful document.

## Projections

`ToDocumentGraph` deterministically maps the authoritative hierarchy to the
v2 graph vocabulary. Explicit links and footnote references carry located
relations. HTML, directives, and unknown extensions become located `raw_block`
or `raw_inline` nodes with the original syntax in `grist.markdown` raw content;
format-specific node data is also retained in the namespaced extension map.

The shared structural segmenter preserves node IDs and locators. Normalized
rendering supports strict Markdown for typed syntax, explicit inert
`raw_fallback` markers for retained extensions, and gap-free generated source
maps. This is normalized rendering, not a claim of byte-identical source
reconstruction.

## Verification

`tests/markdown_universal_contract.rs` covers the authoritative payload,
frontmatter variants, malformed encoding/syntax, raw extensions, graph edges,
segments, render fidelity/source maps, and schema agreement.
`tests/markdown_promotion.rs` evaluates the shared 11-gate parser promotion
harness, including valid/mislabeled/extensionless/malformed/ambiguous detection,
all operation statuses, hostile input, fixture governance, CLI parity, and
minimal-feature isolation. Governed fixtures live under
`fixtures/generated/markdown/` with synthetic Apache-2.0 provenance.
