use super::super::{ParserDescriptor, UnavailableParser, UnavailableReason};
use super::registry::ParserEntry;
use super::{ParserContext, ParserOutput, ParserRegistry, ParserSelection};
use crate::core::{
    ContentIdentity, Diagnostic, Envelope, EnvelopeInvariantError, InputError, OperationKind,
    OperationStatus, ParseRequest, ParserInfo, ResolvedParseRequest, SourceInfo, options_digest,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug, thiserror::Error)]
pub enum ParserDispatchError {
    #[error(transparent)]
    Input(InputError),
    #[error(transparent)]
    Serialization(serde_json::Error),
    #[error(transparent)]
    Envelope(EnvelopeInvariantError),
}

impl From<InputError> for ParserDispatchError {
    fn from(value: InputError) -> Self {
        Self::Input(value)
    }
}

impl From<serde_json::Error> for ParserDispatchError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value)
    }
}

impl From<EnvelopeInvariantError> for ParserDispatchError {
    fn from(value: EnvelopeInvariantError) -> Self {
        Self::Envelope(value)
    }
}

impl ParserRegistry {
    pub fn dispatch(
        &self,
        format: &str,
        request: ParseRequest,
        format_options: Option<Value>,
    ) -> Result<Envelope<Value>, ParserDispatchError> {
        match self.select_format(format) {
            ParserSelection::Available(descriptor) => {
                self.dispatch_selected(&descriptor.id, request.resolve_input()?, format_options)
            }
            ParserSelection::Unsupported {
                format,
                unavailable,
            } => {
                let options = format_options.unwrap_or_else(|| json!({}));
                let digest = options_digest(&json!({
                    "shared": &request.options,
                    "format": &options,
                }))?;
                unsupported_envelope(format, unavailable, request.source, digest)
            }
        }
    }

    pub fn dispatch_selected(
        &self,
        parser_id: &str,
        request: ResolvedParseRequest,
        format_options: Option<Value>,
    ) -> Result<Envelope<Value>, ParserDispatchError> {
        let Some(entry) = self.entries.get(parser_id) else {
            let unavailable = self
                .unavailable
                .get(parser_id)
                .cloned()
                .into_iter()
                .collect();
            return unsupported_envelope(
                parser_id.to_string(),
                unavailable,
                request.source,
                options_digest(&json!({}))?,
            );
        };
        let options = format_options.unwrap_or_else(|| entry.descriptor.options.default.clone());
        let digest = options_digest(&json!({
            "shared": &request.options,
            "format": &options,
        }))?;
        self.execute_entry(entry, request, options, digest)
    }

    fn execute_entry(
        &self,
        entry: &ParserEntry,
        request: ResolvedParseRequest,
        options: Value,
        digest: String,
    ) -> Result<Envelope<Value>, ParserDispatchError> {
        let identity = request.input.content_identity();
        if !options.is_object() {
            return terminal_envelope(
                &entry.descriptor,
                request.source,
                identity,
                digest,
                OperationStatus::Failed,
                Diagnostic::malformed(
                    &entry.descriptor.parser.name,
                    "format options must be a JSON object",
                ),
            );
        }
        self.check_required_providers(entry, request, options, digest, identity)
    }

    fn check_required_providers(
        &self,
        entry: &ParserEntry,
        request: ResolvedParseRequest,
        options: Value,
        digest: String,
        identity: ContentIdentity,
    ) -> Result<Envelope<Value>, ParserDispatchError> {
        for kind in &entry.descriptor.required_providers {
            if request.providers.selected(*kind).is_none() {
                let diagnostic = Diagnostic::unsupported(
                    &entry.descriptor.parser.name,
                    "required provider was not explicitly selected",
                );
                return terminal_envelope(
                    &entry.descriptor,
                    request.source,
                    identity,
                    digest,
                    OperationStatus::Unsupported,
                    diagnostic,
                );
            }
        }
        self.run_parser(entry, request, options, digest, identity)
    }

    fn run_parser(
        &self,
        entry: &ParserEntry,
        request: ResolvedParseRequest,
        options: Value,
        digest: String,
        mut identity: ContentIdentity,
    ) -> Result<Envelope<Value>, ParserDispatchError> {
        if let Err(error) = request.control.checkpoint() {
            return terminal_envelope(
                &entry.descriptor,
                request.source,
                identity,
                digest,
                error.operation_status(0),
                error.diagnostic(&entry.descriptor.parser.name),
            );
        }
        let source = request.source.clone();
        let mut context = ParserContext::new(&request, &entry.descriptor, &options);
        let parsed = catch_unwind(AssertUnwindSafe(|| entry.parser.parse(&mut context)));
        let decoding = context.decoding_metadata();
        let used_provider_names = context.used_provider_names;
        let mut output = match parsed {
            Ok(Ok(output)) => output,
            Ok(Err(diagnostic)) => {
                return terminal_envelope(
                    &entry.descriptor,
                    source,
                    identity,
                    digest,
                    OperationStatus::Failed,
                    *diagnostic,
                );
            }
            Err(_) => {
                let diagnostic = Diagnostic::parser_defect(
                    &entry.descriptor.parser.name,
                    "parser panicked across the registry boundary",
                );
                return terminal_envelope(
                    &entry.descriptor,
                    source,
                    identity,
                    digest,
                    OperationStatus::Failed,
                    diagnostic,
                );
            }
        };
        if let Some((decoded_identity, mut diagnostics, makes_partial)) = decoding {
            identity.decoded = Some(decoded_identity);
            diagnostics.append(&mut output.diagnostics);
            output.diagnostics = diagnostics;
            if makes_partial && output.status == OperationStatus::Complete {
                output.status = OperationStatus::Partial;
            }
        }
        self.finish_output(
            entry,
            request,
            digest,
            identity,
            output,
            used_provider_names,
        )
    }

    fn finish_output(
        &self,
        entry: &ParserEntry,
        request: ResolvedParseRequest,
        digest: String,
        identity: ContentIdentity,
        output: ParserOutput,
        used_provider_names: BTreeSet<String>,
    ) -> Result<Envelope<Value>, ParserDispatchError> {
        let source = request.source.clone();
        if used_provider_names.iter().any(|name| {
            !output
                .providers
                .iter()
                .any(|invocation| &invocation.provider == name)
        }) {
            let diagnostic = Diagnostic::parser_defect(
                &entry.descriptor.parser.name,
                "provider invocation metadata is missing",
            );
            return terminal_envelope(
                &entry.descriptor,
                source,
                identity,
                digest,
                OperationStatus::Failed,
                diagnostic,
            );
        }
        let invalid_payload = output.status.requires_payload() && output.payload.is_none()
            || !output.status.permits_payload() && output.payload.is_some();
        if invalid_payload {
            let diagnostic = Diagnostic::parser_defect(
                &entry.descriptor.parser.name,
                "parser output violated status and payload semantics",
            );
            return terminal_envelope(
                &entry.descriptor,
                source,
                identity,
                digest,
                OperationStatus::Failed,
                diagnostic,
            );
        }
        if let Err(error) = request.control.checkpoint() {
            return terminal_envelope(
                &entry.descriptor,
                source,
                identity,
                digest,
                error.operation_status(0),
                error.diagnostic(&entry.descriptor.parser.name),
            );
        }
        self.enforce_output_budget(entry, request, source, digest, identity, output)
    }

    fn enforce_output_budget(
        &self,
        entry: &ParserEntry,
        request: ResolvedParseRequest,
        source: SourceInfo,
        digest: String,
        identity: ContentIdentity,
        output: ParserOutput,
    ) -> Result<Envelope<Value>, ParserDispatchError> {
        let envelope = build_envelope(&entry.descriptor, source, digest.clone(), output)?
            .with_identity(identity.clone())
            .with_canonical_payload_identity()?;
        let output_size = serde_json::to_vec(&envelope)?.len() as u64;
        if let Err(error) = request.control.budget().consume_output_bytes(output_size) {
            return terminal_envelope(
                &entry.descriptor,
                envelope.source,
                identity,
                digest,
                OperationStatus::Failed,
                error.diagnostic(&entry.descriptor.parser.name),
            );
        }
        Ok(envelope)
    }
}

fn build_envelope(
    descriptor: &ParserDescriptor,
    source: SourceInfo,
    digest: String,
    output: ParserOutput,
) -> Result<Envelope<Value>, EnvelopeInvariantError> {
    let mut envelope = match output.status {
        OperationStatus::Complete => Envelope::complete(
            OperationKind::Parse,
            descriptor.format.artifact_kind.clone(),
            source,
            descriptor.parser.clone(),
            digest,
            descriptor.payload_schema.version.as_str(),
            output.payload.unwrap_or_default(),
        ),
        OperationStatus::Partial => Envelope::partial(
            OperationKind::Parse,
            descriptor.format.artifact_kind.clone(),
            source,
            descriptor.parser.clone(),
            digest,
            descriptor.payload_schema.version.as_str(),
            output.payload,
        ),
        terminal => terminal_base(descriptor, source, digest, terminal)?,
    };
    envelope.providers = output.providers;
    envelope.diagnostics = output.diagnostics;
    envelope.provenance.extend(output.provenance);
    envelope.validate()?;
    Ok(envelope)
}

fn terminal_base(
    descriptor: &ParserDescriptor,
    source: SourceInfo,
    digest: String,
    status: OperationStatus,
) -> Result<Envelope<Value>, EnvelopeInvariantError> {
    Envelope::without_payload(
        OperationKind::Parse,
        descriptor.format.artifact_kind.clone(),
        status,
        source,
        descriptor.parser.clone(),
        digest,
        descriptor.payload_schema.version.as_str(),
    )
}

fn terminal_envelope(
    descriptor: &ParserDescriptor,
    source: SourceInfo,
    identity: ContentIdentity,
    digest: String,
    status: OperationStatus,
    diagnostic: Diagnostic,
) -> Result<Envelope<Value>, ParserDispatchError> {
    Ok(terminal_base(descriptor, source, digest, status)?
        .with_identity(identity)
        .with_diagnostics(vec![diagnostic]))
}

fn unsupported_envelope(
    format: String,
    unavailable: Vec<UnavailableParser>,
    source: SourceInfo,
    digest: String,
) -> Result<Envelope<Value>, ParserDispatchError> {
    let kind = unavailable
        .first()
        .map(|item| item.descriptor.format.artifact_kind.clone())
        .unwrap_or(crate::core::ArtifactKind::Unsupported);
    let schema = unavailable
        .first()
        .map(|item| item.descriptor.payload_schema.version.clone())
        .unwrap_or_else(|| "grist/unsupported/v1".to_string());
    unsupported_result(format, unavailable, source, digest, kind, schema)
}

fn unsupported_result(
    format: String,
    unavailable: Vec<UnavailableParser>,
    source: SourceInfo,
    digest: String,
    kind: crate::core::ArtifactKind,
    schema: String,
) -> Result<Envelope<Value>, ParserDispatchError> {
    let reason = unavailable
        .first()
        .map(|item| unavailable_message(format.as_str(), &item.reason))
        .unwrap_or(format);
    let parser = ParserInfo::new("grist.registry");
    let envelope = Envelope::without_payload(
        OperationKind::Parse,
        kind,
        OperationStatus::Unsupported,
        source,
        parser,
        digest,
        schema.as_str(),
    )?;
    Ok(envelope.with_diagnostics(vec![Diagnostic::unsupported("grist.registry", reason)]))
}

fn unavailable_message(format: &str, reason: &UnavailableReason) -> String {
    let mut message = format.to_string();
    message.push_str(&serde_json::to_string(reason).unwrap_or_default());
    message
}
