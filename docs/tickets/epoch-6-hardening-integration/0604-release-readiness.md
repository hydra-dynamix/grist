# 0604 — Release readiness hardening

## Goal
Prepare Grist for first practical integration/release.

## Dependencies
- 0603

## Scope
- Audit diagnostics for root-cause clarity.
- Confirm no network access or code execution by default.
- Confirm limits are centralized and generous by default.
- Verify schemas, CLI, docs, and dev workflows.
- Run full `dev ci`.

## Acceptance criteria
- `dev ci` passes.
- Public docs explain API/CLI basics and schema versions.
- Security/non-goal boundaries are documented.
- First integration consumers can depend on Grist's public interface.
