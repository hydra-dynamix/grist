# 0204 — Implement file ingestion API

## Goal
Implement single-file ingestion for paths and already-loaded content.

## Dependencies
- 0203

## Scope
- Add raw bytes/text APIs.
- Add path-based API with limits and source metadata.
- Apply detection and parser registry dispatch.
- Return shared envelope plus typed payload.

## Acceptance criteria
- File ingestion works for supported and unsupported files.
- Size/decode/unsupported failures return diagnostics, not panics.
- CLI `ingest file` can call this API.
