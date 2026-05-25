# 0403 — Cover modern Rust constructs

## Goal
Expand Rust ingestion to represent modern real-world Rust codebases.

## Dependencies
- 0402

## Scope
- Cover visibility forms: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`, private, unknown.
- Cover trait items, impl methods, nested modules, macros, macro invocations, attributes, doc comments, tests, and async/unsafe/extern forms where relevant.
- Add fixtures for modern syntax encountered in target repos.

## Acceptance criteria
- Fixture matrix covers modern constructs listed in the spec.
- Gaps are documented as explicit follow-up parser gaps, not silent omissions.
- Parse diagnostics remain meaningful for partial files.
