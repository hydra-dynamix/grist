//! Universal typed parse request and batch correlation identity.

use super::{
    AutoFormatOptions, BudgetSelection, CancellationToken, FormatOptions, Input, InputError,
    OperationControl, ParseOptions, ProviderSet, ResolvedInput, SourceInfo,
};
use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Opaque caller-supplied identifier returned with batch results.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct RequestId(String);

impl RequestId {
    pub const MAX_BYTES: usize = 256;

    pub fn new(value: impl Into<String>) -> Result<Self, RequestIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(RequestIdError::Empty);
        }
        if value.len() > Self::MAX_BYTES {
            return Err(RequestIdError::TooLong {
                actual: value.len(),
                maximum: Self::MAX_BYTES,
            });
        }
        if value.chars().any(char::is_control) {
            return Err(RequestIdError::ContainsControlCharacter);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RequestIdError {
    #[error("request ID must not be empty")]
    Empty,
    #[error("request ID is {actual} bytes; maximum is {maximum}")]
    TooLong { actual: usize, maximum: usize },
    #[error("request ID must not contain control characters")]
    ContainsControlCharacter,
}

/// Caller evidence used to select or verify a parser.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FormatHint {
    pub format: Option<String>,
    pub media_type: Option<String>,
    pub filename: Option<String>,
}

impl FormatHint {
    pub fn exact(format: impl Into<String>) -> Self {
        Self {
            format: Some(format.into()),
            media_type: None,
            filename: None,
        }
    }
}

/// The common runtime boundary for every parser.
///
/// `O` retains each parser's concrete option type. Input readers, providers,
/// and format options may contain secrets, so the request is intentionally not
/// serializable.
pub struct ParseRequest<O: FormatOptions = AutoFormatOptions> {
    pub request_id: RequestId,
    pub input: Input,
    pub source: SourceInfo,
    pub format_hint: Option<FormatHint>,
    pub options: ParseOptions,
    pub format_options: O,
    pub budget: BudgetSelection,
    pub cancellation: CancellationToken,
    pub providers: ProviderSet,
}

impl ParseRequest<AutoFormatOptions> {
    pub fn new(
        request_id: RequestId,
        input: Input,
        source: SourceInfo,
        budget: BudgetSelection,
        providers: ProviderSet,
    ) -> Self {
        Self {
            request_id,
            input,
            source,
            format_hint: None,
            options: ParseOptions::default(),
            format_options: AutoFormatOptions,
            budget,
            cancellation: CancellationToken::new(),
            providers,
        }
    }
}

impl<O: FormatOptions> ParseRequest<O> {
    pub fn with_format_hint(mut self, format_hint: FormatHint) -> Self {
        self.format_hint = Some(format_hint);
        self
    }

    pub fn with_parse_options(mut self, options: ParseOptions) -> Self {
        self.options = options;
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn with_format_options<T: FormatOptions>(self, format_options: T) -> ParseRequest<T> {
        ParseRequest {
            request_id: self.request_id,
            input: self.input,
            source: self.source,
            format_hint: self
                .format_hint
                .or_else(|| Some(FormatHint::exact(T::FORMAT))),
            options: self.options,
            format_options,
            budget: self.budget,
            cancellation: self.cancellation,
            providers: self.providers,
        }
    }

    pub fn resolve_input(self) -> Result<ResolvedParseRequest<O>, InputError> {
        let control = OperationControl::new(&self.budget, self.cancellation.clone())?;
        let input = self.input.resolve_with_control(&control)?;
        Ok(ResolvedParseRequest {
            request_id: self.request_id,
            input,
            source: self.source,
            format_hint: self.format_hint,
            options: self.options,
            format_options: self.format_options,
            budget: self.budget,
            control,
            providers: self.providers,
        })
    }
}

impl<O: FormatOptions> fmt::Debug for ParseRequest<O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParseRequest")
            .field("request_id", &self.request_id)
            .field("input", &self.input)
            .field("source", &self.source)
            .field("format_hint", &self.format_hint)
            .field("options", &self.options)
            .field("format_options_type", &std::any::type_name::<O>())
            .field("budget", &self.budget)
            .field("cancellation", &self.cancellation)
            .field("providers", &self.providers)
            .finish()
    }
}

pub struct ResolvedParseRequest<O: FormatOptions = AutoFormatOptions> {
    pub request_id: RequestId,
    pub input: ResolvedInput,
    pub source: SourceInfo,
    pub format_hint: Option<FormatHint>,
    pub options: ParseOptions,
    pub format_options: O,
    pub budget: BudgetSelection,
    pub control: OperationControl,
    pub providers: ProviderSet,
}
