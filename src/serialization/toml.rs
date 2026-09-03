use super::json::locator;
use super::model::{
    StructuredEntry, StructuredScalar, StructuredValue, StructuredValueKind, pointer_escape,
};
use crate::core::{LineIndex, SourceRange};
use std::ops::Range;

#[derive(Debug, Clone)]
pub(crate) struct TomlParseFailure {
    pub range: Range<usize>,
    pub message: String,
}

pub(crate) struct ParsedToml {
    pub value: StructuredValue,
    pub max_depth: usize,
    pub node_count: usize,
}

pub(crate) fn parse(text: &str) -> Result<ParsedToml, TomlParseFailure> {
    let document = toml_edit::ImDocument::<String>::parse(text.to_string()).map_err(|error| {
        TomlParseFailure {
            range: error.span().unwrap_or(0..text.len().min(1)),
            message: error.to_string(),
        }
    })?;
    let mut builder = Builder {
        text,
        lines: LineIndex::new(text),
        max_depth: 0,
        node_count: 0,
    };
    let value = builder.table(document.as_table(), "", 0..text.len(), 1);
    Ok(ParsedToml {
        value,
        max_depth: builder.max_depth,
        node_count: builder.node_count,
    })
}

struct Builder<'a> {
    text: &'a str,
    lines: LineIndex,
    max_depth: usize,
    node_count: usize,
}

impl Builder<'_> {
    fn table(
        &mut self,
        table: &toml_edit::Table,
        path: &str,
        fallback: Range<usize>,
        depth: usize,
    ) -> StructuredValue {
        self.observe(depth);
        let span = table.span().unwrap_or(fallback);
        let mut entries = Vec::new();
        for (name, item) in table.iter() {
            let Some(key) = table.key(name) else { continue };
            let key_span = key.span().unwrap_or_else(|| span.clone());
            let child_path = format!("{path}/{}", pointer_escape(name));
            let key_node = self.scalar_node(
                key_span.clone(),
                &child_path,
                StructuredValueKind::String,
                StructuredScalar::String { value: name.into() },
            );
            let value = self.item(item, &child_path, key_span.end..span.end, depth + 1);
            entries.push(StructuredEntry {
                index: entries.len(),
                key: Box::new(key_node),
                value: Box::new(value),
                key_text: Some(name.into()),
                duplicate_ordinal: 1,
            });
        }
        self.container(span, path, StructuredValueKind::Object, entries, vec![])
    }

    fn item(
        &mut self,
        item: &toml_edit::Item,
        path: &str,
        fallback: Range<usize>,
        depth: usize,
    ) -> StructuredValue {
        match item {
            toml_edit::Item::None => self.unknown(fallback, path, "empty TOML item"),
            toml_edit::Item::Value(value) => self.value(value, path, fallback, depth),
            toml_edit::Item::Table(table) => self.table(table, path, fallback, depth),
            toml_edit::Item::ArrayOfTables(tables) => {
                self.observe(depth);
                let span = tables.span().unwrap_or(fallback.clone());
                let items = tables
                    .iter()
                    .enumerate()
                    .map(|(index, table)| {
                        self.table(table, &format!("{path}/{index}"), span.clone(), depth + 1)
                    })
                    .collect();
                self.container(span, path, StructuredValueKind::Array, vec![], items)
            }
        }
    }

    fn value(
        &mut self,
        value: &toml_edit::Value,
        path: &str,
        fallback: Range<usize>,
        depth: usize,
    ) -> StructuredValue {
        self.observe(depth);
        let span = value.span().unwrap_or(fallback);
        match value {
            toml_edit::Value::String(value) => self.scalar_node(
                span,
                path,
                StructuredValueKind::String,
                StructuredScalar::String {
                    value: value.value().clone(),
                },
            ),
            toml_edit::Value::Integer(value) => self.scalar_node(
                span,
                path,
                StructuredValueKind::Integer,
                StructuredScalar::Integer {
                    canonical: value.value().to_string(),
                },
            ),
            toml_edit::Value::Float(value) => self.scalar_node(
                span,
                path,
                StructuredValueKind::Float,
                StructuredScalar::Float {
                    canonical: value.value().to_string(),
                    finite: value.value().is_finite(),
                },
            ),
            toml_edit::Value::Boolean(value) => self.scalar_node(
                span,
                path,
                StructuredValueKind::Boolean,
                StructuredScalar::Boolean {
                    value: *value.value(),
                },
            ),
            toml_edit::Value::Datetime(value) => {
                let text = value.value().to_string();
                let (kind, scalar) = if text.contains(['T', 't', ' ']) {
                    (
                        StructuredValueKind::DateTime,
                        StructuredScalar::DateTime { value: text },
                    )
                } else if text.contains(':') {
                    (
                        StructuredValueKind::Time,
                        StructuredScalar::Time { value: text },
                    )
                } else {
                    (
                        StructuredValueKind::Date,
                        StructuredScalar::Date { value: text },
                    )
                };
                self.scalar_node(span, path, kind, scalar)
            }
            toml_edit::Value::Array(array) => {
                let items = array
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.value(value, &format!("{path}/{index}"), span.clone(), depth + 1)
                    })
                    .collect();
                self.container(span, path, StructuredValueKind::Array, vec![], items)
            }
            toml_edit::Value::InlineTable(table) => {
                let mut entries = Vec::new();
                for (name, value) in table.iter() {
                    let key_span = table
                        .key(name)
                        .and_then(toml_edit::Key::span)
                        .unwrap_or_else(|| span.clone());
                    let child_path = format!("{path}/{}", pointer_escape(name));
                    let key = self.scalar_node(
                        key_span,
                        &child_path,
                        StructuredValueKind::String,
                        StructuredScalar::String { value: name.into() },
                    );
                    let value = self.value(value, &child_path, span.clone(), depth + 1);
                    entries.push(StructuredEntry {
                        index: entries.len(),
                        key: Box::new(key),
                        value: Box::new(value),
                        key_text: Some(name.into()),
                        duplicate_ordinal: 1,
                    });
                }
                self.container(span, path, StructuredValueKind::Object, entries, vec![])
            }
        }
    }

    fn scalar_node(
        &mut self,
        span: Range<usize>,
        path: &str,
        kind: StructuredValueKind,
        scalar: StructuredScalar,
    ) -> StructuredValue {
        self.node(span, path, kind, Some(scalar), vec![], vec![], false)
    }

    fn container(
        &mut self,
        span: Range<usize>,
        path: &str,
        kind: StructuredValueKind,
        entries: Vec<StructuredEntry>,
        items: Vec<StructuredValue>,
    ) -> StructuredValue {
        self.node(span, path, kind, None, entries, items, false)
    }

    fn unknown(&mut self, span: Range<usize>, path: &str, reason: &str) -> StructuredValue {
        self.node(
            span,
            path,
            StructuredValueKind::RawUnknown,
            Some(StructuredScalar::String {
                value: reason.into(),
            }),
            vec![],
            vec![],
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &mut self,
        span: Range<usize>,
        path: &str,
        kind: StructuredValueKind,
        scalar: Option<StructuredScalar>,
        entries: Vec<StructuredEntry>,
        items: Vec<StructuredValue>,
        recovered: bool,
    ) -> StructuredValue {
        self.node_count += 1;
        let start = span.start.min(self.text.len());
        let end = span.end.min(self.text.len()).max(start);
        let range = SourceRange::new(start, end, &self.lines);
        StructuredValue {
            id: format!("toml:{}@{start}", if path.is_empty() { "/" } else { path }),
            kind,
            path: path.into(),
            range: range.clone(),
            locator: locator(range, path, None),
            raw: self.text[start..end].into(),
            scalar,
            entries,
            items,
            anchor: None,
            tag: None,
            alias: None,
            alias_target_id: None,
            recovered,
        }
    }

    fn observe(&mut self, depth: usize) {
        self.max_depth = self.max_depth.max(depth);
    }
}
