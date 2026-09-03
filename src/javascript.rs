//! First-class JavaScript and JSX parsing backed by `tree-sitter-javascript`.

use crate::core::{ArtifactKind, Envelope, SchemaVersion, SourceInfo, SourceRange};
use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JavaScriptFile {
    pub schema_version: String,
    pub dialect: JavaScriptDialect,
    pub symbols: Vec<JavaScriptSymbol>,
    pub imports: Vec<JavaScriptImport>,
    pub exports: Vec<JavaScriptExport>,
    pub assignments: Vec<JavaScriptAssignment>,
    pub returns: Vec<JavaScriptReturn>,
    pub calls: Vec<JavaScriptCall>,
    pub branches: Vec<JavaScriptBranch>,
    #[serde(default)]
    pub tests: Vec<JavaScriptTest>,
    #[serde(default)]
    pub comments: Vec<JavaScriptComment>,
    #[serde(default)]
    pub syntax_nodes: Vec<JavaScriptSyntaxNode>,
    pub parse_errors: Vec<JavaScriptParseError>,
    pub detail: Option<JavaScriptSyntaxDetail>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum JavaScriptDialect {
    #[default]
    JavaScript,
    Jsx,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JavaScriptSymbol {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: JavaScriptSymbolKind,
    pub language: String,
    pub path: Option<String>,
    pub range: SourceRange,
    pub visibility: JavaScriptVisibility,
    pub parent: Option<String>,
    pub modifiers: Vec<String>,
    pub decorators: Vec<String>,
    #[serde(default)]
    pub extends: Vec<String>,
    #[serde(default)]
    pub implements: Vec<String>,
    pub doc: Option<String>,
    pub syntax: Option<JavaScriptSyntaxSummary>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JavaScriptSymbolKind {
    Class,
    Function,
    Method,
    Constructor,
    Variable,
    Field,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JavaScriptVisibility {
    Public,
    Protected,
    Private,
    Unknown,
}

macro_rules! javascript_fact {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[cfg_attr(feature = "schemas", derive(JsonSchema))]
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        pub struct $name {
            $(pub $field: $ty,)*
        }
    };
}

javascript_fact!(JavaScriptImport {
    id: String,
    module: String,
    names: Vec<String>,
    default: Option<String>,
    namespace: Option<String>,
    range: SourceRange,
});
javascript_fact!(JavaScriptExport {
    id: String,
    names: Vec<String>,
    source: Option<String>,
    range: SourceRange,
});
javascript_fact!(JavaScriptAssignment {
    id: String,
    lhs: String,
    rhs: Option<String>,
    operator: Option<String>,
    range: SourceRange,
    parent: Option<String>,
});
javascript_fact!(JavaScriptReturn {
    id: String,
    expression: Option<String>,
    range: SourceRange,
    parent: Option<String>,
});
javascript_fact!(JavaScriptCall {
    id: String,
    target: String,
    args: Vec<String>,
    range: SourceRange,
    parent: Option<String>,
});
javascript_fact!(JavaScriptBranch {
    id: String,
    kind: String,
    condition: Option<String>,
    range: SourceRange,
    parent: Option<String>,
});
javascript_fact!(JavaScriptTest {
    id: String,
    name: String,
    framework: String,
    range: SourceRange,
    parent: Option<String>,
});
javascript_fact!(JavaScriptComment {
    id: String,
    text: String,
    doc: bool,
    range: SourceRange,
});
javascript_fact!(JavaScriptSyntaxNode {
    id: String,
    kind: String,
    named: bool,
    error: bool,
    missing: bool,
    parent: Option<String>,
    raw: String,
    range: SourceRange,
});
javascript_fact!(JavaScriptParseError {
    range: SourceRange,
    node_kind: String,
    raw: String,
    missing: bool,
});
javascript_fact!(JavaScriptSyntaxSummary {
    node_kind: String,
    named_child_count: usize,
});
javascript_fact!(JavaScriptSyntaxDetail {
    root_kind: String,
    root_named_child_count: usize,
    node_count: usize,
});

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum JavaScriptDetailMode {
    #[default]
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JavaScriptIngestOptions {
    pub dialect: JavaScriptDialect,
    pub detail: JavaScriptDetailMode,
}

impl crate::core::FormatOptions for JavaScriptIngestOptions {
    const FORMAT: &'static str = "javascript";
}

pub type JavaScriptEnvelope = Envelope<JavaScriptFile>;

pub fn parse_javascript(
    text: &str,
    source: SourceInfo,
    options: &JavaScriptIngestOptions,
) -> JavaScriptEnvelope {
    let family_dialect = match options.dialect {
        JavaScriptDialect::JavaScript => crate::typescript::TypeScriptDialect::JavaScript,
        JavaScriptDialect::Jsx => crate::typescript::TypeScriptDialect::Jsx,
    };
    let detail = match options.detail {
        JavaScriptDetailMode::Semantic => crate::typescript::TypeScriptDetailMode::Semantic,
        JavaScriptDetailMode::SemanticWithSyntax => {
            crate::typescript::TypeScriptDetailMode::SemanticWithSyntax
        }
        JavaScriptDetailMode::SyntaxDebug => crate::typescript::TypeScriptDetailMode::SyntaxDebug,
    };
    let parsed = crate::typescript::parse_ecmascript_family(
        text,
        source,
        family_dialect,
        detail,
        tree_sitter_javascript::LANGUAGE.into(),
        ArtifactKind::JavaScriptCode,
        SchemaVersion::JAVASCRIPT_CODE_V1,
        "tree-sitter-javascript",
        "javascript",
        crate::core::options_digest(options).expect("JavaScript options must serialize"),
    );
    Envelope {
        schema_version: parsed.schema_version,
        operation: parsed.operation,
        kind: parsed.kind,
        status: parsed.status,
        source: parsed.source,
        identity: parsed.identity,
        hashes: parsed.hashes,
        parser: parsed.parser,
        options_digest: parsed.options_digest,
        providers: parsed.providers,
        diagnostics: parsed.diagnostics,
        provenance: parsed.provenance,
        payload_schema_version: parsed.payload_schema_version,
        payload: parsed.payload.map(JavaScriptFile::from),
    }
}

impl From<crate::typescript::TypeScriptFile> for JavaScriptFile {
    fn from(value: crate::typescript::TypeScriptFile) -> Self {
        let mut file = Self {
            schema_version: value.schema_version,
            dialect: match value.dialect {
                crate::typescript::TypeScriptDialect::Jsx => JavaScriptDialect::Jsx,
                _ => JavaScriptDialect::JavaScript,
            },
            symbols: value.symbols.into_iter().map(Into::into).collect(),
            imports: value.imports.into_iter().map(Into::into).collect(),
            exports: value.exports.into_iter().map(Into::into).collect(),
            assignments: value.assignments.into_iter().map(Into::into).collect(),
            returns: value.returns.into_iter().map(Into::into).collect(),
            calls: value.calls.into_iter().map(Into::into).collect(),
            branches: value.branches.into_iter().map(Into::into).collect(),
            tests: value.tests.into_iter().map(Into::into).collect(),
            comments: value.comments.into_iter().map(Into::into).collect(),
            syntax_nodes: value.syntax_nodes.into_iter().map(Into::into).collect(),
            parse_errors: value.parse_errors.into_iter().map(Into::into).collect(),
            detail: value.detail.map(Into::into),
        };
        for id in file
            .symbols
            .iter_mut()
            .map(|value| &mut value.id)
            .chain(file.imports.iter_mut().map(|value| &mut value.id))
            .chain(file.exports.iter_mut().map(|value| &mut value.id))
            .chain(file.assignments.iter_mut().map(|value| &mut value.id))
            .chain(file.returns.iter_mut().map(|value| &mut value.id))
            .chain(file.calls.iter_mut().map(|value| &mut value.id))
            .chain(file.branches.iter_mut().map(|value| &mut value.id))
            .chain(file.tests.iter_mut().map(|value| &mut value.id))
            .chain(file.comments.iter_mut().map(|value| &mut value.id))
            .chain(file.syntax_nodes.iter_mut().map(|value| &mut value.id))
        {
            *id = javascript_id(id);
        }
        for syntax in &mut file.syntax_nodes {
            if let Some(parent) = &mut syntax.parent {
                *parent = javascript_id(parent);
            }
        }
        file
    }
}

fn javascript_id(id: &str) -> String {
    id.strip_prefix("typescript-")
        .map(|suffix| format!("javascript-{suffix}"))
        .unwrap_or_else(|| id.to_string())
}

impl From<crate::typescript::TypeScriptSymbol> for JavaScriptSymbol {
    fn from(value: crate::typescript::TypeScriptSymbol) -> Self {
        use crate::typescript::TypeScriptSymbolKind as T;
        Self {
            id: value.id,
            name: value.name,
            qualified_name: value.qualified_name,
            kind: match value.kind {
                T::Class => JavaScriptSymbolKind::Class,
                T::Function => JavaScriptSymbolKind::Function,
                T::Method => JavaScriptSymbolKind::Method,
                T::Constructor => JavaScriptSymbolKind::Constructor,
                T::Variable => JavaScriptSymbolKind::Variable,
                T::Field => JavaScriptSymbolKind::Field,
                _ => JavaScriptSymbolKind::Unknown,
            },
            language: value.language,
            path: value.path,
            range: value.range,
            visibility: match value.visibility {
                crate::typescript::TypeScriptVisibility::Public => JavaScriptVisibility::Public,
                crate::typescript::TypeScriptVisibility::Protected => {
                    JavaScriptVisibility::Protected
                }
                crate::typescript::TypeScriptVisibility::Private => JavaScriptVisibility::Private,
                crate::typescript::TypeScriptVisibility::Unknown => JavaScriptVisibility::Unknown,
            },
            parent: value.parent,
            modifiers: value.modifiers,
            decorators: value.decorators,
            extends: value.extends,
            implements: value.implements,
            doc: value.doc,
            syntax: value.syntax.map(Into::into),
        }
    }
}

macro_rules! convert_fact {
    ($from:ty => $to:ident { $($field:ident),* $(,)? }) => {
        impl From<$from> for $to {
            fn from(value: $from) -> Self {
                Self { $($field: value.$field,)* }
            }
        }
    };
}

convert_fact!(crate::typescript::TypeScriptImport => JavaScriptImport {
    id, module, names, default, namespace, range
});
convert_fact!(crate::typescript::TypeScriptExport => JavaScriptExport {
    id, names, source, range
});
convert_fact!(crate::typescript::TypeScriptAssignment => JavaScriptAssignment {
    id, lhs, rhs, operator, range, parent
});
convert_fact!(crate::typescript::TypeScriptReturn => JavaScriptReturn {
    id, expression, range, parent
});
convert_fact!(crate::typescript::TypeScriptCall => JavaScriptCall {
    id, target, args, range, parent
});
convert_fact!(crate::typescript::TypeScriptBranch => JavaScriptBranch {
    id, kind, condition, range, parent
});
convert_fact!(crate::typescript::TypeScriptTest => JavaScriptTest {
    id, name, framework, range, parent
});
convert_fact!(crate::typescript::TypeScriptComment => JavaScriptComment {
    id, text, doc, range
});
convert_fact!(crate::typescript::TypeScriptSyntaxNode => JavaScriptSyntaxNode {
    id, kind, named, error, missing, parent, raw, range
});
convert_fact!(crate::typescript::TypeScriptParseError => JavaScriptParseError {
    range, node_kind, raw, missing
});
convert_fact!(crate::typescript::TypeScriptSyntaxSummary => JavaScriptSyntaxSummary {
    node_kind, named_child_count
});
convert_fact!(crate::typescript::TypeScriptSyntaxDetail => JavaScriptSyntaxDetail {
    root_kind, root_named_child_count, node_count
});
