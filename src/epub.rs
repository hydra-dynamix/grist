//! Safe EPUB 2/3 package parser.
//!
//! EPUB is treated as a ZIP package, never as an extraction request. Every
//! member is validated before decompression, expansion is budgeted from the
//! central-directory sizes, and package references resolve only in-archive.

use crate::container::{
    ArtifactDisposition, ArtifactExtraction, ArtifactExtractionStatus, ArtifactMetadata,
    ArtifactParent, ArtifactRelationship, EmbeddedArtifact,
};
use crate::core::{
    ContentIdentity, Diagnostic, FormatIdentity, IndexBase, IndexPosition, LineIndex,
    LocationComponent, OperationStatus, ParserInfo, SchemaVersion, SourceInfo, SourceLocator,
    SourceRange,
};
use crate::html::{
    HtmlDocument, HtmlNode, HtmlNodeKind, HtmlOptions, HtmlSyntax, parse_html_bytes,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};
use crate::security::{ArchiveEntryKind, ArchiveMemberDescriptor, ArchiveSecurityPolicy};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Cursor, Read};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

const PARSER: &str = "grist.epub";
const CONTAINER_PATH: &str = "META-INF/container.xml";
const MIMETYPE: &[u8] = b"application/epub+zip";

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct EpubOptions {
    /// Retain member bytes in embedded-artifact records. Inventory mode keeps
    /// the same nested content identities without duplicating package bytes.
    pub inline_resource_bytes: bool,
    pub retain_non_spine_documents: bool,
}

impl Default for EpubOptions {
    fn default() -> Self {
        Self {
            inline_resource_bytes: false,
            retain_non_spine_documents: true,
        }
    }
}

impl crate::core::FormatOptions for EpubOptions {
    const FORMAT: &'static str = "epub";
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubDocument {
    pub schema_version: String,
    pub version: EpubVersion,
    pub package_path: String,
    pub package_locator: SourceLocator,
    pub unique_identifier_id: Option<String>,
    pub metadata: Vec<EpubMetadata>,
    pub manifest: Vec<EpubManifestItem>,
    pub spine: EpubSpine,
    pub navigation: Vec<EpubNavigation>,
    pub chapters: Vec<EpubChapter>,
    pub footnotes: Vec<EpubFootnote>,
    pub images: Vec<EpubImage>,
    pub styles: Vec<EpubStyle>,
    pub resources: Vec<EpubResource>,
    pub rootfiles: Vec<EpubRootfile>,
    pub encrypted_resources: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EpubVersion {
    Epub2,
    Epub3,
    Unknown(String),
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubRootfile {
    pub full_path: String,
    pub media_type: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubMetadata {
    pub name: String,
    pub value: String,
    pub id: Option<String>,
    pub property: Option<String>,
    pub refines: Option<String>,
    pub scheme: Option<String>,
    pub language: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubManifestItem {
    pub id: String,
    pub href: String,
    pub resolved_path: Option<String>,
    pub media_type: String,
    pub properties: Vec<String>,
    pub fallback: Option<String>,
    pub media_overlay: Option<String>,
    pub encrypted: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubSpine {
    pub toc: Option<String>,
    pub page_progression_direction: Option<String>,
    pub items: Vec<EpubSpineItem>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubSpineItem {
    pub position: usize,
    pub idref: String,
    pub linear: bool,
    pub properties: Vec<String>,
    pub manifest_index: Option<usize>,
    pub resolved_path: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubNavigation {
    pub kind: String,
    pub source_path: String,
    pub entries: Vec<EpubNavigationEntry>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubNavigationEntry {
    pub label: String,
    pub href: String,
    pub resolved_path: Option<String>,
    pub fragment: Option<String>,
    pub depth: usize,
    pub play_order: Option<u64>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubChapter {
    pub spine_position: Option<usize>,
    pub manifest_id: String,
    pub path: String,
    pub linear: bool,
    pub title: Option<String>,
    pub identity: ContentIdentity,
    pub locator: SourceLocator,
    pub document: HtmlDocument,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubFootnote {
    pub id: String,
    pub kind: String,
    pub chapter_path: String,
    pub text: String,
    pub references: Vec<EpubFootnoteReference>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubFootnoteReference {
    pub chapter_path: String,
    pub href: String,
    pub resolved: bool,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubImage {
    pub manifest_id: String,
    pub path: Option<String>,
    pub media_type: String,
    pub properties: Vec<String>,
    pub usages: Vec<EpubImageUsage>,
    pub artifact_id: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubImageUsage {
    pub chapter_path: String,
    pub source: String,
    pub alt_text: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubStyle {
    pub manifest_id: String,
    pub path: Option<String>,
    pub media_type: String,
    pub character_count: Option<u64>,
    pub referenced_resources: Vec<String>,
    pub artifact_id: Option<String>,
    pub locator: SourceLocator,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubResource {
    pub manifest_id: String,
    pub role: EpubResourceRole,
    pub artifact: EmbeddedArtifact,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EpubResourceRole {
    Chapter,
    Navigation,
    Image,
    Style,
    Font,
    Audio,
    Video,
    Script,
    Other,
}

#[derive(Debug, Clone)]
struct ArchiveEntry {
    index: usize,
    name: String,
    compression: String,
    size: u64,
    encrypted: bool,
    kind: ArchiveEntryKind,
    rejected: Option<(String, String)>,
    bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct XmlElement {
    name: String,
    attributes: BTreeMap<String, String>,
    text: String,
    children: Vec<usize>,
    parent: Option<usize>,
    start: usize,
    end: usize,
    path: String,
}

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("zip + quick-xml + html5ever", "4.6.1/0.37.5/0.29.1")
        .with_specification_version("EPUB 2.0.1 and EPUB 3.3")
        .with_feature("epub")
}

pub(crate) fn parse_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    let options: EpubOptions =
        serde_json::from_value(context.options().clone()).map_err(|error| {
            Box::new(Diagnostic::malformed(PARSER, error.to_string())) as ParserError
        })?;
    let bytes = context.bytes();
    let parent_identity = ContentIdentity::for_raw_bytes(bytes)
        .with_format(FormatIdentity::new("epub", Some("application/epub+zip")));
    let (mut entries, mut diagnostics) = archive_entries(bytes, context)?;
    let outcome = parse_package(
        &mut entries,
        context.source(),
        &parent_identity,
        &options,
        &mut diagnostics,
    );
    let document = match outcome {
        Ok(document) => document,
        Err(PackageFailure::Encrypted(message)) => {
            return Ok(ParserOutput::terminal(
                OperationStatus::Encrypted,
                vec![Diagnostic::error(PARSER, "epub.encrypted", message)],
            ));
        }
        Err(PackageFailure::Malformed(message)) => {
            return Err(Box::new(Diagnostic::malformed(PARSER, message)));
        }
    };
    context.consume_child_artifacts(document.resources.len() as u64)?;
    context.consume_nodes(payload_node_count(&document) as u64)?;
    let value = serde_json::to_value(document).map_err(|error| {
        Box::new(Diagnostic::parser_defect(PARSER, error.to_string())) as ParserError
    })?;
    if diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        Ok(ParserOutput::partial(Some(value), diagnostics))
    } else {
        let mut output = ParserOutput::complete(value);
        output.diagnostics = diagnostics;
        Ok(output)
    }
}

fn payload_node_count(document: &EpubDocument) -> usize {
    1usize
        .saturating_add(document.metadata.len())
        .saturating_add(document.manifest.len())
        .saturating_add(document.spine.items.len())
        .saturating_add(
            document
                .navigation
                .iter()
                .map(|nav| nav.entries.len())
                .sum::<usize>(),
        )
        .saturating_add(
            document
                .chapters
                .iter()
                .map(|chapter| chapter.document.nodes.len())
                .sum::<usize>(),
        )
        .saturating_add(document.footnotes.len())
        .saturating_add(document.resources.len())
}

fn archive_entries(
    bytes: &[u8],
    context: &ParserContext<'_>,
) -> Result<(Vec<ArchiveEntry>, Vec<Diagnostic>), ParserError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
        Box::new(Diagnostic::malformed(
            PARSER,
            format!("invalid EPUB ZIP package: {error}"),
        )) as ParserError
    })?;
    context.consume_archive_members(archive.len() as u64)?;
    let policy = ArchiveSecurityPolicy::default();
    let mut entries = Vec::with_capacity(archive.len());
    let mut collisions = HashMap::<String, Vec<usize>>::new();
    let mut compressed = 0u64;
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        context.checkpoint()?;
        let file = archive.by_index_raw(index).map_err(|error| {
            Box::new(Diagnostic::malformed(
                PARSER,
                format!("cannot read ZIP member {index}: {error}"),
            )) as ParserError
        })?;
        let name = file.name().replace(char::from(92), "/");
        let kind = if file.is_dir() {
            ArchiveEntryKind::Directory
        } else if file.is_symlink() {
            ArchiveEntryKind::SymbolicLink
        } else {
            ArchiveEntryKind::RegularFile
        };
        let validated = policy.validate_member(&ArchiveMemberDescriptor {
            path: &name,
            kind,
            link_target: None,
        });
        let rejected = validated
            .as_ref()
            .err()
            .map(|error| (error.code.to_string(), error.message.clone()));
        if let Ok(key) = validated {
            collisions.entry(key).or_default().push(index);
        }
        compressed = compressed.saturating_add(file.compressed_size());
        expanded = expanded.saturating_add(file.size());
        entries.push(ArchiveEntry {
            index,
            name,
            compression: format!("{:?}", file.compression()).to_ascii_lowercase(),
            size: file.size(),
            encrypted: file.encrypted(),
            kind,
            rejected,
            bytes: None,
        });
    }
    context.observe_archive_expansion(compressed, expanded)?;
    context.observe_memory_bytes(expanded)?;
    for indexes in collisions.values().filter(|indexes| indexes.len() > 1) {
        for index in indexes {
            entries[*index].rejected = Some((
                "grist.security.archive.duplicate_path".into(),
                "archive path is ambiguous after cross-platform normalization".into(),
            ));
        }
    }
    let mut diagnostics = Vec::new();
    for entry in &entries {
        if let Some((code, message)) = &entry.rejected {
            diagnostics.push(
                Diagnostic::security_rejection(PARSER, format!("{}: {message}", entry.name))
                    .with_explanation_key(code.clone())
                    .with_locator(member_locator(entry))
                    .partial(),
            );
        }
    }
    for entry in entries.iter_mut().filter(|entry| {
        entry.rejected.is_none() && entry.kind == ArchiveEntryKind::RegularFile && !entry.encrypted
    }) {
        let mut file = match archive.by_index(entry.index) {
            Ok(file) => file,
            Err(error) => {
                diagnostics.push(
                    Diagnostic::unsupported(
                        PARSER,
                        format!("cannot decode ZIP member {}: {error}", entry.name),
                    )
                    .with_locator(member_locator(entry))
                    .partial(),
                );
                continue;
            }
        };
        let capacity = usize::try_from(entry.size)
            .unwrap_or(usize::MAX)
            .min(16 * 1024 * 1024);
        let mut output = Vec::with_capacity(capacity);
        let mut bounded = (&mut file).take(entry.size.saturating_add(1));
        if let Err(error) = bounded.read_to_end(&mut output) {
            diagnostics.push(
                Diagnostic::malformed(
                    PARSER,
                    format!("cannot read member {}: {error}", entry.name),
                )
                .with_locator(member_locator(entry))
                .partial(),
            );
            continue;
        }
        if output.len() as u64 != entry.size {
            diagnostics.push(
                Diagnostic::malformed(
                    PARSER,
                    format!(
                        "member {} size disagrees with its ZIP directory record",
                        entry.name
                    ),
                )
                .with_locator(member_locator(entry))
                .partial(),
            );
            continue;
        }
        entry.bytes = Some(output);
    }
    Ok((entries, diagnostics))
}

enum PackageFailure {
    Encrypted(String),
    Malformed(String),
}

fn parse_package(
    entries: &mut [ArchiveEntry],
    source: &SourceInfo,
    parent_identity: &ContentIdentity,
    options: &EpubOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<EpubDocument, PackageFailure> {
    validate_mimetype(entries, diagnostics);
    let container_entry = find_entry(entries, CONTAINER_PATH).ok_or_else(|| {
        PackageFailure::Malformed("EPUB has no safe META-INF/container.xml member".into())
    })?;
    if container_entry.encrypted {
        return Err(PackageFailure::Encrypted(
            "EPUB container metadata is encrypted".into(),
        ));
    }
    let container_bytes = container_entry.bytes.as_deref().ok_or_else(|| {
        PackageFailure::Malformed("EPUB container metadata cannot be decoded".into())
    })?;
    let (container_xml, container_xml_diagnostics) = parse_xml(container_bytes, container_entry);
    diagnostics.extend(container_xml_diagnostics);
    let mut rootfiles = container_xml
        .iter()
        .filter(|node| local_name(&node.name) == "rootfile")
        .filter_map(|node| {
            let full_path = attr(node, "full-path")?.to_string();
            Some(EpubRootfile {
                full_path,
                media_type: attr(node, "media-type").map(str::to_string),
                locator: xml_locator(container_entry, node),
            })
        })
        .collect::<Vec<_>>();
    if rootfiles.is_empty() {
        return Err(PackageFailure::Malformed(
            "EPUB container.xml declares no rootfile".into(),
        ));
    }
    let package_index = rootfiles
        .iter()
        .position(|root| find_entry(entries, &root.full_path).is_some())
        .ok_or_else(|| {
            PackageFailure::Malformed("no declared EPUB rootfile is present safely".into())
        })?;
    rootfiles.rotate_left(package_index);
    let package_path = rootfiles[0].full_path.clone();
    let package_entry = find_entry(entries, &package_path).ok_or_else(|| {
        PackageFailure::Malformed("selected EPUB package document is unavailable".into())
    })?;
    if package_entry.encrypted {
        return Err(PackageFailure::Encrypted(
            "EPUB package document is encrypted".into(),
        ));
    }
    let package_bytes = package_entry.bytes.as_deref().ok_or_else(|| {
        PackageFailure::Malformed("EPUB package document cannot be decoded".into())
    })?;
    let (package_xml, package_xml_diagnostics) = parse_xml(package_bytes, package_entry);
    diagnostics.extend(package_xml_diagnostics);
    let package_node = package_xml
        .iter()
        .find(|node| local_name(&node.name) == "package")
        .ok_or_else(|| PackageFailure::Malformed("OPF has no package element".into()))?;
    let version_text = attr(package_node, "version")
        .unwrap_or_default()
        .to_string();
    let version = if version_text.starts_with('2') {
        EpubVersion::Epub2
    } else if version_text.starts_with('3') {
        EpubVersion::Epub3
    } else {
        diagnostics.push(
            Diagnostic::unsupported(
                PARSER,
                format!("unknown EPUB package version {version_text:?}"),
            )
            .with_locator(xml_locator(package_entry, package_node))
            .partial(),
        );
        EpubVersion::Unknown(version_text)
    };
    let unique_identifier_id = attr(package_node, "unique-identifier").map(str::to_string);
    let metadata_parent = package_xml
        .iter()
        .find(|node| local_name(&node.name) == "metadata")
        .map(|node| node.path.clone());
    let metadata = package_xml
        .iter()
        .filter(|node| {
            node.parent
                .and_then(|index| package_xml.get(index))
                .is_some_and(|parent| metadata_parent.as_deref() == Some(parent.path.as_str()))
        })
        .map(|node| EpubMetadata {
            name: node.name.clone(),
            value: descendant_text(&package_xml, node).trim().to_string(),
            id: attr(node, "id").map(str::to_string),
            property: attr(node, "property").map(str::to_string),
            refines: attr(node, "refines").map(str::to_string),
            scheme: attr(node, "scheme")
                .or_else(|| attr(node, "opf:scheme"))
                .map(str::to_string),
            language: attr(node, "lang")
                .or_else(|| attr(node, "xml:lang"))
                .map(str::to_string),
            attributes: node.attributes.clone(),
            locator: xml_locator(package_entry, node),
        })
        .collect::<Vec<_>>();
    let encrypted_resources = encrypted_paths(entries, diagnostics);
    let encrypted_set = encrypted_resources.iter().cloned().collect::<BTreeSet<_>>();
    let manifest = parse_manifest(
        &package_xml,
        package_entry,
        &package_path,
        &encrypted_set,
        diagnostics,
    );
    if manifest.is_empty() {
        diagnostics.push(
            Diagnostic::malformed(PARSER, "OPF manifest is empty")
                .with_locator(member_locator(package_entry))
                .partial(),
        );
    }
    let spine = parse_spine(&package_xml, package_entry, &manifest, diagnostics);
    if spine.items.is_empty() {
        diagnostics.push(
            Diagnostic::malformed(PARSER, "OPF spine is empty")
                .with_locator(spine.locator.clone())
                .partial(),
        );
    }
    if spine.items.iter().any(|item| {
        item.resolved_path
            .as_ref()
            .is_some_and(|path| encrypted_set.contains(path))
    }) {
        return Err(PackageFailure::Encrypted(
            "one or more reading-order documents are encrypted".into(),
        ));
    }
    let (resources, artifact_ids) =
        build_resources(entries, &manifest, parent_identity, options, diagnostics);
    let mut chapters = parse_chapters(entries, source, &manifest, &spine, options, diagnostics);
    let navigation = parse_navigation(entries, &manifest, &spine, &mut chapters, diagnostics);
    let footnotes = parse_footnotes(&chapters);
    let images = build_images(&manifest, &chapters, &artifact_ids);
    let styles = build_styles(entries, &manifest, &artifact_ids);
    Ok(EpubDocument {
        schema_version: SchemaVersion::EPUB_V1.to_string(),
        version,
        package_path,
        package_locator: member_locator(package_entry),
        unique_identifier_id,
        metadata,
        manifest,
        spine,
        navigation,
        chapters,
        footnotes,
        images,
        styles,
        resources,
        rootfiles,
        encrypted_resources,
    })
}

fn validate_mimetype(entries: &[ArchiveEntry], diagnostics: &mut Vec<Diagnostic>) {
    let valid = entries.first().is_some_and(|entry| {
        entry.name == "mimetype"
            && entry.compression.contains("stored")
            && entry.bytes.as_deref() == Some(MIMETYPE)
    });
    if !valid {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                "EPUB mimetype must be the first, stored, exact application/epub+zip member",
            )
            .partial(),
        );
    }
}

fn encrypted_paths(entries: &[ArchiveEntry], diagnostics: &mut Vec<Diagnostic>) -> Vec<String> {
    let mut paths = entries
        .iter()
        .filter(|entry| entry.encrypted)
        .map(|entry| entry.name.clone())
        .collect::<BTreeSet<_>>();
    if let Some(entry) = find_entry(entries, "META-INF/encryption.xml")
        && let Some(bytes) = entry.bytes.as_deref()
    {
        let (xml, xml_diagnostics) = parse_xml(bytes, entry);
        diagnostics.extend(xml_diagnostics);
        for node in xml
            .iter()
            .filter(|node| local_name(&node.name) == "CipherReference")
        {
            if let Some(uri) = attr(node, "URI")
                && let Some(path) = resolve_member_path("META-INF/encryption.xml", uri)
            {
                paths.insert(path);
            }
        }
    }
    paths.into_iter().collect()
}

fn parse_manifest(
    xml: &[XmlElement],
    package: &ArchiveEntry,
    package_path: &str,
    encrypted: &BTreeSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<EpubManifestItem> {
    let mut seen = BTreeSet::new();
    xml.iter()
        .filter(|node| local_name(&node.name) == "item")
        .filter_map(|node| {
            let id = attr(node, "id")?.to_string();
            let href = attr(node, "href")?.to_string();
            let media_type = attr(node, "media-type")
                .unwrap_or("application/octet-stream")
                .to_string();
            if !seen.insert(id.clone()) {
                diagnostics.push(
                    Diagnostic::malformed(PARSER, format!("duplicate OPF manifest ID {id}"))
                        .with_locator(xml_locator(package, node))
                        .partial(),
                );
            }
            let resolved_path = resolve_member_path(package_path, &href);
            if resolved_path.is_none() {
                diagnostics.push(
                    Diagnostic::security_rejection(
                        PARSER,
                        format!("manifest href escapes the EPUB package: {href}"),
                    )
                    .with_locator(xml_locator(package, node))
                    .partial(),
                );
            }
            Some(EpubManifestItem {
                id,
                href,
                encrypted: resolved_path
                    .as_ref()
                    .is_some_and(|path| encrypted.contains(path)),
                resolved_path,
                media_type,
                properties: tokens(attr(node, "properties")),
                fallback: attr(node, "fallback").map(str::to_string),
                media_overlay: attr(node, "media-overlay").map(str::to_string),
                locator: xml_locator(package, node),
            })
        })
        .collect()
}

fn parse_spine(
    xml: &[XmlElement],
    package: &ArchiveEntry,
    manifest: &[EpubManifestItem],
    diagnostics: &mut Vec<Diagnostic>,
) -> EpubSpine {
    let spine_node = xml.iter().find(|node| local_name(&node.name) == "spine");
    let locator = spine_node
        .map(|node| xml_locator(package, node))
        .unwrap_or_else(|| member_locator(package));
    let toc = spine_node
        .and_then(|node| attr(node, "toc"))
        .map(str::to_string);
    let page_progression_direction = spine_node
        .and_then(|node| attr(node, "page-progression-direction"))
        .map(str::to_string);
    let parent = spine_node.map(|node| node.path.as_str());
    let mut items = Vec::new();
    for node in xml.iter().filter(|node| {
        local_name(&node.name) == "itemref"
            && node
                .parent
                .and_then(|index| xml.get(index))
                .is_some_and(|candidate| parent == Some(candidate.path.as_str()))
    }) {
        let Some(idref) = attr(node, "idref") else {
            diagnostics.push(
                Diagnostic::malformed(PARSER, "spine itemref has no idref")
                    .with_locator(xml_locator(package, node))
                    .partial(),
            );
            continue;
        };
        let manifest_index = manifest.iter().position(|item| item.id == idref);
        if manifest_index.is_none() {
            diagnostics.push(
                Diagnostic::malformed(
                    PARSER,
                    format!("spine idref {idref} is absent from manifest"),
                )
                .with_locator(xml_locator(package, node))
                .partial(),
            );
        }
        items.push(EpubSpineItem {
            position: items.len(),
            idref: idref.to_string(),
            linear: !attr(node, "linear").is_some_and(|value| value.eq_ignore_ascii_case("no")),
            properties: tokens(attr(node, "properties")),
            manifest_index,
            resolved_path: manifest_index.and_then(|index| manifest[index].resolved_path.clone()),
            locator: xml_locator(package, node),
        });
    }
    EpubSpine {
        toc,
        page_progression_direction,
        items,
        locator,
    }
}

fn build_resources(
    entries: &[ArchiveEntry],
    manifest: &[EpubManifestItem],
    parent_identity: &ContentIdentity,
    options: &EpubOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<EpubResource>, BTreeMap<String, String>) {
    let mut resources = Vec::new();
    let mut artifact_ids = BTreeMap::new();
    for item in manifest {
        let Some(path) = &item.resolved_path else {
            continue;
        };
        let Some(entry) = find_entry(entries, path) else {
            diagnostics.push(
                Diagnostic::malformed(PARSER, format!("manifest resource is missing: {path}"))
                    .with_locator(item.locator.clone())
                    .partial(),
            );
            continue;
        };
        let metadata = ArtifactMetadata::new(
            ArtifactParent::new(parent_identity.clone(), ArtifactRelationship::PackagePartOf),
            member_locator(entry),
            ArtifactDisposition::PackagePart,
        )
        .with_declared_filename(path)
        .with_media_type(&item.media_type);
        let artifact = if entry.encrypted || item.encrypted {
            EmbeddedArtifact::record_unavailable(
                metadata,
                ArtifactExtraction::new(
                    ArtifactExtractionStatus::Encrypted,
                    "epub.resource.encrypted",
                ),
            )
        } else if let Some(bytes) = entry.bytes.as_deref() {
            if options.inline_resource_bytes {
                EmbeddedArtifact::capture_inline(metadata, bytes)
            } else {
                EmbeddedArtifact::inventory(metadata, bytes)
            }
        } else {
            EmbeddedArtifact::record_unavailable(
                metadata,
                ArtifactExtraction::new(
                    ArtifactExtractionStatus::Unsupported,
                    "epub.resource.unavailable",
                ),
            )
        };
        match artifact {
            Ok(artifact) => {
                artifact_ids.insert(item.id.clone(), artifact.identity.artifact_id.clone());
                resources.push(EpubResource {
                    manifest_id: item.id.clone(),
                    role: resource_role(item),
                    artifact,
                });
            }
            Err(error) => diagnostics.push(
                Diagnostic::parser_defect(PARSER, error.to_string())
                    .with_locator(item.locator.clone())
                    .partial(),
            ),
        }
    }
    (resources, artifact_ids)
}

fn resource_role(item: &EpubManifestItem) -> EpubResourceRole {
    if item.properties.iter().any(|value| value == "nav") {
        EpubResourceRole::Navigation
    } else if matches!(
        item.media_type.as_str(),
        "application/xhtml+xml" | "text/html"
    ) {
        EpubResourceRole::Chapter
    } else if item.media_type.starts_with("image/") {
        EpubResourceRole::Image
    } else if item.media_type == "text/css" {
        EpubResourceRole::Style
    } else if item.media_type.starts_with("font/")
        || item.media_type.contains("font")
        || item.media_type.contains("opentype")
    {
        EpubResourceRole::Font
    } else if item.media_type.starts_with("audio/") {
        EpubResourceRole::Audio
    } else if item.media_type.starts_with("video/") {
        EpubResourceRole::Video
    } else if item.media_type.contains("javascript") || item.media_type.contains("ecmascript") {
        EpubResourceRole::Script
    } else {
        EpubResourceRole::Other
    }
}

fn parse_chapters(
    entries: &[ArchiveEntry],
    source: &SourceInfo,
    manifest: &[EpubManifestItem],
    spine: &EpubSpine,
    options: &EpubOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<EpubChapter> {
    let spine_by_id = spine
        .items
        .iter()
        .map(|item| (item.idref.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut selected = manifest
        .iter()
        .filter(|item| {
            matches!(
                item.media_type.as_str(),
                "application/xhtml+xml" | "text/html"
            )
        })
        .filter(|item| {
            options.retain_non_spine_documents || spine_by_id.contains_key(item.id.as_str())
        })
        .collect::<Vec<_>>();
    selected.sort_by_key(|item| {
        spine_by_id
            .get(item.id.as_str())
            .map(|spine| (0usize, spine.position))
            .unwrap_or((
                1,
                manifest
                    .iter()
                    .position(|candidate| candidate.id == item.id)
                    .unwrap_or(usize::MAX),
            ))
    });
    let mut chapters = Vec::new();
    for item in selected {
        let Some(path) = &item.resolved_path else {
            continue;
        };
        let Some(entry) = find_entry(entries, path) else {
            continue;
        };
        let Some(bytes) = entry.bytes.as_deref() else {
            continue;
        };
        let chapter_source = SourceInfo::new(path)
            .with_declared_mime_type(&item.media_type)
            .with_parent(source.clone());
        let html_options = HtmlOptions {
            syntax: if item.media_type == "application/xhtml+xml" {
                HtmlSyntax::Xhtml
            } else {
                HtmlSyntax::Auto
            },
            ..HtmlOptions::default()
        };
        let mut envelope = parse_html_bytes(bytes, chapter_source, &html_options);
        let Some(mut document) = envelope.payload.take() else {
            diagnostics.push(
                Diagnostic::malformed(PARSER, format!("chapter cannot be parsed: {path}"))
                    .with_locator(member_locator(entry))
                    .partial(),
            );
            continue;
        };
        nest_html_locators(&mut document, entry);
        for diagnostic in &mut envelope.diagnostics {
            nest_diagnostic_locator(diagnostic, entry);
            diagnostic.parser = PARSER.to_string();
            diagnostic.module = "epub.chapter".to_string();
        }
        diagnostics.extend(envelope.diagnostics);
        let title = document
            .metadata
            .iter()
            .find(|metadata| metadata.kind == "title")
            .and_then(|metadata| metadata.value.clone());
        let spine_item = spine_by_id.get(item.id.as_str()).copied();
        chapters.push(EpubChapter {
            spine_position: spine_item.map(|item| item.position),
            manifest_id: item.id.clone(),
            path: path.clone(),
            linear: spine_item.is_none_or(|item| item.linear),
            title,
            identity: ContentIdentity::for_raw_bytes(bytes)
                .with_format(FormatIdentity::new("html", Some(item.media_type.clone()))),
            locator: member_locator(entry),
            document,
        });
    }
    chapters
}

fn parse_navigation(
    entries: &[ArchiveEntry],
    manifest: &[EpubManifestItem],
    spine: &EpubSpine,
    chapters: &mut [EpubChapter],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<EpubNavigation> {
    let mut navigation = Vec::new();
    for item in manifest
        .iter()
        .filter(|item| item.properties.iter().any(|value| value == "nav"))
    {
        let Some(path) = item.resolved_path.as_deref() else {
            continue;
        };
        let Some(chapter) = chapters.iter().find(|chapter| chapter.path == path) else {
            diagnostics.push(
                Diagnostic::malformed(
                    PARSER,
                    format!("EPUB 3 navigation document is unavailable: {path}"),
                )
                .with_locator(item.locator.clone())
                .partial(),
            );
            continue;
        };
        navigation.extend(html_navigation(chapter));
    }
    if let Some(toc_id) = spine.toc.as_deref()
        && let Some(item) = manifest.iter().find(|item| item.id == toc_id)
        && let Some(path) = item.resolved_path.as_deref()
        && let Some(entry) = find_entry(entries, path)
        && let Some(bytes) = entry.bytes.as_deref()
    {
        let (xml, xml_diagnostics) = parse_xml(bytes, entry);
        diagnostics.extend(xml_diagnostics);
        navigation.push(ncx_navigation(&xml, entry));
    }
    if navigation.is_empty() {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "epub.navigation.missing",
                "EPUB has no readable EPUB 3 nav or EPUB 2 NCX",
            )
            .partial(),
        );
    }
    navigation
}

fn html_navigation(chapter: &EpubChapter) -> Vec<EpubNavigation> {
    let nodes = &chapter.document.nodes;
    let by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    nodes
        .iter()
        .filter(|node| node.tag_name.as_deref() == Some("nav"))
        .map(|nav| {
            let kind = html_attr(nav, "type")
                .or_else(|| html_attr(nav, "epub:type"))
                .unwrap_or("toc")
                .to_string();
            let entries = nodes
                .iter()
                .filter(|node| {
                    node.tag_name.as_deref() == Some("a") && is_html_descendant(node, nav, &by_id)
                })
                .filter_map(|node| {
                    let href = html_attr(node, "href")?.to_string();
                    let (resolved_path, fragment) = resolve_href(&chapter.path, &href);
                    Some(EpubNavigationEntry {
                        label: html_descendant_text(nodes, &node.id).trim().to_string(),
                        href,
                        resolved_path,
                        fragment,
                        depth: ancestor_tag_count(node, nav, &by_id, "li"),
                        play_order: None,
                        locator: node.locator.clone(),
                    })
                })
                .collect();
            EpubNavigation {
                kind,
                source_path: chapter.path.clone(),
                entries,
                locator: nav.locator.clone(),
            }
        })
        .collect()
}

fn ncx_navigation(xml: &[XmlElement], entry: &ArchiveEntry) -> EpubNavigation {
    let mut entries = Vec::new();
    for node in xml
        .iter()
        .filter(|node| local_name(&node.name) == "navPoint")
    {
        let label = first_descendant(xml, node, "text")
            .map(|node| descendant_text(xml, node).trim().to_string())
            .unwrap_or_default();
        let href = first_descendant(xml, node, "content")
            .and_then(|node| attr(node, "src"))
            .unwrap_or_default()
            .to_string();
        let (resolved_path, fragment) = resolve_href(&entry.name, &href);
        entries.push(EpubNavigationEntry {
            label,
            href,
            resolved_path,
            fragment,
            depth: ancestor_xml_count(xml, node, "navPoint"),
            play_order: attr(node, "playOrder").and_then(|value| value.parse().ok()),
            locator: xml_locator(entry, node),
        });
    }
    EpubNavigation {
        kind: "toc".into(),
        source_path: entry.name.clone(),
        entries,
        locator: member_locator(entry),
    }
}

fn parse_footnotes(chapters: &[EpubChapter]) -> Vec<EpubFootnote> {
    let mut references = BTreeMap::<(String, String), Vec<EpubFootnoteReference>>::new();
    for chapter in chapters {
        for node in chapter
            .document
            .nodes
            .iter()
            .filter(|node| node.tag_name.as_deref() == Some("a"))
        {
            let semantics = format!(
                "{} {}",
                html_attr(node, "type")
                    .or_else(|| html_attr(node, "epub:type"))
                    .unwrap_or_default(),
                html_attr(node, "role").unwrap_or_default()
            );
            if !semantics
                .split_ascii_whitespace()
                .any(|value| matches!(value, "noteref" | "doc-noteref"))
            {
                continue;
            }
            let Some(href) = html_attr(node, "href") else {
                continue;
            };
            let (path, fragment) = resolve_href(&chapter.path, href);
            let target_path = path.unwrap_or_else(|| chapter.path.clone());
            let Some(fragment) = fragment else {
                continue;
            };
            references
                .entry((target_path, fragment))
                .or_default()
                .push(EpubFootnoteReference {
                    chapter_path: chapter.path.clone(),
                    href: href.to_string(),
                    resolved: false,
                    locator: node.locator.clone(),
                });
        }
    }
    let mut notes = Vec::new();
    for chapter in chapters {
        for node in &chapter.document.nodes {
            let semantics = format!(
                "{} {} {}",
                html_attr(node, "type")
                    .or_else(|| html_attr(node, "epub:type"))
                    .unwrap_or_default(),
                html_attr(node, "role").unwrap_or_default(),
                html_attr(node, "class").unwrap_or_default()
            );
            let kind = semantics.split_ascii_whitespace().find(|value| {
                matches!(
                    *value,
                    "footnote" | "endnote" | "rearnote" | "doc-footnote" | "doc-endnote"
                )
            });
            let Some(kind) = kind else {
                continue;
            };
            let Some(id) = html_attr(node, "id") else {
                continue;
            };
            let mut note_refs = references
                .remove(&(chapter.path.clone(), id.to_string()))
                .unwrap_or_default();
            for reference in &mut note_refs {
                reference.resolved = true;
            }
            notes.push(EpubFootnote {
                id: id.to_string(),
                kind: kind.to_string(),
                chapter_path: chapter.path.clone(),
                text: html_descendant_text(&chapter.document.nodes, &node.id)
                    .trim()
                    .to_string(),
                references: note_refs,
                locator: node.locator.clone(),
            });
        }
    }
    for ((path, id), unresolved) in references {
        let Some(locator) = unresolved
            .first()
            .map(|reference| reference.locator.clone())
        else {
            continue;
        };
        notes.push(EpubFootnote {
            id,
            kind: "unresolved".into(),
            chapter_path: path,
            text: String::new(),
            references: unresolved,
            locator,
        });
    }
    notes
}

fn build_images(
    manifest: &[EpubManifestItem],
    chapters: &[EpubChapter],
    artifacts: &BTreeMap<String, String>,
) -> Vec<EpubImage> {
    manifest
        .iter()
        .filter(|item| item.media_type.starts_with("image/"))
        .map(|item| {
            let usages = chapters
                .iter()
                .flat_map(|chapter| {
                    chapter.document.media.iter().flat_map(move |media| {
                        media.sources.iter().filter_map(move |source| {
                            let (path, _) = resolve_href(&chapter.path, source);
                            (path.as_ref() == item.resolved_path.as_ref()).then(|| EpubImageUsage {
                                chapter_path: chapter.path.clone(),
                                source: source.clone(),
                                alt_text: media.alt_text.clone(),
                                locator: media.locator.clone(),
                            })
                        })
                    })
                })
                .collect();
            EpubImage {
                manifest_id: item.id.clone(),
                path: item.resolved_path.clone(),
                media_type: item.media_type.clone(),
                properties: item.properties.clone(),
                usages,
                artifact_id: artifacts.get(&item.id).cloned(),
                locator: item.locator.clone(),
            }
        })
        .collect()
}

fn build_styles(
    entries: &[ArchiveEntry],
    manifest: &[EpubManifestItem],
    artifacts: &BTreeMap<String, String>,
) -> Vec<EpubStyle> {
    manifest
        .iter()
        .filter(|item| item.media_type == "text/css")
        .map(|item| {
            let text = item
                .resolved_path
                .as_deref()
                .and_then(|path| find_entry(entries, path))
                .and_then(|entry| entry.bytes.as_deref())
                .map(String::from_utf8_lossy);
            EpubStyle {
                manifest_id: item.id.clone(),
                path: item.resolved_path.clone(),
                media_type: item.media_type.clone(),
                character_count: text.as_ref().map(|text| text.chars().count() as u64),
                referenced_resources: text.as_deref().map(css_references).unwrap_or_default(),
                artifact_id: artifacts.get(&item.id).cloned(),
                locator: item.locator.clone(),
            }
        })
        .collect()
}

fn css_references(css: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut remainder = css;
    while let Some(position) = remainder.find("url(") {
        remainder = &remainder[position + 4..];
        let Some(end) = remainder.find(')') else {
            break;
        };
        let value = remainder[..end].trim().trim_matches(|character: char| {
            character == char::from(39) || character == char::from(34)
        });
        if !value.is_empty() {
            references.push(value.to_string());
        }
        remainder = &remainder[end + 1..];
    }
    references.sort();
    references.dedup();
    references
}

fn parse_xml(bytes: &[u8], entry: &ArchiveEntry) -> (Vec<XmlElement>, Vec<Diagnostic>) {
    let text = String::from_utf8_lossy(bytes);
    let line_index = LineIndex::new(&text);
    let mut diagnostics = Vec::new();
    if std::str::from_utf8(bytes).is_err() {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                format!(
                    "XML member {} is not valid UTF-8; replacement text was retained",
                    entry.name
                ),
            )
            .with_locator(member_locator(entry))
            .partial(),
        );
    }
    let mut reader = Reader::from_str(&text);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut nodes = Vec::<XmlElement>::new();
    let mut stack = Vec::<usize>::new();
    let mut sibling_counts = Vec::<BTreeMap<String, usize>>::new();
    let mut previous = 0usize;
    loop {
        let event = reader.read_event();
        let end = usize::try_from(reader.buffer_position())
            .unwrap_or(text.len())
            .min(text.len());
        match event {
            Ok(Event::Start(start)) => {
                if let Some(error) = xml_start(
                    &start,
                    false,
                    previous,
                    end,
                    &mut nodes,
                    &mut stack,
                    &mut sibling_counts,
                ) {
                    diagnostics.push(xml_attribute_diagnostic(
                        entry,
                        previous,
                        end,
                        &line_index,
                        error,
                    ));
                }
            }
            Ok(Event::Empty(start)) => {
                if let Some(error) = xml_start(
                    &start,
                    true,
                    previous,
                    end,
                    &mut nodes,
                    &mut stack,
                    &mut sibling_counts,
                ) {
                    diagnostics.push(xml_attribute_diagnostic(
                        entry,
                        previous,
                        end,
                        &line_index,
                        error,
                    ));
                }
            }
            Ok(Event::End(_)) => {
                if let Some(index) = stack.pop() {
                    nodes[index].end = end;
                }
            }
            Ok(Event::Text(value)) => {
                if let Some(index) = stack.last().copied() {
                    nodes[index]
                        .text
                        .push_str(&decode_xml(&String::from_utf8_lossy(value.as_ref())));
                }
            }
            Ok(Event::CData(value)) => {
                if let Some(index) = stack.last().copied() {
                    nodes[index]
                        .text
                        .push_str(&String::from_utf8_lossy(value.as_ref()));
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                let range = SourceRange::new(previous, end.max(previous), &line_index);
                let locator = member_locator(entry)
                    .nested(LocationComponent::from(range))
                    .expect("valid nested XML locator");
                diagnostics.push(
                    Diagnostic::malformed(
                        PARSER,
                        format!("malformed XML in {}: {error}", entry.name),
                    )
                    .with_locator(locator)
                    .partial(),
                );
                break;
            }
            _ => {}
        }
        previous = end;
    }
    if !stack.is_empty() {
        diagnostics.push(
            Diagnostic::malformed(PARSER, format!("unclosed XML elements in {}", entry.name))
                .with_locator(member_locator(entry))
                .partial(),
        );
    }
    (nodes, diagnostics)
}

fn xml_start(
    start: &BytesStart<'_>,
    empty: bool,
    begin: usize,
    end: usize,
    nodes: &mut Vec<XmlElement>,
    stack: &mut Vec<usize>,
    sibling_counts: &mut Vec<BTreeMap<String, usize>>,
) -> Option<String> {
    let name = qname(start);
    let (attributes, attribute_error) = xml_attributes(start);
    let parent = stack.last().copied();
    if sibling_counts.len() <= stack.len() {
        sibling_counts.push(BTreeMap::new());
    }
    let count = sibling_counts[stack.len()].entry(name.clone()).or_default();
    *count += 1;
    let path = parent
        .and_then(|index| nodes.get(index))
        .map(|node| format!("{}/{}[{}]", node.path, name, count))
        .unwrap_or_else(|| format!("/{}[{}]", name, count));
    let index = nodes.len();
    nodes.push(XmlElement {
        name,
        attributes,
        text: String::new(),
        children: Vec::new(),
        parent,
        start: begin,
        end,
        path,
    });
    if let Some(parent) = parent {
        nodes[parent].children.push(index);
    }
    if !empty {
        stack.push(index);
        if sibling_counts.len() <= stack.len() {
            sibling_counts.push(BTreeMap::new());
        } else {
            sibling_counts[stack.len()].clear();
        }
    }
    attribute_error
}

fn qname(start: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(start.name().as_ref()).to_string()
}

fn xml_attributes(start: &BytesStart<'_>) -> (BTreeMap<String, String>, Option<String>) {
    let mut attributes = BTreeMap::new();
    let mut error = None;
    for attribute in start.attributes().with_checks(true) {
        match attribute {
            Ok(attribute) => {
                attributes.insert(
                    String::from_utf8_lossy(attribute.key.as_ref()).to_string(),
                    decode_xml(&String::from_utf8_lossy(attribute.value.as_ref())),
                );
            }
            Err(attribute_error) => error = Some(attribute_error.to_string()),
        }
    }
    (attributes, error)
}

fn xml_attribute_diagnostic(
    entry: &ArchiveEntry,
    start: usize,
    end: usize,
    line_index: &LineIndex,
    error: String,
) -> Diagnostic {
    let range = SourceRange::new(start, end.max(start), line_index);
    let locator = member_locator(entry)
        .nested(LocationComponent::from(range))
        .expect("valid nested XML attribute locator");
    Diagnostic::malformed(
        PARSER,
        format!("malformed XML attributes in {}: {error}", entry.name),
    )
    .with_locator(locator)
    .partial()
}

fn decode_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn attr<'a>(node: &'a XmlElement, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(key, _)| {
            key.eq_ignore_ascii_case(name) || local_name(key).eq_ignore_ascii_case(name)
        })
        .map(|(_, value)| value.as_str())
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn descendant_text(nodes: &[XmlElement], node: &XmlElement) -> String {
    let mut output = node.text.clone();
    for child in &node.children {
        output.push_str(&descendant_text(nodes, &nodes[*child]));
    }
    output
}

fn first_descendant<'a>(
    nodes: &'a [XmlElement],
    root: &XmlElement,
    name: &str,
) -> Option<&'a XmlElement> {
    root.children.iter().find_map(|index| {
        let node = &nodes[*index];
        (local_name(&node.name) == name)
            .then_some(node)
            .or_else(|| first_descendant(nodes, node, name))
    })
}

fn ancestor_xml_count(nodes: &[XmlElement], node: &XmlElement, name: &str) -> usize {
    let mut count = 0;
    let mut parent = node.parent;
    while let Some(index) = parent {
        let node = &nodes[index];
        if local_name(&node.name) == name {
            count += 1;
        }
        parent = node.parent;
    }
    count
}

fn find_entry<'a>(entries: &'a [ArchiveEntry], name: &str) -> Option<&'a ArchiveEntry> {
    entries
        .iter()
        .find(|entry| entry.name == name && entry.rejected.is_none())
}

fn member_locator(entry: &ArchiveEntry) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: entry.name.clone(),
        member_index: IndexPosition::new(entry.index as u64, IndexBase::Zero)
            .expect("zero-based archive member index is valid"),
    })
    .expect("validated archive member locator")
}

fn xml_locator(entry: &ArchiveEntry, node: &XmlElement) -> SourceLocator {
    let text = String::from_utf8_lossy(entry.bytes.as_deref().unwrap_or_default());
    let index = LineIndex::new(&text);
    let range = SourceRange::new(node.start, node.end.max(node.start), &index);
    member_locator(entry)
        .nested(LocationComponent::from(range))
        .and_then(|locator| {
            locator.nested(LocationComponent::XmlPath {
                path: node.path.clone(),
            })
        })
        .expect("valid package XML locator")
}

fn tokens(value: Option<&str>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(str::split_ascii_whitespace)
        .map(str::to_string)
        .collect()
}

fn resolve_href(base_member: &str, href: &str) -> (Option<String>, Option<String>) {
    let (path, fragment) = href
        .split_once('#')
        .map_or((href, None), |(path, fragment)| {
            (
                path,
                (!fragment.is_empty()).then(|| percent_decode(fragment)),
            )
        });
    let resolved = if path.is_empty() {
        Some(base_member.to_string())
    } else {
        resolve_member_path(base_member, path)
    };
    (resolved, fragment)
}

fn resolve_member_path(base_member: &str, reference: &str) -> Option<String> {
    let reference = reference.split('#').next().unwrap_or(reference);
    if reference.contains("//") || reference.starts_with('/') || reference.contains(char::from(92))
    {
        return None;
    }
    let decoded = percent_decode(reference);
    if decoded.contains(':') || decoded.starts_with('/') {
        return None;
    }
    let mut components = base_member.split('/').collect::<Vec<_>>();
    components.pop();
    for component in decoded.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            value if value.chars().any(char::is_control) => return None,
            value => components.push(value),
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            output.push(high * 16 + low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn nest_html_locators(document: &mut HtmlDocument, entry: &ArchiveEntry) {
    let mut value = serde_json::to_value(&*document).expect("HTML document serializes");
    nest_locator_values(&mut value, entry);
    *document = serde_json::from_value(value).expect("nested HTML locators remain valid");
}

fn nest_locator_values(value: &mut serde_json::Value, entry: &ArchiveEntry) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(locator_value) = object.get_mut("locator")
                && let Ok(locator) = serde_json::from_value::<SourceLocator>(locator_value.clone())
            {
                let mut components = vec![LocationComponent::ArchiveMember {
                    member_path: entry.name.clone(),
                    member_index: IndexPosition::new(entry.index as u64, IndexBase::Zero)
                        .expect("zero-based archive member index is valid"),
                }];
                components.extend_from_slice(locator.components());
                if let Ok(nested) = SourceLocator::new(components, locator.precision().clone()) {
                    *locator_value = serde_json::to_value(nested).expect("locator serializes");
                }
            }
            for child in object.values_mut() {
                nest_locator_values(child, entry);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                nest_locator_values(child, entry);
            }
        }
        _ => {}
    }
}

fn nest_diagnostic_locator(diagnostic: &mut Diagnostic, entry: &ArchiveEntry) {
    if let Some(locator) = diagnostic.locator.take() {
        let mut components = vec![LocationComponent::ArchiveMember {
            member_path: entry.name.clone(),
            member_index: IndexPosition::new(entry.index as u64, IndexBase::Zero)
                .expect("zero-based archive member index is valid"),
        }];
        components.extend_from_slice(locator.components());
        diagnostic.locator = SourceLocator::new(components, locator.precision().clone())
            .ok()
            .map(Box::new);
    } else if let Some(range) = diagnostic.range.clone() {
        diagnostic.locator = member_locator(entry)
            .nested(LocationComponent::from(range))
            .ok()
            .map(Box::new);
    }
}

fn html_attr<'a>(node: &'a HtmlNode, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|attribute| {
            attribute.name.eq_ignore_ascii_case(name)
                || attribute.local_name.eq_ignore_ascii_case(name)
        })
        .and_then(|attribute| attribute.value.as_deref())
}

fn html_descendant_text(nodes: &[HtmlNode], root_id: &str) -> String {
    let by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    fn append(node_id: &str, by_id: &HashMap<&str, &HtmlNode>, output: &mut String) {
        let Some(node) = by_id.get(node_id).copied() else {
            return;
        };
        if node.kind == HtmlNodeKind::Text {
            output.push_str(node.text.as_deref().unwrap_or_default());
        }
        for child in &node.children {
            append(child, by_id, output);
        }
    }
    let mut output = String::new();
    append(root_id, &by_id, &mut output);
    output
}

fn is_html_descendant(node: &HtmlNode, root: &HtmlNode, by_id: &HashMap<&str, &HtmlNode>) -> bool {
    let mut parent = node.parent_id.as_deref();
    while let Some(id) = parent {
        if id == root.id {
            return true;
        }
        parent = by_id.get(id).and_then(|node| node.parent_id.as_deref());
    }
    false
}

fn ancestor_tag_count(
    node: &HtmlNode,
    root: &HtmlNode,
    by_id: &HashMap<&str, &HtmlNode>,
    tag: &str,
) -> usize {
    let mut count = 0;
    let mut parent = node.parent_id.as_deref();
    while let Some(id) = parent {
        if id == root.id {
            break;
        }
        let Some(node) = by_id.get(id).copied() else {
            break;
        };
        if node.tag_name.as_deref() == Some(tag) {
            count += 1;
        }
        parent = node.parent_id.as_deref();
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_resolution_never_escapes_package() {
        assert_eq!(
            resolve_member_path("OPS/book.opf", "text/a.xhtml"),
            Some("OPS/text/a.xhtml".into())
        );
        assert_eq!(
            resolve_member_path("OPS/book.opf", "../a.xhtml"),
            Some("a.xhtml".into())
        );
        assert_eq!(resolve_member_path("book.opf", "../a.xhtml"), None);
        assert_eq!(resolve_member_path("OPS/book.opf", "%2e%2e/%2e%2e/a"), None);
    }
}
