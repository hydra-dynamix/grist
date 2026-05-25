# 0103 — Add JSON-only CLI skeleton

## Goal
Create the `grist` CLI as a thin JSON control surface over library APIs.

## Dependencies
- 0101
- 0102

## Scope
- Add commands: `parse markdown`, `parse rust`, `parse json`, `parse model-output`, `ingest file`, `ingest repo`, `schema list`, `schema emit`.
- Support `<path|->` stdin conventions.
- Emit JSON for all success and error paths.
- Stub unimplemented parser commands with structured diagnostics until backing parsers exist.

## Acceptance criteria
- CLI builds and returns valid JSON for `--help`-independent command execution paths.
- Stdin can be read by at least one stub command.
- CLI command structure maps directly to intended library functions.
