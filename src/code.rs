//! Registry-backed tree-sitter adapter for secondary and caller-supplied languages.
//!
//! The adapter parses source as inert data. It never executes active content,
//! starts subprocesses, accesses providers, or performs network I/O.

#[cfg(feature = "secondary-code")]
use crate::core::LineIndex;
use crate::core::{ParserInfo, SchemaVersion, SourceRange};
use crate::document_graph::{
    DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode, DocumentNodeKind,
    DocumentRelation, ProjectionAddress, ToDocumentGraph, TransformError,
};
use crate::registry::{
    Capability, FormatMetadata, OptionsMetadata, ParserDescriptor, ParserOrigin, SchemaMetadata,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub const TREE_SITTER_ADAPTER_SCHEMA_V1: &str = "grist/tree-sitter-language-adapter/v1";
pub const CODE_OPTIONS_SCHEMA_V1: &str = "grist/code-options/v1";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TreeSitterAdapterCapability {
    GrammarDetection,
    SourceRanges,
    ErrorRecovery,
    DocumentGraphProjection,
    InertParsing,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeSitterAdapterMetadata {
    pub schema_version: String,
    pub language: String,
    pub grammar: String,
    pub grammar_version: String,
    pub enabled_feature: String,
    pub capabilities: BTreeSet<TreeSitterAdapterCapability>,
    pub active_content_execution: bool,
    pub network_access: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeSyntaxNode {
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeParseError {
    pub node_kind: String,
    pub missing: bool,
    pub raw: String,
    pub range: SourceRange,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeFile {
    pub schema_version: String,
    pub adapter: TreeSitterAdapterMetadata,
    pub root_kind: String,
    pub syntax_nodes: Vec<CodeSyntaxNode>,
    pub parse_errors: Vec<CodeParseError>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CodeIngestOptions {
    pub include_anonymous_nodes: bool,
}

impl crate::core::FormatOptions for CodeIngestOptions {
    const FORMAT: &'static str = "code";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeSitterLanguageConfig {
    pub language: String,
    pub display_name: String,
    pub parser_id: String,
    pub grammar: String,
    pub grammar_version: String,
    pub enabled_feature: String,
    pub aliases: BTreeSet<String>,
    pub media_types: BTreeSet<String>,
    pub extensions: BTreeSet<String>,
    pub probe_markers: Vec<String>,
    pub minimum_marker_matches: usize,
    pub case_sensitive_markers: bool,
}

impl TreeSitterLanguageConfig {
    pub fn new(
        language: impl Into<String>,
        parser_id: impl Into<String>,
        grammar: impl Into<String>,
        grammar_version: impl Into<String>,
        enabled_feature: impl Into<String>,
    ) -> Self {
        let language = language.into();
        Self {
            display_name: language.clone(),
            language,
            parser_id: parser_id.into(),
            grammar: grammar.into(),
            grammar_version: grammar_version.into(),
            enabled_feature: enabled_feature.into(),
            aliases: BTreeSet::new(),
            media_types: BTreeSet::new(),
            extensions: BTreeSet::new(),
            probe_markers: Vec::new(),
            minimum_marker_matches: 1,
            case_sensitive_markers: true,
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = display_name.into();
        self
    }

    pub fn with_aliases(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.aliases.extend(values.into_iter().map(Into::into));
        self
    }

    pub fn with_media_types(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.media_types.extend(values.into_iter().map(Into::into));
        self
    }

    pub fn with_extensions(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.extensions.extend(values.into_iter().map(Into::into));
        self
    }

    pub fn with_probe_markers(
        mut self,
        values: impl IntoIterator<Item = impl Into<String>>,
        minimum_matches: usize,
        case_sensitive: bool,
    ) -> Self {
        self.probe_markers = values.into_iter().map(Into::into).collect();
        self.minimum_marker_matches = minimum_matches.max(1);
        self.case_sensitive_markers = case_sensitive;
        self
    }

    pub fn format_metadata(&self) -> FormatMetadata {
        let mut format = FormatMetadata::new(&self.language, crate::core::ArtifactKind::Code);
        format.display_name = self.display_name.clone();
        format.aliases = self.aliases.clone();
        format.media_types = self.media_types.clone();
        format.extensions = self.extensions.clone();
        format
    }

    pub fn descriptor(&self, origin: ParserOrigin) -> ParserDescriptor {
        let priority = match origin {
            ParserOrigin::BuiltIn => ParserDescriptor::BUILTIN_PRIORITY,
            ParserOrigin::Caller => ParserDescriptor::CALLER_PRIORITY,
        };
        ParserDescriptor {
            id: self.parser_id.clone(),
            origin,
            priority,
            format: self.format_metadata(),
            parser: ParserInfo::new(&self.parser_id)
                .with_implementation(&self.grammar, &self.grammar_version)
                .with_grammar_version(&self.grammar_version)
                .with_feature(&self.enabled_feature),
            payload_schema: SchemaMetadata::new("code", SchemaVersion::CODE_V1),
            options: OptionsMetadata::new(
                SchemaMetadata::new("code-options", CODE_OPTIONS_SCHEMA_V1),
                serde_json::to_value(CodeIngestOptions::default()).unwrap_or_default(),
            ),
            required_features: BTreeSet::from([self.enabled_feature.clone()]),
            capabilities: BTreeSet::from([
                Capability::NativeExtraction,
                Capability::TypedPayload,
                Capability::DocumentGraphProjection,
                Capability::GrammarDetection,
                Capability::SourceRanges,
                Capability::ErrorRecovery,
                Capability::InertParsing,
            ]),
            allowed_providers: BTreeSet::new(),
            required_providers: BTreeSet::new(),
        }
    }

    pub fn adapter_metadata(&self) -> TreeSitterAdapterMetadata {
        TreeSitterAdapterMetadata {
            schema_version: TREE_SITTER_ADAPTER_SCHEMA_V1.to_string(),
            language: self.language.clone(),
            grammar: self.grammar.clone(),
            grammar_version: self.grammar_version.clone(),
            enabled_feature: self.enabled_feature.clone(),
            capabilities: BTreeSet::from([
                TreeSitterAdapterCapability::GrammarDetection,
                TreeSitterAdapterCapability::SourceRanges,
                TreeSitterAdapterCapability::ErrorRecovery,
                TreeSitterAdapterCapability::DocumentGraphProjection,
                TreeSitterAdapterCapability::InertParsing,
            ]),
            active_content_execution: false,
            network_access: false,
        }
    }

    #[cfg(feature = "secondary-code")]
    fn marker_matches(&self, text: &str) -> bool {
        if self.probe_markers.is_empty() {
            return false;
        }
        let normalized;
        let haystack = if self.case_sensitive_markers {
            text
        } else {
            normalized = text.to_ascii_lowercase();
            &normalized
        };
        self.probe_markers
            .iter()
            .filter(|marker| {
                if self.case_sensitive_markers {
                    contains_marker(haystack, marker)
                } else {
                    contains_marker(haystack, &marker.to_ascii_lowercase())
                }
            })
            .count()
            >= self.minimum_marker_matches
    }
}

#[cfg(feature = "secondary-code")]
fn contains_marker(haystack: &str, marker: &str) -> bool {
    haystack.match_indices(marker).any(|(start, _)| {
        !marker
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || start == 0
            || haystack
                .as_bytes()
                .get(start - 1)
                .is_some_and(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
    })
}

pub fn builtin_language_configs() -> Vec<TreeSitterLanguageConfig> {
    vec![
        TreeSitterLanguageConfig::new("go", "tree-sitter-go", "tree-sitter-go", "0.25.0", "go")
            .with_display_name("Go")
            .with_media_types(["text/x-go"])
            .with_extensions(["go"])
            .with_probe_markers(["package ", "func "], 2, true),
        TreeSitterLanguageConfig::new(
            "java",
            "tree-sitter-java",
            "tree-sitter-java",
            "0.23.5",
            "java",
        )
        .with_display_name("Java")
        .with_media_types(["text/x-java"])
        .with_extensions(["java"])
        .with_probe_markers(
            ["public class ", "static void main", "import java."],
            1,
            true,
        ),
        TreeSitterLanguageConfig::new(
            "kotlin",
            "tree-sitter-kotlin",
            "tree-sitter-kotlin-sg",
            "0.4.1",
            "kotlin",
        )
        .with_display_name("Kotlin")
        .with_media_types(["text/x-kotlin"])
        .with_extensions(["kt", "kts"])
        .with_probe_markers(["fun ", "val ", "data class ", "object "], 2, true),
        TreeSitterLanguageConfig::new("c", "tree-sitter-c", "tree-sitter-c", "0.24.2", "c")
            .with_display_name("C")
            .with_media_types(["text/x-c"])
            .with_extensions(["c"])
            .with_probe_markers(["#include ", "typedef ", "int main("], 1, true),
        TreeSitterLanguageConfig::new("cpp", "tree-sitter-cpp", "tree-sitter-cpp", "0.23.4", "cpp")
            .with_display_name("C++")
            .with_aliases(["c++", "cplusplus"])
            .with_media_types(["text/x-c++src"])
            .with_extensions(["cc", "cpp", "cxx", "hpp", "hh", "hxx"])
            .with_probe_markers(["std::", "#include <iostream", "namespace "], 1, true),
        TreeSitterLanguageConfig::new(
            "csharp",
            "tree-sitter-c-sharp",
            "tree-sitter-c-sharp",
            "0.23.5",
            "csharp",
        )
        .with_display_name("C#")
        .with_aliases(["c#", "cs"])
        .with_media_types(["text/x-csharp"])
        .with_extensions(["cs"])
        .with_probe_markers(["using System", "namespace ", "Console."], 1, true),
        TreeSitterLanguageConfig::new(
            "ruby",
            "tree-sitter-ruby",
            "tree-sitter-ruby",
            "0.23.1",
            "ruby",
        )
        .with_display_name("Ruby")
        .with_media_types(["text/x-ruby"])
        .with_extensions(["rb", "rake"])
        .with_probe_markers(["def ", "require ", "attr_"], 1, true),
        TreeSitterLanguageConfig::new("php", "tree-sitter-php", "tree-sitter-php", "0.24.2", "php")
            .with_display_name("PHP")
            .with_media_types(["application/x-httpd-php", "text/x-php"])
            .with_extensions(["php", "phtml"])
            .with_probe_markers(["<?php", "function ", "$"], 1, true),
        TreeSitterLanguageConfig::new(
            "swift",
            "tree-sitter-swift",
            "tree-sitter-swift",
            "0.7.3",
            "swift",
        )
        .with_display_name("Swift")
        .with_media_types(["text/x-swift"])
        .with_extensions(["swift"])
        .with_probe_markers(["import Foundation", "func ", "let "], 2, true),
        TreeSitterLanguageConfig::new(
            "shell",
            "tree-sitter-bash",
            "tree-sitter-bash",
            "0.25.1",
            "bash",
        )
        .with_display_name("Bash")
        .with_aliases(["bash"])
        .with_media_types(["application/x-sh", "text/x-shellscript"])
        .with_extensions(["sh", "bash"])
        .with_probe_markers(
            ["#!/bin/bash", "#!/usr/bin/env bash", "set -e", "$("],
            1,
            true,
        ),
        TreeSitterLanguageConfig::new(
            "sql",
            "tree-sitter-sequel",
            "tree-sitter-sequel",
            "0.3.11",
            "sql",
        )
        .with_display_name("SQL")
        .with_media_types(["application/sql", "text/x-sql"])
        .with_extensions(["sql"])
        .with_probe_markers(
            ["select ", "create table ", "insert into ", "update "],
            1,
            false,
        ),
        TreeSitterLanguageConfig::new("css", "tree-sitter-css", "tree-sitter-css", "0.25.0", "css")
            .with_display_name("CSS")
            .with_media_types(["text/css"])
            .with_extensions(["css"])
            .with_probe_markers(["@media", "color:", "display:", "--"], 1, false),
    ]
}

#[cfg(feature = "secondary-code")]
#[derive(Clone)]
pub struct TreeSitterLanguageAdapter {
    config: TreeSitterLanguageConfig,
    language: tree_sitter::Language,
}

#[cfg(feature = "secondary-code")]
#[derive(Debug, thiserror::Error)]
pub enum TreeSitterAdapterError {
    #[error("tree-sitter grammar for {language} is incompatible: {message}")]
    IncompatibleGrammar { language: String, message: String },
    #[error("tree-sitter returned no parse tree for {language}")]
    MissingTree { language: String },
}

#[cfg(feature = "secondary-code")]
struct ParsedCode {
    file: CodeFile,
}

#[cfg(feature = "secondary-code")]
enum ParseCodeError<E> {
    Adapter(TreeSitterAdapterError),
    Observer(E),
}

#[cfg(feature = "secondary-code")]
impl TreeSitterLanguageAdapter {
    pub fn new(
        config: TreeSitterLanguageConfig,
        language: tree_sitter::Language,
    ) -> Result<Self, TreeSitterAdapterError> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).map_err(|error| {
            TreeSitterAdapterError::IncompatibleGrammar {
                language: config.language.clone(),
                message: error.to_string(),
            }
        })?;
        Ok(Self { config, language })
    }

    pub fn config(&self) -> &TreeSitterLanguageConfig {
        &self.config
    }

    pub fn caller_descriptor(&self) -> ParserDescriptor {
        self.config.descriptor(ParserOrigin::Caller)
    }

    pub(crate) fn builtin_descriptor(&self) -> ParserDescriptor {
        self.config.descriptor(ParserOrigin::BuiltIn)
    }

    pub fn parse_file(
        &self,
        text: &str,
        options: &CodeIngestOptions,
    ) -> Result<CodeFile, TreeSitterAdapterError> {
        self.parse_with_metrics(text, options, |_| Ok::<_, std::convert::Infallible>(()))
            .map(|parsed| parsed.file)
            .map_err(|error| match error {
                ParseCodeError::Adapter(error) => error,
                ParseCodeError::Observer(never) => match never {},
            })
    }

    fn parse_with_metrics<E>(
        &self,
        text: &str,
        options: &CodeIngestOptions,
        mut observe_node: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<ParsedCode, ParseCodeError<E>> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&self.language).map_err(|error| {
            ParseCodeError::Adapter(TreeSitterAdapterError::IncompatibleGrammar {
                language: self.config.language.clone(),
                message: error.to_string(),
            })
        })?;
        let tree = parser.parse(text, None).ok_or_else(|| {
            ParseCodeError::Adapter(TreeSitterAdapterError::MissingTree {
                language: self.config.language.clone(),
            })
        })?;
        let root = tree.root_node();
        let line_index = LineIndex::new(text);
        let mut syntax_nodes = Vec::new();
        let mut parse_errors = Vec::new();
        let mut stack = vec![(root, None, 1usize)];
        while let Some((node, parent, depth)) = stack.pop() {
            observe_node(depth).map_err(ParseCodeError::Observer)?;
            let retain = options.include_anonymous_nodes
                || node.is_named()
                || node.is_error()
                || node.is_missing();
            let current_parent = if retain {
                let id = format!(
                    "code-syntax-{}-{}-{}",
                    syntax_nodes.len(),
                    node.start_byte(),
                    node.end_byte()
                );
                let range = SourceRange::new(node.start_byte(), node.end_byte(), &line_index);
                let raw = text
                    .get(node.start_byte()..node.end_byte())
                    .unwrap_or_default()
                    .to_string();
                if node.is_error() || node.is_missing() {
                    parse_errors.push(CodeParseError {
                        node_kind: node.kind().to_string(),
                        missing: node.is_missing(),
                        raw: raw.clone(),
                        range: range.clone(),
                    });
                }
                syntax_nodes.push(CodeSyntaxNode {
                    id: id.clone(),
                    kind: node.kind().to_string(),
                    named: node.is_named(),
                    error: node.is_error(),
                    missing: node.is_missing(),
                    parent: parent.clone(),
                    raw,
                    range,
                });
                Some(id)
            } else {
                parent
            };
            for index in (0..node.child_count()).rev() {
                if let Some(child) = node.child(index) {
                    stack.push((child, current_parent.clone(), depth + 1));
                }
            }
        }
        Ok(ParsedCode {
            file: CodeFile {
                schema_version: SchemaVersion::CODE_V1.to_string(),
                adapter: self.config.adapter_metadata(),
                root_kind: root.kind().to_string(),
                syntax_nodes,
                parse_errors,
            },
        })
    }
}

#[cfg(feature = "secondary-code")]
impl crate::registry::Parser for TreeSitterLanguageAdapter {
    fn parse(
        &self,
        context: &mut crate::registry::ParserContext<'_>,
    ) -> Result<crate::registry::ParserOutput, crate::registry::ParserError> {
        context.checkpoint()?;
        let options: CodeIngestOptions = serde_json::from_value(context.options().clone())
            .map_err(|error| {
                Box::new(crate::core::Diagnostic::malformed(
                    &self.config.parser_id,
                    error.to_string(),
                ))
            })?;
        let text = context.utf8_text()?;
        context.consume_decoded_characters(text.chars().count() as u64)?;
        let mut visited_nodes = 0usize;
        let mut observed_depth = 0usize;
        let parsed = self
            .parse_with_metrics(text, &options, |depth| {
                context.consume_nodes(1)?;
                visited_nodes = visited_nodes.saturating_add(1);
                if depth > observed_depth {
                    context.observe_nesting_depth(depth as u64)?;
                    observed_depth = depth;
                }
                if visited_nodes % 1024 == 0 {
                    context.checkpoint()?;
                }
                Ok::<_, crate::registry::ParserError>(())
            })
            .map_err(|error| match error {
                ParseCodeError::Adapter(error) => Box::new(crate::core::Diagnostic::parser_defect(
                    &self.config.parser_id,
                    error.to_string(),
                )),
                ParseCodeError::Observer(error) => error,
            })?;
        context.checkpoint()?;
        let diagnostics = parsed
            .file
            .parse_errors
            .iter()
            .map(|error| {
                let (code, message) = if error.missing {
                    (
                        "code.parse.missing_node",
                        format!("missing {} syntax node", error.node_kind),
                    )
                } else {
                    (
                        "code.parse.error_node",
                        format!("grammar recovery produced {} node", error.node_kind),
                    )
                };
                crate::core::Diagnostic::error(&self.config.parser_id, code, message)
                    .with_range(error.range.clone())
                    .with_recovery(crate::core::RecoveryAction::new(
                        crate::core::RecoveryKind::InspectInput,
                        "inspect or repair the malformed source near this range",
                        true,
                    ))
                    .partial()
            })
            .collect::<Vec<_>>();
        let payload = serde_json::to_value(parsed.file).map_err(|error| {
            Box::new(crate::core::Diagnostic::parser_defect(
                &self.config.parser_id,
                error.to_string(),
            ))
        })?;
        if diagnostics.is_empty() {
            Ok(crate::registry::ParserOutput::complete(payload))
        } else {
            Ok(crate::registry::ParserOutput::partial(
                Some(payload),
                diagnostics,
            ))
        }
    }

    fn grammar_probe(&self, text: &str) -> Option<crate::registry::GrammarProbe> {
        if !self.config.marker_matches(text) {
            return None;
        }
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&self.language).ok()?;
        let tree = parser.parse(text, None)?;
        let root = tree.root_node();
        let named_children = root.named_child_count();
        if named_children == 0 {
            return None;
        }
        Some(crate::registry::GrammarProbe {
            format: self.config.language.clone(),
            media_type: self.config.media_types.iter().next().cloned(),
            parser_id: self.config.parser_id.clone(),
            has_error: root.has_error(),
            named_children,
        })
    }
}

#[cfg(all(
    feature = "secondary-code",
    any(
        feature = "go",
        feature = "java",
        feature = "kotlin",
        feature = "c",
        feature = "cpp",
        feature = "csharp",
        feature = "ruby",
        feature = "php",
        feature = "swift",
        feature = "bash",
        feature = "sql",
        feature = "css"
    )
))]
pub(crate) fn enabled_builtin_adapters()
-> Result<Vec<TreeSitterLanguageAdapter>, TreeSitterAdapterError> {
    let configs = builtin_language_configs()
        .into_iter()
        .map(|config| (config.enabled_feature.clone(), config))
        .collect::<BTreeMap<_, _>>();
    let mut adapters = Vec::new();
    macro_rules! enable {
        ($feature:literal, $language:expr) => {{
            let config = configs
                .get($feature)
                .expect("built-in language config")
                .clone();
            adapters.push(TreeSitterLanguageAdapter::new(config, $language.into())?);
        }};
    }
    #[cfg(feature = "go")]
    enable!("go", tree_sitter_go::LANGUAGE);
    #[cfg(feature = "java")]
    enable!("java", tree_sitter_java::LANGUAGE);
    #[cfg(feature = "kotlin")]
    enable!("kotlin", tree_sitter_kotlin::LANGUAGE);
    #[cfg(feature = "c")]
    enable!("c", tree_sitter_c::LANGUAGE);
    #[cfg(feature = "cpp")]
    enable!("cpp", tree_sitter_cpp::LANGUAGE);
    #[cfg(feature = "csharp")]
    enable!("csharp", tree_sitter_c_sharp::LANGUAGE);
    #[cfg(feature = "ruby")]
    enable!("ruby", tree_sitter_ruby::LANGUAGE);
    #[cfg(feature = "php")]
    enable!("php", tree_sitter_php::LANGUAGE_PHP);
    #[cfg(feature = "swift")]
    enable!("swift", tree_sitter_swift::LANGUAGE);
    #[cfg(feature = "bash")]
    enable!("bash", tree_sitter_bash::LANGUAGE);
    #[cfg(feature = "sql")]
    enable!("sql", tree_sitter_sequel::LANGUAGE);
    #[cfg(feature = "css")]
    enable!("css", tree_sitter_css::LANGUAGE);
    Ok(adapters)
}

#[cfg(all(
    feature = "secondary-code",
    not(any(
        feature = "go",
        feature = "java",
        feature = "kotlin",
        feature = "c",
        feature = "cpp",
        feature = "csharp",
        feature = "ruby",
        feature = "php",
        feature = "swift",
        feature = "bash",
        feature = "sql",
        feature = "css"
    ))
))]
pub(crate) fn enabled_builtin_adapters()
-> Result<Vec<TreeSitterLanguageAdapter>, TreeSitterAdapterError> {
    Ok(Vec::new())
}

impl ToDocumentGraph for CodeFile {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(SchemaVersion::CODE_V1, &self.adapter.grammar)
            .map_err(|error| TransformError::Other {
                message: error.to_string(),
            })?;
        let mut graph = DocumentGraph::new(context.graph_id, DocumentKind::Code).with_projection(
            "code",
            SchemaVersion::CODE_V1,
            "grist.code.to-document-graph.v1",
        );
        graph.source = context.source;
        graph.language = Some(
            context
                .language
                .unwrap_or_else(|| self.adapter.language.clone()),
        );
        graph.dialect = context.dialect;
        graph.attrs = context.attrs;
        let mut ids = BTreeMap::new();
        for node in &self.syntax_nodes {
            let id = identities
                .node_id(&ProjectionAddress::native(
                    ["syntax", node.kind.as_str()],
                    &node.id,
                ))
                .map_err(|error| TransformError::Other {
                    message: error.to_string(),
                })?;
            ids.insert(node.id.clone(), id);
        }
        for node in &self.syntax_nodes {
            let id = ids.get(&node.id).expect("syntax id was indexed").clone();
            let parent = node
                .parent
                .as_ref()
                .and_then(|parent| ids.get(parent))
                .cloned();
            let mut projected =
                DocumentNode::new(&id, graph_node_kind(&node.kind, node.error, node.missing))
                    .with_range(node.range.clone())
                    .with_name(&node.kind)
                    .with_text(&node.raw)
                    .with_ordinal(
                        node.id
                            .split('-')
                            .nth(2)
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(0),
                    );
            projected.parent = parent.clone();
            projected
                .attrs
                .insert("tree_sitter_kind".to_string(), node.kind.clone().into());
            projected
                .attrs
                .insert("error".to_string(), node.error.into());
            projected
                .attrs
                .insert("missing".to_string(), node.missing.into());
            graph.add_node(projected);
            if let Some(parent) = parent {
                graph.add_edge(
                    crate::document_graph::DocumentEdge::new(
                        parent,
                        DocumentRelation::Contains,
                        id,
                    )
                    .with_range(node.range.clone()),
                );
            }
        }
        graph
            .finalize_projection(&identities)
            .map_err(|error| TransformError::Other {
                message: error.to_string(),
            })?;
        Ok(graph)
    }
}

fn graph_node_kind(kind: &str, error: bool, missing: bool) -> DocumentNodeKind {
    if error || missing {
        return DocumentNodeKind::Diagnostic;
    }
    let kind = kind.to_ascii_lowercase();
    let kind = kind.as_str();
    if kind.contains("comment") {
        DocumentNodeKind::Comment
    } else if kind.contains("class") {
        DocumentNodeKind::Class
    } else if kind.contains("interface") {
        DocumentNodeKind::Interface
    } else if kind.contains("function") {
        DocumentNodeKind::Function
    } else if kind.contains("method") {
        DocumentNodeKind::Method
    } else if kind.contains("import") || kind.contains("include") {
        DocumentNodeKind::Import
    } else if kind.contains("return") {
        DocumentNodeKind::Return
    } else if kind.contains("call") {
        DocumentNodeKind::Call
    } else if kind.contains("assignment") {
        DocumentNodeKind::Assignment
    } else if kind.contains("namespace") || kind.contains("package") {
        DocumentNodeKind::Namespace
    } else if kind.contains("error") {
        DocumentNodeKind::Diagnostic
    } else {
        DocumentNodeKind::CodeSymbol
    }
}
