//! Canonical event-stream protocol and batch-as-stream collection.

use super::{
    BudgetUsage, ContentIdentity, Diagnostic, OperationControl, OperationControlError,
    OperationStatus, RequestId,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// One successfully emitted item. Identity is mandatory and survives later
/// cancellation or resource exhaustion.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamItem<T> {
    pub sequence: u64,
    pub request_id: RequestId,
    pub identity: ContentIdentity,
    pub payload: T,
}

impl<T> StreamItem<T> {
    pub fn new(
        sequence: u64,
        request_id: RequestId,
        identity: ContentIdentity,
        payload: T,
    ) -> Self {
        Self {
            sequence,
            request_id,
            identity,
            payload,
        }
    }
}

/// Exactly one terminal event closes every stream, including empty streams.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamTerminal {
    pub status: OperationStatus,
    pub emitted_items: u64,
    pub diagnostics: Vec<Diagnostic>,
    pub budget_usage: BudgetUsage,
}

impl StreamTerminal {
    pub fn complete(emitted_items: u64, budget_usage: BudgetUsage) -> Self {
        Self {
            status: OperationStatus::Complete,
            emitted_items,
            diagnostics: Vec::new(),
            budget_usage,
        }
    }

    pub fn controlled(
        parser: impl Into<String>,
        emitted_items: u64,
        budget_usage: BudgetUsage,
        error: OperationControlError,
    ) -> Self {
        let parser = parser.into();
        Self {
            status: error.operation_status(emitted_items),
            emitted_items,
            diagnostics: vec![error.diagnostic(parser)],
            budget_usage,
        }
    }

    /// Build a terminal directly from the shared control so the usage snapshot
    /// cannot accidentally come from a different budget tree.
    pub fn from_control(
        parser: impl Into<String>,
        emitted_items: u64,
        control: &OperationControl,
        error: OperationControlError,
    ) -> Self {
        Self::controlled(parser, emitted_items, control.usage(), error)
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StreamEvent<T> {
    Item { item: StreamItem<T> },
    Terminal { terminal: StreamTerminal },
}

impl<T> StreamEvent<T> {
    pub fn item(item: StreamItem<T>) -> Self {
        Self::Item { item }
    }
    pub fn terminal(terminal: StreamTerminal) -> Self {
        Self::Terminal { terminal }
    }
}

/// Marker used by large-format adapters returning lazy event iterators.
pub trait OperationStream<T>: Iterator<Item = StreamEvent<T>> {}
impl<T, I: Iterator<Item = StreamEvent<T>>> OperationStream<T> for I {}

/// Convenience batch result collected from the exact event-stream semantics.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchResult<T> {
    pub items: Vec<StreamItem<T>>,
    pub status: OperationStatus,
    pub diagnostics: Vec<Diagnostic>,
    pub budget_usage: BudgetUsage,
}

impl<T> BatchResult<T> {
    pub fn collect(
        stream: impl IntoIterator<Item = StreamEvent<T>>,
    ) -> Result<Self, StreamProtocolError> {
        collect_batch(stream)
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum StreamProtocolError {
    #[error("stream ended without a terminal event")]
    MissingTerminal,
    #[error("stream emitted an event after its terminal event")]
    EventAfterTerminal,
    #[error("stream item sequence {actual} did not match expected {expected}")]
    NonContiguousSequence { expected: u64, actual: u64 },
    #[error("terminal emitted_items {declared} did not match observed {observed}")]
    EmittedCountMismatch { declared: u64, observed: u64 },
    #[error("complete terminal carried a partial-result diagnostic")]
    CompleteWithPartialDiagnostic,
}

/// Collect an event stream without inventing end-of-input semantics. A stream
/// that simply stops is a protocol error, never a complete batch.
pub fn collect_batch<T>(
    stream: impl IntoIterator<Item = StreamEvent<T>>,
) -> Result<BatchResult<T>, StreamProtocolError> {
    let mut items = Vec::new();
    let mut terminal = None;
    for event in stream {
        if terminal.is_some() {
            return Err(StreamProtocolError::EventAfterTerminal);
        }
        match event {
            StreamEvent::Item { item } => {
                let expected = u64::try_from(items.len()).unwrap_or(u64::MAX);
                if item.sequence != expected {
                    return Err(StreamProtocolError::NonContiguousSequence {
                        expected,
                        actual: item.sequence,
                    });
                }
                items.push(item);
            }
            StreamEvent::Terminal { terminal: value } => terminal = Some(value),
        }
    }
    let terminal = terminal.ok_or(StreamProtocolError::MissingTerminal)?;
    let observed = u64::try_from(items.len()).unwrap_or(u64::MAX);
    if terminal.emitted_items != observed {
        return Err(StreamProtocolError::EmittedCountMismatch {
            declared: terminal.emitted_items,
            observed,
        });
    }
    if terminal.status == OperationStatus::Complete
        && terminal
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.partial)
    {
        return Err(StreamProtocolError::CompleteWithPartialDiagnostic);
    }
    Ok(BatchResult {
        items,
        status: terminal.status,
        diagnostics: terminal.diagnostics,
        budget_usage: terminal.budget_usage,
    })
}
