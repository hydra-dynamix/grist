use super::model::{CborTag, MessagePackExtension, ProtobufUnknownField};
use crate::core::{
    Diagnostic, IndexBase, IndexRange, LocationComponent, LocatorPrecision, OperationControl,
    OperationStatus, SourceLocator,
};

#[derive(Debug, Clone)]
pub(crate) struct DecodeError {
    pub code: &'static str,
    pub message: String,
    pub offset: usize,
}

impl DecodeError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>, offset: usize) -> Self {
        Self {
            code,
            message: message.into(),
            offset,
        }
    }
}

pub(crate) struct DecodeState<'a> {
    pub format_name: &'static str,
    pub max_depth_limit: usize,
    pub max_values: usize,
    pub max_collection_items: usize,
    pub max_blob_bytes: usize,
    pub value_count: usize,
    pub max_depth: usize,
    pub tags: Vec<CborTag>,
    pub extensions: Vec<MessagePackExtension>,
    pub unknown_fields: Vec<ProtobufUnknownField>,
    pub diagnostics: Vec<Diagnostic>,
    pub terminal_status: Option<OperationStatus>,
    control: Option<&'a OperationControl>,
}

impl<'a> DecodeState<'a> {
    pub(crate) fn new(
        format_name: &'static str,
        _input_len: usize,
        options: &super::model::StructuredBinaryOptions,
        control: Option<&'a OperationControl>,
    ) -> Self {
        let shared_depth = control
            .and_then(|control| control.budget().budget().max_nesting_depth)
            .and_then(|value| usize::try_from(value).ok());
        Self {
            format_name,
            max_depth_limit: shared_depth.map_or(options.max_nesting_depth, |limit| {
                limit.min(options.max_nesting_depth)
            }),
            max_values: options.max_values,
            max_collection_items: options.max_collection_items,
            max_blob_bytes: options.max_blob_bytes,
            value_count: 0,
            max_depth: 0,
            tags: Vec::new(),
            extensions: Vec::new(),
            unknown_fields: Vec::new(),
            diagnostics: Vec::new(),
            terminal_status: None,
            control,
        }
    }

    pub(crate) fn start_value(&mut self, depth: usize, offset: usize) -> Result<(), DecodeError> {
        if let Some(control) = self.control
            && let Err(error) = control.checkpoint()
        {
            self.terminal_status = Some(error.operation_status(0));
            self.diagnostics
                .push(error.diagnostic("grist.structured-binary"));
            return Err(DecodeError::new(
                "binary.operation_control",
                error.to_string(),
                offset,
            ));
        }
        if depth > self.max_depth_limit {
            return Err(DecodeError::new(
                "binary.nesting_limit",
                format!(
                    "{} nesting depth {} exceeds explicit limit {}",
                    self.format_name, depth, self.max_depth_limit
                ),
                offset,
            ));
        }
        if self.value_count >= self.max_values {
            return Err(DecodeError::new(
                "binary.value_limit",
                format!(
                    "{} value count exceeds explicit limit {}",
                    self.format_name, self.max_values
                ),
                offset,
            ));
        }
        self.value_count += 1;
        self.max_depth = self.max_depth.max(depth);
        Ok(())
    }

    pub(crate) fn check_collection(&self, count: usize, offset: usize) -> Result<(), DecodeError> {
        if count > self.max_collection_items {
            return Err(DecodeError::new(
                "binary.collection_limit",
                format!(
                    "{} collection length {} exceeds explicit limit {}",
                    self.format_name, count, self.max_collection_items
                ),
                offset,
            ));
        }
        Ok(())
    }

    pub(crate) fn check_blob(&self, length: usize, offset: usize) -> Result<(), DecodeError> {
        if length > self.max_blob_bytes {
            return Err(DecodeError::new(
                "binary.blob_limit",
                format!(
                    "{} byte/text length {} exceeds explicit limit {}",
                    self.format_name, length, self.max_blob_bytes
                ),
                offset,
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
    end: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            end: bytes.len(),
        }
    }

    pub(crate) fn bounded(bytes: &'a [u8], start: usize, end: usize) -> Result<Self, DecodeError> {
        if start > end || end > bytes.len() {
            return Err(DecodeError::new(
                "binary.invalid_range",
                "bounded cursor range is outside input",
                start,
            ));
        }
        Ok(Self {
            bytes,
            position: start,
            end,
        })
    }

    pub(crate) fn bounded_bytes(
        source: &Self,
        start: usize,
        end: usize,
        code: &'static str,
    ) -> Result<Self, DecodeError> {
        if start > end || end > source.end {
            return Err(DecodeError::new(
                code,
                "declared bounded range exceeds the containing field",
                start,
            ));
        }
        Self::bounded(source.bytes, start, end)
    }

    pub(crate) const fn position(&self) -> usize {
        self.position
    }

    pub(crate) const fn remaining(&self) -> usize {
        self.end - self.position
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.position == self.end
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        (self.position < self.end).then(|| self.bytes[self.position])
    }

    pub(crate) fn read_u8(&mut self, code: &'static str) -> Result<u8, DecodeError> {
        let offset = self.position;
        let byte = *self
            .bytes
            .get(self.position)
            .filter(|_| self.position < self.end)
            .ok_or_else(|| DecodeError::new(code, "unexpected end of input", offset))?;
        self.position += 1;
        Ok(byte)
    }

    pub(crate) fn take(
        &mut self,
        length: usize,
        code: &'static str,
    ) -> Result<&'a [u8], DecodeError> {
        let start = self.position;
        let end = start.checked_add(length).ok_or_else(|| {
            DecodeError::new(code, "declared length overflows address space", start)
        })?;
        if end > self.end {
            return Err(DecodeError::new(
                code,
                format!(
                    "declared length {} exceeds {} remaining bytes",
                    length,
                    self.remaining()
                ),
                start,
            ));
        }
        self.position = end;
        Ok(&self.bytes[start..end])
    }

    pub(crate) fn read_be_u16(&mut self, code: &'static str) -> Result<u16, DecodeError> {
        let bytes: [u8; 2] = self.take(2, code)?.try_into().expect("exact length");
        Ok(u16::from_be_bytes(bytes))
    }

    pub(crate) fn read_be_u32(&mut self, code: &'static str) -> Result<u32, DecodeError> {
        let bytes: [u8; 4] = self.take(4, code)?.try_into().expect("exact length");
        Ok(u32::from_be_bytes(bytes))
    }

    pub(crate) fn read_be_u64(&mut self, code: &'static str) -> Result<u64, DecodeError> {
        let bytes: [u8; 8] = self.take(8, code)?.try_into().expect("exact length");
        Ok(u64::from_be_bytes(bytes))
    }

    pub(crate) fn read_le_u32(&mut self, code: &'static str) -> Result<u32, DecodeError> {
        let bytes: [u8; 4] = self.take(4, code)?.try_into().expect("exact length");
        Ok(u32::from_le_bytes(bytes))
    }

    pub(crate) fn read_le_u64(&mut self, code: &'static str) -> Result<u64, DecodeError> {
        let bytes: [u8; 8] = self.take(8, code)?.try_into().expect("exact length");
        Ok(u64::from_le_bytes(bytes))
    }

    pub(crate) fn read_varint(&mut self, code: &'static str) -> Result<u64, DecodeError> {
        let start = self.position;
        let mut value = 0u64;
        for shift in (0..70).step_by(7) {
            let byte = self.read_u8(code)?;
            if shift == 63 && byte > 1 {
                return Err(DecodeError::new(code, "varint overflows 64 bits", start));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(DecodeError::new(code, "varint exceeds ten bytes", start))
    }

    pub(crate) fn advance_to(
        &mut self,
        position: usize,
        code: &'static str,
    ) -> Result<(), DecodeError> {
        if position < self.position || position > self.end {
            return Err(DecodeError::new(
                code,
                "requested cursor position is outside the containing field",
                self.position,
            ));
        }
        self.position = position;
        Ok(())
    }

    pub(crate) fn bytes_range(
        &self,
        start: usize,
        end: usize,
        code: &'static str,
    ) -> Result<&'a [u8], DecodeError> {
        self.bytes
            .get(start..end)
            .ok_or_else(|| DecodeError::new(code, "byte range is outside input", start))
    }
}

pub(crate) fn locator(
    collection: &str,
    record_index: usize,
    record_start: usize,
    start: usize,
    end: usize,
    field: Option<String>,
) -> SourceLocator {
    let record = u64::try_from(record_index).unwrap_or(u64::MAX);
    SourceLocator::new(
        vec![
            LocationComponent::RecordRange {
                collection: collection.to_string(),
                records: IndexRange::new(record, record.saturating_add(1), IndexBase::One)
                    .expect("one-based record range is valid"),
                field,
            },
            LocationComponent::ByteRange {
                byte_start: start.saturating_sub(record_start),
                byte_end: end.saturating_sub(record_start),
            },
        ],
        LocatorPrecision::Exact { derived_from: None },
    )
    .expect("binary record locator is valid")
}

pub(crate) fn byte_locator(start: usize, end: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ByteRange {
        byte_start: start,
        byte_end: end,
    })
    .expect("byte locator is valid")
}
