# Apache Arrow IPC and Parquet

Feature gate: columnar (included by structured-data and full)
Payload: grist/columnar/v1
Options: grist/columnar-options/v1

Grist reads Arrow IPC file and stream framing directly from bytes and reads Parquet footer/page metadata through the Thrift Compact Protocol. Both readers are inert: they do not execute code, load extensions, access the filesystem, or make network calls.

The authoritative payload retains:

- ordered schema fields, nullability, nested children, logical/physical types, field metadata, and file metadata;
- Arrow record batches, dictionary batches, validity bitmaps, buffer byte ranges, primitive and nested values;
- Parquet row groups, column paths, repetition/definition levels, page encodings, compression identity, and stable source-row locators;
- booleans, signed and unsigned integers, half/single/double floats with raw bits, decimals, UTF-8, binary, date/time/timestamp/duration/interval values, lists, structs, maps, unions, dictionaries, and explicit nulls;
- deterministic row and column projections through row_start, row_limit, columns, batch_start, and batch_limit.

Arrow supports modern continuation framing and legacy metadata-length framing. File and stream containers remain distinct. The reader validates every FlatBuffers offset, vtable, vector, body range, buffer range, child offset, dictionary index, and UTF-8 value. Record locators use zero-based source rows plus a fully qualified field path; encoded columns and batches additionally carry exact byte ranges.

Parquet supports PLAIN values, PLAIN_DICTIONARY/RLE_DICTIONARY values, RLE/bit-packed hybrid levels, DataPage v1/v2, dictionary pages, uncompressed pages, Snappy blocks, and Gzip members. The schema retains repetition levels and converted/logical type identity. Unknown pages, invalid Thrift, malformed levels, missing dictionaries, unsupported value encodings, and unavailable compression codecs are explicit failed/unsupported diagnostics rather than silent loss.

Resource behavior is caller-visible. max_metadata_bytes and max_encoded_page_bytes reject oversized encoded units before allocation; row/batch/column selectors bound materialized output; OperationControl enforces input, record, cell, node, memory, time, and cancellation budgets. parse_columnar_events exposes schema, dictionary, batch, diagnostic, and terminal events in source order. A budget hit never appears as end-of-input.

DocumentGraph projection creates a document root, batch/row-group containers, stable source rows, and typed field cells. The complete schema remains attached to graph attributes even when rows or columns are projected. Segmentation therefore inherits the exact row/field locators instead of synthesizing anonymous text.

Detection uses ARROW1/PAR1 file boundaries and validates stream FlatBuffers message framing before emitting structural evidence. Registry names are arrow (aliases by extension include feather) and parquet; CLI parse/transform/segment operations use the same library options and envelopes.