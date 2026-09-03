# HTML5 and XHTML contract

The html feature exposes the authoritative grist/html/v2 payload for HTML5
documents, HTML fragments, and XHTML. The backend is html5ever 0.29.1 using
WHATWG tree construction. Grist adds a lossless lexical stream, byte-decoding
evidence, semantic indexes, provenance, and an inert-content policy.

## Inputs and options

parse_html_bytes is the primary API. It retains original bytes and selects an
encoding from BOM, transport MIME parameters, HTML meta charset declarations,
or XML declarations. Conflicts and replacement decoding use the shared decode
diagnostics. parse_html is the UTF-8 convenience API.

HtmlOptions selects document, fragment, or automatic mode; HTML5, XHTML, or
automatic syntax; an optional caller encoding; the HTML fragment context
element; and comment retention. Automatic syntax selects XHTML for
application/xhtml+xml, XML declarations, or the XHTML namespace. Automatic
mode selects document parsing for a doctype, html root, or XML declaration.

The registry aliases are html, htm, and xhtml; recognized media types are
text/html and application/xhtml+xml.

## Authoritative payload

HtmlDocument.nodes is the recovered DOM in depth-first order. Every node has a
parent, ordered children, a deterministic DOM path, namespace, original and
normalized element names, ordered attributes, source range, nested text/XML-path
locator, exact raw source when present, and an explicit synthetic marker for
implied recovery nodes. SVG, MathML, XML, XMLNS, and XLink namespaces survive
the tree projection.

source_tokens independently preserves exact source order, including start and
end tags, whitespace text, comments, processing instructions, doctypes, unknown
declarations, attribute spelling, quoting, and raw byte ranges. This stream is
authoritative for syntax that HTML5 recovery intentionally rewrites. Unknown
custom elements remain DOM elements with known_element set to false and exact
raw source.

The payload also provides deterministic indexes for:

- document metadata, title, language, direction, base URI labels, and meta data;
- links and URL-bearing attributes, including remote and active-scheme flags;
- tables, rows, cells, header state, and row/column spans;
- image, picture, audio, video, track, SVG, canvas, object, embed, and iframe
  references with type and alternative text;
- semantic sections and heading labels;
- HTMX attributes; and
- scripts, event handlers, JavaScript URLs, forms, refresh directives, styles,
  browsing contexts, and plugin objects classified as inert active content.

## Malformed input and XHTML

HTML5 tree-builder errors emit html.tree_recovery, retain the recovered payload,
and make status partial. Implied nodes carry approximate locators; source-backed
nodes and tokens carry exact locators. Lexically truncated tags, comments, and
raw-text elements have specific diagnostics.

XHTML adds well-formedness checks for namespace declaration, exact tag matching,
closed elements, unique attributes, and quoted values. XML processing
instructions remain source tokens. External doctype identifiers are retained
with xhtml.external_identifier_inert; no external subset is resolved. The HTML5
projection may report non-lossy informational adjustments while XHTML
well-formedness determines partial status.

## Security

Parsing performs no network, script, style, form, media, plugin, iframe, HTMX,
or event-handler action. URI values are labels only. No browser, JavaScript
engine, CSS engine, form client, resource loader, XML entity resolver, schema
resolver, or execution provider is called. Active constructs remain typed and
raw with disposition inert.

## Graph, rendering, and segmentation

ToDocumentGraph maps DOM containment and order to deterministic graph nodes,
retains the full HTML node under the grist.html extension, creates explicit link
relations, and maps semantic sections, headings, text, links, lists, tables,
figures, media, forms, metadata, and unknown elements. Unknown elements carry
RawNodeContent. The shared segmenter and normalized HTML/Markdown/LaTeX/text
renderers consume that graph and retain source locators and source maps.
Normalized rendering is not byte reconstruction.

## Schemas, fixtures, and diagnostics

Checked schemas are grist.html.v2.schema.json,
grist.html-envelope.v2.schema.json, and grist.html-options.v1.schema.json.
Fixtures under fixtures/generated/html cover maximum complexity,
malformed/adversarial recovery, XHTML regression, and malicious active content.
The focused universal contract and promotion suite are
tests/html_universal_contract.rs and tests/html_promotion.rs.

Stable parser-specific explanation keys include:

- html.tree_recovery
- html.markup_unclosed
- html.comment_unclosed
- html.raw_text_unclosed
- html.document_missing_doctype
- html.active_content_inert
- xhtml.namespace_missing
- xhtml.attribute_unquoted
- xhtml.attribute_duplicate
- xhtml.element_mismatch
- xhtml.end_tag_unmatched
- xhtml.element_unclosed
- xhtml.external_identifier_inert
