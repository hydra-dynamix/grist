# 0101 — Define core public contract types

## Goal
Implement Grist's shared typed contract layer.

## Dependencies
- 0001

## Scope
- Define shared envelope types.
- Define source identity, source range, hashes, parser metadata, diagnostics, and shared options/limits.
- Use half-open ranges and UTF-8 byte offsets plus 1-based line/column positions.
- Provide serde support for all public output structs.

## Acceptance criteria
- Types serialize deterministically enough for golden tests.
- Diagnostics support severity, code, message, parser, range, partial state, cause/details.
- Shared limits/options are centralized and reusable by module-specific options.
