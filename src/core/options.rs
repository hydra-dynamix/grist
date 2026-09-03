//! Shared parse behavior and typed format options.

use super::DiagnosticOptions;
use crate::security::SecurityPolicy;
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Recovery behavior shared by every parser.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    Strict,
    #[default]
    Recover,
}

/// Options whose meaning is identical for every parser.
///
/// Format-specific configuration is carried separately by the generic option
/// parameter on [`super::ParseRequest`].
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParseOptions {
    pub recovery: RecoveryMode,
    pub diagnostics: DiagnosticOptions,
    /// Universal hostile-input policy. Format options cannot weaken its
    /// no-execution and no-implicit-network invariants.
    pub security: SecurityPolicy,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            recovery: RecoveryMode::Recover,
            diagnostics: DiagnosticOptions::default(),
            security: SecurityPolicy::default(),
        }
    }
}

/// Marker implemented by each parser's statically typed option structure.
pub trait FormatOptions: Send + Sync + 'static {
    const FORMAT: &'static str;
}

/// Typed option value used while automatic format selection is requested.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoFormatOptions;

impl FormatOptions for AutoFormatOptions {
    const FORMAT: &'static str = "auto";
}
