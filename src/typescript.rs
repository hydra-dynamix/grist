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
pub struct TypeScriptFile {
    pub schema_version: String,
    pub dialect: TypeScriptDialect,
    pub symbols: Vec<TypeScriptSymbol>,
    pub imports: Vec<TypeScriptImport>,
    pub exports: Vec<TypeScriptExport>,
    pub assignments: Vec<TypeScriptAssignment>,
    pub returns: Vec<TypeScriptReturn>,
    pub calls: Vec<TypeScriptCall>,
    pub branches: Vec<TypeScriptBranch>,
    pub parse_errors: Vec<TypeScriptParseError>,
    pub detail: Option<TypeScriptSyntaxDetail>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TypeScriptDialect {
    #[serde(rename = "typescript")]
    TypeScript,
    Tsx,
    Jsx,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptSymbol {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: TypeScriptSymbolKind,
    pub language: String,
    pub path: Option<String>,
    pub range: SourceRange,
    pub visibility: TypeScriptVisibility,
    pub parent: Option<String>,
    pub modifiers: Vec<String>,
    pub decorators: Vec<String>,
    pub doc: Option<String>,
    pub syntax: Option<TypeScriptSyntaxSummary>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TypeScriptSymbolKind {
    Class,
    Function,
    Method,
    Constructor,
    Interface,
    TypeAlias,
    Enum,
    Namespace,
    Variable,
    Field,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TypeScriptVisibility {
    Public,
    Protected,
    Private,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptImport {
    pub id: String,
    pub module: String,
    pub names: Vec<String>,
    pub default: Option<String>,
    pub namespace: Option<String>,
    pub range: SourceRange,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptExport {
    pub id: String,
    pub names: Vec<String>,
    pub source: Option<String>,
    pub range: SourceRange,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptAssignment {
    pub id: String,
    pub lhs: String,
    pub rhs: Option<String>,
    pub operator: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptReturn {
    pub id: String,
    pub expression: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptCall {
    pub id: String,
    pub target: String,
    pub args: Vec<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptBranch {
    pub id: String,
    pub kind: String,
    pub condition: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptParseError {
    pub range: SourceRange,
    pub node_kind: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptSyntaxSummary {
    pub node_kind: String,
    pub named_child_count: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypeScriptSyntaxDetail {
    pub root_kind: String,
    pub root_named_child_count: usize,
    pub node_count: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TypeScriptDetailMode {
    #[default]
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TypeScriptIngestOptions {
    pub dialect: TypeScriptDialect,
    pub detail: TypeScriptDetailMode,
}

impl crate::core::FormatOptions for TypeScriptIngestOptions {
    const FORMAT: &'static str = "typescript";
}

impl Default for TypeScriptIngestOptions {
    fn default() -> Self {
        Self {
            dialect: TypeScriptDialect::TypeScript,
            detail: TypeScriptDetailMode::Semantic,
        }
    }
}

pub type TypeScriptEnvelope = Envelope<TypeScriptFile>;

pub fn parse_typescript(
    text: &str,
    source: SourceInfo,
    options: &TypeScriptIngestOptions,
) -> TypeScriptEnvelope {
    let mut parser = Parser::new();
    let language = match options.dialect {
        TypeScriptDialect::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        TypeScriptDialect::Tsx | TypeScriptDialect::Jsx => tree_sitter_typescript::LANGUAGE_TSX,
    };
    parser
        .set_language(&language.into())
        .expect("tree-sitter TypeScript language should load");
    let line_index = LineIndex::new(text);
    let Some(tree) = parser.parse(text, None) else {
        return Envelope::without_payload(
            crate::core::OperationKind::Parse,
            ArtifactKind::TypeScriptCode,
            crate::core::OperationStatus::Failed,
            source,
            ParserInfo::new("tree-sitter-typescript"),
            crate::core::options_digest(options).expect("TypeScript options must serialize"),
            SchemaVersion::TYPESCRIPT_CODE_V1,
        )
        .expect("failed envelope status is valid")
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(vec![Diagnostic::error(
            "tree-sitter-typescript",
            "typescript.parse.none",
            "tree-sitter returned no parse tree",
        )]);
    };

    let root = tree.root_node();
    let source_path = source
        .path
        .clone()
        .or_else(|| Some(source.display_name.clone()));
    let mut collector = TypeScriptCollector {
        text,
        line_index: &line_index,
        dialect: options.dialect,
        symbols: Vec::new(),
        imports: Vec::new(),
        exports: Vec::new(),
        assignments: Vec::new(),
        returns: Vec::new(),
        calls: Vec::new(),
        branches: Vec::new(),
        errors: Vec::new(),
        diagnostics: Vec::new(),
        detail: options.detail,
        source_path,
    };
    collector.walk(root, Vec::new());
    let detail = match options.detail {
        TypeScriptDetailMode::SyntaxDebug => Some(TypeScriptSyntaxDetail {
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
                "tree-sitter-typescript",
                "typescript.parse.error_node",
                format!("TypeScript parse contained {} node", err.node_kind),
            )
            .with_range(err.range.clone())
            .partial(),
        );
    }
    Envelope::new(
        ArtifactKind::TypeScriptCode,
        source,
        ParserInfo::new("tree-sitter-typescript"),
        SchemaVersion::TYPESCRIPT_CODE_V1,
        TypeScriptFile {
            schema_version: SchemaVersion::TYPESCRIPT_CODE_V1.to_string(),
            dialect: options.dialect,
            symbols: collector.symbols,
            imports: collector.imports,
            exports: collector.exports,
            assignments: collector.assignments,
            returns: collector.returns,
            calls: collector.calls,
            branches: collector.branches,
            parse_errors,
            detail,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_options_digest(
        crate::core::options_digest(options).expect("TypeScript options must serialize"),
    )
    .with_diagnostics(diagnostics)
}

#[derive(Debug, Clone)]
struct ParentSymbol {
    id: String,
    name: String,
}

struct TypeScriptCollector<'a, 'b> {
    text: &'a str,
    line_index: &'b LineIndex,
    dialect: TypeScriptDialect,
    symbols: Vec<TypeScriptSymbol>,
    imports: Vec<TypeScriptImport>,
    exports: Vec<TypeScriptExport>,
    assignments: Vec<TypeScriptAssignment>,
    returns: Vec<TypeScriptReturn>,
    calls: Vec<TypeScriptCall>,
    branches: Vec<TypeScriptBranch>,
    errors: Vec<TypeScriptParseError>,
    diagnostics: Vec<Diagnostic>,
    detail: TypeScriptDetailMode,
    source_path: Option<String>,
}

impl TypeScriptCollector<'_, '_> {
    fn walk(&mut self, node: Node, parents: Vec<ParentSymbol>) {
        if node.kind() == "ERROR" || node.is_missing() {
            self.errors.push(TypeScriptParseError {
                range: self.range(node),
                node_kind: node.kind().to_string(),
            });
        }

        let kind = node.kind();
        match kind {
            "import_statement" => self.imports.push(self.import_for(node)),
            "export_statement" => self.exports.push(self.export_for(node)),
            "variable_declarator" | "assignment_expression" => {
                self.assignments.push(self.assignment_for(node, &parents));
            }
            "return_statement" => self.returns.push(self.return_for(node, &parents)),
            "call_expression" | "new_expression" => self.calls.push(self.call_for(node, &parents)),
            "if_statement" | "for_statement" | "for_in_statement" | "while_statement"
            | "do_statement" | "switch_statement" | "try_statement" | "catch_clause" => {
                self.branches.push(self.branch_for(node, &parents));
            }
            _ => {}
        }

        if let Some(symbol_kind) = self.symbol_kind(node, &parents) {
            let name = self
                .name_for(node)
                .unwrap_or_else(|| self.fallback_name(kind, node));
            let qualified_name = qualify(&parents, &name);
            let id = format!(
                "typescript-symbol-{}-{}",
                self.symbols.len(),
                qualified_name.replace(['.', '#'], "_")
            );
            let syntax = (self.detail == TypeScriptDetailMode::SemanticWithSyntax
                || self.detail == TypeScriptDetailMode::SyntaxDebug)
                .then(|| TypeScriptSyntaxSummary {
                    node_kind: kind.to_string(),
                    named_child_count: node.named_child_count(),
                });
            self.symbols.push(TypeScriptSymbol {
                id: id.clone(),
                name: name.clone(),
                qualified_name,
                kind: symbol_kind,
                language: match self.dialect {
                    TypeScriptDialect::TypeScript => "typescript",
                    TypeScriptDialect::Tsx => "tsx",
                    TypeScriptDialect::Jsx => "jsx",
                }
                .to_string(),
                path: self.source_path.clone(),
                range: self.range(node),
                visibility: self.visibility_for(node),
                parent: parents.last().map(|parent| parent.id.clone()),
                modifiers: self.modifiers_for(node),
                decorators: self.decorators_for(node),
                doc: self.doc_before(node),
                syntax,
            });
            let mut child_parents = parents;
            if is_parent_symbol(kind) {
                child_parents.push(ParentSymbol { id, name });
            }
            self.walk_children(node, child_parents);
            return;
        }

        self.walk_children(node, parents);
    }

    fn walk_children(&mut self, node: Node, parents: Vec<ParentSymbol>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, parents.clone());
        }
    }

    fn source(&self, node: Node) -> &str {
        &self.text[node.start_byte()..node.end_byte()]
    }

    fn range(&self, node: Node) -> SourceRange {
        SourceRange::new(node.start_byte(), node.end_byte(), self.line_index)
    }

    fn symbol_kind(&self, node: Node, parents: &[ParentSymbol]) -> Option<TypeScriptSymbolKind> {
        match node.kind() {
            "class_declaration" => Some(TypeScriptSymbolKind::Class),
            "function_declaration" | "generator_function_declaration" => {
                Some(TypeScriptSymbolKind::Function)
            }
            "method_definition" if self.name_for(node).as_deref() == Some("constructor") => {
                Some(TypeScriptSymbolKind::Constructor)
            }
            "method_definition" => Some(TypeScriptSymbolKind::Method),
            "method_signature" => Some(TypeScriptSymbolKind::Method),
            "abstract_method_signature" => Some(TypeScriptSymbolKind::Method),
            "constructor_type" | "constructor" => Some(TypeScriptSymbolKind::Constructor),
            "interface_declaration" => Some(TypeScriptSymbolKind::Interface),
            "type_alias_declaration" => Some(TypeScriptSymbolKind::TypeAlias),
            "enum_declaration" => Some(TypeScriptSymbolKind::Enum),
            "internal_module" | "module" => Some(TypeScriptSymbolKind::Namespace),
            "public_field_definition" | "property_signature" => Some(TypeScriptSymbolKind::Field),
            "variable_declarator" if self.initializer_is_function(node) => {
                Some(TypeScriptSymbolKind::Function)
            }
            "variable_declarator" if parents.is_empty() => Some(TypeScriptSymbolKind::Variable),
            _ => None,
        }
    }

    fn initializer_is_function(&self, node: Node) -> bool {
        node.child_by_field_name("value")
            .map(|value| {
                matches!(
                    value.kind(),
                    "arrow_function" | "function_expression" | "generator_function" | "function"
                )
            })
            .unwrap_or(false)
    }

    fn name_for(&self, node: Node) -> Option<String> {
        node.child_by_field_name("name")
            .map(|child| self.source(child).trim().to_string())
            .or_else(|| {
                let mut cursor = node.walk();
                node.named_children(&mut cursor)
                    .find(|child| {
                        matches!(
                            child.kind(),
                            "identifier"
                                | "property_identifier"
                                | "type_identifier"
                                | "shorthand_property_identifier"
                                | "private_property_identifier"
                        )
                    })
                    .map(|child| self.source(child).trim().to_string())
            })
            .filter(|name| !name.is_empty())
    }

    fn fallback_name(&self, kind: &str, _node: Node) -> String {
        kind.to_string()
    }

    fn import_for(&self, node: Node) -> TypeScriptImport {
        let src = self.source(node).trim();
        let module = string_literals(src).last().cloned().unwrap_or_default();
        let default = default_import_name(src);
        let namespace = namespace_import_name(src);
        TypeScriptImport {
            id: format!("typescript-import-{}", self.imports.len()),
            module,
            names: named_imports(src),
            default,
            namespace,
            range: self.range(node),
        }
    }

    fn export_for(&self, node: Node) -> TypeScriptExport {
        let src = self.source(node).trim();
        TypeScriptExport {
            id: format!("typescript-export-{}", self.exports.len()),
            names: export_names(src),
            source: string_literals(src).last().cloned(),
            range: self.range(node),
        }
    }

    fn assignment_for(&self, node: Node, parents: &[ParentSymbol]) -> TypeScriptAssignment {
        TypeScriptAssignment {
            id: format!("typescript-assignment-{}", self.assignments.len()),
            lhs: self
                .field_source(node, "name")
                .or_else(|| self.field_source(node, "left"))
                .unwrap_or_else(|| self.source(node).trim().to_string()),
            rhs: self
                .field_source(node, "value")
                .or_else(|| self.field_source(node, "right")),
            operator: assignment_operator(self.source(node)),
            range: self.range(node),
            parent: parents.last().map(|parent| parent.id.clone()),
        }
    }

    fn return_for(&self, node: Node, parents: &[ParentSymbol]) -> TypeScriptReturn {
        TypeScriptReturn {
            id: format!("typescript-return-{}", self.returns.len()),
            expression: return_expression(self.source(node)),
            range: self.range(node),
            parent: parents.last().map(|parent| parent.id.clone()),
        }
    }

    fn call_for(&self, node: Node, parents: &[ParentSymbol]) -> TypeScriptCall {
        TypeScriptCall {
            id: format!("typescript-call-{}", self.calls.len()),
            target: self
                .field_source(node, "function")
                .or_else(|| self.field_source(node, "constructor"))
                .unwrap_or_default(),
            args: self
                .field_source(node, "arguments")
                .map(|args| split_args(args.trim_matches(['(', ')'])))
                .unwrap_or_default(),
            range: self.range(node),
            parent: parents.last().map(|parent| parent.id.clone()),
        }
    }

    fn branch_for(&self, node: Node, parents: &[ParentSymbol]) -> TypeScriptBranch {
        TypeScriptBranch {
            id: format!("typescript-branch-{}", self.branches.len()),
            kind: node.kind().to_string(),
            condition: self.field_source(node, "condition"),
            range: self.range(node),
            parent: parents.last().map(|parent| parent.id.clone()),
        }
    }

    fn field_source(&self, node: Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|child| normalize_ws(self.source(child).trim()))
            .filter(|value| !value.is_empty())
    }

    fn visibility_for(&self, node: Node) -> TypeScriptVisibility {
        let src = self.source(node).trim_start();
        if src.starts_with("private ") || src.starts_with("#") {
            TypeScriptVisibility::Private
        } else if src.starts_with("protected ") {
            TypeScriptVisibility::Protected
        } else if src.starts_with("public ") || !src.is_empty() {
            TypeScriptVisibility::Public
        } else {
            TypeScriptVisibility::Unknown
        }
    }

    fn modifiers_for(&self, node: Node) -> Vec<String> {
        let line_start = self.line_start(node.start_byte());
        let src = format!(
            "{}{}",
            &self.text[line_start..node.start_byte()],
            self.source(node)
        );
        let src = src.trim_start();
        let mut modifiers = [
            "export",
            "default",
            "declare",
            "abstract",
            "async",
            "static",
            "readonly",
            "private",
            "protected",
            "public",
        ]
        .into_iter()
        .filter(|modifier| src.starts_with(*modifier) || src.contains(&format!(" {modifier} ")))
        .map(str::to_string)
        .collect::<Vec<_>>();
        modifiers.sort();
        modifiers.dedup();
        modifiers
    }

    fn decorators_for(&self, node: Node) -> Vec<String> {
        let mut decorators = self.decorators_before(node);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "decorator" {
                decorators.push(normalize_ws(self.source(child).trim()));
            }
        }
        decorators.sort();
        decorators.dedup();
        decorators
    }

    fn decorators_before(&self, node: Node) -> Vec<String> {
        let mut decorators = Vec::new();
        let line_start = self.line_start(node.start_byte());
        for line in self.text[..line_start].lines().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if decorators.is_empty() {
                    continue;
                }
                break;
            }
            if trimmed.starts_with('@') {
                decorators.push(normalize_ws(trimmed));
                continue;
            }
            break;
        }
        decorators.reverse();
        decorators
    }

    fn doc_before(&self, node: Node) -> Option<String> {
        let prefix = self.prefix_before_decorators(node);
        let before = prefix.trim_end();
        let end = before.rfind("*/")?;
        if end + 2 != before.len() {
            return None;
        }
        let start = before[..end].rfind("/**")?;
        Some(before[start..end + 2].to_string())
    }

    fn prefix_before_decorators(&self, node: Node) -> &str {
        let mut start = self.line_start(node.start_byte());
        for line in self.text[..start].lines().rev() {
            let line_start = start.saturating_sub(line.len() + 1);
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if start == node.start_byte() {
                    start = line_start;
                    continue;
                }
                break;
            }
            if trimmed.starts_with('@') {
                start = line_start;
                continue;
            }
            break;
        }
        &self.text[..start]
    }

    fn line_start(&self, byte_offset: usize) -> usize {
        self.text[..byte_offset]
            .rfind('\n')
            .map(|idx| idx + 1)
            .unwrap_or(0)
    }
}

fn is_parent_symbol(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration" | "interface_declaration" | "enum_declaration" | "internal_module"
    )
}

fn qualify(parents: &[ParentSymbol], name: &str) -> String {
    if parents.is_empty() {
        name.to_string()
    } else {
        format!(
            "{}.{}",
            parents
                .iter()
                .map(|parent| parent.name.as_str())
                .collect::<Vec<_>>()
                .join("."),
            name
        )
    }
}

fn default_import_name(src: &str) -> Option<String> {
    let rest = src.strip_prefix("import ")?.trim_start();
    if rest.starts_with('{')
        || rest.starts_with('*')
        || rest.starts_with('"')
        || rest.starts_with('\'')
    {
        return None;
    }
    rest.split([',', ' '])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "type")
        .map(str::to_string)
}

fn namespace_import_name(src: &str) -> Option<String> {
    let (_, alias) = src.split_once("* as ")?;
    alias
        .split_whitespace()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn named_imports(src: &str) -> Vec<String> {
    let Some(start) = src.find('{') else {
        return Vec::new();
    };
    let Some(end) = src[start + 1..].find('}') else {
        return Vec::new();
    };
    split_names(&src[start + 1..start + 1 + end])
}

fn export_names(src: &str) -> Vec<String> {
    if let Some(start) = src.find('{') {
        if let Some(end) = src[start + 1..].find('}') {
            return split_names(&src[start + 1..start + 1 + end]);
        }
    }
    for keyword in [
        "class ",
        "function ",
        "interface ",
        "type ",
        "enum ",
        "const ",
        "let ",
        "var ",
    ] {
        if let Some(rest) = src.split_once(keyword).map(|(_, rest)| rest) {
            let name = rest
                .split(|ch: char| !matches!(ch, '_' | '$') && !ch.is_ascii_alphanumeric())
                .next()
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                return vec![name.to_string()];
            }
        }
    }
    Vec::new()
}

fn split_names(src: &str) -> Vec<String> {
    let mut names = src
        .split(',')
        .filter_map(|part| {
            let value = part.trim();
            if value.is_empty() {
                return None;
            }
            Some(
                value
                    .split(" as ")
                    .next()
                    .unwrap_or(value)
                    .trim()
                    .trim_start_matches("type ")
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn string_literals(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = src.char_indices();
    while let Some((start_idx, quote)) = chars.next() {
        if quote != '"' && quote != '\'' {
            continue;
        }
        for (end_idx, candidate) in chars.by_ref() {
            if candidate == quote {
                out.push(src[start_idx + quote.len_utf8()..end_idx].to_string());
                break;
            }
        }
    }
    out
}

fn assignment_operator(src: &str) -> Option<String> {
    [
        "??=", "||=", "&&=", "+=", "-=", "*=", "/=", "%=", "**=", "&=", "|=", "^=", ">>=", "<<=",
        ">>>=", "=",
    ]
    .into_iter()
    .find(|operator| src.contains(operator))
    .map(str::to_string)
}

fn return_expression(src: &str) -> Option<String> {
    src.trim()
        .strip_prefix("return")
        .map(str::trim)
        .map(|value| value.trim_end_matches(';').trim())
        .filter(|expr| !expr.is_empty())
        .map(normalize_ws)
}

fn split_args(src: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (idx, ch) in src.char_indices() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                let arg = normalize_ws(src[start..idx].trim());
                if !arg.is_empty() {
                    args.push(arg);
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    let tail = normalize_ws(src[start..].trim());
    if !tail.is_empty() {
        args.push(tail);
    }
    args
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
    fn extracts_typescript_symbols_imports_and_flow() {
        let src = "import React, { useMemo as memo } from \"react\";\nexport interface Shape { area(): number }\nexport class Box {\n  private value = 1;\n  area() { if (this.value) { return memo(() => this.value, []); } }\n}\n";
        let report = parse_typescript(
            src,
            SourceInfo::stdin("box.ts"),
            &TypeScriptIngestOptions {
                detail: TypeScriptDetailMode::SemanticWithSyntax,
                ..Default::default()
            },
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|symbol| symbol.name == "Shape")
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|symbol| symbol.qualified_name == "Box.area")
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
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .imports
                .iter()
                .any(|import| import.module == "react")
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .branches
                .iter()
                .any(|branch| branch.kind == "if_statement")
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .calls
                .iter()
                .any(|call| call.target == "memo")
        );
    }

    #[test]
    fn parses_tsx_dialect() {
        let src = "export function View() { return <section data-id=\"ok\">Hello</section>; }";
        let report = parse_typescript(
            src,
            SourceInfo::stdin("view.tsx"),
            &TypeScriptIngestOptions {
                dialect: TypeScriptDialect::Tsx,
                detail: TypeScriptDetailMode::SyntaxDebug,
            },
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .dialect,
            TypeScriptDialect::Tsx
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .detail
                .is_some()
        );
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn parses_jsx_dialect() {
        let src = "export function View() { return <section data-id=\"ok\">Hello</section>; }";
        let report = parse_typescript(
            src,
            SourceInfo::stdin("view.jsx"),
            &TypeScriptIngestOptions {
                dialect: TypeScriptDialect::Jsx,
                detail: TypeScriptDetailMode::SyntaxDebug,
            },
        );
        assert_eq!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .dialect,
            TypeScriptDialect::Jsx
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|symbol| symbol.name == "View" && symbol.language == "jsx")
        );
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn extracts_typescript_parity_constructs() {
        let src = "import DefaultThing, * as all from \"pkg\";\nexport { DefaultThing as Renamed } from \"pkg\";\n@sealed\nexport class Service {\n  @trace\n  run(): void {}\n}\nexport interface Shape { area(): number }\nexport type Id = string;\nexport enum Mode { Fast }\nexport const make = () => new Service();\nnamespace Tools { export function build() { return make(); } }\n";
        let report = parse_typescript(
            src,
            SourceInfo::stdin("service.ts"),
            &TypeScriptIngestOptions {
                detail: TypeScriptDetailMode::SemanticWithSyntax,
                ..Default::default()
            },
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|symbol| {
                    symbol.name == "Service"
                        && symbol
                            .decorators
                            .iter()
                            .any(|decorator| decorator == "@sealed")
                })
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|symbol| {
                    symbol.qualified_name == "Service.run"
                        && symbol
                            .decorators
                            .iter()
                            .any(|decorator| decorator == "@trace")
                })
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|symbol| symbol.name == "Shape"
                    && symbol.kind == TypeScriptSymbolKind::Interface)
        );
        assert!(
            report
                .payload.as_ref().expect("complete operation payload")
                .symbols
                .iter()
                .any(|symbol| symbol.name == "Id" && symbol.kind == TypeScriptSymbolKind::TypeAlias)
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(|symbol| symbol.name == "Mode" && symbol.kind == TypeScriptSymbolKind::Enum)
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .symbols
                .iter()
                .any(
                    |symbol| symbol.name == "make" && symbol.kind == TypeScriptSymbolKind::Function
                )
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .imports
                .iter()
                .any(|import| import.default.as_deref() == Some("DefaultThing")
                    && import.namespace.as_deref() == Some("all"))
        );
        assert!(
            report
                .payload
                .as_ref()
                .expect("complete operation payload")
                .exports
                .iter()
                .any(|export| export.source.as_deref() == Some("pkg")
                    && export.names.iter().any(|name| name == "DefaultThing"))
        );
        assert!(report.diagnostics.is_empty());
    }
}
