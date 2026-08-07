//! Caller-supplied source labels and containment identity.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Descriptive source metadata supplied by the caller.
///
/// URI values are labels only. Resolving or fetching them is outside Grist's
/// source contract.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceInfo {
    pub path: Option<String>,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_relative_path: Option<String>,
    /// Caller-supplied timestamp. Grist never inserts the current time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingestion_timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Box<SourceInfo>>,
}

impl SourceInfo {
    pub fn from_path(path: &Path) -> Self {
        Self::new(
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string()),
        )
        .with_path(path)
    }

    pub fn stdin(display_name: impl Into<String>) -> Self {
        Self::new(display_name)
    }

    pub fn from_uri(uri: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self::new(display_name).with_uri(uri)
    }

    pub fn new(display_name: impl Into<String>) -> Self {
        Self {
            path: None,
            display_name: display_name.into(),
            declared_mime_type: None,
            uri: None,
            repository_relative_path: None,
            ingestion_timestamp: None,
            parent: None,
        }
    }

    pub fn with_path(mut self, path: impl AsRef<Path>) -> Self {
        self.path = Some(path.as_ref().to_string_lossy().to_string());
        self
    }

    pub fn with_declared_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.declared_mime_type = Some(mime_type.into());
        self
    }

    pub fn with_uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
        self
    }

    pub fn with_repository_relative_path(mut self, path: impl Into<String>) -> Self {
        self.repository_relative_path = Some(path.into());
        self
    }

    pub fn with_ingestion_timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.ingestion_timestamp = Some(timestamp.into());
        self
    }

    pub fn with_parent(mut self, parent: SourceInfo) -> Self {
        self.parent = Some(Box::new(parent));
        self
    }
}
