# 0303 — Complete Markdown tables, links, fences, and frontmatter

## Goal
Finish the rich Markdown structures required for instruction/docs ingestion.

## Dependencies
- 0301

## Scope
- Parse links with targets, labels, and ranges.
- Parse tables into structured rows/cells with ranges where available.
- Parse YAML frontmatter as raw + structured metadata.
- Improve fence info-string/language handling.

## Acceptance criteria
- Tables, links, frontmatter, and fences have golden fixtures.
- Frontmatter YAML errors are surfaced as diagnostics.
- Markdown tables can be consumed without general CSV parser support.
