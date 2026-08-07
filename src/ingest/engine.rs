//! Single-request ingestion through detection, registry dispatch, and envelopes.

use crate::core::{
    ArtifactKind, AutoFormatOptions, BudgetAmount, BudgetAxis, BudgetExceeded, ContentIdentity,
    Diagnostic, DiagnosticClass, Envelope, EnvelopeInvariantError, InputError, Limits,
    OperationControl, OperationKind, OperationStatus, ParseRequest, ParserInfo,
    ResolvedParseRequest, ResourceBudgetValidationError, SchemaVersion, SourceInfo, options_digest,
};
use crate::detect::{
    Detection, DetectionOptions, DetectionOptionsError, DetectionStatus, detect_source,
};
use crate::registry::{
    ParserDispatchError, ParserRegistry, ParserRegistryError, ParserSelection,
    builtin_parser_registry,
};
use crate::runtime::{MetricEvent, MetricPhase, MetricValues, MetricsHook, MetricsSink};
use serde_json::{Value, json};
use std::time::Instant;

/// A configuration or invariant error outside an individual input result.
/// Input failures are returned as machine-readable envelopes.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error(transparent)]
    Registry(#[from] ParserRegistryError),
    #[error(transparent)]
    Dispatch(#[from] ParserDispatchError),
    #[error(transparent)]
    Envelope(#[from] EnvelopeInvariantError),
    #[error(transparent)]
    DetectionOptions(#[from] DetectionOptionsError),
    #[error(transparent)]
    InvalidBudget(#[from] ResourceBudgetValidationError),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    #[error("batch request ID is duplicated: {0}")]
    DuplicateRequestId(String),
    #[error(transparent)]
    StreamProtocol(#[from] crate::core::StreamProtocolError),
}

/// Unified ingestion boundary for every [`crate::core::Input`] variant.
pub struct Ingestor {
    registry: ParserRegistry,
    detection_options: DetectionOptions,
    metrics: MetricsHook,
}

impl Ingestor {
    pub fn builtin() -> Result<Self, IngestError> {
        Ok(Self::new(builtin_parser_registry()?))
    }

    pub fn new(registry: ParserRegistry) -> Self {
        Self {
            registry,
            detection_options: DetectionOptions::default(),
            metrics: MetricsHook::default(),
        }
    }

    /// Attach a backend-neutral observer. Event types cannot carry source or metadata strings.
    pub fn with_metrics(mut self, sink: impl MetricsSink) -> Self {
        self.metrics = MetricsHook::new(sink);
        self
    }

    pub fn with_metrics_hook(mut self, metrics: MetricsHook) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn metrics_hook(&self) -> &MetricsHook {
        &self.metrics
    }

    pub fn with_detection_options(
        mut self,
        options: DetectionOptions,
    ) -> Result<Self, IngestError> {
        options.validate()?;
        self.detection_options = options;
        Ok(self)
    }

    pub fn registry(&self) -> &ParserRegistry {
        &self.registry
    }

    pub fn detection_options(&self) -> &DetectionOptions {
        &self.detection_options
    }

    pub fn ingest(
        &self,
        request: ParseRequest<AutoFormatOptions>,
    ) -> Result<Envelope<Value>, IngestError> {
        let control = OperationControl::new(&request.budget, request.cancellation.clone())?;
        self.ingest_with_control(request, control)
    }

    pub(crate) fn ingest_with_control(
        &self,
        request: ParseRequest<AutoFormatOptions>,
        control: OperationControl,
    ) -> Result<Envelope<Value>, IngestError> {
        let started = Instant::now();
        let usage_before = control.usage();
        let result = self.ingest_inner(request, control.clone());
        let usage = control.usage();
        let (status, warnings, repairs) =
            result
                .as_ref()
                .map_or((OperationStatus::Failed, 0, 0), |envelope| {
                    (
                        envelope.status,
                        envelope
                            .diagnostics
                            .iter()
                            .filter(|diagnostic| {
                                diagnostic.severity == crate::core::Severity::Warning
                            })
                            .count(),
                        envelope
                            .diagnostics
                            .iter()
                            .filter(|diagnostic| diagnostic.code.as_str().contains("repair"))
                            .count(),
                    )
                });
        let event = MetricEvent::new(
            OperationKind::Ingest,
            MetricPhase::Complete,
            MetricValues {
                input_bytes: usage.input_bytes.saturating_sub(usage_before.input_bytes),
                parser_elapsed_micros: u64::try_from(started.elapsed().as_micros())
                    .unwrap_or(u64::MAX),
                provider_elapsed_micros: usage.provider_millis.saturating_mul(1_000),
                repairs: u64::try_from(repairs).unwrap_or(u64::MAX),
                warnings: u64::try_from(warnings).unwrap_or(u64::MAX),
                ..MetricValues::default()
            },
        )
        .with_status(status)
        .with_budget_usage(usage);
        self.metrics.emit(&event);
        result
    }

    fn ingest_inner(
        &self,
        request: ParseRequest<AutoFormatOptions>,
        control: OperationControl,
    ) -> Result<Envelope<Value>, IngestError> {
        let ParseRequest {
            request_id,
            input,
            source,
            format_hint,
            options,
            format_options,
            budget,
            cancellation: _,
            providers,
        } = request;
        let terminal_digest = options_digest(&json!({
            "shared": &options,
            "detection": &self.detection_options,
            "format_hint": &format_hint,
        }))?;
        let resolved = match input.resolve_with_control(&control) {
            Ok(input) => input,
            Err(error) => {
                return self.input_error_envelope(source, terminal_digest, error, &control);
            }
        };
        let limits = detection_limits(control.budget().budget());
        let detection = detect_source(
            &source,
            resolved.raw_bytes(),
            format_hint.as_ref(),
            &limits,
            &self.registry,
            &self.detection_options,
        )?;
        let detected_identity = detection.apply_to_identity(resolved.content_identity());
        let selected_parser = detection.selected_parser.clone();
        let resolved_request = ResolvedParseRequest {
            request_id,
            input: resolved,
            source: source.clone(),
            format_hint,
            options,
            format_options,
            budget,
            control: control.clone(),
            providers,
        };

        match (detection.status, selected_parser) {
            (DetectionStatus::Selected, Some(parser_id)) => {
                let envelope =
                    self.registry
                        .dispatch_selected(&parser_id, resolved_request, None)?;
                self.finish_detected(envelope, detection, detected_identity, &control)
            }
            _ => self.detection_terminal(
                source,
                terminal_digest,
                detection,
                detected_identity,
                &control,
            ),
        }
    }

    fn finish_detected(
        &self,
        envelope: Envelope<Value>,
        detection: Detection,
        mut identity: ContentIdentity,
        control: &OperationControl,
    ) -> Result<Envelope<Value>, IngestError> {
        let original_size = serde_json::to_vec(&envelope)?.len() as u64;
        if let Some(parser_identity) = envelope.identity.as_ref() {
            identity.decoded.clone_from(&parser_identity.decoded);
            identity
                .canonical_payload
                .clone_from(&parser_identity.canonical_payload);
            identity.aggregate.clone_from(&parser_identity.aggregate);
        }
        let mut diagnostics = detection.diagnostics;
        diagnostics.extend(envelope.diagnostics.clone());
        let mut envelope = envelope
            .with_operation(OperationKind::Ingest)
            .with_identity(identity)
            .with_diagnostics(diagnostics)
            .with_canonical_payload_identity()?;
        normalize_control_status(&mut envelope);
        envelope.validate()?;

        let final_size = serde_json::to_vec(&envelope)?.len() as u64;
        if final_size > original_size {
            if let Err(error) = control
                .budget()
                .consume_output_bytes(final_size - original_size)
            {
                return budget_terminal_from(&envelope, error);
            }
        }
        Ok(envelope)
    }

    fn detection_terminal(
        &self,
        source: SourceInfo,
        digest: String,
        mut detection: Detection,
        identity: ContentIdentity,
        control: &OperationControl,
    ) -> Result<Envelope<Value>, IngestError> {
        let (status, message) = match detection.status {
            DetectionStatus::Ambiguous => (
                OperationStatus::Ambiguous,
                "detection retained multiple candidates within the configured ambiguity margin",
            ),
            DetectionStatus::Unsupported => (
                OperationStatus::Unsupported,
                "the recognized format has no available parser",
            ),
            DetectionStatus::Unknown | DetectionStatus::Selected => (
                OperationStatus::Unsupported,
                "no registered parser could be selected from the available evidence",
            ),
        };
        if !detection.diagnostics.iter().any(|diagnostic| {
            diagnostic.class == DiagnosticClass::UnsupportedContent
                || diagnostic.code.as_str() == "detect.ambiguous"
        }) {
            detection
                .diagnostics
                .push(Diagnostic::unsupported("grist.ingest", message));
        }
        let kind = artifact_kind(&self.registry, &detection);
        let mut envelope = Envelope::without_payload(
            OperationKind::Ingest,
            kind,
            status,
            source,
            ParserInfo::new("grist.ingest"),
            digest,
            "grist/unsupported/v1",
        )?
        .with_identity(identity)
        .with_diagnostics(detection.diagnostics);
        normalize_control_status(&mut envelope);
        let size = serde_json::to_vec(&envelope)?.len() as u64;
        if let Err(error) = control.budget().consume_output_bytes(size) {
            return budget_terminal_from(&envelope, error);
        }
        Ok(envelope)
    }

    fn input_error_envelope(
        &self,
        source: SourceInfo,
        digest: String,
        error: InputError,
        control: &OperationControl,
    ) -> Result<Envelope<Value>, IngestError> {
        let (status, diagnostic) = input_error_metadata(error, control);
        Ok(Envelope::without_payload(
            OperationKind::Ingest,
            ArtifactKind::FileIngest,
            status,
            source,
            ParserInfo::new("grist.ingest"),
            digest,
            SchemaVersion::ENVELOPE_V2,
        )?
        .with_diagnostics(vec![diagnostic]))
    }
}

fn detection_limits(budget: &crate::core::ResourceBudget) -> Limits {
    Limits {
        max_file_bytes: budget
            .max_input_bytes
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(usize::MAX),
        max_parse_depth: budget
            .max_nesting_depth
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(usize::MAX),
        ..Limits::default()
    }
}

fn artifact_kind(registry: &ParserRegistry, detection: &Detection) -> ArtifactKind {
    let Some(candidate) = detection.candidates.first() else {
        return ArtifactKind::Unsupported;
    };
    match registry.select_format(&candidate.identity.format) {
        ParserSelection::Available(descriptor) => descriptor.format.artifact_kind,
        ParserSelection::Unsupported { unavailable, .. } => unavailable
            .first()
            .map(|entry| entry.descriptor.format.artifact_kind.clone())
            .unwrap_or(ArtifactKind::Unsupported),
    }
}

fn input_error_metadata(
    error: InputError,
    control: &OperationControl,
) -> (OperationStatus, Diagnostic) {
    match error {
        InputError::Cancelled => (
            OperationStatus::Cancelled,
            crate::core::CancellationError.diagnostic("grist.ingest"),
        ),
        InputError::ByteLimitExceeded {
            limit,
            observed_at_least,
        } => {
            let error = BudgetExceeded {
                axis: BudgetAxis::InputBytes,
                limit: BudgetAmount::Count(limit),
                observed: BudgetAmount::Count(observed_at_least),
                usage: Box::new(control.usage()),
            };
            (OperationStatus::Failed, error.diagnostic("grist.ingest"))
        }
        InputError::BudgetExceeded(error) => {
            (OperationStatus::Failed, error.diagnostic("grist.ingest"))
        }
        InputError::Io(error) => {
            let mut diagnostic = Diagnostic::malformed("grist.ingest", error.to_string());
            diagnostic.code = "grist.input.io".into();
            (OperationStatus::Failed, diagnostic)
        }
        InputError::InvalidBudget(error) => {
            let mut diagnostic =
                Diagnostic::error("grist.ingest", "grist.budget.invalid", error.to_string());
            diagnostic.class = DiagnosticClass::ResourceBudgetExhaustion;
            (OperationStatus::Failed, diagnostic)
        }
    }
}

fn normalize_control_status(envelope: &mut Envelope<Value>) {
    if envelope
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code.as_str() == "grist.operation.cancelled")
    {
        envelope.status = OperationStatus::Cancelled;
        envelope.payload = None;
    }
}

fn budget_terminal_from(
    envelope: &Envelope<Value>,
    error: BudgetExceeded,
) -> Result<Envelope<Value>, IngestError> {
    Ok(Envelope::without_payload(
        OperationKind::Ingest,
        envelope.kind.clone(),
        OperationStatus::Failed,
        envelope.source.clone(),
        envelope.parser.clone(),
        envelope.options_digest.clone(),
        envelope.payload_schema_version.clone(),
    )?
    .with_identity(envelope.identity.clone().unwrap_or_default())
    .with_diagnostics(vec![error.diagnostic(&envelope.parser.name)]))
}
