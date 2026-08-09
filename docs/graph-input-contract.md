# Optional generic graph-input adapter contract

## Status and scope

This document defines a **non-normative extension contract** for a possible
input-side `grist::graph` adapter. It gives downstream implementation tickets a
stable review target, but it does not claim that the adapter, module, feature,
schema, CLI surface, or format-specific adapters exist.

The [complete parser contract](complete-parser-contract.md) remains the
normative parser target. Nothing here changes its clauses, ownership matrix,
status, diagnostics, identity, resource-budget, security, schema, or promotion
requirements. In particular, this document does not define a Rust API,
implementation architecture, generated JSON Schema, feature wiring, CLI flag,
or `DocumentGraph` projection algorithm.

The extension is useful only if it preserves this boundary:

- `GraphDocument` is the parser-specific, loss-aware result of reading a
  generic graph source. It preserves source node and edge identities, graph
  direction, occurrence order, parallel edges, attributes, and source
  evidence.
- `DocumentGraph` is Grist's normalized cross-format projection described in
  [DocumentGraph architecture](document-graph.md). Its node-kind and relation
  vocabularies, graph identity algorithm, canonical ordering, evidence model,
  and rendering behavior do not become the input syntax of `GraphDocument`.
- Projection from a successfully parsed `GraphDocument` to `DocumentGraph` is
  possible downstream, but is not specified here. A `GraphDocument` is not a
  valid `grist/document-graph/v1` or `grist/document-graph/v2` value merely
  because both values contain nodes and edges.

The proposed adapter is layered over the shared parse envelope, source and
content identities, diagnostics, provenance, operation status, resource
budget, cancellation, and canonical JSON contracts. It does not introduce
graph-specific replacements for those shared contracts.

## Contract layers

There are three deliberately separate layers:

1. **Source encoding.** Raw JSON or YAML bytes have a raw identity and decoding
   evidence.
2. **GraphDocument.** The adapter parses the v1 node-edge dialect below and
   retains its generic graph semantics and declaration evidence.
3. **DocumentGraph.** A later, separately versioned projection may map generic
   nodes and edges into Grist's normalized vocabulary.

Success at one layer does not imply conformance at another. For example, a
cyclic graph can be a valid `GraphDocument` but an invalid LDGR scheduler graph,
and a valid `GraphDocument` still has no normalized `DocumentRelation` until a
projection policy assigns one.

## V1 JSON/YAML node-edge dialect

The compatibility identifier is `grist/graph-document/v1`. JSON and YAML are
two encodings of the same data model; neither encoding has additional graph
semantics.

### Document shape

The top-level value is one mapping with these fields:

| Field | Required | V1 meaning |
| --- | --- | --- |
| `schema_version` | yes | Exact string `grist/graph-document/v1`. |
| `id` | no | Non-empty opaque string identifying the source graph within its producer's scope. |
| `directed` | yes | Boolean default direction for edges without an override. |
| `nodes` | yes | Array of node declarations. Empty is valid. |
| `edges` | yes | Array of edge declarations. Empty is valid. |
| `attrs` | no | JSON object of graph-level application attributes; omitted is equivalent to `{}`. |

Unknown top-level fields are invalid in v1. Extensions belong in `attrs`,
preferably under a collision-resistant key such as `org.example.layout`. This
closed structural surface prevents a misspelled contract field from silently
becoming metadata.

A node declaration has:

| Field | Required | V1 meaning |
| --- | --- | --- |
| `id` | yes | Non-empty opaque string, unique among nodes by exact Unicode scalar sequence. |
| `labels` | no | Array of distinct, non-empty strings in source order; omitted is equivalent to `[]`. |
| `attrs` | no | JSON object of node attributes; omitted is equivalent to `{}`. |

An edge declaration has:

| Field | Required | V1 meaning |
| --- | --- | --- |
| `id` | yes | Non-empty opaque string, unique among edges by exact Unicode scalar sequence. |
| `source` | yes | Exact ID of an existing node. |
| `target` | yes | Exact ID of an existing node. |
| `directed` | no | Boolean override; when absent, the document-level value applies. |
| `label` | no | Non-empty application relation string. Absence means unlabeled, not `unknown`. |
| `attrs` | no | JSON object of edge attributes; omitted is equivalent to `{}`. |

Unknown node or edge fields are invalid. Node IDs and edge IDs occupy separate
namespaces, so a node and an edge may share the same string. IDs are never
trimmed, case-folded, Unicode-normalized, path-normalized, or parsed as numbers.
An adapter preserves them exactly.

### JSON and YAML value rules

Attribute values may be null, boolean, finite JSON number, string, array, or
object recursively. Object keys are strings. Attribute key order is not
semantic; array order is semantic. Empty arrays and objects are retained.

JSON inputs reject duplicate object keys instead of accepting a parser's
first-wins or last-wins behavior.

YAML inputs use one YAML 1.2 document and the JSON-compatible scalar and
collection subset. The following are invalid at this boundary:

- multiple YAML documents;
- duplicate mapping keys or non-string mapping keys;
- aliases, anchors, merge keys, or explicit/custom tags;
- non-finite numbers and implementation-specific scalar values such as native
  timestamps or binary objects.

Strings that a YAML implementation could implicitly resolve to a non-string
value must be quoted when a string is intended. These restrictions make JSON
and YAML decoding deterministic and prevent implementation-specific object
graphs. Comments are source trivia: an adapter may retain them as source
evidence, but comments are not GraphDocument attributes.

### Graph semantics

V1 describes a directed property multigraph:

- Every edge is one occurrence identified by its own `id`.
- An edge is directed according to its `directed` field or, when absent, the
  document default. A directed edge runs from `source` to `target`.
- An undirected edge is one edge occurrence whose endpoints are an unordered
  pair. It is not rewritten as two directed edges.
- Per-edge overrides permit mixed graphs.
- Self-loops are valid.
- Cycles are valid, including directed cycles.
- Parallel edges are valid, including edges with the same endpoints, direction,
  label, and attributes. Distinct edge IDs keep every occurrence observable.
- Node and edge array order preserves source declaration order. Order is not
  graph topology and does not make an otherwise cyclic graph acyclic.

Missing endpoints, duplicate IDs, a non-boolean direction, or an invalid
attribute value invalidate the affected GraphDocument. An adapter never
creates a missing node merely to repair the v1 JSON/YAML dialect. Format
adapters whose native grammar has implicit nodes may create explicit
GraphDocument nodes only when that behavior is part of their reviewed mapping
and carries source/derivation evidence.

## Attributes and loss accounting

`attrs` is the only open-ended v1 extension point. Adapters preserve names,
JSON value types, empty values, and nested structure. They do not stringify all
values, flatten nested objects, or promote application keys into structural
fields. Keys beginning with `grist.` are reserved for reviewed Grist meanings;
other producers use their own namespace.

Attributes are inert data. URLs are not fetched, scripts or callbacks are not
executed, styles are not rendered, and identifiers are not resolved against an
external database during parsing.

When a source format contains a construct that v1 cannot represent without
loss, a format adapter either retains a lossless JSON-compatible source form in
a namespaced attribute or emits an explicit unsupported/loss diagnostic. It
does not silently discard the construct. A retained raw form remains
parser-specific data; it does not acquire normalized `DocumentGraph` meaning.

## Identity and determinism

Source graph IDs and Grist content identities solve different problems:

- Document, node, and edge `id` values are producer-supplied local identities.
  Their stability across edits is only as strong as the source format's stated
  identity guarantee.
- Raw input bytes use `RawContentIdentity`. Equivalent JSON and YAML therefore
  normally have different raw digests.
- The parsed payload uses `CanonicalPayloadIdentity` and
  `grist/canonical-json/v1`. Semantically equivalent JSON and YAML may have the
  same canonical payload identity when they produce the same GraphDocument.
- IDs generated by a format adapter must be deterministic, collision-checked,
  and based on stable native identity when available. Otherwise the adapter
  records that the identity is locator/occurrence-backed and may change after
  edits. Generated IDs must not collapse parallel edge occurrences.

For identical decoded input, parser/backend version, options, and budget, an
adapter produces the same payload, declaration order, diagnostics, operation
status, and provenance. Mapping key insertion order and parallel worker
completion order cannot change canonical serialized bytes. V1 does not sort
the `nodes`, `edges`, or `labels` arrays: their source order is meaningful to
the parser-specific payload. Any canonical reordering done by `DocumentGraph`
belongs to the separate projection layer.

Default expansion is deterministic. Omitting `attrs` or `labels` is
semantically equivalent to the empty value stated above; omitting an edge's
`directed` value is semantically equivalent to copying the document default.
An implementation may retain whether a default was explicit as source trivia,
but that trivia cannot alter graph semantics.

## Provenance and diagnostics

The shared parse envelope is authoritative for source identity, parser and
backend metadata, operation provenance, status, and diagnostics. A
GraphDocument adapter also retains an exact `SourceLocator` for each graph,
node, and edge declaration and for the most specific invalid field available.
Generated declarations identify their source declarations and named derivation
rule. Caller-authored `attrs` never masquerade as trusted provenance.

JSON Pointer locations are appropriate for semantic JSON coordinates. YAML
adapters additionally retain exact decoded text ranges so repeated values and
comments do not make locations ambiguous. Any normalization, repair, or
format-specific mapping appends a shared `ProvenanceStep` with explicit loss
classification. Timestamps appear only when supplied by the caller, preserving
determinism.

Diagnostics use `grist/diagnostic/v1`, shared condition codes, safe structured
details, cause chains, and partial-output effects. The proposed graph-specific
codes below are stable review names for the downstream implementation ticket:

| Condition | Expected code |
| --- | --- |
| Unsupported graph dialect version | `grist.graph.schema_version.unsupported` |
| Duplicate node ID | `grist.graph.node.id.duplicate` |
| Duplicate edge ID | `grist.graph.edge.id.duplicate` |
| Edge endpoint absent from `nodes` | `grist.graph.edge.endpoint.unknown` |
| Closed structural field is unknown | `grist.graph.field.unknown` |
| YAML uses a forbidden feature | `grist.graph.yaml.feature.unsupported` |
| Native graph construct cannot be retained losslessly | `grist.graph.construct.unsupported` |

Syntax errors and wrong field types use `grist.input.malformed` with a more
specific explanation key in structured details. Resource exhaustion uses the
shared `grist.budget.<axis>.exhausted` family. Security rejection and adapter
defects use the existing shared families. Invalid inputs do not return a
silently repaired success payload.

## Limits, cancellation, and partial results

Every parse explicitly selects a shared budget profile or custom
`ResourceBudget`; there is no graph-specific implicit unlimited mode. At a
minimum, adapters account for input bytes, decoded Unicode scalars, node
declarations, edge/record declarations, attribute/container nesting,
estimated live memory, parse time, and emitted output bytes. Format adapters
also charge applicable XML, container, or expansion work. Shared inclusive
limit, cancellation, and terminal-status semantics apply.

Untrusted profiles set finite maxima for individual identifier/label lengths,
attribute depth, attribute entries, and total declarations. A limit hit is
never represented as an empty successful graph. If no meaningful payload is
available, status is `failed`; after retained output, status is `partial`.
Every retained edge in a partial payload still refers to retained nodes, and
omitted declarations are described by diagnostics. Cancellation has status
`cancelled` and retains already emitted evidence according to the shared
stream/batch contract.

## LDGR compatibility boundary

`grist/graph-document/v1` and `ldgr.graph.v1` are different contracts.
GraphDocument parsing never reads or mutates an LDGR database, resolves
`artifact:` or `work:` references, schedules work, or infers completion.

An explicit LDGR adapter may map an `ldgr.graph.v1` dependency graph into a
GraphDocument as follows:

- one LDGR node becomes one graph node with the same ID;
- `artifact` and `work_item` references remain typed, inert attributes;
- an LDGR edge maps `dependency` to `source`, `dependent` to `target`, and
  `kind` to `label`, with `directed: true`;
- a distinct deterministic edge ID is supplied for each edge occurrence.

Before claiming LDGR scheduler compatibility, that adapter also enforces the
stricter LDGR rules: unique node IDs, existing endpoints, valid typed reference
syntax, no self-edge, and no cycle. These are adapter validation rules, not
generic GraphDocument rules. Consequently, a cyclic or self-looping
GraphDocument is valid generic input but cannot be silently submitted to the
Conduct scheduler.

## Criteria for additional source-format adapters

A DOT, Mermaid, or GraphML adapter is promotable only after its supported
language/version is named, its mapping to every v1 field is documented, and
valid, malformed, adversarial, loss, budget, provenance, and determinism
fixtures pass. Each adapter must preserve parallel occurrences and inert source
data, reject or diagnose unsupported constructs, retain exact source evidence,
and never depend on rendering, network access, or code execution.

### DOT

A DOT adapter distinguishes `graph` from `digraph`, preserves `strict` as a
native constraint rather than changing generic multigraph semantics, expands
node/edge/default attribute statements deterministically, and retains subgraph,
port, compass, quoted/HTML-like label, escape, and comment evidence. DOT's
implicit endpoint nodes may become derived explicit nodes with locators and a
named rule. Unsupported HTML-like or layout constructs are retained inertly or
diagnosed; they are never rendered or executed.

### Mermaid

A Mermaid adapter declares the supported Mermaid grammar and diagram families.
For flowchart/graph input it preserves direction declarations, node IDs and
labels, edge occurrences, subgraphs, classes/styles, and link metadata as inert
data. Diagram families without a lossless node-edge mapping are rejected or
retained through a documented parser-specific representation with diagnostics.
Generated IDs for anonymous edges or nodes are source-locator/occurrence based
and deterministic. Click handlers, links, icons, and initialization directives
never execute or fetch resources.

### GraphML

A GraphML adapter is namespace-aware and uses secure, non-networked XML
parsing. It preserves graph `edgedefault`, per-edge direction overrides, graph,
node, and edge IDs, `<key>` declarations, typed `<data>` values and defaults,
and declaration order. Nested graphs, hyperedges, ports, endpoint roles, and
unknown extension elements require lossless namespaced retention or an
explicit unsupported/loss diagnostic. External entities and schemas are never
resolved. XML type coercion and default expansion are deterministic and retain
their source evidence.

Passing criteria for one format do not imply support for another, and no
adapter is listed as supported until its implementation, registry/schema
surface, security review, and complete-parser promotion evidence land in their
own downstream tickets.

## Review fixtures

The review-only fixtures are under `fixtures/generated/graph/`. The valid JSON
and YAML pair encode the same semantic GraphDocument, including a directed
cycle, a self-loop, parallel edges, an undirected override, nested typed
attributes, and opaque IDs. Invalid fixtures pin version, identity,
referential-integrity, and closed-field boundaries. Their expected outcomes
are listed in the fixture README.

These fixtures are contract evidence, not a claim that an executable parser or
schema target already exists. The downstream universal contract test is
expected to consume them when that implementation is added.
