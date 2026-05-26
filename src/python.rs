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
pub struct PythonFile {
    pub schema_version: String,
    pub symbols: Vec<PythonSymbol>,
    pub imports: Vec<PythonImport>,
    pub assignments: Vec<PythonAssignment>,
    pub returns: Vec<PythonReturn>,
    pub calls: Vec<PythonCall>,
    pub branches: Vec<PythonBranch>,
    pub parse_errors: Vec<PythonParseError>,
    pub detail: Option<PythonSyntaxDetail>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonSymbol {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: PythonSymbolKind,
    pub language: String,
    pub path: Option<String>,
    pub range: SourceRange,
    pub visibility: PythonVisibility,
    pub parent: Option<String>,
    pub decorators: Vec<String>,
    pub doc: Option<String>,
    pub syntax: Option<PythonSyntaxSummary>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonSymbolKind {
    Class,
    Function,
    Method,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonVisibility {
    Public,
    Protected,
    Private,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonImport {
    pub id: String,
    pub module: String,
    pub names: Vec<String>,
    pub aliases: Vec<String>,
    pub level: usize,
    pub range: SourceRange,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonAssignment {
    pub id: String,
    pub lhs: String,
    pub rhs: Option<String>,
    pub operator: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonReturn {
    pub id: String,
    pub expression: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonCall {
    pub id: String,
    pub target: String,
    pub args: Vec<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonBranch {
    pub id: String,
    pub kind: String,
    pub condition: Option<String>,
    pub range: SourceRange,
    pub parent: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonParseError {
    pub range: SourceRange,
    pub node_kind: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonSyntaxSummary {
    pub node_kind: String,
    pub named_child_count: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonSyntaxDetail {
    pub root_kind: String,
    pub root_named_child_count: usize,
    pub node_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PythonDetailMode {
    #[default]
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[derive(Debug, Clone, Default)]
pub struct PythonIngestOptions {
    pub detail: PythonDetailMode,
}

pub type PythonEnvelope = Envelope<PythonFile>;

pub fn parse_python(
    text: &str,
    source: SourceInfo,
    options: &PythonIngestOptions,
) -> PythonEnvelope {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser
        .set_language(&language.into())
        .expect("tree-sitter Python language should load");
    let line_index = LineIndex::new(text);
    let Some(tree) = parser.parse(text, None) else {
        return Envelope::new(
            ArtifactKind::PythonCode,
            source,
            ParserInfo::new("tree-sitter-python"),
            SchemaVersion::PYTHON_CODE_V1,
            PythonFile {
                schema_version: SchemaVersion::PYTHON_CODE_V1.to_string(),
                symbols: Vec::new(),
                imports: Vec::new(),
                assignments: Vec::new(),
                returns: Vec::new(),
                calls: Vec::new(),
                branches: Vec::new(),
                parse_errors: Vec::new(),
                detail: None,
            },
        )
        .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
        .with_diagnostics(vec![Diagnostic::error(
            "tree-sitter-python",
            "python.parse.none",
            "tree-sitter returned no parse tree",
        )]);
    };

    let root = tree.root_node();
    let source_path = source
        .path
        .clone()
        .or_else(|| Some(source.display_name.clone()));
    let mut collector = PythonCollector {
        text,
        line_index: &line_index,
        symbols: Vec::new(),
        imports: Vec::new(),
        assignments: Vec::new(),
        returns: Vec::new(),
        calls: Vec::new(),
        branches: Vec::new(),
        errors: Vec::new(),
        diagnostics: Vec::new(),
        detail: options.detail,
        source_path,
    };
    collector.walk(root, Vec::new(), Vec::new());
    let detail = match options.detail {
        PythonDetailMode::SyntaxDebug => Some(PythonSyntaxDetail {
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
                "tree-sitter-python",
                "python.parse.error_node",
                format!("Python parse contained {} node", err.node_kind),
            )
            .with_range(err.range.clone())
            .partial(),
        );
    }
    Envelope::new(
        ArtifactKind::PythonCode,
        source,
        ParserInfo::new("tree-sitter-python"),
        SchemaVersion::PYTHON_CODE_V1,
        PythonFile {
            schema_version: SchemaVersion::PYTHON_CODE_V1.to_string(),
            symbols: collector.symbols,
            imports: collector.imports,
            assignments: collector.assignments,
            returns: collector.returns,
            calls: collector.calls,
            branches: collector.branches,
            parse_errors,
            detail,
        },
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(diagnostics)
}

struct PythonCollector<'a, 'b> {
    text: &'a str,
    line_index: &'b LineIndex,
    symbols: Vec<PythonSymbol>,
    imports: Vec<PythonImport>,
    assignments: Vec<PythonAssignment>,
    returns: Vec<PythonReturn>,
    calls: Vec<PythonCall>,
    branches: Vec<PythonBranch>,
    errors: Vec<PythonParseError>,
    diagnostics: Vec<Diagnostic>,
    detail: PythonDetailMode,
    source_path: Option<String>,
}

impl PythonCollector<'_, '_> {
    fn walk(&mut self, node: Node, parents: Vec<String>, decorators: Vec<String>) {
        if node.kind() == "ERROR" || node.is_missing() {
            self.errors.push(PythonParseError {
                range: self.range(node),
                node_kind: node.kind().to_string(),
            });
        }

        let kind = node.kind();
        if kind == "decorated_definition" {
            let decorators = self.decorators_for(node);
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if matches!(child.kind(), "class_definition" | "function_definition") {
                    self.walk(child, parents.clone(), decorators.clone());
                }
            }
            return;
        }

        if kind == "class_definition" || kind == "function_definition" {
            let name = self
                .name_for(node)
                .unwrap_or_else(|| self.fallback_name(kind, node));
            let symbol_kind = if kind == "class_definition" {
                PythonSymbolKind::Class
            } else if parents.is_empty() {
                PythonSymbolKind::Function
            } else {
                PythonSymbolKind::Method
            };
            let qualified_name = qualify(&parents, &name);
            let id = format!(
                "python-symbol-{}-{}",
                self.symbols.len(),
                qualified_name.replace('.', "_")
            );
            let syntax = (self.detail == PythonDetailMode::SemanticWithSyntax
                || self.detail == PythonDetailMode::SyntaxDebug)
                .then(|| PythonSyntaxSummary {
                    node_kind: kind.to_string(),
                    named_child_count: node.named_child_count(),
                });
            self.symbols.push(PythonSymbol {
                id: id.clone(),
                name: name.clone(),
                qualified_name,
                kind: symbol_kind,
                language: "python".to_string(),
                path: self.source_path.clone(),
                range: self.range(node),
                visibility: visibility_for(&name),
                parent: parents.last().cloned(),
                decorators: decorators.clone(),
                doc: doc_for(self.source(node)),
                syntax,
            });
            let mut child_parents = parents;
            child_parents.push(name);
            self.walk_children(node, child_parents, Vec::new());
            return;
        }

        match kind {
            "import_statement" | "import_from_statement" => {
                self.imports.push(self.import_for(node))
            }
            "assignment" | "augmented_assignment" => {
                self.assignments.push(self.assignment_for(node, &parents));
            }
            "return_statement" => self.returns.push(self.return_for(node, &parents)),
            "call" => self.calls.push(self.call_for(node, &parents)),
            "if_statement" | "elif_clause" | "else_clause" | "for_statement"
            | "while_statement" | "match_statement" => {
                self.branches.push(self.branch_for(node, &parents))
            }
            _ => {}
        }
        self.walk_children(node, parents, decorators);
    }

    fn walk_children(&mut self, node: Node, parents: Vec<String>, decorators: Vec<String>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, parents.clone(), decorators.clone());
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
            .map(|n| self.source(n).trim().to_string())
            .filter(|name| !name.is_empty())
    }

    fn fallback_name(&self, kind: &str, _node: Node) -> String {
        kind.to_string()
    }

    fn decorators_for(&self, node: Node) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "decorator" {
                out.push(normalize_ws(self.source(child).trim()));
            }
        }
        out
    }

    fn import_for(&self, node: Node) -> PythonImport {
        let src = self.source(node).trim();
        let (module, names, aliases, level) = parse_import(src);
        PythonImport {
            id: format!("python-import-{}", self.imports.len()),
            module,
            names,
            aliases,
            level,
            range: self.range(node),
        }
    }

    fn assignment_for(&self, node: Node, parents: &[String]) -> PythonAssignment {
        PythonAssignment {
            id: format!("python-assignment-{}", self.assignments.len()),
            lhs: self
                .field_source(node, "left")
                .unwrap_or_else(|| self.source(node).trim().to_string()),
            rhs: self.field_source(node, "right"),
            operator: assignment_operator(self.source(node)),
            range: self.range(node),
            parent: parents.last().cloned(),
        }
    }

    fn return_for(&self, node: Node, parents: &[String]) -> PythonReturn {
        PythonReturn {
            id: format!("python-return-{}", self.returns.len()),
            expression: return_expression(self.source(node)),
            range: self.range(node),
            parent: parents.last().cloned(),
        }
    }

    fn call_for(&self, node: Node, parents: &[String]) -> PythonCall {
        PythonCall {
            id: format!("python-call-{}", self.calls.len()),
            target: self.field_source(node, "function").unwrap_or_default(),
            args: self
                .field_source(node, "arguments")
                .map(|args| split_args(args.trim_matches(['(', ')'])))
                .unwrap_or_default(),
            range: self.range(node),
            parent: parents.last().cloned(),
        }
    }

    fn branch_for(&self, node: Node, parents: &[String]) -> PythonBranch {
        PythonBranch {
            id: format!("python-branch-{}", self.branches.len()),
            kind: node.kind().to_string(),
            condition: self
                .field_source(node, "condition")
                .or_else(|| self.field_source(node, "right"))
                .or_else(|| self.field_source(node, "subject")),
            range: self.range(node),
            parent: parents.last().cloned(),
        }
    }

    fn field_source(&self, node: Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|child| normalize_ws(self.source(child).trim()))
            .filter(|value| !value.is_empty())
    }
}

fn parse_import(src: &str) -> (String, Vec<String>, Vec<String>, usize) {
    if let Some(rest) = src.strip_prefix("from ") {
        let Some((module_part, import_part)) = rest.split_once(" import ") else {
            return (String::new(), Vec::new(), Vec::new(), 0);
        };
        let level = module_part.chars().take_while(|c| *c == '.').count();
        let module = module_part.trim_start_matches('.').trim().to_string();
        let (names, aliases) = parse_import_list(import_part);
        return (module, names, aliases, level);
    }
    if let Some(rest) = src.strip_prefix("import ") {
        let (names, aliases) = parse_import_list(rest);
        return (String::new(), names, aliases, 0);
    }
    (String::new(), Vec::new(), Vec::new(), 0)
}

fn parse_import_list(src: &str) -> (Vec<String>, Vec<String>) {
    let mut names = Vec::new();
    let mut aliases = Vec::new();
    for part in src.trim_matches(['(', ')']).split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((name, alias)) = part.split_once(" as ") {
            names.push(name.trim().to_string());
            aliases.push(alias.trim().to_string());
        } else {
            names.push(part.to_string());
        }
    }
    names.sort();
    names.dedup();
    aliases.sort();
    aliases.dedup();
    (names, aliases)
}

fn qualify(parents: &[String], name: &str) -> String {
    if parents.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", parents.join("."), name)
    }
}

fn visibility_for(name: &str) -> PythonVisibility {
    if name.starts_with("__") && !name.ends_with("__") {
        PythonVisibility::Private
    } else if name.starts_with('_') && !name.ends_with("__") {
        PythonVisibility::Protected
    } else {
        PythonVisibility::Public
    }
}

fn doc_for(src: &str) -> Option<String> {
    let trimmed = src.trim_start();
    let body = trimmed.split_once(':')?.1.trim_start();
    for quote in ["\"\"\"", "'''"] {
        if let Some(rest) = body.strip_prefix(quote) {
            let end = rest.find(quote)?;
            return Some(rest[..end].to_string());
        }
    }
    None
}

fn assignment_operator(src: &str) -> Option<String> {
    [
        "+=", "-=", "*=", "/=", "//=", "%=", "**=", "@=", "&=", "|=", "^=", ">>=", "<<=", "=",
    ]
    .into_iter()
    .find(|operator| src.contains(operator))
    .map(str::to_string)
}

fn return_expression(src: &str) -> Option<String> {
    src.trim()
        .strip_prefix("return")
        .map(str::trim)
        .filter(|expr| !expr.is_empty())
        .map(normalize_ws)
}

fn split_args(src: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (idx, ch) in src.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
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
    fn extracts_symbols_imports_and_decorators() {
        let src = "import os, sys as system\nfrom .helpers import build as make\nclass Form:\n    @classmethod\n    async def create(cls):\n        value = cls()\n        if value:\n            return value\n        return cls()\n";
        let report = parse_python(
            src,
            SourceInfo::stdin("forms.py"),
            &PythonIngestOptions {
                detail: PythonDetailMode::SemanticWithSyntax,
            },
        );
        assert!(report.payload.symbols.iter().any(|s| s.name == "Form"));
        assert!(
            report
                .payload
                .symbols
                .iter()
                .any(|s| s.qualified_name == "Form.create")
        );
        assert_eq!(report.payload.imports.len(), 2);
        assert!(
            report
                .payload
                .imports
                .iter()
                .any(|import| import.level == 1)
        );
        assert!(report.payload.assignments.iter().any(|a| a.lhs == "value"));
        assert!(
            report
                .payload
                .returns
                .iter()
                .any(|r| r.expression.as_deref() == Some("value"))
        );
        assert!(report.payload.calls.iter().any(|c| c.target == "cls"));
        assert!(
            report
                .payload
                .branches
                .iter()
                .any(|b| b.kind == "if_statement")
        );
    }
}
