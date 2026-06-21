# Grist LDGR Projection Module Spec

Status: draft implementation request for `https://github.com/hydra-dynamix/grist`.

## Purpose

Build a Grist module that parses, validates, renders, and round-trips **LDGR Markdown Projection v1** documents for `ldgr-conduct`.

`ldgr-conduct` owns conduct-specific document schemas and semantics. Grist owns the document parsing, schema validation, projection rendering, schema evolution, and typed object model.

## Integration target

Recommended Grist module/feature:

```text
feature: ldgr-projection
module: grist::ldgr_projection
schema prefix: grist.ldgr_projection.v1
```

Recommended feature dependencies:

```toml
ldgr-projection = ["markdown", "serialization", "schemas"]
```

`ldgr-conduct` should be able to depend on Grist and call this module without implementing its own Markdown parser.

## Existing Grist primitives to reuse

The module should reuse Grist's existing Markdown/frontmatter parser where possible:

- `grist::markdown::parse_markdown`
- frontmatter extraction and YAML parsing;
- fenced-code block detection with source ranges;
- `Envelope<T>` / diagnostics / source info / hashes;
- schema generation/listing.

The LDGR module must inspect only:

- parsed YAML frontmatter;
- fenced code blocks whose info string begins with `ldgr-`.

It must not interpret arbitrary headings, prose, lists, tables, task lists, or other Markdown nodes as machine data.

## Public API

Recommended Rust API:

```rust
pub mod ldgr_projection {
    pub fn parse_ldgr_projection(
        text: &str,
        source: grist::core::SourceInfo,
        options: LdgrProjectionOptions,
    ) -> LdgrProjectionEnvelope;

    pub fn render_ldgr_projection(
        document: &LdgrProjectionDocument,
        options: LdgrProjectionRenderOptions,
    ) -> Result<String, LdgrProjectionRenderError>;

    pub fn validate_ldgr_projection(
        document: &LdgrProjectionDocument,
        options: LdgrProjectionValidationOptions,
    ) -> Vec<grist::core::Diagnostic>;
}
```

Recommended aliases:

```rust
pub type LdgrProjectionEnvelope = grist::core::Envelope<LdgrProjectionDocument>;
```

## Top-level typed object

```rust
pub struct LdgrProjectionDocument {
    pub schema_version: String, // "grist.ldgr_projection.v1"
    pub metadata: LdgrProjectionMetadata,
    pub machine_blocks: Vec<LdgrMachineBlock>,
    pub typed: LdgrDocument,
    pub markdown: Option<MarkdownProjectionTrace>,
}
```

`markdown` is optional trace/provenance data that may include the raw title, ignored contextual ranges, original frontmatter range, block ranges, and original Markdown hash. Conduct should not need it for scheduling decisions.

## Frontmatter contract

Every LDGR Markdown Projection v1 document must start with YAML frontmatter.

Required fields:

```yaml
ldgr_doc: 1
kind: <document_kind>
id: <unique_identifier>
schema: <schema_version>
```

Recommended optional fields:

```yaml
status:
created:
updated:
parent:
depends_on:
produces:
tags:
```

Recommended metadata type:

```rust
pub struct LdgrProjectionMetadata {
    pub ldgr_doc: u64,
    pub kind: LdgrDocumentKind,
    pub id: String,
    pub schema: String,
    pub status: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub parent: Option<LdgrRef>,
    pub depends_on: Vec<LdgrRef>,
    pub produces: Vec<LdgrRef>,
    pub tags: Vec<String>,
    pub extra: serde_json::Map<String, serde_json::Value>,
}
```

Parsing requirements:

- `ldgr_doc` must equal integer `1` for v1.
- `kind` must be a supported kind.
- `id` must be non-empty and stable.
- `schema` must be non-empty and should match the document kind.
- `depends_on`, `produces`, and `tags` accept absent, scalar, or list inputs only if Grist's existing normalization policy allows that; otherwise fail closed and require lists.
- Unknown frontmatter fields are preserved in `extra` but do not affect conduct semantics.

## Supported document kinds

Initial kinds:

```text
spec
epoch
ticket
work_item
artifact
decision
claim
validation
run_report
graph
ticket_index
batch_state
```

Recommended enum:

```rust
pub enum LdgrDocumentKind {
    Spec,
    Epoch,
    Ticket,
    WorkItem,
    Artifact,
    Decision,
    Claim,
    Validation,
    RunReport,
    Graph,
    TicketIndex,
    BatchState,
}
```

Unknown kinds should produce an error diagnostic. Do not silently parse unknown kinds as generic documents unless an explicit non-strict option is supplied.

## Machine block grammar

Machine-readable content lives in fenced code blocks whose info string begins with `ldgr-`.

Examples:

````markdown
```ldgr-contract yaml
requirements:
  - id: req.forward-status
    text: ldgr-conduct status exposes LDGR status behavior
```
````

````markdown
```ldgr-graph yaml
nodes:
  - id: ticket.a
edges:
  - dependency: ticket.a
    dependent: ticket.b
```
````

Info string grammar:

```text
ldgr-<block-kind> <format> [attributes...]
```

V1 required format support:

```text
yaml
```

Future allowed formats may include JSON, TOML, or other Grist-supported serializations, but v1 must support YAML.

Required parser behavior:

- Parse only code fences whose first info token starts with `ldgr-`.
- Preserve all `ldgr-*` machine blocks with source ranges.
- Parse block payload according to the declared format.
- Emit an error diagnostic for invalid YAML or unsupported formats.
- Emit an error diagnostic when a required block for a document kind is missing.
- Unknown `ldgr-*` block kinds should be preserved as generic machine blocks and produce a warning unless strict mode requires an error.

Recommended generic block type:

```rust
pub struct LdgrMachineBlock {
    pub kind: String,          // e.g. "contract", "graph", "validation"
    pub format: String,        // e.g. "yaml"
    pub attributes: Vec<String>,
    pub value: serde_json::Value,
    pub raw: String,
    pub range: Option<grist::core::SourceRange>,
}
```

## Reference syntax

Typed refs are strings with an explicit prefix:

```text
artifact:<id>
run:<id>
work:<slug>
prompt:<slug>
ticket:<id>
graph:<id>
batch:<id>
worker:<id>
path:<relative/path>
db:<relative/path>
```

Recommended type:

```rust
pub struct LdgrRef {
    pub kind: String,
    pub value: String,
}
```

Validation requirements:

- Syntactically reject empty kind or empty value.
- Preserve unknown ref kinds as diagnostics unless strict mode rejects them.
- Do not resolve references against an LDGR database inside Grist. Reference resolution belongs to `ldgr-conduct`.

## Typed document variants

Recommended top-level enum:

```rust
pub enum LdgrDocument {
    Spec(LdgrSpecDocument),
    Epoch(LdgrEpochDocument),
    Ticket(LdgrTicketDocument),
    WorkItem(LdgrWorkItemDocument),
    Artifact(LdgrArtifactDocument),
    Decision(LdgrDecisionDocument),
    Claim(LdgrClaimDocument),
    Validation(LdgrValidationDocument),
    RunReport(LdgrRunReportDocument),
    Graph(LdgrGraphDocument),
    TicketIndex(LdgrTicketIndexDocument),
    BatchState(LdgrBatchStateDocument),
}
```

Each variant should include the parsed metadata plus typed content from the relevant machine blocks. Avoid requiring conduct to inspect raw block maps for the common scheduling path.

## Ticket document schema

Frontmatter:

```yaml
ldgr_doc: 1
kind: ticket
id: ticket.scheduler.ready-selection
status: ready
schema: ldgr.ticket.v1
depends_on:
  - ticket.graph.build
produces:
  - work:scheduler.ready-selection
```

Required block:

````markdown
```ldgr-contract yaml
title: Ready Selection
description: Select runnable tickets whose dependencies are complete.
requirements:
  - id: req.dependencies-complete
    text: Completed dependencies are required before execution.
    evidence_required: true
constraints:
  - id: con.no-shared-worktree
    text: Do not schedule two writable workers into the same worktree.
tests:
  - id: test.scheduler-ready-selection
    command: cargo test scheduler_ready_selection
    required: true
validation_instructions:
  - Run listed tests or explain why they are not applicable.
expected_artifacts:
  - scheduler validation output
  - updated batch_state artifact
```
````

Recommended typed model:

```rust
pub struct LdgrTicketDocument {
    pub title: String,
    pub description: String,
    pub requirements: Vec<LdgrRequirement>,
    pub constraints: Vec<LdgrConstraint>,
    pub tests: Vec<LdgrTest>,
    pub validation_instructions: Vec<String>,
    pub expected_artifacts: Vec<String>,
}

pub struct LdgrRequirement {
    pub id: String,
    pub text: String,
    pub evidence_required: bool,
}

pub struct LdgrConstraint {
    pub id: String,
    pub text: String,
}

pub struct LdgrTest {
    pub id: String,
    pub command: Option<String>,
    pub scenario: Option<String>,
    pub required: bool,
}
```

Ticket validation:

- `title` and `description` are required.
- Requirement IDs must be unique within a ticket.
- Constraint IDs must be unique within a ticket.
- Test IDs must be unique within a ticket.
- At least one requirement is required.
- Empty `tests` is allowed only if the ticket explicitly explains validation without tests in `validation_instructions`.

## Ticket index schema

Frontmatter:

```yaml
ldgr_doc: 1
kind: ticket_index
id: ticket_index.project
schema: ldgr.ticket_index.v1
source: artifact:12
```

Required block:

````markdown
```ldgr-ticket-index yaml
tickets:
  - id: ticket.scheduler.ready-selection
    artifact: artifact:21
    title: Ready Selection
    work_item: work:scheduler.ready-selection
  - id: ticket.batch.state
    artifact: artifact:22
    title: Batch State
```
````

Recommended typed model:

```rust
pub struct LdgrTicketIndexDocument {
    pub tickets: Vec<LdgrTicketIndexEntry>,
}

pub struct LdgrTicketIndexEntry {
    pub id: String,
    pub artifact: LdgrRef,
    pub title: String,
    pub work_item: Option<LdgrRef>,
}
```

Validation:

- Ticket IDs must be unique.
- Artifact refs must use `artifact:<id>` syntax.
- If `work_item` is present, it must use `work:<slug>` syntax.

## Graph schema

Frontmatter:

```yaml
ldgr_doc: 1
kind: graph
id: graph.project
schema: ldgr.graph.v1
source: artifact:20
```

Required block:

````markdown
```ldgr-graph yaml
nodes:
  - id: ticket.graph.build
    artifact: artifact:21
    work_item: work:graph.build
  - id: ticket.scheduler.ready-selection
    artifact: artifact:22
    work_item: work:scheduler.ready-selection
edges:
  - dependency: ticket.graph.build
    dependent: ticket.scheduler.ready-selection
    kind: blocks
```
````

Dependency direction is explicit:

- `dependency` is the prerequisite.
- `dependent` waits for the prerequisite.

Recommended typed model:

```rust
pub struct LdgrGraphDocument {
    pub nodes: Vec<LdgrGraphNode>,
    pub edges: Vec<LdgrGraphEdge>,
}

pub struct LdgrGraphNode {
    pub id: String,
    pub artifact: Option<LdgrRef>,
    pub work_item: Option<LdgrRef>,
}

pub struct LdgrGraphEdge {
    pub dependency: String,
    pub dependent: String,
    pub kind: Option<String>,
}
```

Validation:

- Node IDs must be unique.
- Every edge endpoint must refer to an existing node ID.
- Self-edges are errors.
- Cycles are errors for the conduct scheduler path.

## Batch-state schema

Frontmatter:

```yaml
ldgr_doc: 1
kind: batch_state
id: batch.2026-06-21.001
schema: ldgr.batch_state.v1
```

Required block:

````markdown
```ldgr-batch-state yaml
batch_id: batch.2026-06-21.001
graph_artifact_id: artifact:31
ticket_index_artifact_id: artifact:30
status: running
current_wave: wave-002
waves:
  - wave_id: wave-001
    node_ids:
      - ticket.graph.build
    worker_ids:
      - worker-001
    status: complete
workers:
  - worker_id: worker-001
    ticket_id: ticket.graph.build
    work_item_id: work:graph.build
    worktree_path: path:.ldgr-conduct/worktrees/batch.2026-06-21.001/worker-001-graph-build
    worker_db_path: db:.ldgr-conduct/workers/batch.2026-06-21.001/worker-001/ldgr.db
    status: success
blocked:
  - ticket_id: ticket.scheduler.ready-selection
    reason: waiting for ticket.graph.build
```
````

Allowed batch statuses:

```text
running
blocked
complete
failed
canceled
```

Worker statuses should include at least:

```text
pending
running
success
failure
validation_failed
conflict
skipped
blocked
```

Validation:

- `batch_id` must match frontmatter `id` or produce an error.
- `graph_artifact_id` and `ticket_index_artifact_id` must be artifact refs.
- `current_wave`, when present, must refer to a wave in `waves`.
- Worker IDs must be unique.
- Worker paths must be typed refs: `path:<...>` and `db:<...>`.

## Validation document schema

Frontmatter:

```yaml
ldgr_doc: 1
kind: validation
id: validation.final.batch.001
schema: ldgr.validation.v1
```

Required block:

````markdown
```ldgr-validation yaml
validator: conduct.final-validator
status: accepted
targets:
  - graph:graph.project
  - batch:batch.2026-06-21.001
evidence:
  - artifact:44
findings:
  - id: finding.requirements-covered
    status: passed
    text: Original source requirements are represented by completed tickets.
```
````

Allowed validation statuses:

```text
passed
failed
accepted
rejected
blocked
waived
```

## Run report schema

Use `kind: run_report` for final validation reports, worker summaries, and batch summaries that do not fit the stricter batch-state schema.

Recommended block:

````markdown
```ldgr-run-report yaml
status: success
summary: Worker completed ticket and recorded validation evidence.
links:
  worker_db: artifact:55
  ticket: ticket:scheduler.ready-selection
  work_item: work:scheduler.ready-selection
outcomes:
  - id: req.dependencies-complete
    status: passed
    evidence:
      - artifact:56
```
````

## Rendering requirements

`render_ldgr_projection` should:

1. Render deterministic YAML frontmatter with stable key order.
2. Render a title.
3. Render typed `ldgr-*` machine blocks from typed objects.
4. Preserve or append contextual prose when supplied.
5. Never synthesize machine fields from arbitrary prose.
6. Produce output that parses back to an equivalent typed document.

Round-trip equivalence should compare typed metadata and typed machine content, not arbitrary prose formatting.

## Diagnostic requirements

Diagnostics should use Grist's existing diagnostic model and source ranges.

Required diagnostic cases:

- missing frontmatter;
- invalid frontmatter YAML;
- missing required frontmatter field;
- unsupported `ldgr_doc` version;
- unsupported `kind`;
- unsupported or mismatched `schema`;
- invalid machine block YAML;
- unsupported machine block format;
- required machine block missing;
- duplicate singleton machine block for a kind;
- duplicate IDs inside tickets/graphs/indexes;
- invalid reference syntax;
- graph edge endpoint missing;
- graph cycle detected;
- batch-state current wave missing;
- schema validation failure.

Severity guidance:

- Structural violations needed for typed object safety are errors.
- Unknown preserved fields/blocks are warnings in default mode and errors in strict mode.
- Ignored contextual Markdown is not a diagnostic.

## Schema outputs

Add generated schemas to Grist's schema registry and checked-in `schemas/` directory.

Recommended names:

```text
grist.ldgr-projection.v1.schema.json
grist.ldgr-projection-envelope.v1.schema.json
```

If per-kind schemas are generated, use:

```text
grist.ldgr-ticket.v1.schema.json
grist.ldgr-graph.v1.schema.json
grist.ldgr-ticket-index.v1.schema.json
grist.ldgr-batch-state.v1.schema.json
grist.ldgr-validation.v1.schema.json
grist.ldgr-run-report.v1.schema.json
```

`grist::schema::list_schemas()` and `schema_json()` should expose the new schemas when the feature is enabled.

## CLI expectations

If Grist exposes CLI support for parser modules, add a command equivalent to:

```sh
grist parse ldgr-projection <path> --json
```

The CLI output should be the envelope JSON for `LdgrProjectionDocument`.

## Test fixtures

Minimum Grist test coverage:

1. Parses a valid ticket projection into `LdgrDocument::Ticket`.
2. Parses a valid graph and detects dependency direction.
3. Rejects a graph with a cycle.
4. Parses a valid ticket index.
5. Parses a valid batch-state artifact.
6. Parses a final validation document.
7. Ignores arbitrary headings/lists/prose as machine data.
8. Preserves unknown frontmatter fields in `extra`.
9. Warns or errors on unknown `ldgr-*` blocks according to strictness.
10. Emits source-ranged diagnostics for invalid YAML in frontmatter and machine blocks.
11. Renders a typed ticket document and parses it back equivalently.
12. Exposes generated JSON schemas and keeps checked-in schemas in sync.

## Acceptance criteria for ldgr-conduct integration

`ldgr-conduct` can consider the Grist module ready when it can:

- parse a recorded ticket artifact into a typed ticket object;
- parse a ticket index and graph artifact into typed objects;
- validate graph edges and detect cycles;
- parse and render batch-state artifacts;
- parse final validation artifacts;
- receive source-ranged diagnostics suitable for recording as LDGR observations/artifacts;
- operate without inspecting raw Markdown headings/lists/prose for machine semantics.
