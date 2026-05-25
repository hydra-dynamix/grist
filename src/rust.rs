use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, LineIndex, ParserInfo, SchemaVersion, SourceInfo,
    SourceRange,
};
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustFile {
    pub schema_version: String,
    pub symbols: Vec<RustSymbol>,
    pub imports: Vec<RustImport>,
    pub parse_errors: Vec<RustParseError>,
    pub detail: Option<RustSyntaxDetail>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustSymbol {
    pub id: String,
    pub name: String,
    pub kind: RustSymbolKind,
    pub path: Option<String>,
    pub range: SourceRange,
    pub visibility: RustVisibility,
    pub parent: Option<String>,
    pub attributes: Vec<String>,
    pub doc: Option<String>,
    pub syntax: Option<RustSyntaxSummary>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RustSymbolKind {
    Module,
    Function,
    Method,
    Struct,
    Enum,
    Union,
    Trait,
    Impl,
    Const,
    Static,
    TypeAlias,
    MacroDefinition,
    MacroInvocation,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RustVisibility {
    Public,
    Crate,
    Restricted,
    Private,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustImport {
    pub id: String,
    pub path: String,
    pub alias: Option<String>,
    pub range: SourceRange,
    pub visibility: RustVisibility,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustParseError {
    pub range: SourceRange,
    pub node_kind: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustSyntaxSummary {
    pub node_kind: String,
    pub named_child_count: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustSyntaxDetail {
    pub root_kind: String,
    pub root_named_child_count: usize,
    pub node_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RustDetailMode {
    #[default]
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[derive(Debug, Clone, Default)]
pub struct RustIngestOptions {
    pub detail: RustDetailMode,
}

pub type RustEnvelope = Envelope<RustFile>;

pub fn parse_rust(text: &str, source: SourceInfo, options: &RustIngestOptions) -> RustEnvelope {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE;
    parser
        .set_language(&language.into())
        .expect("tree-sitter Rust language should load");
    let line_index = LineIndex::new(text);
    let Some(tree) = parser.parse(text, None) else {
        return Envelope::new(
            ArtifactKind::RustCode,
            source,
            ParserInfo::new("tree-sitter-rust"),
            SchemaVersion::RUST_CODE_V1,
            RustFile {
                schema_version: SchemaVersion::RUST_CODE_V1.to_string(),
                symbols: Vec::new(),
                imports: Vec::new(),
                parse_errors: Vec::new(),
                detail: None,
            },
        )
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(vec![Diagnostic::error(
            "tree-sitter-rust",
            "rust.parse.none",
            "tree-sitter returned no parse tree",
        )]);
    };
    let root = tree.root_node();
    let mut collector = RustCollector {
        text,
        line_index: &line_index,
        symbols: Vec::new(),
        imports: Vec::new(),
        errors: Vec::new(),
        diagnostics: Vec::new(),
        detail: options.detail,
    };
    collector.walk(root, None, Vec::new(), Vec::new());
    let detail = match options.detail {
        RustDetailMode::SyntaxDebug => Some(RustSyntaxDetail {
            root_kind: root.kind().to_string(),
            root_named_child_count: root.named_child_count(),
            node_count: count_nodes(root),
        }),
        _ => None,
    };
    let parse_errors = collector.errors;
    let mut diagnostics = collector.diagnostics;
    for err in &parse_errors {
        diagnostics.push(
            Diagnostic::error(
                "tree-sitter-rust",
                "rust.parse.error_node",
                format!("Rust parse contained {} node", err.node_kind),
            )
            .with_range(err.range.clone())
            .partial(),
        );
    }
    Envelope::new(
        ArtifactKind::RustCode,
        source,
        ParserInfo::new("tree-sitter-rust"),
        SchemaVersion::RUST_CODE_V1,
        RustFile {
            schema_version: SchemaVersion::RUST_CODE_V1.to_string(),
            symbols: collector.symbols,
            imports: collector.imports,
            parse_errors,
            detail,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(diagnostics)
}

struct RustCollector<'a, 'b> {
    text: &'a str,
    line_index: &'b LineIndex,
    symbols: Vec<RustSymbol>,
    imports: Vec<RustImport>,
    errors: Vec<RustParseError>,
    diagnostics: Vec<Diagnostic>,
    detail: RustDetailMode,
}

impl RustCollector<'_, '_> {
    fn walk(
        &mut self,
        node: Node,
        parent: Option<String>,
        inherited_attrs: Vec<String>,
        inherited_docs: Vec<String>,
    ) {
        let mut attrs = inherited_attrs;
        let mut docs = inherited_docs;
        if node.kind() == "ERROR" || node.is_missing() {
            self.errors.push(RustParseError {
                range: self.range(node),
                node_kind: node.kind().to_string(),
            });
        }
        let kind = node.kind();
        if matches!(kind, "attribute_item" | "inner_attribute_item") {
            attrs.push(self.source(node).trim().to_string());
        }
        if kind == "line_comment" && self.source(node).trim_start().starts_with("///") {
            docs.push(self.source(node).trim().to_string());
        }
        if let Some(symbol_kind) = symbol_kind(kind, &parent) {
            let name = self
                .name_for(node)
                .unwrap_or_else(|| self.fallback_name(kind, node));
            let id = format!(
                "rust-symbol-{}-{}",
                self.symbols.len(),
                name.replace("::", "_")
            );
            let syntax = (self.detail == RustDetailMode::SemanticWithSyntax
                || self.detail == RustDetailMode::SyntaxDebug)
                .then(|| RustSyntaxSummary {
                    node_kind: kind.to_string(),
                    named_child_count: node.named_child_count(),
                });
            let symbol = RustSymbol {
                id: id.clone(),
                name,
                kind: symbol_kind,
                path: None,
                range: self.range(node),
                visibility: self.visibility_for(node),
                parent: parent.clone(),
                attributes: self.attributes_for(node, &attrs),
                doc: (!docs.is_empty()).then(|| docs.join("\n")),
                syntax,
            };
            self.symbols.push(symbol);
            self.walk_children(node, Some(id));
            return;
        }
        if kind == "use_declaration" {
            self.imports.push(RustImport {
                id: format!("rust-import-{}", self.imports.len()),
                path: normalize_ws(
                    self.source(node)
                        .trim()
                        .trim_start_matches("pub")
                        .trim()
                        .trim_start_matches("use")
                        .trim()
                        .trim_end_matches(';'),
                ),
                alias: alias_for_use(self.source(node)),
                range: self.range(node),
                visibility: self.visibility_for(node),
            });
        }
        self.walk_children_with_context(node, parent, attrs, docs);
    }

    fn walk_children(&mut self, node: Node, parent: Option<String>) {
        self.walk_children_with_context(node, parent, Vec::new(), Vec::new());
    }

    fn walk_children_with_context(
        &mut self,
        node: Node,
        parent: Option<String>,
        attrs: Vec<String>,
        docs: Vec<String>,
    ) {
        let mut cursor = node.walk();
        let mut pending_attrs = attrs;
        let mut pending_docs = docs;
        for child in node.children(&mut cursor) {
            match child.kind() {
                "attribute_item" | "inner_attribute_item" => {
                    pending_attrs.push(self.source(child).trim().to_string());
                    continue;
                }
                "line_comment" if self.source(child).trim_start().starts_with("///") => {
                    pending_docs.push(self.source(child).trim().to_string());
                    continue;
                }
                _ => {}
            }
            self.walk(
                child,
                parent.clone(),
                pending_attrs.clone(),
                pending_docs.clone(),
            );
            if symbol_kind(child.kind(), &parent).is_some() {
                pending_attrs.clear();
                pending_docs.clear();
            }
        }
    }

    fn source(&self, node: Node) -> &str {
        &self.text[node.start_byte()..node.end_byte()]
    }

    fn range(&self, node: Node) -> SourceRange {
        SourceRange::new(node.start_byte(), node.end_byte(), self.line_index)
    }

    fn name_for(&self, node: Node) -> Option<String> {
        node.child_by_field_name("name")
            .map(|n| self.source(n).to_string())
            .or_else(|| {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|child| child.kind() == "identifier" || child.kind() == "type_identifier")
                    .map(|child| self.source(child).to_string())
            })
    }

    fn fallback_name(&self, kind: &str, node: Node) -> String {
        match kind {
            "impl_item" => self
                .source(node)
                .lines()
                .next()
                .unwrap_or("impl")
                .trim()
                .trim_end_matches('{')
                .to_string(),
            "macro_invocation" => self
                .source(node)
                .split('!')
                .next()
                .unwrap_or("macro")
                .trim()
                .to_string(),
            _ => kind.to_string(),
        }
    }

    fn visibility_for(&self, node: Node) -> RustVisibility {
        let src = self.source(node).trim_start();
        if src.starts_with("pub(crate)") {
            RustVisibility::Crate
        } else if src.starts_with("pub(super)") || src.starts_with("pub(in ") {
            RustVisibility::Restricted
        } else if src.starts_with("pub") {
            RustVisibility::Public
        } else {
            RustVisibility::Private
        }
    }

    fn attributes_for(&self, node: Node, inherited: &[String]) -> Vec<String> {
        let mut attrs = inherited.to_vec();
        let src = self.source(node);
        for line in src.lines().take(12) {
            let trimmed = line.trim();
            if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
                attrs.push(trimmed.to_string());
            }
        }
        attrs.sort();
        attrs.dedup();
        attrs
    }
}

fn symbol_kind(kind: &str, parent: &Option<String>) -> Option<RustSymbolKind> {
    match kind {
        "mod_item" => Some(RustSymbolKind::Module),
        "function_item" => Some(if parent.is_some() {
            RustSymbolKind::Method
        } else {
            RustSymbolKind::Function
        }),
        "struct_item" => Some(RustSymbolKind::Struct),
        "enum_item" => Some(RustSymbolKind::Enum),
        "union_item" => Some(RustSymbolKind::Union),
        "trait_item" => Some(RustSymbolKind::Trait),
        "impl_item" => Some(RustSymbolKind::Impl),
        "const_item" => Some(RustSymbolKind::Const),
        "static_item" => Some(RustSymbolKind::Static),
        "type_item" => Some(RustSymbolKind::TypeAlias),
        "macro_definition" => Some(RustSymbolKind::MacroDefinition),
        "macro_invocation" => Some(RustSymbolKind::MacroInvocation),
        _ => None,
    }
}

fn alias_for_use(src: &str) -> Option<String> {
    src.split(" as ")
        .nth(1)
        .map(|rest| rest.trim().trim_end_matches(';').to_string())
}

fn normalize_ws(src: &str) -> String {
    src.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn count_nodes(node: Node) -> usize {
    let mut count = 1;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_nodes(child);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_symbols_and_imports() {
        let src = "use std::{fmt, io as sio};\n/// docs\npub(crate) struct Thing;\nimpl Thing { pub fn run(&self) {} }\n#[test]\nfn it_works() {}";
        let report = parse_rust(
            src,
            SourceInfo::stdin("lib.rs"),
            &RustIngestOptions {
                detail: RustDetailMode::SemanticWithSyntax,
            },
        );
        assert!(report.payload.symbols.iter().any(|s| s.name == "Thing"));
        assert!(report.payload.symbols.iter().any(|s| s.name == "run"));
        assert_eq!(report.payload.imports.len(), 1);
    }
}
