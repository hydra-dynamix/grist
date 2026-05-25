# 0203 — Implement internal parser registry

## Goal
Create modular parser dispatch based on enabled features and detection results.

## Dependencies
- 0202

## Scope
- Define parser trait/interface.
- Register parsers internally by kind/language/content type.
- Return explicit unsupported diagnostics when no parser applies.
- Keep dynamic external plugins out of scope.

## Acceptance criteria
- Registry can dispatch to stub parsers.
- Unsupported inputs are represented explicitly.
- Feature-gated parsers can be registered cleanly.
