# Runtime observability, cache, and parallelism

`grist::runtime` supplies runtime policy without choosing a telemetry backend,
persistent cache, scheduler, or thread-pool dependency. These contracts reuse
Grist canonical JSON, content identity, operation status, budget usage, and
provenance.

## Content-safe metrics

`MetricsSink` receives `MetricEvent` values through a cloneable `MetricsHook`.
The event model contains fixed enums, numeric counters, status, and numeric
budget usage only. It has no arbitrary label, source, filename, URI, address,
prompt, metadata, or extracted-content field. Deployment-level labels belong
to the caller's telemetry adapter after this boundary. A sink panic is isolated
and cannot change the operation result.

`Ingestor::with_metrics` exposes the hook on unified single, stream, and batch
ingestion. Cache and parallel helpers have explicit `*_with_metrics` variants.
Default helpers use a no-op sink and never initialize a telemetry backend.

## Caller-owned content-addressed cache

`CacheKey` v1 hashes this complete material with canonical JSON v1:

- operation and the exact input content hash;
- canonical parser metadata hash;
- payload schema version and options digest;
- sorted, unique provider-configuration digests;
- cache namespace and canonicalization version.

`ContentAddressedCache` owns only `get` and `put`; eviction, persistence,
sharing, encryption, and retention remain caller policy. A returned
`CacheEntry` is reusable only after its key digest, output SHA-256, canonical
JSON byte encoding, and requested Rust type round trip all validate. Backend
errors and invalid entries are returned and never become silent recomputation.

`load_or_compute` returns a `CacheReuseRecord`. Validated hits can append its
lossless `grist.cache.reuse/v1` provenance step to an operation envelope. The
cached value's canonical bytes are unchanged by whether it was computed or
reused.

## Deterministic bounded parallelism

`ParallelUnitKind` names pages, sheets, slides, archive members, repository
files, and generic independent units. Every job carries a numeric
`CanonicalOrderKey`; it cannot place a filename or source string in metrics.

`deterministic_parallel_collect` accepts arbitrary job order, sorts before
delivery, rejects duplicate keys, runs bounded windows, and returns canonical
source order. If multiple tasks fail, the first canonical-order error is
returned independent of completion timing.

`deterministic_parallel_for_each_ordered` is the lazy path. Its input must
already be in canonical order. At most `max_in_flight` inputs and results are
retained before ordered delivery to the caller, so page/record/artifact streams
do not require whole-document collection. `workers = 1` uses the same merge
semantics as parallel execution, making byte-stable serial/parallel comparison
straightforward.
