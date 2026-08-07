# Deterministic structural segmentation

`grist::segment` projects a canonical `DocumentGraph` into citation-ready
segments. It does not select retrieval, ranking, embedding, or indexing policy.

## Public entry points

- `segment_document_graph` performs serial preparation and canonical assembly.
- `segment_document_graph_parallel` prepares independent nodes with the
  requested worker count, discards completion order, and uses the same canonical
  assembly path. Its serialized result is identical to the serial result.
- `SegmentTokenizer` admits caller-supplied deterministic tokenizers. The
  configured name, version, and SHA-256 configuration digest must exactly match
  the supplied implementation. `UnicodeWhitespaceTokenizer` is the built-in
  default.

Both entry points require source and document `ContentIdentity` values.
Default or hashless identities are rejected instead of fabricating provenance.

## Boundary and size behavior

`SegmentOptions` selects byte, Unicode scalar, or token sizing and supplies a
target and maximum. Document, section, paragraph, list item, table row, code
symbol, page, slide, sheet, message, notebook-cell, and transcript-cue
boundaries are configurable.

The engine never cuts through a source node because doing so would make the
node's locator broader than the emitted text. Tables, code structures,
equations, and figure-caption pairs can additionally be indivisible structural
units. If one indivisible unit, required ancestry, or requested node overlap
exceeds the maximum, the engine preserves provenance and emits
`segment.maximum_exceeded_for_atomic_source` on the affected segment. It does
not silently split or discard source content.

Overlap is counted and represented as source nodes. The next segment reuses the
same node IDs and locators, marks their references with the `overlap` role, and
links back to the prior stable segment ID. Atomic groups remain whole when they
participate in overlap.

## Traceability invariant

Rendered segment text is the exact concatenation of contributing node text with
no generated separator. Every byte belongs to one `SegmentNodeReference`,
which records:

- the unchanged graph node ID and full `SourceLocator`;
- whether the node contributes ancestry, overlap, or primary content;
- its half-open UTF-8 byte range in the rendered segment.

Legacy `node_ids` and `locators` are ordered projections of these references.
A selected text-bearing node without a locator is omitted with the partial-loss
diagnostic `segment.untraceable_node_omitted`; untraceable text never enters a
segment.

Each segment also records byte, Unicode scalar, and token counts; source and
document identities; structural path; section and document-title context;
tokenizer, renderer, renderer digest, and options digest; deterministic metadata;
and any size or loss diagnostics. Segment IDs are domain-separated SHA-256
digests of canonical identity material rather than sequence numbers.

## Selection and metadata

Inclusion and exclusion rules match serialized `DocumentNodeKind` names.
Exclusion wins over inclusion. Required metadata values match node attributes
as strings or canonical JSON scalar/object text.

`project_metadata` names node attribute keys to retain. Values are emitted
under `metadata.source_attributes` as source-order arrays containing the node
ID and exact JSON value. Caller metadata is copied through a `BTreeMap`, so map
insertion order and worker completion order cannot affect canonical output.

## Validation

The focused regression matrix is:

```sh
cargo test --features document-graph --test segment_deterministic_engine
```

It covers serial/parallel canonical equality, exact rendered-span provenance,
stable IDs and digests, ancestry, overlap, custom tokenizers, byte/scalar/token
counts, selection and metadata projection, untraceable-node diagnostics, and
table/code/equation/figure-caption atomicity.
