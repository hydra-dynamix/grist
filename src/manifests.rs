//! Typed inert parsers for package manifests, lockfiles, and infrastructure configuration.
//! Script and template values are retained as data; this module has no execution or I/O path.

use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, Envelope, FormatIdentity, LineIndex, OperationKind,
    ParserInfo, SourceInfo, SourceLocator, SourceRange, options_digest,
};
use crate::document_graph::{
    DocumentEdge, DocumentGraph, DocumentGraphContext, DocumentKind, DocumentNode,
    DocumentNodeKind, DocumentRelation, ProjectionAddress, RawNodeContent, ToDocumentGraph,
    TransformError,
};
#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MANIFEST_SCHEMA_V1: &str = "grist/manifest/v1";
pub const MANIFEST_OPTIONS_SCHEMA_V1: &str = "grist/manifest-options/v1";
const PARSER: &str = "grist.manifests";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ManifestOptions {
    pub filename: Option<String>,
    pub retain_unknown_fields: bool,
}
impl Default for ManifestOptions {
    fn default() -> Self {
        Self {
            filename: None,
            retain_unknown_fields: true,
        }
    }
}
impl crate::core::FormatOptions for ManifestOptions {
    const FORMAT: &'static str = "manifest";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestFormat {
    CargoToml,
    CargoLock,
    NpmPackage,
    NpmLock,
    PnpmLock,
    YarnLock,
    PythonProject,
    PythonRequirements,
    PythonSetup,
    PythonLock,
    MavenPom,
    GradleBuild,
    GradleSettings,
    GoMod,
    GoSum,
    Dockerfile,
    Compose,
    CiWorkflow,
    Kubernetes,
    Unknown,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestRole {
    PackageManifest,
    Lockfile,
    BuildConfiguration,
    ContainerBuild,
    ServiceComposition,
    ContinuousIntegration,
    Orchestration,
    Unknown,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyScope {
    Runtime,
    Development,
    Build,
    Test,
    Optional,
    Peer,
    Platform,
    Unknown,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestReferenceKind {
    Parent,
    Workspace,
    Registry,
    Repository,
    File,
    Url,
    Image,
    BuildContext,
    Include,
    Action,
    Secret,
    Config,
    Service,
    Artifact,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestProvenance {
    pub source_format: ManifestFormat,
    pub declaration: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestDependency {
    pub name: String,
    pub requirement: Option<String>,
    pub resolved: Option<String>,
    pub integrity: Option<String>,
    pub scope: DependencyScope,
    pub optional: bool,
    pub provenance: ManifestProvenance,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestReference {
    pub kind: ManifestReferenceKind,
    pub target: String,
    pub provenance: ManifestProvenance,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestInstruction {
    pub keyword: String,
    pub arguments: String,
    pub raw: String,
    pub executable: bool,
    pub templated: bool,
    pub range: SourceRange,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnknownManifestField {
    pub path: String,
    pub raw: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestParseError {
    pub message: String,
    pub raw: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestDocument {
    pub schema_version: String,
    pub source: SourceInfo,
    pub format: ManifestFormat,
    pub role: ManifestRole,
    pub detected_name: String,
    pub raw: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default)]
    pub dependencies: Vec<ManifestDependency>,
    #[serde(default)]
    pub references: Vec<ManifestReference>,
    #[serde(default)]
    pub instructions: Vec<ManifestInstruction>,
    #[serde(default)]
    pub unknown_fields: Vec<UnknownManifestField>,
    #[serde(default)]
    pub parse_errors: Vec<ManifestParseError>,
    pub active_content_execution: bool,
    pub template_evaluation: bool,
    pub network_access: bool,
}
pub type ManifestEnvelope = Envelope<ManifestDocument>;

#[derive(Clone)]
struct LocatedLine<'a> {
    start: usize,
    end: usize,
    raw: &'a str,
}
struct Collector<'a> {
    text: &'a str,
    format: ManifestFormat,
    index: LineIndex,
    dependencies: Vec<ManifestDependency>,
    references: Vec<ManifestReference>,
    instructions: Vec<ManifestInstruction>,
    unknown_fields: Vec<UnknownManifestField>,
    parse_errors: Vec<ManifestParseError>,
    metadata: BTreeMap<String, Value>,
    recognized: BTreeSet<usize>,
    location_cursors: BTreeMap<String, usize>,
}
impl<'a> Collector<'a> {
    fn new(text: &'a str, format: ManifestFormat) -> Self {
        Self {
            text,
            format,
            index: LineIndex::new(text),
            dependencies: vec![],
            references: vec![],
            instructions: vec![],
            unknown_fields: vec![],
            parse_errors: vec![],
            metadata: BTreeMap::new(),
            recognized: BTreeSet::new(),
            location_cursors: BTreeMap::new(),
        }
    }
    fn lines(&self) -> Vec<LocatedLine<'a>> {
        located_lines(self.text)
    }
    fn locate(&mut self, needle: &str) -> LocatedLine<'a> {
        let candidates = source_needle_line_starts(self.text, needle);
        if candidates.is_empty() {
            self.error(
                format!("parsed value has no source declaration: {needle:?}"),
                0,
                self.text.len(),
            );
            return LocatedLine {
                start: 0,
                end: self.text.len(),
                raw: self.text,
            };
        }
        let cursor = self.location_cursors.entry(needle.into()).or_default();
        let start = candidates
            .get(*cursor)
            .copied()
            .unwrap_or_else(|| *candidates.last().expect("non-empty candidates"));
        *cursor += 1;
        self.lines()
            .into_iter()
            .find(|line| line.start == start)
            .expect("candidate line start")
    }
    fn provenance(&self, line: &LocatedLine<'_>) -> ManifestProvenance {
        let range = SourceRange::new(line.start, line.end, &self.index);
        ManifestProvenance {
            source_format: self.format.clone(),
            declaration: line.raw.into(),
            locator: SourceLocator::try_from(range.clone()).expect("text range"),
            range,
        }
    }
    fn dependency(
        &mut self,
        line: &LocatedLine<'_>,
        name: impl Into<String>,
        requirement: Option<String>,
        resolved: Option<String>,
        integrity: Option<String>,
        scope: DependencyScope,
        optional: bool,
    ) {
        let name = name.into();
        if name.trim().is_empty() {
            return;
        }
        self.recognized.insert(line.start);
        self.dependencies.push(ManifestDependency {
            name,
            requirement,
            resolved,
            integrity,
            scope,
            optional,
            provenance: self.provenance(line),
        });
    }
    fn reference(
        &mut self,
        line: &LocatedLine<'_>,
        kind: ManifestReferenceKind,
        target: impl Into<String>,
    ) {
        let target = target.into();
        if target.trim().is_empty() {
            return;
        }
        self.recognized.insert(line.start);
        self.references.push(ManifestReference {
            kind,
            target,
            provenance: self.provenance(line),
        });
    }
    fn instruction(&mut self, line: &LocatedLine<'_>, keyword: &str, arguments: &str) {
        self.recognized.insert(line.start);
        let range = SourceRange::new(line.start, line.end, &self.index);
        self.instructions.push(ManifestInstruction {
            keyword: keyword.into(),
            arguments: arguments.into(),
            raw: line.raw.into(),
            executable: false,
            templated: ["${{", "{{", "$(", "${"]
                .iter()
                .any(|marker| arguments.contains(marker)),
            locator: SourceLocator::try_from(range.clone()).expect("text range"),
            range,
        });
    }
    fn error(&mut self, message: impl Into<String>, start: usize, end: usize) {
        let range = SourceRange::new(start, end, &self.index);
        self.parse_errors.push(ManifestParseError {
            message: message.into(),
            raw: self.text.get(start..end).unwrap_or_default().into(),
            locator: SourceLocator::try_from(range.clone()).expect("text range"),
            range,
        });
    }
    fn retain_unknown_lines(&mut self) {
        for line in self.lines() {
            let value = line.raw.trim();
            if value.is_empty()
                || value.starts_with('#')
                || value.starts_with("//")
                || self.recognized.contains(&line.start)
            {
                continue;
            }
            let range = SourceRange::new(line.start, line.end, &self.index);
            self.unknown_fields.push(UnknownManifestField {
                path: format!("line:{}", range.start_line),
                raw: line.raw.into(),
                locator: SourceLocator::try_from(range.clone()).expect("text range"),
                range,
            });
        }
    }
}

pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("grist-inert-manifest", env!("CARGO_PKG_VERSION"))
        .with_feature("manifests")
}

pub fn classify_manifest(filename: &str, text: &str) -> (ManifestFormat, ManifestRole) {
    let path = filename.replace('\\', "/").to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(path.as_str());
    match name {
        "cargo.toml" => (ManifestFormat::CargoToml, ManifestRole::PackageManifest),
        "cargo.lock" => (ManifestFormat::CargoLock, ManifestRole::Lockfile),
        "package.json" => (ManifestFormat::NpmPackage, ManifestRole::PackageManifest),
        "package-lock.json" | "npm-shrinkwrap.json" => {
            (ManifestFormat::NpmLock, ManifestRole::Lockfile)
        }
        "pnpm-lock.yaml" | "pnpm-lock.yml" => (ManifestFormat::PnpmLock, ManifestRole::Lockfile),
        "yarn.lock" => (ManifestFormat::YarnLock, ManifestRole::Lockfile),
        "pyproject.toml" | "pipfile" => {
            (ManifestFormat::PythonProject, ManifestRole::PackageManifest)
        }
        "poetry.lock" | "pipfile.lock" => (ManifestFormat::PythonLock, ManifestRole::Lockfile),
        "setup.py" | "setup.cfg" => (ManifestFormat::PythonSetup, ManifestRole::PackageManifest),
        "pom.xml" => (ManifestFormat::MavenPom, ManifestRole::PackageManifest),
        "build.gradle" | "build.gradle.kts" => (
            ManifestFormat::GradleBuild,
            ManifestRole::BuildConfiguration,
        ),
        "settings.gradle" | "settings.gradle.kts" => (
            ManifestFormat::GradleSettings,
            ManifestRole::BuildConfiguration,
        ),
        "go.mod" => (ManifestFormat::GoMod, ManifestRole::PackageManifest),
        "go.sum" => (ManifestFormat::GoSum, ManifestRole::Lockfile),
        "dockerfile" | "containerfile" => {
            (ManifestFormat::Dockerfile, ManifestRole::ContainerBuild)
        }
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => {
            (ManifestFormat::Compose, ManifestRole::ServiceComposition)
        }
        ".gitlab-ci.yml" | ".gitlab-ci.yaml" => (
            ManifestFormat::CiWorkflow,
            ManifestRole::ContinuousIntegration,
        ),
        _ if name.starts_with("requirements") && name.ends_with(".txt") => (
            ManifestFormat::PythonRequirements,
            ManifestRole::PackageManifest,
        ),
        _ if path.contains("/.github/workflows/") || path.starts_with(".github/workflows/") => (
            ManifestFormat::CiWorkflow,
            ManifestRole::ContinuousIntegration,
        ),
        _ if looks_like_kubernetes(text) => {
            (ManifestFormat::Kubernetes, ManifestRole::Orchestration)
        }
        _ => (ManifestFormat::Unknown, ManifestRole::Unknown),
    }
}

pub fn parse_manifest(
    text: &str,
    source: SourceInfo,
    options: &ManifestOptions,
) -> ManifestEnvelope {
    let filename = options
        .filename
        .as_deref()
        .or(source.repository_relative_path.as_deref())
        .or(source.path.as_deref())
        .unwrap_or(source.display_name.as_str());
    let detected_name = filename.to_string();
    let (format, role) = classify_manifest(filename, text);
    let mut c = Collector::new(text, format.clone());
    match format {
        ManifestFormat::CargoToml
        | ManifestFormat::CargoLock
        | ManifestFormat::PythonProject
        | ManifestFormat::PythonLock => parse_toml_family(&mut c),
        ManifestFormat::NpmPackage | ManifestFormat::NpmLock => parse_npm(&mut c),
        ManifestFormat::PnpmLock
        | ManifestFormat::Compose
        | ManifestFormat::CiWorkflow
        | ManifestFormat::Kubernetes => parse_yaml_family(&mut c),
        ManifestFormat::YarnLock => parse_yarn(&mut c),
        ManifestFormat::PythonRequirements => parse_requirements(&mut c),
        ManifestFormat::PythonSetup => parse_python_setup(&mut c),
        ManifestFormat::MavenPom => parse_maven(&mut c),
        ManifestFormat::GradleBuild | ManifestFormat::GradleSettings => parse_gradle(&mut c),
        ManifestFormat::GoMod | ManifestFormat::GoSum => parse_go(&mut c),
        ManifestFormat::Dockerfile => parse_dockerfile(&mut c),
        ManifestFormat::Unknown => c.error("unrecognized manifest family", 0, text.len()),
    }
    validate_text_family(&mut c);
    if options.retain_unknown_fields {
        c.retain_unknown_lines();
    }
    c.dependencies
        .sort_by_key(|item| item.provenance.range.byte_start);
    c.references
        .sort_by_key(|item| item.provenance.range.byte_start);
    c.instructions.sort_by_key(|item| item.range.byte_start);
    let range = SourceRange::new(0, text.len(), &LineIndex::new(text));
    let document = ManifestDocument {
        schema_version: MANIFEST_SCHEMA_V1.into(),
        source: source.clone(),
        format,
        role,
        detected_name,
        raw: text.into(),
        locator: SourceLocator::try_from(range.clone()).expect("range"),
        range,
        metadata: c.metadata,
        dependencies: c.dependencies,
        references: c.references,
        instructions: c.instructions,
        unknown_fields: c.unknown_fields,
        parse_errors: c.parse_errors,
        active_content_execution: false,
        template_evaluation: false,
        network_access: false,
    };
    let diagnostics = document
        .parse_errors
        .iter()
        .map(|error| {
            Diagnostic::malformed(PARSER, error.message.clone())
                .with_range(error.range.clone())
                .partial()
        })
        .collect::<Vec<_>>();
    let identity = ContentIdentity::for_raw_bytes(text.as_bytes())
        .with_decoded(text, "utf-8", false)
        .with_format(FormatIdentity::new("manifest", Some(PARSER.to_string())));
    let digest = options_digest(options).expect("options serialize");
    if diagnostics.is_empty() {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Manifest,
            source,
            parser_info(),
            digest,
            MANIFEST_SCHEMA_V1,
            document,
        )
        .with_identity(identity)
    } else {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Manifest,
            source,
            parser_info(),
            digest,
            MANIFEST_SCHEMA_V1,
            Some(document),
        )
        .with_identity(identity)
        .with_diagnostics(diagnostics)
    }
}
fn parse_toml_family(c: &mut Collector<'_>) {
    if let Err(error) = c.text.parse::<toml::Value>() {
        c.error(format!("malformed TOML: {error}"), 0, c.text.len());
    }
    let mut section = String::new();
    let mut dependency_subtable = None;
    for line in c.lines() {
        let value = line.raw.trim();
        if value.starts_with('[') && value.ends_with(']') {
            let raw_section = value.trim_matches(['[', ']']);
            section = raw_section.to_ascii_lowercase();
            c.recognized.insert(line.start);
            dependency_subtable = dependency_name_from_toml_subtable(raw_section).map(|name| {
                let scope = scope_for_section(&section);
                c.dependency(
                    &line,
                    name,
                    None,
                    None,
                    None,
                    scope.clone(),
                    scope == DependencyScope::Optional,
                );
                c.dependencies.len() - 1
            });
            continue;
        }
        let Some((raw_name, raw_value)) = value.split_once('=') else {
            continue;
        };
        let name = trim_quotes(raw_name);
        let raw_value = raw_value.trim();
        if matches!(
            c.format,
            ManifestFormat::CargoLock | ManifestFormat::PythonLock
        ) && name == "name"
        {
            c.dependency(
                &line,
                trim_quotes(raw_value),
                None,
                None,
                None,
                DependencyScope::Runtime,
                false,
            );
        } else if matches!(
            c.format,
            ManifestFormat::CargoLock | ManifestFormat::PythonLock
        ) && name == "version"
        {
            if let Some(dependency) = c.dependencies.last_mut() {
                dependency.resolved = Some(trim_quotes(raw_value));
                c.recognized.insert(line.start);
            }
        } else if matches!(
            c.format,
            ManifestFormat::CargoLock | ManifestFormat::PythonLock
        ) && name == "checksum"
        {
            if let Some(dependency) = c.dependencies.last_mut() {
                dependency.integrity = Some(trim_quotes(raw_value));
                c.recognized.insert(line.start);
            }
        } else if matches!(
            c.format,
            ManifestFormat::CargoLock | ManifestFormat::PythonLock
        ) && name == "source"
        {
            c.reference(
                &line,
                ManifestReferenceKind::Registry,
                trim_quotes(raw_value),
            );
        } else if matches!(c.format, ManifestFormat::PythonProject)
            && (name == "dependencies"
                || (section.contains("dependencies") && raw_value.starts_with('[')))
        {
            let scope = if section.contains("optional") {
                DependencyScope::Optional
            } else {
                DependencyScope::Runtime
            };
            for requirement in quoted_values(raw_value) {
                let (dependency, version) = split_requirement(&requirement);
                c.dependency(
                    &line,
                    dependency,
                    version,
                    None,
                    None,
                    scope.clone(),
                    scope == DependencyScope::Optional,
                );
            }
        } else if let Some(index) = dependency_subtable {
            match name.as_str() {
                "version" => {
                    c.dependencies[index].requirement = Some(trim_quotes(raw_value));
                    c.recognized.insert(line.start);
                }
                "checksum" => {
                    c.dependencies[index].integrity = Some(trim_quotes(raw_value));
                    c.recognized.insert(line.start);
                }
                "optional" => {
                    c.dependencies[index].optional = raw_value.eq_ignore_ascii_case("true");
                    c.recognized.insert(line.start);
                }
                "path" | "git" | "url" | "registry" => {
                    let kind = match name.as_str() {
                        "git" => ManifestReferenceKind::Repository,
                        "url" => ManifestReferenceKind::Url,
                        "registry" => ManifestReferenceKind::Registry,
                        _ => ManifestReferenceKind::File,
                    };
                    c.reference(&line, kind, trim_quotes(raw_value));
                }
                _ => {}
            }
        } else if is_dependency_section(&section) {
            let scope = scope_for_section(&section);
            c.dependency(
                &line,
                name,
                inline_value(raw_value, "version").or_else(|| Some(trim_quotes(raw_value))),
                None,
                inline_value(raw_value, "checksum"),
                scope.clone(),
                scope == DependencyScope::Optional,
            );
            for (key, kind) in [
                ("path", ManifestReferenceKind::File),
                ("git", ManifestReferenceKind::Repository),
                ("url", ManifestReferenceKind::Url),
                ("registry", ManifestReferenceKind::Registry),
            ] {
                if let Some(target) = inline_value(raw_value, key) {
                    c.reference(&line, kind, target);
                }
            }
        } else if matches!(
            name.as_str(),
            "workspace" | "members" | "include" | "exclude"
        ) {
            c.reference(
                &line,
                ManifestReferenceKind::Workspace,
                trim_quotes(raw_value),
            );
        } else if matches!(
            name.as_str(),
            "name" | "version" | "edition" | "requires-python"
        ) {
            c.metadata
                .insert(name, Value::String(trim_quotes(raw_value)));
            c.recognized.insert(line.start);
        }
    }
}

fn parse_npm(c: &mut Collector<'_>) {
    let root: Option<Value> = match serde_json::from_str(c.text) {
        Ok(value) => Some(value),
        Err(error) => {
            c.error(format!("malformed JSON: {error}"), 0, c.text.len());
            None
        }
    };
    let Some(root) = root.as_ref().and_then(Value::as_object) else {
        return;
    };
    for key in ["name", "version", "packageManager", "lockfileVersion"] {
        if let Some(value) = root.get(key) {
            c.metadata.insert(key.into(), value.clone());
        }
    }
    for (section, scope) in [
        ("dependencies", DependencyScope::Runtime),
        ("devDependencies", DependencyScope::Development),
        ("peerDependencies", DependencyScope::Peer),
        ("optionalDependencies", DependencyScope::Optional),
    ] {
        if let Some(entries) = root.get(section).and_then(Value::as_object) {
            for (name, item) in entries {
                let requirement = item.as_str().map(str::to_string).or_else(|| {
                    item.get("version")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
                let line = c.locate(name);
                c.dependency(
                    &line,
                    name,
                    requirement,
                    item.get("resolved")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    item.get("integrity")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    scope.clone(),
                    scope == DependencyScope::Optional,
                );
            }
        }
    }
    if let Some(packages) = root.get("packages").and_then(Value::as_object) {
        for (path, item) in packages {
            if path.is_empty() {
                continue;
            }
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(path.trim_start_matches("node_modules/"));
            let line = c.locate(path);
            c.dependency(
                &line,
                name,
                None,
                item.get("version")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                item.get("integrity")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                DependencyScope::Runtime,
                item.get("optional")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            );
        }
    }
    if let Some(scripts) = root.get("scripts").and_then(Value::as_object) {
        for (name, value) in scripts {
            if let Some(command) = value.as_str() {
                let line = c.locate(name);
                c.instruction(&line, &format!("script:{name}"), command);
            }
        }
    }
    for key in ["workspaces", "files", "repository", "homepage"] {
        if let Some(value) = root.get(key) {
            let line = c.locate(key);
            c.reference(&line, reference_kind(value), compact_value(value));
        }
    }
}

fn parse_yaml_family(c: &mut Collector<'_>) {
    let documents = c.text.split("\n---").collect::<Vec<_>>();
    for document in documents {
        match serde_yaml::from_str::<Value>(document) {
            Ok(value) => match c.format {
                ManifestFormat::PnpmLock => collect_pnpm(c, &value),
                ManifestFormat::Compose => collect_compose(c, &value),
                ManifestFormat::CiWorkflow => collect_ci(c, &value),
                ManifestFormat::Kubernetes => collect_kubernetes(c, &value),
                _ => {}
            },
            Err(error) => c.error(format!("malformed YAML: {error}"), 0, c.text.len()),
        }
    }
}
fn collect_pnpm(c: &mut Collector<'_>, value: &Value) {
    for section in ["importers", "packages", "snapshots"] {
        let Some(entries) = value.get(section).and_then(Value::as_object) else {
            continue;
        };
        for (name, item) in entries {
            if section == "importers" {
                collect_named_dependency_maps(c, item);
                continue;
            }
            let line = c.locate(name);
            let clean = package_name_from_lock_key(name);
            c.dependency(
                &line,
                clean,
                item.get("specifier")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                item.get("version")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                item.get("integrity")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                DependencyScope::Runtime,
                false,
            );
        }
    }
}
fn collect_named_dependency_maps(c: &mut Collector<'_>, value: &Value) {
    for (key, scope) in [
        ("dependencies", DependencyScope::Runtime),
        ("devDependencies", DependencyScope::Development),
        ("optionalDependencies", DependencyScope::Optional),
    ] {
        let Some(entries) = value.get(key).and_then(Value::as_object) else {
            continue;
        };
        for (name, item) in entries {
            let line = c.locate(name);
            c.dependency(
                &line,
                name,
                item.get("specifier")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                item.get("version")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                None,
                scope.clone(),
                scope == DependencyScope::Optional,
            );
        }
    }
}
fn collect_compose(c: &mut Collector<'_>, value: &Value) {
    let Some(services) = value.get("services").and_then(Value::as_object) else {
        return;
    };
    for (service, spec) in services {
        c.metadata
            .insert(format!("service.{service}"), Value::String(service.clone()));
        if let Some(image) = spec.get("image").and_then(Value::as_str) {
            let line = c.locate(image);
            c.reference(&line, ManifestReferenceKind::Image, image);
        }
        if let Some(build) = spec.get("build") {
            let line = c.locate(service);
            c.reference(
                &line,
                ManifestReferenceKind::BuildContext,
                compact_value(build),
            );
        }
        if let Some(depends) = spec.get("depends_on") {
            for target in strings_or_keys(depends) {
                let line = c.locate(&target);
                c.reference(&line, ManifestReferenceKind::Service, target);
            }
        }
    }
}
fn collect_ci(c: &mut Collector<'_>, value: &Value) {
    walk_object(value, "", &mut |path, value| {
        let key = path.rsplit('.').next().unwrap_or(path);
        if key == "uses" {
            if let Some(target) = value.as_str() {
                let line = c.locate(target);
                c.reference(&line, ManifestReferenceKind::Action, target);
            }
        } else if matches!(key, "run" | "script" | "before_script" | "after_script") {
            let command = compact_value(value);
            let line = c.locate(key);
            c.instruction(&line, key, &command);
        } else if key == "needs" {
            for target in strings_or_keys(value) {
                let line = c.locate(&target);
                c.reference(&line, ManifestReferenceKind::Service, target);
            }
        }
    });
}
fn collect_kubernetes(c: &mut Collector<'_>, value: &Value) {
    for key in ["apiVersion", "kind"] {
        if let Some(item) = value.get(key) {
            c.metadata.insert(key.into(), item.clone());
        }
    }
    walk_object(value, "", &mut |path, value| {
        let key = path.rsplit('.').next().unwrap_or(path);
        if key == "image" {
            if let Some(target) = value.as_str() {
                let line = c.locate(target);
                c.reference(&line, ManifestReferenceKind::Image, target);
            }
        } else if matches!(key, "configMapRef" | "configMap") {
            let line = c.locate(key);
            c.reference(&line, ManifestReferenceKind::Config, compact_value(value));
        } else if matches!(key, "secretRef" | "secret" | "secretKeyRef") {
            let line = c.locate(key);
            c.reference(&line, ManifestReferenceKind::Secret, compact_value(value));
        } else if matches!(key, "command" | "args") {
            let line = c.locate(key);
            c.instruction(&line, key, &compact_value(value));
        }
    });
}
fn parse_yarn(c: &mut Collector<'_>) {
    let mut current: Option<(String, LocatedLine<'_>)> = None;
    for line in c.lines() {
        let value = line.raw.trim();
        if !line.raw.starts_with(char::is_whitespace) && value.ends_with(':') {
            current = Some((
                package_name_from_lock_key(&trim_quotes(value.trim_end_matches(':'))).into(),
                line.clone(),
            ));
            c.recognized.insert(line.start);
        } else if let Some((name, declaration)) = current.as_ref() {
            if let Some(version) = value.strip_prefix("version ") {
                c.dependency(
                    declaration,
                    name,
                    None,
                    Some(trim_quotes(version)),
                    None,
                    DependencyScope::Runtime,
                    false,
                );
                c.recognized.insert(line.start);
            } else if let Some(url) = value.strip_prefix("resolved ") {
                c.reference(&line, ManifestReferenceKind::Url, trim_quotes(url));
            } else if let Some(integrity) = value.strip_prefix("integrity ") {
                if let Some(dep) = c.dependencies.last_mut() {
                    dep.integrity = Some(trim_quotes(integrity));
                }
                c.recognized.insert(line.start);
            }
        }
    }
}
fn parse_requirements(c: &mut Collector<'_>) {
    for line in c.lines() {
        let value = line.raw.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        if let Some(target) = value
            .strip_prefix("-r ")
            .or_else(|| value.strip_prefix("--requirement "))
        {
            c.reference(&line, ManifestReferenceKind::Include, target.trim());
        } else if value.starts_with("--") {
            c.instruction(&line, "pip-option", value);
        } else {
            let (name, requirement) = split_requirement(value);
            c.dependency(
                &line,
                name,
                requirement,
                None,
                None,
                DependencyScope::Runtime,
                false,
            );
        }
    }
}
fn parse_python_setup(c: &mut Collector<'_>) {
    let mut in_requires = false;
    for line in c.lines() {
        let value = line.raw.trim();
        if value.contains("install_requires") || value.contains("dependencies") {
            in_requires = true;
            c.recognized.insert(line.start);
        }
        if in_requires {
            for quoted in quoted_values(value) {
                let (name, requirement) = split_requirement(&quoted);
                c.dependency(
                    &line,
                    name,
                    requirement,
                    None,
                    None,
                    DependencyScope::Runtime,
                    false,
                );
            }
            if value.contains(']') || value.contains(')') {
                in_requires = false;
            }
        }
        if value.contains("setup(")
            || value.starts_with("[metadata]")
            || value.starts_with("[options]")
        {
            c.instruction(&line, "python-packaging", value);
        }
    }
}
fn parse_maven(c: &mut Collector<'_>) {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(c.text);
    reader.config_mut().trim_text(true);
    let mut stack = Vec::<String>::new();
    let (mut group, mut artifact, mut version) = (None, None, None);
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                stack.push(String::from_utf8_lossy(event.name().as_ref()).into())
            }
            Ok(Event::End(event)) => {
                let name = String::from_utf8_lossy(event.name().as_ref()).into_owned();
                if name == "dependency" {
                    if let Some(artifact_id) = artifact.take() {
                        let full = group
                            .take()
                            .map(|g| format!("{g}:{artifact_id}"))
                            .unwrap_or(artifact_id);
                        let line = c.locate(full.split(':').next_back().unwrap_or(""));
                        c.dependency(
                            &line,
                            full,
                            version.take(),
                            None,
                            None,
                            DependencyScope::Runtime,
                            false,
                        );
                    }
                }
                stack.pop();
            }
            Ok(Event::Text(event)) => {
                let text = event
                    .unescape()
                    .map(|value| value.into_owned())
                    .unwrap_or_default();
                if stack.iter().any(|item| item == "dependency") {
                    match stack.last().map(String::as_str).unwrap_or("") {
                        "groupId" => group = Some(text),
                        "artifactId" => artifact = Some(text),
                        "version" => version = Some(text),
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                let pos = reader.buffer_position() as usize;
                c.error(
                    format!("malformed Maven XML: {error}"),
                    pos.min(c.text.len()),
                    c.text.len(),
                );
                break;
            }
            _ => {}
        }
    }
}
fn parse_gradle(c: &mut Collector<'_>) {
    for line in c.lines() {
        let value = line.raw.trim();
        for (keyword, scope) in [
            ("implementation", DependencyScope::Runtime),
            ("api", DependencyScope::Runtime),
            ("compileOnly", DependencyScope::Build),
            ("runtimeOnly", DependencyScope::Runtime),
            ("testImplementation", DependencyScope::Test),
            ("classpath", DependencyScope::Build),
        ] {
            if value.starts_with(keyword) {
                if let Some(coordinate) = quoted_values(value).first() {
                    let mut parts = coordinate.split(':');
                    let group = parts.next().unwrap_or("");
                    let artifact = parts.next().unwrap_or(group);
                    let version = parts.next().map(str::to_string);
                    let name = if group == artifact {
                        artifact.into()
                    } else {
                        format!("{group}:{artifact}")
                    };
                    c.dependency(&line, name, version, None, None, scope, false);
                } else {
                    c.instruction(&line, keyword, value);
                }
            }
        }
        if value.starts_with("include")
            || value.starts_with("project(")
            || value.starts_with("rootProject.name")
        {
            c.reference(&line, ManifestReferenceKind::Workspace, value);
        }
        if value.starts_with("repositories") || value.contains("maven {") {
            c.instruction(&line, "repository-configuration", value);
        }
    }
}
fn parse_go(c: &mut Collector<'_>) {
    let mut require_block = false;
    for line in c.lines() {
        let value = line.raw.trim();
        if value == "require (" {
            require_block = true;
            c.recognized.insert(line.start);
            continue;
        }
        if require_block && value == ")" {
            require_block = false;
            c.recognized.insert(line.start);
            continue;
        }
        if let Some(module) = value.strip_prefix("module ") {
            c.metadata
                .insert("module".into(), Value::String(module.into()));
            c.recognized.insert(line.start);
        } else if let Some(version) = value.strip_prefix("go ") {
            c.metadata
                .insert("go".into(), Value::String(version.into()));
            c.recognized.insert(line.start);
        } else if let Some(requirement) = value
            .strip_prefix("require ")
            .or_else(|| require_block.then_some(value))
        {
            let mut parts = requirement.split_whitespace();
            if let Some(name) = parts.next() {
                c.dependency(
                    &line,
                    name,
                    parts.next().map(str::to_string),
                    None,
                    None,
                    DependencyScope::Runtime,
                    value.contains("indirect"),
                );
            }
        } else if let Some(replace) = value.strip_prefix("replace ") {
            if let Some((_, target)) = replace.split_once("=>") {
                c.reference(&line, ManifestReferenceKind::Repository, target.trim());
            }
        } else if matches!(c.format, ManifestFormat::GoSum) {
            let mut parts = value.split_whitespace();
            if let (Some(name), Some(version), Some(integrity)) =
                (parts.next(), parts.next(), parts.next())
            {
                c.dependency(
                    &line,
                    name,
                    None,
                    Some(version.into()),
                    Some(integrity.into()),
                    DependencyScope::Runtime,
                    false,
                );
            }
        }
    }
}
fn parse_dockerfile(c: &mut Collector<'_>) {
    let mut continuation = None;
    for line in c.lines() {
        let value = line.raw.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        let (keyword, args) = value.split_once(char::is_whitespace).unwrap_or((value, ""));
        let keyword = keyword.to_ascii_uppercase();
        c.instruction(&line, &keyword, args.trim());
        match keyword.as_str() {
            "FROM" => {
                let image = args
                    .split_whitespace()
                    .find(|item| !item.starts_with("--"))
                    .unwrap_or("");
                c.reference(&line, ManifestReferenceKind::Image, image);
            }
            "COPY" | "ADD" => c.reference(&line, ManifestReferenceKind::File, args.trim()),
            "INCLUDE" => c.reference(&line, ManifestReferenceKind::Include, args.trim()),
            _ => {}
        }
        if value.ends_with('\\') {
            continuation.get_or_insert(line.start);
        } else {
            continuation = None;
        }
    }
    if let Some(start) = continuation {
        c.error("unterminated Dockerfile continuation", start, c.text.len());
    }
}

fn validate_text_family(c: &mut Collector<'_>) {
    let meaningful = c.text.lines().any(|line| {
        let line = line.trim();
        !line.is_empty() && !line.starts_with('#')
    });
    if !meaningful {
        c.error("manifest is empty", 0, c.text.len());
        return;
    }
    match c.format {
        ManifestFormat::YarnLock if c.dependencies.is_empty() => {
            c.error("Yarn lock entry has no version", 0, c.text.len())
        }
        ManifestFormat::PythonRequirements
            if c.dependencies.iter().any(|dependency| {
                dependency
                    .name
                    .chars()
                    .any(|ch| ch.is_whitespace() || ch == '?')
            }) =>
        {
            c.error("malformed Python requirement", 0, c.text.len())
        }
        ManifestFormat::PythonSetup if unmatched_delimiters(c.text) => {
            c.error("unbalanced Python packaging expression", 0, c.text.len())
        }
        ManifestFormat::GradleBuild | ManifestFormat::GradleSettings
            if c.text.lines().any(|line| {
                line.matches('"').count() % 2 == 1 || line.matches('\'').count() % 2 == 1
            }) =>
        {
            c.error("unterminated Gradle string", 0, c.text.len())
        }
        ManifestFormat::GoMod if !c.metadata.contains_key("module") => {
            c.error("Go module has no module directive", 0, c.text.len())
        }
        ManifestFormat::GoSum if c.dependencies.is_empty() => c.error(
            "Go checksum line requires module, version, and checksum",
            0,
            c.text.len(),
        ),
        _ => {}
    }
}
fn unmatched_delimiters(text: &str) -> bool {
    text.chars().filter(|ch| *ch == '[').count() != text.chars().filter(|ch| *ch == ']').count()
        || text.chars().filter(|ch| *ch == '(').count()
            != text.chars().filter(|ch| *ch == ')').count()
}
impl ToDocumentGraph for ManifestDocument {
    fn to_document_graph(
        &self,
        context: DocumentGraphContext,
    ) -> Result<DocumentGraph, TransformError> {
        let identities = context
            .identity_generator(MANIFEST_SCHEMA_V1, PARSER)
            .map_err(|error| TransformError::Other {
                message: error.to_string(),
            })?;
        let mut graph =
            DocumentGraph::new(context.graph_id, DocumentKind::Other("manifest".into()))
                .with_projection(
                    "manifest",
                    MANIFEST_SCHEMA_V1,
                    "grist.manifest.to-document-graph.v1",
                );
        graph.source = context.source.or_else(|| Some(self.source.clone()));
        graph.attrs = context.attrs;
        graph.attrs.insert(
            "format".into(),
            serde_json::to_value(&self.format).unwrap_or_default(),
        );
        let root = identities
            .node_id(&ProjectionAddress::native(
                ["manifest"],
                &self.detected_name,
            ))
            .map_err(|error| TransformError::Other {
                message: error.to_string(),
            })?;
        graph.add_node(
            DocumentNode::new(&root, DocumentNodeKind::Package)
                .with_name(&self.detected_name)
                .with_range(self.range.clone())
                .with_extension(
                    "grist.manifest",
                    serde_json::to_value(self).unwrap_or_default(),
                )
                .map_err(|error| TransformError::Other {
                    message: error.to_string(),
                })?,
        );
        for (ordinal, dependency) in self.dependencies.iter().enumerate() {
            let id = identities
                .node_id(&ProjectionAddress::native(
                    ["dependency"],
                    format!("{}:{ordinal}", dependency.name),
                ))
                .map_err(|error| TransformError::Other {
                    message: error.to_string(),
                })?;
            graph.add_node(
                DocumentNode::new(&id, DocumentNodeKind::Package)
                    .with_name(&dependency.name)
                    .with_text(
                        dependency
                            .requirement
                            .as_deref()
                            .or(dependency.resolved.as_deref())
                            .unwrap_or(""),
                    )
                    .with_range(dependency.provenance.range.clone())
                    .with_ordinal(ordinal)
                    .with_extension(
                        "grist.manifest",
                        serde_json::to_value(dependency).unwrap_or_default(),
                    )
                    .map_err(|error| TransformError::Other {
                        message: error.to_string(),
                    })?,
            );
            graph.add_edge(DocumentEdge::explicit(
                root.clone(),
                DocumentRelation::Requires,
                id,
                dependency.provenance.locator.clone(),
            ));
        }
        for (ordinal, reference) in self.references.iter().enumerate() {
            let id = identities
                .node_id(&ProjectionAddress::native(
                    ["reference"],
                    format!("{}:{ordinal}", reference.target),
                ))
                .map_err(|error| TransformError::Other {
                    message: error.to_string(),
                })?;
            graph.add_node(
                DocumentNode::new(&id, DocumentNodeKind::Reference)
                    .with_name(&reference.target)
                    .with_range(reference.provenance.range.clone())
                    .with_ordinal(ordinal)
                    .with_extension(
                        "grist.manifest",
                        serde_json::to_value(reference).unwrap_or_default(),
                    )
                    .map_err(|error| TransformError::Other {
                        message: error.to_string(),
                    })?,
            );
            graph.add_edge(DocumentEdge::explicit(
                root.clone(),
                DocumentRelation::References,
                id,
                reference.provenance.locator.clone(),
            ));
        }
        for (ordinal, instruction) in self.instructions.iter().enumerate() {
            let id = identities
                .node_id(&ProjectionAddress::native(
                    ["instruction"],
                    format!("{}:{ordinal}", instruction.keyword),
                ))
                .map_err(|error| TransformError::Other {
                    message: error.to_string(),
                })?;
            graph.add_node(
                DocumentNode::new(&id, DocumentNodeKind::CodeBlock)
                    .with_name(&instruction.keyword)
                    .with_text(&instruction.arguments)
                    .with_range(instruction.range.clone())
                    .with_ordinal(ordinal)
                    .with_extension(
                        "grist.manifest",
                        serde_json::to_value(instruction).unwrap_or_default(),
                    )
                    .map_err(|error| TransformError::Other {
                        message: error.to_string(),
                    })?,
            );
            graph.add_edge(DocumentEdge::explicit(
                root.clone(),
                DocumentRelation::Contains,
                id,
                instruction.locator.clone(),
            ));
        }
        for (ordinal, unknown) in self.unknown_fields.iter().enumerate() {
            let id = identities
                .node_id(&ProjectionAddress::native(
                    ["unknown"],
                    format!("{}:{ordinal}", unknown.path),
                ))
                .map_err(|error| TransformError::Other {
                    message: error.to_string(),
                })?;
            let raw = RawNodeContent {
                namespace: "grist.manifest".into(),
                original_kind: "unknown_manifest_field".into(),
                payload: serde_json::to_value(unknown).unwrap_or_default(),
            };
            graph.add_node(
                DocumentNode::new(&id, DocumentNodeKind::Unknown)
                    .with_name(&unknown.path)
                    .with_range(unknown.range.clone())
                    .with_raw(raw)
                    .with_ordinal(ordinal),
            );
            graph.add_edge(DocumentEdge::explicit(
                root.clone(),
                DocumentRelation::Contains,
                id,
                unknown.locator.clone(),
            ));
        }
        graph
            .finalize_projection(&identities)
            .map_err(|error| TransformError::Other {
                message: error.to_string(),
            })?;
        Ok(graph)
    }
}

fn located_lines(text: &str) -> Vec<LocatedLine<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for part in text.split_inclusive('\n') {
        let physical_end = start + part.len();
        let raw = part.trim_end_matches(['\r', '\n']);
        lines.push(LocatedLine {
            start,
            end: start + raw.len(),
            raw,
        });
        start = physical_end;
    }
    if start < text.len() || text.is_empty() {
        lines.push(LocatedLine {
            start,
            end: text.len(),
            raw: &text[start..],
        });
    }
    lines
}
fn source_needle_line_starts(text: &str, needle: &str) -> Vec<usize> {
    let mut offsets = text
        .match_indices(needle)
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    let bytes = text.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if bytes[start] != b'"' {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        let mut escaped = false;
        while end < bytes.len() {
            let byte = bytes[end];
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                break;
            }
            end += 1;
        }
        if end < bytes.len() {
            if serde_json::from_str::<String>(&text[start..=end]).is_ok_and(|value| value == needle)
            {
                offsets.push(start);
            }
            start = end + 1;
        } else {
            break;
        }
    }
    let lines = located_lines(text);
    let mut starts = offsets
        .into_iter()
        .filter_map(|offset| {
            lines
                .iter()
                .find(|line| line.start <= offset && offset <= line.end)
                .map(|line| line.start)
        })
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    starts
}
fn package_name_from_lock_key(value: &str) -> &str {
    let value = value.trim_start_matches('/');
    if value.starts_with('@') {
        value
            .rsplit_once('@')
            .map(|(name, _)| name)
            .filter(|name| !name.is_empty())
            .unwrap_or(value)
    } else {
        value.split_once('@').map(|(name, _)| name).unwrap_or(value)
    }
}
fn trim_quotes(value: &str) -> String {
    value
        .trim()
        .trim_matches(['"', '\''])
        .trim_end_matches(',')
        .into()
}
fn inline_value(value: &str, key: &str) -> Option<String> {
    let pos = value.find(key)?;
    let tail = value[pos + key.len()..]
        .trim_start()
        .strip_prefix('=')?
        .trim_start();
    let quote = tail.chars().next()?;
    if quote == '"' || quote == '\'' {
        let end = tail[1..].find(quote)? + 1;
        Some(tail[1..end].into())
    } else {
        Some(tail.split([',', '}']).next()?.trim().into())
    }
}
fn dependency_name_from_toml_subtable(section: &str) -> Option<String> {
    let lower = section.to_ascii_lowercase();
    let marker = lower.rfind("dependencies.")?;
    let name = section[marker + "dependencies.".len()..].trim_matches(['"', '\'']);
    (!name.is_empty()).then(|| name.to_string())
}
fn is_dependency_section(section: &str) -> bool {
    section.contains("dependencies") || section.ends_with(".requires") || section == "packages"
}
fn scope_for_section(section: &str) -> DependencyScope {
    if section.contains("dev") {
        DependencyScope::Development
    } else if section.contains("build") {
        DependencyScope::Build
    } else if section.contains("test") {
        DependencyScope::Test
    } else if section.contains("optional") {
        DependencyScope::Optional
    } else {
        DependencyScope::Runtime
    }
}
fn looks_like_kubernetes(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with("apiVersion:"))
        && text
            .lines()
            .any(|line| line.trim_start().starts_with("kind:"))
}
fn split_requirement(value: &str) -> (String, Option<String>) {
    let value = value.split(';').next().unwrap_or(value).trim();
    for marker in ["===", "==", ">=", "<=", "~=", "!=", ">", "<", "@"] {
        if let Some(pos) = value.find(marker) {
            return (value[..pos].trim().into(), Some(value[pos..].trim().into()));
        }
    }
    (value.split('[').next().unwrap_or(value).trim().into(), None)
}
fn quoted_values(value: &str) -> Vec<String> {
    let mut values = vec![];
    let mut quote = None;
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if let Some(open) = quote {
            if ch == open {
                values.push(value[start..index].into());
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            start = index + ch.len_utf8();
        }
    }
    values
}
fn compact_value(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}
fn reference_kind(value: &Value) -> ManifestReferenceKind {
    let text = compact_value(value);
    if text.starts_with("http:") || text.starts_with("https:") {
        ManifestReferenceKind::Url
    } else if text.contains("git") {
        ManifestReferenceKind::Repository
    } else {
        ManifestReferenceKind::File
    }
}
fn strings_or_keys(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values.iter().flat_map(strings_or_keys).collect(),
        Value::Object(values) => values.keys().cloned().collect(),
        _ => vec![],
    }
}
fn walk_object(value: &Value, prefix: &str, visit: &mut impl FnMut(&str, &Value)) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                visit(&path, value);
                walk_object(value, &path, visit);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                walk_object(value, &format!("{prefix}[{index}]"), visit);
            }
        }
        _ => {}
    }
}
