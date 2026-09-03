# R Markdown and Quarto parser contract

The `notebooks` feature exposes `grist::formats::rmarkdown_quarto` and the compatible `grist::rmarkdown_quarto` path. R Markdown (`r_markdown`/`rmd`) and Quarto (`quarto`/`qmd`) use the authoritative `grist/markdown/v2` payload and add a resolved `dialect` plus typed dialect metadata.

## Source model

YAML/TOML frontmatter and the complete CommonMark/GFM tree retain their exact decoded and raw-byte ranges. Fenced executable blocks retain the engine, label, R-style header options, Quarto `#|` options, and exact source. Their `executed` field is always `false`; parsing has no code execution, subprocess, provider, or evaluation path. Malformed `#|` metadata produces `executable.metadata` and a partial payload without discarding the block.

Pandoc citations become located citation nodes and `References` graph edges. Markdown images and `fig-*`/`fig-cap` executable blocks carry typed figure records. Quarto `.cell-output*` blocks become stored-output nodes whose raw syntax is preserved for explicit raw-fallback rendering. Strict rendering reports unsupported raw output, raw-fallback rendering preserves it inertly, and lossy rendering records a loss.

## Local references

The parser recognizes bibliography/CSL/body-include frontmatter, Quarto include shortcodes, R Markdown `child`/`dependson` block options, and local figures. Resolution is disabled unless the caller supplies `MarkdownOptions::project_root`. Targets are canonicalized beneath that root, parent traversal and absolute paths are rejected, remote URLs are never fetched, regular-file status is required, and `max_reference_bytes` is enforced before reading. Resolved bytes, a project-relative path, and SHA-256 evidence are stored in the payload.

Stable diagnostics include `reference.project_root_required`, `reference.remote_disabled`, `reference.outside_project_root`, `reference.not_found`, `reference.not_file`, and `reference.budget_exceeded`.

## Public surfaces

Registry and CLI selectors are `r_markdown`/`rmd` and `quarto`/`qmd`. Both payloads project through the shared DocumentGraph, segmentation, normalized rendering, and JSON Schema surfaces. `tests/rmarkdown_quarto_universal_contract.rs` covers complete and malformed constructs, inertness, bounded references, graph/segment/schema/CLI routing, and rendering loss reports.