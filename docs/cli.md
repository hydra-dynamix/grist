# Grist CLI reference

The `grist` binary is a thin JSON-oriented control surface over the public
library APIs. Detection and parser selection come from the shared registry;
the CLI does not contain an independent parser implementation. Successful
structured commands emit one JSON value, while documented streaming modes emit
one JSON event per line.

Install from a checkout with:

```sh
cargo install --path . --features cli
```

## Top-level menu

```text
grist parse      Parse one input into a typed Grist JSON envelope.
grist ingest     Detect and ingest one file or repository into artifact reports.
grist detect     Return ranked format detection with retained ambiguity evidence.
grist inspect    Detect and parse one input into a universal inspection envelope.
grist schema     List or emit checked-in public JSON Schema contracts.
grist render     Render DocumentGraph or serialization inputs with fidelity metadata.
grist segment    Segment a source or DocumentGraph with source provenance.
grist validate   Validate inputs against stable Grist-supported contracts.
grist transform  Convert supported inputs through DocumentGraph and normalized renderers.
grist capabilities Report the complete compiled feature and capability manifest.
```

Use `--help` at any level:

```sh
grist --help
grist parse --help
grist render --help
grist validate --help
grist transform --help
grist parse pdf --help
```

The named parse subcommands cover common formats. Every other enabled selector
uses the registry-routed form `grist parse FORMAT INPUT`; the generic flags are
`--filename`, `--mime`, `--kind`, `--request-id`, and `--options`. Run
`grist parse FORMAT --help` for selector metadata and generic options, and
`grist capabilities` for the complete compiled registry.

## Parse commands

```sh
grist parse markdown README.md
grist parse latex paper.tex
grist parse latex paper.tex --project-root ./project
grist parse latex paper.tex --no-resolve-includes
grist parse bibtex references.bib
grist parse biblatex references.bib --options bibliography-options.json
grist parse pdf document.pdf
grist parse html fragment.html --mode fragment
grist parse xml article.nxml --dialect jats
grist parse csv data.csv --delimiter comma
grist parse rust src/lib.rs --detail semantic-with-syntax
grist parse python script.py --detail semantic
grist parse typescript app.ts --dialect typescript
grist parse typescript app.tsx --dialect tsx
grist parse json config.toml --format toml
grist parse cbor value.cbor
grist parse messagepack events.msgpack
grist parse protobuf person.pb --descriptor api.desc --message example.Person
grist parse sqlite records.sqlite --options sqlite-options.json
grist parse eml message.eml --options email-options.json
grist parse mbox mailbox.mbox --options mbox-options.json
grist parse msg message.msg --options outlook-msg-options.json
grist parse icalendar invite.ics --options icalendar-options.json
grist parse vcard contacts.vcf --options vcard-options.json
grist parse model-output response.txt --json-value
grist parse ldgr-projection ticket.md --strict
grist parse graph network.graph.json
grist parse graph - --filename network.graph.yaml
```

All parser commands accept `-` for stdin. Use `parse auto - --filename NAME`
and optionally `--mime TYPE` or `--kind FORMAT` when stdin needs detection
context. Genuine ambiguity remains an `ambiguous` envelope.

SQLite record extraction requires an options document with selected table names
and positive `max_tables` and `max_rows_per_table` limits. Omitting
`record_selection` performs schema-only inspection. See `docs/sqlite.md`.

EML parsing preserves every source header and MIME entity, decodes RFC 2047/2231
text and transfer encodings, inventories exact attachment identities, and may
recursively parse only attachment bytes already present in the message. Remote
HTML images, links, and `Content-Location` values are retained but never fetched.
See `docs/email.md`.

iCalendar and vCard parsing preserves content-line folding, scheduling and
contact fields, recurrence, time zones, attendee state, and attachment
relationships. Calendar methods, RSVP state, alarms, URLs, and MIME calendar
parts remain inert: the CLI never sends responses, edits calendars or contacts,
or resolves remote content. See `docs/calendar-contact.md`.

MBOX parsing emits ordered messages with stable decoded-content identities,
source envelope separators and escaping evidence, mailbox-level thread links,
and the same inert MIME and attachment behavior as EML. The public library also
exposes a terminal-event stream for mailboxes too large to collect eagerly.

Outlook MSG parsing traverses the compound file and MAPI property streams under
explicit limits. It preserves recipients, all body alternatives, attachments,
embedded messages, named and unknown properties, dates, message identifiers,
and exact storage/stream locators without opening attachments or active HTML.
See `docs/email.md`.

The HTML selector also accepts registry aliases htm and xhtml. Raw-byte registry
parsing honors HTML meta and XHTML XML encoding declarations. The XML selector also accepts jats and nxml aliases; `--dialect auto|xml|jats` controls structural interpretation without enabling entity, XInclude, schema, or network resolution. DOM, source-token,
metadata, link, table, media, semantic-section, and inert active-content records
are emitted in the JSON payload; no referenced resource is fetched.

## Ingest commands

```sh
grist ingest file README.md
grist ingest file - --filename paper.tex
grist ingest repo . --exclude target/**
grist ingest repo . --include 'src/**' --external-artifact-dir .tmp/grist-artifacts
grist ingest archive bundle.zip --inventory-only
grist ingest batch README.md src/lib.rs --request-id docs --request-id library
grist ingest batch README.md src/lib.rs --collect
```

File ingestion detects supported content kinds and returns a `FileIngestReport`. Repository ingestion returns inventory, artifact metadata, detected languages, skipped files, and test hints where available.

Batch ingestion uses the library's ordered `IngestStream`. The default output
is NDJSON with item events followed by exactly one terminal event; `--collect`
emits the equivalent `BatchResult`. Each item includes its stable request ID
and sequence. Archive ingestion uses the shared container controller and reports
unsupported decoders, encryption, rejection, cancellation, and budget limits
as structured traversal states.

## Schema commands

```sh
grist schema list
grist schema emit markdown-envelope
grist schema emit document-graph
grist schema emit document-graph-envelope
grist schema emit rendered-summary
grist schema emit dynamic-event-explorer-dataset
grist schema emit latex-envelope
grist schema emit bibliography-envelope
grist schema emit bibliography-citation-resolution
grist schema emit pdf-envelope
```

Checked-in schema artifacts live in `schemas/` and are validated by schema drift tests.

`grist transform ... --to graph` emits a `graph-transform-envelope`, not a bare
graph. Its payload contains the graph plus complete node attribution. Commands
that consume graph files accept both this envelope and legacy bare graph JSON.
For normalized text targets, `--manifest` includes both the graph-transform and
generated-text source maps. Package reconstruction is a separate format adapter
operation and is never implied by `transform` or `render`.

## Render commands

Render commands parse supported input and project it into deterministic inspection artifacts. Output is JSON, not prose-only.

```sh
grist render json-summary data.json
grist render json-summary - --profile dynamic-event-dataset
grist render serialization-summary config.yaml --format yaml
grist render json-summary data.json --schema schemas/dynamic-event-explorer.dataset.v1.schema.json
grist render graph.json --to markdown
grist render graph.json --to text --text-output output.txt --request-id render-7
```

`rendered-summary` output uses this shape:

```json
{
  "schema_version": "grist/rendered-summary/v1",
  "source_schema_version": "grist/structured-text/v2",
  "title": "Dynamic Event Dataset",
  "profile": "dynamic-event-dataset",
  "sections": [
    {"heading": "Canonical Events", "facts": ["9 canonical events", "7 participants"]}
  ],
  "tables": [],
  "diagnostics": []
}
```

Generic JSON summaries report root type, top-level keys, array lengths, repeated object shapes, and detected id/time/name-like fields. The optional `dynamic-event-dataset` profile adds dataset-oriented facts such as canonical event count, signal count, participant count, time extent, context keys, metadata keys, malformed/missing-field diagnostics, and representative events.

Normalized graph rendering returns `RenderResult`, including the generated
content, gap-free source map, explicit fidelity mode, named losses,
reconstruction claim, renderer identity, and options digest. `--text-output`
is the explicit raw-output mode: the content is written to the requested path
and stdout contains a `cli-text-output-manifest` with its SHA-256, destination,
source map, fidelity report, and request ID.

## Validate commands

```sh
grist validate json dataset.json --schema schemas/dynamic-event-explorer.dataset.v1.schema.json
grist validate graph.json --schema document-graph
```

Validation emits the normal `serialization-envelope`; callers should consume `payload.value` only when blocking diagnostics are absent and `payload.validation.valid` is true.

`validate input` accepts either a registered schema name or a schema path and
returns an operation `validate` envelope containing `SchemaValidationReport`.

## Transform commands

Transforms parse the source file, project it into `DocumentGraph`, optionally run semantic passes, then emit the requested target.

Supported source formats are selected by the same built-in parser registry used
by `parse auto`, using extension and content evidence. The compiled selector
set is reported by `grist capabilities`; the retained format/feature matrix is
in `cross-format-integration.md`.

Targets:

```sh
grist transform README.md --to graph
grist parse restructured-text guide.rst
grist parse asciidoc guide.adoc
grist transform README.md --to latex
grist transform README.md --to latex README.tex
grist transform --file README.md --to latex README.tex
grist transform --file README.md --to latex --output README.tex
grist transform paper.tex --to markdown
grist transform requirements.md --to graph --extract-obligations
grist transform README.md --to html --output README.html --manifest README.html.manifest.json
grist transform README.md --to text --fidelity raw-fallback
```

Target behavior:

The input may be positional (`grist transform README.md --to latex`) or named with `--file`. Output goes to stdout unless you provide a second positional output path or `--output`.

- `--to graph` emits a JSON `graph-transform-envelope` whose payload contains
  the `DocumentGraph` and complete attribution.
- `--to markdown|latex|html|text` emits normalized text.
- `--manifest PATH` writes the source-map/fidelity companion for that text.
- `--fidelity strict|raw-fallback|lossy` makes the loss policy explicit.
- `--extract-obligations` runs deterministic conditional-obligation extraction before output.

Unsupported render nodes fail explicitly rather than being silently dropped unless library callers use permissive `TransformOptions` directly.

## Detect, inspect, segment, and capabilities

```sh
grist detect README.md
grist detect - --filename README.md --mime text/markdown
grist inspect - --filename module.rs --kind rust
grist segment graph.json --graph --config segment-options.json
grist segment README.md --config segment-options.json --stream
grist capabilities
```

`detect` emits `cli-detection-report/v1` with the source identity, ranked
candidates, evidence, parser availability, diagnostics, and request ID.
`inspect` is unified auto-ingestion with the same ambiguity policy.
`segment --stream` emits documented `SegmentEvent` NDJSON in canonical order;
collector mode emits an operation envelope. `capabilities` emits the canonical
`grist/capability-manifest/v1` library value: compiled feature status, registry
and normalized format availability, payload/options/envelope/schema versions,
backend identities, provider requirements, budget profiles and axes, security
modes, reconstruction claims, and explicitly unsupported capabilities. See
`capability-discovery.md` for the complete contract.

## Feature notes

- `cli` enables the binary and includes the parser features needed by the command surface, including `latex`, `word-ooxml`, `presentation-ooxml`, `presentation-odf`, `odf-word`, and `rtf`; use `grist parse docx|docm|dotx|dotm PATH` for OOXML Word packages, `grist parse pptx|pptm|potx|ppsx PATH` for OOXML presentation packages, `grist parse odp|otp PATH` for OpenDocument presentation packages, `grist parse odt|ott PATH` for OpenDocument text packages, and `grist parse rtf PATH` for Rich Text Format.
- `document-graph` is enabled by default and provides normalized projections and renderers.
- `basin` depends on `document-graph` and routes code-structure walks through the normalized graph.
