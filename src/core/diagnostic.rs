//! Stable diagnostics, structured causal chains, and secret-safe details.

use super::{ContentIdentity, SourceLocator, SourceRange};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Diagnostic importance at the public API boundary.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Fatal,
}

/// Stable top-level condition families shared by every parser and operation.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticClass {
    MalformedInput,
    UnsupportedContent,
    LossyNormalization,
    ProviderFailure,
    ParserDefect,
    SecurityRejection,
    ResourceBudgetExhaustion,
    #[default]
    Unclassified,
}

impl DiagnosticClass {
    pub const fn code(self) -> &'static str {
        match self {
            Self::MalformedInput => "grist.input.malformed",
            Self::UnsupportedContent => "grist.content.unsupported",
            Self::LossyNormalization => "grist.normalization.lossy",
            Self::ProviderFailure => "grist.provider.failed",
            Self::ParserDefect => "grist.parser.defect",
            Self::SecurityRejection => "grist.security.rejected",
            Self::ResourceBudgetExhaustion => "grist.budget.exhausted",
            Self::Unclassified => "grist.diagnostic.unclassified",
        }
    }

    const fn default_severity(self) -> Severity {
        match self {
            Self::LossyNormalization => Severity::Warning,
            Self::ParserDefect | Self::SecurityRejection => Severity::Fatal,
            Self::MalformedInput
            | Self::UnsupportedContent
            | Self::ProviderFailure
            | Self::ResourceBudgetExhaustion
            | Self::Unclassified => Severity::Error,
        }
    }
}

/// Open, forward-compatible diagnostic code with stable built-in values.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct DiagnosticCode(String);

impl DiagnosticCode {
    pub fn new(code: impl Into<String>) -> Self {
        Self(code.into())
    }

    pub fn for_class(class: DiagnosticClass) -> Self {
        Self(class.code().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for DiagnosticCode {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for DiagnosticCode {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq<str> for DiagnosticCode {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for DiagnosticCode {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// Structured details whose field names cannot carry common secret material.
///
/// Credentials should use runtime-only secret types. This boundary also rejects
/// common credential field names recursively, including during deserialization.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Clone, Serialize, PartialEq)]
#[serde(transparent)]
pub struct DiagnosticDetails(Value);

impl DiagnosticDetails {
    pub fn new(values: BTreeMap<String, Value>) -> Result<Self, DiagnosticDetailsError> {
        validate_detail_map(&values, "details")?;
        Ok(Self(Value::Object(values.into_iter().collect())))
    }

    pub fn from_value(value: Value) -> Result<Self, DiagnosticDetailsError> {
        let Value::Object(values) = value else {
            return Err(DiagnosticDetailsError::NotAnObject);
        };
        Self::new(values.into_iter().collect())
    }

    pub fn as_value(&self) -> &Value {
        &self.0
    }
}

impl fmt::Debug for DiagnosticDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for DiagnosticDetails {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        validate_detail_value(&value, "details").map_err(serde::de::Error::custom)?;
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DiagnosticDetailsError {
    #[error("diagnostic details must be a JSON object")]
    NotAnObject,
    #[error("sensitive diagnostic detail field is forbidden at {path}")]
    SensitiveField { path: String },
    #[error("authorization-like diagnostic detail value is forbidden at {path}")]
    AuthorizationValue { path: String },
}

fn validate_detail_map(
    values: &BTreeMap<String, Value>,
    path: &str,
) -> Result<(), DiagnosticDetailsError> {
    for (key, value) in values {
        let child_path = format!("{path}.{key}");
        if is_sensitive_key(key) {
            return Err(DiagnosticDetailsError::SensitiveField { path: child_path });
        }
        validate_detail_value(value, &child_path)?;
    }
    Ok(())
}

fn validate_detail_value(value: &Value, path: &str) -> Result<(), DiagnosticDetailsError> {
    match value {
        Value::Object(values) => {
            let values = values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            validate_detail_map(&values, path)
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_detail_value(value, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        Value::String(value)
            if ["bearer ", "basic "]
                .iter()
                .any(|prefix| value.trim_start().to_ascii_lowercase().starts_with(prefix)) =>
        {
            Err(DiagnosticDetailsError::AuthorizationValue { path: path.into() })
        }
        _ => Ok(()),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "password"
            | "passwd"
            | "secret"
            | "clientsecret"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "apikey"
            | "authorization"
            | "credential"
            | "credentials"
            | "privatekey"
            | "accesskey"
            | "cookie"
            | "sessioncookie"
    )
}

/// One nested reason beneath a diagnostic's root-cause message.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticCause {
    pub code: DiagnosticCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<DiagnosticDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<Box<DiagnosticCause>>,
}

impl DiagnosticCause {
    pub fn new(code: impl Into<DiagnosticCode>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            module: None,
            parser: None,
            details: None,
            cause: None,
        }
    }

    pub fn with_emitter(mut self, module: impl Into<String>, parser: impl Into<String>) -> Self {
        self.module = Some(module.into());
        self.parser = Some(parser.into());
        self
    }

    pub fn with_details(mut self, details: DiagnosticDetails) -> Self {
        self.details = Some(details);
        self
    }

    pub fn caused_by(mut self, cause: DiagnosticCause) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }
}

/// Machine-actionable recovery families.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryKind {
    InspectInput,
    SelectParser,
    EnableFeature,
    RetryProvider,
    IncreaseBudget,
    SupplyCredentials,
    ChangeOptions,
    ReportParserDefect,
}

/// A recovery action is guidance, never an action executed by Grist.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryAction {
    pub kind: RecoveryKind,
    pub description: String,
    pub retryable: bool,
}

impl RecoveryAction {
    pub fn new(kind: RecoveryKind, description: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind,
            description: description.into(),
            retryable,
        }
    }
}

/// A root-cause diagnostic with source attribution and structured causes.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
    pub parser: String,
    #[serde(default)]
    pub module: String,
    #[serde(default)]
    pub class: DiagnosticClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_identity: Option<Box<ContentIdentity>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<SourceRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<Box<SourceLocator>>,
    #[serde(default)]
    pub partial: bool,
    /// Legacy flat causes retained for v1 wire compatibility.
    #[serde(default)]
    pub cause: Vec<String>,
    #[serde(default)]
    pub causes: Vec<DiagnosticCause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<DiagnosticDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryAction>,
    #[serde(default)]
    pub affected_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation_key: Option<String>,
}

impl Diagnostic {
    pub fn malformed(parser: impl Into<String>, message: impl Into<String>) -> Self {
        Self::condition(DiagnosticClass::MalformedInput, parser, message)
    }

    pub fn unsupported(parser: impl Into<String>, message: impl Into<String>) -> Self {
        Self::condition(DiagnosticClass::UnsupportedContent, parser, message)
    }

    pub fn lossy(parser: impl Into<String>, message: impl Into<String>) -> Self {
        Self::condition(DiagnosticClass::LossyNormalization, parser, message).partial()
    }

    pub fn provider_failure(parser: impl Into<String>, message: impl Into<String>) -> Self {
        Self::condition(DiagnosticClass::ProviderFailure, parser, message)
    }

    pub fn parser_defect(parser: impl Into<String>, message: impl Into<String>) -> Self {
        Self::condition(DiagnosticClass::ParserDefect, parser, message)
    }

    pub fn security_rejection(parser: impl Into<String>, message: impl Into<String>) -> Self {
        Self::condition(DiagnosticClass::SecurityRejection, parser, message)
    }

    pub fn budget_exhausted(parser: impl Into<String>, message: impl Into<String>) -> Self {
        Self::condition(DiagnosticClass::ResourceBudgetExhaustion, parser, message).partial()
    }

    pub fn condition(
        class: DiagnosticClass,
        parser: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let parser = parser.into();
        Self {
            severity: class.default_severity(),
            code: DiagnosticCode::for_class(class),
            message: message.into(),
            parser: parser.clone(),
            module: parser,
            class,
            source: None,
            source_identity: None,
            range: None,
            locator: None,
            partial: false,
            cause: Vec::new(),
            causes: Vec::new(),
            details: None,
            recovery: None,
            affected_ids: Vec::new(),
            documentation_uri: None,
            explanation_key: Some(format!("diagnostic.{}", class.code())),
        }
    }

    pub fn error(
        parser: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Severity::Error, parser, code, message)
    }

    pub fn warning(
        parser: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Severity::Warning, parser, code, message)
    }

    pub fn info(
        parser: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Severity::Info, parser, code, message)
    }

    pub fn new(
        severity: Severity,
        parser: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let parser = parser.into();
        Self {
            severity,
            code: DiagnosticCode::new(code),
            message: message.into(),
            parser: parser.clone(),
            module: parser,
            class: DiagnosticClass::Unclassified,
            source: None,
            source_identity: None,
            range: None,
            locator: None,
            partial: false,
            cause: Vec::new(),
            causes: Vec::new(),
            details: None,
            recovery: None,
            affected_ids: Vec::new(),
            documentation_uri: None,
            explanation_key: None,
        }
    }

    pub fn with_module(mut self, module: impl Into<String>) -> Self {
        self.module = module.into();
        self
    }

    pub fn with_parser(mut self, parser: impl Into<String>) -> Self {
        self.parser = parser.into();
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_source_identity(mut self, identity: ContentIdentity) -> Self {
        self.source_identity = Some(Box::new(identity));
        self
    }

    pub fn with_range(mut self, range: SourceRange) -> Self {
        self.range = Some(range);
        self
    }

    pub fn with_locator(mut self, locator: SourceLocator) -> Self {
        self.locator = Some(Box::new(locator));
        self
    }

    pub fn with_cause(mut self, cause: DiagnosticCause) -> Self {
        self.causes.push(cause);
        self
    }

    pub fn with_details(mut self, details: DiagnosticDetails) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_recovery(mut self, recovery: RecoveryAction) -> Self {
        self.recovery = Some(recovery);
        self
    }

    pub fn with_affected_ids(mut self, affected_ids: Vec<String>) -> Self {
        self.affected_ids = affected_ids;
        self
    }

    pub fn with_documentation_uri(mut self, uri: impl Into<String>) -> Self {
        self.documentation_uri = Some(uri.into());
        self
    }

    pub fn with_explanation_key(mut self, key: impl Into<String>) -> Self {
        self.explanation_key = Some(key.into());
        self
    }

    pub fn partial(mut self) -> Self {
        self.partial = true;
        self
    }
}
