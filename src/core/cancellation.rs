//! Cooperative cancellation and the operation control shared by all adapters.

use super::{
    BudgetExceeded, BudgetSelection, BudgetTracker, BudgetUsage, Diagnostic, OperationStatus,
    ResourceBudgetValidationError,
};
use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

/// Cloneable caller-controlled cancellation signal.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<(), CancellationError> {
        if self.is_cancelled() {
            Err(CancellationError)
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
#[error("operation cancelled by caller")]
pub struct CancellationError;

impl CancellationError {
    pub fn diagnostic(self, parser: impl Into<String>) -> Diagnostic {
        Diagnostic::info(parser, "grist.operation.cancelled", self.to_string())
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum OperationControlError {
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
    #[error(transparent)]
    BudgetExceeded(#[from] BudgetExceeded),
}

impl OperationControlError {
    pub fn operation_status(&self, emitted_items: u64) -> OperationStatus {
        match self {
            Self::Cancelled(_) => OperationStatus::Cancelled,
            Self::BudgetExceeded(error) => error.operation_status(emitted_items),
        }
    }
    pub fn diagnostic(&self, parser: impl Into<String>) -> Diagnostic {
        match self {
            Self::Cancelled(error) => error.diagnostic(parser),
            Self::BudgetExceeded(error) => error.diagnostic(parser),
        }
    }
}

/// One operation's cancellation token and shared budget tree.
#[derive(Debug, Clone)]
pub struct OperationControl {
    budget: BudgetTracker,
    cancellation: CancellationToken,
}

impl OperationControl {
    pub fn new(
        selection: &BudgetSelection,
        cancellation: CancellationToken,
    ) -> Result<Self, ResourceBudgetValidationError> {
        Ok(Self {
            budget: BudgetTracker::new(selection)?,
            cancellation,
        })
    }

    pub fn budget(&self) -> &BudgetTracker {
        &self.budget
    }
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
    pub fn usage(&self) -> BudgetUsage {
        self.budget.snapshot()
    }

    /// Adapters call this between bounded units of work and before emission.
    pub fn checkpoint(&self) -> Result<(), OperationControlError> {
        self.cancellation.check()?;
        self.budget.checkpoint_parse_time()?;
        Ok(())
    }

    /// Attribute wall time to the provider axis while retaining parse time too.
    pub fn run_provider<T>(
        &self,
        operation: impl FnOnce() -> T,
    ) -> Result<T, OperationControlError> {
        self.checkpoint()?;
        let started = Instant::now();
        let result = operation();
        self.budget.consume_provider_time(started.elapsed())?;
        self.checkpoint()?;
        Ok(result)
    }
}
