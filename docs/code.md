# Code and repository format contract

Grist parses source as inert evidence; it never compiles, imports, evaluates or
runs it. Primary typed parsers cover Rust, Python, JavaScript/JSX and
TypeScript/TSX. The shared language registry covers C, C++, C#, CSS, Go, Java,
Kotlin, PHP, Ruby, shell, SQL and Swift. Manifest parsing covers common build,
dependency, container, CI and orchestration files without executing scripts,
templates, package managers or network resolution.

Payloads preserve declarations, syntax/unknown regions, comments, dependency
and reference evidence, exact source ranges and diagnostics. All code and
manifest selectors participate in detection, generated schemas, the CLI,
`DocumentGraph` projection and structural segmentation. Repository ingestion
adds explicit-root containment, ignore/skip outcomes, symlink/submodule policy,
budgets and deterministic identities.
