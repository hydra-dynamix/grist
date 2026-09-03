//! Explicit resource policies and shared, deterministic budget accounting.

use super::{
    Diagnostic, DiagnosticCode, DiagnosticDetails, OperationStatus, RecoveryAction, RecoveryKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// All limits visible at every public operation boundary. `None` is explicitly unbounded.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceBudget {
    pub max_input_bytes: Option<u64>,
    pub max_decoded_characters: Option<u64>,
    pub max_pages: Option<u64>,
    pub max_records: Option<u64>,
    pub max_cells: Option<u64>,
    pub max_nodes: Option<u64>,
    pub max_nesting_depth: Option<u64>,
    pub max_archive_expansion_ratio: Option<f64>,
    pub max_archive_members: Option<u64>,
    pub max_child_artifacts: Option<u64>,
    pub max_parse_millis: Option<u64>,
    pub max_provider_millis: Option<u64>,
    pub max_memory_bytes: Option<u64>,
    pub max_temporary_storage_bytes: Option<u64>,
    pub max_output_bytes: Option<u64>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("max_archive_expansion_ratio must be finite and non-negative")]
pub struct ResourceBudgetValidationError;

/// Named, versioned policies. The version is part of the serialized variant.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetProfile {
    UntrustedServiceV1,
    TrustedUnboundedV1,
}

impl BudgetProfile {
    pub const fn name(self) -> &'static str {
        match self {
            Self::UntrustedServiceV1 => "untrusted_service",
            Self::TrustedUnboundedV1 => "trusted_unbounded",
        }
    }
    pub const fn version(self) -> u32 {
        1
    }
    pub const fn is_trusted(self) -> bool {
        matches!(self, Self::TrustedUnboundedV1)
    }
    pub fn budget(self) -> ResourceBudget {
        match self {
            Self::UntrustedServiceV1 => ResourceBudget::untrusted_service_v1(),
            Self::TrustedUnboundedV1 => ResourceBudget::trusted_unbounded(),
        }
    }
    pub fn definition(self) -> BudgetProfileDefinition {
        BudgetProfileDefinition {
            name: self.name().into(),
            version: self.version(),
            trusted: self.is_trusted(),
            budget: self.budget(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetProfileDefinition {
    pub name: String,
    pub version: u32,
    pub trusted: bool,
    pub budget: ResourceBudget,
}

/// Explicit selection with no implicit default.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "selection", content = "value", rename_all = "snake_case")]
pub enum BudgetSelection {
    Profile(BudgetProfile),
    Custom(Box<ResourceBudget>),
}

impl BudgetSelection {
    pub fn custom(budget: ResourceBudget) -> Self {
        Self::Custom(Box::new(budget))
    }
    pub fn budget(&self) -> ResourceBudget {
        match self {
            Self::Profile(profile) => profile.budget(),
            Self::Custom(budget) => budget.as_ref().clone(),
        }
    }
    pub fn validate(&self) -> Result<(), ResourceBudgetValidationError> {
        self.budget().validate()
    }
    pub fn profile_definition(&self) -> Option<BudgetProfileDefinition> {
        match self {
            Self::Profile(profile) => Some(profile.definition()),
            Self::Custom(_) => None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAxis {
    InputBytes,
    DecodedCharacters,
    Pages,
    Records,
    Cells,
    Nodes,
    NestingDepth,
    ArchiveExpansionRatio,
    ArchiveMembers,
    ChildArtifacts,
    ParseMillis,
    ProviderMillis,
    MemoryBytes,
    TemporaryStorageBytes,
    OutputBytes,
}

impl BudgetAxis {
    pub const ALL: [Self; 15] = [
        Self::InputBytes,
        Self::DecodedCharacters,
        Self::Pages,
        Self::Records,
        Self::Cells,
        Self::Nodes,
        Self::NestingDepth,
        Self::ArchiveExpansionRatio,
        Self::ArchiveMembers,
        Self::ChildArtifacts,
        Self::ParseMillis,
        Self::ProviderMillis,
        Self::MemoryBytes,
        Self::TemporaryStorageBytes,
        Self::OutputBytes,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::InputBytes => "input_bytes",
            Self::DecodedCharacters => "decoded_characters",
            Self::Pages => "pages",
            Self::Records => "records",
            Self::Cells => "cells",
            Self::Nodes => "nodes",
            Self::NestingDepth => "nesting_depth",
            Self::ArchiveExpansionRatio => "archive_expansion_ratio",
            Self::ArchiveMembers => "archive_members",
            Self::ChildArtifacts => "child_artifacts",
            Self::ParseMillis => "parse_millis",
            Self::ProviderMillis => "provider_millis",
            Self::MemoryBytes => "memory_bytes",
            Self::TemporaryStorageBytes => "temporary_storage_bytes",
            Self::OutputBytes => "output_bytes",
        }
    }
}

impl fmt::Display for BudgetAxis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum BudgetAmount {
    Count(u64),
    Ratio(f64),
}

impl fmt::Display for BudgetAmount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count(value) => value.fmt(formatter),
            Self::Ratio(value) => value.fmt(formatter),
        }
    }
}

/// Auditable shared consumption at a checkpoint or terminal event.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BudgetUsage {
    pub input_bytes: u64,
    pub decoded_characters: u64,
    pub pages: u64,
    pub records: u64,
    pub cells: u64,
    pub nodes: u64,
    pub nesting_depth: u64,
    pub archive_expansion_ratio: f64,
    pub archive_members: u64,
    pub child_artifacts: u64,
    pub parse_millis: u64,
    pub provider_millis: u64,
    pub memory_bytes: u64,
    pub temporary_storage_bytes: u64,
    pub output_bytes: u64,
}

impl BudgetUsage {
    pub fn amount(&self, axis: BudgetAxis) -> BudgetAmount {
        match axis {
            BudgetAxis::InputBytes => BudgetAmount::Count(self.input_bytes),
            BudgetAxis::DecodedCharacters => BudgetAmount::Count(self.decoded_characters),
            BudgetAxis::Pages => BudgetAmount::Count(self.pages),
            BudgetAxis::Records => BudgetAmount::Count(self.records),
            BudgetAxis::Cells => BudgetAmount::Count(self.cells),
            BudgetAxis::Nodes => BudgetAmount::Count(self.nodes),
            BudgetAxis::NestingDepth => BudgetAmount::Count(self.nesting_depth),
            BudgetAxis::ArchiveExpansionRatio => BudgetAmount::Ratio(self.archive_expansion_ratio),
            BudgetAxis::ArchiveMembers => BudgetAmount::Count(self.archive_members),
            BudgetAxis::ChildArtifacts => BudgetAmount::Count(self.child_artifacts),
            BudgetAxis::ParseMillis => BudgetAmount::Count(self.parse_millis),
            BudgetAxis::ProviderMillis => BudgetAmount::Count(self.provider_millis),
            BudgetAxis::MemoryBytes => BudgetAmount::Count(self.memory_bytes),
            BudgetAxis::TemporaryStorageBytes => BudgetAmount::Count(self.temporary_storage_bytes),
            BudgetAxis::OutputBytes => BudgetAmount::Count(self.output_bytes),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, thiserror::Error)]
#[error("resource budget {axis} exceeded: limit {limit}, observed {observed}")]
pub struct BudgetExceeded {
    pub axis: BudgetAxis,
    pub limit: BudgetAmount,
    pub observed: BudgetAmount,
    pub usage: Box<BudgetUsage>,
}

impl BudgetExceeded {
    pub fn operation_status(&self, emitted_items: u64) -> OperationStatus {
        if emitted_items == 0 {
            OperationStatus::Failed
        } else {
            OperationStatus::Partial
        }
    }

    pub fn diagnostic(&self, parser: impl Into<String>) -> Diagnostic {
        let mut diagnostic = Diagnostic::budget_exhausted(
            parser,
            format!(
                "{} budget exhausted: limit {}, observed {}",
                self.axis, self.limit, self.observed
            ),
        );
        diagnostic.code =
            DiagnosticCode::new(format!("grist.budget.{}.exhausted", self.axis.name()));
        diagnostic.recovery = Some(RecoveryAction::new(
            RecoveryKind::IncreaseBudget,
            format!(
                "increase the {} limit or select another budget profile",
                self.axis
            ),
            true,
        ));
        let value = |amount| match amount {
            BudgetAmount::Count(v) => Value::from(v),
            BudgetAmount::Ratio(v) => Value::from(v),
        };
        let values = BTreeMap::from([
            ("axis".into(), Value::String(self.axis.name().into())),
            ("limit".into(), value(self.limit)),
            ("observed".into(), value(self.observed)),
        ]);
        if let Ok(details) = DiagnosticDetails::new(values) {
            diagnostic.details = Some(details);
        }
        diagnostic
    }
}

struct BudgetTrackerInner {
    budget: ResourceBudget,
    usage: Mutex<BudgetUsage>,
    started: Instant,
}

/// Cloneable shared meter. Clones and child operations charge one budget tree.
#[derive(Clone)]
pub struct BudgetTracker(Arc<BudgetTrackerInner>);

impl fmt::Debug for BudgetTracker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BudgetTracker")
            .field("budget", &self.0.budget)
            .field("usage", &self.snapshot())
            .finish_non_exhaustive()
    }
}

impl BudgetTracker {
    pub fn new(selection: &BudgetSelection) -> Result<Self, ResourceBudgetValidationError> {
        let budget = selection.budget();
        budget.validate()?;
        Ok(Self(Arc::new(BudgetTrackerInner {
            budget,
            usage: Mutex::new(BudgetUsage::default()),
            started: Instant::now(),
        })))
    }
    pub fn budget(&self) -> &ResourceBudget {
        &self.0.budget
    }
    pub fn snapshot(&self) -> BudgetUsage {
        self.lock_usage().clone()
    }

    pub fn consume_input_bytes(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::InputBytes, n)
    }
    pub fn consume_decoded_characters(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::DecodedCharacters, n)
    }
    pub fn consume_pages(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::Pages, n)
    }
    pub fn consume_records(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::Records, n)
    }
    pub fn consume_cells(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::Cells, n)
    }
    pub fn consume_nodes(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::Nodes, n)
    }
    pub fn observe_nesting_depth(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.observe(BudgetAxis::NestingDepth, n)
    }
    pub fn consume_archive_members(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::ArchiveMembers, n)
    }
    pub fn consume_child_artifacts(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::ChildArtifacts, n)
    }
    pub fn observe_memory_bytes(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.observe(BudgetAxis::MemoryBytes, n)
    }
    pub fn observe_temporary_storage_bytes(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.observe(BudgetAxis::TemporaryStorageBytes, n)
    }
    pub fn consume_output_bytes(&self, n: u64) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::OutputBytes, n)
    }

    pub fn observe_parse_time(&self, elapsed: Duration) -> Result<(), BudgetExceeded> {
        self.observe(BudgetAxis::ParseMillis, duration_millis_ceil(elapsed))
    }
    pub fn checkpoint_parse_time(&self) -> Result<(), BudgetExceeded> {
        self.observe_parse_time(self.0.started.elapsed())
    }
    pub fn consume_provider_time(&self, elapsed: Duration) -> Result<(), BudgetExceeded> {
        self.consume(BudgetAxis::ProviderMillis, duration_millis_ceil(elapsed))
    }

    pub fn observe_archive_expansion(
        &self,
        compressed: u64,
        expanded: u64,
    ) -> Result<(), BudgetExceeded> {
        let ratio = match (compressed, expanded) {
            (0, 0) => 0.0,
            // Keep public snapshots JSON-serializable while representing an
            // effectively infinite expansion against a zero-byte source.
            (0, _) => f64::MAX,
            _ => expanded as f64 / compressed as f64,
        };
        let mut usage = self.lock_usage();
        usage.archive_expansion_ratio = usage.archive_expansion_ratio.max(ratio);
        let observed = usage.archive_expansion_ratio;
        if let Some(limit) = self.0.budget.max_archive_expansion_ratio
            && observed > limit
        {
            return Err(BudgetExceeded {
                axis: BudgetAxis::ArchiveExpansionRatio,
                limit: BudgetAmount::Ratio(limit),
                observed: BudgetAmount::Ratio(observed),
                usage: Box::new(usage.clone()),
            });
        }
        Ok(())
    }

    fn consume(&self, axis: BudgetAxis, amount: u64) -> Result<(), BudgetExceeded> {
        let mut usage = self.lock_usage();
        let observed = count_mut(&mut usage, axis).saturating_add(amount);
        *count_mut(&mut usage, axis) = observed;
        self.enforce(axis, observed, &usage)
    }

    fn observe(&self, axis: BudgetAxis, observed: u64) -> Result<(), BudgetExceeded> {
        let mut usage = self.lock_usage();
        let slot = count_mut(&mut usage, axis);
        *slot = (*slot).max(observed);
        self.enforce(axis, *slot, &usage)
    }

    fn enforce(
        &self,
        axis: BudgetAxis,
        observed: u64,
        usage: &BudgetUsage,
    ) -> Result<(), BudgetExceeded> {
        if let Some(limit) = count_limit(&self.0.budget, axis)
            && observed > limit
        {
            return Err(BudgetExceeded {
                axis,
                limit: BudgetAmount::Count(limit),
                observed: BudgetAmount::Count(observed),
                usage: Box::new(usage.clone()),
            });
        }
        Ok(())
    }

    fn lock_usage(&self) -> std::sync::MutexGuard<'_, BudgetUsage> {
        self.0
            .usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn duration_millis_ceil(duration: Duration) -> u64 {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        0
    } else {
        u64::try_from(nanos.div_ceil(1_000_000)).unwrap_or(u64::MAX)
    }
}

fn count_mut(usage: &mut BudgetUsage, axis: BudgetAxis) -> &mut u64 {
    match axis {
        BudgetAxis::InputBytes => &mut usage.input_bytes,
        BudgetAxis::DecodedCharacters => &mut usage.decoded_characters,
        BudgetAxis::Pages => &mut usage.pages,
        BudgetAxis::Records => &mut usage.records,
        BudgetAxis::Cells => &mut usage.cells,
        BudgetAxis::Nodes => &mut usage.nodes,
        BudgetAxis::NestingDepth => &mut usage.nesting_depth,
        BudgetAxis::ArchiveMembers => &mut usage.archive_members,
        BudgetAxis::ChildArtifacts => &mut usage.child_artifacts,
        BudgetAxis::ParseMillis => &mut usage.parse_millis,
        BudgetAxis::ProviderMillis => &mut usage.provider_millis,
        BudgetAxis::MemoryBytes => &mut usage.memory_bytes,
        BudgetAxis::TemporaryStorageBytes => &mut usage.temporary_storage_bytes,
        BudgetAxis::OutputBytes => &mut usage.output_bytes,
        BudgetAxis::ArchiveExpansionRatio => unreachable!("ratio uses ratio accounting"),
    }
}

fn count_limit(budget: &ResourceBudget, axis: BudgetAxis) -> Option<u64> {
    match axis {
        BudgetAxis::InputBytes => budget.max_input_bytes,
        BudgetAxis::DecodedCharacters => budget.max_decoded_characters,
        BudgetAxis::Pages => budget.max_pages,
        BudgetAxis::Records => budget.max_records,
        BudgetAxis::Cells => budget.max_cells,
        BudgetAxis::Nodes => budget.max_nodes,
        BudgetAxis::NestingDepth => budget.max_nesting_depth,
        BudgetAxis::ArchiveMembers => budget.max_archive_members,
        BudgetAxis::ChildArtifacts => budget.max_child_artifacts,
        BudgetAxis::ParseMillis => budget.max_parse_millis,
        BudgetAxis::ProviderMillis => budget.max_provider_millis,
        BudgetAxis::MemoryBytes => budget.max_memory_bytes,
        BudgetAxis::TemporaryStorageBytes => budget.max_temporary_storage_bytes,
        BudgetAxis::OutputBytes => budget.max_output_bytes,
        BudgetAxis::ArchiveExpansionRatio => None,
    }
}

impl ResourceBudget {
    pub fn validate(&self) -> Result<(), ResourceBudgetValidationError> {
        match self.max_archive_expansion_ratio {
            Some(ratio) if !ratio.is_finite() || ratio < 0.0 => Err(ResourceBudgetValidationError),
            _ => Ok(()),
        }
    }
}

impl ResourceBudget {
    pub fn trusted_unbounded() -> Self {
        Self {
            max_input_bytes: None,
            max_decoded_characters: None,
            max_pages: None,
            max_records: None,
            max_cells: None,
            max_nodes: None,
            max_nesting_depth: None,
            max_archive_expansion_ratio: None,
            max_archive_members: None,
            max_child_artifacts: None,
            max_parse_millis: None,
            max_provider_millis: None,
            max_memory_bytes: None,
            max_temporary_storage_bytes: None,
            max_output_bytes: None,
        }
    }

    pub fn untrusted_service_v1() -> Self {
        Self {
            max_input_bytes: Some(67_108_864),
            max_decoded_characters: Some(67_108_864),
            max_pages: Some(10_000),
            max_records: Some(1_000_000),
            max_cells: Some(5_000_000),
            max_nodes: Some(2_000_000),
            max_nesting_depth: Some(256),
            max_archive_expansion_ratio: Some(100.0),
            max_archive_members: Some(100_000),
            max_child_artifacts: Some(10_000),
            max_parse_millis: Some(120_000),
            max_provider_millis: Some(120_000),
            max_memory_bytes: Some(536_870_912),
            max_temporary_storage_bytes: Some(1_073_741_824),
            max_output_bytes: Some(268_435_456),
        }
    }
}
