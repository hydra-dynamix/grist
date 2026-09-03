//! OpenDocument presentation package interpretation.

use super::archive::{PackageEntry, find_entry};
use super::model::*;
use super::xml::{
    XmlContent, XmlDocument, XmlElement, attr, descendant_text, element_locator, local_name,
    parse_xml, raw_xml,
};
use super::{PARSER, member_locator};
use crate::container::{
    ArtifactDisposition, ArtifactMetadata, ArtifactParent, ArtifactRelationship, EmbeddedArtifact,
};
use crate::core::{
    BoundingBox, ContentIdentity, CoordinateOrigin, CoordinateUnit, Diagnostic, FormatIdentity,
    IndexBase, IndexPosition, LocationComponent, OperationStatus, SchemaVersion,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};
use crate::security::ArchiveEntryKind;
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

const CONTENT: &str = "content.xml";
const STYLES: &str = "styles.xml";
const META: &str = "meta.xml";
const SETTINGS: &str = "settings.xml";
const MANIFEST: &str = "META-INF/manifest.xml";

#[derive(Default)]
struct SlideSemantics {
    tables: Vec<OdfPresentationTable>,
    charts: Vec<OdfPresentationChart>,
    images: Vec<OdfPresentationImage>,
    links: Vec<OdfPresentationLink>,
    objects: Vec<OdfPresentationEmbeddedObject>,
}

pub(super) fn parse_registered(
    context: &mut ParserContext<'_>,
    expected_kind: OdfPresentationPackageKind,
) -> Result<ParserOutput, ParserError> {
    let options: OdfPresentationOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(PARSER, error.to_string())))?;
    let mut package = super::archive::read_package(context.bytes(), context)?;
    let mut diagnostics = std::mem::take(&mut package.diagnostics);
    super::xml::observe_xml_nesting(&package.entries, context)?;
    if package
        .entries
        .iter()
        .any(|entry| entry.encrypted && matches!(entry.path.as_str(), "mimetype" | CONTENT))
    {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![Diagnostic::error(
                PARSER,
                "presentation_odf.encrypted.core_member",
                "the OpenDocument presentation core is encrypted",
            )],
        ));
    }
    validate_mimetype(&package.entries, expected_kind)?;
    let manifest = parse_manifest(&package.entries, &mut diagnostics);
    if let Some(entry) = manifest
        .iter()
        .find(|entry| entry.full_path == CONTENT && entry.encrypted)
    {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![
                Diagnostic::error(
                    PARSER,
                    "presentation_odf.encrypted.content",
                    "manifest declares content.xml as encrypted",
                )
                .with_locator(entry.locator.clone()),
            ],
        ));
    }
    for entry in manifest
        .iter()
        .filter(|entry| entry.encrypted && entry.full_path != CONTENT)
    {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "presentation_odf.encrypted.member",
                format!(
                    "encrypted member {} is inventoried but unavailable",
                    entry.full_path
                ),
            )
            .with_locator(entry.locator.clone())
            .partial(),
        );
    }

    let content_entry = find_entry(&package.entries, CONTENT).ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "OpenDocument presentation has no safe content.xml member",
        )) as ParserError
    })?;
    let content_xml = parse_xml(content_entry, &mut diagnostics);
    let version = content_xml.as_ref().and_then(|document| {
        document
            .nodes
            .first()
            .and_then(|node| attr(node, "version"))
            .map(str::to_string)
    });
    let styles_xml =
        find_entry(&package.entries, STYLES).and_then(|entry| parse_xml(entry, &mut diagnostics));
    let metadata = parse_properties(&package.entries, META, "meta", &mut diagnostics);
    let settings = parse_properties(&package.entries, SETTINGS, "settings", &mut diagnostics);
    let (styles, page_layouts) = parse_styles(
        &package.entries,
        content_xml.as_ref(),
        styles_xml.as_ref(),
        &mut diagnostics,
    );
    let master_pages = styles_xml.as_ref().map_or_else(Vec::new, |document| {
        let entry = find_entry(&package.entries, STYLES).expect("parsed styles entry exists");
        parse_master_pages(entry, document)
    });
    let parent_identity = ContentIdentity::for_raw_bytes(context.bytes()).with_format(
        FormatIdentity::new(expected_kind.format_id(), Some(expected_kind.media_type())),
    );
    let artifacts = build_artifacts(&package.entries, &manifest, &parent_identity, &options)?;
    context.consume_child_artifacts(artifacts.len() as u64)?;
    let artifact_map = artifact_map(&artifacts);
    let slides = content_xml.as_ref().map_or_else(Vec::new, |document| {
        parse_slides(
            content_entry,
            document,
            &package.entries,
            &manifest,
            &artifact_map,
            &mut diagnostics,
        )
    });
    let parts = package_parts(&package.entries, &manifest);
    let node_count = parts.len()
        + manifest.len()
        + metadata.len()
        + settings.len()
        + styles.len()
        + page_layouts.len()
        + master_pages.len()
        + slides.iter().map(slide_node_count).sum::<usize>()
        + artifacts.len();
    context.consume_nodes(node_count as u64)?;
    let raw_elements = content_xml.as_ref().map_or_else(Vec::new, |document| {
        document.nodes.first().map_or_else(Vec::new, |root| {
            child_indexes(root)
                .filter(|index| {
                    !matches!(
                        local_name(&document.nodes[*index].name),
                        "automatic-styles" | "body" | "font-face-decls" | "scripts"
                    )
                })
                .map(|index| metadata_node(content_entry, document, index))
                .collect()
        })
    });
    let document = OdfPresentationDocument {
        schema_version: SchemaVersion::PRESENTATION_ODF_V1.into(),
        package_kind: expected_kind,
        package_media_type: expected_kind.media_type().into(),
        version,
        parts,
        manifest,
        metadata,
        settings,
        styles,
        page_layouts,
        master_pages,
        slides,
        embedded_artifacts: artifacts,
        raw_elements,
    };
    let value = serde_json::to_value(document)
        .map_err(|error| Box::new(Diagnostic::parser_defect(PARSER, error.to_string())))?;
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
    expected: OdfPresentationPackageKind,
) -> Result<(), ParserError> {
    let entry = find_entry(entries, "mimetype").ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "package has no safe mimetype member",
        )) as ParserError
    })?;
    let actual = entry
        .bytes
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(str::trim)
        .unwrap_or_default();
    if actual != expected.media_type() {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            format!(
                "requested {} but mimetype declares {actual:?}",
                expected.format_id()
            ),
        )));
    }
    Ok(())
}

fn parse_manifest(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<OdfPresentationManifestEntry> {
    let Some(entry) = find_entry(entries, MANIFEST) else {
        diagnostics.push(Diagnostic::malformed(PARSER, "package has no manifest.xml").partial());
        return Vec::new();
    };
    let Some(document) = parse_xml(entry, diagnostics) else {
        return Vec::new();
    };
    document
        .nodes
        .iter()
        .filter(|node| local_name(&node.name) == "file-entry")
        .map(|node| OdfPresentationManifestEntry {
            full_path: attr(node, "full-path").unwrap_or_default().to_string(),
            media_type: attr(node, "media-type")
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            version: attr(node, "version").map(str::to_string),
            size: attr(node, "size").and_then(|value| value.parse().ok()),
            encrypted: descendants(&document, node_index(&document, node))
                .any(|child| local_name(&child.name) == "encryption-data"),
            checksum: attr(node, "checksum").map(str::to_string),
            checksum_type: attr(node, "checksum-type").map(str::to_string),
            attributes: node.attributes.clone(),
            locator: element_locator(entry, &document, node),
        })
        .collect()
}

fn parse_properties(
    entries: &[PackageEntry],
    path: &str,
    container: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<OdfPresentationProperty> {
    let Some(entry) = find_entry(entries, path) else {
        return Vec::new();
    };
    let Some(document) = parse_xml(entry, diagnostics) else {
        return Vec::new();
    };
    let Some(root) = document
        .nodes
        .iter()
        .position(|node| local_name(&node.name) == container)
    else {
        return Vec::new();
    };
    child_indexes(&document.nodes[root])
        .map(|index| {
            let node = &document.nodes[index];
            OdfPresentationProperty {
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
    content: Option<&XmlDocument>,
    styles_document: Option<&XmlDocument>,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<OdfPresentationStyle>, Vec<OdfPresentationPageLayout>) {
    let mut styles = Vec::new();
    let mut layouts = Vec::new();
    for (entry, document) in [
        find_entry(entries, CONTENT).zip(content),
        find_entry(entries, STYLES).zip(styles_document),
    ]
    .into_iter()
    .flatten()
    {
        for (index, node) in document.nodes.iter().enumerate() {
            match local_name(&node.name) {
                "style" | "default-style" => {
                    let mut properties = BTreeMap::new();
                    for child in child_indexes(node) {
                        let property = &document.nodes[child];
                        if local_name(&property.name).ends_with("properties") {
                            properties.insert(property.name.clone(), property.attributes.clone());
                        }
                    }
                    styles.push(OdfPresentationStyle {
                        qualified_name: node.name.clone(),
                        name: attr(node, "name").map(str::to_string),
                        display_name: attr(node, "display-name").map(str::to_string),
                        family: attr(node, "family").map(str::to_string),
                        parent_style_name: attr(node, "parent-style-name").map(str::to_string),
                        page_layout_name: attr(node, "page-layout-name").map(str::to_string),
                        presentation_page_layout_name: attr(node, "presentation-page-layout-name")
                            .map(str::to_string),
                        attributes: node.attributes.clone(),
                        properties,
                        locator: element_locator(entry, document, node),
                    });
                }
                "page-layout" | "presentation-page-layout" => {
                    let properties = descendants(document, index)
                        .find(|item| local_name(&item.name) == "page-layout-properties");
                    let source = properties.unwrap_or(node);
                    let placeholders = descendants(document, index)
                        .filter(|item| local_name(&item.name) == "placeholder")
                        .map(|item| OdfPresentationPlaceholder {
                            presentation_class: attr(item, "class").map(str::to_string),
                            object: attr(item, "object").map(str::to_string),
                            geometry: geometry(item),
                            attributes: item.attributes.clone(),
                            locator: element_locator(entry, document, item),
                        })
                        .collect();
                    layouts.push(OdfPresentationPageLayout {
                        name: attr(node, "name").map(str::to_string),
                        page_width: attr(source, "page-width").map(str::to_string),
                        page_height: attr(source, "page-height").map(str::to_string),
                        orientation: attr(source, "print-orientation").map(str::to_string),
                        print_orientation: attr(source, "print-page-order").map(str::to_string),
                        attributes: node.attributes.clone(),
                        placeholders,
                        locator: element_locator(entry, document, node),
                    });
                }
                _ => {}
            }
        }
    }
    if styles.is_empty() {
        diagnostics.push(Diagnostic::warning(
            PARSER,
            "presentation_odf.styles.empty",
            "presentation contains no resolved styles",
        ));
    }
    (styles, layouts)
}

fn parse_master_pages(
    entry: &PackageEntry,
    document: &XmlDocument,
) -> Vec<OdfPresentationMasterPage> {
    document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| local_name(&node.name) == "master-page")
        .map(|(index, node)| {
            let mut z = 0;
            let mut shapes = Vec::new();
            collect_shapes(
                entry,
                document,
                index,
                0,
                "master",
                None,
                &mut z,
                &mut shapes,
            );
            OdfPresentationMasterPage {
                name: attr(node, "name").map(str::to_string),
                display_name: attr(node, "display-name").map(str::to_string),
                page_layout_name: attr(node, "page-layout-name").map(str::to_string),
                presentation_page_layout_name: attr(node, "presentation-page-layout-name")
                    .map(str::to_string),
                style_name: attr(node, "style-name").map(str::to_string),
                shapes,
                raw_elements: child_indexes(node)
                    .filter(|child| !is_shape(&document.nodes[*child]))
                    .map(|child| metadata_node(entry, document, child))
                    .collect(),
                locator: element_locator(entry, document, node),
            }
        })
        .collect()
}

fn package_parts(
    entries: &[PackageEntry],
    manifest: &[OdfPresentationManifestEntry],
) -> Vec<OdfPresentationPart> {
    entries
        .iter()
        .map(|entry| OdfPresentationPart {
            package_index: entry.index,
            path: entry.path.clone(),
            media_type: media_type_for(manifest, &entry.path),
            compression: entry.compression.clone(),
            compressed_size: entry.compressed_size,
            uncompressed_size: entry.uncompressed_size,
            crc32: entry.crc32,
            status: if entry.rejected.is_some() {
                OdfPresentationPartStatus::Rejected
            } else if entry.kind == ArchiveEntryKind::Directory {
                OdfPresentationPartStatus::Directory
            } else if entry.encrypted {
                OdfPresentationPartStatus::Encrypted
            } else {
                OdfPresentationPartStatus::Available
            },
            rejection_code: entry.rejected.as_ref().map(|value| value.0.clone()),
            identity: entry.bytes.as_deref().map(ContentIdentity::for_raw_bytes),
            locator: member_locator(entry)
                .unwrap_or_else(|_| super::fallback_member_locator(entry.index)),
        })
        .collect()
}

fn build_artifacts(
    entries: &[PackageEntry],
    manifest: &[OdfPresentationManifestEntry],
    parent: &ContentIdentity,
    options: &OdfPresentationOptions,
) -> Result<Vec<EmbeddedArtifact>, ParserError> {
    let mut artifacts = Vec::new();
    for entry in entries.iter().filter(|entry| {
        entry.bytes.is_some() && !is_core_member(&entry.path) && !entry.path.ends_with('/')
    }) {
        let bytes = entry.bytes.as_deref().expect("filtered bytes");
        let media_type = media_type_for(manifest, &entry.path);
        let metadata = ArtifactMetadata::new(
            ArtifactParent::new(parent.clone(), ArtifactRelationship::EmbeddedIn),
            member_locator(entry).expect("safe member"),
            ArtifactDisposition::Inline,
        )
        .with_declared_filename(&entry.path)
        .with_media_type(
            media_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".into()),
        );
        let artifact = if options.inline_embedded_artifact_bytes {
            EmbeddedArtifact::capture_inline(metadata, bytes)
        } else {
            EmbeddedArtifact::inventory(metadata, bytes)
        }
        .map_err(|error| {
            Box::new(Diagnostic::parser_defect(PARSER, error.to_string())) as ParserError
        })?;
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

fn is_core_member(path: &str) -> bool {
    matches!(
        path,
        "mimetype" | CONTENT | STYLES | META | SETTINGS | MANIFEST
    )
}

fn artifact_map(artifacts: &[EmbeddedArtifact]) -> HashMap<String, String> {
    artifacts
        .iter()
        .filter_map(|artifact| {
            artifact
                .declared_filename
                .as_ref()
                .map(|path| (path.clone(), artifact.identity.artifact_id.clone()))
        })
        .collect()
}

fn parse_slides(
    entry: &PackageEntry,
    document: &XmlDocument,
    entries: &[PackageEntry],
    manifest: &[OdfPresentationManifestEntry],
    artifacts: &HashMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<OdfPresentationSlide> {
    let pages = document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.name.ends_with("draw:page") || node.name == "draw:page")
        .collect::<Vec<_>>();
    if pages.is_empty() {
        diagnostics.push(
            Diagnostic::malformed(PARSER, "content.xml has no draw:page slides")
                .with_locator(member_locator(entry).expect("safe member"))
                .partial(),
        );
    }
    pages
        .into_iter()
        .enumerate()
        .map(|(order, (page_index, page))| {
            let slide_id = attr(page, "id")
                .or_else(|| attr(page, "name"))
                .map(str::to_string)
                .unwrap_or_else(|| format!("slide-{}", order + 1));
            let mut shapes = Vec::new();
            let mut z = 0;
            collect_shapes(
                entry,
                document,
                page_index,
                order + 1,
                &slide_id,
                None,
                &mut z,
                &mut shapes,
            );
            let notes = child_indexes(page)
                .filter(|index| local_name(&document.nodes[*index].name) == "notes")
                .map(|index| {
                    let node = &document.nodes[index];
                    let mut note_shapes = Vec::new();
                    let mut note_z = 0;
                    collect_shapes(
                        entry,
                        document,
                        index,
                        order + 1,
                        &slide_id,
                        None,
                        &mut note_z,
                        &mut note_shapes,
                    );
                    OdfPresentationNote {
                        text: descendant_text(document, index).trim().to_string(),
                        shapes: note_shapes,
                        locator: element_locator(entry, document, node),
                    }
                })
                .collect();
            let comments = descendants(document, page_index)
                .filter(|node| local_name(&node.name) == "annotation")
                .map(|node| {
                    let index = node_index(document, node);
                    OdfPresentationComment {
                        name: attr(node, "name").map(str::to_string),
                        creator: descendant_named_text(document, index, "creator"),
                        created_at: descendant_named_text(document, index, "date"),
                        text: descendant_text(document, index).trim().to_string(),
                        attributes: node.attributes.clone(),
                        locator: element_locator(entry, document, node),
                    }
                })
                .collect();
            let mut semantics = SlideSemantics::default();
            for shape in &shapes {
                collect_shape_semantics(
                    entry,
                    document,
                    page_index,
                    shape,
                    entries,
                    manifest,
                    artifacts,
                    &mut semantics,
                );
            }
            let transition = parse_transition(entry, document, page_index);
            let animations = parse_animations(entry, document, page_index);
            let reading_order = reading_order(&shapes);
            let known = |node: &XmlElement| {
                is_shape(node)
                    || matches!(
                        local_name(&node.name),
                        "notes" | "annotation" | "animations" | "transition"
                    )
            };
            OdfPresentationSlide {
                order: order + 1,
                slide_id,
                name: attr(page, "name").map(str::to_string),
                master_page_name: attr(page, "master-page-name").map(str::to_string),
                style_name: attr(page, "style-name").map(str::to_string),
                presentation_page_layout_name: attr(page, "presentation-page-layout-name")
                    .map(str::to_string),
                visible: attr(page, "visibility") != Some("hidden"),
                attributes: page.attributes.clone(),
                shapes,
                notes,
                comments,
                tables: semantics.tables,
                charts: semantics.charts,
                images: semantics.images,
                links: semantics.links,
                transition,
                animations,
                embedded_objects: semantics.objects,
                reading_order,
                raw_elements: child_indexes(page)
                    .filter(|index| !known(&document.nodes[*index]))
                    .map(|index| metadata_node(entry, document, index))
                    .collect(),
                locator: slide_locator(entry, document, page, order + 1, None, None),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn collect_shapes(
    entry: &PackageEntry,
    document: &XmlDocument,
    root: usize,
    slide: usize,
    slide_id: &str,
    parent: Option<&str>,
    z: &mut usize,
    output: &mut Vec<OdfPresentationShape>,
) {
    for child in child_indexes(&document.nodes[root]) {
        let node = &document.nodes[child];
        if local_name(&node.name) == "notes" {
            continue;
        }
        if is_shape(node) {
            let current = *z;
            *z += 1;
            let shape_id = attr(node, "id")
                .or_else(|| attr(node, "name"))
                .map(str::to_string)
                .unwrap_or_else(|| format!("{slide_id}-shape-{}", current + 1));
            let geometry = geometry(node);
            let locator = if slide == 0 {
                element_locator(entry, document, node)
            } else {
                slide_locator(
                    entry,
                    document,
                    node,
                    slide,
                    Some(shape_id.clone()),
                    geometry.bbox_points,
                )
            };
            output.push(OdfPresentationShape {
                shape_id: shape_id.clone(),
                qualified_name: node.name.clone(),
                kind: shape_kind(node),
                parent_shape_id: parent.map(str::to_string),
                z_order: current,
                name: attr(node, "name").map(str::to_string),
                presentation_class: attr(node, "class").map(str::to_string),
                style_name: attr(node, "style-name").map(str::to_string),
                text_style_name: attr(node, "text-style-name").map(str::to_string),
                layer: attr(node, "layer").map(str::to_string),
                geometry,
                text: parse_text_body(entry, document, child),
                alt_title: descendant_named_text(document, child, "title"),
                alt_description: descendant_named_text(document, child, "desc"),
                attributes: node.attributes.clone(),
                raw_xml: raw_xml(document, node).to_string(),
                locator,
            });
            collect_shapes(
                entry,
                document,
                child,
                slide,
                slide_id,
                Some(&shape_id),
                z,
                output,
            );
        }
    }
}

fn parse_text_body(
    entry: &PackageEntry,
    document: &XmlDocument,
    root: usize,
) -> Option<OdfPresentationTextBody> {
    let paragraphs = descendants(document, root)
        .filter(|node| matches!(local_name(&node.name), "p" | "h"))
        .enumerate()
        .map(|(index, node)| {
            let node_index = node_index(document, node);
            let mut runs = Vec::new();
            collect_text_runs(entry, document, node_index, &mut runs);
            OdfPresentationTextParagraph {
                index,
                kind: local_name(&node.name).to_string(),
                level: attr(node, "outline-level").and_then(|value| value.parse().ok()),
                style_name: attr(node, "style-name").map(str::to_string),
                text: descendant_text(document, node_index),
                runs,
                attributes: node.attributes.clone(),
                locator: element_locator(entry, document, node),
            }
        })
        .collect::<Vec<_>>();
    if paragraphs.is_empty() {
        return None;
    }
    let text = paragraphs
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some(OdfPresentationTextBody {
        locator: element_locator(entry, document, &document.nodes[root]),
        paragraphs,
        text,
    })
}

fn collect_text_runs(
    entry: &PackageEntry,
    document: &XmlDocument,
    root: usize,
    output: &mut Vec<OdfPresentationTextRun>,
) {
    for content in &document.nodes[root].content {
        match content {
            XmlContent::Text { value, start, end } if !value.is_empty() => {
                output.push(OdfPresentationTextRun {
                    index: output.len(),
                    kind: "text".into(),
                    text: value.clone(),
                    style_name: None,
                    href: None,
                    attributes: BTreeMap::new(),
                    locator: super::xml::text_locator(entry, document, *start, *end),
                });
            }
            XmlContent::Child(index) => {
                let node = &document.nodes[*index];
                let name = local_name(&node.name);
                if matches!(name, "span" | "a" | "s" | "tab" | "line-break") {
                    let text = match name {
                        "s" => " ".repeat(
                            attr(node, "c")
                                .and_then(|value| value.parse().ok())
                                .unwrap_or(1),
                        ),
                        "tab" => "\t".into(),
                        "line-break" => "\n".into(),
                        _ => descendant_text(document, *index),
                    };
                    output.push(OdfPresentationTextRun {
                        index: output.len(),
                        kind: name.to_string(),
                        text,
                        style_name: attr(node, "style-name").map(str::to_string),
                        href: attr(node, "href").map(str::to_string),
                        attributes: node.attributes.clone(),
                        locator: element_locator(entry, document, node),
                    });
                } else {
                    collect_text_runs(entry, document, *index, output);
                }
            }
            XmlContent::Text { .. } => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_shape_semantics(
    entry: &PackageEntry,
    document: &XmlDocument,
    page: usize,
    shape: &OdfPresentationShape,
    entries: &[PackageEntry],
    manifest: &[OdfPresentationManifestEntry],
    artifacts: &HashMap<String, String>,
    output: &mut SlideSemantics,
) {
    let Some(root) = find_shape_node(document, page, shape) else {
        return;
    };
    for node in descendants(document, root) {
        let index = node_index(document, node);
        match local_name(&node.name) {
            "table" if node.name.contains("table:") => {
                output
                    .tables
                    .push(parse_table(entry, document, index, &shape.shape_id));
            }
            "image" if node.name.contains("draw:") => {
                let href = attr(node, "href").unwrap_or_default().to_string();
                let resolved = resolve_member_path(CONTENT, &href);
                output.images.push(OdfPresentationImage {
                    shape_id: shape.shape_id.clone(),
                    href,
                    resolved_member: resolved.clone(),
                    media_type: resolved
                        .as_deref()
                        .and_then(|path| media_type_for(manifest, path)),
                    identity: resolved
                        .as_deref()
                        .and_then(|path| find_entry(entries, path))
                        .and_then(|entry| entry.bytes.as_deref())
                        .map(ContentIdentity::for_raw_bytes),
                    alt_title: shape.alt_title.clone(),
                    alt_description: shape.alt_description.clone(),
                    artifact_ids: resolved
                        .as_ref()
                        .and_then(|path| artifacts.get(path))
                        .cloned()
                        .into_iter()
                        .collect(),
                    attributes: node.attributes.clone(),
                    locator: element_locator(entry, document, node),
                });
            }
            "object" | "object-ole" | "plugin" | "applet" | "floating-frame" => {
                let href = attr(node, "href").map(str::to_string);
                let members = href
                    .as_deref()
                    .map(|href| matching_members(CONTENT, href, entries))
                    .unwrap_or_default();
                let media_type = members
                    .first()
                    .and_then(|path| media_type_for(manifest, path));
                let artifact_ids = members
                    .iter()
                    .filter_map(|path| artifacts.get(path).cloned())
                    .collect();
                if local_name(&node.name) == "object" {
                    if let Some(chart) =
                        parse_chart(entry, document, node, &shape.shape_id, entries, &members)
                    {
                        output.charts.push(chart);
                    }
                }
                output.objects.push(OdfPresentationEmbeddedObject {
                    shape_id: shape.shape_id.clone(),
                    kind: local_name(&node.name).to_string(),
                    href,
                    resolved_members: members,
                    media_type,
                    artifact_ids,
                    attributes: node.attributes.clone(),
                    locator: element_locator(entry, document, node),
                });
            }
            "a" => {
                let href = attr(node, "href").unwrap_or_default().to_string();
                output.links.push(OdfPresentationLink {
                    source_shape_id: Some(shape.shape_id.clone()),
                    source_run: None,
                    resolved_member: resolve_member_path(CONTENT, &href),
                    external: is_external_href(&href),
                    action: attr(node, "action").map(str::to_string),
                    show: attr(node, "show").map(str::to_string),
                    text: descendant_text(document, index),
                    href,
                    attributes: node.attributes.clone(),
                    locator: element_locator(entry, document, node),
                });
            }
            _ => {}
        }
    }
}

fn parse_table(
    entry: &PackageEntry,
    document: &XmlDocument,
    root: usize,
    shape_id: &str,
) -> OdfPresentationTable {
    let node = &document.nodes[root];
    let columns = child_indexes(node)
        .filter(|index| local_name(&document.nodes[*index].name) == "table-column")
        .enumerate()
        .map(|(index, child)| {
            let column = &document.nodes[child];
            OdfPresentationTableColumn {
                index,
                repeated: attr(column, "number-columns-repeated")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1),
                style_name: attr(column, "style-name").map(str::to_string),
                locator: element_locator(entry, document, column),
            }
        })
        .collect();
    let mut row_number = 0;
    let rows = child_indexes(node)
        .filter(|index| local_name(&document.nodes[*index].name) == "table-row")
        .map(|child| {
            let row = &document.nodes[child];
            let repeated = attr(row, "number-rows-repeated")
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            let mut column = 0;
            let cells = child_indexes(row)
                .filter_map(|cell_index| {
                    let cell = &document.nodes[cell_index];
                    let covered = local_name(&cell.name) == "covered-table-cell";
                    if !covered && local_name(&cell.name) != "table-cell" {
                        return None;
                    }
                    let repeated = attr(cell, "number-columns-repeated")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(1);
                    let result = OdfPresentationTableCell {
                        row: row_number,
                        column,
                        repeated,
                        column_span: attr(cell, "number-columns-spanned")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1),
                        row_span: attr(cell, "number-rows-spanned")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(1),
                        covered,
                        value_type: attr(cell, "value-type").map(str::to_string),
                        value: attr(cell, "value")
                            .or_else(|| attr(cell, "string-value"))
                            .map(str::to_string),
                        formula: attr(cell, "formula").map(str::to_string),
                        text: descendant_text(document, cell_index),
                        attributes: cell.attributes.clone(),
                        locator: element_locator(entry, document, cell),
                    };
                    column += repeated;
                    Some(result)
                })
                .collect();
            let result = OdfPresentationTableRow {
                index: row_number,
                repeated,
                cells,
                locator: element_locator(entry, document, row),
            };
            row_number += repeated;
            result
        })
        .collect();
    OdfPresentationTable {
        shape_id: shape_id.to_string(),
        name: attr(node, "name").map(str::to_string),
        columns,
        rows,
        style_name: attr(node, "style-name").map(str::to_string),
        locator: element_locator(entry, document, node),
    }
}

fn parse_chart(
    source_entry: &PackageEntry,
    source_document: &XmlDocument,
    object: &XmlElement,
    shape_id: &str,
    entries: &[PackageEntry],
    members: &[String],
) -> Option<OdfPresentationChart> {
    let href = attr(object, "href")?.to_string();
    let chart_path = members.iter().find(|path| path.ends_with("content.xml"))?;
    let chart_entry = find_entry(entries, chart_path)?;
    let mut ignored = Vec::new();
    let chart = parse_xml(chart_entry, &mut ignored)?;
    let root = chart
        .nodes
        .iter()
        .position(|node| local_name(&node.name) == "chart")?;
    let chart_node = &chart.nodes[root];
    let title = descendants(&chart, root)
        .find(|node| local_name(&node.name) == "title")
        .map(|node| {
            descendant_text(&chart, node_index(&chart, node))
                .trim()
                .to_string()
        });
    let series = descendants(&chart, root)
        .filter(|node| local_name(&node.name) == "series")
        .enumerate()
        .map(|(index, node)| OdfPresentationChartSeries {
            index,
            values_range: attr(node, "values-cell-range-address").map(str::to_string),
            label_cell: attr(node, "label-cell-address").map(str::to_string),
            class: attr(node, "class").map(str::to_string),
            categories_range: descendants(&chart, node_index(&chart, node))
                .find(|child| local_name(&child.name) == "categories")
                .and_then(|child| attr(child, "cell-range-address"))
                .map(str::to_string),
            attributes: node.attributes.clone(),
            locator: element_locator(chart_entry, &chart, node),
        })
        .collect();
    Some(OdfPresentationChart {
        shape_id: shape_id.to_string(),
        href,
        resolved_members: members.to_vec(),
        class: attr(chart_node, "class").map(str::to_string),
        title,
        series,
        attributes: object.attributes.clone(),
        locator: element_locator(source_entry, source_document, object),
    })
}

fn parse_transition(
    entry: &PackageEntry,
    document: &XmlDocument,
    page: usize,
) -> Option<OdfPresentationTransition> {
    let page_node = &document.nodes[page];
    let transition =
        descendants(document, page).find(|node| local_name(&node.name).contains("transition"));
    if transition.is_none()
        && !page_node.attributes.keys().any(|key| {
            let local = local_name(key);
            local.starts_with("transition-") || local.starts_with("duration")
        })
    {
        return None;
    }
    let source = transition.unwrap_or(page_node);
    Some(OdfPresentationTransition {
        style: attr(source, "transition-style").map(str::to_string),
        type_name: attr(source, "type").map(str::to_string),
        subtype: attr(source, "subtype").map(str::to_string),
        direction: attr(source, "direction").map(str::to_string),
        duration: attr(source, "dur")
            .or_else(|| attr(source, "duration"))
            .map(str::to_string),
        speed: attr(source, "transition-speed").map(str::to_string),
        advance_on_click: attr(source, "transition-on-click").and_then(parse_bool),
        advance_after: attr(source, "duration").map(str::to_string),
        attributes: source.attributes.clone(),
        locator: element_locator(entry, document, source),
    })
}

fn parse_animations(
    entry: &PackageEntry,
    document: &XmlDocument,
    page: usize,
) -> Vec<OdfPresentationAnimation> {
    descendants(document, page)
        .filter(|node| is_animation(node))
        .enumerate()
        .map(|(index, node)| OdfPresentationAnimation {
            index,
            qualified_name: node.name.clone(),
            target_element: attr(node, "targetElement")
                .or_else(|| attr(node, "target-element"))
                .map(str::to_string),
            begin: attr(node, "begin").map(str::to_string),
            duration: attr(node, "dur").map(str::to_string),
            attribute_name: attr(node, "attributeName").map(str::to_string),
            values: attr(node, "values").map(str::to_string),
            attributes: node.attributes.clone(),
            text: descendant_text(document, node_index(document, node)),
            raw_xml: raw_xml(document, node).to_string(),
            locator: element_locator(entry, document, node),
        })
        .collect()
}

fn is_animation(node: &XmlElement) -> bool {
    node.name.starts_with("anim:")
        || matches!(
            local_name(&node.name),
            "animate"
                | "animateColor"
                | "animateMotion"
                | "animateTransform"
                | "set"
                | "par"
                | "seq"
                | "audio"
                | "command"
        )
}

fn geometry(node: &XmlElement) -> OdfPresentationGeometry {
    let x = attr(node, "x").map(str::to_string);
    let y = attr(node, "y").map(str::to_string);
    let width = attr(node, "width").map(str::to_string);
    let height = attr(node, "height").map(str::to_string);
    let bbox_points = match (
        x.as_deref().and_then(measurement_points),
        y.as_deref().and_then(measurement_points),
        width.as_deref().and_then(measurement_points),
        height.as_deref().and_then(measurement_points),
    ) {
        (Some(x), Some(y), Some(width), Some(height)) if width >= 0.0 && height >= 0.0 => {
            Some(BoundingBox {
                x,
                y,
                width,
                height,
                unit: CoordinateUnit::Points,
                origin: CoordinateOrigin::TopLeft,
            })
        }
        _ => None,
    };
    OdfPresentationGeometry {
        x,
        y,
        width,
        height,
        transform: attr(node, "transform").map(str::to_string),
        view_box: attr(node, "viewBox").map(str::to_string),
        points: attr(node, "points").map(str::to_string),
        path: attr(node, "d")
            .or_else(|| attr(node, "enhanced-path"))
            .map(str::to_string),
        rotation_angle: attr(node, "rotation-angle").map(str::to_string),
        bbox_points,
        attributes: node
            .attributes
            .iter()
            .filter(|(key, _)| {
                matches!(
                    local_name(key),
                    "x" | "y"
                        | "width"
                        | "height"
                        | "transform"
                        | "viewBox"
                        | "points"
                        | "d"
                        | "enhanced-path"
                        | "rotation-angle"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    }
}

fn measurement_points(value: &str) -> Option<f64> {
    let split = value
        .find(|character: char| {
            !character.is_ascii_digit() && !matches!(character, '.' | '-' | '+')
        })
        .unwrap_or(value.len());
    let number = value[..split].parse::<f64>().ok()?;
    let unit = value[split..].trim().to_ascii_lowercase();
    Some(match unit.as_str() {
        "cm" => number * 72.0 / 2.54,
        "mm" => number * 72.0 / 25.4,
        "in" => number * 72.0,
        "pc" => number * 12.0,
        "px" => number * 0.75,
        "pt" | "" => number,
        _ => return None,
    })
}

fn reading_order(shapes: &[OdfPresentationShape]) -> OdfPresentationReadingOrder {
    let mut ordered = shapes.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| reading_key(left).cmp(&reading_key(right)));
    let all_positioned = shapes
        .iter()
        .all(|shape| shape.geometry.bbox_points.is_some());
    let confidence = if shapes.is_empty() {
        1.0
    } else if all_positioned {
        0.86
    } else {
        0.62
    };
    let entries = ordered
        .into_iter()
        .enumerate()
        .map(|(rank, shape)| {
            let title = matches!(
                shape.presentation_class.as_deref(),
                Some("title" | "subtitle")
            );
            let mut evidence = Vec::new();
            if title {
                evidence.push("presentation title placeholder priority".into());
            }
            if shape.geometry.bbox_points.is_some() {
                evidence.push("top-to-bottom then left-to-right geometry".into());
            }
            evidence.push("source z-order fallback and tie-break".into());
            OdfPresentationReadingOrderEntry {
                rank,
                shape_id: shape.shape_id.clone(),
                source_z_order: shape.z_order,
                confidence: if title { 0.94 } else { confidence },
                evidence,
                locator: shape.locator.clone(),
            }
        })
        .collect();
    OdfPresentationReadingOrder {
        method: "grist.presentation_odf.reading-order.v1".into(),
        confidence,
        entries,
    }
}

fn reading_key(shape: &OdfPresentationShape) -> (u8, i64, i64, usize, &str) {
    let title = !matches!(
        shape.presentation_class.as_deref(),
        Some("title" | "subtitle")
    );
    let (y, x) = shape
        .geometry
        .bbox_points
        .map_or((i64::MAX, i64::MAX), |bbox| {
            ((bbox.y * 1_000.0) as i64, (bbox.x * 1_000.0) as i64)
        });
    (u8::from(title), y, x, shape.z_order, &shape.shape_id)
}

fn slide_locator(
    entry: &PackageEntry,
    document: &XmlDocument,
    node: &XmlElement,
    slide: usize,
    shape_id: Option<String>,
    bbox: Option<BoundingBox>,
) -> crate::core::SourceLocator {
    element_locator(entry, document, node)
        .nested(LocationComponent::SlideRegion {
            slide: IndexPosition::new(slide as u64, IndexBase::One).expect("slides are one-based"),
            shape_id,
            bbox,
        })
        .expect("valid ODF slide locator")
}

fn shape_kind(node: &XmlElement) -> OdfPresentationShapeKind {
    match local_name(&node.name) {
        "frame" => OdfPresentationShapeKind::Frame,
        "g" => OdfPresentationShapeKind::Group,
        "rect" => OdfPresentationShapeKind::Rectangle,
        "ellipse" | "circle" => OdfPresentationShapeKind::Ellipse,
        "line" => OdfPresentationShapeKind::Line,
        "connector" => OdfPresentationShapeKind::Connector,
        "custom-shape" => OdfPresentationShapeKind::CustomShape,
        "polygon" => OdfPresentationShapeKind::Polygon,
        "polyline" => OdfPresentationShapeKind::Polyline,
        "path" => OdfPresentationShapeKind::Path,
        "caption" => OdfPresentationShapeKind::Caption,
        "measure" => OdfPresentationShapeKind::Measure,
        "control" => OdfPresentationShapeKind::Control,
        "page-thumbnail" => OdfPresentationShapeKind::PageThumbnail,
        _ => OdfPresentationShapeKind::Unknown,
    }
}

fn is_shape(node: &XmlElement) -> bool {
    node.name.starts_with("draw:")
        && matches!(
            local_name(&node.name),
            "frame"
                | "g"
                | "rect"
                | "ellipse"
                | "circle"
                | "line"
                | "connector"
                | "custom-shape"
                | "polygon"
                | "polyline"
                | "path"
                | "caption"
                | "measure"
                | "control"
                | "page-thumbnail"
        )
}

fn find_shape_node(
    document: &XmlDocument,
    page: usize,
    shape: &OdfPresentationShape,
) -> Option<usize> {
    descendants(document, page).find_map(|node| {
        let matches_id = attr(node, "id")
            .or_else(|| attr(node, "name"))
            .is_some_and(|value| value == shape.shape_id);
        (is_shape(node) && (matches_id || raw_xml(document, node) == shape.raw_xml))
            .then(|| node_index(document, node))
    })
}

fn metadata_node(
    entry: &PackageEntry,
    document: &XmlDocument,
    index: usize,
) -> OdfPresentationXmlMetadata {
    let node = &document.nodes[index];
    OdfPresentationXmlMetadata {
        qualified_name: node.name.clone(),
        attributes: node.attributes.clone(),
        text: descendant_text(document, index),
        raw_xml: raw_xml(document, node).to_string(),
        locator: element_locator(entry, document, node),
    }
}

fn media_type_for(manifest: &[OdfPresentationManifestEntry], path: &str) -> Option<String> {
    manifest
        .iter()
        .find(|entry| entry.full_path.trim_end_matches('/') == path.trim_end_matches('/'))
        .and_then(|entry| entry.media_type.clone())
}

fn matching_members(base: &str, href: &str, entries: &[PackageEntry]) -> Vec<String> {
    let Some(path) = resolve_member_path(base, href) else {
        return Vec::new();
    };
    let prefix = format!("{}/", path.trim_end_matches('/'));
    entries
        .iter()
        .filter(|entry| entry.path == path || entry.path.starts_with(&prefix))
        .map(|entry| entry.path.clone())
        .collect()
}

fn resolve_member_path(base: &str, href: &str) -> Option<String> {
    if href.is_empty() || href.starts_with('#') || is_external_href(href) {
        return None;
    }
    let href = href.split('#').next().unwrap_or(href);
    let mut path = if href.starts_with('/') {
        PathBuf::new()
    } else {
        Path::new(base)
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf()
    };
    for component in Path::new(href.trim_start_matches('/')).components() {
        match component {
            Component::Normal(value) => path.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !path.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(path.to_string_lossy().replace(char::from(92), "/"))
}

fn is_external_href(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    lower.contains("://")
        || lower.starts_with("mailto:")
        || lower.starts_with("javascript:")
        || lower.starts_with("data:")
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn descendant_named_text(document: &XmlDocument, root: usize, name: &str) -> Option<String> {
    descendants(document, root)
        .find(|node| local_name(&node.name) == name)
        .map(|node| {
            descendant_text(document, node_index(document, node))
                .trim()
                .to_string()
        })
}

fn child_indexes(node: &XmlElement) -> impl Iterator<Item = usize> + '_ {
    node.content.iter().filter_map(|content| match content {
        XmlContent::Child(index) => Some(*index),
        XmlContent::Text { .. } => None,
    })
}

fn descendants(document: &XmlDocument, root: usize) -> impl Iterator<Item = &XmlElement> {
    let prefix = format!("{}/", document.nodes[root].path);
    document
        .nodes
        .iter()
        .skip(root + 1)
        .take_while(move |node| node.path.starts_with(&prefix))
}

fn node_index(document: &XmlDocument, node: &XmlElement) -> usize {
    document
        .nodes
        .iter()
        .position(|candidate| std::ptr::eq(candidate, node))
        .expect("XML element belongs to document")
}

fn slide_node_count(slide: &OdfPresentationSlide) -> usize {
    1 + slide.shapes.len()
        + slide.notes.len()
        + slide.comments.len()
        + slide
            .tables
            .iter()
            .map(|table| {
                1 + table.rows.len() + table.rows.iter().map(|row| row.cells.len()).sum::<usize>()
            })
            .sum::<usize>()
        + slide.charts.len()
        + slide.images.len()
        + slide.links.len()
        + slide.animations.len()
        + slide.embedded_objects.len()
        + usize::from(slide.transition.is_some())
}
