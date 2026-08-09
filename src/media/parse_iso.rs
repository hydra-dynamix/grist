use super::model::*;
use super::parse::{
    MediaParseError, PARSER, byte_locator, limit_diagnostic, synthesized_media_locator,
    timed_locator, utf8_lossy,
};
use crate::core::{Diagnostic, OperationControl};
use std::fmt::Write as _;

#[derive(Debug)]
struct BoxNode<'a> {
    kind: [u8; 4],
    start: usize,
    payload_start: usize,
    end: usize,
    payload: &'a [u8],
    children: Vec<BoxNode<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IsoFamily {
    Heif,
    QuickTime,
    Mp4,
}

fn iso_family(ftyp: &[u8]) -> Option<IsoFamily> {
    if ftyp.len() < 8 {
        return None;
    }
    let brands = std::iter::once(&ftyp[..4]).chain(ftyp[8..].chunks_exact(4));
    let mut family = None;
    for brand in brands {
        if matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis"
        ) {
            return Some(IsoFamily::Heif);
        }
        if brand == b"qt  " {
            family = Some(IsoFamily::QuickTime);
        } else if family.is_none()
            && matches!(
                brand,
                b"isom"
                    | b"iso2"
                    | b"iso3"
                    | b"iso4"
                    | b"iso5"
                    | b"iso6"
                    | b"mp41"
                    | b"mp42"
                    | b"avc1"
                    | b"dash"
                    | b"M4A "
                    | b"M4B "
                    | b"M4P "
                    | b"M4V "
            )
        {
            family = Some(IsoFamily::Mp4);
        }
    }
    family
}

pub(crate) fn parse_iso_bmff(
    bytes: &[u8],
    requested: MediaFormat,
    options: &MediaOptions,
    control: &OperationControl,
) -> Result<ParsedMedia, MediaParseError> {
    if bytes.len() < 8 {
        return Err(MediaParseError::Malformed(
            "truncated ISO BMFF header".into(),
        ));
    }
    let mut count = 0u64;
    let roots = parse_boxes(
        bytes,
        0,
        bytes.len(),
        0,
        false,
        options,
        control,
        &mut count,
    )?;
    let ftyp = roots
        .iter()
        .find(|node| &node.kind == b"ftyp")
        .ok_or_else(|| MediaParseError::Malformed("ISO BMFF container has no ftyp box".into()))?;
    let major = utf8_lossy(ftyp.payload.get(..4).unwrap_or_default());
    let actual = match iso_family(ftyp.payload) {
        Some(IsoFamily::QuickTime) => MediaFormat::QuickTime,
        Some(IsoFamily::Mp4) => MediaFormat::Mp4,
        Some(IsoFamily::Heif) => {
            return Err(MediaParseError::Malformed(
                "HEIF/HEIC/AVIF brand belongs to the image parser".into(),
            ));
        }
        None => {
            return Err(MediaParseError::Malformed(
                "ISO BMFF ftyp has no supported MP4/QuickTime brand".into(),
            ));
        }
    };
    if requested != actual {
        return Err(MediaParseError::Malformed(
            "selected QuickTime parser does not match MP4 brand".into(),
        ));
    }
    let mut out = ParsedMedia {
        technical: MediaTechnicalMetadata {
            byte_length: bytes.len() as u64,
            container_profile: Some(major),
            ..Default::default()
        },
        complete: true,
        ..Default::default()
    };
    let mut retained_bytes = count
        .checked_mul(std::mem::size_of::<BoxNode<'_>>() as u64)
        .ok_or_else(|| MediaParseError::Malformed("ISO BMFF retained memory overflow".into()))?;
    if let Some(moov) = roots.iter().find(|node| &node.kind == b"moov") {
        if let Some(mvhd) = find(moov, b"mvhd") {
            let (scale, duration) = fullbox_time(mvhd.payload);
            out.technical.time_scale = scale;
            out.technical.duration_ms = duration
                .zip(scale)
                .filter(|(_, s)| *s > 0)
                .and_then(|(d, s)| d.checked_mul(1000).map(|value| value / s));
        }
        for trak in moov.children.iter().filter(|node| &node.kind == b"trak") {
            parse_track(bytes, trak, &mut out, options, control, &mut retained_bytes)?;
        }
        parse_chapters(moov, &mut out);
        parse_ilst(
            bytes.len() as u64,
            moov,
            &mut out,
            options,
            control,
            &mut retained_bytes,
        )?;
    } else {
        out.complete = false;
        out.diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "media.mp4.moov_missing",
                "ISO BMFF container has no moov metadata box",
            )
            .partial(),
        );
    }
    if out.streams.iter().any(|stream| stream.encrypted) {
        out.encrypted = true;
    }
    Ok(out)
}

fn parse_boxes<'a>(
    bytes: &'a [u8],
    start: usize,
    end: usize,
    depth: u64,
    parent_ilst: bool,
    options: &MediaOptions,
    control: &OperationControl,
    count: &mut u64,
) -> Result<Vec<BoxNode<'a>>, MediaParseError> {
    if depth > options.max_nesting_depth {
        return Err(MediaParseError::Malformed(
            "ISO BMFF nesting exceeds max_nesting_depth".into(),
        ));
    }
    control.budget().observe_nesting_depth(depth)?;
    let mut nodes = Vec::new();
    let mut cursor = start;
    while cursor < end {
        control.checkpoint()?;
        if end - cursor < 8 {
            return Err(MediaParseError::Malformed(format!(
                "truncated ISO BMFF box header at byte {cursor}"
            )));
        }
        let size32 = be_u32(&bytes[cursor..cursor + 4]) as u64;
        let kind: [u8; 4] = bytes[cursor + 4..cursor + 8]
            .try_into()
            .expect("four bytes");
        let (size, header) = if size32 == 1 {
            if end - cursor < 16 {
                return Err(MediaParseError::Malformed(
                    "truncated extended ISO BMFF box size".into(),
                ));
            }
            (be_u64(&bytes[cursor + 8..cursor + 16]), 16usize)
        } else if size32 == 0 {
            ((end - cursor) as u64, 8)
        } else {
            (size32, 8)
        };
        if size < header as u64 {
            return Err(MediaParseError::Malformed(format!(
                "invalid {} box size",
                fourcc(kind)
            )));
        }
        let addressable_size = usize::try_from(size).map_err(|_| {
            MediaParseError::Malformed(format!("{} box size is not addressable", fourcc(kind)))
        })?;
        let box_end = cursor
            .checked_add(addressable_size)
            .filter(|value| *value <= end)
            .ok_or_else(|| {
                MediaParseError::Malformed(format!("{} box exceeds parent bounds", fourcc(kind)))
            })?;
        *count = count
            .checked_add(1)
            .ok_or_else(|| MediaParseError::Malformed("ISO BMFF box count overflow".into()))?;
        if *count > options.max_boxes {
            return Err(MediaParseError::Malformed(
                "ISO BMFF box count exceeds max_boxes".into(),
            ));
        }
        let retained = count
            .checked_mul(2)
            .and_then(|value| value.checked_mul(std::mem::size_of::<BoxNode<'_>>() as u64))
            .and_then(|value| value.checked_add(bytes.len() as u64))
            .ok_or_else(|| {
                MediaParseError::Malformed("ISO BMFF memory estimate overflow".into())
            })?;
        control.budget().observe_memory_bytes(retained)?;
        nodes.reserve_exact(1);
        let mut payload_start = cursor + header;
        if &kind == b"meta" {
            payload_start = payload_start
                .checked_add(4)
                .filter(|value| *value <= box_end)
                .ok_or_else(|| {
                    MediaParseError::Malformed("truncated ISO BMFF meta full-box header".into())
                })?;
        }
        let container = is_container(kind) || parent_ilst;
        let children = if container {
            parse_boxes(
                bytes,
                payload_start,
                box_end,
                depth + 1,
                &kind == b"ilst",
                options,
                control,
                count,
            )?
        } else {
            Vec::new()
        };
        nodes.push(BoxNode {
            kind,
            start: cursor,
            payload_start,
            end: box_end,
            payload: &bytes[payload_start..box_end],
            children,
        });
        cursor = box_end;
        if size32 == 0 {
            break;
        }
    }
    Ok(nodes)
}

fn is_container(kind: [u8; 4]) -> bool {
    matches!(
        &kind,
        b"moov"
            | b"trak"
            | b"mdia"
            | b"minf"
            | b"stbl"
            | b"edts"
            | b"udta"
            | b"meta"
            | b"ilst"
            | b"moof"
            | b"traf"
            | b"mvex"
            | b"dinf"
    )
}

fn parse_track(
    bytes: &[u8],
    trak: &BoxNode<'_>,
    out: &mut ParsedMedia,
    options: &MediaOptions,
    control: &OperationControl,
    retained_bytes: &mut u64,
) -> Result<(), MediaParseError> {
    let index = out.streams.len() as u64;
    let tkhd = find(trak, b"tkhd");
    let native_id = tkhd
        .and_then(|node| track_id(node.payload))
        .map(|id| id.to_string());
    let base_id = native_id
        .as_deref()
        .map(|value| format!("stream:track:{value}"))
        .unwrap_or_else(|| format!("stream:at:{}", trak.start));
    let id = if out.streams.iter().any(|stream| stream.id == base_id) {
        format!("{base_id}:at:{}", trak.start)
    } else {
        base_id
    };
    let mdia = trak.children.iter().find(|node| &node.kind == b"mdia");
    let handler = mdia
        .and_then(|node| find(node, b"hdlr"))
        .and_then(|node| node.payload.get(8..12));
    let kind = match handler {
        Some(b"soun") => MediaStreamKind::Audio,
        Some(b"vide") => MediaStreamKind::Video,
        Some(b"text" | b"sbtl" | b"subt" | b"clcp") => MediaStreamKind::Subtitle,
        Some(b"meta") => MediaStreamKind::Data,
        _ => MediaStreamKind::Unknown,
    };
    let mdhd = mdia.and_then(|node| find(node, b"mdhd"));
    let (scale, duration) = mdhd
        .map(|node| fullbox_time(node.payload))
        .unwrap_or((None, None));
    let duration_ms = duration
        .zip(scale)
        .filter(|(_, s)| *s > 0)
        .and_then(|(d, s)| d.checked_mul(1000).map(|value| value / s));
    let language = mdhd.and_then(|node| mdhd_language(node.payload));
    let stsd = mdia.and_then(|node| find(node, b"stsd"));
    let sample = stsd.and_then(|node| sample_entry(node));
    let codec = sample
        .as_ref()
        .map(|entry| fourcc(entry.kind))
        .unwrap_or_else(|| "unknown".into());
    let encrypted = matches!(
        sample.as_ref().map(|entry| &entry.kind),
        Some(b"enca" | b"encv")
    );
    let (channels, sample_rate, width, height) = sample
        .as_ref()
        .map(sample_dimensions)
        .unwrap_or((None, None, None, None));
    let inspection = if encrypted {
        CodecInspectionStatus::Encrypted
    } else if known_codec(&codec) {
        CodecInspectionStatus::MetadataOnly
    } else {
        CodecInspectionStatus::Unsupported
    };
    let stream = MediaStream {
        id: id.clone(),
        index,
        native_id,
        container_track_number: None,
        kind,
        codec: codec.clone(),
        inspection,
        language: language.clone(),
        name: None,
        duration_ms,
        sample_rate,
        channels,
        width,
        height,
        encrypted,
        locator: byte_locator(trak.start, trak.end),
    };
    if kind == MediaStreamKind::Subtitle && !encrypted {
        if let Some(candidate) = extract_subtitle(
            bytes,
            mdia,
            &stream,
            scale,
            options,
            control,
            retained_bytes,
            out,
        )? {
            out.subtitle_candidates.push(candidate);
        }
    }
    out.streams.push(stream);
    Ok(())
}

#[derive(Debug)]
struct SampleEntry<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
}
fn sample_entry<'a>(stsd: &'a BoxNode<'a>) -> Option<SampleEntry<'a>> {
    let data = stsd.payload;
    if data.len() < 16 {
        return None;
    }
    let size = be_u32(&data[8..12]) as usize;
    if size < 8 || 8 + size > data.len() {
        return None;
    }
    Some(SampleEntry {
        kind: data[12..16].try_into().ok()?,
        payload: &data[16..8 + size],
    })
}
fn sample_dimensions(
    entry: &SampleEntry<'_>,
) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
    match &entry.kind {
        b"mp4a" | b"alac" | b"enca" | b"ac-3" | b"ec-3" if entry.payload.len() >= 28 => (
            Some(be_u16(&entry.payload[16..18]) as u64),
            Some((be_u32(&entry.payload[24..28]) >> 16) as u64),
            None,
            None,
        ),
        b"avc1" | b"hvc1" | b"hev1" | b"vp09" | b"av01" | b"encv" if entry.payload.len() >= 28 => (
            None,
            None,
            Some(be_u16(&entry.payload[24..26]) as u64),
            Some(be_u16(&entry.payload[26..28]) as u64),
        ),
        _ => (None, None, None, None),
    }
}

fn extract_subtitle(
    bytes: &[u8],
    mdia: Option<&BoxNode<'_>>,
    stream: &MediaStream,
    scale: Option<u64>,
    options: &MediaOptions,
    control: &OperationControl,
    retained_bytes: &mut u64,
    out: &mut ParsedMedia,
) -> Result<Option<SubtitleCandidate>, MediaParseError> {
    let Some(mdia) = mdia else {
        unsupported_subtitle_layout(out, stream, "MP4 subtitle track has no mdia box");
        return Ok(None);
    };
    let Some(stsz) = find(mdia, b"stsz") else {
        unsupported_subtitle_layout(
            out,
            stream,
            "MP4 subtitle track has no stsz sample-size table",
        );
        return Ok(None);
    };
    let mut working_bytes = *retained_bytes;
    let Some(sizes) = parse_stsz(
        stsz,
        options.max_subtitle_bytes,
        bytes.len() as u64,
        working_bytes,
        control,
    )?
    else {
        out.complete = false;
        out.diagnostics.push(limit_diagnostic(
            "media.subtitle.limit",
            "MP4 subtitle sample table exceeds max_subtitle_bytes",
            &stream.locator,
        ));
        return Ok(None);
    };
    if sizes.is_empty() {
        return Ok(None);
    }
    let size_table_bytes = (sizes.len() as u64)
        .checked_mul(std::mem::size_of::<usize>() as u64)
        .ok_or_else(|| MediaParseError::Malformed("MP4 stsz memory overflow".into()))?;
    working_bytes = working_bytes
        .checked_add(size_table_bytes)
        .ok_or_else(|| MediaParseError::Malformed("MP4 subtitle working memory overflow".into()))?;
    let offsets = if let Some(node) = find(mdia, b"stco") {
        parse_offsets(
            node,
            false,
            sizes.len(),
            bytes.len() as u64,
            working_bytes,
            control,
        )?
    } else if let Some(node) = find(mdia, b"co64") {
        parse_offsets(
            node,
            true,
            sizes.len(),
            bytes.len() as u64,
            working_bytes,
            control,
        )?
    } else {
        unsupported_subtitle_layout(
            out,
            stream,
            "MP4 subtitle track has no stco/co64 chunk-offset table",
        );
        Vec::new()
    };
    if offsets.is_empty() {
        return Ok(None);
    }
    if offsets.len() != 1 && offsets.len() != sizes.len() {
        out.complete = false;
        out.diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "media.subtitle.chunk_layout_unsupported",
                "MP4 subtitle chunk layout requires stsc mapping and was inventoried without decoding",
            )
            .partial()
            .with_locator(stream.locator.clone()),
        );
        return Ok(None);
    }
    let total = sizes.iter().try_fold(0usize, |total, size| {
        total.checked_add(*size).ok_or_else(|| {
            MediaParseError::Malformed("MP4 subtitle sample byte count overflow".into())
        })
    })?;
    let offset_table_bytes = (offsets.len() as u64)
        .checked_mul(std::mem::size_of::<u64>() as u64)
        .ok_or_else(|| MediaParseError::Malformed("MP4 chunk-offset memory overflow".into()))?;
    let projection_table_bytes = (sizes.len() as u64)
        .checked_mul((2 * std::mem::size_of::<usize>()) as u64)
        .ok_or_else(|| MediaParseError::Malformed("MP4 subtitle table memory overflow".into()))?;
    working_bytes = working_bytes
        .checked_add(offset_table_bytes)
        .and_then(|value| value.checked_add(projection_table_bytes))
        .ok_or_else(|| MediaParseError::Malformed("MP4 subtitle working memory overflow".into()))?;
    let memory = (bytes.len() as u64)
        .checked_add(total as u64)
        .and_then(|value| value.checked_add(working_bytes))
        .ok_or_else(|| {
            MediaParseError::Malformed("MP4 subtitle memory estimate overflow".into())
        })?;
    control.budget().observe_memory_bytes(memory)?;
    let mut samples = Vec::with_capacity(sizes.len());
    let mut sequential = usize::try_from(offsets[0]).map_err(|_| {
        MediaParseError::Malformed("MP4 subtitle chunk offset is not addressable".into())
    })?;
    for (index, size) in sizes.into_iter().enumerate() {
        control.checkpoint()?;
        let start = if offsets.len() == 1 {
            sequential
        } else {
            usize::try_from(offsets[index]).map_err(|_| {
                MediaParseError::Malformed("MP4 subtitle chunk offset is not addressable".into())
            })?
        };
        let end = start.checked_add(size).ok_or_else(|| {
            MediaParseError::Malformed("MP4 subtitle sample range overflow".into())
        })?;
        let sample = bytes.get(start..end).ok_or_else(|| {
            MediaParseError::Malformed("MP4 subtitle sample exceeds input".into())
        })?;
        samples.push(sample);
        sequential = end;
    }
    let codec = stream.codec.as_str();
    let timing_required = matches!(codec, "wvtt" | "tx3g" | "text");
    let stts = find(mdia, b"stts");
    if timing_required && stts.is_none() {
        unsupported_subtitle_layout(
            out,
            stream,
            "MP4 timed subtitle track has no stts timing table",
        );
        return Ok(None);
    }
    let durations = stts
        .map(|node| {
            parse_stts(
                node,
                samples.len(),
                bytes.len() as u64,
                working_bytes,
                control,
            )
        })
        .transpose()?
        .unwrap_or_default();
    let duration_table_bytes = (durations.len() as u64)
        .checked_mul(std::mem::size_of::<u64>() as u64)
        .ok_or_else(|| MediaParseError::Malformed("MP4 stts memory overflow".into()))?;
    working_bytes = working_bytes
        .checked_add(duration_table_bytes)
        .ok_or_else(|| MediaParseError::Malformed("MP4 subtitle working memory overflow".into()))?;
    let assembled = if codec == "wvtt" {
        control.budget().observe_memory_bytes(
            (bytes.len() as u64)
                .checked_add(working_bytes)
                .and_then(|value| value.checked_add(8))
                .ok_or_else(|| {
                    MediaParseError::Malformed("MP4 subtitle text memory overflow".into())
                })?,
        )?;
        let mut text = String::from("WEBVTT\n\n");
        let mut time = 0u64;
        for (index, sample) in samples.iter().enumerate() {
            let payload_bytes = extract_payl_bytes(sample).unwrap_or(sample);
            let decoded_bound = payload_bytes.len().checked_mul(3).ok_or_else(|| {
                MediaParseError::Malformed("MP4 subtitle decode size overflow".into())
            })?;
            observe_synthesized_text(
                bytes.len() as u64,
                working_bytes,
                text.len(),
                decoded_bound,
                options.max_subtitle_bytes,
                control,
            )?;
            let payload = utf8_lossy(payload_bytes);
            let ticks = durations.get(index).copied().unwrap_or(
                scale.unwrap_or(1000).checked_mul(2).ok_or_else(|| {
                    MediaParseError::Malformed("MP4 subtitle duration overflow".into())
                })?,
            );
            let unit = scale.unwrap_or(1000).max(1);
            let start = time.checked_mul(1000).ok_or_else(|| {
                MediaParseError::Malformed("MP4 subtitle start time overflow".into())
            })? / unit;
            time = time.checked_add(ticks).ok_or_else(|| {
                MediaParseError::Malformed("MP4 subtitle time accumulation overflow".into())
            })?;
            let end = time.checked_mul(1000).ok_or_else(|| {
                MediaParseError::Malformed("MP4 subtitle end time overflow".into())
            })? / unit;
            write!(
                text,
                "{}\n{} --> {}\n{}\n\n",
                index + 1,
                vtt_time(start),
                vtt_time(end),
                payload
            )
            .expect("writing to String cannot fail");
        }
        text.into_bytes()
    } else if codec == "stpp" {
        let projected = total
            .checked_add(samples.len().saturating_sub(1))
            .ok_or_else(|| MediaParseError::Malformed("MP4 stpp size overflow".into()))?;
        if projected as u64 > options.max_subtitle_bytes {
            out.complete = false;
            out.diagnostics.push(limit_diagnostic(
                "media.subtitle.limit",
                "MP4 stpp projection exceeds max_subtitle_bytes",
                &stream.locator,
            ));
            return Ok(None);
        }
        control.budget().observe_memory_bytes(
            (bytes.len() as u64)
                .checked_add(working_bytes)
                .and_then(|value| value.checked_add(projected as u64))
                .ok_or_else(|| MediaParseError::Malformed("MP4 stpp memory overflow".into()))?,
        )?;
        samples.iter().enumerate().fold(
            Vec::with_capacity(projected),
            |mut output, (index, sample)| {
                if index > 0 {
                    output.push(b'\n');
                }
                output.extend_from_slice(sample);
                output
            },
        )
    } else if codec == "tx3g" || codec == "text" {
        let mut text = String::new();
        let mut time = 0u64;
        let unit = scale.unwrap_or(1000).max(1);
        for (index, sample) in samples.iter().enumerate() {
            let len = sample.get(..2).map(be_u16).unwrap_or(0) as usize;
            let value_bytes = sample.get(2..2 + len).unwrap_or_default();
            let decoded_bound = value_bytes.len().checked_mul(3).ok_or_else(|| {
                MediaParseError::Malformed("MP4 subtitle decode size overflow".into())
            })?;
            observe_synthesized_text(
                bytes.len() as u64,
                working_bytes,
                text.len(),
                decoded_bound,
                options.max_subtitle_bytes,
                control,
            )?;
            let value = utf8_lossy(value_bytes);
            let start = time.checked_mul(1000).ok_or_else(|| {
                MediaParseError::Malformed("MP4 subtitle start time overflow".into())
            })? / unit;
            let fallback = unit.checked_mul(2).ok_or_else(|| {
                MediaParseError::Malformed("MP4 subtitle duration overflow".into())
            })?;
            time = time
                .checked_add(durations.get(index).copied().unwrap_or(fallback))
                .ok_or_else(|| {
                    MediaParseError::Malformed("MP4 subtitle time accumulation overflow".into())
                })?;
            let end = time.checked_mul(1000).ok_or_else(|| {
                MediaParseError::Malformed("MP4 subtitle end time overflow".into())
            })? / unit;
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
        text.into_bytes()
    } else {
        Vec::new()
    };
    let duration_ms = scale
        .filter(|value| *value > 0)
        .and_then(|unit| {
            durations
                .iter()
                .copied()
                .try_fold(0u64, u64::checked_add)
                .and_then(|ticks| ticks.checked_mul(1000).map(|value| value / unit))
        })
        .unwrap_or(0);
    *retained_bytes = retained_bytes
        .checked_add(assembled.len() as u64)
        .ok_or_else(|| MediaParseError::Malformed("MP4 retained subtitle overflow".into()))?;
    Ok(Some(SubtitleCandidate {
        stream_id: stream.id.clone(),
        codec: stream.codec.clone(),
        language: stream.language.clone(),
        bytes: assembled,
        locator: synthesized_media_locator(0, duration_ms, Some(stream.index)),
        document: None,
        inventory: None,
    }))
}

fn unsupported_subtitle_layout(out: &mut ParsedMedia, stream: &MediaStream, message: &str) {
    out.complete = false;
    out.diagnostics.push(
        Diagnostic::warning(PARSER, "media.subtitle.chunk_layout_unsupported", message)
            .partial()
            .with_locator(stream.locator.clone()),
    );
}

fn observe_synthesized_text(
    input_bytes: u64,
    retained_bytes: u64,
    current: usize,
    appended: usize,
    max_bytes: u64,
    control: &OperationControl,
) -> Result<(), MediaParseError> {
    let projected = current
        .checked_add(appended)
        .and_then(|value| value.checked_add(96))
        .ok_or_else(|| MediaParseError::Malformed("MP4 subtitle text size overflow".into()))?;
    if projected as u64 > max_bytes {
        return Err(MediaParseError::Malformed(
            "MP4 synthesized subtitle exceeds max_subtitle_bytes".into(),
        ));
    }
    let peak = input_bytes
        .checked_add(retained_bytes)
        .and_then(|value| value.checked_add(projected as u64))
        .ok_or_else(|| MediaParseError::Malformed("MP4 subtitle text memory overflow".into()))?;
    control.budget().observe_memory_bytes(peak)?;
    Ok(())
}

fn parse_chapters(moov: &BoxNode<'_>, out: &mut ParsedMedia) {
    for chpl in find_all(moov, b"chpl") {
        let data = chpl.payload;
        let mut cursor = if data.len() >= 5 { 5 } else { continue };
        let count = data[4] as usize;
        for _ in 0..count {
            if cursor + 9 > data.len() {
                break;
            }
            let start = be_u64(&data[cursor..cursor + 8]) / 10_000;
            let len = data[cursor + 8] as usize;
            cursor += 9;
            if cursor + len > data.len() {
                break;
            }
            let title = utf8_lossy(&data[cursor..cursor + len]);
            let index = out.chapters.len();
            out.chapters.push(MediaChapter {
                id: format!("chapter:chpl:{index}:{start}"),
                index: index as u64,
                native_id: None,
                start_ms: start,
                end_ms: None,
                title: Some(title),
                locator: timed_locator(chpl.start, chpl.end, start, start, None),
            });
            cursor += len;
        }
    }
}

fn parse_ilst(
    input_bytes: u64,
    moov: &BoxNode<'_>,
    out: &mut ParsedMedia,
    options: &MediaOptions,
    control: &OperationControl,
    retained_bytes: &mut u64,
) -> Result<(), MediaParseError> {
    let mut retained_artwork = 0u64;
    let mut retained_metadata = 0u64;
    for ilst in find_all(moov, b"ilst") {
        for atom in &ilst.children {
            for data in atom.children.iter().filter(|node| &node.kind == b"data") {
                control.checkpoint()?;
                let value = data.payload.get(8..).unwrap_or_default();
                if &atom.kind == b"covr" {
                    let next = retained_artwork
                        .checked_add(value.len() as u64)
                        .ok_or_else(|| {
                            MediaParseError::Malformed("MP4 artwork byte count overflow".into())
                        })?;
                    let over_limit = next > options.max_attachment_bytes;
                    let inventory =
                        over_limit.then(|| (value.len() as u64, crate::core::sha256_hex(value)));
                    if over_limit {
                        out.complete = false;
                        out.diagnostics.push(limit_diagnostic(
                            "media.artwork.limit",
                            "MP4 artwork exceeds max_attachment_bytes; retained as inventory only",
                            &byte_locator(data.payload_start + 8, data.end),
                        ));
                    } else {
                        retained_artwork = next;
                        *retained_bytes = retained_bytes
                            .checked_add(value.len() as u64)
                            .ok_or_else(|| {
                                MediaParseError::Malformed("MP4 artwork memory overflow".into())
                            })?;
                        let memory = input_bytes.checked_add(*retained_bytes).ok_or_else(|| {
                            MediaParseError::Malformed("MP4 artwork memory overflow".into())
                        })?;
                        control.budget().observe_memory_bytes(memory)?;
                    }
                    out.artwork_candidates.push(ArtworkCandidate {
                        picture_type: Some("cover".into()),
                        description: None,
                        media_type: super::parse::infer_media_type(None, value),
                        bytes: if over_limit {
                            Vec::new()
                        } else {
                            value.to_vec()
                        },
                        locator: byte_locator(data.payload_start + 8, data.end),
                        image: None,
                        inventory,
                    });
                } else {
                    let next = retained_metadata
                        .checked_add(value.len() as u64)
                        .ok_or_else(|| {
                            MediaParseError::Malformed("MP4 metadata byte count overflow".into())
                        })?;
                    if next > options.max_metadata_bytes {
                        out.complete = false;
                        out.diagnostics.push(limit_diagnostic(
                            "media.metadata.limit",
                            "MP4 metadata exceeds cumulative max_metadata_bytes",
                            &byte_locator(data.payload_start + 8, data.end),
                        ));
                        continue;
                    }
                    *retained_bytes =
                        retained_bytes
                            .checked_add(value.len() as u64)
                            .ok_or_else(|| {
                                MediaParseError::Malformed("MP4 metadata memory overflow".into())
                            })?;
                    let memory = input_bytes.checked_add(*retained_bytes).ok_or_else(|| {
                        MediaParseError::Malformed("MP4 metadata memory overflow".into())
                    })?;
                    control.budget().observe_memory_bytes(memory)?;
                    retained_metadata = next;
                    out.metadata.push(MediaMetadataEntry {
                        key: fourcc(atom.kind),
                        value: utf8_lossy(value),
                        source: "mp4.ilst".into(),
                        locator: byte_locator(data.payload_start + 8, data.end),
                    });
                }
            }
        }
    }
    Ok(())
}

fn find<'a>(node: &'a BoxNode<'a>, kind: &[u8; 4]) -> Option<&'a BoxNode<'a>> {
    if &node.kind == kind {
        return Some(node);
    }
    for child in &node.children {
        if let Some(value) = find(child, kind) {
            return Some(value);
        }
    }
    None
}
fn find_all<'a>(node: &'a BoxNode<'a>, kind: &[u8; 4]) -> Vec<&'a BoxNode<'a>> {
    let mut out = Vec::new();
    if &node.kind == kind {
        out.push(node)
    }
    for child in &node.children {
        out.extend(find_all(child, kind));
    }
    out
}
fn fullbox_time(data: &[u8]) -> (Option<u64>, Option<u64>) {
    let version = *data.first().unwrap_or(&0);
    if version == 1 && data.len() >= 32 {
        (
            Some(be_u32(&data[20..24]) as u64),
            Some(be_u64(&data[24..32])),
        )
    } else if data.len() >= 20 {
        (
            Some(be_u32(&data[12..16]) as u64),
            Some(be_u32(&data[16..20]) as u64),
        )
    } else {
        (None, None)
    }
}
fn track_id(data: &[u8]) -> Option<u32> {
    if *data.first()? == 1 {
        Some(be_u32(data.get(20..24)?))
    } else {
        Some(be_u32(data.get(12..16)?))
    }
}
fn mdhd_language(data: &[u8]) -> Option<String> {
    let offset = if *data.first()? == 1 { 32 } else { 20 };
    let code = be_u16(data.get(offset..offset + 2)?);
    Some(
        [(code >> 10) & 31, (code >> 5) & 31, code & 31]
            .iter()
            .map(|v| char::from_u32(u32::from(*v) + 0x60).unwrap_or('?'))
            .collect(),
    )
}
fn parse_stsz(
    node: &BoxNode<'_>,
    max_bytes: u64,
    input_bytes: u64,
    retained_bytes: u64,
    control: &OperationControl,
) -> Result<Option<Vec<usize>>, MediaParseError> {
    let d = node.payload;
    if d.len() < 12 {
        return Ok(Some(vec![]));
    }
    let fixed = be_u32(&d[4..8]) as usize;
    let count = be_u32(&d[8..12]) as usize;
    if fixed > 0 {
        let total = fixed.checked_mul(count).ok_or_else(|| {
            MediaParseError::Malformed("MP4 stsz declared byte count overflow".into())
        })?;
        if total as u64 > max_bytes {
            return Ok(None);
        }
        observe_table_memory(count, input_bytes, retained_bytes, control, "MP4 stsz")?;
        return Ok(Some(vec![fixed; count]));
    }
    let available = d.len().saturating_sub(12) / 4;
    if count > available {
        return Err(MediaParseError::Malformed(
            "MP4 stsz sample count exceeds table bounds".into(),
        ));
    }
    let mut total = 0u64;
    for value in d[12..].chunks_exact(4).take(count) {
        total = total.checked_add(be_u32(value) as u64).ok_or_else(|| {
            MediaParseError::Malformed("MP4 stsz declared byte count overflow".into())
        })?;
        if total > max_bytes {
            return Ok(None);
        }
    }
    observe_table_memory(count, input_bytes, retained_bytes, control, "MP4 stsz")?;
    let sizes = d[12..]
        .chunks_exact(4)
        .take(count)
        .map(|value| be_u32(value) as usize)
        .collect();
    Ok(Some(sizes))
}

fn observe_table_memory(
    count: usize,
    input_bytes: u64,
    retained_bytes: u64,
    control: &OperationControl,
    table: &str,
) -> Result<(), MediaParseError> {
    let memory = (count as u64)
        .checked_mul(std::mem::size_of::<usize>() as u64)
        .and_then(|value| value.checked_add(retained_bytes))
        .and_then(|value| value.checked_add(input_bytes))
        .ok_or_else(|| MediaParseError::Malformed(format!("{table} memory estimate overflow")))?;
    control.budget().observe_memory_bytes(memory)?;
    Ok(())
}
fn parse_offsets(
    node: &BoxNode<'_>,
    wide: bool,
    max_entries: usize,
    input_bytes: u64,
    retained_bytes: u64,
    control: &OperationControl,
) -> Result<Vec<u64>, MediaParseError> {
    let d = node.payload;
    if d.len() < 8 {
        return Ok(vec![]);
    }
    let count = be_u32(&d[4..8]) as usize;
    let width = if wide { 8 } else { 4 };
    let available = d.len().saturating_sub(8) / width;
    if count > available || count > max_entries {
        return Err(MediaParseError::Malformed(
            "MP4 chunk offset count exceeds subtitle sample table bounds".into(),
        ));
    }
    let memory = (count as u64)
        .checked_mul(std::mem::size_of::<u64>() as u64)
        .and_then(|value| value.checked_add(retained_bytes))
        .and_then(|value| value.checked_add(input_bytes))
        .ok_or_else(|| {
            MediaParseError::Malformed("MP4 chunk offset memory estimate overflow".into())
        })?;
    control.budget().observe_memory_bytes(memory)?;
    let offsets = if wide {
        d[8..].chunks_exact(8).take(count).map(be_u64).collect()
    } else {
        d[8..]
            .chunks_exact(4)
            .take(count)
            .map(|v| be_u32(v) as u64)
            .collect()
    };
    Ok(offsets)
}
fn parse_stts(
    node: &BoxNode<'_>,
    max_samples: usize,
    input_bytes: u64,
    retained_bytes: u64,
    control: &OperationControl,
) -> Result<Vec<u64>, MediaParseError> {
    let d = node.payload;
    if d.len() < 8 {
        return Ok(vec![]);
    }
    let count = be_u32(&d[4..8]) as usize;
    let available = d.len().saturating_sub(8) / 8;
    if count > available {
        return Err(MediaParseError::Malformed(
            "MP4 stts entry count exceeds table bounds".into(),
        ));
    }
    let mut expanded = 0usize;
    for entry in d[8..].chunks_exact(8).take(count) {
        let repeat = be_u32(&entry[..4]) as usize;
        expanded = expanded
            .checked_add(repeat)
            .filter(|len| *len <= max_samples)
            .ok_or_else(|| {
                MediaParseError::Malformed(
                    "MP4 stts repetitions exceed subtitle sample count".into(),
                )
            })?;
    }
    let memory = (expanded as u64)
        .checked_mul(std::mem::size_of::<u64>() as u64)
        .and_then(|value| value.checked_add(retained_bytes))
        .and_then(|value| value.checked_add(input_bytes))
        .ok_or_else(|| MediaParseError::Malformed("MP4 stts memory estimate overflow".into()))?;
    control.budget().observe_memory_bytes(memory)?;
    let mut out = Vec::with_capacity(expanded);
    for entry in d[8..].chunks_exact(8).take(count) {
        let repeat = be_u32(&entry[..4]) as usize;
        let delta = be_u32(&entry[4..]) as u64;
        if out
            .len()
            .checked_add(repeat)
            .is_none_or(|len| len > expanded)
        {
            return Err(MediaParseError::Malformed(
                "MP4 stts repetitions exceed subtitle sample count".into(),
            ));
        }
        out.extend(std::iter::repeat_n(delta, repeat));
    }
    Ok(out)
}
fn extract_payl_bytes(sample: &[u8]) -> Option<&[u8]> {
    let mut c = 0;
    while c + 8 <= sample.len() {
        let n = be_u32(&sample[c..c + 4]) as usize;
        if n < 8 || c + n > sample.len() {
            break;
        }
        if &sample[c + 4..c + 8] == b"payl" {
            return Some(&sample[c + 8..c + n]);
        }
        c += n;
    }
    None
}
fn known_codec(codec: &str) -> bool {
    matches!(
        codec,
        "mp4a"
            | "alac"
            | "ac-3"
            | "ec-3"
            | "avc1"
            | "hvc1"
            | "hev1"
            | "vp09"
            | "av01"
            | "wvtt"
            | "stpp"
            | "tx3g"
            | "text"
    )
}
fn fourcc(value: [u8; 4]) -> String {
    if value.iter().all(|v| v.is_ascii_graphic() || *v == b' ') {
        String::from_utf8_lossy(&value).trim().into()
    } else {
        format!(
            "0x{}",
            value.iter().map(|v| format!("{v:02x}")).collect::<String>()
        )
    }
}
fn be_u16(v: &[u8]) -> u16 {
    u16::from_be_bytes([v[0], v[1]])
}
fn be_u32(v: &[u8]) -> u32 {
    u32::from_be_bytes([v[0], v[1], v[2], v[3]])
}
fn be_u64(v: &[u8]) -> u64 {
    u64::from_be_bytes(v[..8].try_into().expect("eight bytes"))
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
