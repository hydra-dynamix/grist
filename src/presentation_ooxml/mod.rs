//! Safe PPTX, PPTM, POTX, and PPSX package parser.
//!
//! This module parses the OPC package layer without executing macros, actions,
//! external relationships, or embedded objects.

mod archive;
mod artifacts;
mod content;
mod content_types;
mod graph;
mod model;
mod properties;
mod relationships;
mod structure;
mod xml;

pub use model::*;

use crate::core::{
    ContentIdentity, Diagnostic, FormatIdentity, LocationComponent, OperationStatus, ParserInfo,
    SchemaVersion, SourceLocator, SourceLocatorError,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};
use crate::security::ArchiveEntryKind;
use archive::PackageEntry;
use content_types::{content_type_for, parse_manifest};

pub(crate) const PARSER: &str = "grist.presentation_ooxml";

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("zip + quick-xml", "4.6.1/0.37.5")
        .with_specification_version("ECMA-376 OPC and PresentationML package conventions")
        .with_feature("presentation-ooxml")
}

pub(crate) fn parse_pptx_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, PresentationPackageKind::Presentation)
}

pub(crate) fn parse_pptm_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, PresentationPackageKind::MacroEnabledPresentation)
}

pub(crate) fn parse_potx_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, PresentationPackageKind::Template)
}

pub(crate) fn parse_ppsx_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, PresentationPackageKind::Slideshow)
}

fn parse_registered(
    context: &mut ParserContext<'_>,
    expected_kind: PresentationPackageKind,
) -> Result<ParserOutput, ParserError> {
    let options: PresentationOoxmlOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(PARSER, error.to_string())))?;
    if looks_like_encrypted_ooxml(context.bytes()) {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![Diagnostic::error(
                PARSER,
                "presentation_ooxml.encrypted.compound_package",
                "encrypted PresentationML compound package requires an explicit decryption provider",
            )],
        ));
    }
    let mut package = archive::read_package(context.bytes(), context)?;
    if package.encrypted {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![Diagnostic::error(
                PARSER,
                "presentation_ooxml.encrypted.zip_member",
                "one or more PresentationML package parts are encrypted",
            )],
        ));
    }
    let mut diagnostics = std::mem::take(&mut package.diagnostics);
    let manifest = parse_manifest(&package.entries, &mut diagnostics)
        .map_err(|message| Box::new(Diagnostic::malformed(PARSER, message)))?;
    if manifest.package_kind != expected_kind {
        return Err(Box::new(Diagnostic::error(
            PARSER,
            "presentation_ooxml.package_kind_mismatch",
            format!(
                "requested {} but package content types declare {}",
                expected_kind.format_id(),
                manifest.package_kind.format_id()
            ),
        )));
    }
    require_available_part(&package.entries, &manifest.main_presentation_part)?;
    let relationships = relationships::parse_relationships(&package.entries, &mut diagnostics);
    validate_main_relationship(&relationships, &manifest.main_presentation_part)?;
    let parent_identity =
        ContentIdentity::for_raw_bytes(context.bytes()).with_format(FormatIdentity::new(
            manifest.package_kind.format_id(),
            Some(manifest.package_media_type.clone()),
        ));
    let properties =
        properties::parse_properties(&package.entries, &relationships, &mut diagnostics);
    let structure = structure::parse_structure(
        &package.entries,
        &relationships,
        &manifest.main_presentation_part,
        &mut diagnostics,
    );
    let slide_contents = content::parse_slide_contents(
        &package.entries,
        &relationships,
        &manifest.content_types,
        &structure.slides,
        &mut diagnostics,
    );
    let (macro_projects, child_artifacts) = artifacts::build_artifacts(
        &package.entries,
        &manifest.content_types,
        &relationships,
        &parent_identity,
        &options,
    )?;
    if !manifest.package_kind.macro_enabled() && !macro_projects.is_empty() {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "presentation_ooxml.macro.unexpected_in_non_macro_package",
                "macro content was found in a non-macro presentation and was quarantined",
            )
            .with_locator(macro_projects[0].locator.clone())
            .partial(),
        );
    }
    context.consume_child_artifacts(
        u64::try_from(macro_projects.len().saturating_add(child_artifacts.len()))
            .unwrap_or(u64::MAX),
    )?;
    let parts = package_parts(&package.entries, &manifest.content_types);
    let node_count = 1usize
        .saturating_add(parts.len())
        .saturating_add(relationships.len())
        .saturating_add(properties.core.len())
        .saturating_add(properties.extended.len())
        .saturating_add(properties.custom.len())
        .saturating_add(structure.slides.len())
        .saturating_add(slide_content_node_count(&slide_contents))
        .saturating_add(structure.masters.len())
        .saturating_add(structure.layouts.len())
        .saturating_add(structure.themes.len())
        .saturating_add(structure.actions.len())
        .saturating_add(macro_projects.len())
        .saturating_add(child_artifacts.len());
    context.consume_nodes(u64::try_from(node_count).unwrap_or(u64::MAX))?;
    let document = PresentationOoxmlDocument {
        schema_version: SchemaVersion::PRESENTATION_OOXML_V1.into(),
        package_kind: manifest.package_kind,
        package_media_type: manifest.package_media_type,
        main_presentation_locator: part_locator(&manifest.main_presentation_part)
            .expect("validated main presentation part"),
        main_presentation_part: manifest.main_presentation_part,
        content_types: manifest.content_types,
        parts,
        relationships,
        properties,
        slides: structure.slides,
        slide_contents,
        masters: structure.masters,
        layouts: structure.layouts,
        themes: structure.themes,
        actions: structure.actions,
        macro_projects,
        child_artifacts,
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

fn package_parts(
    entries: &[PackageEntry],
    types: &PresentationContentTypes,
) -> Vec<PresentationPackagePart> {
    entries
        .iter()
        .map(|entry| {
            let status = if entry.rejected.is_some() {
                PresentationPartStatus::Rejected
            } else if entry.encrypted {
                PresentationPartStatus::Encrypted
            } else if entry.kind == ArchiveEntryKind::Directory {
                PresentationPartStatus::Directory
            } else {
                PresentationPartStatus::Available
            };
            let content_type = content_type_for(types, &entry.path);
            let identity = entry.bytes.as_deref().map(|bytes| {
                ContentIdentity::for_raw_bytes(bytes).with_format(FormatIdentity::new(
                    "presentation_ooxml_part",
                    content_type.clone(),
                ))
            });
            PresentationPackagePart {
                package_index: entry.index,
                path: entry.path.clone(),
                content_type,
                compression: entry.compression.clone(),
                compressed_size: entry.compressed_size,
                uncompressed_size: entry.uncompressed_size,
                crc32: entry.crc32,
                status,
                rejection_code: entry.rejected.as_ref().map(|(code, _)| code.clone()),
                identity,
                locator: part_locator(&entry.path)
                    .unwrap_or_else(|_| fallback_part_locator(entry.index)),
            }
        })
        .collect()
}

fn slide_content_node_count(slides: &[PresentationSlideContent]) -> usize {
    slides.iter().fold(0usize, |count, slide| {
        let text = slide
            .shapes
            .iter()
            .filter_map(|shape| shape.text_body.as_ref())
            .fold(0usize, |count, body| {
                count.saturating_add(body.paragraphs.iter().fold(0usize, |count, paragraph| {
                    count.saturating_add(1 + paragraph.runs.len())
                }))
            });
        let notes = slide.notes.iter().fold(0usize, |count, note| {
            count.saturating_add(1 + note.shapes.len())
        });
        let table_cells = slide
            .tables
            .iter()
            .flat_map(|table| &table.rows)
            .fold(0usize, |count, row| {
                count.saturating_add(1 + row.cells.len())
            });
        count.saturating_add(
            1 + slide.shapes.len()
                + text
                + notes
                + slide.comments.len()
                + slide.tables.len()
                + table_cells
                + slide.charts.len()
                + slide.equations.len()
                + slide.images.len()
                + slide.links.len()
                + usize::from(slide.transition.is_some())
                + slide.animations.len()
                + slide.embedded_objects.len()
                + slide.reading_order.entries.len(),
        )
    })
}

fn require_available_part(entries: &[PackageEntry], path: &str) -> Result<(), ParserError> {
    if entries.iter().any(|entry| {
        entry.path == path && entry.bytes.is_some() && entry.rejected.is_none() && !entry.encrypted
    }) {
        Ok(())
    } else {
        Err(Box::new(Diagnostic::malformed(
            PARSER,
            format!("declared PresentationML main part {path} is missing or unavailable"),
        )))
    }
}

fn validate_main_relationship(
    relationships: &[PresentationRelationship],
    main_part: &str,
) -> Result<(), ParserError> {
    let office = relationships
        .iter()
        .filter(|relationship| {
            relationship.source_part.is_none()
                && relationship.relationship_type == relationships::OFFICE_DOCUMENT_RELATIONSHIP
        })
        .collect::<Vec<_>>();
    if office.len() == 1
        && office[0].resolved_part.as_deref() == Some(main_part)
        && office[0].target_exists == Some(true)
    {
        Ok(())
    } else {
        Err(Box::new(Diagnostic::malformed(
            PARSER,
            "package root relationships must resolve exactly one officeDocument target matching the declared PresentationML main part",
        )))
    }
}

pub(super) fn part_locator(path: &str) -> Result<SourceLocator, SourceLocatorError> {
    SourceLocator::exact(LocationComponent::OoxmlPart {
        part: path.to_string(),
        paragraph: None,
        run: None,
        table: None,
        row: None,
        column: None,
        object_id: None,
    })
}

pub(super) fn fallback_part_locator(index: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::OoxmlPart {
        part: "[rejected-package-member]".into(),
        paragraph: None,
        run: None,
        table: None,
        row: None,
        column: None,
        object_id: Some(format!("package-index:{index}")),
    })
    .expect("fallback OOXML locator")
}

pub(super) fn xml_locator(part: &str, path: &str) -> SourceLocator {
    part_locator(part)
        .expect("validated package part")
        .nested(LocationComponent::XmlPath { path: path.into() })
        .expect("absolute XML path")
}

pub(super) fn normalize_manifest_part_name(value: &str) -> Option<String> {
    relationships::resolve_internal_target(None, value).ok()
}

fn looks_like_encrypted_ooxml(bytes: &[u8]) -> bool {
    const OLE_MAGIC: &[u8] = &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    bytes.starts_with(OLE_MAGIC)
        && (contains_utf16le_ascii(bytes, "EncryptedPackage")
            || contains_utf16le_ascii(bytes, "EncryptionInfo"))
}

fn contains_utf16le_ascii(bytes: &[u8], needle: &str) -> bool {
    let encoded = needle
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    bytes.windows(encoded.len()).any(|window| window == encoded)
}
