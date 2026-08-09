//! Safe, inert LaTeX project parsing.
//!
//! TeX engines, shell primitives, external commands, and remote resources are
//! never executed or fetched. Includes resolve only below explicit roots.

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, Envelope, FormatIdentity, LineIndex, OperationKind,
    OperationStatus, ParserInfo, SchemaVersion, SourceInfo, SourceLocator, SourceRange,
    options_digest, sha256_hex,
};
use crate::decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeReport, DecodedByteRange, DecodedText,
    RawByteRange, TextEncoding, decode_text,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

#[cfg(feature = "pdf")]
mod pdf_association;
#[cfg(feature = "pdf")]
pub use pdf_association::*;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexDocument {
    pub schema_version: String,
    #[serde(default)]
    pub raw_bytes: Vec<u8>,
    #[serde(default)]
    pub decoded_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_range: Option<SourceRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<TextEncoding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoding: Option<DecodeReport>,
    #[serde(default)]
    pub project: LatexProject,
    #[serde(default)]
    pub metadata: BTreeMap<String, Vec<LatexLocatedValue>>,
    #[serde(default)]
    pub macros: Vec<LatexMacroDefinition>,
    pub nodes: Vec<LatexNode>,
    pub parse_errors: Vec<LatexParseError>,
    pub detail: Option<LatexSyntaxDetail>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LatexProject {
    #[serde(default)]
    pub allowed_roots: Vec<String>,
    #[serde(default)]
    pub root_candidates: Vec<LatexRootCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_root: Option<String>,
    #[serde(default)]
    pub root_ambiguous: bool,
    #[serde(default)]
    pub resolved_files: Vec<LatexProjectFile>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatexRootCandidate {
    pub repository_relative_path: String,
    pub score: u16,
    pub has_document_class: bool,
    pub has_document_environment: bool,
    pub referenced_by: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexProjectFile {
    pub repository_relative_path: String,
    pub source: SourceInfo,
    pub content_sha256: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexLocatedValue {
    pub value: String,
    pub source: SourceInfo,
    pub range: SourceRange,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexNode {
    pub id: String,
    pub kind: LatexNodeKind,
    pub range: SourceRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_range: Option<RawByteRange>,
    #[serde(default)]
    pub raw: String,
    pub command: Option<String>,
    pub name: Option<String>,
    pub argument: Option<String>,
    pub text: Option<String>,
    #[serde(default)]
    pub attrs: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<LatexIncludeReference>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LatexNodeKind {
    DocumentClass,
    Metadata,
    MacroDefinition,
    MacroUse,
    Include,
    Section,
    Paragraph,
    Text,
    Command,
    Environment,
    List,
    ListItem,
    Table,
    TableRow,
    TableCell,
    Figure,
    Caption,
    Equation,
    MathInline,
    MathBlock,
    Label,
    Ref,
    Citation,
    Comment,
    RawCommand,
    RawInline,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LatexIncludeStatus {
    Resolved,
    ReferenceOnly,
    RemoteDisabled,
    OutsideAllowedRoots,
    Missing,
    NotFile,
    Cycle,
    DepthExceeded,
    BudgetExceeded,
    DecodeFailed,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexIncludeReference {
    pub target: String,
    pub command: String,
    pub status: LatexIncludeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<LatexResolvedFile>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexResolvedFile {
    pub source: SourceInfo,
    pub raw_bytes: Vec<u8>,
    pub content_sha256: String,
    pub decoded_text: String,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub macros: Vec<LatexMacroDefinition>,
    pub nodes: Vec<LatexNode>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexMacroDefinition {
    pub name: String,
    pub argument_count: u8,
    pub body: String,
    pub source: SourceInfo,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub raw: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LatexExpansionStatus {
    Expanded,
    ArityMismatch,
    Cycle,
    DepthExceeded,
    ExpansionLimit,
    OutputLimit,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexParseError {
    pub range: SourceRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<SourceLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceInfo>,
    pub message: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatexSyntaxDetail {
    pub node_count: usize,
    pub raw_command_count: usize,
    pub included_file_count: usize,
    pub macro_definition_count: usize,
    pub macro_expansion_count: usize,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LatexDetailMode {
    #[default]
    Semantic,
    SemanticWithSyntax,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LatexOptions {
    pub detail: LatexDetailMode,
    pub encoding: Option<String>,
    pub project_root: Option<PathBuf>,
    pub allowed_roots: Vec<PathBuf>,
    pub resolve_includes: bool,
    pub detect_roots: bool,
    pub retain_comments: bool,
    pub max_include_depth: u16,
    pub max_include_bytes: u64,
    pub max_project_files: u32,
    pub max_project_bytes: u64,
    pub max_macro_depth: u16,
    pub max_macro_expansions: u32,
    pub max_expanded_characters: u64,
}

impl Default for LatexOptions {
    fn default() -> Self {
        Self {
            detail: LatexDetailMode::Semantic,
            encoding: None,
            project_root: None,
            allowed_roots: Vec::new(),
            resolve_includes: true,
            detect_roots: true,
            retain_comments: true,
            max_include_depth: 16,
            max_include_bytes: 8 * 1024 * 1024,
            max_project_files: 4096,
            max_project_bytes: 64 * 1024 * 1024,
            max_macro_depth: 32,
            max_macro_expansions: 10_000,
            max_expanded_characters: 4 * 1024 * 1024,
        }
    }
}

impl crate::core::FormatOptions for LatexOptions {
    const FORMAT: &'static str = "latex";
}

pub type LatexEnvelope = Envelope<LatexDocument>;

pub fn parse_latex(text: &str, source: SourceInfo, options: &LatexOptions) -> LatexEnvelope {
    parse_latex_bytes(text.as_bytes(), source, options)
}

pub fn parse_latex_bytes(
    bytes: &[u8],
    source: SourceInfo,
    options: &LatexOptions,
) -> LatexEnvelope {
    let mut decode_options =
        DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("latex"));
    decode_options.context = DecodeContext::PlainText;
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    match decode_text(bytes, &decode_options) {
        Ok(decoded) => envelope_from_decoded(&decoded, source, options),
        Err(error) => failed_decode_envelope(bytes, source, options, error),
    }
}

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new("grist.latex")
        .with_implementation("grist-safe-latex", env!("CARGO_PKG_VERSION"))
        .with_feature("latex")
}

fn failed_decode_envelope(
    bytes: &[u8],
    source: SourceInfo,
    options: &LatexOptions,
    error: DecodeError,
) -> LatexEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Latex,
        OperationStatus::Failed,
        source,
        parser_info(),
        options_digest(options).expect("options serialize"),
        SchemaVersion::LATEX_V1,
    )
    .expect("valid failed status")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes)
            .with_format(FormatIdentity::new("latex", Some("application/x-latex"))),
    )
    .with_diagnostics(vec![error.diagnostic().with_parser("grist.latex")])
}

fn envelope_from_decoded(
    decoded: &DecodedText,
    source: SourceInfo,
    options: &LatexOptions,
) -> LatexEnvelope {
    let mut state = ParseState::new(options, &source);
    let parsed = parse_file(decoded, &source, 0, &mut state);
    let index = LineIndex::new(&decoded.text);
    let whole = SourceRange::new(0, decoded.text.len(), &index);
    let payload = LatexDocument {
        schema_version: SchemaVersion::LATEX_V1.to_string(),
        raw_bytes: decoded.raw_bytes().to_vec(),
        decoded_text: decoded.text.clone(),
        decoded_range: Some(whole.clone()),
        locator: Some(SourceLocator::exact(whole).expect("exact source")),
        encoding: Some(decoded.report.encoding.clone()),
        decoding: Some(decoded.report.clone()),
        project: state.project,
        metadata: parsed.metadata,
        macros: parsed.macros,
        nodes: parsed.nodes,
        parse_errors: state.parse_errors,
        detail: None,
    };
    finish_envelope(decoded, source, options, payload, state.diagnostics)
}

fn finish_envelope(
    decoded: &DecodedText,
    source: SourceInfo,
    options: &LatexOptions,
    mut payload: LatexDocument,
    mut diagnostics: Vec<Diagnostic>,
) -> LatexEnvelope {
    if options.detail == LatexDetailMode::SemanticWithSyntax {
        payload.detail = Some(LatexSyntaxDetail {
            node_count: count_nodes(&payload.nodes),
            raw_command_count: count_kind(&payload.nodes, LatexNodeKind::RawCommand),
            included_file_count: payload.project.resolved_files.len(),
            macro_definition_count: payload.macros.len(),
            macro_expansion_count: count_kind(&payload.nodes, LatexNodeKind::MacroUse),
        });
    }
    let partial = decoded.report.makes_operation_partial() || diagnostics.iter().any(|d| d.partial);
    let digest = options_digest(options).expect("LaTeX options serialize");
    let mut all = decoded.report.diagnostics.clone();
    all.append(&mut diagnostics);
    let mut envelope = if partial {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Latex,
            source,
            parser_info(),
            digest,
            SchemaVersion::LATEX_V1,
            Some(payload),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Latex,
            source,
            parser_info(),
            digest,
            SchemaVersion::LATEX_V1,
            payload,
        )
    };
    envelope.diagnostics = all;
    envelope
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    envelope
        .with_identity(
            ContentIdentity::for_raw_bytes(decoded.raw_bytes())
                .with_decoded(
                    &decoded.text,
                    decoded.report.encoding.label(),
                    decoded.report.is_lossy(),
                )
                .with_format(FormatIdentity::new("latex", Some("application/x-latex"))),
        )
        .with_canonical_payload_identity()
        .expect("canonical LaTeX payload")
}

struct ParsedFile {
    nodes: Vec<LatexNode>,
    macros: Vec<LatexMacroDefinition>,
    metadata: BTreeMap<String, Vec<LatexLocatedValue>>,
}

struct ParseState<'a> {
    options: &'a LatexOptions,
    roots: Vec<PathBuf>,
    stack: BTreeSet<PathBuf>,
    included_bytes: u64,
    expansion_count: u32,
    macro_defs: BTreeMap<String, LatexMacroDefinition>,
    diagnostics: Vec<Diagnostic>,
    parse_errors: Vec<LatexParseError>,
    project: LatexProject,
}

impl<'a> ParseState<'a> {
    fn new(options: &'a LatexOptions, source: &SourceInfo) -> Self {
        let (roots, mut diagnostics) = canonical_roots(options);
        let (mut project, root_diagnostics) = if options.detect_roots {
            detect_project(options)
        } else {
            (LatexProject::default(), Vec::new())
        };
        diagnostics.extend(root_diagnostics);
        project.allowed_roots = roots
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        let mut stack = BTreeSet::new();
        if let Some(path) = source
            .path
            .as_deref()
            .and_then(|p| fs::canonicalize(p).ok())
        {
            stack.insert(path);
        }
        Self {
            options,
            roots,
            stack,
            included_bytes: 0,
            expansion_count: 0,
            macro_defs: BTreeMap::new(),
            diagnostics,
            parse_errors: Vec::new(),
            project,
        }
    }
}

fn canonical_roots(options: &LatexOptions) -> (Vec<PathBuf>, Vec<Diagnostic>) {
    let candidates = options
        .allowed_roots
        .iter()
        .chain(options.project_root.iter());
    let mut roots = Vec::new();
    let mut diagnostics = Vec::new();
    for candidate in candidates {
        match fs::canonicalize(candidate) {
            Ok(path) if path.is_dir() => roots.push(path),
            Ok(_) => diagnostics.push(
                Diagnostic::warning(
                    "grist.latex.project",
                    "latex.project.root_not_directory",
                    "allowed root is not a directory",
                )
                .partial(),
            ),
            Err(error) => diagnostics.push(
                Diagnostic::warning(
                    "grist.latex.project",
                    "latex.project.root_unavailable",
                    error.to_string(),
                )
                .partial(),
            ),
        }
    }
    roots.sort();
    roots.dedup();
    (roots, diagnostics)
}

pub fn detect_latex_roots(options: &LatexOptions) -> (LatexProject, Vec<Diagnostic>) {
    detect_project(options)
}

fn detect_project(options: &LatexOptions) -> (LatexProject, Vec<Diagnostic>) {
    let (roots, mut diagnostics) = canonical_roots(options);
    let mut files = Vec::new();
    let mut bytes_seen = 0_u64;
    for root in &roots {
        collect_tex_files(root, root, &mut files, options.max_project_files as usize);
    }
    files.sort();
    files.dedup();
    let mut contents = BTreeMap::new();
    for (root, path) in files {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_file() {
            continue;
        }
        bytes_seen = bytes_seen.saturating_add(metadata.len());
        if bytes_seen > options.max_project_bytes {
            diagnostics.push(
                Diagnostic::warning(
                    "grist.latex.project",
                    "latex.project.scan_budget_exceeded",
                    "project root detection byte budget was exceeded",
                )
                .partial(),
            );
            break;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            let relative = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            contents.insert(relative, text);
        }
    }
    let mut referenced: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (source, text) in &contents {
        for target in include_targets(text) {
            let normalized = normalize_tex_target(&target);
            referenced
                .entry(normalized)
                .or_default()
                .push(source.clone());
        }
    }
    let mut candidates = Vec::new();
    for (path, text) in &contents {
        let class = text.contains("\\documentclass");
        let document = text.contains("\\begin{document}");
        if !class && !document {
            continue;
        }
        let refs = referenced.get(path).cloned().unwrap_or_default();
        let score =
            u16::from(class) * 60 + u16::from(document) * 40 + u16::from(refs.is_empty()) * 10;
        candidates.push(LatexRootCandidate {
            repository_relative_path: path.clone(),
            score,
            has_document_class: class,
            has_document_environment: document,
            referenced_by: refs,
        });
    }
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.repository_relative_path.cmp(&b.repository_relative_path))
    });
    let ambiguous = candidates.get(1).is_some_and(|next| {
        candidates
            .first()
            .is_some_and(|first| next.score == first.score)
    });
    let selected = (!ambiguous)
        .then(|| {
            candidates
                .first()
                .map(|c| c.repository_relative_path.clone())
        })
        .flatten();
    if ambiguous {
        diagnostics.push(
            Diagnostic::warning(
                "grist.latex.project",
                "latex.project.root_ambiguous",
                "multiple LaTeX project roots have the same score",
            )
            .partial(),
        );
    }
    (
        LatexProject {
            allowed_roots: roots
                .iter()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect(),
            root_candidates: candidates,
            selected_root: selected,
            root_ambiguous: ambiguous,
            resolved_files: Vec::new(),
        },
        diagnostics,
    )
}

fn collect_tex_files(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if out.len() >= limit {
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_tex_files(root, &path, out, limit);
        } else if path
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("tex"))
        {
            out.push((root.to_path_buf(), path));
        }
    }
}

pub fn parse_latex_project(root: &Path, options: &LatexOptions) -> LatexEnvelope {
    let mut selected_options = options.clone();
    if selected_options.allowed_roots.is_empty() && selected_options.project_root.is_none() {
        selected_options.allowed_roots.push(root.to_path_buf());
    }
    let (project, diagnostics) = detect_project(&selected_options);
    let Some(selected) = project.selected_root.as_ref() else {
        return failed_project(
            root,
            &selected_options,
            diagnostics,
            "no unambiguous LaTeX root",
        );
    };
    let path = canonical_roots(&selected_options)
        .0
        .into_iter()
        .find_map(|allowed| {
            let candidate = allowed.join(selected);
            candidate.is_file().then_some(candidate)
        });
    let Some(path) = path else {
        return failed_project(
            root,
            &selected_options,
            diagnostics,
            "detected root unavailable",
        );
    };
    match fs::read(&path) {
        Ok(bytes) => parse_latex_bytes(&bytes, SourceInfo::from_path(&path), &selected_options),
        Err(error) => failed_project(root, &selected_options, diagnostics, &error.to_string()),
    }
}

fn failed_project(
    root: &Path,
    options: &LatexOptions,
    mut diagnostics: Vec<Diagnostic>,
    message: &str,
) -> LatexEnvelope {
    diagnostics.push(Diagnostic::error(
        "grist.latex.project",
        "latex.project.root_not_found",
        message,
    ));
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Latex,
        OperationStatus::Failed,
        SourceInfo::from_path(root),
        parser_info(),
        options_digest(options).expect("options serialize"),
        SchemaVersion::LATEX_V1,
    )
    .expect("valid status")
    .with_diagnostics(diagnostics)
}

struct NodeBuilder<'a> {
    decoded: &'a DecodedText,
    source: &'a SourceInfo,
    index: LineIndex,
    nodes: Vec<LatexNode>,
}

impl<'a> NodeBuilder<'a> {
    fn new(decoded: &'a DecodedText, source: &'a SourceInfo) -> Self {
        Self {
            decoded,
            source,
            index: LineIndex::new(&decoded.text),
            nodes: Vec::new(),
        }
    }

    fn push(&mut self, kind: LatexNodeKind, range: Range<usize>, parent: Option<usize>) -> usize {
        let range =
            range.start.min(self.decoded.text.len())..range.end.min(self.decoded.text.len());
        let source_range = SourceRange::new(range.start, range.end, &self.index);
        let id = format!("latex-node-{:06}", self.nodes.len());
        let parent_id = parent.map(|p| self.nodes[p].id.clone());
        self.nodes.push(LatexNode {
            id,
            kind,
            range: source_range.clone(),
            locator: Some(SourceLocator::exact(source_range).expect("exact LaTeX range")),
            source: Some(self.source.clone()),
            raw_range: self.decoded.raw_range_for_decoded(DecodedByteRange {
                start: range.start as u64,
                end: range.end as u64,
            }),
            raw: self.decoded.text[range].to_string(),
            command: None,
            name: None,
            argument: None,
            text: None,
            attrs: BTreeMap::new(),
            parent_id,
            children: Vec::new(),
            include: None,
        });
        let index = self.nodes.len() - 1;
        if let Some(parent) = parent {
            let child = self.nodes[index].id.clone();
            self.nodes[parent].children.push(child);
        }
        index
    }
}

#[derive(Clone)]
struct CommandToken {
    name: String,
    range: Range<usize>,
    arguments: Vec<String>,
}

fn command_at(text: &str, start: usize) -> Option<CommandToken> {
    if text.as_bytes().get(start) != Some(&b'\\') {
        return None;
    }
    let mut end = start + 1;
    while text
        .as_bytes()
        .get(end)
        .is_some_and(u8::is_ascii_alphabetic)
    {
        end += 1;
    }
    if end == start + 1 {
        end = (end + 1).min(text.len());
    }
    let name = text[start + 1..end].to_string();
    let mut cursor = end;
    if name == "def" {
        while text
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        if text.as_bytes().get(cursor) != Some(&b'\\') {
            return None;
        }
        let macro_start = cursor;
        cursor += 1;
        while text
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            cursor += 1;
        }
        let macro_name = text[macro_start..cursor].to_string();
        while text.as_bytes().get(cursor) != Some(&b'{') {
            cursor += 1;
            if cursor >= text.len() {
                return None;
            }
        }
        let close = balanced_end(text, cursor, b'{', b'}')?;
        return Some(CommandToken {
            name,
            range: start..close,
            arguments: vec![macro_name, text[cursor + 1..close - 1].to_string()],
        });
    }
    let mut arguments = Vec::new();
    loop {
        while text
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        if text.as_bytes().get(cursor) == Some(&b'[') {
            let close = balanced_end(text, cursor, b'[', b']')?;
            cursor = close;
            continue;
        }
        if text.as_bytes().get(cursor) != Some(&b'{') {
            break;
        }
        let close = balanced_end(text, cursor, b'{', b'}')?;
        arguments.push(text[cursor + 1..close - 1].to_string());
        cursor = close;
    }
    Some(CommandToken {
        name,
        range: start..cursor,
        arguments,
    })
}

fn balanced_end(text: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0_u32;
    let mut escaped = false;
    for (offset, byte) in text.as_bytes()[start..].iter().copied().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == open {
            depth += 1;
        }
        if byte == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(start + offset + 1);
            }
        }
    }
    None
}

fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            ranges.push(start..index + 1);
            start = index + 1;
        }
    }
    if start < text.len() || text.is_empty() {
        ranges.push(start..text.len());
    }
    ranges
}

fn parse_file(
    decoded: &DecodedText,
    source: &SourceInfo,
    depth: u16,
    state: &mut ParseState<'_>,
) -> ParsedFile {
    let text = &decoded.text;
    let mut builder = NodeBuilder::new(decoded, source);
    let mut macros = Vec::new();
    let mut metadata: BTreeMap<String, Vec<LatexLocatedValue>> = BTreeMap::new();

    for line in line_ranges(text) {
        let raw = &text[line.clone()];
        let content_end = unescaped_comment(raw).unwrap_or(raw.len());
        let content = &raw[..content_end];
        let trimmed = content.trim();
        if let Some(comment) = unescaped_comment(raw) {
            if state.options.retain_comments {
                let ci = builder.push(LatexNodeKind::Comment, line.start + comment..line.end, None);
                builder.nodes[ci].text = Some(raw[comment + 1..].trim().to_string());
            }
        }
        if trimmed.is_empty() {
            continue;
        }
        let start = line.start + content.find(trimmed).unwrap_or(0);
        let index = builder.push(
            LatexNodeKind::Paragraph,
            start..line.start + content_end,
            None,
        );
        builder.nodes[index].text = Some(trimmed.to_string());
    }

    let mut cursor = 0;
    while cursor < text.len() {
        let Some(relative) = text[cursor..].find('\\') else {
            break;
        };
        let start = cursor + relative;
        let Some(token) = command_at(text, start) else {
            let end = text[start..]
                .find(char::is_whitespace)
                .map(|n| start + n)
                .unwrap_or(text.len());
            let index = builder.push(LatexNodeKind::RawInline, start..end.max(start + 1), None);
            builder.nodes[index].text = Some(text[start..end.max(start + 1)].to_string());
            malformed(
                state,
                source,
                &builder.nodes[index],
                "malformed or unclosed LaTeX command",
            );
            cursor = end.max(start + 1);
            continue;
        };
        cursor = token.range.end.max(start + 1);
        if inside_comment(text, start) {
            continue;
        }
        if token.name == "end" {
            continue;
        }

        if token.name == "begin" {
            let env = token.arguments.first().cloned().unwrap_or_default();
            let end_marker = format!("\\end{{{env}}}");
            let full_end = text[token.range.end..]
                .find(&end_marker)
                .map(|n| token.range.end + n + end_marker.len());
            let range = start..full_end.unwrap_or(token.range.end);
            let kind = environment_kind(&env);
            let parent = builder.push(kind.clone(), range.clone(), None);
            builder.nodes[parent].command = Some("begin".into());
            builder.nodes[parent].name = Some(env.clone());
            builder.nodes[parent].text =
                Some(environment_body(text, &token, full_end, &end_marker));
            if full_end.is_none() {
                malformed(
                    state,
                    source,
                    &builder.nodes[parent],
                    &format!("environment {env} is not closed"),
                );
            }
            if kind == LatexNodeKind::Table {
                add_table_nodes(
                    &mut builder,
                    parent,
                    token.range.end..full_end.unwrap_or(token.range.end),
                    &end_marker,
                );
            }
            continue;
        }

        let kind = command_kind(&token.name, &state.macro_defs);
        let index = builder.push(kind.clone(), token.range.clone(), None);
        builder.nodes[index].command = Some(token.name.clone());
        builder.nodes[index].name = token.arguments.first().cloned();
        builder.nodes[index].argument = token.arguments.first().cloned();
        builder.nodes[index].text = token.arguments.first().cloned();

        if matches!(
            token.name.as_str(),
            "documentclass" | "usepackage" | "title" | "author" | "date" | "thanks" | "institute"
        ) {
            let value = token.arguments.last().cloned().unwrap_or_default();
            let located = LatexLocatedValue {
                value: value.clone(),
                source: source.clone(),
                range: builder.nodes[index].range.clone(),
                locator: builder.nodes[index].locator.clone().expect("node locator"),
            };
            metadata
                .entry(token.name.clone())
                .or_default()
                .push(located);
            builder.nodes[index].text = Some(value);
        }

        if matches!(
            token.name.as_str(),
            "newcommand" | "renewcommand" | "providecommand" | "def"
        ) {
            if let Some(definition) = macro_definition(text, source, &token, &builder.nodes[index])
            {
                builder.nodes[index].name = Some(definition.name.clone());
                builder.nodes[index].text = Some(definition.body.clone());
                state
                    .macro_defs
                    .insert(definition.name.clone(), definition.clone());
                macros.push(definition);
            } else {
                malformed(
                    state,
                    source,
                    &builder.nodes[index],
                    "malformed macro definition",
                );
            }
        } else if matches!(token.name.as_str(), "input" | "include") {
            let target = token.arguments.first().cloned().unwrap_or_default();
            let include = resolve_include(
                &token.name,
                target,
                &builder.nodes[index],
                source,
                depth,
                state,
            );
            builder.nodes[index].include = Some(include);
        } else if kind == LatexNodeKind::MacroUse {
            apply_macro_expansion(&mut builder.nodes[index], &token, state);
        } else if is_active_command(&token.name) {
            builder.nodes[index].attrs.insert(
                "inert_security_class".into(),
                Value::String("active_tex_primitive".into()),
            );
            state.diagnostics.push(
                Diagnostic::warning(
                    "grist.latex.security",
                    "latex.security.active_command_inert",
                    format!(
                        "active TeX command \\{} was retained without execution",
                        token.name
                    ),
                )
                .with_range(builder.nodes[index].range.clone())
                .with_locator(builder.nodes[index].locator.clone().expect("locator")),
            );
        }
    }
    add_math_nodes(&mut builder, state);
    ParsedFile {
        nodes: builder.nodes,
        macros,
        metadata,
    }
}

fn command_kind(name: &str, macros: &BTreeMap<String, LatexMacroDefinition>) -> LatexNodeKind {
    if macros.contains_key(name) {
        return LatexNodeKind::MacroUse;
    }
    match name {
        "documentclass" => LatexNodeKind::DocumentClass,
        "title" | "author" | "date" | "thanks" | "institute" | "usepackage" => {
            LatexNodeKind::Metadata
        }
        "newcommand" | "renewcommand" | "providecommand" | "def" => LatexNodeKind::MacroDefinition,
        "input" | "include" => LatexNodeKind::Include,
        "part" | "chapter" | "section" | "subsection" | "subsubsection" | "paragraph"
        | "subparagraph" => LatexNodeKind::Section,
        "item" => LatexNodeKind::ListItem,
        "caption" => LatexNodeKind::Caption,
        "label" => LatexNodeKind::Label,
        "ref" | "pageref" | "eqref" | "autoref" | "cref" | "Cref" => LatexNodeKind::Ref,
        value if value.starts_with("cite") || value == "nocite" => LatexNodeKind::Citation,
        "textbf" | "textit" | "emph" | "underline" | "url" | "href" | "includegraphics" => {
            LatexNodeKind::Command
        }
        value if is_active_command(value) => LatexNodeKind::RawCommand,
        _ => LatexNodeKind::RawCommand,
    }
}

fn environment_kind(name: &str) -> LatexNodeKind {
    match name.trim_end_matches('*') {
        "itemize" | "enumerate" | "description" => LatexNodeKind::List,
        "tabular" | "tabularx" | "longtable" | "array" => LatexNodeKind::Table,
        "figure" | "wrapfigure" => LatexNodeKind::Figure,
        "equation" | "align" | "gather" | "multline" | "displaymath" | "math" => {
            LatexNodeKind::Equation
        }
        _ => LatexNodeKind::Environment,
    }
}

fn resolve_include(
    command: &str,
    target: String,
    node: &LatexNode,
    source: &SourceInfo,
    depth: u16,
    state: &mut ParseState<'_>,
) -> LatexIncludeReference {
    let unresolved = |status| LatexIncludeReference {
        target: target.clone(),
        command: command.to_string(),
        status,
        resolved_path: None,
        resolved: None,
    };
    let mut reject = |status, code: &'static str, message: String| {
        state.diagnostics.push(
            Diagnostic::warning("grist.latex.include", code, message)
                .with_range(node.range.clone())
                .with_locator(node.locator.clone().expect("locator"))
                .partial(),
        );
        unresolved(status)
    };
    if !state.options.resolve_includes {
        return reject(
            LatexIncludeStatus::ReferenceOnly,
            "latex.include.resolution_disabled",
            "include resolution is disabled".into(),
        );
    }
    if looks_remote(&target) {
        return reject(
            LatexIncludeStatus::RemoteDisabled,
            "latex.include.remote_disabled",
            "remote include retained without a network request".into(),
        );
    }
    if state.roots.is_empty() {
        return reject(
            LatexIncludeStatus::ReferenceOnly,
            "latex.include.allowed_root_required",
            "include retained because no allowed root was supplied".into(),
        );
    }
    if depth >= state.options.max_include_depth {
        return reject(
            LatexIncludeStatus::DepthExceeded,
            "latex.include.depth_exceeded",
            "include depth limit exceeded".into(),
        );
    }
    let normalized = normalize_tex_target(&target);
    let base = source
        .path
        .as_deref()
        .and_then(|p| fs::canonicalize(p).ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| state.roots[0].clone());
    let candidate = if Path::new(&normalized).is_absolute() {
        PathBuf::from(&normalized)
    } else {
        base.join(&normalized)
    };
    let canonical = match fs::canonicalize(candidate) {
        Ok(path) => path,
        Err(error) => {
            return reject(
                LatexIncludeStatus::Missing,
                "latex.include.not_found",
                error.to_string(),
            );
        }
    };
    let Some(root) = state
        .roots
        .iter()
        .find(|root| canonical.starts_with(root))
        .cloned()
    else {
        return reject(
            LatexIncludeStatus::OutsideAllowedRoots,
            "latex.include.outside_allowed_roots",
            "include resolves outside every allowed root".into(),
        );
    };
    if !canonical.is_file() {
        return reject(
            LatexIncludeStatus::NotFile,
            "latex.include.not_file",
            "include target is not a regular file".into(),
        );
    }
    let relative = canonical
        .strip_prefix(&root)
        .unwrap_or(&canonical)
        .to_string_lossy()
        .replace('\\', "/");
    if !state.stack.insert(canonical.clone()) {
        let mut result = reject(
            LatexIncludeStatus::Cycle,
            "latex.include.cycle",
            format!("include cycle at {relative}"),
        );
        result.resolved_path = Some(relative);
        return result;
    }
    let bytes = match fs::read(&canonical) {
        Ok(bytes) => bytes,
        Err(error) => {
            state.stack.remove(&canonical);
            return reject(
                LatexIncludeStatus::Missing,
                "latex.include.not_found",
                error.to_string(),
            );
        }
    };
    state.included_bytes = state.included_bytes.saturating_add(bytes.len() as u64);
    if bytes.len() as u64 > state.options.max_include_bytes
        || state.included_bytes > state.options.max_project_bytes
    {
        state.stack.remove(&canonical);
        let mut result = reject(
            LatexIncludeStatus::BudgetExceeded,
            "latex.include.budget_exceeded",
            "include byte budget exceeded".into(),
        );
        result.resolved_path = Some(relative);
        return result;
    }
    let mut decode_options =
        DecodeOptions::for_media_type(Some("application/x-latex"), Some("latex"));
    decode_options.context = DecodeContext::PlainText;
    if let Some(encoding) = &state.options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    let decoded = match decode_text(&bytes, &decode_options) {
        Ok(value) => value,
        Err(error) => {
            state.stack.remove(&canonical);
            return reject(
                LatexIncludeStatus::DecodeFailed,
                "latex.include.decode_failed",
                error.to_string(),
            );
        }
    };
    let child_source = SourceInfo::new(canonical.file_name().unwrap_or_default().to_string_lossy())
        .with_path(&canonical)
        .with_repository_relative_path(relative.clone())
        .with_parent(source.clone());
    let child = parse_file(&decoded, &child_source, depth + 1, state);
    state.stack.remove(&canonical);
    state.project.resolved_files.push(LatexProjectFile {
        repository_relative_path: relative.clone(),
        source: child_source.clone(),
        content_sha256: sha256_hex(&bytes),
    });
    LatexIncludeReference {
        target,
        command: command.to_string(),
        status: LatexIncludeStatus::Resolved,
        resolved_path: Some(relative),
        resolved: Some(LatexResolvedFile {
            source: child_source,
            raw_bytes: bytes.clone(),
            content_sha256: sha256_hex(&bytes),
            decoded_text: decoded.text,
            encoding: decoded.report.encoding.clone(),
            decoding: decoded.report,
            macros: child.macros,
            nodes: child.nodes,
        }),
    }
}

fn normalize_tex_target(target: &str) -> String {
    let target = target.trim().replace('\\', "/");
    if Path::new(&target).extension().is_none() {
        format!("{target}.tex")
    } else {
        target
    }
}

fn looks_remote(target: &str) -> bool {
    let lower = target.trim().to_ascii_lowercase();
    lower.contains("://") || lower.starts_with("file:") || lower.starts_with("//")
}

fn include_targets(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find('\\') {
        let start = cursor + relative;
        if let Some(token) = command_at(text, start) {
            if matches!(token.name.as_str(), "input" | "include") {
                if let Some(target) = token.arguments.first() {
                    out.push(target.clone());
                }
            }
            cursor = token.range.end.max(start + 1);
        } else {
            cursor = start + 1;
        }
    }
    out
}

fn macro_definition(
    text: &str,
    source: &SourceInfo,
    token: &CommandToken,
    node: &LatexNode,
) -> Option<LatexMacroDefinition> {
    let name = token
        .arguments
        .first()?
        .trim()
        .trim_start_matches('\\')
        .to_string();
    if name.is_empty() {
        return None;
    }
    let body = token.arguments.last()?.clone();
    let raw = &text[token.range.clone()];
    let argument_count = if token.name == "def" {
        (1..=9)
            .rev()
            .find(|index| raw.contains(&format!("#{index}")))
            .unwrap_or(0)
    } else {
        raw.find('[')
            .and_then(|start| {
                raw[start + 1..]
                    .find(']')
                    .and_then(|end| raw[start + 1..start + 1 + end].parse::<u8>().ok())
            })
            .unwrap_or(0)
    };
    Some(LatexMacroDefinition {
        name,
        argument_count,
        body,
        source: source.clone(),
        range: node.range.clone(),
        locator: node.locator.clone().expect("locator"),
        raw: node.raw.clone(),
    })
}

fn apply_macro_expansion(node: &mut LatexNode, token: &CommandToken, state: &mut ParseState<'_>) {
    let Some(definition) = state.macro_defs.get(&token.name).cloned() else {
        return;
    };
    let mut status = LatexExpansionStatus::Expanded;
    let expanded = if token.arguments.len() != definition.argument_count as usize {
        status = LatexExpansionStatus::ArityMismatch;
        None
    } else {
        let mut value = definition.body.clone();
        for (index, argument) in token.arguments.iter().enumerate() {
            value = value.replace(&format!("#{}", index + 1), argument);
        }
        let mut stack = vec![token.name.clone()];
        match expand_text(&value, state, &mut stack, 1) {
            Ok(value) => Some(value),
            Err(error) => {
                status = error;
                None
            }
        }
    };
    node.attrs.insert(
        "expansion_status".into(),
        serde_json::to_value(&status).unwrap_or(Value::Null),
    );
    if let Some(expanded) = expanded {
        node.attrs
            .insert("expanded_text".into(), Value::String(expanded.clone()));
        node.text = Some(expanded);
    } else {
        state.diagnostics.push(
            Diagnostic::warning(
                "grist.latex.macro",
                "latex.macro.expansion_limited",
                format!(
                    "macro \\{} could not be fully expanded: {status:?}",
                    token.name
                ),
            )
            .with_range(node.range.clone())
            .with_locator(node.locator.clone().expect("locator"))
            .partial(),
        );
    }
}

fn expand_text(
    text: &str,
    state: &mut ParseState<'_>,
    stack: &mut Vec<String>,
    depth: u16,
) -> Result<String, LatexExpansionStatus> {
    if depth > state.options.max_macro_depth {
        return Err(LatexExpansionStatus::DepthExceeded);
    }
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find('\\') {
        let start = cursor + relative;
        output.push_str(&text[cursor..start]);
        let Some(token) = command_at(text, start) else {
            output.push('\\');
            cursor = start + 1;
            continue;
        };
        let Some(definition) = state.macro_defs.get(&token.name).cloned() else {
            output.push_str(&text[token.range.clone()]);
            cursor = token.range.end;
            continue;
        };
        if stack.contains(&token.name) {
            return Err(LatexExpansionStatus::Cycle);
        }
        if token.arguments.len() != definition.argument_count as usize {
            return Err(LatexExpansionStatus::ArityMismatch);
        }
        state.expansion_count = state.expansion_count.saturating_add(1);
        if state.expansion_count > state.options.max_macro_expansions {
            return Err(LatexExpansionStatus::ExpansionLimit);
        }
        let mut body = definition.body;
        for (index, argument) in token.arguments.iter().enumerate() {
            body = body.replace(&format!("#{}", index + 1), argument);
        }
        stack.push(token.name);
        output.push_str(&expand_text(&body, state, stack, depth + 1)?);
        stack.pop();
        if output.chars().count() as u64 > state.options.max_expanded_characters {
            return Err(LatexExpansionStatus::OutputLimit);
        }
        cursor = token.range.end;
    }
    output.push_str(&text[cursor..]);
    if output.chars().count() as u64 > state.options.max_expanded_characters {
        Err(LatexExpansionStatus::OutputLimit)
    } else {
        Ok(output)
    }
}

fn environment_body(
    text: &str,
    token: &CommandToken,
    full_end: Option<usize>,
    marker: &str,
) -> String {
    let end = full_end
        .map(|end| end.saturating_sub(marker.len()))
        .unwrap_or(token.range.end);
    text[token.range.end..end].trim().to_string()
}

fn add_table_nodes(builder: &mut NodeBuilder<'_>, table: usize, body: Range<usize>, marker: &str) {
    let end = body.end.saturating_sub(marker.len()).max(body.start);
    let body_text = &builder.decoded.text[body.start..end.min(builder.decoded.text.len())];
    let mut offset = 0;
    for row in body_text.split("\\\\") {
        let trimmed = row.trim();
        if trimmed.is_empty() {
            offset += row.len() + 2;
            continue;
        }
        let row_start = body.start + offset + row.find(trimmed).unwrap_or(0);
        let row_index = builder.push(
            LatexNodeKind::TableRow,
            row_start..row_start + trimmed.len(),
            Some(table),
        );
        builder.nodes[row_index].text = Some(trimmed.to_string());
        let mut cell_offset = 0;
        for cell in trimmed.split('&') {
            let value = cell.trim();
            let relative = trimmed[cell_offset..].find(value).unwrap_or(0) + cell_offset;
            let cell_index = builder.push(
                LatexNodeKind::TableCell,
                row_start + relative..row_start + relative + value.len(),
                Some(row_index),
            );
            builder.nodes[cell_index].text = Some(value.to_string());
            cell_offset = relative
                + value.len()
                + usize::from(trimmed[relative + value.len()..].starts_with('&'));
        }
        offset += row.len() + 2;
    }
}

fn add_math_nodes(builder: &mut NodeBuilder<'_>, state: &mut ParseState<'_>) {
    let text = builder.decoded.text.clone();
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'$' || (cursor > 0 && bytes[cursor - 1] == b'\\') {
            cursor += 1;
            continue;
        }
        let block = bytes.get(cursor + 1) == Some(&b'$');
        let delimiter = if block { "$$" } else { "$" };
        let content_start = cursor + delimiter.len();
        let Some(relative) = text[content_start..].find(delimiter) else {
            let index = builder.push(
                if block {
                    LatexNodeKind::MathBlock
                } else {
                    LatexNodeKind::MathInline
                },
                cursor..text.len(),
                None,
            );
            malformed(
                state,
                builder.source,
                &builder.nodes[index],
                "unclosed math delimiter",
            );
            break;
        };
        let end_start = content_start + relative;
        let index = builder.push(
            if block {
                LatexNodeKind::MathBlock
            } else {
                LatexNodeKind::MathInline
            },
            cursor..end_start + delimiter.len(),
            None,
        );
        builder.nodes[index].text = Some(text[content_start..end_start].to_string());
        cursor = end_start + delimiter.len();
    }
}

fn unescaped_comment(line: &str) -> Option<usize> {
    line.char_indices().find_map(|(index, ch)| {
        if ch != '%' {
            return None;
        }
        let slashes = line[..index]
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\\')
            .count();
        (slashes % 2 == 0).then_some(index)
    })
}

fn inside_comment(text: &str, offset: usize) -> bool {
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    unescaped_comment(&text[line_start..offset]).is_some()
}

fn is_active_command(name: &str) -> bool {
    matches!(
        name,
        "write18" | "immediate" | "openout" | "write" | "inputlineno" | "read" | "csname"
    )
}

fn malformed(state: &mut ParseState<'_>, source: &SourceInfo, node: &LatexNode, message: &str) {
    state.parse_errors.push(LatexParseError {
        range: node.range.clone(),
        locator: node.locator.clone(),
        source: Some(source.clone()),
        message: message.to_string(),
    });
    state.diagnostics.push(
        Diagnostic::error("grist.latex", "latex.parse.malformed", message)
            .with_range(node.range.clone())
            .with_locator(node.locator.clone().expect("locator"))
            .partial(),
    );
}

fn count_nodes(nodes: &[LatexNode]) -> usize {
    nodes
        .iter()
        .map(|node| {
            1 + node
                .include
                .as_ref()
                .and_then(|include| include.resolved.as_ref())
                .map(|resolved| count_nodes(&resolved.nodes))
                .unwrap_or(0)
        })
        .sum()
}

fn count_kind(nodes: &[LatexNode], kind: LatexNodeKind) -> usize {
    nodes
        .iter()
        .map(|node| {
            usize::from(node.kind == kind)
                + node
                    .include
                    .as_ref()
                    .and_then(|include| include.resolved.as_ref())
                    .map(|resolved| count_kind(&resolved.nodes, kind.clone()))
                    .unwrap_or(0)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_rich_inert_latex() {
        let source = r#"\documentclass{article}
\newcommand{\hello}[1]{Hello #1}
\title{Paper}
\begin{document}
\section{Intro}\label{sec:intro}
\hello{world} \ref{sec:intro} \cite{paper} $x+1$
\begin{table}\begin{tabular}{cc}A&B\\1&2\end{tabular}\caption{Data}\end{table}
\begin{figure}\includegraphics{plot.png}\caption{Plot}\end{figure}
\write18{touch impossible}
\unknown{raw}
\end{document}"#;
        let envelope = parse_latex(
            source,
            SourceInfo::stdin("paper.tex"),
            &LatexOptions::default(),
        );
        assert_eq!(envelope.status, OperationStatus::Complete);
        let payload = envelope.payload.unwrap();
        for kind in [
            LatexNodeKind::DocumentClass,
            LatexNodeKind::MacroDefinition,
            LatexNodeKind::MacroUse,
            LatexNodeKind::Section,
            LatexNodeKind::Label,
            LatexNodeKind::Ref,
            LatexNodeKind::Citation,
            LatexNodeKind::MathInline,
            LatexNodeKind::Table,
            LatexNodeKind::Figure,
            LatexNodeKind::Caption,
            LatexNodeKind::RawCommand,
        ] {
            assert!(
                payload.nodes.iter().any(|node| node.kind == kind),
                "{kind:?}"
            );
        }
        assert!(payload.nodes.iter().all(|node| {
            node.locator
                .as_ref()
                .is_some_and(|locator| locator.validate().is_ok())
        }));
    }
}
