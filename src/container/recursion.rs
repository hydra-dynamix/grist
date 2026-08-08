//! Deterministic recursive traversal shared by every container format.

use super::{
    ArtifactDisposition, ArtifactExtraction, ArtifactExtractionStatus, ArtifactMetadata,
    ArtifactParent, ArtifactRelationship, ContentAddressedArtifactSink, EmbeddedArtifact,
};
use crate::core::{
    AutoFormatOptions, BudgetAxis, BudgetExceeded, BudgetSelection, BudgetUsage, CancellationToken,
    CompoundMemberInput, ContentIdentity, Diagnostic, DiagnosticClass, Envelope, FormatHint,
    FormatIdentity, Input, LocationComponent, OperationControl, OperationControlError,
    OperationStatus, ParseOptions, ParseRequest, ProviderSet, RequestId, ResourceBudget,
    SchemaVersion, SourceInfo, SourceLocator, SourceLocatorError, canonical_json_sha256,
};
use crate::ingest::{IngestError, Ingestor};
use crate::security::{
    ArchiveEntryKind, ArchiveMemberDescriptor, ArchiveRejection, ArchiveSecurityPolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerClass {
    Archive,
    Package,
    Email,
    CompoundDocument,
}

impl ContainerClass {
    const fn charges_archive_members(self) -> bool {
        matches!(self, Self::Archive | Self::Package)
    }
}

/// A locator expressed only in the coordinate system of the immediate parent.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParentRelativeLocator {
    components: Vec<LocationComponent>,
}

impl ParentRelativeLocator {
    pub fn new(components: Vec<LocationComponent>) -> Result<Self, SourceLocatorError> {
        let locator = Self { components };
        locator.validate()?;
        Ok(locator)
    }

    pub fn single(component: impl Into<LocationComponent>) -> Result<Self, SourceLocatorError> {
        Self::new(vec![component.into()])
    }

    pub fn components(&self) -> &[LocationComponent] {
        &self.components
    }

    pub fn archive_member_path(&self) -> Option<&str> {
        self.components
            .iter()
            .rev()
            .find_map(|component| match component {
                LocationComponent::ArchiveMember { member_path, .. } => Some(member_path.as_str()),
                _ => None,
            })
    }

    pub fn resolve(
        &self,
        parent: Option<&SourceLocator>,
    ) -> Result<SourceLocator, SourceLocatorError> {
        let mut components = self.components.iter().cloned();
        let first = components
            .next()
            .expect("validated relative locators are non-empty");
        let mut resolved = match parent {
            Some(parent) => parent.clone().nested(first)?,
            None => SourceLocator::exact(first)?,
        };
        for component in components {
            resolved = resolved.nested(component)?;
        }
        Ok(resolved)
    }

    pub fn validate(&self) -> Result<(), SourceLocatorError> {
        let mut components = self.components.iter().cloned();
        let first = components
            .next()
            .ok_or(SourceLocatorError::EmptyComponents)?;
        let mut locator = SourceLocator::exact(first)?;
        for component in components {
            locator = locator.nested(component)?;
        }
        Ok(())
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerUnavailableKind {
    Skipped,
    Encrypted,
    Unsupported,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerUnavailable {
    pub kind: ContainerUnavailableKind,
    pub code: String,
    pub message: Option<String>,
}

impl ContainerUnavailable {
    pub fn new(kind: ContainerUnavailableKind, code: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
            message: None,
        }
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// Format-neutral archive header facts retained for every storage mode.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveMemberMetadata {
    pub entry_kind: ArchiveEntryKind,
    pub compression_method: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crc32: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gid: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_time: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
    pub encrypted: bool,
    pub zip64: bool,
}

#[derive(Debug)]
pub enum ContainerMemberBody {
    Available(Vec<u8>),
    Unavailable(ContainerUnavailable),
}

/// One decoder-produced member before shared policy is applied.
#[derive(Debug)]
pub struct ContainerMember {
    pub source_order: u64,
    pub relative_locator: ParentRelativeLocator,
    pub declared_filename: Option<String>,
    pub media_type: Option<String>,
    pub relationship: ArtifactRelationship,
    pub disposition: ArtifactDisposition,
    pub compressed_bytes: u64,
    pub format_hint: Option<FormatHint>,
    pub nested_container_format: Option<String>,
    pub entry_kind: ArchiveEntryKind,
    pub link_target: Option<String>,
    pub archive_metadata: Option<ArchiveMemberMetadata>,
    pub body: ContainerMemberBody,
}

impl ContainerMember {
    pub fn available(
        source_order: u64,
        relative_locator: ParentRelativeLocator,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        let bytes = bytes.into();
        Self {
            source_order,
            relative_locator,
            declared_filename: None,
            media_type: None,
            relationship: ArtifactRelationship::EmbeddedIn,
            disposition: ArtifactDisposition::Unspecified,
            compressed_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            format_hint: None,
            nested_container_format: None,
            entry_kind: ArchiveEntryKind::RegularFile,
            link_target: None,
            archive_metadata: None,
            body: ContainerMemberBody::Available(bytes),
        }
    }

    pub fn unavailable(
        source_order: u64,
        relative_locator: ParentRelativeLocator,
        unavailable: ContainerUnavailable,
    ) -> Self {
        Self {
            source_order,
            relative_locator,
            declared_filename: None,
            media_type: None,
            relationship: ArtifactRelationship::EmbeddedIn,
            disposition: ArtifactDisposition::Unspecified,
            compressed_bytes: 0,
            format_hint: None,
            nested_container_format: None,
            entry_kind: ArchiveEntryKind::RegularFile,
            link_target: None,
            archive_metadata: None,
            body: ContainerMemberBody::Unavailable(unavailable),
        }
    }

    pub fn with_declared_filename(mut self, value: impl Into<String>) -> Self {
        self.declared_filename = Some(value.into());
        self
    }
    pub fn with_media_type(mut self, value: impl Into<String>) -> Self {
        self.media_type = Some(value.into());
        self
    }
    pub fn with_relationship(mut self, value: ArtifactRelationship) -> Self {
        self.relationship = value;
        self
    }
    pub fn with_disposition(mut self, value: ArtifactDisposition) -> Self {
        self.disposition = value;
        self
    }
    pub fn with_compressed_bytes(mut self, value: u64) -> Self {
        self.compressed_bytes = value;
        self
    }
    pub fn with_format_hint(mut self, value: FormatHint) -> Self {
        self.format_hint = Some(value);
        self
    }
    pub fn with_nested_container_format(mut self, value: impl Into<String>) -> Self {
        self.nested_container_format = Some(value.into());
        self
    }
    pub fn with_entry_kind(mut self, value: ArchiveEntryKind) -> Self {
        self.entry_kind = value;
        self
    }
    pub fn with_link_target(mut self, value: impl Into<String>) -> Self {
        self.link_target = Some(value.into());
        self
    }
    pub fn with_archive_metadata(mut self, value: ArchiveMemberMetadata) -> Self {
        self.archive_metadata = Some(value);
        self
    }
}

/// Read-only execution context supplied to a concrete decoder.
pub struct ContainerDecodeContext<'a> {
    bytes: &'a [u8],
    source: &'a SourceInfo,
    identity: &'a ContentIdentity,
    locator: Option<&'a SourceLocator>,
    depth: u64,
    control: &'a OperationControl,
    archive_policy: &'a ArchiveSecurityPolicy,
}

impl<'a> ContainerDecodeContext<'a> {
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
    pub fn source(&self) -> &'a SourceInfo {
        self.source
    }
    pub fn identity(&self) -> &'a ContentIdentity {
        self.identity
    }
    pub fn locator(&self) -> Option<&'a SourceLocator> {
        self.locator
    }
    pub const fn depth(&self) -> u64 {
        self.depth
    }
    pub fn checkpoint(&self) -> Result<(), ContainerDecodeFailure> {
        self.control
            .checkpoint()
            .map_err(ContainerDecodeFailure::controlled)
    }
    /// Decoders call this before allocating a declared expanded member.
    pub fn check_member_capacity(&self, bytes: u64) -> Result<(), ContainerDecodeFailure> {
        self.control
            .checkpoint()
            .map_err(ContainerDecodeFailure::controlled)?;
        self.control
            .budget()
            .observe_memory_bytes(bytes)
            .map_err(ContainerDecodeFailure::budget)
    }
    pub fn archive_security_policy(&self) -> &ArchiveSecurityPolicy {
        self.archive_policy
    }
    /// Reject declared archive work before allocating or decompressing members.
    pub fn check_archive_preflight(
        &self,
        member_count: u64,
        compressed: u64,
        expanded: u64,
    ) -> Result<(), ContainerDecodeFailure> {
        self.checkpoint()?;
        self.check_archive_member_count(member_count)?;
        let tracker = self.control.budget();
        tracker
            .observe_archive_expansion(compressed, expanded)
            .map_err(ContainerDecodeFailure::budget)?;
        tracker
            .observe_memory_bytes(expanded)
            .map_err(ContainerDecodeFailure::budget)
    }
    pub fn check_archive_member_count(
        &self,
        member_count: u64,
    ) -> Result<(), ContainerDecodeFailure> {
        let tracker = self.control.budget();
        let usage = tracker.snapshot();
        if let Some(limit) = tracker.budget().max_archive_members {
            let observed = usage.archive_members.saturating_add(member_count);
            if observed > limit {
                return Err(ContainerDecodeFailure::budget(BudgetExceeded {
                    axis: BudgetAxis::ArchiveMembers,
                    limit: crate::core::BudgetAmount::Count(limit),
                    observed: crate::core::BudgetAmount::Count(observed),
                    usage: Box::new(usage),
                }));
            }
        }
        Ok(())
    }
}

/// A decoder inventories immediate members only; the controller owns recursion.
pub trait ContainerDecoder: Send + Sync + 'static {
    fn format(&self) -> &str;
    fn class(&self) -> ContainerClass;
    fn decode(
        &self,
        context: &ContainerDecodeContext<'_>,
    ) -> Result<Vec<ContainerMember>, ContainerDecodeFailure>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContainerDecodeFailure {
    pub status: ContainerChildStatus,
    pub diagnostic: Box<Diagnostic>,
    pub budget_axis: Option<BudgetAxis>,
}

impl ContainerDecodeFailure {
    pub fn new(status: ContainerChildStatus, diagnostic: Diagnostic) -> Self {
        Self {
            status,
            diagnostic: Box::new(diagnostic),
            budget_axis: None,
        }
    }
    pub fn malformed(parser: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            ContainerChildStatus::Failed,
            Diagnostic::malformed(parser, message),
        )
    }
    fn budget(error: BudgetExceeded) -> Self {
        let axis = error.axis;
        Self {
            status: ContainerChildStatus::BudgetLimited,
            diagnostic: Box::new(error.diagnostic("grist.container")),
            budget_axis: Some(axis),
        }
    }
    fn controlled(error: OperationControlError) -> Self {
        match error {
            OperationControlError::Cancelled(error) => Self::new(
                ContainerChildStatus::Cancelled,
                error.diagnostic("grist.container"),
            ),
            OperationControlError::BudgetExceeded(error) => Self::budget(error),
        }
    }
}

#[derive(Default)]
pub struct ContainerDecoderRegistry {
    decoders: BTreeMap<String, Arc<dyn ContainerDecoder>>,
}

impl ContainerDecoderRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(
        &mut self,
        decoder: Arc<dyn ContainerDecoder>,
    ) -> Result<(), ContainerRegistryError> {
        let format = decoder.format().trim();
        if format.is_empty()
            || format.chars().any(char::is_control)
            || format != format.to_ascii_lowercase()
        {
            return Err(ContainerRegistryError::InvalidFormat(format.into()));
        }
        if self.decoders.contains_key(format) {
            return Err(ContainerRegistryError::DuplicateFormat(format.into()));
        }
        self.decoders.insert(format.into(), decoder);
        Ok(())
    }
    pub fn get(&self, format: &str) -> Option<&dyn ContainerDecoder> {
        self.decoders.get(format).map(Arc::as_ref)
    }
    pub fn formats(&self) -> Vec<String> {
        self.decoders.keys().cloned().collect()
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ContainerRegistryError {
    #[error("invalid container format ID {0:?}")]
    InvalidFormat(String),
    #[error("duplicate container format registration {0}")]
    DuplicateFormat(String),
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerArtifactMode {
    InventoryOnly,
    InlinePayload,
    ContentAddressed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerParseOptions {
    pub artifact_mode: ContainerArtifactMode,
    pub parse_leaf_payloads: bool,
}

impl ContainerParseOptions {
    pub const fn new(artifact_mode: ContainerArtifactMode) -> Self {
        Self {
            artifact_mode,
            parse_leaf_payloads: true,
        }
    }
    pub const fn without_leaf_payloads(mut self) -> Self {
        self.parse_leaf_payloads = false;
        self
    }
}

pub struct ContainerParseRequest {
    pub request_id: RequestId,
    pub input: Vec<u8>,
    pub source: SourceInfo,
    pub container_format: String,
    pub options: ContainerParseOptions,
    pub parse_options: ParseOptions,
    pub budget: BudgetSelection,
    pub cancellation: CancellationToken,
    pub providers: ProviderSet,
}

impl ContainerParseRequest {
    pub fn new(
        request_id: RequestId,
        input: impl Into<Vec<u8>>,
        source: SourceInfo,
        container_format: impl Into<String>,
        options: ContainerParseOptions,
        budget: BudgetSelection,
    ) -> Self {
        Self {
            request_id,
            input: input.into(),
            source,
            container_format: container_format.into(),
            options,
            parse_options: ParseOptions::default(),
            budget,
            cancellation: CancellationToken::new(),
            providers: ProviderSet::none(),
        }
    }
    pub fn with_parse_options(mut self, value: ParseOptions) -> Self {
        self.parse_options = value;
        self
    }
    pub fn with_cancellation(mut self, value: CancellationToken) -> Self {
        self.cancellation = value;
        self
    }
    pub fn with_providers(mut self, value: ProviderSet) -> Self {
        self.providers = value;
        self
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerChildStatus {
    Parsed,
    InventoryOnly,
    Skipped,
    Encrypted,
    Unsupported,
    Rejected,
    BudgetLimited,
    Failed,
    Cancelled,
}

impl ContainerChildStatus {
    const fn is_loss(self) -> bool {
        !matches!(self, Self::Parsed | Self::InventoryOnly)
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContainerChild {
    pub allocation_id: String,
    pub source_order: u64,
    pub depth: u64,
    pub status: ContainerChildStatus,
    pub artifact: EmbeddedArtifact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_metadata: Option<ArchiveMemberMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed: Option<Envelope<Value>>,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub children: Vec<ContainerChild>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetAllocationNode {
    pub allocation_id: String,
    pub parent_allocation_id: Option<String>,
    pub depth: u64,
    pub locator: Option<SourceLocator>,
    pub allocated: ResourceBudget,
    pub usage_before: BudgetUsage,
    pub usage_after: BudgetUsage,
    pub limit_hit: Option<BudgetAxis>,
    #[serde(default)]
    pub children: Vec<BudgetAllocationNode>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContainerBudgetTree {
    pub limits: ResourceBudget,
    pub root: BudgetAllocationNode,
    pub final_usage: BudgetUsage,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContainerTraversal {
    pub schema_version: String,
    pub request_id: RequestId,
    pub status: OperationStatus,
    pub source: SourceInfo,
    pub identity: ContentIdentity,
    pub container_format: String,
    pub options: ContainerParseOptions,
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub children: Vec<ContainerChild>,
    pub budget_tree: ContainerBudgetTree,
}

pub struct ContainerRecursor<'a> {
    ingestor: &'a Ingestor,
    decoders: &'a ContainerDecoderRegistry,
}

impl<'a> ContainerRecursor<'a> {
    pub fn new(ingestor: &'a Ingestor, decoders: &'a ContainerDecoderRegistry) -> Self {
        Self { ingestor, decoders }
    }

    pub fn parse(
        &self,
        request: ContainerParseRequest,
        sink: Option<&dyn ContentAddressedArtifactSink>,
    ) -> Result<ContainerTraversal, ContainerTraversalError> {
        let control = OperationControl::new(&request.budget, request.cancellation.clone())?;
        self.parse_controlled(request, sink, control, false)
    }

    pub fn parse_with_control(
        &self,
        request: ContainerParseRequest,
        sink: Option<&dyn ContentAddressedArtifactSink>,
        control: OperationControl,
    ) -> Result<ContainerTraversal, ContainerTraversalError> {
        self.parse_controlled(request, sink, control, true)
    }

    fn parse_controlled(
        &self,
        request: ContainerParseRequest,
        sink: Option<&dyn ContentAddressedArtifactSink>,
        control: OperationControl,
        input_already_charged: bool,
    ) -> Result<ContainerTraversal, ContainerTraversalError> {
        if request.container_format.trim().is_empty() {
            return Err(ContainerTraversalError::InvalidContainerFormat);
        }
        if request.options.artifact_mode == ContainerArtifactMode::ContentAddressed
            && sink.is_none()
        {
            return Err(ContainerTraversalError::ContentAddressedSinkRequired);
        }
        let limits = control.budget().budget().clone();
        let identity =
            ContentIdentity::for_raw_bytes(&request.input).with_format(FormatIdentity::new(
                request.container_format.clone(),
                request.source.declared_mime_type.clone(),
            ));
        let root_id = allocation_id(None, None, 0, &identity)?;
        let usage_before = control.usage();
        let mut root = BudgetAllocationNode {
            allocation_id: root_id.clone(),
            parent_allocation_id: None,
            depth: 0,
            locator: None,
            allocated: remaining_budget(&limits, &usage_before),
            usage_before,
            usage_after: control.usage(),
            limit_hit: None,
            children: Vec::new(),
        };
        let root_bytes = u64::try_from(request.input.len()).unwrap_or(u64::MAX);
        let mut initial = control
            .checkpoint()
            .and_then(|()| {
                control
                    .budget()
                    .observe_nesting_depth(0)
                    .map_err(Into::into)
            })
            .and_then(|()| {
                control
                    .budget()
                    .observe_memory_bytes(root_bytes)
                    .map_err(Into::into)
            });
        if !input_already_charged {
            initial = initial.and_then(|()| {
                control
                    .budget()
                    .consume_input_bytes(root_bytes)
                    .map_err(Into::into)
            });
        }
        if let Err(error) = initial {
            let failure = ContainerDecodeFailure::controlled(error);
            root.limit_hit = failure.budget_axis;
            root.usage_after = control.usage();
            let status = if failure.status == ContainerChildStatus::Cancelled {
                OperationStatus::Cancelled
            } else {
                OperationStatus::Failed
            };
            return Ok(ContainerTraversal {
                schema_version: SchemaVersion::CONTAINER_TRAVERSAL_V1.into(),
                request_id: request.request_id,
                status,
                source: request.source,
                identity,
                container_format: request.container_format,
                options: request.options,
                diagnostics: vec![*failure.diagnostic],
                children: Vec::new(),
                budget_tree: ContainerBudgetTree {
                    limits,
                    root,
                    final_usage: control.usage(),
                },
            });
        }

        let cancellation = control.cancellation().clone();
        let mut state = TraversalState {
            recursor: self,
            control,
            budget_selection: request.budget,
            cancellation,
            providers: request.providers,
            parse_options: request.parse_options,
            artifact_mode: request.options.artifact_mode,
            parse_leaf_payloads: request.options.parse_leaf_payloads,
            sink,
            root_compressed_bytes: root_bytes,
            expanded_bytes: 0,
        };
        let decoded = state.decode_container(
            &request.container_format,
            &request.input,
            &request.source,
            &identity,
            None,
            0,
            &root_id,
        );
        let (children, allocations, mut diagnostics, root_failure) = match decoded {
            Ok(value) => value,
            Err(failure) => {
                root.limit_hit = failure.budget_axis;
                (
                    Vec::new(),
                    Vec::new(),
                    vec![*failure.diagnostic],
                    Some(failure.status),
                )
            }
        };
        root.children = allocations;
        root.usage_after = state.control.usage();
        let mut status = traversal_status(&children, root_failure);
        let mut result = ContainerTraversal {
            schema_version: SchemaVersion::CONTAINER_TRAVERSAL_V1.into(),
            request_id: request.request_id,
            status,
            source: request.source,
            identity,
            container_format: request.container_format,
            options: request.options,
            diagnostics: diagnostics.clone(),
            children,
            budget_tree: ContainerBudgetTree {
                limits,
                root,
                final_usage: state.control.usage(),
            },
        };
        let output_bytes = u64::try_from(serde_json::to_vec(&result)?.len()).unwrap_or(u64::MAX);
        if let Err(error) = state.control.budget().consume_output_bytes(output_bytes) {
            diagnostics.push(error.diagnostic("grist.container"));
            if status == OperationStatus::Complete {
                status = OperationStatus::Partial;
            }
            result.status = status;
            result.diagnostics = diagnostics;
            result.budget_tree.root.limit_hit = Some(error.axis);
        }
        result.budget_tree.root.usage_after = state.control.usage();
        result.budget_tree.final_usage = state.control.usage();
        Ok(result)
    }
}

struct TraversalState<'a, 'b> {
    recursor: &'a ContainerRecursor<'a>,
    control: OperationControl,
    budget_selection: BudgetSelection,
    cancellation: CancellationToken,
    providers: ProviderSet,
    parse_options: ParseOptions,
    artifact_mode: ContainerArtifactMode,
    parse_leaf_payloads: bool,
    sink: Option<&'b dyn ContentAddressedArtifactSink>,
    root_compressed_bytes: u64,
    expanded_bytes: u64,
}

type DecodedChildren = (
    Vec<ContainerChild>,
    Vec<BudgetAllocationNode>,
    Vec<Diagnostic>,
    Option<ContainerChildStatus>,
);

impl TraversalState<'_, '_> {
    #[allow(clippy::too_many_arguments)]
    fn decode_container(
        &mut self,
        format: &str,
        bytes: &[u8],
        source: &SourceInfo,
        identity: &ContentIdentity,
        locator: Option<&SourceLocator>,
        depth: u64,
        parent_allocation_id: &str,
    ) -> Result<DecodedChildren, ContainerDecodeFailure> {
        let decoder = self.recursor.decoders.get(format).ok_or_else(|| {
            ContainerDecodeFailure::new(
                ContainerChildStatus::Unsupported,
                Diagnostic::unsupported(
                    "grist.container",
                    format!("no container decoder is registered for {format}"),
                ),
            )
        })?;
        let context = ContainerDecodeContext {
            bytes,
            source,
            identity,
            locator,
            depth,
            control: &self.control,
            archive_policy: &self.parse_options.security.archive,
        };
        let decoded =
            catch_unwind(AssertUnwindSafe(|| decoder.decode(&context))).map_err(|_| {
                ContainerDecodeFailure::new(
                    ContainerChildStatus::Failed,
                    Diagnostic::parser_defect(
                        "grist.container",
                        format!("container decoder {format} panicked"),
                    ),
                )
            })??;
        let class = decoder.class();
        let mut members = decoded;
        members.sort_by_cached_key(member_sort_key);
        let rejections = archive_rejections(class, &members, &self.parse_options.security.archive);
        let mut children = Vec::with_capacity(members.len());
        let mut allocations = Vec::with_capacity(members.len());
        let mut diagnostics = Vec::new();
        for (member, rejection) in members.into_iter().zip(rejections) {
            let (child, allocation) = self.process_member(
                class,
                source,
                identity,
                locator,
                depth.saturating_add(1),
                parent_allocation_id,
                member,
                rejection,
            )?;
            diagnostics.extend(child.diagnostics.clone());
            allocations.push(allocation);
            children.push(child);
        }
        Ok((children, allocations, diagnostics, None))
    }

    #[allow(clippy::too_many_arguments)]
    fn process_member(
        &mut self,
        parent_class: ContainerClass,
        parent_source: &SourceInfo,
        parent_identity: &ContentIdentity,
        parent_locator: Option<&SourceLocator>,
        depth: u64,
        parent_allocation_id: &str,
        member: ContainerMember,
        security_rejection: Option<ArchiveRejection>,
    ) -> Result<(ContainerChild, BudgetAllocationNode), ContainerDecodeFailure> {
        let locator = member
            .relative_locator
            .resolve(parent_locator)
            .map_err(|error| {
                ContainerDecodeFailure::malformed("grist.container", error.to_string())
            })?;
        let archive_metadata = member.archive_metadata.clone();
        let known_identity = match &member.body {
            ContainerMemberBody::Available(bytes) => ContentIdentity::for_raw_bytes(bytes),
            ContainerMemberBody::Unavailable(_) => ContentIdentity::default(),
        };
        let allocation = allocation_id(
            Some(parent_allocation_id),
            Some(&locator),
            member.source_order,
            &known_identity,
        )
        .map_err(|error| {
            ContainerDecodeFailure::new(
                ContainerChildStatus::Failed,
                Diagnostic::parser_defect("grist.container", error.to_string()),
            )
        })?;
        let usage_before = self.control.usage();
        let limits = self.control.budget().budget().clone();
        let mut audit = BudgetAllocationNode {
            allocation_id: allocation.clone(),
            parent_allocation_id: Some(parent_allocation_id.into()),
            depth,
            locator: Some(locator.clone()),
            allocated: remaining_budget(&limits, &usage_before),
            usage_before,
            usage_after: self.control.usage(),
            limit_hit: None,
            children: Vec::new(),
        };
        let metadata = member_metadata(parent_identity, &locator, &member);
        if let Err(failure) = self.charge_member(
            parent_class,
            depth,
            &member.body,
            security_rejection.is_none(),
        ) {
            audit.limit_hit = failure.budget_axis;
            let bytes = available_bytes(&member.body).unwrap_or_default();
            let extraction = ArtifactExtraction::new(
                ArtifactExtractionStatus::BudgetLimited,
                "container.child.budget_limited",
            )
            .with_message(failure.diagnostic.message.clone())
            .with_diagnostic(failure.diagnostic.code.as_str());
            let artifact = EmbeddedArtifact::record_known_unavailable(metadata, bytes, extraction)
                .map_err(artifact_failure)?;
            let diagnostic = attributed(*failure.diagnostic, &artifact);
            audit.usage_after = self.control.usage();
            return Ok((
                ContainerChild {
                    allocation_id: allocation,
                    source_order: member.source_order,
                    depth,
                    status: failure.status,
                    artifact,
                    parsed: None,
                    archive_metadata: archive_metadata.clone(),
                    diagnostics: vec![diagnostic],
                    children: Vec::new(),
                },
                audit,
            ));
        }

        if let Some(rejection) = security_rejection {
            let bytes = available_bytes(&member.body).unwrap_or_default();
            let extraction =
                ArtifactExtraction::new(ArtifactExtractionStatus::Rejected, rejection.code)
                    .with_message(rejection.message.clone())
                    .with_diagnostic(rejection.code);
            let artifact = EmbeddedArtifact::record_known_unavailable(metadata, bytes, extraction)
                .map_err(artifact_failure)?;
            let mut diagnostic = Diagnostic::security_rejection(
                "grist.container",
                rejection.message,
            )
            .with_locator(locator)
            .with_affected_ids(vec![artifact.identity.artifact_id.clone()])
            .with_recovery(crate::core::RecoveryAction::new(
                crate::core::RecoveryKind::InspectInput,
                "inspect the rejected archive member and rebuild the archive with safe unique regular-file paths",
                false,
            ))
            .partial();
            diagnostic.code = rejection.code.into();
            audit.usage_after = self.control.usage();
            return Ok((
                ContainerChild {
                    allocation_id: allocation,
                    source_order: member.source_order,
                    depth,
                    status: ContainerChildStatus::Rejected,
                    artifact,
                    parsed: None,
                    archive_metadata: archive_metadata.clone(),
                    diagnostics: vec![diagnostic],
                    children: Vec::new(),
                },
                audit,
            ));
        }

        let result = match member.body {
            ContainerMemberBody::Unavailable(unavailable) => self.unavailable_child(
                allocation.clone(),
                depth,
                member.source_order,
                metadata,
                unavailable,
            )?,
            ContainerMemberBody::Available(bytes) => self.available_child(
                allocation.clone(),
                depth,
                member.source_order,
                parent_source,
                locator,
                metadata,
                member.format_hint,
                member.nested_container_format,
                bytes,
            )?,
        };
        let mut child = result.0;
        child.archive_metadata = archive_metadata;
        audit.children = result.1;
        audit.limit_hit = child.diagnostics.iter().find_map(diagnostic_budget_axis);
        audit.usage_after = self.control.usage();
        Ok((child, audit))
    }

    fn charge_member(
        &mut self,
        parent_class: ContainerClass,
        depth: u64,
        body: &ContainerMemberBody,
        charge_expansion: bool,
    ) -> Result<(), ContainerDecodeFailure> {
        self.control
            .checkpoint()
            .map_err(ContainerDecodeFailure::controlled)?;
        self.control
            .budget()
            .observe_nesting_depth(depth)
            .map_err(ContainerDecodeFailure::budget)?;
        if parent_class.charges_archive_members() {
            self.control
                .budget()
                .consume_archive_members(1)
                .map_err(ContainerDecodeFailure::budget)?;
        }
        self.control
            .budget()
            .consume_child_artifacts(1)
            .map_err(ContainerDecodeFailure::budget)?;
        if charge_expansion && let Some(bytes) = available_bytes(body) {
            let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            self.expanded_bytes = self.expanded_bytes.saturating_add(length);
            self.control
                .budget()
                .observe_archive_expansion(self.root_compressed_bytes, self.expanded_bytes)
                .map_err(ContainerDecodeFailure::budget)?;
            self.control
                .budget()
                .observe_memory_bytes(length)
                .map_err(ContainerDecodeFailure::budget)?;
        }
        Ok(())
    }

    fn unavailable_child(
        &self,
        allocation_id: String,
        depth: u64,
        source_order: u64,
        metadata: ArtifactMetadata,
        unavailable: ContainerUnavailable,
    ) -> Result<(ContainerChild, Vec<BudgetAllocationNode>), ContainerDecodeFailure> {
        if unavailable.code.trim().is_empty() {
            return Err(ContainerDecodeFailure::malformed(
                "grist.container",
                "container member terminal status code is empty",
            ));
        }
        let (status, extraction_status) = unavailable_status(unavailable.kind);
        let mut extraction = ArtifactExtraction::new(extraction_status, unavailable.code.clone());
        if let Some(message) = unavailable.message.clone() {
            extraction = extraction.with_message(message);
        }
        let artifact =
            EmbeddedArtifact::record_unavailable(metadata, extraction).map_err(artifact_failure)?;
        let diagnostic = unavailable_diagnostic(&unavailable, &artifact);
        Ok((
            ContainerChild {
                allocation_id,
                source_order,
                depth,
                status,
                artifact,
                parsed: None,
                archive_metadata: None,
                diagnostics: vec![diagnostic],
                children: Vec::new(),
            },
            Vec::new(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn available_child(
        &mut self,
        allocation_id: String,
        depth: u64,
        source_order: u64,
        parent_source: &SourceInfo,
        locator: SourceLocator,
        metadata: ArtifactMetadata,
        format_hint: Option<FormatHint>,
        nested_container_format: Option<String>,
        bytes: Vec<u8>,
    ) -> Result<(ContainerChild, Vec<BudgetAllocationNode>), ContainerDecodeFailure> {
        let unavailable_metadata = metadata.clone();
        let captured = match self.artifact_mode {
            ContainerArtifactMode::InventoryOnly => EmbeddedArtifact::inventory(metadata, &bytes),
            ContainerArtifactMode::InlinePayload => {
                EmbeddedArtifact::capture_inline(metadata, &bytes)
            }
            ContainerArtifactMode::ContentAddressed => EmbeddedArtifact::capture_content_addressed(
                metadata,
                &bytes,
                self.sink.expect("content-addressed mode validated a sink"),
            ),
        };
        let artifact = match captured {
            Ok(artifact) => artifact,
            Err(error) => {
                let message = error.to_string();
                let extraction = ArtifactExtraction::new(
                    ArtifactExtractionStatus::Failed,
                    "container.child.artifact_capture_failed",
                )
                .with_message(message.clone())
                .with_diagnostic("container.child.artifact_capture_failed");
                let artifact = EmbeddedArtifact::record_known_unavailable(
                    unavailable_metadata,
                    &bytes,
                    extraction,
                )
                .map_err(artifact_failure)?;
                let mut diagnostic = Diagnostic::error(
                    "grist.container",
                    "container.child.artifact_capture_failed",
                    message,
                );
                diagnostic.class = DiagnosticClass::ParserDefect;
                return Ok((
                    ContainerChild {
                        allocation_id,
                        source_order,
                        depth,
                        status: ContainerChildStatus::Failed,
                        diagnostics: vec![attributed(diagnostic, &artifact)],
                        artifact,
                        archive_metadata: None,
                        parsed: None,
                        children: Vec::new(),
                    },
                    Vec::new(),
                ));
            }
        };
        let child_source = child_source(parent_source, &artifact, source_order);
        let mut status = if self.artifact_mode == ContainerArtifactMode::InventoryOnly {
            ContainerChildStatus::InventoryOnly
        } else {
            ContainerChildStatus::Skipped
        };
        let mut parsed = None;
        let mut diagnostics = Vec::new();
        let mut children = Vec::new();
        let mut allocations = Vec::new();

        if let Some(format) = nested_container_format {
            match self.decode_container(
                &format,
                &bytes,
                &child_source,
                &artifact.identity.content,
                Some(&locator),
                depth,
                &allocation_id,
            ) {
                Ok((nested, nested_allocations, nested_diagnostics, failure)) => {
                    children = nested;
                    allocations = nested_allocations;
                    diagnostics.extend(nested_diagnostics);
                    status = failure.unwrap_or(ContainerChildStatus::Parsed);
                }
                Err(failure) => {
                    status = failure.status;
                    diagnostics.push(attributed(*failure.diagnostic, &artifact));
                }
            }
        } else if self.artifact_mode != ContainerArtifactMode::InventoryOnly
            && self.parse_leaf_payloads
        {
            let member_name = artifact
                .declared_filename
                .clone()
                .unwrap_or_else(|| format!("member-{source_order}"));
            let mut request = ParseRequest::<AutoFormatOptions>::new(
                RequestId::new(format!("container:{}", &allocation_id[11..]))
                    .expect("allocation-derived request IDs are valid"),
                Input::compound_member(CompoundMemberInput::new(
                    parent_source.clone(),
                    member_name,
                    Some(source_order),
                    Input::bytes(bytes),
                )),
                child_source,
                self.budget_selection.clone(),
                self.providers.clone(),
            )
            .with_parse_options(self.parse_options.clone())
            .with_cancellation(self.cancellation.clone());
            request.format_hint = format_hint;
            match self
                .recursor
                .ingestor
                .ingest_with_control(request, self.control.clone())
            {
                Ok(envelope) => {
                    status = envelope_status(envelope.status);
                    diagnostics.extend(envelope.diagnostics.clone());
                    parsed = Some(envelope);
                }
                Err(error) => {
                    status = ContainerChildStatus::Failed;
                    diagnostics.push(attributed(ingest_diagnostic(error), &artifact));
                }
            }
        } else if self.artifact_mode != ContainerArtifactMode::InventoryOnly {
            diagnostics.push(attributed(
                Diagnostic::unsupported(
                    "grist.container",
                    "leaf parsing was disabled by the explicit container options",
                ),
                &artifact,
            ));
        }

        Ok((
            ContainerChild {
                allocation_id,
                source_order,
                depth,
                status,
                artifact,
                parsed,
                archive_metadata: None,
                diagnostics,
                children,
            },
            allocations,
        ))
    }
}

fn member_metadata(
    parent_identity: &ContentIdentity,
    locator: &SourceLocator,
    member: &ContainerMember,
) -> ArtifactMetadata {
    let mut metadata = ArtifactMetadata::new(
        ArtifactParent::new(parent_identity.clone(), member.relationship.clone()),
        locator.clone(),
        member.disposition.clone(),
    );
    if let Some(filename) = &member.declared_filename {
        metadata = metadata.with_declared_filename(filename);
    }
    if let Some(media_type) = &member.media_type {
        metadata = metadata.with_media_type(media_type);
    }
    metadata
}

fn archive_rejections(
    class: ContainerClass,
    members: &[ContainerMember],
    policy: &ArchiveSecurityPolicy,
) -> Vec<Option<ArchiveRejection>> {
    if !matches!(class, ContainerClass::Archive | ContainerClass::Package) {
        return vec![None; members.len()];
    }
    let inspected = members
        .iter()
        .map(|member| {
            let path = member
                .relative_locator
                .archive_member_path()
                .or(member.declared_filename.as_deref())
                .ok_or_else(|| ArchiveRejection {
                    code: "grist.security.archive.missing_path",
                    message: "archive member has no parent-relative archive path".into(),
                    normalized_path: None,
                })?;
            policy.validate_member(&ArchiveMemberDescriptor {
                path,
                kind: member.entry_kind,
                link_target: member.link_target.as_deref(),
            })
        })
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::<String, usize>::new();
    for path in inspected.iter().filter_map(|result| result.as_ref().ok()) {
        *counts.entry(path.clone()).or_default() += 1;
    }
    inspected
        .into_iter()
        .map(|result| match result {
            Err(rejection) => Some(rejection),
            Ok(path) if counts[&path] > 1 => Some(policy.duplicate_rejection(path)),
            Ok(_) => None,
        })
        .collect()
}

fn member_sort_key(member: &ContainerMember) -> (u64, String, String, String) {
    let locator = serde_json::to_string(&member.relative_locator).unwrap_or_default();
    let filename = member.declared_filename.clone().unwrap_or_default();
    let identity = match &member.body {
        ContainerMemberBody::Available(bytes) => crate::core::sha256_hex(bytes),
        ContainerMemberBody::Unavailable(unavailable) => unavailable.code.clone(),
    };
    (member.source_order, locator, filename, identity)
}

fn available_bytes(body: &ContainerMemberBody) -> Option<&[u8]> {
    match body {
        ContainerMemberBody::Available(bytes) => Some(bytes),
        ContainerMemberBody::Unavailable(_) => None,
    }
}

fn unavailable_status(
    kind: ContainerUnavailableKind,
) -> (ContainerChildStatus, ArtifactExtractionStatus) {
    match kind {
        ContainerUnavailableKind::Skipped => (
            ContainerChildStatus::Skipped,
            ArtifactExtractionStatus::Skipped,
        ),
        ContainerUnavailableKind::Encrypted => (
            ContainerChildStatus::Encrypted,
            ArtifactExtractionStatus::Encrypted,
        ),
        ContainerUnavailableKind::Unsupported => (
            ContainerChildStatus::Unsupported,
            ArtifactExtractionStatus::Unsupported,
        ),
        ContainerUnavailableKind::Rejected => (
            ContainerChildStatus::Rejected,
            ArtifactExtractionStatus::Rejected,
        ),
        ContainerUnavailableKind::Failed => (
            ContainerChildStatus::Failed,
            ArtifactExtractionStatus::Failed,
        ),
    }
}

fn unavailable_diagnostic(
    unavailable: &ContainerUnavailable,
    artifact: &EmbeddedArtifact,
) -> Diagnostic {
    let message = unavailable
        .message
        .clone()
        .unwrap_or_else(|| unavailable.code.clone());
    let mut diagnostic = match unavailable.kind {
        ContainerUnavailableKind::Unsupported => {
            Diagnostic::unsupported("grist.container", message)
        }
        ContainerUnavailableKind::Rejected => {
            Diagnostic::security_rejection("grist.container", message)
        }
        ContainerUnavailableKind::Encrypted
        | ContainerUnavailableKind::Skipped
        | ContainerUnavailableKind::Failed => {
            Diagnostic::error("grist.container", unavailable.code.clone(), message)
        }
    };
    diagnostic.code = unavailable.code.clone().into();
    attributed(diagnostic, artifact)
}

fn child_source(parent: &SourceInfo, artifact: &EmbeddedArtifact, order: u64) -> SourceInfo {
    let mut source = SourceInfo::new(
        artifact
            .declared_filename
            .clone()
            .unwrap_or_else(|| format!("member-{order}")),
    )
    .with_parent(parent.clone());
    if let Some(media_type) = &artifact.media_type {
        source = source.with_declared_mime_type(media_type);
    }
    source
}

fn attributed(mut diagnostic: Diagnostic, artifact: &EmbeddedArtifact) -> Diagnostic {
    diagnostic.locator = Some(Box::new(artifact.locator.clone()));
    diagnostic.source_identity = Some(Box::new(artifact.identity.content.clone()));
    diagnostic
        .affected_ids
        .push(artifact.identity.artifact_id.clone());
    diagnostic.affected_ids.sort();
    diagnostic.affected_ids.dedup();
    diagnostic.partial = true;
    diagnostic
}

fn artifact_failure(error: super::EmbeddedArtifactError) -> ContainerDecodeFailure {
    ContainerDecodeFailure::new(
        ContainerChildStatus::Failed,
        Diagnostic::parser_defect("grist.container", error.to_string()),
    )
}

fn ingest_diagnostic(error: IngestError) -> Diagnostic {
    let mut diagnostic = Diagnostic::parser_defect("grist.container", error.to_string());
    diagnostic.code = "container.child.ingest_failed".into();
    diagnostic
}

fn envelope_status(status: OperationStatus) -> ContainerChildStatus {
    match status {
        OperationStatus::Complete | OperationStatus::Partial => ContainerChildStatus::Parsed,
        OperationStatus::Unsupported | OperationStatus::Ambiguous => {
            ContainerChildStatus::Unsupported
        }
        OperationStatus::Encrypted => ContainerChildStatus::Encrypted,
        OperationStatus::Cancelled => ContainerChildStatus::Cancelled,
        OperationStatus::Failed => ContainerChildStatus::Failed,
    }
}

fn traversal_status(
    children: &[ContainerChild],
    root_failure: Option<ContainerChildStatus>,
) -> OperationStatus {
    if root_failure == Some(ContainerChildStatus::Cancelled)
        || children
            .iter()
            .any(|child| child.status == ContainerChildStatus::Cancelled)
    {
        return OperationStatus::Cancelled;
    }
    if root_failure.is_some() && children.is_empty() {
        return match root_failure {
            Some(ContainerChildStatus::Unsupported) => OperationStatus::Unsupported,
            Some(ContainerChildStatus::Encrypted) => OperationStatus::Encrypted,
            _ => OperationStatus::Failed,
        };
    }
    if children.iter().any(child_has_loss) {
        OperationStatus::Partial
    } else {
        OperationStatus::Complete
    }
}

fn child_has_loss(child: &ContainerChild) -> bool {
    child.status.is_loss() || child.children.iter().any(child_has_loss)
}

#[derive(Serialize)]
struct AllocationIdMaterial<'a> {
    schema_version: &'static str,
    parent_allocation_id: Option<&'a str>,
    locator: Option<&'a SourceLocator>,
    source_order: u64,
    identity: &'a ContentIdentity,
}

fn allocation_id(
    parent_allocation_id: Option<&str>,
    locator: Option<&SourceLocator>,
    source_order: u64,
    identity: &ContentIdentity,
) -> Result<String, serde_json::Error> {
    Ok(format!(
        "allocation:{}",
        canonical_json_sha256(&AllocationIdMaterial {
            schema_version: SchemaVersion::CONTAINER_TRAVERSAL_V1,
            parent_allocation_id,
            locator,
            source_order,
            identity,
        })?
    ))
}

fn remaining_budget(limits: &ResourceBudget, usage: &BudgetUsage) -> ResourceBudget {
    let remaining =
        |limit: Option<u64>, consumed: u64| limit.map(|value| value.saturating_sub(consumed));
    ResourceBudget {
        max_input_bytes: remaining(limits.max_input_bytes, usage.input_bytes),
        max_decoded_characters: remaining(limits.max_decoded_characters, usage.decoded_characters),
        max_pages: remaining(limits.max_pages, usage.pages),
        max_records: remaining(limits.max_records, usage.records),
        max_cells: remaining(limits.max_cells, usage.cells),
        max_nodes: remaining(limits.max_nodes, usage.nodes),
        max_nesting_depth: remaining(limits.max_nesting_depth, usage.nesting_depth),
        max_archive_expansion_ratio: limits.max_archive_expansion_ratio,
        max_archive_members: remaining(limits.max_archive_members, usage.archive_members),
        max_child_artifacts: remaining(limits.max_child_artifacts, usage.child_artifacts),
        max_parse_millis: remaining(limits.max_parse_millis, usage.parse_millis),
        max_provider_millis: remaining(limits.max_provider_millis, usage.provider_millis),
        max_memory_bytes: remaining(limits.max_memory_bytes, usage.memory_bytes),
        max_temporary_storage_bytes: remaining(
            limits.max_temporary_storage_bytes,
            usage.temporary_storage_bytes,
        ),
        max_output_bytes: remaining(limits.max_output_bytes, usage.output_bytes),
    }
}

fn diagnostic_budget_axis(diagnostic: &Diagnostic) -> Option<BudgetAxis> {
    if diagnostic.class != DiagnosticClass::ResourceBudgetExhaustion {
        return None;
    }
    BudgetAxis::ALL
        .into_iter()
        .find(|axis| diagnostic.code.as_str() == format!("grist.budget.{}.exhausted", axis.name()))
}

#[derive(Debug, thiserror::Error)]
pub enum ContainerTraversalError {
    #[error("container format ID cannot be empty")]
    InvalidContainerFormat,
    #[error("content-addressed mode requires an artifact sink")]
    ContentAddressedSinkRequired,
    #[error(transparent)]
    InvalidBudget(#[from] crate::core::ResourceBudgetValidationError),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}
