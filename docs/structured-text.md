# Structured text data

The `serialization` feature exposes the authoritative
`grist/structured-text/v2` payload for JSON, JSONL/NDJSON, YAML, TOML, and a
loss-aware projection of the generic XML parser. The compatibility module is
`grist::serialization`; the canonical namespace is
`grist::formats::structured_text`.

## Ordering and types

`StructuredTextDocument.ordering` is `source`. Documents, JSONL records,
mapping entries, sequence items, XML attributes, and mixed XML content are
serialized in source order. Objects are represented as an entry vector rather
than a map, so duplicate keys cannot disappear. Every value carries its exact
source spelling, UTF-8 byte/line/column range, stable structural path, and a
`JsonPointer` or `XmlPath` locator. JSONL values additionally carry a one-based
`RecordRange`.

Scalars retain null, boolean, arbitrary-size integer spelling, finite or
non-finite float spelling, string, date, time, and date-time distinctions.
`payload.value` remains a compatibility JSON projection; the typed
`documents` and `records` trees are authoritative whenever JSON cannot express
a source type.

## Duplicate keys and YAML aliases

JSON and YAML duplicate entries are always retained with a
`duplicate_ordinal` and paired first/duplicate locators. JSON duplicates are
valid loss-aware output. YAML duplicates produce a partial diagnostic because
the YAML mapping-key uniqueness contract is violated. TOML duplicates are
rejected by the TOML 1.0 parser and receive an exact malformed-input
diagnostic.

YAML anchors remain annotations on their defining nodes. Aliases remain
reference nodes with a target ID; Grist never recursively expands them. Merge
keys are consequently preserved as ordinary source entries rather than
silently changing mapping contents. Missing anchors are partial diagnostics.
Tags and multi-document stream order are retained.

## Records, recovery, and limits

`stream_jsonl` is a lazy iterator over physical non-blank records. A malformed
record is emitted with its raw bytes and diagnostic, and later records remain
available. Cancellation and record/node/nesting budgets produce exactly one
terminal event; a limit is never reported as end-of-input.

Single-document syntax uses `SerializationOptions.malformed_recovery`:

- `strict` returns `failed` without fabricating a payload.
- `preserve_raw` returns `partial` with a `RawStructuredUnknown` covering the
  retained source.

`max_nesting_depth` is an explicit parser-stack safety ceiling for legacy
convenience calls. Registry parsing also applies the shared `ResourceBudget`
and uses the lower ceiling.

## XML security

XML is parsed by the namespace-aware `grist.xml` engine and then projected to
structured values. Attribute order, mixed content, comments, processing
instructions, qualified names, namespaces, raw unknowns, and exact XML paths
survive. External entities, remote schemas, XInclude fetching, scripts, and
active content never execute or access the network. Security and malformed
recovery diagnostics from the XML envelope are retained unchanged.

## Graph, schema, and CLI

Objects/arrays/scalars project to `StructuredValue`, mappings to ordered
`Field` children, JSONL lines to `Record`, and malformed regions to `Raw`
nodes. All projected source nodes retain locators and therefore remain
segmentable and citation-ready.

Checked contracts are:

- `schemas/grist.structured-text.v2.schema.json`
- `schemas/grist.structured-text-options.v2.schema.json`
- `schemas/grist.serialization-envelope.v2.schema.json`

Registry and CLI selectors are `json`, `jsonl` (alias `ndjson`), `yaml`,
`toml`, and `xml`. CLI routing uses the same library parsers and options.
