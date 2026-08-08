use super::model::{
    DuplicateKey, DuplicateKeyDisposition, StructuredEntry, StructuredScalar, StructuredValue,
    StructuredValueKind, pointer_escape,
};
use crate::core::{
    IndexBase, IndexRange, LineIndex, LocationComponent, SourceLocator, SourceRange,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct JsonParseFailure {
    pub offset: usize,
    pub message: String,
}

pub(crate) struct ParsedJson {
    pub value: StructuredValue,
    pub duplicate_keys: Vec<DuplicateKey>,
    pub max_depth: usize,
    pub node_count: usize,
}

pub(crate) fn parse(
    full_text: &str,
    start: usize,
    end: usize,
    record: Option<usize>,
) -> Result<ParsedJson, JsonParseFailure> {
    let mut parser = Parser {
        text: full_text,
        end,
        cursor: start,
        lines: LineIndex::new(full_text),
        record,
        duplicates: Vec::new(),
        max_depth: 0,
        node_count: 0,
    };
    parser.ws();
    let value = parser.value("", 1)?;
    parser.ws();
    if parser.cursor != end {
        return Err(parser.error("trailing content after the JSON value"));
    }
    Ok(ParsedJson {
        value,
        duplicate_keys: parser.duplicates,
        max_depth: parser.max_depth,
        node_count: parser.node_count,
    })
}

struct Parser<'a> {
    text: &'a str,
    end: usize,
    cursor: usize,
    lines: LineIndex,
    record: Option<usize>,
    duplicates: Vec<DuplicateKey>,
    max_depth: usize,
    node_count: usize,
}

impl Parser<'_> {
    fn value(&mut self, path: &str, depth: usize) -> Result<StructuredValue, JsonParseFailure> {
        self.ws();
        self.max_depth = self.max_depth.max(depth);
        self.node_count += 1;
        match self.byte() {
            Some(b'{') => self.object(path, depth),
            Some(b'[') => self.array(path, depth),
            Some(b'"') => {
                let start = self.cursor;
                let value = self.string()?;
                Ok(self.scalar(
                    start,
                    self.cursor,
                    path,
                    StructuredValueKind::String,
                    StructuredScalar::String { value },
                ))
            }
            Some(b't') => self.keyword(path, "true", StructuredScalar::Boolean { value: true }),
            Some(b'f') => self.keyword(path, "false", StructuredScalar::Boolean { value: false }),
            Some(b'n') => self.keyword(path, "null", StructuredScalar::Null),
            Some(b'-' | b'0'..=b'9') => self.number(path),
            Some(_) => Err(self.error("expected a JSON value")),
            None => Err(self.error("unexpected end of input while reading a JSON value")),
        }
    }

    fn object(&mut self, path: &str, depth: usize) -> Result<StructuredValue, JsonParseFailure> {
        let start = self.cursor;
        self.cursor += 1;
        self.ws();
        let mut entries = Vec::new();
        let mut seen = BTreeMap::<String, (usize, SourceLocator)>::new();
        if self.consume(b'}') {
            return Ok(self.container(
                start,
                self.cursor,
                path,
                StructuredValueKind::Object,
                entries,
                vec![],
            ));
        }
        loop {
            self.ws();
            if self.byte() != Some(b'"') {
                return Err(self.error("JSON object keys must be strings"));
            }
            let key_start = self.cursor;
            let key_text = self.string()?;
            let key_end = self.cursor;
            let child_path = format!("{path}/{}", pointer_escape(&key_text));
            let key = self.scalar(
                key_start,
                key_end,
                &child_path,
                StructuredValueKind::String,
                StructuredScalar::String {
                    value: key_text.clone(),
                },
            );
            self.ws();
            if !self.consume(b':') {
                return Err(self.error("expected `:` after JSON object key"));
            }
            let value = self.value(&child_path, depth + 1)?;
            let occurrence = seen.get(&key_text).map_or(1, |(count, _)| count + 1);
            if let Some((_, first_locator)) = seen.get(&key_text) {
                self.duplicates.push(DuplicateKey {
                    path: path.to_string(),
                    key: key_text.clone(),
                    occurrence,
                    first_locator: first_locator.clone(),
                    duplicate_locator: key.locator.clone(),
                    disposition: DuplicateKeyDisposition::Preserved,
                });
            }
            seen.insert(key_text.clone(), (occurrence, key.locator.clone()));
            entries.push(StructuredEntry {
                index: entries.len(),
                key: Box::new(key),
                value: Box::new(value),
                key_text: Some(key_text),
                duplicate_ordinal: occurrence,
            });
            self.ws();
            if self.consume(b'}') {
                break;
            }
            if !self.consume(b',') {
                return Err(self.error("expected `,` or `}` in JSON object"));
            }
        }
        Ok(self.container(
            start,
            self.cursor,
            path,
            StructuredValueKind::Object,
            entries,
            vec![],
        ))
    }

    fn array(&mut self, path: &str, depth: usize) -> Result<StructuredValue, JsonParseFailure> {
        let start = self.cursor;
        self.cursor += 1;
        self.ws();
        let mut items = Vec::new();
        if self.consume(b']') {
            return Ok(self.container(
                start,
                self.cursor,
                path,
                StructuredValueKind::Array,
                vec![],
                items,
            ));
        }
        loop {
            let item_path = format!("{path}/{}", items.len());
            items.push(self.value(&item_path, depth + 1)?);
            self.ws();
            if self.consume(b']') {
                break;
            }
            if !self.consume(b',') {
                return Err(self.error("expected `,` or `]` in JSON array"));
            }
        }
        Ok(self.container(
            start,
            self.cursor,
            path,
            StructuredValueKind::Array,
            vec![],
            items,
        ))
    }

    fn string(&mut self) -> Result<String, JsonParseFailure> {
        let start = self.cursor;
        self.cursor += 1;
        let mut escaped = false;
        while let Some(byte) = self.byte() {
            if byte < 0x20 {
                return Err(self.error("unescaped control character in JSON string"));
            }
            self.cursor += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return serde_json::from_str(&self.text[start..self.cursor]).map_err(|error| {
                    JsonParseFailure {
                        offset: start,
                        message: error.to_string(),
                    }
                });
            }
        }
        Err(JsonParseFailure {
            offset: start,
            message: "unterminated JSON string".into(),
        })
    }

    fn keyword(
        &mut self,
        path: &str,
        keyword: &str,
        scalar: StructuredScalar,
    ) -> Result<StructuredValue, JsonParseFailure> {
        let start = self.cursor;
        if !self.text[self.cursor..self.end].starts_with(keyword) {
            return Err(self.error(format!("invalid JSON token; expected `{keyword}`")));
        }
        self.cursor += keyword.len();
        let kind = match scalar {
            StructuredScalar::Null => StructuredValueKind::Null,
            StructuredScalar::Boolean { .. } => StructuredValueKind::Boolean,
            _ => unreachable!(),
        };
        Ok(self.scalar(start, self.cursor, path, kind, scalar))
    }

    fn number(&mut self, path: &str) -> Result<StructuredValue, JsonParseFailure> {
        let start = self.cursor;
        self.consume(b'-');
        match self.byte() {
            Some(b'0') => self.cursor += 1,
            Some(b'1'..=b'9') => {
                self.cursor += 1;
                while matches!(self.byte(), Some(b'0'..=b'9')) {
                    self.cursor += 1;
                }
            }
            _ => return Err(self.error("invalid JSON number")),
        }
        let mut float = false;
        if self.consume(b'.') {
            float = true;
            if !matches!(self.byte(), Some(b'0'..=b'9')) {
                return Err(self.error("JSON fraction requires at least one digit"));
            }
            while matches!(self.byte(), Some(b'0'..=b'9')) {
                self.cursor += 1;
            }
        }
        if matches!(self.byte(), Some(b'e' | b'E')) {
            float = true;
            self.cursor += 1;
            if matches!(self.byte(), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            if !matches!(self.byte(), Some(b'0'..=b'9')) {
                return Err(self.error("JSON exponent requires at least one digit"));
            }
            while matches!(self.byte(), Some(b'0'..=b'9')) {
                self.cursor += 1;
            }
        }
        let raw = &self.text[start..self.cursor];
        let (kind, scalar) = if float {
            (
                StructuredValueKind::Float,
                StructuredScalar::Float {
                    canonical: raw.to_string(),
                    finite: true,
                },
            )
        } else {
            (
                StructuredValueKind::Integer,
                StructuredScalar::Integer {
                    canonical: raw.to_string(),
                },
            )
        };
        Ok(self.scalar(start, self.cursor, path, kind, scalar))
    }

    fn scalar(
        &self,
        start: usize,
        end: usize,
        path: &str,
        kind: StructuredValueKind,
        scalar: StructuredScalar,
    ) -> StructuredValue {
        self.node(start, end, path, kind, Some(scalar), vec![], vec![])
    }

    fn container(
        &self,
        start: usize,
        end: usize,
        path: &str,
        kind: StructuredValueKind,
        entries: Vec<StructuredEntry>,
        items: Vec<StructuredValue>,
    ) -> StructuredValue {
        self.node(start, end, path, kind, None, entries, items)
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &self,
        start: usize,
        end: usize,
        path: &str,
        kind: StructuredValueKind,
        scalar: Option<StructuredScalar>,
        entries: Vec<StructuredEntry>,
        items: Vec<StructuredValue>,
    ) -> StructuredValue {
        let range = SourceRange::new(start, end, &self.lines);
        let locator = locator(range.clone(), path, self.record);
        StructuredValue {
            id: format!("json:{}@{start}", if path.is_empty() { "/" } else { path }),
            kind,
            path: path.to_string(),
            range,
            locator,
            raw: self.text[start..end].to_string(),
            scalar,
            entries,
            items,
            anchor: None,
            tag: None,
            alias: None,
            alias_target_id: None,
            recovered: false,
        }
    }

    fn ws(&mut self) {
        while matches!(self.byte(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.cursor += 1;
        }
    }

    fn byte(&self) -> Option<u8> {
        (self.cursor < self.end).then(|| self.text.as_bytes()[self.cursor])
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.byte() == Some(expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn error(&self, message: impl Into<String>) -> JsonParseFailure {
        JsonParseFailure {
            offset: self.cursor,
            message: message.into(),
        }
    }
}

pub(crate) fn locator(range: SourceRange, pointer: &str, record: Option<usize>) -> SourceLocator {
    let mut locator = SourceLocator::exact(range).expect("source range is valid");
    if let Some(record) = record {
        locator = locator
            .nested(LocationComponent::RecordRange {
                collection: "records".into(),
                records: IndexRange::new(record as u64, record as u64 + 1, IndexBase::One)
                    .expect("record index is one based"),
                field: None,
            })
            .expect("record locator is valid");
    }
    locator
        .nested(LocationComponent::JsonPointer {
            pointer: pointer.to_string(),
        })
        .expect("JSON pointer is valid")
}
