//! Backend-neutral observability, caller-owned caching, and deterministic parallel execution.

mod cache;
mod metrics;
mod parallel;

pub use cache::{
    CacheEntry, CacheExecutionError, CacheKey, CacheKeyError, CacheReuseOutcome, CacheReuseRecord,
    CacheValidationError, CacheValue, ContentAddressedCache, load_or_compute,
    load_or_compute_with_metrics,
};
pub use metrics::{
    MetricEvent, MetricPhase, MetricValues, MetricsHook, MetricsSink, NoopMetricsSink,
};
pub use parallel::{
    CanonicalOrderKey, ParallelError, ParallelJob, ParallelOutput, ParallelUnitKind,
    ParallelismOptions, deterministic_parallel_collect,
    deterministic_parallel_collect_with_metrics, deterministic_parallel_for_each_ordered,
    deterministic_parallel_for_each_ordered_with_metrics,
};
