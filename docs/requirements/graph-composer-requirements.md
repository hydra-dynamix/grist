I reviewed the current paths. No edits made.

**Current State**
LLM interpretation is centralized but small in [normalization.rs](/mnt/d/apps/graph-composer/crates/graph-worker-inference/src/normalization.rs:60). It supports:

- `Namespace.Command(arg={...})` parsing
- balanced JSON-object extraction
- JSON-ish repair for `True`/`False`/`None`
- parse reports via callers, not inside the parser itself

Call sites already depend on this in composer, orchestrator, coding tools, and evals. The richer work-graph-specific repair logic is still local to [graph-worker-orchestrator/src/main.rs](/mnt/d/apps/graph-composer/crates/graph-worker-orchestrator/src/main.rs:336), especially field aliases like `nodes -> leaves`, `source -> from`, and default leaf fields.

Rust/code ingestion lives in [repo.rs](/mnt/d/apps/graph-composer/crates/graph-domain-coding-tools/src/repo.rs:244). It currently reads bounded file bytes, detects Rust by extension, then uses line heuristics in [rust_facts](/mnt/d/apps/graph-composer/crates/graph-domain-coding-tools/src/repo.rs:530). It extracts only simple public symbols, top-level imports, and tests. No tree-sitter yet.

**Requirements For The External Utility Library**
I’d make the new library cover two modules:

1. `model_output`
- Preserve current public behavior: `parse_model_output`, `parse_command_json_arg`, `parse_json_from_text`.
- Return a structured report equivalent to `NormalizedModelOutput`: `ok`, `grammar`, command, argument, value, normalizations, warnings, error.
- Support accepted command/arg allowlists.
- Keep trust boundaries out of scope: it should parse and normalize format only, then callers still deserialize and validate.
- Add reusable candidate extraction: fenced markdown, prose wrappers, command calls, first balanced object.
- Add pluggable schema-specific normalizers so work-graph, graph mutation, code realization, and review parsing can register alias/repair rules outside orchestrator code.

2. `rust_ingest`
- Accept `(root, rel_path, max_bytes)` or raw source plus path.
- Use `tree-sitter` + `tree-sitter-rust`.
- Emit a structure that can map directly into `CodeFacts`: symbols, imports, tests, line ranges, language, truncated/generated/vendor metadata.
- Capture more robust Rust constructs than the current heuristic:
  - `pub(crate)`, `pub(super)`, `pub(in ...)`
  - impl methods, trait items, macros, modules
  - nested modules
  - `#[test]`, `#[tokio::test]`, other attr-based test functions
  - grouped imports and aliases
- Use deterministic ordering and stable line ranges.
- Never read outside root; keep path normalization/root safety in the caller or expose a safe helper with the same behavior.

**Dependency Placement**
Best direct dependencies:

- `graph-worker-inference` should depend on the external library for `model_output`, then re-export or wrap its API to avoid changing all callers at once.
- `graph-domain-coding-tools` should depend on it for `rust_ingest`, replacing `rust_facts`.
- Do not put this dependency in core contracts. `graph-domain-coding-contracts` should stay as pure data contracts like `CodeFacts`, `CodeSymbol`, and `LineRange` in [lib.rs](/mnt/d/apps/graph-composer/crates/graph-domain-coding-contracts/src/lib.rs:216).

Minimal integration shape:

```toml
# graph-worker-inference/Cargo.toml
graph-interpretation = { path = "../graph-interpretation" }

# graph-domain-coding-tools/Cargo.toml
graph-interpretation = { path = "../graph-interpretation", features = ["rust-tree-sitter"] }
```

The clean first milestone is: move current normalization into the new lib unchanged, add tests proving parity, then replace `rust_facts` with tree-sitter output while keeping the existing `CodeFacts` JSON schema stable.