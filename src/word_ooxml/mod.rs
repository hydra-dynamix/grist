//! Safe DOCX, DOCM, DOTX, and DOTM package parser.
//!
//! This module owns the OPC package layer. WordprocessingML body semantics are
//! deliberately separate so parts, relationships, metadata, macros, and child
//! artifacts remain available even before a visible-text projection is chosen.

mod archive;
mod artifacts;
mod content;
mod content_types;
mod formatting;
pub(crate) mod graph;
mod model;
mod numbering;
mod properties;
mod relationships;
mod rich_content;
mod styles;
mod xml_util;

pub use model::*;

use crate::core::{
    ContentIdentity, Diagnostic, FormatIdentity, LocationComponent, OperationStatus, ParserInfo,
    SchemaVersion, SourceLocator, SourceLocatorError,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};
use crate::security::ArchiveEntryKind;
use archive::PackageEntry;
use content_types::{content_type_for, parse_manifest};

pub(crate) const PARSER: &str = "grist.word_ooxml";

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("zip + quick-xml", "4.6.1/0.37.5")
        .with_specification_version("ECMA-376 OPC and WordprocessingML package conventions")
        .with_feature("word-ooxml")
}

pub(crate) fn parse_docx_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, WordPackageKind::Document)
}

pub(crate) fn parse_docm_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, WordPackageKind::MacroEnabledDocument)
}

pub(crate) fn parse_dotx_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, WordPackageKind::Template)
}

pub(crate) fn parse_dotm_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, WordPackageKind::MacroEnabledTemplate)
}

fn parse_registered(
    context: &mut ParserContext<'_>,
    expected_kind: WordPackageKind,
) -> Result<ParserOutput, ParserError> {
    let options: WordOoxmlOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(PARSER, error.to_string())))?;
    if looks_like_encrypted_ooxml(context.bytes()) {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![Diagnostic::error(
                PARSER,
                "word_ooxml.encrypted.compound_package",
                "encrypted OOXML compound package requires an explicit decryption provider",
            )],
        ));
    }
    let mut package = archive::read_package(context.bytes(), context)?;
    if package.encrypted {
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![Diagnostic::error(
                PARSER,
                "word_ooxml.encrypted.zip_member",
                "one or more OOXML package parts are encrypted",
            )],
        ));
    }
    let mut diagnostics = std::mem::take(&mut package.diagnostics);
    let manifest = parse_manifest(&package.entries, &mut diagnostics)
        .map_err(|message| Box::new(Diagnostic::malformed(PARSER, message)))?;
    if manifest.package_kind != expected_kind {
        return Err(Box::new(Diagnostic::error(
            PARSER,
            "word_ooxml.package_kind_mismatch",
            format!(
                "requested {} but package content types declare {}",
                expected_kind.format_id(),
                manifest.package_kind.format_id()
            ),
        )));
    }
    require_available_part(&package.entries, &manifest.main_document_part)?;

    let relationships = relationships::parse_relationships(&package.entries, &mut diagnostics);
    validate_main_relationship(&relationships, &manifest.main_document_part)?;
    let parent_identity =
        ContentIdentity::for_raw_bytes(context.bytes()).with_format(FormatIdentity::new(
            manifest.package_kind.format_id(),
            Some(manifest.package_media_type.clone()),
        ));
    let properties =
        properties::parse_properties(&package.entries, &relationships, &mut diagnostics);
    let styles = styles::parse_styles(
        &package.entries,
        &relationships,
        &manifest.main_document_part,
        &mut diagnostics,
    );
    let numbering = numbering::parse_numbering(
        &package.entries,
        &relationships,
        &manifest.main_document_part,
        &mut diagnostics,
    );
    let content = content::parse_content(
        &package.entries,
        &relationships,
        &manifest.main_document_part,
        &styles,
        &numbering,
        &mut diagnostics,
    );
    let rich_content = rich_content::parse_rich_content(
        &package.entries,
        &relationships,
        &manifest.main_document_part,
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
                "word_ooxml.macro.unexpected_in_non_macro_package",
                "macro content was found in a non-macro Word package and was quarantined",
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
        .saturating_add(macro_projects.len())
        .saturating_add(child_artifacts.len());
    let node_count = node_count
        .saturating_add(styles.len())
        .saturating_add(numbering.abstract_definitions.len())
        .saturating_add(numbering.instances.len())
        .saturating_add(content.node_count)
        .saturating_add(rich_content.node_count);
    context.consume_nodes(u64::try_from(node_count).unwrap_or(u64::MAX))?;
    let document = WordOoxmlDocument {
        schema_version: SchemaVersion::WORD_OOXML_V1.into(),
        package_kind: manifest.package_kind,
        package_media_type: manifest.package_media_type,
        main_document_locator: part_locator(&manifest.main_document_part)
            .expect("validated main document part"),
        main_document_part: manifest.main_document_part,
        content_types: manifest.content_types,
        parts,
        relationships,
        properties,
        macro_projects,
        child_artifacts,
        styles,
        numbering,
        body: content.body,
        footnotes: content.footnotes,
        endnotes: content.endnotes,
        headers: content.headers,
        footers: content.footers,
        revision_graph: rich_content.revision_graph,
        comments: rich_content.comments,
        content_controls: rich_content.content_controls,
        equations: rich_content.equations,
        drawings: rich_content.drawings,
        charts: rich_content.charts,
        captions: rich_content.captions,
        text_boxes: rich_content.text_boxes,
        embedded_objects: rich_content.embedded_objects,
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

fn package_parts(entries: &[PackageEntry], types: &WordContentTypes) -> Vec<WordPackagePart> {
    entries
        .iter()
        .map(|entry| {
            let status = if entry.rejected.is_some() {
                WordPartStatus::Rejected
            } else if entry.encrypted {
                WordPartStatus::Encrypted
            } else if entry.kind == ArchiveEntryKind::Directory {
                WordPartStatus::Directory
            } else {
                WordPartStatus::Available
            };
            let content_type = content_type_for(types, &entry.path);
            let identity = entry.bytes.as_deref().map(|bytes| {
                ContentIdentity::for_raw_bytes(bytes)
                    .with_format(FormatIdentity::new("ooxml_part", content_type.clone()))
            });
            WordPackagePart {
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

fn require_available_part(entries: &[PackageEntry], path: &str) -> Result<(), ParserError> {
    if entries.iter().any(|entry| {
        entry.path == path && entry.bytes.is_some() && entry.rejected.is_none() && !entry.encrypted
    }) {
        Ok(())
    } else {
        Err(Box::new(Diagnostic::malformed(
            PARSER,
            format!("declared Word main document part {path} is missing or unavailable"),
        )))
    }
}

fn validate_main_relationship(
    relationships: &[WordRelationship],
    main_document_part: &str,
) -> Result<(), ParserError> {
    let office = relationships
        .iter()
        .filter(|relationship| {
            relationship.source_part.is_none()
                && relationship.relationship_type == relationships::OFFICE_DOCUMENT_RELATIONSHIP
        })
        .collect::<Vec<_>>();
    if office.len() == 1
        && office[0].resolved_part.as_deref() == Some(main_document_part)
        && office[0].target_exists == Some(true)
    {
        Ok(())
    } else {
        Err(Box::new(Diagnostic::malformed(
            PARSER,
            "package root relationships must resolve exactly one officeDocument target matching the declared Word main part",
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
    const OLE_MAGIC: &[u8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";
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
