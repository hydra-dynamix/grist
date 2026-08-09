use super::model::*;
use crate::container::ArtifactSafety;
use crate::core::{
    Diagnostic, IndexPosition, LocationComponent, LocatorConfidence, OperationControl, SourceInfo,
    SourceLocator,
};
use std::collections::BTreeSet;

pub(crate) const PARSER: &str = "grist.media";

#[derive(Debug, thiserror::Error)]
pub(crate) enum MediaParseError {
    #[error("{0}")]
    Malformed(String),
    #[error(transparent)]
    Control(#[from] crate::core::OperationControlError),
}

impl From<crate::core::BudgetExceeded> for MediaParseError {
    fn from(error: crate::core::BudgetExceeded) -> Self {
        Self::Control(error.into())
    }
}

pub(crate) fn parse_document(
    bytes: &[u8],
    format: MediaFormat,
    options: &MediaOptions,
    control: &OperationControl,
    charge_input: bool,
) -> Result<MediaDocument, MediaParseError> {
    validate_options(options)?;
    if charge_input {
        control.budget().consume_input_bytes(bytes.len() as u64)?;
    }
    control.checkpoint()?;
    control.budget().observe_memory_bytes(bytes.len() as u64)?;
    let mut parsed = match format {
        MediaFormat::Mp3 => super::parse_audio::parse_mp3(bytes, options, control)?,
        MediaFormat::Wav => super::parse_audio::parse_wav(bytes, options, control)?,
        MediaFormat::Flac => super::parse_audio::parse_flac(bytes, options, control)?,
        MediaFormat::Mp4 | MediaFormat::QuickTime => {
            super::parse_iso::parse_iso_bmff(bytes, format, options, control)?
        }
        MediaFormat::Matroska => super::parse_ebml::parse_matroska(bytes, options, control)?,
    };
    route_children(&mut parsed, options, control)?;
    control.budget().consume_nodes(
        (1 + parsed.streams.len()
            + parsed.chapters.len()
            + parsed.attachment_candidates.len()
            + parsed.subtitle_candidates.len()
            + parsed.artwork_candidates.len()
            + parsed.metadata.len()) as u64,
    )?;
    control.checkpoint()?;

    let attachments = parsed
        .attachment_candidates
        .drain(..)
        .enumerate()
        .map(|(index, candidate)| build_attachment(index, candidate, options))
        .collect();
    let subtitle_tracks = parsed
        .subtitle_candidates
        .drain(..)
        .enumerate()
        .map(|(index, candidate)| build_subtitle(index, candidate, options))
        .collect();
    let artwork = parsed
        .artwork_candidates
        .drain(..)
        .enumerate()
        .map(|(index, candidate)| build_artwork(index, candidate, options))
        .collect();

    let mut document = MediaDocument {
        schema_version: crate::core::SchemaVersion::MEDIA_V1.into(),
        format,
        technical: parsed.technical,
        streams: parsed.streams,
        chapters: parsed.chapters,
        attachments,
        subtitle_tracks,
        artwork,
        metadata: parsed.metadata,
        encrypted: parsed.encrypted,
        diagnostics: parsed.diagnostics,
        complete: parsed.complete,
    };
    disambiguate_document_ids(&mut document);
    Ok(document)
}

fn disambiguate_document_ids(document: &mut MediaDocument) {
    let mut chapter_ids = BTreeSet::new();
    for chapter in &mut document.chapters {
        chapter.id = unique_source_id(&chapter.id, &chapter.locator, &mut chapter_ids);
    }
    let mut attachment_ids = BTreeSet::new();
    for attachment in &mut document.attachments {
        attachment.id = unique_source_id(&attachment.id, &attachment.locator, &mut attachment_ids);
    }
    let mut artwork_ids = BTreeSet::new();
    for artwork in &mut document.artwork {
        artwork.id = unique_source_id(&artwork.id, &artwork.locator, &mut artwork_ids);
    }
}

fn unique_source_id(base: &str, locator: &SourceLocator, used: &mut BTreeSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let digest = crate::core::sha256_hex(
        &crate::core::canonical_json_bytes(locator).expect("source locator serializes"),
    );
    let suffix = digest.strip_prefix("sha256:").unwrap_or(&digest);
    let mut candidate = format!("{base}:source:{}", &suffix[..12]);
    let mut collision = 2u64;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}:source:{}:{collision}", &suffix[..12]);
        collision += 1;
    }
    candidate
}

fn validate_options(options: &MediaOptions) -> Result<(), MediaParseError> {
    if options.max_boxes == 0
        || options.max_nesting_depth == 0
        || options.max_metadata_bytes == 0
        || options.max_attachment_bytes == 0
        || options.max_subtitle_bytes == 0
    {
        return Err(MediaParseError::Malformed(
            "media limits must all be greater than zero".into(),
        ));
    }
    Ok(())
}

fn route_children(
    parsed: &mut ParsedMedia,
    options: &MediaOptions,
    control: &OperationControl,
) -> Result<(), MediaParseError> {
    for candidate in &mut parsed.attachment_candidates {
        control.budget().consume_child_artifacts(1)?;
        if candidate.inventory.is_some() {
            continue;
        }
        if candidate.bytes.len() as u64 > options.max_attachment_bytes {
            parsed.complete = false;
            parsed.diagnostics.push(limit_diagnostic(
                "media.attachment.limit",
                "embedded attachment exceeds max_attachment_bytes; retained as inventory only",
                &candidate.locator,
            ));
            candidate.inventory = Some((
                candidate.bytes.len() as u64,
                crate::core::sha256_hex(&candidate.bytes),
            ));
            candidate.bytes.clear();
            continue;
        }
        control
            .budget()
            .observe_memory_bytes(candidate.bytes.len() as u64)?;
        let media_type = candidate
            .media_type
            .clone()
            .or_else(|| infer_media_type(candidate.filename.as_deref(), &candidate.bytes));
        if options.parse_embedded
            && let Some(format) = subtitle_format(
                media_type
                    .as_deref()
                    .unwrap_or(candidate.filename.as_deref().unwrap_or("")),
                &candidate.bytes,
            )
        {
            let source =
                SourceInfo::stdin(candidate.filename.as_deref().unwrap_or("embedded-subtitle"))
                    .with_declared_mime_type(format.media_type());
            let envelope = crate::subtitle::parse_subtitle_with_operation_control(
                &candidate.bytes,
                source,
                format,
                &crate::subtitle::SubtitleOptions::default(),
                control,
            );
            if envelope.status != crate::core::OperationStatus::Complete {
                parsed.complete = false;
            }
            candidate.parsed_subtitle = envelope.payload.map(|mut document| {
                prefix_subtitle_locators(&candidate.locator, &mut document);
                document
            });
            parsed.diagnostics.extend(envelope.diagnostics);
        } else if options.parse_embedded
            && media_type
                .as_deref()
                .is_some_and(|value| value.starts_with("image/") && value != "image/svg+xml")
        {
            let envelope = crate::image::parse_image_with_operation_control(
                &candidate.bytes,
                SourceInfo::stdin(candidate.filename.as_deref().unwrap_or("embedded-image")),
                &crate::image::ImageOptions::default(),
                control,
            );
            if envelope.status != crate::core::OperationStatus::Complete {
                parsed.complete = false;
            }
            candidate.parsed_image = envelope.payload.map(|mut document| {
                prefix_image_locators(&candidate.locator, &mut document);
                document
            });
            parsed.diagnostics.extend(envelope.diagnostics);
        } else {
            control
                .budget()
                .consume_input_bytes(candidate.bytes.len() as u64)?;
        }
    }
    for candidate in &mut parsed.artwork_candidates {
        control.budget().consume_child_artifacts(1)?;
        if candidate.inventory.is_some() {
            continue;
        }
        if candidate.bytes.len() as u64 > options.max_attachment_bytes {
            parsed.complete = false;
            parsed.diagnostics.push(limit_diagnostic(
                "media.artwork.limit",
                "embedded artwork exceeds max_attachment_bytes; retained as inventory only",
                &candidate.locator,
            ));
            candidate.inventory = Some((
                candidate.bytes.len() as u64,
                crate::core::sha256_hex(&candidate.bytes),
            ));
            candidate.bytes.clear();
            continue;
        }
        control
            .budget()
            .observe_memory_bytes(candidate.bytes.len() as u64)?;
        if options.parse_embedded {
            let media_type = candidate
                .media_type
                .clone()
                .or_else(|| infer_media_type(None, &candidate.bytes));
            if media_type.as_deref() != Some("image/svg+xml") {
                let envelope = crate::image::parse_image_with_operation_control(
                    &candidate.bytes,
                    SourceInfo::stdin("embedded-artwork"),
                    &crate::image::ImageOptions::default(),
                    control,
                );
                if envelope.status != crate::core::OperationStatus::Complete {
                    parsed.complete = false;
                }
                candidate.image = envelope.payload.map(|mut document| {
                    prefix_image_locators(&candidate.locator, &mut document);
                    document
                });
                parsed.diagnostics.extend(envelope.diagnostics);
            } else {
                control
                    .budget()
                    .consume_input_bytes(candidate.bytes.len() as u64)?;
            }
        } else {
            control
                .budget()
                .consume_input_bytes(candidate.bytes.len() as u64)?;
        }
    }
    for candidate in &mut parsed.subtitle_candidates {
        control.budget().consume_child_artifacts(1)?;
        if candidate.inventory.is_some() {
            continue;
        }
        if candidate.bytes.len() as u64 > options.max_subtitle_bytes {
            parsed.complete = false;
            parsed.diagnostics.push(limit_diagnostic(
                "media.subtitle.limit",
                "embedded subtitle exceeds max_subtitle_bytes; retained as inventory only",
                &candidate.locator,
            ));
            candidate.inventory = Some((
                candidate.bytes.len() as u64,
                crate::core::sha256_hex(&candidate.bytes),
            ));
            candidate.bytes.clear();
            continue;
        }
        // Subtitle parsing charges its own input and decoded-character budgets.
        if options.parse_embedded && !candidate.bytes.is_empty() {
            let format = subtitle_format(&candidate.codec, &candidate.bytes);
            if let Some(format) = format {
                let source = SourceInfo::stdin(format!(
                    "embedded-{}.{}",
                    candidate.stream_id,
                    format.as_str()
                ))
                .with_declared_mime_type(format.media_type());
                let envelope = crate::subtitle::parse_subtitle_with_operation_control(
                    &candidate.bytes,
                    source,
                    format,
                    &crate::subtitle::SubtitleOptions::default(),
                    control,
                );
                if envelope.status != crate::core::OperationStatus::Complete {
                    parsed.complete = false;
                }
                if let Some(document) = envelope.payload {
                    let mut document = document;
                    prefix_subtitle_locators(&candidate.locator, &mut document);
                    candidate.document = Some(document);
                } else {
                    parsed.complete = false;
                    parsed
                        .diagnostics
                        .extend(envelope.diagnostics.into_iter().map(|mut diagnostic| {
                            diagnostic.module = "media.embedded_subtitle".into();
                            diagnostic
                        }));
                }
            }
        } else {
            control
                .budget()
                .consume_input_bytes(candidate.bytes.len() as u64)?;
        }
    }
    Ok(())
}

fn build_attachment(
    index: usize,
    candidate: EmbeddedCandidate,
    options: &MediaOptions,
) -> MediaAttachment {
    let media_type = candidate
        .media_type
        .clone()
        .or_else(|| infer_media_type(candidate.filename.as_deref(), &candidate.bytes));
    let safety = ArtifactSafety::classify(
        media_type.as_deref(),
        candidate.filename.as_deref(),
        &candidate.bytes,
        None,
    );

    let inventory_only = candidate.inventory.is_some();
    let (byte_length, sha256) = candidate.inventory.clone().unwrap_or_else(|| {
        (
            candidate.bytes.len() as u64,
            crate::core::sha256_hex(&candidate.bytes),
        )
    });
    MediaAttachment {
        id: candidate
            .native_id
            .as_deref()
            .map(|id| format!("attachment:{id}"))
            .unwrap_or_else(|| format!("attachment:{sha256}")),
        index: index as u64,
        native_id: candidate.native_id,
        filename: candidate.filename,
        media_type,
        byte_length,
        sha256,
        safety,
        bytes: (!inventory_only && options.retain_embedded_bytes).then_some(candidate.bytes),
        parsed_subtitle: candidate.parsed_subtitle,
        parsed_image: candidate.parsed_image,
        locator: candidate.locator,
    }
}

fn build_subtitle(
    _index: usize,
    candidate: SubtitleCandidate,
    options: &MediaOptions,
) -> EmbeddedSubtitleTrack {
    let inventory_only = candidate.inventory.is_some();
    let (byte_length, sha256) = candidate.inventory.clone().unwrap_or_else(|| {
        (
            candidate.bytes.len() as u64,
            crate::core::sha256_hex(&candidate.bytes),
        )
    });
    EmbeddedSubtitleTrack {
        id: format!("subtitle:{}", candidate.stream_id),
        stream_id: candidate.stream_id,
        codec: candidate.codec,
        language: candidate.language,
        byte_length,
        sha256,
        bytes: (!inventory_only && options.retain_embedded_bytes).then_some(candidate.bytes),
        document: candidate.document,
        locator: candidate.locator,
    }
}

fn build_artwork(
    index: usize,
    candidate: ArtworkCandidate,
    options: &MediaOptions,
) -> MediaArtwork {
    let media_type = candidate
        .media_type
        .clone()
        .or_else(|| infer_media_type(None, &candidate.bytes));
    let safety = ArtifactSafety::classify(media_type.as_deref(), None, &candidate.bytes, None);

    let inventory_only = candidate.inventory.is_some();
    let (byte_length, sha256) = candidate.inventory.clone().unwrap_or_else(|| {
        (
            candidate.bytes.len() as u64,
            crate::core::sha256_hex(&candidate.bytes),
        )
    });
    MediaArtwork {
        id: format!("artwork:{sha256}"),
        index: index as u64,
        picture_type: candidate.picture_type,
        description: candidate.description,
        media_type,
        byte_length,
        sha256,
        safety,
        bytes: (!inventory_only && options.retain_embedded_bytes).then_some(candidate.bytes),
        image: candidate.image,
        locator: candidate.locator,
    }
}

fn subtitle_format(hint: &str, bytes: &[u8]) -> Option<crate::subtitle::SubtitleFormat> {
    let hint = hint.to_ascii_lowercase();
    if hint.contains("webvtt")
        || hint.contains("text/vtt")
        || hint.ends_with(".vtt")
        || bytes.starts_with(b"WEBVTT")
    {
        Some(crate::subtitle::SubtitleFormat::WebVtt)
    } else if hint.contains("ttml")
        || hint.contains("stpp")
        || hint.ends_with(".ttml")
        || bytes.windows(3).any(|w| w == b"<tt")
    {
        Some(crate::subtitle::SubtitleFormat::Ttml)
    } else if hint.contains("subrip")
        || hint.contains("utf8")
        || hint.ends_with(".srt")
        || bytes.windows(3).any(|w| w == b"-->")
    {
        Some(crate::subtitle::SubtitleFormat::Srt)
    } else {
        None
    }
}

pub(crate) fn infer_media_type(filename: Option<&str>, bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png".into());
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg".into());
    }
    if bytes.starts_with(b"GIF8") {
        return Some("image/gif".into());
    }
    let extension = filename
        .and_then(|name| std::path::Path::new(name).extension()?.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "srt" => Some("application/x-subrip".into()),
        "vtt" => Some("text/vtt".into()),
        "ttml" | "dfxp" => Some("application/ttml+xml".into()),
        "png" => Some("image/png".into()),
        "jpg" | "jpeg" => Some("image/jpeg".into()),
        "gif" => Some("image/gif".into()),
        "webp" => Some("image/webp".into()),
        _ => None,
    }
}

pub(crate) fn byte_locator(start: usize, end: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ByteRange {
        byte_start: start,
        byte_end: end,
    })
    .expect("ordered byte range")
}

pub(crate) fn timed_locator(
    start: usize,
    end: usize,
    start_ms: u64,
    end_ms: u64,
    track: Option<u64>,
) -> SourceLocator {
    with_media_time(byte_locator(start, end), start_ms, end_ms, track)
}

pub(crate) fn with_media_time(
    locator: SourceLocator,
    start_ms: u64,
    end_ms: u64,
    track: Option<u64>,
) -> SourceLocator {
    locator
        .nested(LocationComponent::MediaTime {
            start_ms,
            end_ms: end_ms.max(start_ms),
            track: track.map(IndexPosition::zero_based),
        })
        .expect("ordered media time")
}

pub(crate) fn synthesized_media_locator(
    start_ms: u64,
    end_ms: u64,
    track: Option<u64>,
) -> SourceLocator {
    SourceLocator::approximate(
        LocationComponent::MediaTime {
            start_ms,
            end_ms: end_ms.max(start_ms),
            track: track.map(IndexPosition::zero_based),
        },
        LocatorConfidence::new(0.95).expect("constant confidence is valid"),
    )
    .expect("ordered synthesized media time")
}

fn prefixed_locator(parent: &SourceLocator, child: &SourceLocator) -> SourceLocator {
    let mut components = parent.components().to_vec();
    components.extend_from_slice(child.components());
    SourceLocator::new(components, parent.precision().clone())
        .expect("valid parent and child locators compose")
}

fn prefix_subtitle_locators(
    parent: &SourceLocator,
    document: &mut crate::subtitle::SubtitleDocument,
) {
    document.locator = prefixed_locator(parent, &document.locator);
    for track in &mut document.tracks {
        track.locator = prefixed_locator(parent, &track.locator);
    }
    for region in &mut document.regions {
        region.locator = prefixed_locator(parent, &region.locator);
    }
    for style in &mut document.styles {
        style.locator = prefixed_locator(parent, &style.locator);
    }
    for cue in &mut document.cues {
        cue.locator = prefixed_locator(parent, &cue.locator);
        cue.text_locator = prefixed_locator(parent, &cue.text_locator);
        cue.structure_locator = cue
            .structure_locator
            .take()
            .map(|locator| prefixed_locator(parent, &locator));
        cue.timing.locator = cue
            .timing
            .locator
            .take()
            .map(|locator| prefixed_locator(parent, &locator));
        for run in &mut cue.runs {
            run.locator = prefixed_locator(parent, &run.locator);
        }
    }
    for entry in &mut document.transcript.entries {
        entry.text_locator = prefixed_locator(parent, &entry.text_locator);
        entry.time_locator = prefixed_locator(parent, &entry.time_locator);
    }
}

fn prefix_image_locators(parent: &SourceLocator, document: &mut crate::image::ImageDocument) {
    for frame in &mut document.frames {
        frame.locator = prefixed_locator(parent, &frame.locator);
    }
    for block in &mut document.metadata.blocks {
        block.locator = prefixed_locator(parent, &block.locator);
    }
    for text in &mut document.embedded_text {
        text.locator = prefixed_locator(parent, &text.locator);
    }
    for text in &mut document.text.native.value.entries {
        text.locator = prefixed_locator(parent, &text.locator);
    }
    for attempt in &mut document.text.ocr_attempts {
        attempt.scope.locator = prefixed_locator(parent, &attempt.scope.locator);
        for region in &mut attempt.regions {
            region.locator = prefixed_locator(parent, &region.locator);
        }
    }
    if let Some(reconciled) = &mut document.text.reconciled {
        for item in &mut reconciled.value.items {
            item.locator = prefixed_locator(parent, &item.locator);
        }
    }
    for chunk in &mut document.chunks {
        chunk.locator = prefixed_locator(parent, &chunk.locator);
    }
    for link in &mut document.links {
        link.locator = prefixed_locator(parent, &link.locator);
    }
    for active in &mut document.active_content {
        active.locator = prefixed_locator(parent, &active.locator);
    }
}

pub(crate) fn limit_diagnostic(code: &str, message: &str, locator: &SourceLocator) -> Diagnostic {
    Diagnostic::warning(PARSER, code, message)
        .partial()
        .with_locator(locator.clone())
}

pub(crate) fn utf8_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches(char::from(0))
        .trim()
        .to_string()
}
