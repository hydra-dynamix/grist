# Grist CLI reference

The `grist` binary is a JSON-oriented control surface over the library APIs. Successful parser, ingest, and schema commands emit JSON unless a transform renderer explicitly emits Markdown or LaTeX text.

Install from a checkout with:

```sh
cargo install --path . --features cli
```

## Top-level menu

```text
grist parse      Parse one input into a typed Grist JSON envelope.
grist ingest     Detect and ingest one file or repository into artifact reports.
grist schema     List or emit checked-in public JSON Schema contracts.
grist transform  Convert supported inputs through DocumentGraph and render graph/Markdown/LaTeX.
```

Use `--help` at any level:

```sh
grist --help
grist parse --help
grist transform --help
```

## Parse commands

```sh
grist parse markdown README.md
grist parse latex paper.tex
grist parse html fragment.html --mode fragment
grist parse csv data.csv --delimiter comma
grist parse rust src/lib.rs --detail semantic-with-syntax
grist parse python script.py --detail semantic
grist parse typescript app.ts --dialect typescript
grist parse typescript app.tsx --dialect tsx
grist parse json config.toml --format toml
grist parse model-output response.txt --json-value
grist parse ldgr-projection ticket.md --strict
```

All parser commands accept `-` for stdin where the command can operate without path-based detection.

## Ingest commands

```sh
grist ingest file README.md
grist ingest file - --filename paper.tex
grist ingest repo . --exclude target/**
grist ingest repo . --include 'src/**' --external-artifact-dir .tmp/grist-artifacts
```

File ingestion detects supported content kinds and returns a `FileIngestReport`. Repository ingestion returns inventory, artifact metadata, detected languages, skipped files, and test hints where available.

## Schema commands

```sh
grist schema list
grist schema emit markdown-envelope
grist schema emit document-graph
grist schema emit document-graph-envelope
grist schema emit latex-envelope
```

Checked-in schema artifacts live in `schemas/` and are validated by schema drift tests.

## Transform commands

Transforms parse the source file, project it into `DocumentGraph`, optionally run semantic passes, then emit the requested target.

Supported source extensions:

- `.md`, `.markdown`
- `.tex`, `.latex`
- `.py`, `.pyi`
- `.rs`
- `.ts`, `.mts`, `.cts`, `.tsx`, `.jsx`

Targets:

```sh
grist transform README.md --to graph
grist transform README.md --to latex
grist transform paper.tex --to markdown
grist transform requirements.md --to graph --extract-obligations
```

Target behavior:

- `--to graph` emits JSON `DocumentGraph`.
- `--to markdown` emits rendered Markdown text.
- `--to latex` emits rendered LaTeX text.
- `--extract-obligations` runs deterministic conditional-obligation extraction before output.

Unsupported render nodes fail explicitly rather than being silently dropped unless library callers use permissive `TransformOptions` directly.

## Feature notes

- `cli` enables the binary and includes the parser features needed by the command surface, including `latex`.
- `document-graph` is enabled by default and provides normalized projections and renderers.
- `basin` depends on `document-graph` and routes code-structure walks through the normalized graph.
