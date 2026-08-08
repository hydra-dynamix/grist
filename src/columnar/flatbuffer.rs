use super::binary::{DecodeError, Result, checked_range, read_i32, read_i64, read_u32};

#[derive(Clone, Copy)]
pub(crate) struct Table<'a> {
    bytes: &'a [u8],
    pos: usize,
    base: usize,
}

impl<'a> Table<'a> {
    pub fn root(bytes: &'a [u8], base: usize) -> Result<Self> {
        let pos = read_u32(bytes, 0)? as usize;
        Self::at(bytes, pos, base)
    }
    pub fn at(bytes: &'a [u8], pos: usize, base: usize) -> Result<Self> {
        let delta = read_i32(bytes, pos)?;
        if delta <= 0 || delta as usize > pos {
            return Err(DecodeError::new(
                "arrow.invalid_flatbuffer",
                "invalid FlatBuffers vtable offset",
                base + pos,
            ));
        }
        let vtable = pos - delta as usize;
        let size = read_u16(bytes, vtable)? as usize;
        checked_range(bytes, vtable, size)?;
        Ok(Self { bytes, pos, base })
    }
    fn field(&self, slot: usize) -> Result<Option<usize>> {
        let delta = read_i32(self.bytes, self.pos)? as usize;
        let vtable = self.pos.checked_sub(delta).ok_or_else(|| {
            DecodeError::new(
                "arrow.invalid_flatbuffer",
                "invalid vtable",
                self.base + self.pos,
            )
        })?;
        let size = read_u16(self.bytes, vtable)? as usize;
        let entry = 4 + slot * 2;
        if entry + 2 > size {
            return Ok(None);
        }
        let offset = read_u16(self.bytes, vtable + entry)? as usize;
        Ok((offset != 0).then_some(self.pos + offset))
    }
    pub fn u8(&self, slot: usize, default: u8) -> Result<u8> {
        Ok(self
            .field(slot)?
            .and_then(|p| self.bytes.get(p).copied())
            .unwrap_or(default))
    }
    pub fn bool(&self, slot: usize, default: bool) -> Result<bool> {
        Ok(self.u8(slot, u8::from(default))? != 0)
    }
    pub fn i16(&self, slot: usize, default: i16) -> Result<i16> {
        match self.field(slot)? {
            Some(p) => Ok(read_u16(self.bytes, p)? as i16),
            None => Ok(default),
        }
    }
    pub fn i32(&self, slot: usize, default: i32) -> Result<i32> {
        match self.field(slot)? {
            Some(p) => read_i32(self.bytes, p),
            None => Ok(default),
        }
    }
    pub fn i64(&self, slot: usize, default: i64) -> Result<i64> {
        match self.field(slot)? {
            Some(p) => read_i64(self.bytes, p),
            None => Ok(default),
        }
    }
    fn indirect(&self, slot: usize) -> Result<Option<usize>> {
        match self.field(slot)? {
            Some(p) => Ok(Some(
                p.checked_add(read_u32(self.bytes, p)? as usize)
                    .ok_or_else(|| {
                        DecodeError::new(
                            "arrow.offset_overflow",
                            "FlatBuffers offset overflow",
                            self.base + p,
                        )
                    })?,
            )),
            None => Ok(None),
        }
    }
    pub fn table(&self, slot: usize) -> Result<Option<Self>> {
        self.indirect(slot)?
            .map(|p| Self::at(self.bytes, p, self.base))
            .transpose()
    }
    pub fn string(&self, slot: usize) -> Result<Option<String>> {
        let Some(p) = self.indirect(slot)? else {
            return Ok(None);
        };
        let len = read_u32(self.bytes, p)? as usize;
        let raw = checked_range(self.bytes, p + 4, len)?;
        String::from_utf8(raw.to_vec()).map(Some).map_err(|_| {
            DecodeError::new(
                "arrow.invalid_utf8",
                "invalid UTF-8 in Arrow metadata",
                self.base + p + 4,
            )
        })
    }
    fn vector(&self, slot: usize, width: usize) -> Result<Option<(usize, usize)>> {
        let Some(p) = self.indirect(slot)? else {
            return Ok(None);
        };
        let len = read_u32(self.bytes, p)? as usize;
        checked_range(
            self.bytes,
            p + 4,
            len.checked_mul(width).ok_or_else(|| {
                DecodeError::new(
                    "arrow.offset_overflow",
                    "vector size overflow",
                    self.base + p,
                )
            })?,
        )?;
        Ok(Some((p + 4, len)))
    }
    pub fn table_vec(&self, slot: usize) -> Result<Vec<Self>> {
        let Some((p, len)) = self.vector(slot, 4)? else {
            return Ok(Vec::new());
        };
        (0..len)
            .map(|i| {
                let item = p + i * 4;
                let target = item + read_u32(self.bytes, item)? as usize;
                Self::at(self.bytes, target, self.base)
            })
            .collect()
    }
    pub fn i32_vec(&self, slot: usize) -> Result<Vec<i32>> {
        let Some((p, len)) = self.vector(slot, 4)? else {
            return Ok(Vec::new());
        };
        (0..len).map(|i| read_i32(self.bytes, p + i * 4)).collect()
    }
    pub fn struct_vec_16(&self, slot: usize) -> Result<Vec<(i64, i64)>> {
        let Some((p, len)) = self.vector(slot, 16)? else {
            return Ok(Vec::new());
        };
        (0..len)
            .map(|i| {
                Ok((
                    read_i64(self.bytes, p + i * 16)?,
                    read_i64(self.bytes, p + i * 16 + 8)?,
                ))
            })
            .collect()
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let raw = bytes.get(offset..offset + 2).ok_or_else(|| {
        DecodeError::new(
            "arrow.truncated_metadata",
            "truncated FlatBuffers scalar",
            offset,
        )
    })?;
    Ok(u16::from_le_bytes(raw.try_into().expect("two bytes")))
}

pub(crate) fn key_values(
    table: &Table<'_>,
    slot: usize,
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut out = std::collections::BTreeMap::new();
    for kv in table.table_vec(slot)? {
        let key = kv.string(0)?.unwrap_or_default();
        let value = kv.string(1)?.unwrap_or_default();
        out.insert(key, value);
    }
    Ok(out)
}
