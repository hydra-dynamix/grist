# 0601 — Add externalized repo artifact mode

## Goal
Support large repositories by writing parsed artifacts separately from repo manifest output.

## Dependencies
- 0205

## Scope
- Add repo ingest output mode for externalized artifact files.
- Include artifact references, paths, hashes, kind, schema version, and diagnostics in the manifest.
- Keep inline mode as default for agents/tests/smaller repos.

## Acceptance criteria
- Inline and externalized modes are both tested.
- Manifest references are deterministic and content-addressed or otherwise stable.
- Large-output limits are respected without losing diagnostics.
