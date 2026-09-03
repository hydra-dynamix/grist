# Cross-format integration contract

This document is the retained-format integration matrix for the built-in Grist
registry. It covers the 92 selectors compiled by `--all-features`. Selector
aliases share the authoritative payload and options schemas of their parser
family; they do not create duplicate wire contracts.

Legacy binary Office and Outlook store parsing is outside the retained scope.
The built-in registry does not advertise `doc`, `ppt`, `xls`, `xlsb`, `pst`,
`ost`, WordProcessingML 2003, SpreadsheetML 2003, or Flat OPC parsers. TNEF and
S/MIME are bounded, inert representations inside the email payload rather than
standalone format selectors.

## Registry matrix

`Graph + segment` means the authoritative payload has a public
`ToDocumentGraph` implementation, CLI `transform --to graph` routing, and
structural segmentation through that graph. Segment identities and citation
anchors retain the graph node identity and source locator. `Payload only`
means the format intentionally has no normalized graph or segmentation claim.

| Parser family | Retained selectors | Authoritative payload schema | Projection |
|---|---|---|---|
| Archive | `bzip2`, `gzip`, `seven_zip`, `tar`, `xz`, `zip`, `zstd` | `archive` | Graph + segment |
| Plain/publishing text | `text`, `markdown`, `quarto`, `r_markdown`, `restructured-text`, `asciidoc` | respectively `text`, `markdown`, `quarto`, `r_markdown`, `restructured-text`, `asciidoc` | Graph + segment |
| Markup/scholarly | `html`, `xml`, `latex`, `bibtex`, `epub` | `html`, `xml`, `latex`, `bibliography`, `epub` | Graph + segment |
| PDF | `pdf` | `pdf` | Graph + segment |
| Word OOXML | `docx`, `docm`, `dotx`, `dotm` | `word-ooxml` | Graph + segment |
| Presentation OOXML | `pptx`, `pptm`, `potx`, `ppsx` | `presentation-ooxml` | Graph + segment |
| Spreadsheet OOXML | `xlsx`, `xlsm` | `spreadsheet-ooxml` | Graph + segment |
| ODF presentation | `odp`, `otp` | `presentation-odf` | Graph + segment |
| ODF spreadsheet | `ods`, `ots` | `spreadsheet-odf` | Graph + segment |
| ODF word processing | `odt`, `ott` | `odf-word` | Graph + segment |
| Rich text | `rtf` | `rtf` | Graph + segment |
| Delimited data | `csv` | `csv` | Graph + segment |
| Structured text | `json`, `jsonl`, `yaml`, `toml` | `serialization` | Graph + segment |
| Structured binary | `cbor`, `messagepack`, `protobuf` | `structured-binary` | Graph + segment |
| Columnar data | `arrow`, `parquet` | `columnar` | Graph + segment |
| Database | `sqlite` | `sqlite` | Graph + segment |
| Email/message | `eml`, `mbox`, `msg`, `icalendar`, `vcard` | `email`, `mbox`, `outlook-msg`, `icalendar`, `vcard` | Graph + segment |
| Notebook | `ipynb` | `ipynb` | Graph + segment |
| Primary code | `rust`, `python`, `javascript`, `jsx`, `typescript`, `tsx` | language-specific `*-code` contract | Graph + segment |
| Secondary code | `c`, `cpp`, `csharp`, `css`, `go`, `java`, `kotlin`, `php`, `ruby`, `shell`, `sql`, `swift` | `code` | Graph + segment |
| Manifest | `manifest` | `manifest` | Graph + segment |
| Image | `bmp`, `gif`, `heif`, `jpeg`, `png`, `svg`, `tiff`, `webp` | `image` | Graph + segment |
| Timed media | `flac`, `matroska`, `mp3`, `mp4`, `quicktime`, `wav` | `media` | Graph + segment |
| Subtitle | `srt`, `ttml`, `webvtt` | `subtitle` | Graph + segment |
| Graph input | `graph` | `graph` | Graph + segment |
| Model response | `model_output` | `model-output` | Payload only |
| LDGR Markdown projection | `ldgr_projection` | `ldgr-projection` | Payload only |

## Surface invariants

- `grist capabilities` is generated from the same registry used by detection
  and parsing. Every available selector advertises resolvable current payload
  and options schema names and versions.
- `document_graph_projection` is present exactly when the payload kind has a
  library and CLI projection. `model_output` and `ldgr_projection` omit it
  explicitly rather than failing after advertising support.
- A graph is a normalized derived view. The typed payload remains
  authoritative. Unknown or opaque source material stays in typed raw fields,
  graph attributes, embedded-artifact records, or named diagnostics; it is not
  silently promoted into text.
- Segmentation consumes text-bearing graph nodes only. It preserves stable node
  identity, source locators, provenance, and deterministic order. Empty or
  opaque-only graphs validly produce no text segments.
- Citation anchors are derived from source locators and content identity, not
  from display text. Provider-derived content remains separately attributed.
- Feature-disabled builds keep schema identity metadata on unavailable parser
  descriptors and do not claim runtime availability or graph execution. A
  format's generated schema body is embedded only when its owning parser
  feature is compiled; consumers must check `available` before resolving it.
- The schema catalog and canonical-example manifest remain the source of truth
  for wire contracts. Selector-family mappings are tested against exact schema
  names and versions to prevent registry drift.

The integration invariant is enforced in `tests/capabilities_manifest.rs` and
is exercised alongside the CLI, feature-topology, schema drift, and
format-specific universal-contract suites.
