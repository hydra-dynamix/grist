use super::{SqliteHeader, SqliteJournalMode, SqliteTextEncoding, SqliteValue};
use crate::core::OperationControl;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub(crate) struct DecodeError {
    pub code: &'static str,
    pub message: String,
    pub offset: usize,
}
impl DecodeError {
    fn new(code: &'static str, message: impl Into<String>, offset: usize) -> Self {
        Self {
            code,
            message: message.into(),
            offset,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RawRecord {
    pub rowid: Option<i64>,
    pub values: Vec<SqliteValue>,
    pub page: u32,
    pub cell: usize,
    pub byte_start: usize,
    pub byte_end: usize,
}

pub(crate) struct Database<'a> {
    bytes: &'a [u8],
    pub header: SqliteHeader,
    page_size: usize,
    usable_size: usize,
    control: Option<&'a OperationControl>,
}

impl<'a> Database<'a> {
    pub fn open(
        bytes: &'a [u8],
        control: Option<&'a OperationControl>,
    ) -> Result<Self, DecodeError> {
        if bytes.len() < 100 || bytes.get(..15) != Some(b"SQLite format 3") || bytes[15] != 0 {
            return Err(DecodeError::new(
                "sqlite.header.invalid",
                "missing or truncated SQLite 3 header",
                0,
            ));
        }
        let raw = read_u16(bytes, 16)?;
        let page_size = if raw == 1 { 65_536 } else { usize::from(raw) };
        if !(512..=65_536).contains(&page_size) || !page_size.is_power_of_two() {
            return Err(DecodeError::new(
                "sqlite.header.page_size",
                format!("invalid page size {page_size}"),
                16,
            ));
        }
        let reserved = usize::from(bytes[20]);
        if reserved >= page_size || page_size - reserved < 480 {
            return Err(DecodeError::new(
                "sqlite.header.reserved_space",
                "invalid usable page size",
                20,
            ));
        }
        if bytes.len() % page_size != 0 {
            return Err(DecodeError::new(
                "sqlite.file.truncated",
                "file length is not a page multiple",
                bytes.len(),
            ));
        }
        let actual_pages = bytes.len() / page_size;
        if actual_pages == 0 || actual_pages > u32::MAX as usize {
            return Err(DecodeError::new(
                "sqlite.file.page_count",
                "invalid database page count",
                28,
            ));
        }
        let header_pages = read_u32(bytes, 28)?;
        if header_pages != 0 && header_pages as usize > actual_pages {
            return Err(DecodeError::new(
                "sqlite.file.truncated",
                "header declares pages that are not present",
                28,
            ));
        }
        let mode = |value: u8, offset| match value {
            1 => Ok(SqliteJournalMode::Legacy),
            2 => Ok(SqliteJournalMode::Wal),
            _ => Err(DecodeError::new(
                "sqlite.header.journal_mode",
                format!("unsupported journal mode byte {value}"),
                offset,
            )),
        };
        let encoding = match read_u32(bytes, 56)? {
            1 => SqliteTextEncoding::Utf8,
            2 => SqliteTextEncoding::Utf16Le,
            3 => SqliteTextEncoding::Utf16Be,
            value => {
                return Err(DecodeError::new(
                    "sqlite.header.text_encoding",
                    format!("unsupported text encoding {value}"),
                    56,
                ));
            }
        };
        let header = SqliteHeader {
            page_size: page_size as u32,
            usable_page_size: (page_size - reserved) as u32,
            page_count: actual_pages as u32,
            header_page_count: header_pages,
            write_version: mode(bytes[18], 18)?,
            read_version: mode(bytes[19], 19)?,
            text_encoding: encoding,
            schema_cookie: read_u32(bytes, 40)?,
            schema_format: read_u32(bytes, 44)?,
            user_version: read_u32(bytes, 60)?,
            application_id: read_u32(bytes, 68)?,
            sqlite_version_number: read_u32(bytes, 96)?,
        };
        Ok(Self {
            bytes,
            header,
            page_size,
            usable_size: page_size - reserved,
            control,
        })
    }

    pub fn table_records(
        &self,
        root_page: u32,
        limit: usize,
    ) -> Result<(Vec<RawRecord>, bool), DecodeError> {
        if root_page == 0 {
            return Err(DecodeError::new(
                "sqlite.table.no_storage",
                "table has no on-disk root page",
                0,
            ));
        }
        let mut records = Vec::new();
        self.walk(
            root_page,
            limit.saturating_add(1),
            0,
            &mut BTreeSet::new(),
            &mut records,
        )?;
        let truncated = records.len() > limit;
        records.truncate(limit);
        Ok((records, truncated))
    }

    fn walk(
        &self,
        page_number: u32,
        limit: usize,
        depth: usize,
        visited: &mut BTreeSet<u32>,
        out: &mut Vec<RawRecord>,
    ) -> Result<(), DecodeError> {
        if out.len() >= limit {
            return Ok(());
        }
        let page_offset = self.page_offset(page_number)?;
        if depth > 128 {
            return Err(DecodeError::new(
                "sqlite.btree.depth",
                "B-tree nesting exceeds 128 pages",
                page_offset,
            ));
        }
        if !visited.insert(page_number) {
            return Err(DecodeError::new(
                "sqlite.btree.cycle",
                format!("B-tree page {page_number} is visited more than once"),
                page_offset,
            ));
        }
        if let Some(control) = self.control {
            control.checkpoint().map_err(|error| {
                DecodeError::new(
                    "sqlite.operation.interrupted",
                    error.to_string(),
                    page_offset,
                )
            })?;
            control.budget().consume_pages(1).map_err(|error| {
                DecodeError::new("sqlite.budget.pages", error.to_string(), page_offset)
            })?;
            control
                .budget()
                .observe_nesting_depth(depth as u64)
                .map_err(|error| {
                    DecodeError::new("sqlite.budget.depth", error.to_string(), page_offset)
                })?;
        }
        let page = &self.bytes[page_offset..page_offset + self.page_size];
        let header_at = if page_number == 1 { 100 } else { 0 };
        let page_type = *page.get(header_at).ok_or_else(|| {
            DecodeError::new(
                "sqlite.btree.header",
                "truncated B-tree header",
                page_offset,
            )
        })?;
        let interior = matches!(page_type, 0x02 | 0x05);
        if !matches!(page_type, 0x02 | 0x05 | 0x0a | 0x0d) {
            return Err(DecodeError::new(
                "sqlite.btree.page_type",
                format!("unsupported B-tree page type 0x{page_type:02x}"),
                page_offset + header_at,
            ));
        }
        let header_len = if interior { 12 } else { 8 };
        let count = usize::from(read_u16(page, header_at + 3)?);
        if header_at + header_len + count.saturating_mul(2) > self.usable_size {
            return Err(DecodeError::new(
                "sqlite.btree.cell_pointers",
                "cell pointer array exceeds usable page",
                page_offset + header_at,
            ));
        }
        for cell_index in 0..count {
            if out.len() >= limit {
                break;
            }
            let pointer_at = header_at + header_len + cell_index * 2;
            let ptr = usize::from(read_u16(page, pointer_at)?);
            if ptr >= self.usable_size {
                return Err(DecodeError::new(
                    "sqlite.btree.cell_offset",
                    format!("cell {cell_index} points outside page"),
                    page_offset + pointer_at,
                ));
            }
            match page_type {
                0x05 => self.walk(read_u32(page, ptr)?, limit, depth + 1, visited, out)?,
                0x02 => {
                    self.walk(read_u32(page, ptr)?, limit, depth + 1, visited, out)?;
                    if out.len() < limit {
                        out.push(self.leaf_record(
                            page_number,
                            page,
                            ptr + 4,
                            cell_index,
                            false,
                        )?);
                    }
                }
                0x0d => out.push(self.leaf_record(page_number, page, ptr, cell_index, true)?),
                0x0a => out.push(self.leaf_record(page_number, page, ptr, cell_index, false)?),
                _ => unreachable!(),
            }
        }
        if interior && out.len() < limit {
            self.walk(
                read_u32(page, header_at + 8)?,
                limit,
                depth + 1,
                visited,
                out,
            )?;
        }
        Ok(())
    }

    fn leaf_record(
        &self,
        page_number: u32,
        page: &[u8],
        ptr: usize,
        cell: usize,
        table_leaf: bool,
    ) -> Result<RawRecord, DecodeError> {
        let (payload_size, n1) = varint(page, ptr)?;
        let (rowid, n2) = if table_leaf {
            let (value, width) = varint(page, ptr + n1)?;
            (Some(value as i64), width)
        } else {
            (None, 0)
        };
        let payload = self.payload(
            page_number,
            page,
            ptr + n1 + n2,
            payload_size as usize,
            table_leaf,
        )?;
        Ok(RawRecord {
            rowid,
            values: decode_record(&payload.bytes, self.header.text_encoding)?,
            page: page_number,
            cell,
            byte_start: self.page_offset(page_number)? + ptr,
            byte_end: self.page_offset(page_number)? + payload.local_end,
        })
    }

    fn payload(
        &self,
        page_number: u32,
        page: &[u8],
        start: usize,
        size: usize,
        table_leaf: bool,
    ) -> Result<Payload, DecodeError> {
        let max_local = if table_leaf {
            self.usable_size - 35
        } else {
            ((self.usable_size - 12) * 64 / 255) - 23
        };
        let min_local = ((self.usable_size - 12) * 32 / 255) - 23;
        let local = if size <= max_local {
            size
        } else {
            let candidate = min_local + (size - min_local) % (self.usable_size - 4);
            if candidate <= max_local {
                candidate
            } else {
                min_local
            }
        };
        let local_end = start.checked_add(local).ok_or_else(|| {
            DecodeError::new("sqlite.payload.overflow", "payload offset overflow", start)
        })?;
        if local_end > self.usable_size || local_end > page.len() {
            return Err(DecodeError::new(
                "sqlite.payload.truncated",
                "local record payload exceeds page",
                self.page_offset(page_number)? + start,
            ));
        }
        let mut bytes = page[start..local_end].to_vec();
        if local < size {
            if local_end + 4 > self.usable_size {
                return Err(DecodeError::new(
                    "sqlite.overflow.pointer",
                    "missing overflow page pointer",
                    self.page_offset(page_number)? + local_end,
                ));
            }
            let mut next = read_u32(page, local_end)?;
            let mut seen = BTreeSet::new();
            while bytes.len() < size {
                if let Some(control) = self.control {
                    control.checkpoint().map_err(|error| {
                        DecodeError::new(
                            "sqlite.operation.interrupted",
                            error.to_string(),
                            self.page_offset(page_number).unwrap_or(0) + local_end,
                        )
                    })?;
                    control.budget().consume_pages(1).map_err(|error| {
                        DecodeError::new(
                            "sqlite.budget.pages",
                            error.to_string(),
                            self.page_offset(page_number).unwrap_or(0) + local_end,
                        )
                    })?;
                }
                if next == 0 || !seen.insert(next) {
                    return Err(DecodeError::new(
                        "sqlite.overflow.chain",
                        "broken or cyclic overflow page chain",
                        self.page_offset(page_number)? + local_end,
                    ));
                }
                let offset = self.page_offset(next)?;
                let overflow = &self.bytes[offset..offset + self.page_size];
                next = read_u32(overflow, 0)?;
                let take = (size - bytes.len()).min(self.usable_size - 4);
                bytes.extend_from_slice(&overflow[4..4 + take]);
            }
        }
        Ok(Payload { bytes, local_end })
    }

    fn page_offset(&self, page_number: u32) -> Result<usize, DecodeError> {
        if page_number == 0 || page_number > self.header.page_count {
            return Err(DecodeError::new(
                "sqlite.page.out_of_range",
                format!("page {page_number} is outside the database"),
                0,
            ));
        }
        Ok((page_number as usize - 1) * self.page_size)
    }
}

struct Payload {
    bytes: Vec<u8>,
    local_end: usize,
}

fn decode_record(
    bytes: &[u8],
    encoding: SqliteTextEncoding,
) -> Result<Vec<SqliteValue>, DecodeError> {
    let (header_size, header_width) = varint(bytes, 0)?;
    let header_size = header_size as usize;
    if header_size < header_width || header_size > bytes.len() {
        return Err(DecodeError::new(
            "sqlite.record.header",
            "record header size exceeds payload",
            0,
        ));
    }
    let mut at = header_width;
    let mut serials = Vec::new();
    while at < header_size {
        let (serial, width) = varint(bytes, at)?;
        at += width;
        if at > header_size {
            return Err(DecodeError::new(
                "sqlite.record.serial_types",
                "serial type crosses record header",
                at,
            ));
        }
        serials.push(serial);
    }
    let mut body = header_size;
    let mut output = Vec::with_capacity(serials.len());
    for serial in serials {
        let length = serial_len(serial)?;
        if body + length > bytes.len() {
            return Err(DecodeError::new(
                "sqlite.record.value",
                "record value exceeds payload",
                body,
            ));
        }
        output.push(decode_value(serial, &bytes[body..body + length], encoding));
        body += length;
    }
    Ok(output)
}

fn serial_len(serial: u64) -> Result<usize, DecodeError> {
    match serial {
        0 | 8 | 9 | 10 | 11 => Ok(0),
        1 => Ok(1),
        2 => Ok(2),
        3 => Ok(3),
        4 => Ok(4),
        5 => Ok(6),
        6 | 7 => Ok(8),
        n if n >= 12 => usize::try_from((n - 12) / 2)
            .map_err(|_| DecodeError::new("sqlite.record.length", "record field is too large", 0)),
        _ => unreachable!(),
    }
}

fn decode_value(serial: u64, raw: &[u8], encoding: SqliteTextEncoding) -> SqliteValue {
    match serial {
        0 => SqliteValue::Null,
        1..=6 => SqliteValue::Integer {
            value: signed_be(raw),
        },
        7 => SqliteValue::Real {
            value: f64::from_bits(u64::from_be_bytes(raw.try_into().unwrap())),
        },
        8 => SqliteValue::Integer { value: 0 },
        9 => SqliteValue::Integer { value: 1 },
        10 | 11 => SqliteValue::Reserved {
            serial_type: serial,
        },
        n if n % 2 == 0 => SqliteValue::Blob {
            hex: hex(raw),
            byte_length: raw.len(),
        },
        _ => {
            let (value, lossy) = decode_text(raw, encoding);
            SqliteValue::Text {
                value,
                encoding,
                lossy,
            }
        }
    }
}

fn decode_text(raw: &[u8], encoding: SqliteTextEncoding) -> (String, bool) {
    match encoding {
        SqliteTextEncoding::Utf8 => {
            let text = String::from_utf8_lossy(raw);
            let lossy = matches!(text, std::borrow::Cow::Owned(_));
            (text.into_owned(), lossy)
        }
        SqliteTextEncoding::Utf16Le | SqliteTextEncoding::Utf16Be => {
            let odd = raw.len() % 2 != 0;
            let units = raw
                .chunks_exact(2)
                .map(|pair| {
                    if encoding == SqliteTextEncoding::Utf16Le {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                })
                .collect::<Vec<_>>();
            let lossy = odd || char::decode_utf16(units.iter().copied()).any(|item| item.is_err());
            (String::from_utf16_lossy(&units), lossy)
        }
    }
}

fn signed_be(raw: &[u8]) -> i64 {
    let mut full = if raw.first().is_some_and(|byte| byte & 0x80 != 0) {
        [0xff; 8]
    } else {
        [0; 8]
    };
    full[8 - raw.len()..].copy_from_slice(raw);
    i64::from_be_bytes(full)
}

fn varint(bytes: &[u8], offset: usize) -> Result<(u64, usize), DecodeError> {
    let mut value = 0;
    for index in 0..9 {
        let byte = *bytes.get(offset + index).ok_or_else(|| {
            DecodeError::new(
                "sqlite.varint.truncated",
                "truncated SQLite varint",
                offset + index,
            )
        })?;
        if index == 8 {
            return Ok(((value << 8) | u64::from(byte), 9));
        }
        value = (value << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    unreachable!()
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DecodeError> {
    let raw = bytes.get(offset..offset + 2).ok_or_else(|| {
        DecodeError::new("sqlite.read.truncated", "truncated 16-bit field", offset)
    })?;
    Ok(u16::from_be_bytes(raw.try_into().unwrap()))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DecodeError> {
    let raw = bytes.get(offset..offset + 4).ok_or_else(|| {
        DecodeError::new("sqlite.read.truncated", "truncated 32-bit field", offset)
    })?;
    Ok(u32::from_be_bytes(raw.try_into().unwrap()))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}
