# Recursive container controller

`grist::container::ContainerRecursor` is the shared traversal boundary for
archives, packages, email bodies, attachments, and compound documents. Concrete
format modules implement `ContainerDecoder`; a decoder inventories only its
immediate members. The controller owns recursion, child ingestion, storage
policy, cancellation, and resource accounting, so a nested format cannot reset
or replace those policies.

The public result is `grist/container-traversal/v1`, exposed as the
`container-traversal` schema. It contains the root identity, traversal options,
all child records and diagnostics, and the single budget allocation tree.

## Parent-relative locations and identities

Each decoder returns a validated `ParentRelativeLocator`. The controller appends
its components to the immediate parent's full `SourceLocator`. A text attachment
inside an email member inside an archive therefore retains an ordered chain such
as:

```text
archive_member -> email_part -> text_range
```

Every `EmbeddedArtifact` retains the exact immediate parent content identity and
the full resolved locator. Its artifact ID is derived from those values and the
child's raw identity. Decoder order, filenames, terminal state, payload parsing,
and storage mode are not identity inputs.

Decoder members carry an explicit `source_order`; the controller sorts by source
order and stable locator/content tie-breakers before any budget is allocated.
Allocation IDs and output ordering are consequently repeatable even if a decoder
discovers members in another order.

## One budget and one allocation tree

The root creates one `OperationControl`. Every decoder checkpoint, descendant,
normal leaf parser, and output charge uses clones of that same shared tracker.
There is no child-local tracker that can reset a counter.

Before processing a member the result records a `BudgetAllocationNode` with:

- a deterministic allocation ID and parent allocation ID;
- depth and full locator;
- the remaining allowance on every budget axis;
- usage immediately before and after the complete child subtree;
- the exact axis hit, when processing was limited;
- nested allocation nodes in the same shape as recursive children.

Archive/package members charge `max_archive_members`; every discovered child
charges `max_child_artifacts`. Nesting depth is checked before storage or parse.
Expansion is the cumulative number of available descendant bytes divided by the
root container byte length, so splitting a bomb across many members or nested
containers cannot evade the ratio. A zero-byte root with non-empty expansion
uses the existing maximum-finite ratio rule. Input, memory, parse time, provider
time, and output charges continue through the common budget tracker.

When a limit is hit, the member remains in the inventory with exact identity
when bytes were already available, status `budget_limited`, a structured
`grist.budget.<axis>.exhausted` diagnostic, and the matching `limit_hit` value in
the budget tree. Descendants are not entered after their allocation is denied.

## Artifact modes and child outcomes

The caller explicitly selects one of three modes:

- `inventory_only` hashes available bytes but retains no bytes and does not parse
  ordinary leaf payloads. Registered nested containers are still inventoried
  recursively.
- `inline_payload` retains exact child bytes inline and sends ordinary leaves
  through the unified `Ingestor` with the shared control.
- `content_addressed` requires a caller-supplied sink, stores every available
  child by verified SHA-256 reference, and otherwise has the same parse behavior
  as inline mode.

Available-byte identities are identical in all three modes, including empty
children. Storage failure is explicit and never falls back to an unrequested
mode.

Each child reports one of `parsed`, `inventory_only`, `skipped`, `encrypted`,
`unsupported`, `rejected`, `budget_limited`, `failed`, or `cancelled`. Decoder
terminal states always require a non-empty machine code. Parsed leaves retain
their complete unified-ingestion envelope; unsupported, ambiguous, encrypted,
failed, and cancelled parse envelopes map to the corresponding child outcome.
Any lossy descendant makes the root traversal partial, while root decoder
failure, unsupported format, encryption, or cancellation remains a typed root
status.

Decoders must treat member names and headers as metadata, never paths or actions.
They call `checkpoint` between bounded units and `check_member_capacity` before
allocating declared expanded content. The controller catches decoder panics,
rechecks all shared limits before retaining or parsing a member, never executes
active content, never fetches network content, and never materializes artifacts.
