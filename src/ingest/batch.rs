//! Stable request-correlated ingestion streams and their batch collector.

use super::{IngestError, Ingestor};
use crate::core::{
    BatchResult, BudgetSelection, CancellationToken, ContentIdentity, DiagnosticClass, Envelope,
    OperationControl, OperationStatus, ParseRequest, StreamEvent, StreamItem, StreamTerminal,
};
use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};

pub type IngestStreamEvent = StreamEvent<Envelope<Value>>;
pub type IngestBatchResult = BatchResult<Envelope<Value>>;

/// A lazy, input-ordered stream sharing one budget tree and cancellation token.
pub struct IngestStream<'a> {
    ingestor: &'a Ingestor,
    requests: VecDeque<ParseRequest>,
    control: OperationControl,
    sequence: u64,
    pending_terminal: Option<StreamTerminal>,
    finished: bool,
}

impl Ingestor {
    /// Stream requests in caller order under one explicit budget and token.
    /// The batch policy is applied to every item so all work charges one tree.
    pub fn stream<I>(
        &self,
        requests: I,
        budget: BudgetSelection,
        cancellation: CancellationToken,
    ) -> Result<IngestStream<'_>, IngestError>
    where
        I: IntoIterator<Item = ParseRequest>,
    {
        let control = OperationControl::new(&budget, cancellation.clone())?;
        let mut seen = BTreeSet::new();
        let mut ordered = VecDeque::new();
        for mut request in requests {
            if !seen.insert(request.request_id.clone()) {
                return Err(IngestError::DuplicateRequestId(
                    request.request_id.to_string(),
                ));
            }
            request.budget = budget.clone();
            request.cancellation = cancellation.clone();
            ordered.push_back(request);
        }
        Ok(IngestStream {
            ingestor: self,
            requests: ordered,
            control,
            sequence: 0,
            pending_terminal: None,
            finished: false,
        })
    }

    /// Collect the exact stream protocol without a separate batch code path.
    pub fn batch<I>(
        &self,
        requests: I,
        budget: BudgetSelection,
        cancellation: CancellationToken,
    ) -> Result<IngestBatchResult, IngestError>
    where
        I: IntoIterator<Item = ParseRequest>,
    {
        Ok(IngestBatchResult::collect(self.stream(
            requests,
            budget,
            cancellation,
        )?)?)
    }
}

impl Iterator for IngestStream<'_> {
    type Item = IngestStreamEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if let Some(terminal) = self.pending_terminal.take() {
            self.finished = true;
            return Some(StreamEvent::terminal(terminal));
        }
        let Some(request) = self.requests.pop_front() else {
            self.finished = true;
            return Some(StreamEvent::terminal(StreamTerminal::complete(
                self.sequence,
                self.control.usage(),
            )));
        };
        let request_id = request.request_id.clone();
        let envelope = match self
            .ingestor
            .ingest_with_control(request, self.control.clone())
        {
            Ok(envelope) => envelope,
            Err(error) => {
                self.finished = true;
                return Some(StreamEvent::terminal(StreamTerminal {
                    status: OperationStatus::Failed,
                    emitted_items: self.sequence,
                    diagnostics: vec![crate::core::Diagnostic::parser_defect(
                        "grist.ingest",
                        error.to_string(),
                    )],
                    budget_usage: self.control.usage(),
                }));
            }
        };
        let identity = envelope
            .identity
            .clone()
            .unwrap_or_else(ContentIdentity::default);
        let terminal = terminal_after(&envelope, self.sequence + 1, &self.control);
        let item = StreamItem::new(self.sequence, request_id, identity, envelope);
        self.sequence += 1;
        self.pending_terminal = terminal;
        Some(StreamEvent::item(item))
    }
}

fn terminal_after(
    envelope: &Envelope<Value>,
    emitted_items: u64,
    control: &OperationControl,
) -> Option<StreamTerminal> {
    let cancelled = envelope.status == OperationStatus::Cancelled;
    let budget_limited = envelope.diagnostics.iter().any(|diagnostic| {
        diagnostic.class == DiagnosticClass::ResourceBudgetExhaustion
            || diagnostic.code.as_str().starts_with("grist.budget.")
    });
    if !cancelled && !budget_limited {
        return None;
    }
    Some(StreamTerminal {
        status: if cancelled {
            OperationStatus::Cancelled
        } else if emitted_items == 0 {
            OperationStatus::Failed
        } else {
            OperationStatus::Partial
        },
        emitted_items,
        diagnostics: envelope.diagnostics.clone(),
        budget_usage: control.usage(),
    })
}
