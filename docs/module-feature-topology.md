# Module and feature topology

Grist is one package whose public boundaries are arranged so they can later be
split into crates without changing contracts. Shared operations live in
`core`, `detect`, `registry`, `ingest`, `container`, `document_graph`,
`segment`, `transform`, `render`, `schema`, and `provider`. Authoritative
format payloads live under `formats`; model-output interpretation remains the
first-class `model_output` module. The `cli` module and binary contain adapters,
not parser implementations.

## Cargo selections

- `--no-default-features` is the minimal shared library.
- `default` preserves the existing 0.1 built-in parser set: text/publishing,
  structured data, code, model output, graph projection, and schemas.
- `cli` enables the command adapter and the complete built-in set.
- `full` enables every built-in format-family gate without enabling the binary.

The independently selectable family gates are `text-publishing`, `scholarly`,
`pdf`, `word-processing`, `presentations`, `spreadsheets`, `structured-data`,
`email-message`, `notebooks`, `code`, `archives`, `media`, and `model-output`.
A family gate with no parser in 0.1 is deliberately dependency-free: it
reserves the stable selection name but does not report a parser as available.
Individual 0.1 gates such as `markdown`, `restructured-text`, `asciidoc`,
`word-ooxml`, `presentation-ooxml`, `odf-word`, `csv`, `rust`, and `serialization`
remain supported for consumers that need a narrower dependency graph.

## Compatibility paths

The canonical parser namespaces are `grist::formats::markdown`,
grist::formats::restructured_text, grist::formats::asciidoc,
`grist::formats::html`, `grist::formats::word_ooxml`,
`grist::formats::presentation_ooxml`, `grist::formats::presentation_odf`, `grist::formats::odf_word`, `grist::formats::delimited`,
`grist::formats::structured_text`, `grist::formats::latex`, and the enabled
code-language modules. Existing paths such as `grist::markdown`, `grist::csv`,
and `grist::rust` remain public compatibility aliases. The
`grist::word_ooxml`, `grist::presentation_ooxml`, `grist::presentation_odf`, and `grist::odf_word`
paths are compatibility aliases for the corresponding package models. Both
canonical and compatibility paths export the
same Rust items, so values cross the boundary without conversion and serialized
contracts do not change. These aliases will not be removed during the 0.x
compatibility window without a separately documented migration.
