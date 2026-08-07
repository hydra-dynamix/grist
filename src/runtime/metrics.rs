//! Structured metrics whose type system excludes source and metadata strings.

use crate::core::{BudgetUsage, OperationKind, OperationStatus, SchemaVersion};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// A fixed operation phase. Arbitrary labels are deliberately unsupported.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricPhase {
    Detection,
    Parser,
    Provider,
    Cache,
    Parallelism,
    Complete,
}

/// Numeric measurements safe for the default metrics boundary.
///
/// This record cannot carry source text, names, paths, addresses, prompts, or
/// document metadata. Backends may attach deployment-level labels outside
/// Grist after receiving it.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricValues {
    pub input_bytes: u64,
    pub pages: u64,
    pub records: u64,
    pub cells: u64,
    pub nodes: u64,
    pub children: u64,
    pub parser_elapsed_micros: u64,
    pub provider_elapsed_micros: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub repairs: u64,
    pub warnings: u64,
    pub completed_units: u64,
    pub peak_in_flight: u64,
}

/// One backend-neutral metrics event.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricEvent {
    pub schema_version: String,
    pub operation: OperationKind,
    pub phase: MetricPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<OperationStatus>,
    pub values: MetricValues,
    pub budget_usage: BudgetUsage,
}

impl MetricEvent {
    pub fn new(operation: OperationKind, phase: MetricPhase, values: MetricValues) -> Self {
        Self {
            schema_version: SchemaVersion::METRIC_EVENT_V1.into(),
            operation,
            phase,
            status: None,
            values,
            budget_usage: BudgetUsage::default(),
        }
    }

    pub fn with_status(mut self, status: OperationStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_budget_usage(mut self, usage: BudgetUsage) -> Self {
        self.budget_usage = usage;
        self
    }
}

/// Caller-supplied telemetry adapter. Grist has no telemetry dependency.
pub trait MetricsSink: Send + Sync + 'static {
    fn record(&self, event: &MetricEvent);
}

impl<T: MetricsSink + ?Sized> MetricsSink for Arc<T> {
    fn record(&self, event: &MetricEvent) {
        (**self).record(event);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopMetricsSink;

impl MetricsSink for NoopMetricsSink {
    fn record(&self, _event: &MetricEvent) {}
}

/// Cloneable hook used by operations and runtime helpers.
///
/// A telemetry adapter panic is isolated because observation must not change a
/// parse result or cross the public API boundary.
#[derive(Clone)]
pub struct MetricsHook(Arc<dyn MetricsSink>);

impl MetricsHook {
    pub fn new(sink: impl MetricsSink) -> Self {
        Self(Arc::new(sink))
    }

    pub fn emit(&self, event: &MetricEvent) {
        let _ = catch_unwind(AssertUnwindSafe(|| self.0.record(event)));
    }
}

impl Default for MetricsHook {
    fn default() -> Self {
        Self::new(NoopMetricsSink)
    }
}

impl fmt::Debug for MetricsHook {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MetricsHook(<backend-neutral sink>)")
    }
}
