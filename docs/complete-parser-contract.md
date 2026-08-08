# Complete parser contract and implementation gap matrix

This document adopts the archived **Grist Complete Parser Specification** from LDGR run 4 as the normative target. It is an inventory, not a reduced roadmap: `partial` and `absent` identify gaps and do not waive, defer, or narrow the target.

The source contains 155 operative `SHALL` occurrences, 3 operative uppercase `SHOULD` occurrences, and 11 operative uppercase `MAY` occurrences, excluding the modal-word declaration. The normative index accounts for every occurrence. One row may cover multiple occurrences in the same source paragraph; its counts preserve the total. Sections without uppercase modals have explicit scope rows so Sections 1–19 are all owned.

Status is strict: **conforming** means the entire clause is implemented as written; **partial** means useful implementation exists but required behavior or evidence is missing; **absent** means no implementation of the public target was found. No row in this baseline is a waiver. The verification owner is an exact LDGR work slug; `complete-parser-release-readiness` performs the final re-audit.

Fixture keys are `contract`, `detect`, `universal` (all §16.1 classes), `provenance`, `security`, `budget`, `provider`, `render`, `cli`, and `performance`. `behavior-only` means a verified rule that adds no standalone serialized type. Target feature names express architecture; `crate-module-feature-topology` owns final Cargo spelling and compatibility shims.

## Ownership profiles

Every normative row names one profile plus an explicit verification owner. The profile maps the clause to a public module, feature, schema family, diagnostic family, and fixture class.

| Profile | Public module | Target feature | Required schema family | Diagnostic family | Fixture class |
|---|---|---|---|---|---|
| ARCH | crate/module topology | family gates; `full`; `cli` | capabilities | `feature.*` | contract; cli |
| CORE | `grist::core`; `grist::ingest` | always | parse request/options/source | `request.*`; `input.*`; `options.*` | contract; budget |
| ENV | `grist::core`; all operations | always | envelope; operation status | `status.*`; `operation.*` | contract; universal |
| ID | `grist::core`; `grist::schema` | always; `schema` | content identity; canonicalization | `identity.*`; `determinism.*` | contract; provider |
| LOC | `grist::core`; graph; segment; container | always; graph; segment; container | source locator; provenance | `locator.*`; `provenance.*` | provenance |
| DIAG | `grist::core`; every emitter | always; every family | diagnostic; parser info; provenance | malformed; unsupported; loss; provider; defect; security; budget | contract; universal |
| CITE | `grist::core`; graph; segment | always; graph; segment | citation anchor/verification | `citation.*` | provenance |
| DETECT | `grist::detect` | always | detection; decoded identity | `detect.*`; `decode.*` | detect; provenance |
| FORMAT | `grist::formats::<family>`; detect; graph; schema; cli | family; `full`; `cli` | payload; envelope; graph | `format.<family>.*` | universal; provenance; security; cli |
| GRAPH | `grist::document_graph`; every format | `document-graph`; family | graph; payload extension | `graph.*` | contract; universal; provenance |
| CONTAINER | `grist::container`; core | `container`; archives | artifact; locator; budget allocation | `artifact.*`; `container.*`; `archive.*` | universal; provenance; security; budget |
| PROVIDER | `grist::provider`; using formats | `provider`; using family | provider invocation/result | `provider.*` | provider; security; provenance |
| SEGMENT | `grist::segment`; core locator | `segment` | segment/options | `segment.*` | contract; provenance; universal |
| RENDER | `grist::render`; `grist::transform` | `render`; family | render result/source map/fidelity | `render.*`; `reconstruct.*` | render; provenance |
| BUDGET | `grist::core`; ingest; large formats | always; large family | budget profile; event/cancellation | `budget.*`; `status.cancelled` | budget; performance |
| SECURITY | `grist::security`; every byte-facing module | every family | safety metadata; artifact | `security.*`; `parser.panic` | security; universal |
| CLI | `grist::cli`; registry; detect | `cli`; `full` | CLI envelope/event/capabilities | `cli.*`; `capability.*` | cli; detect |
| SCHEMA | `grist::schema`; every public module | `schema`; every family | every public type/migration/example | `schema.*`; `compatibility.*`; `migration.*` | contract; universal |
| VERIFY | tests; fixtures; fuzz; benches | every family; `full`; `cli` | golden/fixture/benchmark manifests | all expected families | all fixture classes |
| RUNTIME | runtime hooks; core identity; large formats | always; large family | metrics/cache/merge manifest | `metrics.*`; `cache.*`; `runtime.*` | contract; security; performance |
| DOCS | docs; crate-wide release | every family; `full`; `cli` | capability/format manifests | all | universal; contract |

## Normative occurrence index

Evidence keys: `E1` = `Cargo.toml`, `README.md`, `src/lib.rs`; `E2` = `src/core.rs`; `E3` = `src/detect.rs`; `E4` = current format modules; `E5` = `src/document_graph.rs` and graph golden; `E6` = `src/ingest.rs`; `E7` = `src/model_output.rs`; `E8` = `src/schema.rs` and `schemas/`; `E9` = `src/main.rs`, CLI docs/e2e; `E10` = current unit/e2e tests; `E0` = tracked-file inventory proves the target surface absent. Each `partial` row names evidence but retains the full clause by source reference.

| ID | Ref | SHALL | Advisory | Profile | Verification owner | Baseline / evidence |
|---|---:|---:|---:|---|---|---|
| CP-01-SCOPE | §1 | 0 | 0 | DOCS | complete-parser-release-readiness | **partial** E1 — parser boundary exists; complete owned surface absent. |
| CP-01-TOOLCHAIN | §1 | 0 | 0 | ARCH | feature-ci-release-matrix | **conforming** E1 — `edition = 2024` and `rust-version = 1.85` match the specification header. |
| CP-02-01 | §2.1 | 1 | 0 | DIAG | parser-promotion-harness | **partial** E4/E5 — no universal no-silent-loss gate. |
| CP-02-02 | §2.2 | 1 | 0 | GRAPH | all-format-document-graph-projections | **partial** E5 — subset projections only. |
| CP-02-03 | §2.3 | 1 | 0 | LOC | core-source-locator | **partial** E2/E5 — text ranges only. |
| CP-02-04 | §2.4 | 1 | 0 | ENV | core-envelope-operation-status | **absent** E2 — mandatory payload; no operation status. |
| CP-02-05 | §2.5 | 1 | 0 | ID | core-content-identity-canonical-json | **partial** E2 — SHA-256 exists; canonical JSON absent. |
| CP-02-06 | §2.6 | 1 | 0 | SECURITY | universal-security-fuzz-suite | **partial** E4 — current parsers inert; universal enforcement absent. |
| CP-02-07 | §2.7 | 1 | 0 | PROVIDER | provider-contracts | **absent** E0. |
| CP-02-08 | §2.8 | 1 | 0 | LOC | embedded-artifact-contract | **absent** E2. |
| CP-02-09 | §2.9 | 1 | 0 | DIAG | core-diagnostics-provenance | **partial** E7 — normalization strings are not provenance steps. |
| CP-02-10 | §2.10 | 2 | 0 | BUDGET | core-resource-budget-cancellation | **partial** E2 — four cutoffs only. |
| CP-02-11 | §2.11 | 1 | 0 | SCHEMA | schema-compatibility-migrations | **partial** E2/E8 — current versions/drift only. |
| CP-03-ARCH | §3 | 3 | 0 | ARCH | crate-module-feature-topology | **partial** E1 — no `full` or complete family topology. |
| CP-04-01-REQUEST | §4.1 | 1 | 0 | CORE | core-parse-request-input | **absent** E4 — unrelated entry signatures. |
| CP-04-01-INPUT | §4.1 | 1 | 0 | CORE | core-parse-request-input | **partial** E4/E6 — paths/strings, not all input variants. |
| CP-04-01-OPTIONS | §4.1 | 2 | 0 | CORE | core-parse-request-input | **partial** E4 — typed per-module options, no shared boundary. |
| CP-04-02-ENVELOPE | §4.2 | 1 | 0 | ENV | core-envelope-operation-status | **partial** E2/E9 — operations inconsistent. |
| CP-04-02-VERSIONING | §4.2 | 2 | MAY1 | ENV | core-envelope-operation-status | **partial** E2 — two versions; payload not optional. |
| CP-04-03 | §4.3 | 1 | 0 | ENV | core-envelope-operation-status | **absent** E2 — seven statuses absent. |
| CP-04-04-SOURCE | §4.4 | 2 | 0 | CORE | core-parse-request-input | **partial** E2 — path/display name only. |
| CP-04-04-IDENTITY | §4.4 | 1 | 0 | ID | core-content-identity-canonical-json | **partial** E2/E3 — identity incomplete. |
| CP-04-04-HASHING | §4.4 | 2 | 0 | ID | core-content-identity-canonical-json | **partial** E2 — canonicalization absent. |
| CP-04-05-TAGGED | §4.5 | 1 | 0 | LOC | core-source-locator | **absent** E2. |
| CP-04-05-NESTING | §4.5 | 2 | SHOULD1 | LOC | core-source-locator | **partial** E2 — text conventions only. |
| CP-04-05-DERIVED | §4.5 | 2 | 0 | LOC | core-source-locator | **absent** E0. |
| CP-04-06-SHAPE | §4.6 | 1 | 0 | DIAG | core-diagnostics-provenance | **partial** E2 — required fields missing. |
| CP-04-06-FAMILIES | §4.6 | 1 | 0 | DIAG | core-diagnostics-provenance | **partial** E2/E4 — no complete taxonomy. |
| CP-04-07-PARSER | §4.7 | 1 | 0 | DIAG | core-diagnostics-provenance | **partial** E2 — name/version only. |
| CP-04-07-PROVENANCE | §4.7 | 1 | 0 | DIAG | core-diagnostics-provenance | **absent** E0. |
| CP-04-08-ANCHOR | §4.8 | 2 | 0 | CITE | core-citation-anchor | **absent** E0. |
| CP-04-08-VERIFY | §4.8 | 3 | MAY1 | CITE | core-citation-anchor | **absent** E0. |
| CP-05-SIGNALS | §5 | 1 | 0 | DETECT | detect-ranked-format | **partial** E3 — required signals incomplete. |
| CP-05-RANK | §5 | 3 | 0 | DETECT | detect-ranked-format | **partial** E3 — one selection, no ranked ambiguity. |
| CP-05-DECODE | §5 | 3 | MAY1 | DETECT | decode-charset | **absent** E3 — required charset contract missing. |
| CP-06-SURFACE | §6 | 1 | 0 | FORMAT | all-format-cli-schema-integration | **partial** E1/E4/E8/E9 — small current subset only; inventory below is authoritative. |
| CP-06-01-INCLUDES | §6.1 | 1 | 0 | FORMAT | format-restructuredtext | **conforming** — reStructuredText has explicit project-root containment, cycle/depth/byte checks, and reference-only diagnostics; AsciiDoc remains queued. |
| CP-06-01-ACTIVE | §6.1 | 1 | 0 | SECURITY | format-html-xhtml | **conforming** for HTML/XHTML — scripts, forms, event handlers, active URLs, media, frames, and external XHTML identifiers are retained and typed but never run or fetched; XML/JATS remains separately queued. |
| CP-06-02-LOCATORS | §6.2 | 1 | MAY1 | LOC | format-latex-pdf-association | **absent** E0. |
| CP-06-02-NOEXEC | §6.2 | 1 | 0 | SECURITY | format-latex-project | **partial** E4 — single-file parser inert; project isolation absent. |
| CP-06-03-COVERAGE | §6.3 | 1 | 0 | FORMAT | format-pdf-core | **partial** E4 — container identity, object/xref structure, catalog/page tree, page labels/geometry, metadata, filters, encryption state, repairs, inert active-content inventory, and native glyph/token/line/block/font/reading-order layout are implemented; semantic/OCR/interactive payload work remains queued. |
| CP-06-03-PAYLOAD | §6.3 | 1 | 0 | FORMAT | format-pdf-semantic-structure | **absent** E0 — all ten payload groups required. |
| CP-06-03-RECONCILE | §6.3 | 3 | 0 | PROVIDER | format-pdf-ocr-reconciliation | **conforming** — explicit per-page/region provider calls retain native, OCR, and reconciled identities; deterministic recording replay, provider failure preservation, confidence evidence, locators, graph/segments, and named duplicate suppression are covered. |
| CP-06-04-MACRO | §6.4 | 1 | 0 | SECURITY | format-word-ooxml-revisions-objects | **absent** E0. |
| CP-06-04-PAYLOAD | §6.4 | 1 | 0 | FORMAT | format-word-ooxml-content | **absent** E0 — every listed payload group required. |
| CP-06-04-REVISION | §6.4 | 1 | 0 | RENDER | format-word-ooxml-revisions-objects | **absent** E0. |
| CP-06-05-ORDER | §6.5 | 1 | 0 | FORMAT | format-presentation-content-order | **absent** E0. |
| CP-06-05-NOEXEC | §6.5 | 1 | 0 | SECURITY | format-presentation-content-order | **absent** E0. |
| CP-06-06-SQLITE | §6.6 | 2 | 0 | FORMAT | format-sqlite-readonly | **conforming** — raw format-3 inspection preserves schema/tables/views/indexes and bounded selected records with stable locators; no SQL engine, extensions, triggers, functions, journals, or writes execute. |
| CP-06-06-FORMULAS | §6.6 | 3 | 0 | SECURITY | format-spreadsheet-ooxml | **conforming** — XLSX/XLSM formulas are inert source with explicitly labeled workbook-stored caches; no calculation or macro execution occurs. |
| CP-06-07-SECURE | §6.7 | 2 | 0 | PROVIDER | format-secure-message-parts | **absent** E0. |
| CP-06-07-CALENDAR | §6.7 | 2 | 0 | SECURITY | format-calendar-contact | **absent** E0. |
| CP-06-08 | §6.8 | 1 | 0 | FORMAT | format-jupyter-notebook | **conforming** - nbformat 3/4 cells, metadata, attachments, execution counts, rich MIME/error/widget outputs, locators, stable graph/segment identities, schemas, CLI, limits, and no-execution proof are covered. |
| CP-06-09 | §6.9 | 1 | 0 | FORMAT | ingest-repository | **partial** E3/E6 — ignore/outcomes exist; complete skip/symlink/submodule/identity contract missing. |
| CP-06-10 | §6.10 | 1 | 0 | CONTAINER | container-safe-recursion | **absent** E0. |
| CP-06-11-OCR | §6.11 | 1 | 0 | PROVIDER | format-image-ocr-layout | **absent** E0. |
| CP-06-11-SUBTITLE | §6.11 | 1 | 0 | FORMAT | format-subtitles | **absent** E0. |
| CP-06-11-MEDIA | §6.11 | 1 | 0 | PROVIDER | format-media-container | **absent** E0. |
| CP-06-12-SURFACE | §6.12 | 1 | 0 | FORMAT | model-output-streaming | **partial** E7 — batch/event types exist; full stream contract missing. |
| CP-06-12-SEMANTICS | §6.12 | 3 | MAY1 | FORMAT | model-output-repair-validation | **partial** E7 — candidate retention/ambiguity/validation exist; typed repair provenance missing. |
| CP-07-PAYLOAD-GRAPH | §7 | 2 | 0 | GRAPH | all-format-document-graph-projections | **partial** E5 — subset only. |
| CP-07-01-VOCAB | §7.1 | 1 | 0 | GRAPH | document-graph-contract | **partial** E5 — many required kinds absent. |
| CP-07-01-UNKNOWN | §7.1 | 2 | 0 | GRAPH | document-graph-contract | **partial** E5 — attrs/`Other`; not universally preserved. |
| CP-07-02-VOCAB | §7.2 | 1 | 0 | GRAPH | document-graph-contract | **partial** E5 — many required relations absent. |
| CP-07-02-EDGE | §7.2 | 2 | 0 | GRAPH | document-graph-contract | **partial** E5 — optional text range/attrs; no typed inference metadata. |
| CP-07-03-ID | §7.3 | 3 | 0 | GRAPH | document-graph-stable-identity | **partial** E5 — required identity inputs/edit stability contract absent. |
| CP-07-03-ORDER | §7.3 | 2 | 0 | GRAPH | document-graph-stable-identity | **absent** E5 — no universal canonical/parallel merge contract. |
| CP-08-ARTIFACT | §8 | 1 | 0 | CONTAINER | embedded-artifact-contract | **partial** E6 — repo artifact subset only. |
| CP-08-BUDGET | §8 | 2 | 0 | CONTAINER | container-safe-recursion | **absent** E0. |
| CP-08-MATERIALIZE | §8 | 2 | 0 | CONTAINER | embedded-artifact-contract | **partial** E6 — no general compound materialization. |
| CP-09-TRAITS | §9 | 1 | 0 | PROVIDER | provider-contracts | **absent** E0. |
| CP-09-INVOCATION | §9 | 4 | 0 | PROVIDER | provider-contracts | **absent** E0. |
| CP-09-RESULT | §9 | 2 | MAY1 | PROVIDER | provider-contracts | **absent** E0. |
| CP-10-BOUNDARY | §10 | 2 | 0 | SEGMENT | segment-deterministic-engine | **absent** E0. |
| CP-10-OPTIONS | §10 | 1 | 0 | SEGMENT | segment-deterministic-engine | **absent** E0. |
| CP-10-SEGMENT | §10 | 3 | 0 | SEGMENT | all-format-segmentation-provenance | **absent** E0. |
| CP-11-RENDER | §11 | 2 | 0 | RENDER | render-normalized-source-maps | **partial** E5 — Markdown/LaTeX only; modes incomplete. |
| CP-11-SOURCEMAP | §11 | 2 | 0 | RENDER | render-normalized-source-maps | **absent** E5 — strings/warnings, no generated-range map. |
| CP-11-RECONSTRUCT | §11 | 1 | 0 | RENDER | transform-reconstruction-contract | **absent** E0. |
| CP-12-BUDGET | §12 | 1 | 0 | BUDGET | core-resource-budget-cancellation | **partial** E2 — four axes versus complete set. |
| CP-12-EXPLICIT | §12 | 2 | 0 | BUDGET | core-resource-budget-cancellation | **absent** E2 — APIs cannot express all limit-hit states. |
| CP-12-UNBOUNDED | §12 | 1 | MAY1; SHOULD1 | BUDGET | core-resource-budget-cancellation | **absent** E0. |
| CP-12-STREAM | §12 | 3 | 0 | BUDGET | core-resource-budget-cancellation | **partial** E7 — event types only; general stream/cancel absent. |
| CP-12-INCREMENTAL | §12 | 2 | MAY1 | RUNTIME | observability-cache-parallelism | **conforming** — exact content/options/parser/provider keys validate canonical entries; reuse is explicit provenance and leaves cached bytes unchanged. |
| CP-13-HOSTILE | §13 | 1 | 0 | SECURITY | security-isolation-secrets | **partial** E4/E6 — inert subset; eleven controls not universal. |
| CP-13-PANIC | §13 | 1 | 0 | SECURITY | universal-security-fuzz-suite | **absent** E0 — no `fuzz/` tree/public no-panic suite. |
| CP-13-INVENTORY | §13 | 1 | 0 | SECURITY | universal-security-fuzz-suite | **absent** E0. |
| CP-14-SURFACE | §14 | 1 | 0 | CLI | cli-complete-surface | **partial** E9 — required commands/routing incomplete. |
| CP-14-OUTPUT | §14 | 3 | MAY1 | CLI | cli-complete-surface | **partial** E9 — current JSON/errors; universal envelopes/fidelity manifest absent. |
| CP-14-INPUT | §14 | 3 | 0 | CLI | cli-complete-surface | **partial** E9 — MIME/kind ambiguity and batch request IDs absent. |
| CP-14-CAPABILITIES | §14 | 1 | 0 | CLI | capabilities-manifest | **absent** E9. |
| CP-15-SCHEMAS | §15 | 1 | 0 | SCHEMA | schema-compatibility-migrations | **partial** E8 — current subset only. |
| CP-15-PATCH | §15 | 1 | MAY1 | SCHEMA | schema-compatibility-migrations | **absent** E0. |
| CP-15-MIGRATION | §15 | 1 | 0 | SCHEMA | schema-compatibility-migrations | **absent** E0. |
| CP-15-DRIFT | §15 | 1 | 0 | SCHEMA | schema-compatibility-migrations | **partial** E8 — generated schemas, no canonical-example/full CI gate. |
| CP-15-UNKNOWN | §15 | 1 | 0 | SCHEMA | schema-compatibility-migrations | **partial** E5 — some `Other(String)`, not universal. |
| CP-15-BACKEND | §15 | 1 | 0 | SCHEMA | schema-compatibility-migrations | **absent** E0. |
| CP-16-PROMOTION | §16 | 1 | 0 | VERIFY | parser-promotion-harness | **conforming** — the shared adapter-driven harness fails closed and emits a deterministic report. |
| CP-16-FIXTURES | §16.1 | 0 | 0 | VERIFY | fixture-corpus-foundation | **partial** E0 ? governed manifest, licensing/sensitivity policy, deterministic builders/recordings, canonical goldens, and regression intake conform; per-parser corpus population remains format-owned. |
| CP-16-TESTS | §16.2 | 0 | 0 | VERIFY | parser-promotion-harness | **conforming** — every verification kind is mandatory; only differential and round-trip permit justified non-applicability. |
| CP-16-GATES | §16.3 | 0 | 0 | VERIFY | parser-promotion-harness | **partial** E10 — all eleven gates are executable and schema-backed; each format still needs its own passing promotion report. |
| CP-17-METRICS | §17 | 1 | SHOULD1 | RUNTIME | observability-cache-parallelism | **conforming** — backend-neutral hooks expose typed counters, timing, status, budget, cache, and parallel-unit measurements. |
| CP-17-PRIVACY | §17 | 1 | 0 | RUNTIME | observability-cache-parallelism | **conforming** — the default metric schema has no arbitrary string/source/metadata fields and integration tests reject content leakage. |
| CP-17-PARALLEL | §17 | 3 | 0 | RUNTIME | observability-cache-parallelism | **conforming** — pages, sheets, slides, members, and files share bounded workers and canonical-order merge/delivery; serial/parallel canonical bytes match. |
| CP-17-CACHE | §17 | 2 | MAY1 | RUNTIME | observability-cache-parallelism | **conforming** — callers own persistence behind a validated content-addressed interface and Grist owns key/canonical-entry/reuse contracts. |
| CP-18-TOPOLOGY | §18 | 1 | 0 | ARCH | crate-module-feature-topology | **partial** E1 — target trees and fixtures/fuzz/benches missing. |
| CP-18-DOCS | §18 | 1 | 0 | DOCS | format-contract-documentation | **absent** E0 — no exact document for every format. |
| CP-19-DOD | §19 | 0 | 0 | DOCS | complete-parser-release-readiness | **partial** E1/E10 — complete surface/gates not present; nothing waived. |

The fixed totals are **155 SHALL**, **3 SHOULD**, and **11 MAY**. `tests/complete_parser_contract.rs` checks the totals, all 19 sections, ownership profiles, statuses, and inventories below.

## Complete required format inventory

Every row inherits §6: detection, authoritative typed payload, meaningful graph projection, JSON Schema, CLI routing, malformed/adversarial fixtures, and provenance tests. `partial` never makes a listed member optional.

| Family | Required formats/constructs | Target module and feature | Verification owner | Baseline evidence |
|---|---|---|---|---|
| Text | Plain text with encoding/newline preservation | formats/text; `text-publishing` | format-plain-text | **partial** — `src/text.rs`, UTF-8 blocks only. |
| Text | Markdown CommonMark/GFM and every listed extension/raw node | formats/markdown; `text-publishing` | format-markdown | **implemented** — v2 byte/decode fidelity, typed syntax, raw extensions, exact locators, graph/segment/render projections, and 11-gate promotion evidence. |
| Text | reStructuredText | formats/restructured_text; `text-publishing` | format-restructuredtext | **implemented** — v1 byte/decode fidelity, headings, inert directives/roles, bounded includes, tables, code, footnotes/citations, cross-references, raw fallbacks, graph/segment/schema/CLI parity, and 11-gate promotion evidence. |
| Text | AsciiDoc | formats/asciidoc; `text-publishing` | format-asciidoc | **implemented** — v1 byte/decode fidelity, headings, attributes/roles, inert macros, bounded includes, tables, code, footnotes, cross-references, raw fallbacks, graph/segment/schema/CLI parity, and 11-gate promotion evidence. |
| Markup | HTML5 documents/fragments and XHTML | formats/html; `text-publishing` | format-html-xhtml | **implemented** — html5ever DOM recovery plus exact lexical order, namespaces/attributes, byte decoding, metadata, links, tables, media, semantic sections, raw unknown elements, inert active content, graph/segment/render/schema/CLI parity, and 11-gate promotion evidence. |
| Markup | XML | formats/xml; `text-publishing` | format-xml-jats-core | **implemented** — namespace-aware inert pull parsing preserves attributes, ordered text, exact XML paths, metadata, links, tables, media, entity records, and raw unknown/recovery content with graph/segment/render/schema/CLI parity and 11-gate promotion evidence. |
| Scholarly | JATS XML with cross-links | formats/jats; `scholarly` | format-jats-scholarly-links | **absent**. |
| Publishing | EPUB 2 and EPUB 3 | formats/epub; `text-publishing` | format-epub | **conforming** — package/container metadata, manifest, spine, EPUB 3 nav, EPUB 2 NCX, chapters, notes, images, styles, embedded resources, nested identity/locators, deterministic graph projection, schemas, and bounded archive security are implemented. |
| Scholarly | LaTeX projects/source maps | formats/latex; `scholarly` | format-latex-project | **partial** — `src/latex.rs` single source only. |
| Scholarly | BibTeX and BibLaTeX | formats/bibliography; `scholarly` | format-bibliography | **implemented** ? v1 exact entry/field/value provenance, strings and bounded expansion, crossref/xdata inheritance, duplicate-safe citation APIs, comments/raw recovery, graph/schema/CLI parity, and malformed/adversarial contract coverage. |
| Scholarly | LaTeX-to-compiled-PDF association | formats/latex; `scholarly`; `pdf` | format-latex-pdf-association | **absent**. |
| PDF | Born-digital, scanned, hybrid PDF and every §6.3 payload group | formats/pdf; `pdf` | format-pdf-core | **partial** — core structure, native layout, semantic structure, inert interactive/embedded content, and explicit scanned/hybrid OCR reconciliation are implemented with stable locators, confidence, provider provenance, and named losses; remaining cross-format integration gates stay queued. |
| Word | DOCX, DOCM, DOTX, DOTM | formats/word_ooxml; `word-processing` | format-word-ooxml-content | **partial** -- bounded OPC parsing plus producing-order paragraphs/runs, styles and direct formatting, headings/sections/breaks/columns, original and resolved numbering, merged/nested tables, links/bookmarks/fields/citations, notes, headers/footers, exact locators, and graph/segment/render/schema/CLI surfaces are implemented; revisions and rich objects remain queued. |
| Word | Legacy DOC, WordProcessingML 2003, Flat OPC | formats/word_legacy; `word-processing` | format-legacy-word-backend | **absent**. |
| Word | ODT and OTT | formats/odf_word; `word-processing` | format-odf-word | **conforming** ? bounded inert OpenDocument packages preserve package/manifest metadata, settings, styles, paragraphs, sections, lists, merged and nested tables, links, notes, annotations, tracked revisions with visible/original/accepted/rejected views, drawings, MathML, embedded objects, unknown XML, exact package/XML locators, and deterministic graph/segment/render/schema/CLI surfaces; malformed, encrypted, hostile, and budget-limited inputs are explicit. |
| Word | RTF | formats/rtf; `word-processing` | format-rtf | **absent**. |
| Presentation | PPTX, PPTM, POTX, PPSX | formats/presentation_ooxml; `presentations` | format-presentation-ooxml-package | **partial** — bounded inert OPC parsing preserves relationships, metadata, producing slide order and stable identities, masters/layouts/themes, actions, macro quarantine, embedded artifacts, exact locators, and graph/schema/CLI surfaces; detailed slide content and inferred reading order remain queued. |
| Presentation | ODP and OTP | formats/presentation_odf; `presentations` | format-presentation-odf | **conforming** — bounded inert OpenDocument packages preserve masters/styles/layouts, slide geometry and nested text, notes/comments, repeated and merged tables, embedded charts/images/objects, links, transitions/animations, raw unknown XML, exact package/XML/slide locators, deterministic confidence-bearing reading order, and graph/segment/schema/CLI surfaces; malformed, encrypted, hostile, and budget-limited inputs are explicit. |
| Presentation | Legacy PPT | formats/presentation_legacy; `presentations` | format-legacy-powerpoint-backend | **absent**. |
| Delimited | CSV and TSV | formats/delimited; `structured-data` | format-delimited-data | **partial** — `src/csv.rs`; TSV/full fidelity missing. |
| Spreadsheet | XLSX, XLSM, XLSB | formats/spreadsheet_ooxml; `spreadsheets` | format-spreadsheet-ooxml | **partial** — XLSX/XLSM have bounded read-only OOXML parsing that preserves workbook structure, named ranges, cell source/display/cache identity, styles, comments, links, merges, tables, charts/images, hidden state, panes, metadata, exact locators, and quarantined macros across graph/segment/schema/CLI surfaces; XLSB remains absent under format-spreadsheet-binary. |
| Spreadsheet | ODS and OTS | formats/spreadsheet_odf; `spreadsheets` | format-spreadsheet-odf | **conforming** -- bounded inert OpenDocument parsing preserves workbook/package metadata, repeated rows and cells, source formulas and labeled caches, stored/displayed values, styles, comments, links, merges, charts/images/objects, hidden state, panes, exact locators, raw extensions, and deterministic graph/segment/schema/CLI surfaces; malformed, encrypted, hostile, large, and budget-limited packages are explicit. |
| Spreadsheet | Legacy XLS and SpreadsheetML 2003 | formats/spreadsheet_legacy; `spreadsheets` | format-spreadsheet-binary | **absent**. |
| Structured text | JSON, JSONL/NDJSON, YAML, TOML | formats/structured_text; `structured-data` | format-structured-text-data | **conforming** — v2 source-ordered typed trees preserve duplicate keys, arbitrary scalar spellings, YAML tags/anchors/non-expanding aliases, lazy record streams, raw malformed recovery, exact pointers/ranges, budgets, graph/segment/schema/CLI parity, and deterministic identity. |
| Structured text | XML structured values | formats/xml; `structured-data` | format-structured-text-data | **conforming** — the secure generic XML payload projects ordered attributes and mixed content into typed values while retaining namespaces, raw unknowns, exact XML paths, security findings, and malformed recovery. |
| Structured binary | CBOR, MessagePack, descriptor-supplied Protocol Buffers | formats/structured_binary; `structured-data` | format-structured-binary-data | **conforming** — typed CBOR and MessagePack values preserve tags, extensions, non-JSON scalars, arbitrary/duplicate map keys, concatenated records, and exact byte/record locators; descriptor-set-driven Protobuf preserves schema identity, nested/scalar/repeated/packed/map/enum/oneof/extension fields and raw unknown wire fields. Limits, malformed recovery, deterministic JSON/graph/segment/schema/CLI surfaces, and inert security behavior are explicit. |
| Columnar | Apache Arrow IPC and Parquet | formats/columnar; `structured-data` | format-columnar-data | **conforming**: Arrow file/stream and Parquet preserve schemas, metadata, physical batches/row groups, typed/nested/dictionary/null values, stable record locators, projection, and bounded streaming. |
| Database | SQLite strictly read-only | formats/sqlite; `structured-data` | format-sqlite-readonly | **conforming** — inert raw-page parsing exposes headers, schema SQL, tables, views, indexes, selected records, exact byte/record locators, graph/schema/CLI surfaces, and explicit malformed/locked/sidecar/budget status without an execution or write path. |
| Email | EML/RFC 5322 and MIME | formats/email; `email-message` | format-email-mime | **conforming** — byte-oriented inert parsing preserves ordered/folded headers, encoded textual facts, addresses, dates/zones, IDs and thread evidence, MIME hierarchy/alternatives, text, inline resources, signatures/encryption inventory, attachment identities, exact MIME/byte locators, local recursive child parsing, graph/schema/CLI surfaces, and explicit malformed/budget diagnostics without remote resolution. |
| Email | MBOX | formats/mbox; `email-message` | format-mbox | **conforming** — mboxo/mboxrd/mboxcl/mboxcl2 inputs stream in source order with exact separator, line-ending, escaping, Content-Length, and nested message provenance; decoded-content identities remain stable across unrelated edits and distinguish duplicates; malformed/truncated messages recover explicitly; MIME attachments share one budget tree; aggregate thread evidence, graph, schema, detection, and CLI routing are integrated. |
| Email | MSG | formats/outlook; `email-message` | format-outlook-msg | **conforming** — bounded MS-CFB and MAPI parsing preserves property tables and value streams, named and unknown properties, recipients, sender/thread/date evidence, plain/HTML/compressed-RTF alternatives, attachments and recursive embedded messages, exact compound-object/byte locators, encrypted/signed status, and graph/segment/schema/CLI surfaces while keeping HTML, OLE, executables, links, and attachments inert. |
| Email | PST and OST | formats/outlook; `email-message` | format-outlook-store | **absent**. |
| Email | TNEF and S/MIME | formats/secure_message; `email-message` | format-secure-message-parts | **absent**. |
| Calendar/contact | iCalendar/ICS and vCard/VCF including MIME parts | formats/calendar_contact; `email-message` | format-calendar-contact | **conforming** - bounded inert parsing preserves folded/raw properties, events, alarms, time zones, recurrence and exceptions, attendees, multi-version contacts, URI/binary attachments, exact record locators, graph/segment/schema/CLI surfaces, and calendar MIME parts without writes, responses, or network access. |
| Notebook | Jupyter Notebook/IPYNB | formats/jupyter; `notebooks` | format-jupyter-notebook | **conforming** - inert nbformat 3/4 parsing preserves ordered typed cells, exact source/metadata/attachments, execution counts, rich MIME/error/widget outputs, explicit notebook locators and ownership, deterministic identities, bounded malformed/deep/large handling, graph/segment/schema/CLI surfaces, and never executes cells or active output. |
| Notebook | R Markdown and Quarto | formats/rmarkdown_quarto; `notebooks` | format-rmarkdown-quarto | **implemented** — Markdown v2 dialect payloads preserve frontmatter, inert executable metadata, citations, figures, stored outputs, and explicit-root bounded local references with graph/segment/render/schema/CLI parity. |
| Code | Tree-sitter Rust, Python, JavaScript, TypeScript, TSX, JSX | formats/code; `code` | format-code-primary-languages | **partial** — Rust/Python/TypeScript/TSX/JSX modules; JS/full semantics incomplete. |
| Code | Go, Java, Kotlin, C, C++, C#, Ruby, PHP, Swift, Bash, SQL, CSS registry | formats/code; `code` | format-code-language-registry | **absent**. |
| Repository | Cargo, npm/pnpm/yarn, Python packaging, Maven/Gradle, Go, Dockerfile, Compose, CI, Kubernetes manifests/lockfiles | formats/manifests; `code` | format-manifests-lockfiles | **conforming** - typed inert payloads retain raw declarations, dependency/reference provenance and exact locators; malformed and unknown syntax remain recoverable; detection, graph, schema, and registry-backed CLI routing are integrated without script, template, or network execution. |
| Archive | ZIP, ZIP64, TAR | formats/archive; `archives` | format-archive-zip-tar | **conforming** — ordered path/header/compression/hash inventory, ZIP64 and TAR metadata, encrypted/unsupported outcomes, nested traversal, graph/schema/CLI surfaces, and mode-invariant identities share bounded traversal/collision/link/device/bomb policy. |
| Compression | GZIP, BZIP2, XZ, Zstandard, 7z, compound packages | formats/compression; `archives` | format-compression-7z | **absent**. |
| Image | PNG, JPEG, TIFF, WebP, GIF, BMP, HEIF/HEIC, SVG, camera metadata | formats/image; `media` | format-image-native | **absent**. |
| Subtitle | SRT, WebVTT, TTML, embedded subtitle tracks | formats/subtitle; `media` | format-subtitles | **absent**. |
| Timed media | Audio/video metadata, chapters, streams, subtitles | formats/media; `media` | format-media-container | **absent**. |
| Model output | JSON variants, OpenAI calls, MCP JSON-RPC, opt-in Python calls, YAML/TOML/XML tool blocks, candidates/streams/repairs/aliases/schema validation | `grist::model_output`; `model-output` | model-output-candidate-batch | **partial** — `src/model_output.rs`; typed repairs/true streaming incomplete. |

## Required fixture classes

| §16.1 class | Verification owner | Baseline |
|---|---|---|
| Minimal valid, representative real-world, maximum-complexity, empty, truncated, malformed, adversarial, encrypted, oversized, deeply nested | fixture-corpus-foundation | **partial** ? every class is typed and governed; deterministic maximum-complexity/nested/oversized seeds exist, while format-owned population remains. |
| Mixed encoding and invalid text | decode-charset | **absent**. |
| Nested attachments and containers | container-safe-recursion | **absent**. |
| Unsupported constructs retained as raw/unknown | parser-promotion-harness | **partial** in a few parsers; no universal evidence. |
| Deterministic OCR/transcription provider recordings | provider-contracts | **conforming** ? secret-free catalogs recompute request/output digests and replay exact network-denied OCR/transcription requests. |
| Malicious active content proving no execution/network | universal-security-fuzz-suite | **absent** as a universal suite. |
| Downstream parser regressions | fixture-corpus-foundation | **partial** ? governed public/external-only intake is implemented; individual downstream cases remain format-owned. |

## Required test types

| §16.2 test type | Verification owner | Baseline |
|---|---|---|
| Unit tests for decoding, locators, hashing, diagnostics, format primitives | parser-promotion-harness | **partial** — module tests exist; contract coverage incomplete. |
| Goldens for payloads, graph, segments, schemas, CLI envelopes | universal-golden-compatibility-suite | **partial** — schema drift and one graph golden; no full matrix. |
| Property and fuzz tests for every byte-facing parser/repair path | universal-security-fuzz-suite | **absent**. |
| Differential tests against authoritative readers where licensing permits | parser-promotion-harness | **absent**. |
| Round-trip tests for reconstructable claims | transform-reconstruction-contract | **absent**. |
| Source-map tests for every emitted node | all-format-document-graph-projections | **absent**. |
| Concurrency and cancellation tests | universal-performance-streaming-suite | **absent**. |
| Schema backward-compatibility and migration tests | universal-golden-compatibility-suite | **absent** beyond same-version drift. |
| Resource-budget and decompression-bomb tests | universal-security-fuzz-suite | **absent**. |
| Panic, leak, path-escape, active-content security tests | universal-security-fuzz-suite | **absent**. |
| Performance benchmarks with checked thresholds | universal-performance-streaming-suite | **absent**; no `benches/` tree. |

## Universal parser acceptance gates

| §16.3 gate | Verification owner | Baseline |
|---:|---|---|
| 1. Detection covers valid, mislabeled, extensionless, malformed, ambiguous | detect-ranked-format | **implemented for the current registry surface** with ranked typed evidence, decisive magic/package policy, parser availability, and configurable ambiguity outcomes; each future parser still owns promotion fixtures. |
| 2. Every construct typed, raw-retained, or specifically diagnosed | parser-promotion-harness | **partial**; no universal construct accounting. |
| 3. Exact or explicitly approximate locator for every fact | core-source-locator | **absent** beyond optional text ranges. |
| 4. Complete/partial/failed/encrypted/unsupported/budget distinction | core-envelope-operation-status | **absent**. |
| 5. Deterministic bytes/options/backend/provider output | core-content-identity-canonical-json | **partial**; no canonical JSON/provider guarantee. |
| 6. Payload projects to graph without silent loss | all-format-document-graph-projections | **partial** for subset; most formats absent. |
| 7. Segmentation preserves identity and locators | all-format-segmentation-provenance | **absent**. |
| 8. Schemas, canonical examples, Rust API, CLI agree | all-format-cli-schema-integration | **partial** for current schemas/CLI. |
| 9. Hostile input cannot execute, fetch, escape, expand unboundedly, or panic | universal-security-fuzz-suite | **absent** as a universal gate. |
| 10. Real-world, malformed, adversarial, fuzz, provenance, regression fixtures | fixture-corpus-foundation | **partial** ? shared provenance and intake governance exists; parser promotion still requires per-format evidence. |
| 11. Feature CI, lint, formatting, schema drift, docs pass | feature-ci-release-matrix | **partial** — project commands exist; complete matrix absent. |

## Hostile-document controls

The single `SHALL` at §13 introduces all eleven controls; none is collapsed into a generic “safe parser” claim.

| Required control | Verification owner | Baseline |
|---|---|---|
| Never execute code, scripts, macros, formulas, notebook cells, actions, executables | universal-security-fuzz-suite | **partial** for current inert parsers; full format inventory absent. |
| No network except explicit provider permission | provider-contracts | **absent** as a public permission contract. |
| Disable XML entities, XInclude network, remote schemas | security-isolation-secrets | **conforming** — XML/JATS tests prove external entities, XInclude, and remote schemas remain inert and diagnosed with no resolver or network surface. |
| Prevent archive traversal, absolute paths, unsafe links/devices, collisions, bombs | container-safe-recursion | **absent**. |
| Isolate external/native backends with time/memory/filesystem/process limits | security-isolation-secrets | **absent**. |
| Private temporary directories and declared retention/deletion | security-isolation-secrets | **absent**. |
| Escape active HTML/script in renderers by default | render-normalized-source-maps | **absent** as a cross-renderer gate. |
| Keep passwords/provider secrets out of serialization/log/debug/hash/diagnostics | security-isolation-secrets | **absent** — no secret wrapper/provider request. |
| Classify and quarantine embedded executables/macros | embedded-artifact-contract | **absent**. |
| Do not transmit/persist input metadata without caller instruction | security-isolation-secrets | **partial** — no network code, but no explicit policy/API test. |
| Fuzz malformed inputs and never panic across public API | universal-security-fuzz-suite | **absent**. |

## §18 repository deliverables

| Required path/job | Baseline |
|---|---|
| `src/core/`, `src/detect/`, `src/ingest/`, `src/document_graph/`, `src/model_output/`, `src/schema/` | **partial** — equivalent flat files exist, pending target topology. |
| `src/registry/`, `src/container/`, `src/segment/`, `src/transform/`, `src/render/`, `src/provider/`, `src/formats/<family>/`, `src/cli/` | **absent** as required module trees. |
| `schemas/` every checked public schema | **partial** — current subset only. |
| `fixtures/` licensed/synthetic corpus and expected outputs | **absent**. |
| `tests/` golden/integration/security/compatibility | **partial** — e2e/graph tests only. |
| `fuzz/` byte-facing targets/corpora | **absent**. |
| `benches/` representative suites | **absent**. |
| `docs/` format/security/compatibility/examples | **partial** — current docs do not cover every format contract. |

## Completion policy

Only `complete-parser-release-readiness` may mark this contract complete, and only with evidence-backed passes for every normative row, every format, fixture/test class, promotion gate, security control, and repository deliverable. A missing backend, fixture license, provider, dependency, or test environment is evidence of a gap, not permission to weaken the target.
