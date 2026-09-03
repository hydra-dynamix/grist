//! Embedded-child and inert VBA-project artifact construction.

use super::PARSER;
use super::archive::PackageEntry;
use super::content_types::VBA_PROJECT;
use super::model::{
    PresentationChildArtifact, PresentationContentTypes, PresentationMacroProject,
    PresentationOoxmlOptions, PresentationRelationship, PresentationRelationshipTargetMode,
};
use crate::container::{
    ArtifactDisposition, ArtifactMetadata, ArtifactParent, ArtifactRelationship,
    ArtifactSafetyClassification, EmbeddedArtifact,
};
use crate::core::{ContentIdentity, Diagnostic, FormatIdentity};
use crate::registry::ParserError;

pub(super) fn build_artifacts(
    entries: &[PackageEntry],
    types: &PresentationContentTypes,
    relationships: &[PresentationRelationship],
    parent_identity: &ContentIdentity,
    options: &PresentationOoxmlOptions,
) -> Result<
    (
        Vec<PresentationMacroProject>,
        Vec<PresentationChildArtifact>,
    ),
    ParserError,
> {
    let mut macros = Vec::new();
    let mut children = Vec::new();
    for entry in entries.iter().filter(|entry| {
        entry.bytes.is_some()
            && entry.rejected.is_none()
            && entry.kind == crate::security::ArchiveEntryKind::RegularFile
    }) {
        let Some(bytes) = entry.bytes.as_deref() else {
            continue;
        };
        let content_type = super::content_types::content_type_for(types, &entry.path);
        let related = relationships
            .iter()
            .filter(|relationship| {
                relationship.target_mode == PresentationRelationshipTargetMode::Internal
                    && relationship.resolved_part.as_deref() == Some(entry.path.as_str())
            })
            .collect::<Vec<_>>();
        let locator = super::part_locator(&entry.path).expect("validated package path");
        if is_macro_part(&entry.path, content_type.as_deref(), &related) {
            let metadata = ArtifactMetadata::new(
                ArtifactParent::new(parent_identity.clone(), ArtifactRelationship::EmbeddedIn),
                locator.clone(),
                ArtifactDisposition::PackagePart,
            )
            .with_declared_filename(entry.path.clone())
            .with_media_type(
                content_type
                    .clone()
                    .unwrap_or_else(|| VBA_PROJECT.to_string()),
            )
            .with_safety_hint(ArtifactSafetyClassification::Macro);
            let artifact = if options.extract_macro_bytes {
                EmbeddedArtifact::capture_inline(metadata, bytes)
            } else {
                EmbeddedArtifact::inventory(metadata, bytes)
            }
            .map_err(artifact_error)?;
            let identity = ContentIdentity::for_raw_bytes(bytes)
                .with_format(FormatIdentity::new("vba_project", content_type.clone()));
            macros.push(PresentationMacroProject {
                part: entry.path.clone(),
                content_type,
                relationship_ids: relationship_ids(&related),
                identity,
                locator,
                artifact,
            });
        } else if is_child_part(&entry.path, &related) {
            let mut metadata = ArtifactMetadata::new(
                ArtifactParent::new(parent_identity.clone(), ArtifactRelationship::EmbeddedIn),
                locator.clone(),
                ArtifactDisposition::Inline,
            )
            .with_declared_filename(entry.path.clone());
            if let Some(content_type) = &content_type {
                metadata = metadata.with_media_type(content_type.clone());
            }
            let artifact = if options.inline_child_artifact_bytes {
                EmbeddedArtifact::capture_inline(metadata, bytes)
            } else {
                EmbeddedArtifact::inventory(metadata, bytes)
            }
            .map_err(artifact_error)?;
            children.push(PresentationChildArtifact {
                part: entry.path.clone(),
                content_type,
                relationship_ids: relationship_ids(&related),
                locator,
                artifact,
            });
        }
    }
    macros.sort_by(|left, right| left.part.cmp(&right.part));
    children.sort_by(|left, right| left.part.cmp(&right.part));
    Ok((macros, children))
}

fn is_macro_part(
    path: &str,
    content_type: Option<&str>,
    relationships: &[&PresentationRelationship],
) -> bool {
    content_type.is_some_and(|value| value.eq_ignore_ascii_case(VBA_PROJECT))
        || path.eq_ignore_ascii_case("ppt/vbaProject.bin")
        || relationships.iter().any(|relationship| {
            relationship
                .relationship_type
                .to_ascii_lowercase()
                .ends_with("/vbaproject")
        })
}

fn is_child_part(path: &str, relationships: &[&PresentationRelationship]) -> bool {
    let normalized = path.to_ascii_lowercase();
    normalized.starts_with("ppt/embeddings/")
        || normalized.starts_with("ppt/media/")
        || relationships.iter().any(|relationship| {
            let relationship_type = relationship.relationship_type.to_ascii_lowercase();
            [
                "/package",
                "/oleobject",
                "/object",
                "/embeddedobject",
                "/image",
                "/audio",
                "/video",
                "/afchunk",
            ]
            .iter()
            .any(|suffix| relationship_type.ends_with(suffix))
        })
}

fn relationship_ids(relationships: &[&PresentationRelationship]) -> Vec<String> {
    let mut ids = relationships
        .iter()
        .map(|relationship| {
            format!(
                "{}#{}",
                relationship.source_part.as_deref().unwrap_or("/"),
                relationship.id
            )
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn artifact_error(error: crate::container::EmbeddedArtifactError) -> ParserError {
    Box::new(Diagnostic::parser_defect(
        PARSER,
        format!("embedded-artifact invariant failed: {error}"),
    ))
}
