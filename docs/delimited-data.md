# CSV and TSV parsing

The `csv` feature provides Grist's built-in inert delimited-data scanner. It accepts CSV and TSV through the `csv` parser (`tsv` is a registry and CLI alias) and emits the authoritative `grist/csv/v2` payload.

## Dialect and fidelity

`CsvOptions::delimiter = auto` scores comma, tab, semicolon, and pipe candidates over bounded logical records while respecting quoted newlines. A non-auto delimiter is a declaration and is never replaced by detection. The payload retains the decision source, every candidate score, delimiter, quote, escape, and doubled-quote policy.

Each header, record, and cell retains exact decoded source text and UTF-8 ranges. Cells separately retain the source lexeme (including quotes and escapes), decoded field text, quote state, value range, typed scalar candidates, and a compatibility-selected JSON scalar. Record terminators distinguish LF, CRLF, CR, and unterminated final records; aggregate counts expose mixed-newline inputs. Quoted multiline cells remain one logical record.

Malformed quoting, trailing content after a quote, unexpected quotes, and ragged widths are retained on partial records and produce partial diagnostics rather than disappearing. Empty input is a complete document with no records. Parsing never evaluates cell text, accesses the network, or executes formulas or commands.

## Provenance, graph, and streaming

Public record and cell locators nest a one-based `RecordRange` with an exact decoded `TextRange`. `ToDocumentGraph` projects one document, table, row nodes, and cell nodes while retaining the complete parser-specific cell under the `grist.csv` extension. Generic structural segmentation therefore carries the same source locators.

`stream_csv` is the authoritative lazy path. It emits header/data `StreamEvent` items followed by exactly one terminal event. Record and cell budgets are charged before emission; exhaustion is `failed` before the first item and `partial` afterward. `parse_csv_with_control` is a collector over that same stream, so batch and streaming semantics cannot drift. Registry parsing additionally uses the shared decoder, including BOM-aware UTF encodings and enabled legacy encodings, and preserves decode diagnostics and identity.

The CLI supports `grist parse csv` (alias `tsv`) with `--delimiter auto|comma|tab|semicolon|pipe`, `--no-headers`, generic detection/ingestion, schema emission, and CSV/TSV-to-graph transforms.
