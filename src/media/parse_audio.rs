use super::model::*;
use super::parse::{
    MediaParseError, PARSER, byte_locator, limit_diagnostic, timed_locator, utf8_lossy,
    with_media_time,
};
use crate::core::{Diagnostic, OperationControl};

pub(crate) fn parse_mp3(
    bytes: &[u8],
    options: &MediaOptions,
    control: &OperationControl,
) -> Result<ParsedMedia, MediaParseError> {
    if !bytes.starts_with(b"ID3") && find_mpeg_frame(bytes, 0).is_none() {
        return Err(MediaParseError::Malformed(
            "input is not an MP3/ID3 stream".into(),
        ));
    }
    let mut out = parsed(bytes);
    let mut retained_embedded_bytes = 0u64;
    let mut audio_start = 0usize;
    if bytes.starts_with(b"ID3") {
        if bytes.len() < 10 {
            return Err(MediaParseError::Malformed("truncated ID3 header".into()));
        }
        let version = bytes[3];
        if !(2..=4).contains(&version) {
            return Err(MediaParseError::Malformed(format!(
                "unsupported ID3 version 2.{version}"
            )));
        }
        let tag_size = synchsafe(&bytes[6..10])
            .ok_or_else(|| MediaParseError::Malformed("invalid ID3 synchsafe size".into()))?
            as usize;
        let tag_end = 10usize
            .checked_add(tag_size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| MediaParseError::Malformed("ID3 tag exceeds input".into()))?;
        if tag_size as u64 > options.max_metadata_bytes {
            out.complete = false;
            out.diagnostics.push(limit_diagnostic(
                "media.metadata.limit",
                "ID3 metadata exceeds max_metadata_bytes",
                &byte_locator(0, tag_end),
            ));
        } else {
            parse_id3_frames(
                &bytes[10..tag_end],
                10,
                version,
                bytes[5],
                options,
                control,
                &mut retained_embedded_bytes,
                &mut out,
            )?;
        }
        audio_start =
            tag_end + usize::from(bytes.get(5).is_some_and(|flags| flags & 0x10 != 0)) * 10;
        audio_start = audio_start.min(bytes.len());
        out.technical.container_profile = Some(format!("ID3v2.{version}"));
    }
    let frame_offset = find_mpeg_frame(bytes, audio_start);
    let (bit_rate, sample_rate, channels) = frame_offset
        .and_then(|offset| mpeg_header(bytes.get(offset..offset + 4)?))
        .unwrap_or((None, None, None));
    out.technical.bit_rate = bit_rate;
    if let Some(rate) = bit_rate.filter(|rate| *rate > 0) {
        out.technical.duration_ms = (bytes.len().saturating_sub(audio_start) as u64)
            .checked_mul(8_000)
            .map(|value| value / rate);
    }
    out.streams.push(MediaStream {
        id: "stream:audio:0".into(),
        index: 0,
        native_id: None,
        container_track_number: None,
        kind: MediaStreamKind::Audio,
        codec: "mp3".into(),
        inspection: CodecInspectionStatus::MetadataOnly,
        language: None,
        name: None,
        duration_ms: out.technical.duration_ms,
        sample_rate,
        channels,
        width: None,
        height: None,
        encrypted: false,
        locator: byte_locator(audio_start, bytes.len()),
    });
    control.checkpoint()?;
    Ok(out)
}

fn parse_id3_frames(
    data: &[u8],
    base: usize,
    version: u8,
    tag_flags: u8,
    options: &MediaOptions,
    control: &OperationControl,
    retained_embedded_bytes: &mut u64,
    out: &mut ParsedMedia,
) -> Result<(), MediaParseError> {
    if tag_flags & 0x80 != 0 {
        out.complete = false;
        out.diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "media.id3.unsynchronization_unsupported",
                "ID3 tag-level unsynchronization was inventoried without decoding frames",
            )
            .partial()
            .with_locator(byte_locator(base, base + data.len())),
        );
        return Ok(());
    }
    if tag_flags & 0x40 != 0 {
        out.complete = false;
        let (code, message) = if version == 2 {
            (
                "media.id3.compression_unsupported",
                "compressed ID3v2.2 tag was inventoried without decoding frames",
            )
        } else {
            (
                "media.id3.extended_header_unsupported",
                "ID3 extended header was inventoried without decoding frames",
            )
        };
        out.diagnostics.push(
            Diagnostic::warning(PARSER, code, message)
                .partial()
                .with_locator(byte_locator(base, base + data.len())),
        );
        return Ok(());
    }
    let mut cursor = 0usize;
    while cursor < data.len() {
        control.checkpoint()?;
        let (id, size, header) = if version == 2 {
            if data.len() - cursor < 6 {
                break;
            }
            (
                &data[cursor..cursor + 3],
                be_u24(&data[cursor + 3..cursor + 6]) as usize,
                6,
            )
        } else {
            if data.len() - cursor < 10 {
                break;
            }
            let size = if version == 4 {
                synchsafe(&data[cursor + 4..cursor + 8]).unwrap_or(u32::MAX)
            } else {
                be_u32(&data[cursor + 4..cursor + 8])
            } as usize;
            (&data[cursor..cursor + 4], size, 10)
        };
        if id.iter().all(|byte| *byte == 0) {
            break;
        }
        if !id
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            out.complete = false;
            out.diagnostics.push(
                Diagnostic::warning(PARSER, "media.id3.frame_id", "invalid ID3 frame identifier")
                    .partial()
                    .with_locator(byte_locator(
                        base + cursor,
                        base + (cursor + header).min(data.len()),
                    )),
            );
            break;
        }
        let end = cursor
            .checked_add(header)
            .and_then(|v| v.checked_add(size))
            .filter(|end| *end <= data.len());
        let Some(end) = end else {
            out.complete = false;
            out.diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "media.id3.frame_truncated",
                    "ID3 frame exceeds tag bounds",
                )
                .partial()
                .with_locator(byte_locator(base + cursor, base + data.len())),
            );
            break;
        };
        let frame = &data[cursor + header..end];
        let id = std::str::from_utf8(id).unwrap_or("????");
        let locator = byte_locator(base + cursor, base + end);
        let encrypted = match version {
            3 => data.get(cursor + 9).is_some_and(|flag| flag & 0x40 != 0),
            4 => data.get(cursor + 9).is_some_and(|flag| flag & 0x04 != 0),
            _ => false,
        };
        let frame_unsynchronized =
            version == 4 && data.get(cursor + 9).is_some_and(|flag| flag & 0x02 != 0);
        let frame_compressed = version >= 3
            && data.get(cursor + 9).is_some_and(|flag| {
                if version == 3 {
                    flag & 0x80 != 0
                } else {
                    flag & 0x08 != 0
                }
            });
        if frame_unsynchronized || frame_compressed {
            out.complete = false;
            out.diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    if frame_unsynchronized {
                        "media.id3.unsynchronization_unsupported"
                    } else {
                        "media.id3.compression_unsupported"
                    },
                    format!("encoded ID3 frame {id} was inventoried without decoding"),
                )
                .partial()
                .with_locator(locator),
            );
        } else if encrypted {
            out.encrypted = true;
            out.complete = false;
            out.diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "media.id3.encrypted_frame",
                    format!("encrypted ID3 frame {id} was inventoried but not decoded"),
                )
                .partial()
                .with_locator(locator),
            );
        } else if id == "APIC" || id == "PIC" {
            if let Some(art) = parse_apic(
                frame,
                id,
                options.max_attachment_bytes,
                out.technical.byte_length,
                control,
                retained_embedded_bytes,
                base + cursor + header,
            )? {
                if art.inventory.is_some() {
                    out.complete = false;
                    out.diagnostics.push(limit_diagnostic(
                        "media.artwork.limit",
                        "ID3 artwork exceeds max_attachment_bytes; retained as inventory only",
                        &locator,
                    ));
                }
                out.artwork_candidates.push(art);
            }
        } else if id == "CHAP" {
            if let Some(chapter) = parse_chap(frame, out.chapters.len(), locator.clone()) {
                out.chapters.push(chapter);
            }
        } else if id.starts_with('T') && id != "TXXX" {
            if frame.len() as u64 <= options.max_metadata_bytes {
                if let Some(value) = decode_id3_text(frame) {
                    out.metadata.push(metadata(id, value, "id3", locator));
                }
            }
        } else if matches!(id, "COMM" | "USLT") {
            if let Some(value) = decode_id3_text(frame.get(4..).unwrap_or_default()) {
                out.metadata.push(metadata(id, value, "id3", locator));
            }
        }
        cursor = end;
    }
    Ok(())
}

fn parse_apic(
    frame: &[u8],
    id: &str,
    max_bytes: u64,
    input_bytes: u64,
    control: &OperationControl,
    retained_embedded_bytes: &mut u64,
    frame_start: usize,
) -> Result<Option<ArtworkCandidate>, MediaParseError> {
    let Some(encoding) = frame.first().copied() else {
        return Ok(None);
    };
    let mut cursor = 1usize;
    let media_type = if id == "PIC" {
        let Some(kind_bytes) = frame.get(cursor..cursor + 3) else {
            return Ok(None);
        };
        let Ok(kind) = std::str::from_utf8(kind_bytes) else {
            return Ok(None);
        };
        let kind = kind.to_ascii_lowercase();
        cursor += 3;
        Some(
            match kind.as_str() {
                "png" => "image/png",
                "jpg" => "image/jpeg",
                _ => "application/octet-stream",
            }
            .into(),
        )
    } else {
        let Some(end) = frame[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|v| v + cursor)
        else {
            return Ok(None);
        };
        let value = utf8_lossy(&frame[cursor..end]);
        cursor = end + 1;
        Some(value)
    };
    let picture_type = frame
        .get(cursor)
        .copied()
        .map(|value| format!("id3:{value}"));
    cursor += 1;
    let Some(description_end) = terminated_end(frame, cursor, encoding) else {
        return Ok(None);
    };
    let description = decode_encoded(encoding, &frame[cursor..description_end]);
    cursor = description_end + if matches!(encoding, 1 | 2) { 2 } else { 1 };
    let Some(image) = frame.get(cursor..) else {
        return Ok(None);
    };
    if image.is_empty() {
        return Ok(None);
    }
    let next = retained_embedded_bytes
        .checked_add(image.len() as u64)
        .ok_or_else(|| MediaParseError::Malformed("ID3 artwork byte count overflow".into()))?;
    let over_limit = next > max_bytes;
    if !over_limit {
        let memory = input_bytes
            .checked_add(next)
            .ok_or_else(|| MediaParseError::Malformed("ID3 artwork memory overflow".into()))?;
        control.budget().observe_memory_bytes(memory)?;
        *retained_embedded_bytes = next;
    }
    Ok(Some(ArtworkCandidate {
        picture_type,
        description: Some(description),
        media_type,
        bytes: if over_limit {
            Vec::new()
        } else {
            image.to_vec()
        },
        locator: byte_locator(frame_start + cursor, frame_start + frame.len()),
        image: None,
        inventory: over_limit.then(|| (image.len() as u64, crate::core::sha256_hex(image))),
    }))
}

fn parse_chap(
    frame: &[u8],
    index: usize,
    locator: crate::core::SourceLocator,
) -> Option<MediaChapter> {
    let nul = frame.iter().position(|byte| *byte == 0)?;
    let native = utf8_lossy(&frame[..nul]);
    let fields = frame.get(nul + 1..nul + 17)?;
    let start_ms = be_u32(&fields[0..4]) as u64;
    let end = be_u32(&fields[4..8]);
    let title = find_embedded_text_frame(frame.get(nul + 17..).unwrap_or_default(), b"TIT2");
    Some(MediaChapter {
        id: format!("chapter:{native}"),
        index: index as u64,
        native_id: Some(native),
        start_ms,
        end_ms: (end != u32::MAX).then_some(end as u64),
        title,
        locator: with_media_time(
            locator,
            start_ms,
            (end != u32::MAX).then_some(end as u64).unwrap_or(start_ms),
            None,
        ),
    })
}

fn find_embedded_text_frame(data: &[u8], wanted: &[u8; 4]) -> Option<String> {
    let pos = data.windows(4).position(|value| value == wanted)?;
    let size = be_u32(data.get(pos + 4..pos + 8)?) as usize;
    decode_id3_text(data.get(pos + 10..pos + 10 + size)?)
}

pub(crate) fn parse_wav(
    bytes: &[u8],
    options: &MediaOptions,
    control: &OperationControl,
) -> Result<ParsedMedia, MediaParseError> {
    if bytes.len() < 12
        || (!bytes.starts_with(b"RIFF") && !bytes.starts_with(b"RF64"))
        || &bytes[8..12] != b"WAVE"
    {
        return Err(MediaParseError::Malformed(
            "input is not a RIFF/RF64 WAVE container".into(),
        ));
    }
    let mut out = parsed(bytes);
    let mut retained_embedded_bytes = 0u64;
    out.technical.container_profile = Some(utf8_lossy(&bytes[..4]));
    let mut cursor = 12usize;
    let mut stream = None;
    let mut data_range = None;
    let mut cue_offsets = Vec::new();
    while cursor + 8 <= bytes.len() {
        control.checkpoint()?;
        let id = &bytes[cursor..cursor + 4];
        let size = le_u32(&bytes[cursor + 4..cursor + 8]) as usize;
        let end = cursor
            .checked_add(8)
            .and_then(|v| v.checked_add(size))
            .filter(|v| *v <= bytes.len())
            .ok_or_else(|| {
                MediaParseError::Malformed(format!("WAVE chunk {} exceeds input", utf8_lossy(id)))
            })?;
        let data = &bytes[cursor + 8..end];
        let locator = byte_locator(cursor, end);
        match id {
            b"fmt " => {
                if data.len() < 16 {
                    return Err(MediaParseError::Malformed(
                        "truncated WAVE fmt chunk".into(),
                    ));
                }
                let tag = le_u16(&data[0..2]);
                let channels = le_u16(&data[2..4]) as u64;
                let sample_rate = le_u32(&data[4..8]) as u64;
                let bit_rate = le_u32(&data[8..12]) as u64 * 8;
                let codec = wav_codec(tag, data);
                out.technical.bit_rate = Some(bit_rate);
                stream = Some(MediaStream {
                    id: "stream:audio:0".into(),
                    index: 0,
                    native_id: Some("fmt:0".into()),
                    container_track_number: None,
                    kind: MediaStreamKind::Audio,
                    codec,
                    inspection: if matches!(tag, 1 | 3 | 6 | 7 | 0xfffe) {
                        CodecInspectionStatus::MetadataOnly
                    } else {
                        CodecInspectionStatus::Unsupported
                    },
                    language: None,
                    name: None,
                    duration_ms: None,
                    sample_rate: Some(sample_rate),
                    channels: Some(channels),
                    width: None,
                    height: None,
                    encrypted: false,
                    locator,
                });
            }
            b"data" => data_range = Some((cursor + 8, end)),
            b"LIST"
                if data.starts_with(b"INFO") && data.len() as u64 <= options.max_metadata_bytes =>
            {
                parse_riff_info(&data[4..], cursor + 12, &mut out)
            }
            b"cue " => parse_wave_cues(data, cursor + 8, &mut cue_offsets),
            b"id3 " | b"ID3 " if data.len() as u64 <= options.max_metadata_bytes => {
                if data.starts_with(b"ID3") && data.len() >= 10 {
                    let tag_end =
                        (10 + synchsafe(&data[6..10]).unwrap_or(0) as usize).min(data.len());
                    parse_id3_frames(
                        &data[10..tag_end],
                        cursor + 18,
                        data[3],
                        data[5],
                        options,
                        control,
                        &mut retained_embedded_bytes,
                        &mut out,
                    )?;
                }
            }
            _ => {}
        }
        cursor = end + (size & 1);
    }
    let Some(mut stream) = stream else {
        return Err(MediaParseError::Malformed("WAVE has no fmt chunk".into()));
    };
    if let Some((start, end)) = data_range {
        stream.locator = byte_locator(start, end);
        if let Some(bit_rate) = out.technical.bit_rate.filter(|v| *v > 0) {
            out.technical.duration_ms = ((end - start) as u64)
                .checked_mul(8_000)
                .map(|value| value / bit_rate);
            stream.duration_ms = out.technical.duration_ms;
        }
    }
    out.streams.push(stream);
    for (index, (sample, start, end)) in cue_offsets.into_iter().enumerate() {
        let sample_rate = out.streams[0].sample_rate.unwrap_or(1);
        let start_ms = sample
            .checked_mul(1000)
            .ok_or_else(|| MediaParseError::Malformed("WAVE cue time overflow".into()))?
            / sample_rate;
        out.chapters.push(MediaChapter {
            id: format!("chapter:cue:{sample}"),
            index: index as u64,
            native_id: Some(sample.to_string()),
            start_ms,
            end_ms: None,
            title: None,
            locator: timed_locator(start, end, start_ms, start_ms, Some(0)),
        });
    }
    Ok(out)
}

fn parse_riff_info(mut data: &[u8], mut base: usize, out: &mut ParsedMedia) {
    while data.len() >= 8 {
        let id = utf8_lossy(&data[..4]);
        let size = le_u32(&data[4..8]) as usize;
        if 8usize.checked_add(size).is_none_or(|end| end > data.len()) {
            break;
        }
        out.metadata.push(metadata(
            &id,
            utf8_lossy(&data[8..8 + size]),
            "riff.info",
            byte_locator(base, base + 8 + size),
        ));
        let advance = 8 + size + (size & 1);
        data = &data[advance.min(data.len())..];
        base += advance;
    }
}

fn parse_wave_cues(data: &[u8], base: usize, output: &mut Vec<(u64, usize, usize)>) {
    let count = data.get(..4).map(le_u32).unwrap_or(0) as usize;
    for (index, entry) in data
        .get(4..)
        .unwrap_or_default()
        .chunks_exact(24)
        .take(count)
        .enumerate()
    {
        let start = base + 4 + index * 24;
        output.push((le_u32(&entry[20..24]) as u64, start, start + 24));
    }
}

pub(crate) fn parse_flac(
    bytes: &[u8],
    options: &MediaOptions,
    control: &OperationControl,
) -> Result<ParsedMedia, MediaParseError> {
    if !bytes.starts_with(b"fLaC") {
        return Err(MediaParseError::Malformed(
            "input is not a FLAC stream".into(),
        ));
    }
    let mut out = parsed(bytes);
    out.technical.container_profile = Some("FLAC native".into());
    let mut retained_embedded_bytes = 0u64;
    let mut cursor = 4usize;
    let mut last = false;
    let mut stream = None;
    while !last {
        control.checkpoint()?;
        if cursor + 4 > bytes.len() {
            return Err(MediaParseError::Malformed(
                "truncated FLAC metadata block header".into(),
            ));
        }
        last = bytes[cursor] & 0x80 != 0;
        let kind = bytes[cursor] & 0x7f;
        let size = be_u24(&bytes[cursor + 1..cursor + 4]) as usize;
        let end = cursor
            .checked_add(4)
            .and_then(|v| v.checked_add(size))
            .filter(|v| *v <= bytes.len())
            .ok_or_else(|| {
                MediaParseError::Malformed("FLAC metadata block exceeds input".into())
            })?;
        let data = &bytes[cursor + 4..end];
        let locator = byte_locator(cursor, end);
        if size as u64 > options.max_metadata_bytes && kind != 0 {
            out.complete = false;
            out.diagnostics.push(limit_diagnostic(
                "media.metadata.limit",
                "FLAC metadata block exceeds max_metadata_bytes",
                &locator,
            ));
        } else {
            match kind {
                0 => {
                    if data.len() != 34 {
                        return Err(MediaParseError::Malformed(
                            "invalid FLAC STREAMINFO size".into(),
                        ));
                    }
                    let packed = u64::from_be_bytes(
                        data[10..18]
                            .try_into()
                            .expect("FLAC STREAMINFO packed fields"),
                    );
                    let sample_rate = (packed >> 44) & 0xfffff;
                    let channels = ((packed >> 41) & 7) + 1;
                    let total_samples = packed & 0x0f_ffff_ffff;
                    let duration = (sample_rate > 0)
                        .then(|| total_samples.checked_mul(1000).map(|v| v / sample_rate))
                        .flatten();
                    out.technical.duration_ms = duration;
                    stream = Some(MediaStream {
                        id: "stream:audio:0".into(),
                        index: 0,
                        native_id: Some("streaminfo:0".into()),
                        container_track_number: None,
                        kind: MediaStreamKind::Audio,
                        codec: "flac".into(),
                        inspection: CodecInspectionStatus::MetadataOnly,
                        language: None,
                        name: None,
                        duration_ms: duration,
                        sample_rate: Some(sample_rate),
                        channels: Some(channels),
                        width: None,
                        height: None,
                        encrypted: false,
                        locator: locator.clone(),
                    });
                }
                4 => parse_vorbis_comment(data, cursor + 4, &mut out),
                5 => parse_cuesheet(
                    data,
                    cursor + 4,
                    out.streams
                        .first()
                        .and_then(|s| s.sample_rate)
                        .or_else(|| stream.as_ref().and_then(|s| s.sample_rate))
                        .unwrap_or(44_100),
                    &mut out,
                ),
                6 => {
                    if let Some(art) = parse_flac_picture(
                        data,
                        options.max_attachment_bytes,
                        bytes.len() as u64,
                        control,
                        &mut retained_embedded_bytes,
                        cursor + 4,
                    )? {
                        if art.inventory.is_some() {
                            out.complete = false;
                            out.diagnostics.push(limit_diagnostic(
                                "media.artwork.limit",
                                "FLAC artwork exceeds max_attachment_bytes; retained as inventory only",
                                &locator,
                            ));
                        }
                        out.artwork_candidates.push(art);
                    }
                }
                _ => {}
            }
        }
        cursor = end;
    }
    let Some(mut stream) = stream else {
        return Err(MediaParseError::Malformed(
            "FLAC has no STREAMINFO block".into(),
        ));
    };
    stream.locator = byte_locator(4, cursor);
    out.streams.push(stream);
    Ok(out)
}

fn parse_vorbis_comment(data: &[u8], base: usize, out: &mut ParsedMedia) {
    let Some(vendor_len) = data.get(..4).map(le_u32).map(|v| v as usize) else {
        return;
    };
    let Some(mut cursor) = 4usize
        .checked_add(vendor_len)
        .filter(|v| *v + 4 <= data.len())
    else {
        return;
    };
    let count = le_u32(&data[cursor..cursor + 4]) as usize;
    cursor += 4;
    for _ in 0..count {
        if cursor + 4 > data.len() {
            break;
        }
        let size = le_u32(&data[cursor..cursor + 4]) as usize;
        let start = cursor;
        cursor += 4;
        if cursor.checked_add(size).is_none_or(|end| end > data.len()) {
            break;
        }
        let text = utf8_lossy(&data[cursor..cursor + size]);
        let (key, value) = text.split_once('=').unwrap_or(("COMMENT", text.as_str()));
        out.metadata.push(metadata(
            key,
            value.to_string(),
            "vorbis_comment",
            byte_locator(base + start, base + cursor + size),
        ));
        cursor += size;
    }
}

fn parse_cuesheet(data: &[u8], base: usize, sample_rate: u64, out: &mut ParsedMedia) {
    if data.len() < 396 {
        return;
    }
    let count = data[395] as usize;
    let mut cursor = 396usize;
    for _ in 0..count {
        if cursor + 36 > data.len() {
            break;
        }
        let sample = be_u64(&data[cursor..cursor + 8]);
        let number = data[cursor + 8];
        if number != 170 {
            let Some(start_ms) = sample
                .checked_mul(1000)
                .map(|value| value / sample_rate.max(1))
            else {
                let Some(advance) = (data[cursor + 35] as usize)
                    .checked_mul(12)
                    .and_then(|value| value.checked_add(36))
                else {
                    break;
                };
                let Some(next) = cursor.checked_add(advance) else {
                    break;
                };
                cursor = next;
                continue;
            };
            out.chapters.push(MediaChapter {
                id: format!("chapter:track:{number}"),
                index: out.chapters.len() as u64,
                native_id: Some(number.to_string()),
                start_ms,
                end_ms: None,
                title: None,
                locator: timed_locator(
                    base + cursor,
                    base + cursor + 36,
                    start_ms,
                    start_ms,
                    Some(0),
                ),
            });
        }
        let indices = data[cursor + 35] as usize;
        let Some(advance) = indices
            .checked_mul(12)
            .and_then(|value| value.checked_add(36))
        else {
            break;
        };
        let Some(next) = cursor.checked_add(advance) else {
            break;
        };
        cursor = next;
    }
}

fn parse_flac_picture(
    data: &[u8],
    max_bytes: u64,
    input_bytes: u64,
    control: &OperationControl,
    retained_embedded_bytes: &mut u64,
    data_start: usize,
) -> Result<Option<ArtworkCandidate>, MediaParseError> {
    if data.len() < 32 {
        return Ok(None);
    }
    let picture_type = be_u32(&data[..4]);
    let mime_len = be_u32(&data[4..8]) as usize;
    let Some(mime_end) = 8usize.checked_add(mime_len) else {
        return Ok(None);
    };
    let Some(desc_len_bytes) = data.get(mime_end..mime_end + 4) else {
        return Ok(None);
    };
    let desc_len = be_u32(desc_len_bytes) as usize;
    let desc_start = mime_end + 4;
    let Some(desc_end) = desc_start.checked_add(desc_len) else {
        return Ok(None);
    };
    let Some(data_len_offset) = desc_end.checked_add(16) else {
        return Ok(None);
    };
    let Some(image_len_bytes) = data.get(data_len_offset..data_len_offset + 4) else {
        return Ok(None);
    };
    let image_len = be_u32(image_len_bytes) as usize;
    let image_start = data_len_offset + 4;
    let Some(image_end) = image_start.checked_add(image_len) else {
        return Ok(None);
    };
    let Some(image) = data.get(image_start..image_end) else {
        return Ok(None);
    };
    let next = retained_embedded_bytes
        .checked_add(image.len() as u64)
        .ok_or_else(|| MediaParseError::Malformed("FLAC artwork byte count overflow".into()))?;
    let over_limit = next > max_bytes;
    if !over_limit {
        let memory = input_bytes
            .checked_add(next)
            .ok_or_else(|| MediaParseError::Malformed("FLAC artwork memory overflow".into()))?;
        control.budget().observe_memory_bytes(memory)?;
        *retained_embedded_bytes = next;
    }
    Ok(Some(ArtworkCandidate {
        picture_type: Some(format!("flac:{picture_type}")),
        description: Some(utf8_lossy(&data[desc_start..desc_end])),
        media_type: Some(utf8_lossy(&data[8..mime_end])),
        bytes: if over_limit {
            Vec::new()
        } else {
            image.to_vec()
        },
        locator: byte_locator(data_start + image_start, data_start + image_end),
        image: None,
        inventory: over_limit.then(|| (image.len() as u64, crate::core::sha256_hex(image))),
    }))
}

fn parsed(bytes: &[u8]) -> ParsedMedia {
    ParsedMedia {
        technical: MediaTechnicalMetadata {
            byte_length: bytes.len() as u64,
            ..Default::default()
        },
        complete: true,
        ..Default::default()
    }
}
fn metadata(
    key: &str,
    value: String,
    source: &str,
    locator: crate::core::SourceLocator,
) -> MediaMetadataEntry {
    MediaMetadataEntry {
        key: key.into(),
        value,
        source: source.into(),
        locator,
    }
}
fn synchsafe(bytes: &[u8]) -> Option<u32> {
    if bytes.len() != 4 || bytes.iter().any(|v| v & 0x80 != 0) {
        None
    } else {
        Some(
            bytes
                .iter()
                .fold(0, |value, byte| (value << 7) | u32::from(*byte)),
        )
    }
}
fn be_u16(v: &[u8]) -> u16 {
    u16::from_be_bytes([v[0], v[1]])
}
fn le_u16(v: &[u8]) -> u16 {
    u16::from_le_bytes([v[0], v[1]])
}
fn be_u24(v: &[u8]) -> u32 {
    u32::from_be_bytes([0, v[0], v[1], v[2]])
}
fn be_u32(v: &[u8]) -> u32 {
    u32::from_be_bytes([v[0], v[1], v[2], v[3]])
}
fn le_u32(v: &[u8]) -> u32 {
    u32::from_le_bytes([v[0], v[1], v[2], v[3]])
}
fn be_u64(v: &[u8]) -> u64 {
    u64::from_be_bytes(v[..8].try_into().expect("eight bytes"))
}
fn find_mpeg_frame(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .windows(4)
        .position(|value| mpeg_header(value).is_some())
        .map(|v| v + start)
}
fn mpeg_header(bytes: &[u8]) -> Option<(Option<u64>, Option<u64>, Option<u64>)> {
    if bytes.len() < 4 {
        return None;
    }
    let version = (bytes[1] >> 3) & 3;
    let layer = (bytes[1] >> 1) & 3;
    let br = (bytes[2] >> 4) & 15;
    let sr = (bytes[2] >> 2) & 3;
    if version == 1 || layer == 0 || br == 0 || br == 15 || sr == 3 {
        return None;
    }
    let rates = if version == 3 {
        [44_100, 48_000, 32_000]
    } else if version == 2 {
        [22_050, 24_000, 16_000]
    } else {
        [11_025, 12_000, 8_000]
    };
    let bitrates = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ];
    Some((
        Some(bitrates[br as usize] * 1000),
        Some(rates[sr as usize]),
        Some(if bytes[3] >> 6 == 3 { 1 } else { 2 }),
    ))
}
fn terminated_end(data: &[u8], start: usize, encoding: u8) -> Option<usize> {
    if matches!(encoding, 1 | 2) {
        data.get(start..)?
            .windows(2)
            .position(|v| v == [0, 0])
            .map(|v| start + v)
    } else {
        data.get(start..)?
            .iter()
            .position(|v| *v == 0)
            .map(|v| start + v)
    }
}
fn decode_id3_text(frame: &[u8]) -> Option<String> {
    Some(decode_encoded(*frame.first()?, frame.get(1..)?))
}
fn decode_encoded(encoding: u8, data: &[u8]) -> String {
    match encoding {
        0 => data
            .iter()
            .map(|b| char::from(*b))
            .collect::<String>()
            .trim_matches(char::from(0))
            .to_string(),
        3 => utf8_lossy(data),
        1 => {
            let (be, start) = if data.starts_with(&[0xfe, 0xff]) {
                (true, 2)
            } else {
                (false, usize::from(data.starts_with(&[0xff, 0xfe])) * 2)
            };
            decode_utf16(&data[start..], be)
        }
        2 => decode_utf16(data, true),
        _ => utf8_lossy(data),
    }
}
fn decode_utf16(data: &[u8], be: bool) -> String {
    String::from_utf16_lossy(
        &data
            .chunks_exact(2)
            .map(|v| if be { be_u16(v) } else { le_u16(v) })
            .collect::<Vec<_>>(),
    )
    .trim_matches(char::from(0))
    .to_string()
}
fn wav_codec(tag: u16, data: &[u8]) -> String {
    match tag {
        1 => "pcm".into(),
        3 => "ieee-float".into(),
        6 => "g711-alaw".into(),
        7 => "g711-mulaw".into(),
        0x55 => "mp3".into(),
        0xfffe if data.len() >= 40 => format!("wave-extensible:{:02x?}", &data[24..40]),
        _ => format!("wave-format-0x{tag:04x}"),
    }
}
