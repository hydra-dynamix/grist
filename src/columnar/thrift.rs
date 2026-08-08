use super::binary::{DecodeError, Result, checked_range};
use std::collections::BTreeMap;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) enum Value {
    Bool(bool),
    I64(i64),
    Double(u64),
    Binary(Vec<u8>),
    List(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Struct(BTreeMap<i16, Value>),
}
impl Value {
    pub fn int(&self) -> Option<i64> {
        if let Self::I64(v) = self {
            Some(*v)
        } else {
            None
        }
    }
    pub fn string(&self) -> Option<String> {
        if let Self::Binary(v) = self {
            String::from_utf8(v.clone()).ok()
        } else {
            None
        }
    }
    pub fn list(&self) -> Option<&[Value]> {
        if let Self::List(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn structure(&self) -> Option<&BTreeMap<i16, Value>> {
        if let Self::Struct(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pub pos: usize,
    max_depth: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8], offset: usize) -> Self {
        Self {
            bytes,
            pos: offset,
            max_depth: 128,
        }
    }
    pub fn read_struct(&mut self) -> Result<BTreeMap<i16, Value>> {
        self.structure(0)
    }
    fn structure(&mut self, depth: usize) -> Result<BTreeMap<i16, Value>> {
        if depth > self.max_depth {
            return Err(DecodeError::new(
                "parquet.nesting_limit",
                "Thrift metadata nesting limit exceeded",
                self.pos,
            ));
        }
        let mut fields = BTreeMap::new();
        let mut previous = 0i16;
        loop {
            let header = self.byte()?;
            if header == 0 {
                break;
            }
            let kind = header & 15;
            let delta = (header >> 4) as i16;
            let id = if delta == 0 {
                self.zigzag()? as i16
            } else {
                previous + delta
            };
            previous = id;
            let value = match kind {
                1 => Value::Bool(true),
                2 => Value::Bool(false),
                _ => self.value(kind, depth + 1)?,
            };
            fields.insert(id, value);
        }
        Ok(fields)
    }
    fn value(&mut self, kind: u8, depth: usize) -> Result<Value> {
        Ok(match kind {
            3 => Value::I64(self.byte()? as i8 as i64),
            4..=6 => Value::I64(self.zigzag()?),
            7 => {
                let raw = checked_range(self.bytes, self.pos, 8)?;
                self.pos += 8;
                Value::Double(u64::from_le_bytes(raw.try_into().unwrap()))
            }
            8 => {
                let len = self.varint()? as usize;
                let raw = checked_range(self.bytes, self.pos, len)?.to_vec();
                self.pos += len;
                Value::Binary(raw)
            }
            9 | 10 => {
                let head = self.byte()?;
                let mut len = (head >> 4) as usize;
                let item = head & 15;
                if len == 15 {
                    len = self.varint()? as usize
                }
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(self.value(item, depth + 1)?)
                }
                Value::List(values)
            }
            11 => {
                let len = self.varint()? as usize;
                if len == 0 {
                    Value::Map(Vec::new())
                } else {
                    let types = self.byte()?;
                    let key = types >> 4;
                    let value = types & 15;
                    let mut values = Vec::with_capacity(len);
                    for _ in 0..len {
                        values.push((self.value(key, depth + 1)?, self.value(value, depth + 1)?))
                    }
                    Value::Map(values)
                }
            }
            12 => Value::Struct(self.structure(depth + 1)?),
            _ => {
                return Err(DecodeError::new(
                    "parquet.invalid_thrift",
                    format!("unknown compact type {kind}"),
                    self.pos,
                ));
            }
        })
    }
    fn byte(&mut self) -> Result<u8> {
        let value = *self.bytes.get(self.pos).ok_or_else(|| {
            DecodeError::new(
                "parquet.truncated_metadata",
                "truncated Thrift compact value",
                self.pos,
            )
        })?;
        self.pos += 1;
        Ok(value)
    }
    fn varint(&mut self) -> Result<u64> {
        let start = self.pos;
        let mut out = 0u64;
        for shift in (0..70).step_by(7) {
            let b = self.byte()?;
            if shift == 63 && b > 1 {
                return Err(DecodeError::new(
                    "parquet.invalid_varint",
                    "Thrift varint overflow",
                    start,
                ));
            }
            out |= ((b & 127) as u64) << shift;
            if b & 128 == 0 {
                return Ok(out);
            }
        }
        Err(DecodeError::new(
            "parquet.invalid_varint",
            "unterminated Thrift varint",
            start,
        ))
    }
    fn zigzag(&mut self) -> Result<i64> {
        let v = self.varint()?;
        Ok(((v >> 1) as i64) ^ (-((v & 1) as i64)))
    }
}

pub(crate) fn field(map: &BTreeMap<i16, Value>, id: i16) -> Option<&Value> {
    map.get(&id)
}
pub(crate) fn int(map: &BTreeMap<i16, Value>, id: i16, default: i64) -> i64 {
    field(map, id).and_then(Value::int).unwrap_or(default)
}
pub(crate) fn string(map: &BTreeMap<i16, Value>, id: i16) -> Option<String> {
    field(map, id).and_then(Value::string)
}
pub(crate) fn structs(map: &BTreeMap<i16, Value>, id: i16) -> Vec<&BTreeMap<i16, Value>> {
    field(map, id)
        .and_then(Value::list)
        .map(|items| items.iter().filter_map(Value::structure).collect())
        .unwrap_or_default()
}
pub(crate) fn ints(map: &BTreeMap<i16, Value>, id: i16) -> Vec<i64> {
    field(map, id)
        .and_then(Value::list)
        .map(|items| items.iter().filter_map(Value::int).collect())
        .unwrap_or_default()
}
pub(crate) fn strings(map: &BTreeMap<i16, Value>, id: i16) -> Vec<String> {
    field(map, id)
        .and_then(Value::list)
        .map(|items| items.iter().filter_map(Value::string).collect())
        .unwrap_or_default()
}
