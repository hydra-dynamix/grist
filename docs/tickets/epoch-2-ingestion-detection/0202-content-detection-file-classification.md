# 0202 — Implement content detection and file classification

## Goal
Detect file kind, content type, and language using safe, lightweight rules.

## Dependencies
- 0201

## Scope
- Use extension, special filenames, shebangs, and lightweight sniffing.
- Classify source, test, manifest, lockfile, generated, binary, documentation, and unknown files.
- Include confidence/reason metadata where useful.
- Avoid execution and risky deep inspection.

## Acceptance criteria
- Common Rust/Markdown/JSON/YAML/TOML files are detected correctly.
- Binary files are identified and skipped/reported.
- Detection diagnostics explain ambiguity where relevant.
