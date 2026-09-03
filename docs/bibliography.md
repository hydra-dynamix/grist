# BibTeX and BibLaTeX

The `bibliography` feature provides one inert, source-preserving parser selected
as `bibtex`, `biblatex`, `bibliography`, or `bib`. It accepts `.bib` bytes,
decodes them through Grist's shared charset contract, detects BibTeX versus
BibLaTeX vocabulary, and emits `grist/bibliography/v1`.

The authoritative payload retains source-order entries, duplicate keys,
duplicate fields, `@string` definitions, `@preamble`, `@comment`, percent-line
comments, and unknown or malformed raw spans. Entries and every field/value
atom carry exact UTF-8 byte/line locators and original spelling. Braced,
quoted, numeric, and string-identifier atoms remain separate even when a
resolved value is available.

String concatenation and month macros are resolved without invoking TeX.
Expansion records literal, built-in, and `@string` definition provenance.
Undefined or duplicate identifiers, cycles, depth/operation/output limits, and
malformed values produce a partial envelope with the original value intact.
Limits are configured through `BibliographyOptions`.

`crossref` and BibLaTeX `xdata` inheritance produce an `effective_fields` view;
authored fields remain unchanged. Missing or duplicate targets, cycles, and
depth limits are explicit `CrossrefResolution` records and partial diagnostics.
Citation lookup never selects among duplicate keys:

```rust
let envelope = grist::bibliography::parse_bibliography(
    "@book{example, title={Exact}}",
    grist::core::SourceInfo::inline("references.bib"),
    &grist::bibliography::BibliographyOptions::default(),
);
let citation = envelope.payload.unwrap().resolve_citation("example");
```

The `DocumentGraph` projection emits bibliography-entry and field nodes,
retains all format-specific data under `grist.bibliography`, and creates exact
`references` edges for uniquely resolved crossrefs. Comments and unknown raw
constructs remain graph nodes. Checked schemas cover the payload, envelope,
options, and citation-resolution report.

No source text is executed, included, or fetched. TeX commands, shell syntax,
URLs, and file-looking values are inert strings. Synthetic regression fixtures
live under `fixtures/generated/bibliography/`.
