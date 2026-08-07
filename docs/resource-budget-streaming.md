# Resource budgets, streaming, and cancellation

Every public operation selects either `BudgetSelection::Profile` or an explicit
custom `ResourceBudget`; there is deliberately no default selection. The
built-in profiles are `untrusted_service_v1` (finite) and
`trusted_unbounded_v1` (explicitly trusted and unbounded). `BudgetProfile`
exposes the stable profile name, version, trust classification, and expanded
limits for capabilities manifests. Custom archive expansion ratios must be
finite and non-negative.

## Accounting semantics

`BudgetTracker` is the shared budget tree. Its clones charge the same counters,
so recursive members and child artifacts cannot reset a parent limit. Limits
are inclusive: an observation equal to the limit succeeds; the first larger
observation returns `BudgetExceeded`. The failed attempted usage remains in the
snapshot and diagnostic.

The following axes are cumulative: input bytes, decoded Unicode scalar values,
pages, records, cells, nodes, archive members, child artifacts, provider time,
and output bytes. Nesting depth, estimated live memory, and live temporary
storage record their maximum observation. Archive expansion records the maximum
`expanded_bytes / compressed_bytes` ratio; non-empty expansion from zero input
is represented by the maximum finite ratio and therefore exceeds every practical
finite ratio. Parse time records elapsed
wall time from tracker construction. Durations round up to milliseconds so a
positive sub-millisecond operation cannot evade a zero-millisecond policy.

Adapters call the axis-specific `consume_*` or `observe_*` method at bounded
work units. They call `OperationControl::checkpoint` between those units and
before emitting an item. Provider adapters use `run_provider`, or equivalently
charge measured provider time and checkpoint before and after the invocation.

A budget hit before any output has status `failed`; after one or more emitted
items it has status `partial`. Its stable diagnostic code is
`grist.budget.<axis>.exhausted`. Cancellation always has status `cancelled` and
code `grist.operation.cancelled`.

## Stream protocol and batch collection

Large adapters expose lazy iterators of `StreamEvent<T>` (the
`OperationStream<T>` marker). Every emitted `StreamItem<T>` carries a contiguous
zero-based sequence, the caller's request ID, and a mandatory content identity.
Exactly one `StreamTerminal` closes the stream, including empty, failed,
budget-limited, and cancelled streams. The terminal carries status, diagnostics,
emitted item count, and final budget usage.

`collect_batch` and `BatchResult::collect` are convenience collectors over that
same protocol. They do not infer success from iterator exhaustion. A missing
terminal, an event after terminal, a sequence gap, or an emitted-count mismatch
is a `StreamProtocolError`. Already emitted items and their identities remain in
a cancelled or partial batch.

```rust
use grist::core::{
    BatchResult, BudgetProfile, BudgetSelection, CancellationToken,
    OperationControl, StreamEvent, StreamTerminal,
};

let cancellation = CancellationToken::new();
let control = OperationControl::new(
    &BudgetSelection::Profile(BudgetProfile::UntrustedServiceV1),
    cancellation,
)?;

// An adapter checkpoints and charges axes while producing item events.
control.checkpoint()?;
let terminal: StreamEvent<serde_json::Value> = StreamEvent::terminal(
    StreamTerminal::complete(0, control.usage()),
);
let batch = BatchResult::collect([terminal])?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The checked schemas are `grist.resource-budget.v1`,
`grist.stream-event.v1`, and `grist.batch-result.v1`.
