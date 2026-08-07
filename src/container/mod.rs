//! Archive, compound-document, and embedded-artifact ownership boundary.
//!
//! Container traversal is separate from [`crate::ingest`] so nested inputs can
//! share one resource and provenance policy without coupling formats to I/O.

mod artifact;
mod materialize;
mod recursion;

pub use crate::security::ArchiveEntryKind;

pub use artifact::{
    ArtifactCaptureOptions, ArtifactContent, ArtifactDisposition, ArtifactExtraction,
    ArtifactExtractionStatus, ArtifactIdentity, ArtifactInlineBytes, ArtifactMetadata,
    ArtifactParent, ArtifactRelationship, ArtifactSafety, ArtifactSafetyClassification,
    ArtifactSafetyEvidence, ArtifactStoreError, ContentAddressedArtifactReference,
    ContentAddressedArtifactResolver, ContentAddressedArtifactSink, EmbeddedArtifact,
    EmbeddedArtifactError,
};
pub use materialize::{
    ArtifactMaterializationError, ArtifactMaterializationOutcome, MaterializationRequest,
};
pub use recursion::{
    BudgetAllocationNode, ContainerArtifactMode, ContainerBudgetTree, ContainerChild,
    ContainerChildStatus, ContainerClass, ContainerDecodeContext, ContainerDecodeFailure,
    ContainerDecoder, ContainerDecoderRegistry, ContainerMember, ContainerMemberBody,
    ContainerParseOptions, ContainerParseRequest, ContainerRecursor, ContainerRegistryError,
    ContainerTraversal, ContainerTraversalError, ContainerUnavailable, ContainerUnavailableKind,
    ParentRelativeLocator,
};
