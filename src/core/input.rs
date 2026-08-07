//! Runtime input variants and lossless resolution to original bytes.

use super::{
    AggregateMemberIdentity, BudgetAxis, BudgetExceeded, BudgetSelection, CancellationToken,
    ContentIdentity, OperationControl, OperationControlError, ResourceBudgetValidationError,
    SourceInfo,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub trait ReadSeek: Read + Seek {}

impl<T: Read + Seek> ReadSeek for T {}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Bytes,
    Utf8Text,
    SeekableReader,
    Stream,
    Path,
    CompoundMember,
}

impl fmt::Debug for Input {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Input").field(&self.kind()).finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputOrigin {
    Bytes,
    Utf8Text,
    SeekableReader,
    Stream,
    Path(PathBuf),
    CompoundMember {
        parent_source: SourceInfo,
        member_path: String,
        member_index: Option<u64>,
        backing: Box<InputOrigin>,
    },
}

/// Exact bytes obtained from a runtime input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInput {
    raw_bytes: Vec<u8>,
    declared_utf8: bool,
    origin: InputOrigin,
}

impl ResolvedInput {
    fn new(raw_bytes: Vec<u8>, declared_utf8: bool, origin: InputOrigin) -> Self {
        Self {
            raw_bytes,
            declared_utf8,
            origin,
        }
    }

    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw_bytes
    }

    /// Identity of the exact resolved bytes and caller-declared UTF-8 text.
    pub fn content_identity(&self) -> ContentIdentity {
        let identity = ContentIdentity::for_raw_bytes(&self.raw_bytes);
        match self.utf8_text() {
            Some(text) => identity.with_decoded(text, "utf-8", false),
            None => identity,
        }
    }

    /// Expose this resolved virtual member in a compound aggregate manifest.
    pub fn aggregate_member_identity(&self) -> Option<AggregateMemberIdentity> {
        let InputOrigin::CompoundMember {
            member_path,
            member_index,
            ..
        } = &self.origin
        else {
            return None;
        };
        Some(AggregateMemberIdentity::new(
            member_path.clone(),
            *member_index,
            &self.content_identity(),
        ))
    }

    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.raw_bytes
    }

    pub fn declared_utf8(&self) -> bool {
        self.declared_utf8
    }

    pub fn utf8_text(&self) -> Option<&str> {
        self.declared_utf8
            .then(|| std::str::from_utf8(&self.raw_bytes).expect("UTF-8 input invariant"))
    }

    pub fn origin(&self) -> &InputOrigin {
        &self.origin
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("input I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("input exceeded max_input_bytes {limit}; observed at least {observed_at_least} bytes")]
    ByteLimitExceeded { limit: u64, observed_at_least: u64 },
    #[error("operation cancelled by caller while resolving input")]
    Cancelled,
    #[error(transparent)]
    BudgetExceeded(BudgetExceeded),
    #[error(transparent)]
    InvalidBudget(#[from] ResourceBudgetValidationError),
}

impl From<OperationControlError> for InputError {
    fn from(error: OperationControlError) -> Self {
        match error {
            OperationControlError::Cancelled(_) => Self::Cancelled,
            OperationControlError::BudgetExceeded(error) => error.into(),
        }
    }
}

impl From<BudgetExceeded> for InputError {
    fn from(error: BudgetExceeded) -> Self {
        if error.axis != BudgetAxis::InputBytes {
            return Self::BudgetExceeded(error);
        }
        let limit = match error.limit {
            super::BudgetAmount::Count(value) => value,
            _ => 0,
        };
        let observed_at_least = match error.observed {
            super::BudgetAmount::Count(value) => value,
            _ => 0,
        };
        Self::ByteLimitExceeded {
            limit,
            observed_at_least,
        }
    }
}

fn read_controlled(
    mut reader: impl Read,
    control: &OperationControl,
) -> Result<Vec<u8>, InputError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        control.checkpoint()?;
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        control
            .budget()
            .consume_input_bytes(u64::try_from(read).unwrap_or(u64::MAX))?;
        control.budget().observe_memory_bytes(
            u64::try_from(bytes.len().saturating_add(read)).unwrap_or(u64::MAX),
        )?;
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(bytes)
}

/// An input member that exists inside a caller-supplied virtual container.
pub struct CompoundMemberInput {
    pub parent_source: SourceInfo,
    pub member_path: String,
    pub member_index: Option<u64>,
    pub input: Box<Input>,
}

impl CompoundMemberInput {
    pub fn new(
        parent_source: SourceInfo,
        member_path: impl Into<String>,
        member_index: Option<u64>,
        input: Input,
    ) -> Self {
        Self {
            parent_source,
            member_path: member_path.into(),
            member_index,
            input: Box::new(input),
        }
    }
}

impl fmt::Debug for CompoundMemberInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompoundMemberInput")
            .field("parent_source", &self.parent_source)
            .field("member_path", &self.member_path)
            .field("member_index", &self.member_index)
            .field("input_kind", &self.input.kind())
            .finish()
    }
}

/// A one-shot parse input.
///
/// This enum deliberately does not implement serde. Readers, streams, local
/// paths, and raw input contents are runtime values rather than request JSON.
pub enum Input {
    Bytes(Vec<u8>),
    Utf8Text(String),
    SeekableReader(Box<dyn ReadSeek + Send>),
    Stream(Box<dyn Read + Send>),
    Path(PathBuf),
    CompoundMember(CompoundMemberInput),
}

impl Input {
    pub fn bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(bytes.into())
    }

    pub fn utf8(text: impl Into<String>) -> Self {
        Self::Utf8Text(text.into())
    }

    pub fn seekable(reader: impl Read + Seek + Send + 'static) -> Self {
        Self::SeekableReader(Box::new(reader))
    }

    pub fn stream(reader: impl Read + Send + 'static) -> Self {
        Self::Stream(Box::new(reader))
    }

    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::Path(path.into())
    }

    pub fn compound_member(member: CompoundMemberInput) -> Self {
        Self::CompoundMember(member)
    }

    pub fn kind(&self) -> InputKind {
        match self {
            Self::Bytes(_) => InputKind::Bytes,
            Self::Utf8Text(_) => InputKind::Utf8Text,
            Self::SeekableReader(_) => InputKind::SeekableReader,
            Self::Stream(_) => InputKind::Stream,
            Self::Path(_) => InputKind::Path,
            Self::CompoundMember(_) => InputKind::CompoundMember,
        }
    }

    /// Consume the input and retain the exact original byte sequence.
    pub fn resolve(self, budget: &BudgetSelection) -> Result<ResolvedInput, InputError> {
        let control = OperationControl::new(budget, CancellationToken::new())?;
        self.resolve_with_control(&control)
    }

    pub fn resolve_with_control(
        self,
        control: &OperationControl,
    ) -> Result<ResolvedInput, InputError> {
        self.resolve_at_depth(control, 0)
    }

    fn resolve_at_depth(
        self,
        control: &OperationControl,
        nesting_depth: u64,
    ) -> Result<ResolvedInput, InputError> {
        control.checkpoint()?;
        control.budget().observe_nesting_depth(nesting_depth)?;
        match self {
            Self::Bytes(bytes) => {
                control
                    .budget()
                    .consume_input_bytes(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
                control
                    .budget()
                    .observe_memory_bytes(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
                Ok(ResolvedInput::new(bytes, false, InputOrigin::Bytes))
            }
            Self::Utf8Text(text) => {
                control
                    .budget()
                    .consume_input_bytes(u64::try_from(text.len()).unwrap_or(u64::MAX))?;
                control.budget().consume_decoded_characters(
                    u64::try_from(text.chars().count()).unwrap_or(u64::MAX),
                )?;
                control
                    .budget()
                    .observe_memory_bytes(u64::try_from(text.len()).unwrap_or(u64::MAX))?;
                Ok(ResolvedInput::new(
                    text.into_bytes(),
                    true,
                    InputOrigin::Utf8Text,
                ))
            }
            Self::SeekableReader(mut reader) => {
                reader.seek(SeekFrom::Start(0))?;
                let bytes = read_controlled(reader.as_mut(), control)?;
                Ok(ResolvedInput::new(
                    bytes,
                    false,
                    InputOrigin::SeekableReader,
                ))
            }
            Self::Stream(reader) => {
                let bytes = read_controlled(reader, control)?;
                Ok(ResolvedInput::new(bytes, false, InputOrigin::Stream))
            }
            Self::Path(path) => {
                let bytes = read_controlled(File::open(&path)?, control)?;
                Ok(ResolvedInput::new(bytes, false, InputOrigin::Path(path)))
            }
            Self::CompoundMember(member) => {
                let resolved = member
                    .input
                    .resolve_at_depth(control, nesting_depth.saturating_add(1))?;
                Ok(ResolvedInput {
                    raw_bytes: resolved.raw_bytes,
                    declared_utf8: resolved.declared_utf8,
                    origin: InputOrigin::CompoundMember {
                        parent_source: member.parent_source,
                        member_path: member.member_path,
                        member_index: member.member_index,
                        backing: Box::new(resolved.origin),
                    },
                })
            }
        }
    }
}
