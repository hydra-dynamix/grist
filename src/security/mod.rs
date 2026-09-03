//! Universal hostile-input policy and enforcement primitives.

mod archive;
mod policy;
mod render;
mod secret;
mod temporary;
mod xml;

pub use archive::{
    ArchiveEntryKind, ArchiveMemberDescriptor, ArchiveRejection, ArchiveSecurityPolicy,
};
pub use policy::{
    ActiveContentPolicy, ExecutionPolicy, InputMetadataPolicy, NetworkPolicy, SecurityPolicy,
};
pub use render::{
    escape_active_html, escape_latex_text, inert_latex_literal, inert_markdown_code,
    sanitize_link_destination,
};
pub use secret::SecretRedactor;
pub use temporary::{PrivateTemporaryStorage, TemporaryRetentionPolicy, TemporaryStorageError};
pub use xml::{XmlSecurityFinding, XmlSecurityFindingKind, XmlSecurityPolicy, inspect_xml};
