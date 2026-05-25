# 0603 — Validate Bathysphere, graph-composer, and research-chain compatibility

## Goal
Verify that Grist's interface meets the three immediate integration targets.

## Dependencies
- 0602

## Scope
- Map Grist repo/Rust outputs to Bathysphere orientation needs.
- Map model-output parser behavior to graph-composer normalization needs.
- Map document/evidence ingestion outputs to research-chain needs.
- Document any integration gaps as tickets.

## Acceptance criteria
- Each target project has a compatibility note or fixture.
- No consumer needs to reach into Grist internals for required data.
- Gaps are explicit and prioritized.
