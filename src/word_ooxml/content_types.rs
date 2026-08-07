//! OPC content-type manifest parsing and Word package classification.

use super::archive::PackageEntry;
use super::model::{
    WordContentTypeDefault, WordContentTypeOverride, WordContentTypes, WordPackageKind,
};
use super::xml_util::{attribute, local_name, parse_xml_part};
use super::{PARSER, normalize_manifest_part_name, part_locator, xml_locator};
use crate::core::Diagnostic;

pub(super) const CONTENT_TYPES_PART: &str = "[Content_Types].xml";
pub(super) const DOCX_MAIN: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
pub(super) const DOCM_MAIN: &str = "application/vnd.ms-word.document.macroEnabled.main+xml";
pub(super) const DOTX_MAIN: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
pub(super) const DOTM_MAIN: &str = "application/vnd.ms-word.template.macroEnabledTemplate.main+xml";
pub(super) const VBA_PROJECT: &str = "application/vnd.ms-office.vbaProject";

pub(super) struct Manifest {
    pub content_types: WordContentTypes,
    pub package_kind: WordPackageKind,
    pub package_media_type: String,
    pub main_document_part: String,
}

pub(super) fn parse_manifest(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Manifest, String> {
    let entry = entries
        .iter()
        .find(|entry| entry.path == CONTENT_TYPES_PART && entry.rejected.is_none())
        .ok_or_else(|| "OOXML package is missing [Content_Types].xml".to_string())?;
    let bytes = entry
        .bytes
        .as_deref()
        .ok_or_else(|| "OOXML content-type manifest is unavailable or encrypted".to_string())?;
    let nodes = parse_xml_part(CONTENT_TYPES_PART, bytes, diagnostics)
        .map_err(|()| "OOXML content-type manifest could not be parsed safely".to_string())?;
    let root = nodes
        .first()
        .filter(|node| local_name(&node.name) == "Types")
        .ok_or_else(|| "OOXML content-type manifest has no Types root".to_string())?;

    let mut defaults = Vec::new();
    let mut overrides = Vec::new();
    for child in &root.children {
        let node = &nodes[*child];
        match local_name(&node.name) {
            "Default" => {
                let extension = attribute(node, "Extension")
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "content-type Default is missing Extension".to_string())?;
                let content_type = attribute(node, "ContentType")
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "content-type Default is missing ContentType".to_string())?;
                defaults.push(WordContentTypeDefault {
                    extension: extension.to_ascii_lowercase(),
                    content_type: content_type.to_string(),
                    locator: xml_locator(CONTENT_TYPES_PART, &node.path),
                });
            }
            "Override" => {
                let part_name = attribute(node, "PartName")
                    .ok_or_else(|| "content-type Override is missing PartName".to_string())?;
                let content_type = attribute(node, "ContentType")
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| "content-type Override is missing ContentType".to_string())?;
                let part_name = normalize_manifest_part_name(part_name).ok_or_else(|| {
                    format!("content-type Override has unsafe PartName {part_name:?}")
                })?;
                overrides.push(WordContentTypeOverride {
                    part_name,
                    content_type: content_type.to_string(),
                    locator: xml_locator(CONTENT_TYPES_PART, &node.path),
                });
            }
            other => diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.content_types.unknown_element",
                    format!("unknown content-type manifest element {other} was retained as a package part"),
                )
                .with_locator(xml_locator(CONTENT_TYPES_PART, &node.path))
                .partial(),
            ),
        }
    }
    defaults.sort_by(|left, right| {
        left.extension
            .cmp(&right.extension)
            .then_with(|| left.content_type.cmp(&right.content_type))
    });
    overrides.sort_by(|left, right| left.part_name.cmp(&right.part_name));
    diagnose_duplicates(&defaults, &overrides, diagnostics);

    let main = overrides
        .iter()
        .filter_map(|item| {
            package_kind(&item.content_type)
                .map(|kind| (kind, item.part_name.clone(), item.content_type.clone()))
        })
        .collect::<Vec<_>>();
    if main.len() != 1 {
        return Err(format!(
            "OOXML package must declare exactly one supported Word main document part, found {}",
            main.len()
        ));
    }
    let (package_kind, main_document_part, _) = main.into_iter().next().unwrap();
    Ok(Manifest {
        content_types: WordContentTypes {
            locator: part_locator(CONTENT_TYPES_PART).expect("constant part locator"),
            defaults,
            overrides,
        },
        package_kind,
        package_media_type: package_media_type(package_kind).into(),
        main_document_part,
    })
}

fn diagnose_duplicates(
    defaults: &[WordContentTypeDefault],
    overrides: &[WordContentTypeOverride],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for pair in defaults.windows(2) {
        if pair[0].extension == pair[1].extension {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.content_types.duplicate_default",
                    format!("duplicate content-type default for .{}", pair[0].extension),
                )
                .with_locator(pair[1].locator.clone())
                .partial(),
            );
        }
    }
    for pair in overrides.windows(2) {
        if pair[0].part_name == pair[1].part_name {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "word_ooxml.content_types.duplicate_override",
                    format!("duplicate content-type override for {}", pair[0].part_name),
                )
                .with_locator(pair[1].locator.clone())
                .partial(),
            );
        }
    }
}

pub(super) fn content_type_for(types: &WordContentTypes, part: &str) -> Option<String> {
    if let Some(item) = types.overrides.iter().find(|item| item.part_name == part) {
        return Some(item.content_type.clone());
    }
    let extension = part.rsplit_once('.')?.1.to_ascii_lowercase();
    types
        .defaults
        .iter()
        .find(|item| item.extension == extension)
        .map(|item| item.content_type.clone())
}

fn package_kind(content_type: &str) -> Option<WordPackageKind> {
    match content_type {
        DOCX_MAIN => Some(WordPackageKind::Document),
        DOCM_MAIN => Some(WordPackageKind::MacroEnabledDocument),
        DOTX_MAIN => Some(WordPackageKind::Template),
        DOTM_MAIN => Some(WordPackageKind::MacroEnabledTemplate),
        _ => None,
    }
}

pub(super) const fn package_media_type(kind: WordPackageKind) -> &'static str {
    match kind {
        WordPackageKind::Document => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        WordPackageKind::MacroEnabledDocument => "application/vnd.ms-word.document.macroEnabled.12",
        WordPackageKind::Template => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template"
        }
        WordPackageKind::MacroEnabledTemplate => "application/vnd.ms-word.template.macroEnabled.12",
    }
}
