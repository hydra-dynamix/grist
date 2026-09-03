use super::{
    PdfIndirectObject, PdfObjectLocation, PdfObjectLocator, PdfOptions, PdfReference, PdfStream,
    PdfStreamDecodeStatus, PdfString, PdfStringEncoding, PdfValue,
};
use crate::core::sha256_hex;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone)]
pub(crate) struct SyntaxIssue {
    pub code: &'static str,
    pub message: String,
    pub offset: usize,
    pub object: Option<PdfReference>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawObject {
    pub model: PdfIndirectObject,
    pub stream_bytes: Option<Vec<u8>>,
    pub decoded_stream: Option<Vec<u8>>,
}

pub(crate) fn scan_indirect_objects(
    bytes: &[u8],
    options: &PdfOptions,
) -> (Vec<RawObject>, Vec<SyntaxIssue>, bool) {
    let mut objects = Vec::new();
    let mut issues = Vec::new();
    let mut revisions = HashMap::<PdfReference, u32>::new();
    let mut cursor = 0usize;
    let mut limit_hit = false;
    while cursor < bytes.len() {
        if objects.len() >= options.max_objects as usize {
            limit_hit = true;
            issues.push(SyntaxIssue {
                code: "pdf.budget.objects",
                message: format!("PDF object limit {} was reached", options.max_objects),
                offset: cursor,
                object: None,
            });
            break;
        }
        let Some((reference, value_start)) = indirect_header(bytes, cursor) else {
            cursor = cursor.saturating_add(1);
            continue;
        };
        let mut parser = ValueParser::new(bytes, value_start, options.max_object_depth);
        let value = match parser.parse_value(0) {
            Ok(value) => value,
            Err(error) => {
                issues.push(SyntaxIssue {
                    code: "pdf.object.invalid",
                    message: error.message,
                    offset: error.offset,
                    object: Some(reference),
                });
                cursor = value_start.saturating_add(1);
                continue;
            }
        };
        let value_end = parser.position();
        parser.skip_space();
        let mut stream_bytes = None;
        let mut stream_model = None;
        let mut repaired = false;
        if parser.consume_keyword(b"stream") {
            consume_stream_eol(bytes, &mut parser.cursor);
            let stream_start = parser.cursor;
            let declared_length = value
                .as_dictionary()
                .and_then(|dictionary| dictionary.get("Length"))
                .and_then(PdfValue::as_integer)
                .and_then(|length| u64::try_from(length).ok());
            let (stream_end, after_stream, length_repaired) = locate_stream_end(
                bytes,
                stream_start,
                declared_length.and_then(|length| usize::try_from(length).ok()),
            );
            repaired |= length_repaired;
            if length_repaired {
                issues.push(SyntaxIssue {
                    code: "pdf.stream.length_repaired",
                    message: "stream boundary did not match the declared Length; recovered from endstream".into(),
                    offset: stream_start,
                    object: Some(reference),
                });
            }
            let encoded = bytes[stream_start..stream_end].to_vec();
            stream_model = Some(PdfStream {
                byte_start: stream_start as u64,
                byte_end: stream_end as u64,
                declared_length,
                actual_length: encoded.len() as u64,
                encoded_sha256: sha256_hex(&encoded),
                decoded_length: None,
                decoded_sha256: None,
                decode_status: PdfStreamDecodeStatus::NotFiltered,
            });
            stream_bytes = Some(encoded);
            parser.cursor = after_stream;
        }
        parser.skip_space();
        let end = if parser.consume_keyword(b"endobj") {
            parser.cursor
        } else if let Some(relative) = find_token(&bytes[parser.cursor..], b"endobj") {
            repaired = true;
            issues.push(SyntaxIssue {
                code: "pdf.object.end_repaired",
                message: "object end marker required bounded recovery".into(),
                offset: parser.cursor,
                object: Some(reference),
            });
            parser.cursor + relative + b"endobj".len()
        } else {
            repaired = true;
            issues.push(SyntaxIssue {
                code: "pdf.object.truncated",
                message: "object has no endobj marker".into(),
                offset: parser.cursor,
                object: Some(reference),
            });
            value_end.max(parser.cursor)
        };
        let revision = revisions.entry(reference).or_default();
        let object_revision = *revision;
        *revision = revision.saturating_add(1);
        let locator = PdfObjectLocator::direct(reference, cursor, end);
        objects.push(RawObject {
            model: PdfIndirectObject {
                object: reference,
                revision: object_revision,
                locator,
                value,
                stream: stream_model,
                raw_sha256: sha256_hex(&bytes[cursor..end.min(bytes.len())]),
                repaired,
            },
            stream_bytes,
            decoded_stream: None,
        });
        cursor = end.max(cursor.saturating_add(1));
    }
    (objects, issues, limit_hit)
}

fn indirect_header(bytes: &[u8], start: usize) -> Option<(PdfReference, usize)> {
    if start > 0 && !is_boundary(bytes[start - 1]) {
        return None;
    }
    let mut cursor = start;
    let object_number = parse_unsigned(bytes, &mut cursor)?;
    if !skip_required_space(bytes, &mut cursor) {
        return None;
    }
    let generation = parse_unsigned(bytes, &mut cursor)?;
    if !skip_required_space(bytes, &mut cursor) || !consume_token(bytes, &mut cursor, b"obj") {
        return None;
    }
    Some((
        PdfReference {
            object_number: u32::try_from(object_number).ok()?,
            generation: u16::try_from(generation).ok()?,
        },
        cursor,
    ))
}

pub(crate) fn parse_value_at(
    bytes: &[u8],
    offset: usize,
    max_depth: u16,
) -> Result<(PdfValue, usize), SyntaxIssue> {
    let mut parser = ValueParser::new(bytes, offset, max_depth);
    let value = parser.parse_value(0).map_err(|error| SyntaxIssue {
        code: "pdf.object.invalid",
        message: error.message,
        offset: error.offset,
        object: None,
    })?;
    Ok((value, parser.position()))
}

pub(crate) fn parse_dictionary_at(
    bytes: &[u8],
    offset: usize,
    max_depth: u16,
) -> Result<(BTreeMap<String, PdfValue>, usize), SyntaxIssue> {
    let (value, end) = parse_value_at(bytes, offset, max_depth)?;
    value
        .as_dictionary()
        .cloned()
        .map(|dictionary| (dictionary, end))
        .ok_or(SyntaxIssue {
            code: "pdf.dictionary.expected",
            message: "expected a PDF dictionary".into(),
            offset,
            object: None,
        })
}

struct ValueParser<'a> {
    bytes: &'a [u8],
    cursor: usize,
    max_depth: u16,
}

#[derive(Debug)]
struct ValueError {
    offset: usize,
    message: String,
}

impl<'a> ValueParser<'a> {
    fn new(bytes: &'a [u8], cursor: usize, max_depth: u16) -> Self {
        Self {
            bytes,
            cursor,
            max_depth,
        }
    }

    fn position(&self) -> usize {
        self.cursor
    }

    fn skip_space(&mut self) {
        skip_space_and_comments(self.bytes, &mut self.cursor);
    }

    fn consume_keyword(&mut self, keyword: &[u8]) -> bool {
        consume_token(self.bytes, &mut self.cursor, keyword)
    }

    fn parse_value(&mut self, depth: u16) -> Result<PdfValue, ValueError> {
        if depth > self.max_depth {
            return Err(self.error("PDF object nesting limit was exceeded"));
        }
        self.skip_space();
        let Some(byte) = self.bytes.get(self.cursor).copied() else {
            return Err(self.error("unexpected end of PDF object"));
        };
        match byte {
            b'<' if self.bytes.get(self.cursor + 1) == Some(&b'<') => self.parse_dictionary(depth),
            b'<' => self.parse_hex_string(),
            b'[' => self.parse_array(depth),
            b'(' => self.parse_literal_string(),
            b'/' => self.parse_name().map(PdfValue::Name),
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.parse_number_or_reference(),
            _ => self.parse_keyword(),
        }
    }

    fn parse_dictionary(&mut self, depth: u16) -> Result<PdfValue, ValueError> {
        self.cursor += 2;
        let mut dictionary = BTreeMap::new();
        loop {
            self.skip_space();
            if self.bytes.get(self.cursor..self.cursor + 2) == Some(b">>") {
                self.cursor += 2;
                return Ok(PdfValue::Dictionary(dictionary));
            }
            if self.cursor >= self.bytes.len() {
                return Err(self.error("unterminated PDF dictionary"));
            }
            if self.bytes[self.cursor] != b'/' {
                return Err(self.error("PDF dictionary key is not a name"));
            }
            let key = self.parse_name()?;
            let value = self.parse_value(depth.saturating_add(1))?;
            dictionary.insert(key, value);
        }
    }

    fn parse_array(&mut self, depth: u16) -> Result<PdfValue, ValueError> {
        self.cursor += 1;
        let mut values = Vec::new();
        loop {
            self.skip_space();
            match self.bytes.get(self.cursor) {
                Some(b']') => {
                    self.cursor += 1;
                    return Ok(PdfValue::Array(values));
                }
                None => return Err(self.error("unterminated PDF array")),
                _ => values.push(self.parse_value(depth.saturating_add(1))?),
            }
        }
    }

    fn parse_name(&mut self) -> Result<String, ValueError> {
        if self.bytes.get(self.cursor) != Some(&b'/') {
            return Err(self.error("expected PDF name"));
        }
        self.cursor += 1;
        let mut output = Vec::new();
        while let Some(&byte) = self.bytes.get(self.cursor) {
            if is_space(byte) || is_delimiter(byte) {
                break;
            }
            if byte == b'#'
                && let (Some(high), Some(low)) = (
                    self.bytes
                        .get(self.cursor + 1)
                        .and_then(|value| hex(*value)),
                    self.bytes
                        .get(self.cursor + 2)
                        .and_then(|value| hex(*value)),
                )
            {
                output.push((high << 4) | low);
                self.cursor += 3;
            } else {
                output.push(byte);
                self.cursor += 1;
            }
        }
        Ok(String::from_utf8_lossy(&output).into_owned())
    }

    fn parse_hex_string(&mut self) -> Result<PdfValue, ValueError> {
        self.cursor += 1;
        let mut nibbles = Vec::new();
        while let Some(&byte) = self.bytes.get(self.cursor) {
            self.cursor += 1;
            if byte == b'>' {
                return Ok(PdfValue::String(pdf_string_from_hex(nibbles)));
            }
            if is_space(byte) {
                continue;
            }
            let Some(value) = hex(byte) else {
                return Err(self.error("invalid hexadecimal PDF string"));
            };
            nibbles.push(value);
        }
        Err(self.error("unterminated hexadecimal PDF string"))
    }

    fn parse_literal_string(&mut self) -> Result<PdfValue, ValueError> {
        self.cursor += 1;
        let mut output = Vec::new();
        let mut nesting = 1u32;
        while let Some(&byte) = self.bytes.get(self.cursor) {
            self.cursor += 1;
            match byte {
                b'(' => {
                    nesting += 1;
                    output.push(byte);
                }
                b')' => {
                    nesting -= 1;
                    if nesting == 0 {
                        return Ok(PdfValue::String(pdf_string(output)));
                    }
                    output.push(byte);
                }
                b'\\' => self.parse_string_escape(&mut output),
                _ => output.push(byte),
            }
        }
        Err(self.error("unterminated literal PDF string"))
    }

    fn parse_string_escape(&mut self, output: &mut Vec<u8>) {
        let Some(&byte) = self.bytes.get(self.cursor) else {
            return;
        };
        self.cursor += 1;
        match byte {
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'b' => output.push(8),
            b'f' => output.push(12),
            b'(' | b')' | b'\\' => output.push(byte),
            b'\r' => {
                if self.bytes.get(self.cursor) == Some(&b'\n') {
                    self.cursor += 1;
                }
            }
            b'\n' => {}
            b'0'..=b'7' => {
                let mut value = u32::from(byte - b'0');
                for _ in 0..2 {
                    let Some(&next) = self.bytes.get(self.cursor) else {
                        break;
                    };
                    if !(b'0'..=b'7').contains(&next) {
                        break;
                    }
                    value = (value << 3) | u32::from(next - b'0');
                    self.cursor += 1;
                }
                output.push(value as u8);
            }
            _ => output.push(byte),
        }
    }

    fn parse_number_or_reference(&mut self) -> Result<PdfValue, ValueError> {
        let first_start = self.cursor;
        let first_token = self.number_token()?;
        let first_value =
            parse_number(first_token).ok_or_else(|| self.error("invalid PDF number"))?;
        let after_first = self.cursor;
        if let PdfValue::Integer(object_number) = first_value {
            self.skip_space();
            let _generation_start = self.cursor;
            if let Ok(second_token) = self.number_token()
                && let Some(PdfValue::Integer(generation)) = parse_number(second_token)
            {
                self.skip_space();
                if self.consume_keyword(b"R")
                    && object_number >= 0
                    && generation >= 0
                    && let (Ok(object_number), Ok(generation)) =
                        (u32::try_from(object_number), u16::try_from(generation))
                {
                    return Ok(PdfValue::Reference(PdfReference {
                        object_number,
                        generation,
                    }));
                }
            }
            self.cursor = after_first;
            return Ok(PdfValue::Integer(object_number));
        }
        self.cursor = after_first.max(first_start);
        Ok(first_value)
    }

    fn number_token(&mut self) -> Result<&'a [u8], ValueError> {
        let start = self.cursor;
        if matches!(self.bytes.get(self.cursor), Some(b'+') | Some(b'-')) {
            self.cursor += 1;
        }
        let mut digits = 0usize;
        while matches!(self.bytes.get(self.cursor), Some(b'0'..=b'9')) {
            self.cursor += 1;
            digits += 1;
        }
        if self.bytes.get(self.cursor) == Some(&b'.') {
            self.cursor += 1;
            while matches!(self.bytes.get(self.cursor), Some(b'0'..=b'9')) {
                self.cursor += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            self.cursor = start;
            return Err(self.error("expected PDF number"));
        }
        Ok(&self.bytes[start..self.cursor])
    }

    fn parse_keyword(&mut self) -> Result<PdfValue, ValueError> {
        let start = self.cursor;
        while let Some(&byte) = self.bytes.get(self.cursor) {
            if is_space(byte) || is_delimiter(byte) {
                break;
            }
            self.cursor += 1;
        }
        if start == self.cursor {
            return Err(self.error("unexpected PDF delimiter"));
        }
        let keyword = String::from_utf8_lossy(&self.bytes[start..self.cursor]);
        Ok(match keyword.as_ref() {
            "null" => PdfValue::Null,
            "true" => PdfValue::Boolean(true),
            "false" => PdfValue::Boolean(false),
            _ => PdfValue::Keyword(keyword.into_owned()),
        })
    }

    fn error(&self, message: impl Into<String>) -> ValueError {
        ValueError {
            offset: self.cursor,
            message: message.into(),
        }
    }
}

fn parse_number(token: &[u8]) -> Option<PdfValue> {
    let text = std::str::from_utf8(token).ok()?;
    if text.contains('.') {
        text.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(PdfValue::Real)
    } else {
        text.parse::<i64>().ok().map(PdfValue::Integer)
    }
}

fn pdf_string_from_hex(mut nibbles: Vec<u8>) -> PdfString {
    if nibbles.len() % 2 == 1 {
        nibbles.push(0);
    }
    let bytes = nibbles
        .chunks_exact(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect();
    pdf_string(bytes)
}

fn pdf_string(bytes: Vec<u8>) -> PdfString {
    let raw_hex = bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    if bytes.starts_with(&[0xfe, 0xff]) {
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        PdfString {
            raw_hex,
            text: String::from_utf16_lossy(&units),
            encoding: PdfStringEncoding::Utf16BigEndian,
        }
    } else {
        let text = bytes
            .iter()
            .map(|byte| char::from(*byte))
            .collect::<String>();
        let encoding = if bytes.iter().all(|byte| *byte >= 9) {
            PdfStringEncoding::PdfDoc
        } else {
            PdfStringEncoding::Binary
        };
        PdfString {
            raw_hex,
            text,
            encoding,
        }
    }
}

fn locate_stream_end(bytes: &[u8], start: usize, declared: Option<usize>) -> (usize, usize, bool) {
    if let Some(length) = declared
        && let Some(end) = start.checked_add(length)
        && end <= bytes.len()
    {
        let mut marker = end;
        while matches!(bytes.get(marker), Some(b'\r') | Some(b'\n')) {
            marker += 1;
        }
        if bytes.get(marker..marker + 9) == Some(b"endstream") {
            return (end, marker + 9, false);
        }
    }
    if let Some(relative) = find_token(&bytes[start..], b"endstream") {
        let mut end = start + relative;
        while end > start && matches!(bytes[end - 1], b'\r' | b'\n') {
            end -= 1;
        }
        (end, start + relative + 9, true)
    } else {
        (bytes.len(), bytes.len(), true)
    }
}

fn consume_stream_eol(bytes: &[u8], cursor: &mut usize) {
    if bytes.get(*cursor) == Some(&b'\r') {
        *cursor += 1;
    }
    if bytes.get(*cursor) == Some(&b'\n') {
        *cursor += 1;
    }
}

pub(crate) fn skip_space_and_comments(bytes: &[u8], cursor: &mut usize) {
    loop {
        while bytes.get(*cursor).is_some_and(|byte| is_space(*byte)) {
            *cursor += 1;
        }
        if bytes.get(*cursor) != Some(&b'%') {
            break;
        }
        while let Some(&byte) = bytes.get(*cursor) {
            *cursor += 1;
            if matches!(byte, b'\r' | b'\n') {
                break;
            }
        }
    }
}

pub(crate) fn find_token(bytes: &[u8], token: &[u8]) -> Option<usize> {
    if token.is_empty() {
        return Some(0);
    }
    bytes
        .windows(token.len())
        .position(|window| window == token)
}

pub(crate) fn parse_unsigned(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let start = *cursor;
    let mut value = 0u64;
    while let Some(byte @ b'0'..=b'9') = bytes.get(*cursor).copied() {
        value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
        *cursor += 1;
    }
    (*cursor > start).then_some(value)
}

fn skip_required_space(bytes: &[u8], cursor: &mut usize) -> bool {
    let before = *cursor;
    skip_space_and_comments(bytes, cursor);
    *cursor > before
}

pub(crate) fn consume_token(bytes: &[u8], cursor: &mut usize, token: &[u8]) -> bool {
    if bytes.get(*cursor..cursor.saturating_add(token.len())) != Some(token) {
        return false;
    }
    let after = cursor.saturating_add(token.len());
    if bytes.get(after).is_some_and(|byte| !is_boundary(*byte)) {
        return false;
    }
    *cursor = after;
    true
}

fn is_boundary(byte: u8) -> bool {
    is_space(byte) || is_delimiter(byte)
}
fn is_space(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}
fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn object_stream_locator(
    object: PdfReference,
    container: PdfReference,
    start: usize,
    end: usize,
) -> PdfObjectLocator {
    PdfObjectLocator {
        stable_id: format!("pdf-object-{}-{}", object.object_number, object.generation),
        object,
        location: PdfObjectLocation::ObjectStream {
            container,
            decoded_byte_start: start as u64,
            decoded_byte_end: end as u64,
        },
        key_path: Vec::new(),
    }
}
