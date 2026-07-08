# DocumentGraph architecture

`DocumentGraph` is Grist's normalized projection layer for moving between parser-specific documents, cross-format renderers, graph-oriented code consumers, and semantic passes.

It is intentionally **not** a replacement for parser-specific payloads. `MarkdownDocument`, `LatexDocument`, `PythonFile`, `RustFile`, `TypeScriptFile`, and the other parser outputs remain the authoritative detailed parse contracts. `DocumentGraph` is the shared IR used when a downstream workflow needs a common graph over document structure, code facts, references, transforms, or semantic obligations.

## Contract

The public graph model lives in `grist::document_graph` and is available with the `document-graph` feature, enabled by default.

Core types:

- `DocumentGraph` — graph envelope payload: id, kind, source, language/dialect, nodes, edges, diagnostics, attrs.
- `DocumentNode` — normalized node with id, kind, optional range/text/name/qualified_name/parent/ordinal, and JSON attrs.
- `DocumentEdge` — normalized relation with source, relation, target, optional range, and attrs.
- `DocumentNodeKind` — prose, code, raw, math, citation/reference, and semantic node vocabulary.
- `DocumentRelation` — structural, code, reference, provenance, and semantic relation vocabulary.
- `ToDocumentGraph` — trait for parser-specific payloads to project into the graph.
- `TransformOptions` / `TransformError` — renderer controls and structured failure modes.

The schema names are:

```sh
grist schema emit document-graph
grist schema emit document-graph-envelope
```

## Existing projections

Current parser payloads that project into `DocumentGraph`:

- Markdown: headings, paragraphs, text, links, code fences, tables, table rows/cells, frontmatter.
- LaTeX: sections, paragraphs, formatting commands, inline/block math, labels, refs, citations, environments, comments, raw unknown commands.
- Python: symbols, imports, calls, assignments, returns, branches, containment, inheritance.
- Rust: symbols, imports, containment.
- TypeScript/TSX/JSX: symbols, imports, exports, calls, assignments, returns, branches, containment.

## Rendering and transforms

Current renderers:

- `render_markdown(&DocumentGraph, TransformOptions) -> Result<String, TransformError>`
- `render_latex(&DocumentGraph, TransformOptions) -> Result<String, TransformError>`

CLI examples:

```sh
# Emit normalized graph JSON
grist transform README.md --to graph

# Markdown -> LaTeX through DocumentGraph
grist transform README.md --to latex

# LaTeX -> Markdown through DocumentGraph
grist transform paper.tex --to markdown

# Extract explicit obligations while emitting graph JSON
grist transform requirements.md --to graph --extract-obligations
```

Transforms are not byte-identical round trips. Unsupported nodes fail explicitly unless `TransformOptions` permits lossy output or raw fallback. Renderers must not silently drop unsupported semantic/code nodes.

## Conditional obligations

Conditional obligations are represented as semantic graph structure, not as a bare edge.

Typical shape:

```text
obligation node
  - ConditionalOn -> condition node
  - Requires/Forbids/Allows -> required state node
  - DerivedFrom -> source prose node
source prose node
  - EvidenceFor -> obligation node
```

`ObligationAttrs` preserves:

- modality (`must`, `shall`, `should`, `may`, and negative variants);
- polarity (`positive`, `permission`, `prohibition`, etc.);
- subject;
- predicate/action;
- source text;
- extraction method;
- confidence;
- extension attrs.

The deterministic extraction pass is:

```rust
grist::document_graph::extract_conditional_obligations(&mut graph);
```

It intentionally handles only explicit patterns like:

```text
If the file is executable, it must have a shebang.
When an input is invalid, the parser should emit a diagnostic.
Unless a raw fallback is configured, unsupported nodes must not be dropped.
```

Ambiguous prose is left untouched rather than guessed. There is no LLM/NLP dependency in this pass.

## Basin and code graph consumers

The `basin` feature now routes Python helper APIs through `DocumentGraph` and also exposes direct graph entry points:

```rust
grist::basin::graph_from_document_graph(&graph);
grist::basin::walks_from_document_graph(&graph, options);
```

This makes the basin operator language-neutral for current and future code parser projections while preserving the existing signature and relaxation behavior.

## Migration guidance

For downstream consumers:

1. Keep using parser-specific payloads when you need exact parser details.
2. Use `ToDocumentGraph` when you need a shared graph over multiple input formats.
3. Use `DocumentGraph` for cross-format transforms, semantic obligation extraction, and graph consumers such as basin retrieval.
4. Treat graph attrs as the compatibility escape hatch for parser-specific metadata that has no normalized field.
5. Treat renderer output as normalized, not byte-identical source reproduction.

Existing parser APIs remain available. No downstream consumer is required to migrate unless it needs cross-format graph behavior.

## Validation

The integration regression matrix is in:

```text
tests/document_graph_golden.rs
```

Recommended promotion command:

```sh
cargo test --features "cli latex basin"
```
