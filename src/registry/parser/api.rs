use super::super::ParserDescriptor;
use crate::core::{
    DecodedContentIdentity, Diagnostic, Envelope, NetworkAccess, OperationControl, OperationStatus,
    ProvenanceStep, Provider, ProviderInvocation, ProviderKind, ResolvedParseRequest, SourceInfo,
};
use crate::decode::{DecodeError, DecodeOptions, DecodedText, decode_text};
use crate::provider::{ProviderContractError, ProviderRequest, ProviderResponse};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Constrained execution surface for registered parsers.
pub struct ParserContext<'a> {
    pub(super) request: &'a ResolvedParseRequest,
    pub(super) descriptor: &'a ParserDescriptor,
    pub(super) format_options: &'a Value,
    pub(super) used_provider_names: BTreeSet<String>,
    decoded: OnceLock<Result<DecodedText, DecodeError>>,
}

pub type ParserError = Box<Diagnostic>;

pub trait Parser: Send + Sync + 'static {
    fn parse(&self, context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError>;
}

impl<'a> ParserContext<'a> {
    pub(super) fn new(
        request: &'a ResolvedParseRequest,
        descriptor: &'a ParserDescriptor,
        format_options: &'a Value,
    ) -> Self {
        Self {
            request,
            descriptor,
            format_options,
            used_provider_names: BTreeSet::new(),
            decoded: OnceLock::new(),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        self.request.input.raw_bytes()
    }

    pub fn source(&self) -> &SourceInfo {
        &self.request.source
    }

    pub fn options(&self) -> &Value {
        self.format_options
    }

    pub fn control(&self) -> &OperationControl {
        &self.request.control
    }

    pub fn consume_cells(&self, count: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .consume_cells(count)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    /// Charge parser-discovered local inputs that were not part of the primary request body.
    pub fn consume_input_bytes(&self, count: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .consume_input_bytes(count)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn checkpoint(&self) -> Result<(), ParserError> {
        self.request
            .control
            .checkpoint()
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn utf8_text(&self) -> Result<&str, ParserError> {
        self.decoded_text().map(|decoded| decoded.text.as_str())
    }

    /// Decode through the shared charset contract and retain the exact raw bytes.
    pub fn decoded_text(&self) -> Result<&DecodedText, ParserError> {
        let options = DecodeOptions::for_media_type(
            self.request.source.declared_mime_type.as_deref(),
            Some(&self.descriptor.format.id),
        );
        self.decoded_text_with_options(options)
    }

    /// Decode through the shared contract with parser-specific caller
    /// evidence, while retaining one deterministic decode per parser run.
    pub fn decoded_text_with_options(
        &self,
        options: DecodeOptions,
    ) -> Result<&DecodedText, ParserError> {
        self.decoded
            .get_or_init(|| {
                if self.request.input.declared_utf8() {
                    Ok(crate::decode::decode_declared_utf8(
                        self.request
                            .input
                            .utf8_text()
                            .expect("declared UTF-8 input invariant"),
                    ))
                } else {
                    decode_text(self.bytes(), &options)
                }
            })
            .as_ref()
            .map_err(|error| Box::new(error.diagnostic().with_parser(&self.descriptor.parser.name)))
    }

    /// Decode the resolved raw bytes identically for every input adapter.
    ///
    /// Format parsers whose authoritative payload includes decode evidence use
    /// this when canonical payload identity must depend on bytes and options,
    /// not on whether the caller supplied bytes, a string, a reader, or a path.
    pub fn decoded_bytes_with_options(
        &self,
        options: DecodeOptions,
    ) -> Result<&DecodedText, ParserError> {
        self.decoded
            .get_or_init(|| decode_text(self.bytes(), &options))
            .as_ref()
            .map_err(|error| Box::new(error.diagnostic().with_parser(&self.descriptor.parser.name)))
    }

    pub(super) fn decoding_metadata(
        &self,
    ) -> Option<(DecodedContentIdentity, Vec<Diagnostic>, bool)> {
        let decoded = self.decoded.get()?.as_ref().ok()?;
        Some((
            decoded.report.decoded_identity.clone(),
            decoded.report.diagnostics.clone(),
            decoded.report.makes_operation_partial(),
        ))
    }

    pub fn consume_decoded_characters(&self, count: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .consume_decoded_characters(count)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn consume_nodes(&self, count: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .consume_nodes(count)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn consume_records(&self, count: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .consume_records(count)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn consume_archive_members(&self, count: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .consume_archive_members(count)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn consume_child_artifacts(&self, count: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .consume_child_artifacts(count)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn observe_archive_expansion(
        &self,
        compressed: u64,
        expanded: u64,
    ) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .observe_archive_expansion(compressed, expanded)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn observe_memory_bytes(&self, bytes: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .observe_memory_bytes(bytes)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn observe_nesting_depth(&self, depth: u64) -> Result<(), ParserError> {
        self.request
            .control
            .budget()
            .observe_nesting_depth(depth)
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))
    }

    pub fn provider_is_allowed(&self, kind: ProviderKind) -> bool {
        self.descriptor.allowed_providers.contains(&kind)
    }

    /// Return the explicitly selected network policy for a provider kind.
    /// Absence means the caller did not select that provider for this request.
    pub fn selected_provider_network(&self, kind: ProviderKind) -> Option<NetworkAccess> {
        self.request
            .providers
            .selected(kind)
            .map(crate::core::ProviderBinding::network_access)
    }

    pub fn run_provider(
        &mut self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderResponse, ParserError> {
        let kind = request.kind();
        if !self.provider_is_allowed(kind) {
            return Err(Box::new(Diagnostic::security_rejection(
                &self.descriptor.parser.name,
                "provider kind not permitted by parser registration",
            )));
        }
        let binding = self.request.providers.selected(kind).ok_or_else(|| {
            Box::new(Diagnostic::provider_failure(
                &self.descriptor.parser.name,
                "provider was not explicitly selected",
            ))
        })?;
        let name = binding.provider().name().to_string();
        let response = self
            .request
            .control
            .run_provider(|| binding.invoke(request))
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))?
            .map_err(|error| {
                let diagnostic = match error {
                    ProviderContractError::NetworkPermissionMismatch
                    | ProviderContractError::BackendPermissionDenied => {
                        Diagnostic::security_rejection(
                            &self.descriptor.parser.name,
                            error.to_string(),
                        )
                    }
                    _ => Diagnostic::provider_failure(
                        &self.descriptor.parser.name,
                        error.to_string(),
                    ),
                };
                Box::new(diagnostic)
            })?;
        self.used_provider_names.insert(name);
        Ok(response)
    }

    /// Build and invoke a provider request over the parser's resolved input
    /// without cloning the input bytes or exposing a binding to the parser.
    pub fn run_provider_for_input<T, F>(
        &mut self,
        kind: ProviderKind,
        public_configuration: &T,
        build: F,
    ) -> Result<ProviderResponse, ParserError>
    where
        T: Serialize + ?Sized,
        F: for<'input> FnOnce(
            crate::provider::ProviderRequestContext<'input>,
        ) -> ProviderRequest<'input>,
    {
        if !self.provider_is_allowed(kind) {
            return Err(Box::new(Diagnostic::security_rejection(
                &self.descriptor.parser.name,
                "provider kind not permitted by parser registration",
            )));
        }
        let binding = self.request.providers.selected(kind).ok_or_else(|| {
            Box::new(Diagnostic::provider_failure(
                &self.descriptor.parser.name,
                "provider was not explicitly selected",
            ))
        })?;
        let name = binding.provider().name().to_string();
        let request_context = crate::provider::ProviderRequestContext::new(
            self.request.input.raw_bytes(),
            binding.network_access(),
            public_configuration,
        )
        .map_err(|error| {
            Box::new(Diagnostic::provider_failure(
                &self.descriptor.parser.name,
                error.to_string(),
            ))
        })?;
        let request = build(request_context);
        if request.kind() != kind {
            return Err(Box::new(Diagnostic::parser_defect(
                &self.descriptor.parser.name,
                "provider request builder returned the wrong request kind",
            )));
        }
        let response = self
            .request
            .control
            .run_provider(|| binding.invoke(&request))
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))?
            .map_err(|error| {
                let diagnostic = match error {
                    ProviderContractError::NetworkPermissionMismatch
                    | ProviderContractError::BackendPermissionDenied => {
                        Diagnostic::security_rejection(
                            &self.descriptor.parser.name,
                            error.to_string(),
                        )
                    }
                    _ => Diagnostic::provider_failure(
                        &self.descriptor.parser.name,
                        error.to_string(),
                    ),
                };
                Box::new(diagnostic)
            })?;
        self.used_provider_names.insert(name);
        Ok(response)
    }

    pub fn run_provider_json(
        &mut self,
        kind: ProviderKind,
        operation: impl FnOnce(&dyn Provider, NetworkAccess) -> Result<Value, ParserError>,
    ) -> Result<Value, ParserError> {
        if !self.provider_is_allowed(kind) {
            return Err(Box::new(Diagnostic::security_rejection(
                &self.descriptor.parser.name,
                "provider kind not permitted by parser registration",
            )));
        }
        let binding = self.request.providers.selected(kind).ok_or_else(|| {
            Box::new(Diagnostic::provider_failure(
                &self.descriptor.parser.name,
                "provider was not explicitly selected",
            ))
        })?;
        let name = binding.provider().name().to_string();
        let network = binding.network_access();
        let value = self
            .request
            .control
            .run_provider(|| operation(binding.provider(), network))
            .map_err(|error| Box::new(error.diagnostic(&self.descriptor.parser.name)))??;
        self.used_provider_names.insert(name);
        Ok(value)
    }
}

/// Result data produced before the registry constructs the public envelope.
#[derive(Debug, Clone)]
pub struct ParserOutput {
    pub status: OperationStatus,
    pub payload: Option<Value>,
    pub diagnostics: Vec<Diagnostic>,
    pub providers: Vec<ProviderInvocation>,
    pub provenance: Vec<ProvenanceStep>,
}

impl ParserOutput {
    pub fn complete(payload: impl Into<Value>) -> Self {
        Self {
            status: OperationStatus::Complete,
            payload: Some(payload.into()),
            diagnostics: Vec::new(),
            providers: Vec::new(),
            provenance: Vec::new(),
        }
    }

    pub fn partial(payload: Option<Value>, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            status: OperationStatus::Partial,
            payload,
            diagnostics,
            providers: Vec::new(),
            provenance: Vec::new(),
        }
    }

    pub fn terminal(status: OperationStatus, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            status,
            payload: None,
            diagnostics,
            providers: Vec::new(),
            provenance: Vec::new(),
        }
    }

    pub fn from_envelope<T: Serialize>(envelope: Envelope<T>) -> Result<Self, serde_json::Error> {
        Ok(Self {
            status: envelope.status,
            payload: envelope.payload.map(serde_json::to_value).transpose()?,
            diagnostics: envelope.diagnostics,
            providers: envelope.providers,
            provenance: envelope.provenance.into_iter().skip(1).collect(),
        })
    }
}
