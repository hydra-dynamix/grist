use crate::core::{IndexBase, IndexRange, LocationComponent, SourceLocator};

#[derive(Debug, Clone)]
pub(crate) struct DecodeError {
    pub code: &'static str,
    pub message: String,
    pub offset: usize,
}
impl DecodeError {
    pub fn new(code: &'static str, message: impl Into<String>, offset: usize) -> Self {
        Self {
            code,
            message: message.into(),
            offset,
        }
    }
}
pub(crate) type Result<T> = std::result::Result<T, DecodeError>;

pub(crate) fn byte_locator(start: usize, end: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ByteRange {
        byte_start: start,
        byte_end: end,
    })
    .expect("valid byte locator")
}
pub(crate) fn record_locator(
    collection: impl Into<String>,
    row: u64,
    field: Option<String>,
) -> SourceLocator {
    SourceLocator::exact(LocationComponent::RecordRange {
        collection: collection.into(),
        records: IndexRange {
            start: row,
            end: row.saturating_add(1),
            base: IndexBase::Zero,
        },
        field,
    })
    .expect("valid record locator")
}
pub(crate) fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| DecodeError::new("columnar.truncated", "truncated 32-bit value", offset))?;
    Ok(u32::from_le_bytes(value.try_into().expect("four bytes")))
}
pub(crate) fn read_i32(bytes: &[u8], offset: usize) -> Result<i32> {
    Ok(read_u32(bytes, offset)? as i32)
}
pub(crate) fn read_i64(bytes: &[u8], offset: usize) -> Result<i64> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| DecodeError::new("columnar.truncated", "truncated 64-bit value", offset))?;
    Ok(i64::from_le_bytes(value.try_into().expect("eight bytes")))
}
pub(crate) fn checked_range(bytes: &[u8], start: usize, len: usize) -> Result<&[u8]> {
    let end = start.checked_add(len).ok_or_else(|| {
        DecodeError::new("columnar.offset_overflow", "encoded range overflow", start)
    })?;
    bytes
        .get(start..end)
        .ok_or_else(|| DecodeError::new("columnar.truncated", "encoded range exceeds input", start))
}
