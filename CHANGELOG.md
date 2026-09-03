# Changelog

## 0.2.0

- Expanded the built-in registry across document, archive, Office, media, structured-data, code, and model-output formats.
- Added capability discovery, bounded parser execution, provider boundaries, graph projection, schema and fixture contracts, and fuzz regression coverage.

## 0.1.0

- Initial public Grist release.
- Added typed parsers and JSON envelopes for Markdown, HTML, CSV, Rust, Python, TypeScript/TSX/JSX, serialization formats, model outputs, plain text, repository ingestion, and LDGR projection documents.
- Added checked-in JSON Schemas for public output contracts.
- Added optional `grist` CLI behind the `cli` feature.
- Added the optional generic graph JSON/YAML adapter with ranked detection,
  shared ingest/CLI dispatch, validation and DAG analysis, LDGR conversion,
  normalized DocumentGraph projection, and generated schemas.
- Added inert TNEF attribute inventory and signed/encrypted S/MIME evidence,
  with explicit provider-routed decryption, preserved ciphertext, provider
  provenance, graph/segment integration, and secret-safe defaults.
