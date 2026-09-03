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
    #[serde(default)]
    pub exports: Vec<RustExport>,
    #[serde(default)]
    pub assignments: Vec<RustAssignment>,
    #[serde(default)]
    pub returns: Vec<RustReturn>,
    #[serde(default)]
    pub calls: Vec<RustCall>,
    #[serde(default)]
    pub branches: Vec<RustBranch>,
    #[serde(default)]
    pub inheritances: Vec<RustInheritance>,
    #[serde(default)]
    pub tests: Vec<RustTest>,
    #[serde(default)]
    pub comments: Vec<RustComment>,
    #[serde(default)]
    pub syntax_nodes: Vec<RustSyntaxNode>,
    pub parse_errors: Vec<RustParseError>,
    pub detail: Option<RustSyntaxDetail>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustSymbol {
    pub id: String,
    pub name: String,
    pub kind: RustSymbolKind,
    pub language: String,
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
pub struct RustExport {
    pub id: String,
    pub name: String,
    pub kind: RustSymbolKind,
    pub range: SourceRange,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustAssignment {
    pub id: String,
    pub lhs: String,
    pub rhs: Option<String>,
    pub operator: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustReturn {
    pub id: String,
    pub expression: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustCall {
    pub id: String,
    pub target: String,
    pub arguments: Vec<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustBranch {
    pub id: String,
    pub kind: String,
    pub condition: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustInheritance {
    pub id: String,
    pub implementation: String,
    pub trait_name: Option<String>,
    pub target: String,
    pub range: SourceRange,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustTest {
    pub id: String,
    pub symbol_id: String,
    pub name: String,
    pub attributes: Vec<String>,
    pub range: SourceRange,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustComment {
    pub id: String,
    pub text: String,
    pub doc: bool,
    pub block: bool,
    pub range: SourceRange,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustSyntaxNode {
    pub id: String,
    pub kind: String,
    pub named: bool,
    pub error: bool,
    pub missing: bool,
    pub parent: Option<String>,
    pub raw: String,
    pub range: SourceRange,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustParseError {
    pub range: SourceRange,
    pub node_kind: String,
    #[serde(default)]
    pub raw: String,
    #[serde(default)]
    pub missing: bool,
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

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RustDetailMode {
    #[default]
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RustIngestOptions {
    pub detail: RustDetailMode,
}

impl crate::core::FormatOptions for RustIngestOptions {
    const FORMAT: &'static str = "rust";
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
        return Envelope::without_payload(
            crate::core::OperationKind::Parse,
            ArtifactKind::RustCode,
            crate::core::OperationStatus::Failed,
            source,
            ParserInfo::new("tree-sitter-rust"),
            crate::core::options_digest(options).expect("Rust options must serialize"),
            SchemaVersion::RUST_CODE_V1,
        )
        .expect("failed envelope status is valid")
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(vec![Diagnostic::error(
            "tree-sitter-rust",
            "rust.parse.none",
            "tree-sitter returned no parse tree",
        )]);
    };
    let root = tree.root_node();
    let source_path = source
        .path
        .clone()
        .or_else(|| Some(source.display_name.clone()));
    let mut collector = RustCollector {
        text,
        line_index: &line_index,
        symbols: Vec::new(),
        imports: Vec::new(),
        assignments: Vec::new(),
        returns: Vec::new(),
        calls: Vec::new(),
        branches: Vec::new(),
        inheritances: Vec::new(),
        errors: Vec::new(),
        diagnostics: Vec::new(),
        detail: options.detail,
        source_path,
    };
    collector.walk(root, None, Vec::new(), Vec::new());
    let comments = collect_comments(root, text, &line_index);
    let syntax_nodes = collect_syntax_nodes(root, text, &line_index, options.detail);
    let mut exports = collector
        .symbols
        .iter()
        .filter(|symbol| symbol.visibility != RustVisibility::Private)
        .enumerate()
        .map(|(index, symbol)| RustExport {
            id: format!("rust-export-{index}"),
            name: symbol.name.clone(),
            kind: symbol.kind.clone(),
            range: symbol.range.clone(),
        })
        .collect::<Vec<_>>();
    for import in &collector.imports {
        if import.visibility != RustVisibility::Private {
            exports.push(RustExport {
                id: format!("rust-export-{}", exports.len()),
                name: import.alias.clone().unwrap_or_else(|| import.path.clone()),
                kind: RustSymbolKind::Unknown,
                range: import.range.clone(),
            });
        }
    }
    let tests = collector
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.kind == RustSymbolKind::Function
                && symbol
                    .attributes
                    .iter()
                    .any(|attribute| attribute.contains("test"))
        })
        .enumerate()
        .map(|(index, symbol)| RustTest {
            id: format!("rust-test-{index}"),
            symbol_id: symbol.id.clone(),
            name: symbol.name.clone(),
            attributes: symbol.attributes.clone(),
            range: symbol.range.clone(),
        })
        .collect();
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
            exports,
            assignments: collector.assignments,
            returns: collector.returns,
            calls: collector.calls,
            branches: collector.branches,
            inheritances: collector.inheritances,
            tests,
            comments,
            syntax_nodes,
            parse_errors,
            detail,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_options_digest(crate::core::options_digest(options).expect("Rust options must serialize"))
    .with_diagnostics(diagnostics)
}

struct RustCollector<'a, 'b> {
    text: &'a str,
    line_index: &'b LineIndex,
    symbols: Vec<RustSymbol>,
    imports: Vec<RustImport>,
    assignments: Vec<RustAssignment>,
    returns: Vec<RustReturn>,
    calls: Vec<RustCall>,
    branches: Vec<RustBranch>,
    inheritances: Vec<RustInheritance>,
    errors: Vec<RustParseError>,
    diagnostics: Vec<Diagnostic>,
    detail: RustDetailMode,
    source_path: Option<String>,
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
                raw: self.source(node).to_string(),
                missing: node.is_missing(),
            });
        }
        let kind = node.kind();
        if matches!(kind, "attribute_item" | "inner_attribute_item") {
            attrs.push(self.source(node).trim().to_string());
        }
        if is_rust_doc_comment(kind, self.source(node)) {
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
                name: name.clone(),
                kind: symbol_kind,
                language: "rust".to_string(),
                path: self.source_path.clone(),
                range: self.range(node),
                visibility: self.visibility_for(node),
                parent: parent.clone(),
                attributes: self.attributes_for(node, &attrs),
                doc: (!docs.is_empty()).then(|| docs.join("\n")),
                syntax,
            };
            self.symbols.push(symbol);
            if kind == "impl_item" {
                self.inheritances.push(self.inheritance_for(node));
            }
            self.walk_children(node, Some(name));
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
        match kind {
            "let_declaration" | "assignment_expression" | "compound_assignment_expr" => self
                .assignments
                .push(self.assignment_for(node, parent.as_deref())),
            "return_expression" => self.returns.push(RustReturn {
                id: format!("rust-return-{}", self.returns.len()),
                expression: node
                    .named_child(0)
                    .map(|child| normalize_ws(self.source(child).trim())),
                range: self.range(node),
                parent: parent.clone(),
            }),
            "call_expression" => self.calls.push(self.call_for(node, parent.as_deref())),
            "if_expression" | "for_expression" | "while_expression" | "loop_expression"
            | "match_expression" => self.branches.push(self.branch_for(node, parent.as_deref())),
            _ => {}
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
                "line_comment" | "block_comment"
                    if is_rust_doc_comment(child.kind(), self.source(child)) =>
                {
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

    fn field_source(&self, node: Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|child| normalize_ws(self.source(child).trim()))
            .filter(|value| !value.is_empty())
    }

    fn assignment_for(&self, node: Node, parent: Option<&str>) -> RustAssignment {
        RustAssignment {
            id: format!("rust-assignment-{}", self.assignments.len()),
            lhs: self
                .field_source(node, "pattern")
                .or_else(|| self.field_source(node, "left"))
                .unwrap_or_else(|| normalize_ws(self.source(node).trim())),
            rhs: self
                .field_source(node, "value")
                .or_else(|| self.field_source(node, "right")),
            operator: assignment_operator(self.source(node)),
            range: self.range(node),
            parent: parent.map(str::to_string),
        }
    }

    fn call_for(&self, node: Node, parent: Option<&str>) -> RustCall {
        RustCall {
            id: format!("rust-call-{}", self.calls.len()),
            target: self.field_source(node, "function").unwrap_or_default(),
            arguments: self
                .field_source(node, "arguments")
                .map(|arguments| split_args(arguments.trim_matches(['(', ')'])))
                .unwrap_or_default(),
            range: self.range(node),
            parent: parent.map(str::to_string),
        }
    }

    fn branch_for(&self, node: Node, parent: Option<&str>) -> RustBranch {
        RustBranch {
            id: format!("rust-branch-{}", self.branches.len()),
            kind: node.kind().to_string(),
            condition: self
                .field_source(node, "condition")
                .or_else(|| self.field_source(node, "value")),
            range: self.range(node),
            parent: parent.map(str::to_string),
        }
    }

    fn inheritance_for(&self, node: Node) -> RustInheritance {
        RustInheritance {
            id: format!("rust-inheritance-{}", self.inheritances.len()),
            implementation: normalize_ws(self.source(node).lines().next().unwrap_or("impl")),
            trait_name: self.field_source(node, "trait"),
            target: self
                .field_source(node, "type")
                .unwrap_or_else(|| self.fallback_name("impl_item", node)),
            range: self.range(node),
        }
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

fn is_rust_doc_comment(kind: &str, src: &str) -> bool {
    let trimmed = src.trim_start();
    matches!(kind, "line_comment" | "block_comment")
        && (trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with("/**")
            || trimmed.starts_with("/*!"))
}

fn normalize_ws(src: &str) -> String {
    src.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assignment_operator(src: &str) -> Option<String> {
    [
        "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", ">>=", "<<=", "=",
    ]
    .into_iter()
    .find(|operator| src.contains(operator))
    .map(str::to_string)
}

fn split_args(src: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, character) in src.char_indices() {
        match character {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                let argument = normalize_ws(src[start..index].trim());
                if !argument.is_empty() {
                    arguments.push(argument);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = normalize_ws(src[start..].trim());
    if !tail.is_empty() {
        arguments.push(tail);
    }
    arguments
}

fn collect_comments(root: Node, text: &str, line_index: &LineIndex) -> Vec<RustComment> {
    fn walk(node: Node, text: &str, line_index: &LineIndex, comments: &mut Vec<RustComment>) {
        if matches!(node.kind(), "line_comment" | "block_comment") {
            let raw = &text[node.start_byte()..node.end_byte()];
            comments.push(RustComment {
                id: format!("rust-comment-{}", comments.len()),
                text: raw.to_string(),
                doc: raw.starts_with("///")
                    || raw.starts_with("//!")
                    || raw.starts_with("/**")
                    || raw.starts_with("/*!"),
                block: node.kind() == "block_comment",
                range: SourceRange::new(node.start_byte(), node.end_byte(), line_index),
            });
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, text, line_index, comments);
        }
    }
    let mut comments = Vec::new();
    walk(root, text, line_index, &mut comments);
    comments
}

fn collect_syntax_nodes(
    root: Node,
    text: &str,
    line_index: &LineIndex,
    detail: RustDetailMode,
) -> Vec<RustSyntaxNode> {
    fn walk(
        node: Node,
        parent: Option<String>,
        text: &str,
        line_index: &LineIndex,
        include_named: bool,
        include_anonymous: bool,
        next_ordinal: &mut usize,
        nodes: &mut Vec<RustSyntaxNode>,
    ) {
        let ordinal = *next_ordinal;
        *next_ordinal += 1;
        let include = node.is_error()
            || node.is_missing()
            || include_anonymous
            || (include_named && node.is_named());
        let id = format!(
            "rust-syntax-{ordinal}-{}-{}-{}",
            node.start_byte(),
            node.end_byte(),
            node.kind()
        );
        let child_parent = if include {
            Some(id.clone())
        } else {
            parent.clone()
        };
        if include {
            nodes.push(RustSyntaxNode {
                id,
                kind: node.kind().to_string(),
                named: node.is_named(),
                error: node.is_error(),
                missing: node.is_missing(),
                parent,
                raw: text[node.start_byte()..node.end_byte()].to_string(),
                range: SourceRange::new(node.start_byte(), node.end_byte(), line_index),
            });
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(
                child,
                child_parent.clone(),
                text,
                line_index,
                include_named,
                include_anonymous,
                next_ordinal,
                nodes,
            );
        }
    }
    let mut nodes = Vec::new();
    let mut next_ordinal = 0;
    walk(
        root,
        None,
        text,
        line_index,
        detail != RustDetailMode::Semantic,
        detail == RustDetailMode::SyntaxDebug,
        &mut next_ordinal,
        &mut nodes,
    );
    nodes
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
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|s| s.name == "Thing")
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|s| s.name == "run")
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .imports
                .len(),
            1
        );
    }
}
