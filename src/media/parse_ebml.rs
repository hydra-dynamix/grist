use super::model::*;
use super::parse::{
    MediaParseError, PARSER, byte_locator, infer_media_type, limit_diagnostic,
    synthesized_media_locator, timed_locator, utf8_lossy,
};
use crate::core::{Diagnostic, OperationControl};
use std::collections::BTreeSet;
use std::fmt::Write as _;

#[derive(Debug)]
struct Element<'a> {
    id: u64,
    start: usize,
    payload_start: usize,
    end: usize,
    data: &'a [u8],
    children: Vec<Element<'a>>,
}
#[derive(Debug)]
struct Block<'a> {
    track: u64,
    time_ms: i64,
    bytes: &'a [u8],
    locator: crate::core::SourceLocator,
}

pub(crate) fn parse_matroska(
    bytes: &[u8],
    options: &MediaOptions,
    control: &OperationControl,
) -> Result<ParsedMedia, MediaParseError> {
    if !bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Err(MediaParseError::Malformed(
            "input is not an EBML/Matroska container".into(),
        ));
    }
    let mut count = 0;
    let roots = parse_elements(bytes, 0, bytes.len(), 0, options, control, &mut count)?;
    let header = roots
        .iter()
        .find(|element| element.id == 0x1a45dfa3)
        .ok_or_else(|| MediaParseError::Malformed("EBML header element is missing".into()))?;
    let doc_type = child(header, 0x4282)
        .map(|element| utf8_lossy(element.data).to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "matroska" | "webm"))
        .ok_or_else(|| {
            MediaParseError::Malformed("EBML DocType is missing or is not Matroska/WebM".into())
        })?;
    let segment = roots
        .iter()
        .find(|e| e.id == 0x18538067)
        .ok_or_else(|| MediaParseError::Malformed("Matroska Segment element is missing".into()))?;
    let mut out = ParsedMedia {
        technical: MediaTechnicalMetadata {
            byte_length: bytes.len() as u64,
            container_profile: Some(doc_type),
            ..Default::default()
        },
        complete: true,
        ..Default::default()
    };
    let info = find(segment, 0x1549a966);
    let scale = info
        .and_then(|e| child(e, 0x2ad7b1))
        .map(uint)
        .unwrap_or(1_000_000);
    out.technical.time_scale = Some(scale);
    out.technical.duration_ms = info
        .and_then(|e| child(e, 0x4489))
        .and_then(float)
        .map(|duration| (duration * scale as f64 / 1_000_000.0).max(0.0) as u64);
    if let Some(info) = info {
        for entry in &info.children {
            if matches!(entry.id, 0x7ba9 | 0x4d80 | 0x5741) {
                out.metadata.push(MediaMetadataEntry {
                    key: format!("ebml:0x{:x}", entry.id),
                    value: utf8_lossy(entry.data),
                    source: "matroska.info".into(),
                    locator: byte_locator(entry.start, entry.end),
                });
            }
        }
    }
    if let Some(tracks) = find(segment, 0x1654ae6b) {
        for entry in tracks.children.iter().filter(|e| e.id == 0xae) {
            parse_track(entry, &mut out)
        }
    }
    parse_chapters(segment, &mut out);
    let structural_bytes = count
        .checked_mul(std::mem::size_of::<Element<'_>>() as u64)
        .ok_or_else(|| MediaParseError::Malformed("EBML retained memory overflow".into()))?;
    let mut retained_bytes = parse_attachments(
        bytes.len() as u64,
        structural_bytes,
        segment,
        options,
        control,
        &mut out,
    )?;
    parse_tags(segment, &mut out);
    let subtitle_tracks = out
        .streams
        .iter()
        .filter(|stream| stream.kind == MediaStreamKind::Subtitle && !stream.encrypted)
        .filter_map(|stream| stream.container_track_number)
        .collect::<BTreeSet<_>>();
    let mut blocks = Vec::new();
    let mut subtitle_bytes = 0u64;
    collect_blocks(
        segment,
        scale,
        0,
        &subtitle_tracks,
        options,
        control,
        bytes.len() as u64,
        &mut subtitle_bytes,
        &mut retained_bytes,
        &mut blocks,
        &mut out,
    )?;
    route_subtitle_blocks(
        blocks,
        options,
        control,
        bytes.len() as u64,
        &mut retained_bytes,
        &mut out,
    )?;
    if out.streams.iter().any(|s| s.encrypted) {
        out.encrypted = true
    }
    Ok(out)
}

fn parse_elements<'a>(
    bytes: &'a [u8],
    start: usize,
    end: usize,
    depth: u64,
    options: &MediaOptions,
    control: &OperationControl,
    count: &mut u64,
) -> Result<Vec<Element<'a>>, MediaParseError> {
    if depth > options.max_nesting_depth {
        return Err(MediaParseError::Malformed(
            "EBML nesting exceeds max_nesting_depth".into(),
        ));
    }
    control.budget().observe_nesting_depth(depth)?;
    let mut out = Vec::new();
    let mut cursor = start;
    while cursor < end {
        control.checkpoint()?;
        let (id, id_len) = read_vint(bytes, cursor, true).ok_or_else(|| {
            MediaParseError::Malformed(format!("invalid EBML element ID at byte {cursor}"))
        })?;
        let (size, size_len) = read_vint(bytes, cursor + id_len, false).ok_or_else(|| {
            MediaParseError::Malformed(format!("invalid EBML size at byte {cursor}"))
        })?;
        let data_start = cursor
            .checked_add(id_len)
            .and_then(|value| value.checked_add(size_len))
            .filter(|value| *value <= end)
            .ok_or_else(|| MediaParseError::Malformed("EBML header range overflow".into()))?;
        let available = end - data_start;
        let size = if size == u64::MAX {
            available
        } else {
            usize::try_from(size).map_err(|_| {
                MediaParseError::Malformed("EBML element size is not addressable".into())
            })?
        };
        let element_end = data_start
            .checked_add(size)
            .filter(|v| *v <= end)
            .ok_or_else(|| {
                MediaParseError::Malformed(format!("EBML element 0x{id:x} exceeds parent bounds"))
            })?;
        *count = count
            .checked_add(1)
            .ok_or_else(|| MediaParseError::Malformed("EBML element count overflow".into()))?;
        if *count > options.max_boxes {
            return Err(MediaParseError::Malformed(
                "EBML element count exceeds max_boxes".into(),
            ));
        }
        let retained = count
            .checked_mul(2)
            .and_then(|value| value.checked_mul(std::mem::size_of::<Element<'_>>() as u64))
            .and_then(|value| value.checked_add(bytes.len() as u64))
            .ok_or_else(|| MediaParseError::Malformed("EBML memory estimate overflow".into()))?;
        control.budget().observe_memory_bytes(retained)?;
        out.reserve_exact(1);
        let children = if is_master(id) {
            parse_elements(
                bytes,
                data_start,
                element_end,
                depth + 1,
                options,
                control,
                count,
            )?
        } else {
            Vec::new()
        };
        out.push(Element {
            id,
            start: cursor,
            payload_start: data_start,
            end: element_end,
            data: &bytes[data_start..element_end],
            children,
        });
        cursor = element_end;
        if size == available && size as u64 == u64::MAX {
            break;
        }
    }
    Ok(out)
}
fn is_master(id: u64) -> bool {
    matches!(
        id,
        0x1a45dfa3
            | 0x18538067
            | 0x1549a966
            | 0x1654ae6b
            | 0xae
            | 0xe1
            | 0xe0
            | 0x6d80
            | 0x6240
            | 0x5035
            | 0x1043a770
            | 0x45b9
            | 0xb6
            | 0x80
            | 0x1941a469
            | 0x61a7
            | 0x1254c367
            | 0x7373
            | 0x67c8
            | 0x1f43b675
            | 0xa0
    )
}

fn parse_track(entry: &Element<'_>, out: &mut ParsedMedia) {
    let number = child(entry, 0xd7)
        .map(uint)
        .unwrap_or((out.streams.len() + 1) as u64);
    let uid = child(entry, 0x73c5).map(uint);
    let native = uid.unwrap_or(number).to_string();
    let kind = match child(entry, 0x83).map(uint) {
        Some(1) => MediaStreamKind::Video,
        Some(2) => MediaStreamKind::Audio,
        Some(0x11) => MediaStreamKind::Subtitle,
        Some(0x12) => MediaStreamKind::Data,
        _ => MediaStreamKind::Unknown,
    };
    let codec = child(entry, 0x86)
        .map(|e| utf8_lossy(e.data))
        .unwrap_or_else(|| "unknown".into());
    let encrypted = find(entry, 0x5035).is_some();
    let inspection = if encrypted {
        CodecInspectionStatus::Encrypted
    } else if known_codec(&codec) {
        CodecInspectionStatus::MetadataOnly
    } else {
        CodecInspectionStatus::Unsupported
    };
    let audio = child(entry, 0xe1);
    let video = child(entry, 0xe0);
    let base_id = format!("stream:track:{native}");
    let id = if out.streams.iter().any(|stream| stream.id == base_id) {
        format!("{base_id}:at:{}", entry.start)
    } else {
        base_id
    };
    let stream = MediaStream {
        id,
        index: out.streams.len() as u64,
        native_id: Some(native),
        container_track_number: Some(number),
        kind,
        codec,
        inspection,
        language: child(entry, 0x22b59c).map(|e| utf8_lossy(e.data)),
        name: child(entry, 0x536e).map(|e| utf8_lossy(e.data)),
        duration_ms: None,
        sample_rate: audio
            .and_then(|e| child(e, 0xb5))
            .and_then(float)
            .map(|v| v as u64),
        channels: audio.and_then(|e| child(e, 0x9f)).map(uint),
        width: video.and_then(|e| child(e, 0xb0)).map(uint),
        height: video.and_then(|e| child(e, 0xba)).map(uint),
        encrypted,
        locator: byte_locator(entry.start, entry.end),
    };
    out.streams.push(stream);
}
fn known_codec(codec: &str) -> bool {
    matches!(
        codec,
        "A_AAC"
            | "A_MPEG/L3"
            | "A_OPUS"
            | "A_VORBIS"
            | "A_FLAC"
            | "V_MPEG4/ISO/AVC"
            | "V_MPEGH/ISO/HEVC"
            | "V_VP8"
            | "V_VP9"
            | "V_AV1"
            | "S_TEXT/UTF8"
            | "S_TEXT/WEBVTT"
    )
}

fn parse_chapters(segment: &Element<'_>, out: &mut ParsedMedia) {
    if let Some(chapters) = find(segment, 0x1043a770) {
        for atom in find_all(chapters, 0xb6) {
            let start = child(atom, 0x91).map(uint).unwrap_or(0) / 1_000_000;
            let end = child(atom, 0x92).map(uint).map(|v| v / 1_000_000);
            let uid = child(atom, 0x73c4).map(uint);
            let title = find(atom, 0x85).map(|e| utf8_lossy(e.data));
            let native = uid.map(|v| v.to_string());
            let id = native
                .as_deref()
                .map(|v| format!("chapter:{v}"))
                .unwrap_or_else(|| format!("chapter:{}:{start}", out.chapters.len()));
            out.chapters.push(MediaChapter {
                id,
                index: out.chapters.len() as u64,
                native_id: native,
                start_ms: start,
                end_ms: end,
                title,
                locator: timed_locator(atom.start, atom.end, start, end.unwrap_or(start), None),
            });
        }
    }
}
fn parse_attachments(
    input_bytes: u64,
    retained_bytes: u64,
    segment: &Element<'_>,
    options: &MediaOptions,
    control: &OperationControl,
    out: &mut ParsedMedia,
) -> Result<u64, MediaParseError> {
    let mut retained = retained_bytes;
    let mut embedded = 0u64;
    if let Some(attachments) = find(segment, 0x1941a469) {
        for file in attachments.children.iter().filter(|e| e.id == 0x61a7) {
            control.checkpoint()?;
            let Some(data) = child(file, 0x465c) else {
                continue;
            };
            let filename = child(file, 0x466e).map(|e| utf8_lossy(e.data));
            let media = child(file, 0x4660)
                .map(|e| utf8_lossy(e.data))
                .or_else(|| infer_media_type(filename.as_deref(), data.data));
            let uid = child(file, 0x46ae).map(uint).map(|v| v.to_string());
            let data_len = data.data.len() as u64;
            let next_embedded = embedded.checked_add(data_len).ok_or_else(|| {
                MediaParseError::Malformed("embedded attachment byte count overflow".into())
            })?;
            let over_limit = next_embedded > options.max_attachment_bytes;
            let inventory = over_limit.then(|| (data_len, crate::core::sha256_hex(data.data)));
            if over_limit {
                out.complete = false;
                out.diagnostics.push(limit_diagnostic(
                    "media.attachment.limit",
                    "embedded attachments exceed max_attachment_bytes; retained as inventory only",
                    &byte_locator(data.payload_start, data.end),
                ));
            } else {
                let next_retained = retained.checked_add(data_len).ok_or_else(|| {
                    MediaParseError::Malformed("embedded attachment memory overflow".into())
                })?;
                let memory = input_bytes.checked_add(next_retained).ok_or_else(|| {
                    MediaParseError::Malformed("embedded attachment memory overflow".into())
                })?;
                control.budget().observe_memory_bytes(memory)?;
                retained = next_retained;
                embedded = next_embedded;
            }
            let retained_bytes = if over_limit {
                Vec::new()
            } else {
                data.data.to_vec()
            };
            if media.as_deref().is_some_and(|v| v.starts_with("image/")) {
                out.artwork_candidates.push(ArtworkCandidate {
                    picture_type: Some("attachment".into()),
                    description: filename,
                    media_type: media,
                    bytes: retained_bytes,
                    locator: byte_locator(data.payload_start, data.end),
                    image: None,
                    inventory,
                });
            } else {
                out.attachment_candidates.push(EmbeddedCandidate {
                    native_id: uid,
                    filename,
                    media_type: media,
                    bytes: retained_bytes,
                    locator: byte_locator(data.payload_start, data.end),
                    parsed_subtitle: None,
                    parsed_image: None,
                    inventory,
                });
            }
        }
    }
    Ok(retained)
}
fn parse_tags(segment: &Element<'_>, out: &mut ParsedMedia) {
    if let Some(tags) = find(segment, 0x1254c367) {
        for simple in find_all(tags, 0x67c8) {
            if let (Some(name), Some(value)) = (child(simple, 0x45a3), child(simple, 0x4487)) {
                out.metadata.push(MediaMetadataEntry {
                    key: utf8_lossy(name.data),
                    value: utf8_lossy(value.data),
                    source: "matroska.tag".into(),
                    locator: byte_locator(simple.start, simple.end),
                });
            }
        }
    }
}

fn collect_blocks<'a>(
    element: &Element<'a>,
    scale: u64,
    cluster_time: u64,
    subtitle_tracks: &BTreeSet<u64>,
    options: &MediaOptions,
    control: &OperationControl,
    input_bytes: u64,
    subtitle_bytes: &mut u64,
    retained_bytes: &mut u64,
    out: &mut Vec<Block<'a>>,
    parsed: &mut ParsedMedia,
) -> Result<(), MediaParseError> {
    control.checkpoint()?;
    let cluster_time = if element.id == 0x1f43b675 {
        child(element, 0xe7).map(uint).unwrap_or(0)
    } else {
        cluster_time
    };
    if element.id == 0xa3 || element.id == 0xa1 {
        let track = read_vint(element.data, 0, false).map(|(track, _)| track);
        if track.is_some_and(|track| subtitle_tracks.contains(&track)) {
            if let Some(block) = parse_block(element, scale, cluster_time)? {
                let next = subtitle_bytes
                    .checked_add(block.bytes.len() as u64)
                    .ok_or_else(|| {
                        MediaParseError::Malformed("subtitle block byte count overflow".into())
                    })?;
                if next > options.max_subtitle_bytes {
                    parsed.complete = false;
                    parsed.diagnostics.push(limit_diagnostic(
                        "media.subtitle.limit",
                        "Matroska subtitle blocks exceed max_subtitle_bytes",
                        &block.locator,
                    ));
                } else {
                    let next_retained = retained_bytes
                        .checked_add(std::mem::size_of::<Block<'_>>() as u64)
                        .ok_or_else(|| {
                            MediaParseError::Malformed("subtitle block table overflow".into())
                        })?;
                    let memory = input_bytes.checked_add(next_retained).ok_or_else(|| {
                        MediaParseError::Malformed("subtitle block memory overflow".into())
                    })?;
                    control.budget().observe_memory_bytes(memory)?;
                    *retained_bytes = next_retained;
                    *subtitle_bytes = next;
                    out.push(block);
                }
            } else {
                parsed.complete = false;
                parsed.diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "media.matroska.block_unsupported",
                        "malformed or laced subtitle block was inventoried but not decoded",
                    )
                    .partial()
                    .with_locator(byte_locator(element.start, element.end)),
                );
            }
        }
    }
    for child in &element.children {
        collect_blocks(
            child,
            scale,
            cluster_time,
            subtitle_tracks,
            options,
            control,
            input_bytes,
            subtitle_bytes,
            retained_bytes,
            out,
            parsed,
        )?;
    }
    Ok(())
}
fn parse_block<'a>(
    element: &Element<'a>,
    scale: u64,
    cluster_time: u64,
) -> Result<Option<Block<'a>>, MediaParseError> {
    let Some((track, n)) = read_vint(element.data, 0, false) else {
        return Ok(None);
    };
    let Some(data) = element.data.get(n..) else {
        return Ok(None);
    };
    if data.len() < 3 || data[2] & 0x06 != 0 {
        return Ok(None);
    }
    let relative = i16::from_be_bytes([data[0], data[1]]) as i64;
    let base_ticks = i64::try_from(cluster_time)
        .map_err(|_| MediaParseError::Malformed("Matroska cluster time exceeds i64".into()))?;
    let ticks = base_ticks
        .checked_add(relative)
        .ok_or_else(|| MediaParseError::Malformed("Matroska block time overflow".into()))?
        .max(0) as u64;
    let time_ms = ticks
        .checked_mul(scale)
        .ok_or_else(|| MediaParseError::Malformed("Matroska block time overflow".into()))?
        / 1_000_000;
    Ok(Some(Block {
        track,
        time_ms: i64::try_from(time_ms)
            .map_err(|_| MediaParseError::Malformed("Matroska block time exceeds i64".into()))?,
        bytes: &data[3..],
        locator: byte_locator(element.payload_start + n + 3, element.end),
    }))
}
fn route_subtitle_blocks(
    blocks: Vec<Block<'_>>,
    options: &MediaOptions,
    control: &OperationControl,
    input_bytes: u64,
    retained_bytes: &mut u64,
    out: &mut ParsedMedia,
) -> Result<(), MediaParseError> {
    for stream in out
        .streams
        .iter()
        .filter(|s| s.kind == MediaStreamKind::Subtitle && !s.encrypted)
    {
        let number = stream.container_track_number.unwrap_or(stream.index + 1);
        let selected_count = blocks.iter().filter(|block| block.track == number).count();
        if selected_count == 0 {
            continue;
        }
        let reference_bytes = (selected_count as u64)
            .checked_mul(std::mem::size_of::<&Block<'_>>() as u64)
            .ok_or_else(|| {
                MediaParseError::Malformed("Matroska subtitle reference table overflow".into())
            })?;
        let peak = input_bytes
            .checked_add(*retained_bytes)
            .and_then(|value| value.checked_add(reference_bytes))
            .ok_or_else(|| {
                MediaParseError::Malformed("Matroska subtitle reference memory overflow".into())
            })?;
        control.budget().observe_memory_bytes(peak)?;
        let mut selected = blocks
            .iter()
            .filter(|b| b.track == number)
            .collect::<Vec<_>>();
        selected.sort_by_key(|b| b.time_ms);
        let mut text = String::new();
        if stream.codec == "S_TEXT/WEBVTT" {
            control.budget().observe_memory_bytes(
                input_bytes
                    .checked_add(*retained_bytes)
                    .and_then(|value| value.checked_add(reference_bytes + 8))
                    .ok_or_else(|| {
                        MediaParseError::Malformed("Matroska subtitle text memory overflow".into())
                    })?,
            )?;
            text.push_str("WEBVTT\n\n");
        }
        for index in 0..selected.len() {
            control.checkpoint()?;
            let start = selected[index].time_ms.max(0) as u64;
            let end = selected
                .get(index + 1)
                .map(|b| b.time_ms.max(start as i64) as u64)
                .unwrap_or(start.checked_add(2_000).ok_or_else(|| {
                    MediaParseError::Malformed("Matroska subtitle end time overflow".into())
                })?);
            let decoded_bound = selected[index].bytes.len().checked_mul(3).ok_or_else(|| {
                MediaParseError::Malformed("Matroska subtitle decode size overflow".into())
            })?;
            let projected = text
                .len()
                .checked_add(decoded_bound)
                .and_then(|value| value.checked_add(96))
                .ok_or_else(|| {
                    MediaParseError::Malformed("Matroska subtitle projection size overflow".into())
                })?;
            if projected as u64 > options.max_subtitle_bytes {
                out.complete = false;
                out.diagnostics.push(limit_diagnostic(
                    "media.subtitle.limit",
                    "Matroska subtitle projection exceeds max_subtitle_bytes",
                    &selected[index].locator,
                ));
                break;
            }
            let peak = input_bytes
                .checked_add(*retained_bytes)
                .and_then(|value| value.checked_add(reference_bytes))
                .and_then(|value| value.checked_add(projected as u64))
                .ok_or_else(|| {
                    MediaParseError::Malformed("Matroska subtitle text memory overflow".into())
                })?;
            control.budget().observe_memory_bytes(peak)?;
            let value = utf8_lossy(selected[index].bytes);
            if stream.codec == "S_TEXT/WEBVTT" {
                write!(
                    text,
                    "{}\n{} --> {}\n{}\n\n",
                    index + 1,
                    vtt_time(start),
                    vtt_time(end),
                    value
                )
                .expect("writing to String cannot fail");
            } else if stream.codec == "S_TEXT/UTF8" {
                write!(
                    text,
                    "{}\n{} --> {}\n{}\n\n",
                    index + 1,
                    srt_time(start),
                    srt_time(end),
                    value
                )
                .expect("writing to String cannot fail");
            }
        }
        if !text.is_empty() {
            let subtitle_end = selected
                .last()
                .map(|block| block.time_ms.max(0) as u64)
                .unwrap_or(0)
                .checked_add(2_000)
                .ok_or_else(|| {
                    MediaParseError::Malformed("Matroska subtitle locator time overflow".into())
                })?;
            *retained_bytes = retained_bytes
                .checked_add(text.len() as u64)
                .ok_or_else(|| {
                    MediaParseError::Malformed("Matroska retained subtitle overflow".into())
                })?;
            out.subtitle_candidates.push(SubtitleCandidate {
                stream_id: stream.id.clone(),
                codec: stream.codec.clone(),
                language: stream.language.clone(),
                bytes: text.into_bytes(),
                locator: synthesized_media_locator(
                    selected[0].time_ms.max(0) as u64,
                    subtitle_end,
                    Some(stream.index),
                ),
                document: None,
                inventory: None,
            });
        }
    }
    Ok(())
}

fn read_vint(bytes: &[u8], offset: usize, id: bool) -> Option<(u64, usize)> {
    let first = *bytes.get(offset)?;
    let leading = first.leading_zeros() as usize;
    if leading >= 8 {
        return None;
    }
    let len = leading + 1;
    if id && len > 4 || !id && len > 8 {
        return None;
    }
    let mut value = if id {
        u64::from(first)
    } else {
        u64::from(first & ((1u8 << (8 - len)) - 1))
    };
    for byte in bytes.get(offset + 1..offset + len)? {
        value = (value << 8) | u64::from(*byte)
    }
    if !id && value == (1u64 << (7 * len)) - 1 {
        Some((u64::MAX, len))
    } else {
        Some((value, len))
    }
}
fn uint(e: &Element<'_>) -> u64 {
    e.data.iter().fold(0, |v, b| (v << 8) | u64::from(*b))
}
fn float(e: &Element<'_>) -> Option<f64> {
    match e.data.len() {
        4 => Some(f32::from_bits(u32::from_be_bytes(e.data.try_into().ok()?)) as f64),
        8 => Some(f64::from_bits(u64::from_be_bytes(e.data.try_into().ok()?))),
        _ => None,
    }
}
fn child<'a>(e: &'a Element<'a>, id: u64) -> Option<&'a Element<'a>> {
    e.children.iter().find(|v| v.id == id)
}
fn find<'a>(e: &'a Element<'a>, id: u64) -> Option<&'a Element<'a>> {
    if e.id == id {
        return Some(e);
    }
    for c in &e.children {
        if let Some(v) = find(c, id) {
            return Some(v);
        }
    }
    None
}
fn find_all<'a>(e: &'a Element<'a>, id: u64) -> Vec<&'a Element<'a>> {
    let mut out = Vec::new();
    if e.id == id {
        out.push(e)
    }
    for c in &e.children {
        out.extend(find_all(c, id))
    }
    out
}
fn vtt_time(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}
fn srt_time(ms: u64) -> String {
    vtt_time(ms).replace('.', ",")
}
