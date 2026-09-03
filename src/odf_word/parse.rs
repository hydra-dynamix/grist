//! OpenDocument package interpretation and structural text projections.

use super::archive::{PackageEntry, find_entry};
use super::model::*;
use super::xml::{
    XmlContent, XmlDocument, XmlElement, attr, descendant_text, element_locator, local_name,
    parse_xml, raw_xml, text_locator,
};
use super::{PARSER, member_locator};
use crate::container::{
    ArtifactDisposition, ArtifactMetadata, ArtifactParent, ArtifactRelationship, EmbeddedArtifact,
};
use crate::core::{ContentIdentity, Diagnostic, FormatIdentity, OperationStatus, SchemaVersion};
use crate::registry::{ParserContext, ParserError, ParserOutput};
use crate::security::ArchiveEntryKind;
use std::collections::{BTreeMap, HashMap};

const CONTENT: &str = "content.xml";
const STYLES: &str = "styles.xml";
const META: &str = "meta.xml";
const SETTINGS: &str = "settings.xml";
const MANIFEST: &str = "META-INF/manifest.xml";

pub(super) fn parse_registered(
    context: &mut ParserContext<'_>,
    expected_kind: OdfPackageKind,
) -> Result<ParserOutput, ParserError> {
    let options: OdfWordOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(PARSER, error.to_string())))?;
    let mut package = super::archive::read_package(context.bytes(), context)?;
    let mut diagnostics = std::mem::take(&mut package.diagnostics);
    super::xml::observe_xml_nesting(&package.entries, context)?;
    if package
        .entries
        .iter()
        .any(|entry| entry.path == "mimetype" && entry.encrypted)
    {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![Diagnostic::error(
                PARSER,
                "odf_word.encrypted.mimetype",
                "OpenDocument mimetype member is encrypted and cannot be validated",
            )],
        ));
    }
    validate_mimetype(&package.entries, expected_kind)?;
    if package
        .entries
        .iter()
        .any(|entry| entry.path == CONTENT && entry.encrypted)
    {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![Diagnostic::error(
                PARSER,
                "odf_word.encrypted.content",
                "OpenDocument content.xml is encrypted and requires a decryption provider",
            )],
        ));
    }
    let manifest = parse_manifest(&package.entries, &mut diagnostics);
    if let Some(encrypted) = manifest
        .iter()
        .find(|entry| entry.full_path == CONTENT && entry.encrypted)
    {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![
                Diagnostic::error(
                    PARSER,
                    "odf_word.encrypted.manifest",
                    "OpenDocument manifest declares content.xml as encrypted",
                )
                .with_locator(encrypted.locator.clone()),
            ],
        ));
    }
    for encrypted in manifest
        .iter()
        .filter(|entry| entry.encrypted && entry.full_path != CONTENT)
    {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "odf_word.encrypted.member",
                format!(
                    "encrypted OpenDocument member `{}` is inventoried but unavailable",
                    encrypted.full_path
                ),
            )
            .with_locator(encrypted.locator.clone())
            .partial(),
        );
    }
    for encrypted in package.entries.iter().filter(|entry| {
        entry.encrypted
            && entry.path != CONTENT
            && entry.path != "mimetype"
            && !manifest
                .iter()
                .any(|item| item.full_path == entry.path && item.encrypted)
    }) {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "odf_word.encrypted.member",
                format!(
                    "encrypted OpenDocument member `{}` is inventoried but unavailable",
                    encrypted.path
                ),
            )
            .with_locator(
                member_locator(encrypted)
                    .unwrap_or_else(|_| super::fallback_member_locator(encrypted.index)),
            )
            .partial(),
        );
    }
    let content_entry = find_entry(&package.entries, CONTENT).ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "OpenDocument package has no safe content.xml member",
        )) as ParserError
    })?;
    let content_xml = parse_xml(content_entry, &mut diagnostics);
    let (body, version, revisions) = if let Some(document) = &content_xml {
        let version = document
            .nodes
            .first()
            .and_then(|node| attr(node, "version"))
            .map(str::to_string);
        let root = document
            .nodes
            .iter()
            .position(|node| local_name(&node.name) == "text");
        let body = root.map_or_else(
            || {
                diagnostics.push(
                    Diagnostic::malformed(PARSER, "content.xml has no office:text body")
                        .with_locator(member_locator(content_entry).expect("valid member"))
                        .partial(),
                );
                empty_body(content_entry)
            },
            |index| Converter::new(content_entry, document).convert(index, None),
        );
        (body, version, parse_revisions(content_entry, document))
    } else {
        (empty_body(content_entry), None, Vec::new())
    };
    let metadata = parse_properties_member(&package.entries, META, "meta", &mut diagnostics);
    let settings =
        parse_properties_member(&package.entries, SETTINGS, "settings", &mut diagnostics);
    let (styles, list_styles, master_pages) =
        parse_styles(&package.entries, content_xml.as_ref(), &mut diagnostics);
    let parent_identity = ContentIdentity::for_raw_bytes(context.bytes()).with_format(
        FormatIdentity::new(expected_kind.format_id(), Some(expected_kind.media_type())),
    );
    let artifacts = build_artifacts(&package.entries, &manifest, &parent_identity, &options)?;
    context.consume_child_artifacts(artifacts.len() as u64)?;
    let artifact_paths = artifact_path_map(&artifacts);
    let mut semantics = Semantics::default();
    collect_semantics(&body, &manifest, &artifact_paths, &mut semantics);
    collect_embedded_equations(&package.entries, &mut diagnostics, &mut semantics.equations);
    validate_revision_graph(&body, &revisions, &mut diagnostics);
    let views = render_views(&body, &revisions);
    let parts = package_parts(&package.entries, &manifest);
    let node_count = count_nodes(&body)
        .saturating_add(parts.len())
        .saturating_add(manifest.len())
        .saturating_add(metadata.len())
        .saturating_add(settings.len())
        .saturating_add(styles.len())
        .saturating_add(list_styles.len())
        .saturating_add(master_pages.len())
        .saturating_add(revisions.len())
        .saturating_add(artifacts.len());
    context.consume_nodes(node_count as u64)?;
    let document = OdfWordDocument {
        schema_version: SchemaVersion::ODF_WORD_V1.into(),
        package_kind: expected_kind,
        package_media_type: expected_kind.media_type().into(),
        version,
        parts,
        manifest,
        metadata,
        settings,
        styles,
        list_styles,
        master_pages,
        body,
        revisions,
        links: semantics.links,
        notes: semantics.notes,
        annotations: semantics.annotations,
        drawings: semantics.drawings,
        equations: semantics.equations,
        embedded_objects: semantics.embedded_objects,
        embedded_artifacts: artifacts,
        views,
    };
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

fn validate_mimetype(
    entries: &[PackageEntry],
    expected_kind: OdfPackageKind,
) -> Result<(), ParserError> {
    let entry = find_entry(entries, "mimetype").ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "OpenDocument package has no safe mimetype member",
        )) as ParserError
    })?;
    let actual = entry
        .bytes
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(str::trim)
        .unwrap_or_default();
    if actual != expected_kind.media_type() {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            format!(
                "requested {} but package mimetype declares {actual:?}",
                expected_kind.format_id()
            ),
        )));
    }
    Ok(())
}

fn parse_manifest(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<OdfManifestEntry> {
    let Some(entry) = find_entry(entries, MANIFEST) else {
        diagnostics.push(
            Diagnostic::malformed(PARSER, "OpenDocument package has no safe manifest.xml")
                .partial(),
        );
        return Vec::new();
    };
    let Some(document) = parse_xml(entry, diagnostics) else {
        return Vec::new();
    };
    document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| local_name(&node.name) == "file-entry")
        .filter_map(|(index, node)| {
            let full_path = attr(node, "full-path")?.to_string();
            let encryption_nodes = descendants(&document, index)
                .filter(|child| local_name(&child.name) == "encryption-data")
                .collect::<Vec<_>>();
            let mut encryption_attributes = BTreeMap::new();
            for encryption in &encryption_nodes {
                encryption_attributes.extend(encryption.attributes.clone());
            }
            Some(OdfManifestEntry {
                full_path,
                media_type: attr(node, "media-type")
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                version: attr(node, "version").map(str::to_string),
                size: attr(node, "size").and_then(|value| value.parse().ok()),
                encrypted: !encryption_nodes.is_empty(),
                checksum: encryption_nodes
                    .iter()
                    .find_map(|node| attr(node, "checksum"))
                    .map(str::to_string),
                checksum_type: encryption_nodes
                    .iter()
                    .find_map(|node| attr(node, "checksum-type"))
                    .map(str::to_string),
                encryption_attributes,
                locator: element_locator(entry, &document, node),
            })
        })
        .collect()
}

fn parse_properties_member(
    entries: &[PackageEntry],
    path: &str,
    container_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<OdfProperty> {
    let Some(entry) = find_entry(entries, path) else {
        return Vec::new();
    };
    let Some(document) = parse_xml(entry, diagnostics) else {
        return Vec::new();
    };
    let Some(container) = document
        .nodes
        .iter()
        .position(|node| local_name(&node.name) == container_name)
    else {
        return Vec::new();
    };
    child_indexes(&document.nodes[container])
        .map(|index| {
            let node = &document.nodes[index];
            OdfProperty {
                qualified_name: node.name.clone(),
                value: descendant_text(&document, index).trim().to_string(),
                attributes: node.attributes.clone(),
                locator: element_locator(entry, &document, node),
            }
        })
        .collect()
}

fn parse_styles(
    entries: &[PackageEntry],
    content_xml: Option<&XmlDocument>,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<OdfStyle>, Vec<OdfListStyle>, Vec<OdfMasterPage>) {
    let mut sources = Vec::new();
    if let Some(entry) = find_entry(entries, STYLES)
        && let Some(document) = parse_xml(entry, diagnostics)
    {
        sources.push((entry, document));
    }
    if let Some(entry) = find_entry(entries, CONTENT)
        && let Some(document) = content_xml
    {
        sources.push((
            entry,
            XmlDocument {
                nodes: document.nodes.clone(),
                text: document.text.clone(),
                line_index: document.line_index.clone(),
            },
        ));
    }
    let mut styles = Vec::new();
    let mut list_styles = Vec::new();
    let mut master_pages = Vec::new();
    for (entry, document) in &sources {
        for node in &document.nodes {
            match local_name(&node.name) {
                "style" | "default-style" | "page-layout" | "presentation-page-layout" => {
                    let properties = child_indexes(node)
                        .filter_map(|child| {
                            let child = &document.nodes[child];
                            local_name(&child.name)
                                .ends_with("properties")
                                .then(|| (child.name.clone(), child.attributes.clone()))
                        })
                        .collect();
                    styles.push(OdfStyle {
                        qualified_name: node.name.clone(),
                        name: attr(node, "name").map(str::to_string),
                        display_name: attr(node, "display-name").map(str::to_string),
                        family: attr(node, "family").map(str::to_string),
                        parent_style_name: attr(node, "parent-style-name").map(str::to_string),
                        next_style_name: attr(node, "next-style-name").map(str::to_string),
                        list_style_name: attr(node, "list-style-name").map(str::to_string),
                        data_style_name: attr(node, "data-style-name").map(str::to_string),
                        attributes: node.attributes.clone(),
                        properties,
                        locator: element_locator(entry, document, node),
                    });
                }
                "list-style" | "outline-style" => {
                    let levels = child_indexes(node)
                        .filter_map(|child| {
                            let child = &document.nodes[child];
                            let local = local_name(&child.name);
                            local.starts_with("list-level-style").then(|| OdfListLevel {
                                level: attr(child, "level").and_then(|value| value.parse().ok()),
                                kind: local.to_string(),
                                number_format: attr(child, "num-format").map(str::to_string),
                                bullet_character: attr(child, "bullet-char").map(str::to_string),
                                prefix: attr(child, "num-prefix").map(str::to_string),
                                suffix: attr(child, "num-suffix").map(str::to_string),
                                start_value: attr(child, "start-value")
                                    .and_then(|value| value.parse().ok()),
                                attributes: child.attributes.clone(),
                                locator: element_locator(entry, document, child),
                            })
                        })
                        .collect();
                    list_styles.push(OdfListStyle {
                        name: attr(node, "name").map(str::to_string),
                        consecutive_numbering: attr(node, "consecutive-numbering")
                            .map(str::to_string),
                        levels,
                        locator: element_locator(entry, document, node),
                    });
                }
                "master-page" => {
                    let converter = Converter::new(entry, document);
                    let headers = child_indexes(node)
                        .filter(|child| {
                            local_name(&document.nodes[*child].name).starts_with("header")
                        })
                        .map(|child| converter.convert(child, None))
                        .collect();
                    let footers = child_indexes(node)
                        .filter(|child| {
                            local_name(&document.nodes[*child].name).starts_with("footer")
                        })
                        .map(|child| converter.convert(child, None))
                        .collect();
                    master_pages.push(OdfMasterPage {
                        name: attr(node, "name").map(str::to_string),
                        display_name: attr(node, "display-name").map(str::to_string),
                        page_layout_name: attr(node, "page-layout-name").map(str::to_string),
                        next_style_name: attr(node, "next-style-name").map(str::to_string),
                        headers,
                        footers,
                        locator: element_locator(entry, document, node),
                    });
                }
                _ => {}
            }
        }
    }
    styles.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.qualified_name.cmp(&right.qualified_name))
    });
    list_styles.sort_by(|left, right| left.name.cmp(&right.name));
    master_pages.sort_by(|left, right| left.name.cmp(&right.name));
    (styles, list_styles, master_pages)
}

struct Converter<'a> {
    entry: &'a PackageEntry,
    document: &'a XmlDocument,
}

impl<'a> Converter<'a> {
    fn new(entry: &'a PackageEntry, document: &'a XmlDocument) -> Self {
        Self { entry, document }
    }

    fn convert(&self, index: usize, inherited_change: Option<String>) -> OdfNode {
        let source = &self.document.nodes[index];
        let kind = classify(source);
        let own_change = change_id(source);
        let change = own_change.clone().or(inherited_change.clone());
        let mut active = change.clone();
        let mut content = Vec::new();
        for item in &source.content {
            match item {
                XmlContent::Text { value, start, end } => content.push(OdfContent::Text {
                    value: value.clone(),
                    locator: text_locator(self.entry, self.document, *start, *end),
                }),
                XmlContent::Child(child) => {
                    let child_source = &self.document.nodes[*child];
                    let child_kind = classify(child_source);
                    let child_change = change_id(child_source);
                    content.push(OdfContent::Element {
                        node: Box::new(self.convert(*child, active.clone())),
                    });
                    match child_kind {
                        OdfNodeKind::ChangeStart => active = child_change,
                        OdfNodeKind::ChangeEnd => {
                            if child_change.is_none() || active == child_change {
                                active = inherited_change.clone();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        OdfNode {
            id: format!("{}#{}", self.entry.path, source.path),
            kind: kind.clone(),
            qualified_name: source.name.clone(),
            attributes: source.attributes.clone(),
            style_name: attr(source, "style-name").map(str::to_string),
            change_id: change,
            content,
            raw_xml: matches!(kind, OdfNodeKind::Unknown | OdfNodeKind::Equation)
                .then(|| raw_xml(self.document, source).to_string()),
            locator: element_locator(self.entry, self.document, source),
        }
    }
}

fn classify(node: &XmlElement) -> OdfNodeKind {
    let local = local_name(&node.name);
    match local {
        "text" | "body" | "document-content" => OdfNodeKind::Document,
        "section" => OdfNodeKind::Section,
        "h" => OdfNodeKind::Heading,
        "p" => OdfNodeKind::Paragraph,
        "span" => OdfNodeKind::Span,
        "list" => OdfNodeKind::List,
        "list-item" | "list-header" => OdfNodeKind::ListItem,
        "table" => OdfNodeKind::Table,
        "table-row" => OdfNodeKind::TableRow,
        "table-cell" => OdfNodeKind::TableCell,
        "covered-table-cell" => OdfNodeKind::CoveredTableCell,
        "a" => OdfNodeKind::Link,
        value if value.starts_with("bookmark") => OdfNodeKind::Bookmark,
        value if value.ends_with("-ref") || value == "reference-mark" => OdfNodeKind::Reference,
        "sequence" | "variable-set" | "user-field-get" | "expression" => OdfNodeKind::Field,
        "note" => match attr(node, "note-class") {
            Some("endnote") => OdfNodeKind::Endnote,
            _ => OdfNodeKind::Footnote,
        },
        "note-citation" => OdfNodeKind::NoteCitation,
        "note-body" => OdfNodeKind::NoteBody,
        value if value.starts_with("header") => OdfNodeKind::Header,
        value if value.starts_with("footer") => OdfNodeKind::Footer,
        "annotation" => OdfNodeKind::Annotation,
        "annotation-end" => OdfNodeKind::AnnotationEnd,
        "tracked-changes" => OdfNodeKind::RevisionContainer,
        "changed-region" => OdfNodeKind::RevisionRegion,
        "change-start" => OdfNodeKind::ChangeStart,
        "change-end" => OdfNodeKind::ChangeEnd,
        "change" => OdfNodeKind::Change,
        "frame" => OdfNodeKind::Drawing,
        "image" => OdfNodeKind::Image,
        "text-box" => OdfNodeKind::TextBox,
        "object" | "object-ole" => OdfNodeKind::EmbeddedObject,
        "math" => OdfNodeKind::Equation,
        "s" => OdfNodeKind::Space,
        "tab" => OdfNodeKind::Tab,
        "line-break" => OdfNodeKind::LineBreak,
        "soft-page-break" => OdfNodeKind::PageBreak,
        _ if node.name.starts_with("draw:") => OdfNodeKind::Drawing,
        _ => OdfNodeKind::Unknown,
    }
}

fn change_id(node: &XmlElement) -> Option<String> {
    attr(node, "change-id")
        .or_else(|| attr(node, "id"))
        .map(str::to_string)
}

fn parse_revisions(entry: &PackageEntry, document: &XmlDocument) -> Vec<OdfRevision> {
    document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| local_name(&node.name) == "changed-region")
        .map(|(index, region)| {
            let change = child_indexes(region)
                .map(|child| (child, &document.nodes[child]))
                .find(|(_, node)| {
                    matches!(
                        local_name(&node.name),
                        "insertion" | "deletion" | "format-change"
                    )
                });
            let (change_index, change_node, kind) = change.map_or(
                (index, region, OdfRevisionKind::Unknown),
                |(child, node)| {
                    let kind = match local_name(&node.name) {
                        "insertion" => OdfRevisionKind::Insertion,
                        "deletion" => OdfRevisionKind::Deletion,
                        "format-change" => OdfRevisionKind::FormatChange,
                        _ => OdfRevisionKind::Unknown,
                    };
                    (child, node, kind)
                },
            );
            let converter = Converter::new(entry, document);
            let deleted_content = if kind == OdfRevisionKind::Deletion {
                document.nodes[change_index]
                    .content
                    .iter()
                    .filter_map(|content| match content {
                        XmlContent::Text { value, start, end } => Some(OdfContent::Text {
                            value: value.clone(),
                            locator: text_locator(entry, document, *start, *end),
                        }),
                        XmlContent::Child(child)
                            if local_name(&document.nodes[*child].name) != "change-info" =>
                        {
                            Some(OdfContent::Element {
                                node: Box::new(converter.convert(*child, None)),
                            })
                        }
                        _ => None,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let deleted_text = render_contents(&deleted_content, View::Visible, &HashMap::new());
            OdfRevision {
                id: attr(region, "id")
                    .or_else(|| attr(region, "change-id"))
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("change-{}", index + 1)),
                kind,
                creator: descendant_named_text(document, change_index, "creator"),
                date: descendant_named_text(document, change_index, "date"),
                comment: descendant_named_text(document, change_index, "p"),
                deleted_content,
                deleted_text,
                locator: element_locator(entry, document, change_node),
            }
        })
        .collect()
}

fn descendant_named_text(document: &XmlDocument, index: usize, name: &str) -> Option<String> {
    descendants(document, index)
        .find(|node| local_name(&node.name) == name)
        .and_then(|node| {
            document
                .nodes
                .iter()
                .position(|candidate| std::ptr::eq(candidate, node))
        })
        .map(|index| descendant_text(document, index).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn descendants(document: &XmlDocument, index: usize) -> impl Iterator<Item = &XmlElement> {
    let mut indexes = Vec::new();
    let mut stack = child_indexes(&document.nodes[index]).collect::<Vec<_>>();
    while let Some(child) = stack.pop() {
        indexes.push(child);
        stack.extend(child_indexes(&document.nodes[child]));
    }
    indexes.into_iter().map(|index| &document.nodes[index])
}

fn child_indexes(node: &XmlElement) -> impl Iterator<Item = usize> + '_ {
    node.content.iter().filter_map(|content| match content {
        XmlContent::Child(index) => Some(*index),
        XmlContent::Text { .. } => None,
    })
}

fn package_parts(entries: &[PackageEntry], manifest: &[OdfManifestEntry]) -> Vec<OdfPackagePart> {
    entries
        .iter()
        .map(|entry| {
            let media_type = media_type_for(manifest, &entry.path);
            let status = if entry.rejected.is_some() {
                OdfPartStatus::Rejected
            } else if entry.encrypted
                || manifest
                    .iter()
                    .any(|item| item.full_path == entry.path && item.encrypted)
            {
                OdfPartStatus::Encrypted
            } else if entry.kind == ArchiveEntryKind::Directory {
                OdfPartStatus::Directory
            } else {
                OdfPartStatus::Available
            };
            OdfPackagePart {
                package_index: entry.index,
                path: entry.path.clone(),
                media_type: media_type.clone(),
                compression: entry.compression.clone(),
                compressed_size: entry.compressed_size,
                uncompressed_size: entry.uncompressed_size,
                crc32: entry.crc32,
                status,
                rejection_code: entry.rejected.as_ref().map(|(code, _)| code.clone()),
                identity: entry.bytes.as_deref().map(|bytes| {
                    ContentIdentity::for_raw_bytes(bytes).with_format(FormatIdentity::new(
                        "odf_package_member",
                        media_type.clone(),
                    ))
                }),
                locator: member_locator(entry)
                    .unwrap_or_else(|_| super::fallback_member_locator(entry.index)),
            }
        })
        .collect()
}

fn build_artifacts(
    entries: &[PackageEntry],
    manifest: &[OdfManifestEntry],
    parent_identity: &ContentIdentity,
    options: &OdfWordOptions,
) -> Result<Vec<EmbeddedArtifact>, ParserError> {
    let mut artifacts = Vec::new();
    for entry in entries.iter().filter(|entry| {
        entry.bytes.is_some()
            && entry.rejected.is_none()
            && entry.kind == ArchiveEntryKind::RegularFile
            && !is_core_member(&entry.path)
    }) {
        let bytes = entry.bytes.as_deref().expect("filtered available bytes");
        let media_type = media_type_for(manifest, &entry.path);
        let mut metadata = ArtifactMetadata::new(
            ArtifactParent::new(parent_identity.clone(), ArtifactRelationship::EmbeddedIn),
            member_locator(entry).expect("validated artifact member"),
            ArtifactDisposition::PackagePart,
        )
        .with_declared_filename(entry.path.clone());
        if let Some(media_type) = &media_type {
            metadata = metadata.with_media_type(media_type.clone());
        }
        let artifact = if options.inline_embedded_artifact_bytes {
            EmbeddedArtifact::capture_inline(metadata, bytes)
        } else {
            EmbeddedArtifact::inventory(metadata, bytes)
        }
        .map_err(|error| {
            Box::new(Diagnostic::parser_defect(
                PARSER,
                format!("embedded-artifact invariant failed: {error}"),
            )) as ParserError
        })?;
        artifacts.push(artifact);
    }
    artifacts.sort_by(|left, right| left.declared_filename.cmp(&right.declared_filename));
    Ok(artifacts)
}

fn is_core_member(path: &str) -> bool {
    matches!(
        path,
        "mimetype" | CONTENT | STYLES | META | SETTINGS | MANIFEST
    )
}

fn media_type_for(manifest: &[OdfManifestEntry], path: &str) -> Option<String> {
    manifest
        .iter()
        .filter(|entry| {
            entry.full_path == path
                || (entry.full_path.ends_with('/') && path.starts_with(&entry.full_path))
        })
        .max_by_key(|entry| entry.full_path.len())
        .and_then(|entry| entry.media_type.clone())
}

fn artifact_path_map(artifacts: &[EmbeddedArtifact]) -> HashMap<String, String> {
    artifacts
        .iter()
        .filter_map(|artifact| {
            Some((
                artifact.declared_filename.clone()?,
                artifact.identity.artifact_id.clone(),
            ))
        })
        .collect()
}

#[derive(Default)]
struct Semantics {
    links: Vec<OdfLink>,
    notes: Vec<OdfNote>,
    annotations: Vec<OdfAnnotation>,
    drawings: Vec<OdfDrawing>,
    equations: Vec<OdfEquation>,
    embedded_objects: Vec<OdfEmbeddedObject>,
}

fn collect_semantics(
    node: &OdfNode,
    manifest: &[OdfManifestEntry],
    artifact_paths: &HashMap<String, String>,
    output: &mut Semantics,
) {
    let text = node_text(node);
    let href = model_attr(node, "href").map(str::to_string);
    let resolved = href
        .as_deref()
        .and_then(|href| resolve_member_path(CONTENT, href));
    match node.kind {
        OdfNodeKind::Link => {
            let href = href.unwrap_or_default();
            let fragment = href
                .split_once('#')
                .map(|(_, fragment)| fragment.to_string());
            output.links.push(OdfLink {
                source_node_id: node.id.clone(),
                external: is_external_href(&href),
                resolved_member: resolved,
                fragment,
                href,
                text,
                locator: node.locator.clone(),
            });
        }
        OdfNodeKind::Footnote | OdfNodeKind::Endnote => {
            output.notes.push(OdfNote {
                source_node_id: node.id.clone(),
                id: model_attr(node, "id").map(str::to_string),
                class: if node.kind == OdfNodeKind::Endnote {
                    "endnote".into()
                } else {
                    "footnote".into()
                },
                citation: child_text(node, OdfNodeKind::NoteCitation)
                    .trim()
                    .to_string(),
                text: child_text(node, OdfNodeKind::NoteBody).trim().to_string(),
                locator: node.locator.clone(),
            });
        }
        OdfNodeKind::Annotation => {
            output.annotations.push(OdfAnnotation {
                source_node_id: node.id.clone(),
                name: model_attr(node, "name").map(str::to_string),
                creator: descendant_model_text(node, "creator"),
                date: descendant_model_text(node, "date"),
                text,
                locator: node.locator.clone(),
            });
        }
        _ => collect_rich_semantics(node, href, resolved, text, manifest, artifact_paths, output),
    }
    for content in &node.content {
        if let OdfContent::Element { node } = content {
            collect_semantics(node, manifest, artifact_paths, output);
        }
    }
}

fn collect_rich_semantics(
    node: &OdfNode,
    href: Option<String>,
    resolved: Option<String>,
    text: String,
    manifest: &[OdfManifestEntry],
    artifact_paths: &HashMap<String, String>,
    output: &mut Semantics,
) {
    if matches!(
        node.kind,
        OdfNodeKind::Drawing
            | OdfNodeKind::Image
            | OdfNodeKind::TextBox
            | OdfNodeKind::EmbeddedObject
    ) {
        let kind = match local_name(&node.qualified_name) {
            "frame" => OdfDrawingKind::Frame,
            "image" => OdfDrawingKind::Image,
            "text-box" => OdfDrawingKind::TextBox,
            "object-ole" => OdfDrawingKind::OleObject,
            "object" => OdfDrawingKind::Object,
            _ => OdfDrawingKind::Shape,
        };
        let artifact_ids = artifact_ids_for(resolved.as_deref(), artifact_paths);
        let media_type = resolved
            .as_deref()
            .and_then(|path| media_type_for(manifest, path));
        output.drawings.push(OdfDrawing {
            source_node_id: node.id.clone(),
            kind,
            name: model_attr(node, "name").map(str::to_string),
            href: href.clone(),
            resolved_member: resolved.clone(),
            mime_type: media_type.clone(),
            alt_text: descendant_model_text(node, "desc")
                .or_else(|| descendant_model_text(node, "title")),
            text,
            geometry: node
                .attributes
                .iter()
                .filter(|(key, _)| {
                    matches!(
                        local_name(key),
                        "x" | "y" | "width" | "height" | "z-index" | "anchor-type"
                    )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            artifact_ids: artifact_ids.clone(),
            locator: node.locator.clone(),
        });
        if node.kind == OdfNodeKind::EmbeddedObject
            && let Some(href) = href
        {
            let resolved_members = resolved
                .as_deref()
                .map(|path| matching_artifact_paths(path, artifact_paths))
                .unwrap_or_default();
            output.embedded_objects.push(OdfEmbeddedObject {
                source_node_id: node.id.clone(),
                href,
                resolved_members,
                media_type,
                artifact_ids,
                locator: node.locator.clone(),
            });
        }
    } else if node.kind == OdfNodeKind::Equation {
        output.equations.push(OdfEquation {
            source_node_id: node.id.clone(),
            text,
            mathml: node.raw_xml.clone().unwrap_or_default(),
            embedded_member: node.id.split('#').next().unwrap_or(CONTENT).to_string(),
            locator: node.locator.clone(),
        });
    }
}

fn collect_embedded_equations(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
    equations: &mut Vec<OdfEquation>,
) {
    for entry in entries.iter().filter(|entry| {
        entry.path != CONTENT
            && entry.path.ends_with("/content.xml")
            && entry.bytes.is_some()
            && entry.rejected.is_none()
    }) {
        let Some(document) = parse_xml(entry, diagnostics) else {
            continue;
        };
        let converter = Converter::new(entry, &document);
        for (index, node) in document.nodes.iter().enumerate() {
            if local_name(&node.name) == "math" {
                let converted = converter.convert(index, None);
                equations.push(OdfEquation {
                    source_node_id: converted.id.clone(),
                    text: node_text(&converted),
                    mathml: converted.raw_xml.clone().unwrap_or_default(),
                    embedded_member: entry.path.clone(),
                    locator: converted.locator.clone(),
                });
            }
        }
    }
}

fn artifact_ids_for(
    resolved: Option<&str>,
    artifact_paths: &HashMap<String, String>,
) -> Vec<String> {
    resolved
        .map(|path| {
            matching_artifact_paths(path, artifact_paths)
                .iter()
                .filter_map(|path| artifact_paths.get(path).cloned())
                .collect()
        })
        .unwrap_or_default()
}

fn matching_artifact_paths(path: &str, artifact_paths: &HashMap<String, String>) -> Vec<String> {
    let prefix = format!("{}/", path.trim_end_matches('/'));
    let mut paths = artifact_paths
        .keys()
        .filter(|candidate| candidate.as_str() == path || candidate.starts_with(&prefix))
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn is_external_href(href: &str) -> bool {
    href.starts_with("//")
        || href
            .split_once(':')
            .is_some_and(|(scheme, _)| !scheme.contains('/') && !scheme.contains('#'))
}

fn resolve_member_path(base_member: &str, href: &str) -> Option<String> {
    if is_external_href(href) {
        return None;
    }
    let path = href.split('#').next().unwrap_or_default();
    if path.is_empty() {
        return Some(base_member.to_string());
    }
    if path.starts_with('/') || path.contains(char::from(92)) {
        return None;
    }
    let mut components = base_member.split('/').collect::<Vec<_>>();
    components.pop();
    for component in path.trim_start_matches("./").split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            value if value.contains(':') || value.chars().any(char::is_control) => return None,
            value => components.push(value),
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

fn model_attr<'a>(node: &'a OdfNode, name: &str) -> Option<&'a str> {
    node.attributes
        .iter()
        .find(|(key, _)| local_name(key) == name)
        .map(|(_, value)| value.as_str())
}

fn descendant_model_text(node: &OdfNode, name: &str) -> Option<String> {
    for content in &node.content {
        if let OdfContent::Element { node: child } = content {
            if local_name(&child.qualified_name) == name {
                let text = node_text(child);
                if !text.is_empty() {
                    return Some(text);
                }
            }
            if let Some(text) = descendant_model_text(child, name) {
                return Some(text);
            }
        }
    }
    None
}

fn child_text(node: &OdfNode, kind: OdfNodeKind) -> String {
    node.content
        .iter()
        .find_map(|content| match content {
            OdfContent::Element { node } if node.kind == kind => Some(node_text(node)),
            _ => None,
        })
        .unwrap_or_default()
}

pub(super) fn node_text(node: &OdfNode) -> String {
    render_node(node, View::Visible, &HashMap::new())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Visible,
    Original,
    Accepted,
    Rejected,
}

fn render_views(body: &OdfNode, revisions: &[OdfRevision]) -> OdfTextViews {
    let revisions = revisions
        .iter()
        .map(|revision| (revision.id.clone(), revision))
        .collect::<HashMap<_, _>>();
    OdfTextViews {
        visible: render_node(body, View::Visible, &revisions),
        original: render_node(body, View::Original, &revisions),
        accepted: render_node(body, View::Accepted, &revisions),
        rejected: render_node(body, View::Rejected, &revisions),
    }
}

fn validate_revision_graph(
    body: &OdfNode,
    revisions: &[OdfRevision],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut definitions = BTreeMap::<&str, Vec<&OdfRevision>>::new();
    for revision in revisions {
        definitions.entry(&revision.id).or_default().push(revision);
    }
    for (id, duplicates) in definitions.iter().filter(|(_, values)| values.len() > 1) {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                format!("duplicate tracked-change definition `{id}`"),
            )
            .with_locator(duplicates[1].locator.clone())
            .partial(),
        );
    }

    let mut markers = Vec::new();
    collect_revision_markers(body, &mut markers);
    let mut starts = BTreeMap::<&str, Vec<&OdfNode>>::new();
    let mut ends = BTreeMap::<&str, Vec<&OdfNode>>::new();
    for node in markers {
        let Some(id) = model_attr(node, "change-id") else {
            diagnostics.push(
                Diagnostic::malformed(PARSER, "tracked-change marker has no change-id")
                    .with_locator(node.locator.clone())
                    .partial(),
            );
            continue;
        };
        if !definitions.contains_key(id) {
            diagnostics.push(
                Diagnostic::malformed(
                    PARSER,
                    format!("tracked-change marker references unknown change `{id}`"),
                )
                .with_locator(node.locator.clone())
                .partial(),
            );
        }
        match node.kind {
            OdfNodeKind::ChangeStart => starts.entry(id).or_default().push(node),
            OdfNodeKind::ChangeEnd => ends.entry(id).or_default().push(node),
            _ => {}
        }
    }
    for id in starts
        .keys()
        .chain(ends.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
    {
        let start_count = starts.get(id).map_or(0, Vec::len);
        let end_count = ends.get(id).map_or(0, Vec::len);
        if start_count != end_count {
            let locator = starts
                .get(id)
                .and_then(|nodes| nodes.first())
                .or_else(|| ends.get(id).and_then(|nodes| nodes.first()))
                .expect("revision marker group is non-empty")
                .locator
                .clone();
            diagnostics.push(
                Diagnostic::malformed(
                    PARSER,
                    format!(
                        "tracked-change range `{id}` has {start_count} start marker(s) and {end_count} end marker(s)"
                    ),
                )
                .with_locator(locator)
                .partial(),
            );
        }
    }
}

fn collect_revision_markers<'a>(node: &'a OdfNode, output: &mut Vec<&'a OdfNode>) {
    if matches!(
        node.kind,
        OdfNodeKind::ChangeStart | OdfNodeKind::ChangeEnd | OdfNodeKind::Change
    ) {
        output.push(node);
    }
    for content in &node.content {
        if let OdfContent::Element { node } = content {
            collect_revision_markers(node, output);
        }
    }
}

fn render_node(node: &OdfNode, view: View, revisions: &HashMap<String, &OdfRevision>) -> String {
    if node.kind == OdfNodeKind::RevisionContainer {
        return String::new();
    }
    if let Some(change_id) = &node.change_id
        && let Some(revision) = revisions.get(change_id)
        && revision.kind == OdfRevisionKind::Insertion
        && matches!(view, View::Original | View::Rejected)
        && !matches!(
            node.kind,
            OdfNodeKind::ChangeStart | OdfNodeKind::ChangeEnd | OdfNodeKind::Change
        )
    {
        return String::new();
    }
    if matches!(node.kind, OdfNodeKind::Change | OdfNodeKind::ChangeStart)
        && matches!(view, View::Original | View::Rejected)
        && let Some(change_id) = model_attr(node, "change-id").or(node.change_id.as_deref())
        && let Some(revision) = revisions.get(change_id)
        && revision.kind == OdfRevisionKind::Deletion
    {
        return revision.deleted_text.clone();
    }
    let mut output = match node.kind {
        OdfNodeKind::Space => " ".repeat(
            model_attr(node, "c")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1),
        ),
        OdfNodeKind::Tab => "\t".into(),
        OdfNodeKind::LineBreak | OdfNodeKind::PageBreak => "\n".into(),
        OdfNodeKind::Change | OdfNodeKind::ChangeStart | OdfNodeKind::ChangeEnd => String::new(),
        _ => render_contents(&node.content, view, revisions),
    };
    match node.kind {
        OdfNodeKind::Paragraph
        | OdfNodeKind::Heading
        | OdfNodeKind::ListItem
        | OdfNodeKind::Section
        | OdfNodeKind::Header
        | OdfNodeKind::Footer => ensure_suffix(&mut output, '\n'),
        OdfNodeKind::TableCell | OdfNodeKind::CoveredTableCell => ensure_suffix(&mut output, '\t'),
        OdfNodeKind::TableRow => {
            while output.ends_with('\t') {
                output.pop();
            }
            ensure_suffix(&mut output, '\n');
        }
        _ => {}
    }
    if node.kind == OdfNodeKind::Document {
        normalize_text(output)
    } else {
        output
    }
}

fn render_contents(
    contents: &[OdfContent],
    view: View,
    revisions: &HashMap<String, &OdfRevision>,
) -> String {
    let mut output = String::new();
    for content in contents {
        match content {
            OdfContent::Text { value, .. } => output.push_str(value),
            OdfContent::Element { node } => {
                output.push_str(&render_node(node, view, revisions));
            }
        }
    }
    output
}

fn ensure_suffix(output: &mut String, suffix: char) {
    if !output.ends_with(suffix) {
        output.push(suffix);
    }
}

fn normalize_text(value: String) -> String {
    let lines = value.lines().map(str::trim_end).collect::<Vec<_>>();
    let mut output = lines.join("\n");
    while output.contains("\n\n\n") {
        output = output.replace("\n\n\n", "\n\n");
    }
    output.trim().to_string()
}

fn count_nodes(node: &OdfNode) -> usize {
    1usize.saturating_add(
        node.content
            .iter()
            .filter_map(|content| match content {
                OdfContent::Element { node } => Some(count_nodes(node)),
                OdfContent::Text { .. } => None,
            })
            .sum::<usize>(),
    )
}

fn empty_body(entry: &PackageEntry) -> OdfNode {
    OdfNode {
        id: format!("{}#/office:text[0]", entry.path),
        kind: OdfNodeKind::Document,
        qualified_name: "office:text".into(),
        attributes: BTreeMap::new(),
        style_name: None,
        change_id: None,
        content: Vec::new(),
        raw_xml: None,
        locator: member_locator(entry).expect("valid content member"),
    }
}
