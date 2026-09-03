#![cfg(feature = "media")]

use grist::core::{
    BudgetSelection, Limits, LocationComponent, OperationControl, OperationStatus, ResourceBudget,
    SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentKind, DocumentNodeKind, ToDocumentGraph,
};
use grist::media::{
    CodecInspectionStatus, MediaFormat, MediaOptions, parse_media_bytes,
    parse_media_with_operation_control,
};
use grist::registry::{Capability, ParserSelection, builtin_parser_registry};
use std::path::Path;

fn box_(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}
fn id3_frame(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = kind.to_vec();
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(payload);
    out
}
fn synchsafe(value: usize) -> [u8; 4] {
    [
        ((value >> 21) & 0x7f) as u8,
        ((value >> 14) & 0x7f) as u8,
        ((value >> 7) & 0x7f) as u8,
        (value & 0x7f) as u8,
    ]
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & ((crc & 1).wrapping_neg()));
        }
    }
    !crc
}
fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(&out[4..]).to_be_bytes());
    out
}
fn tiny_png() -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    out.extend(png_chunk(b"IHDR", &ihdr));
    out.extend(png_chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1],
    ));
    out.extend(png_chunk(b"IEND", &[]));
    out
}
fn mp3() -> Vec<u8> {
    let title = id3_frame(b"TIT2", b"\x03Container title");
    let mut chapter_payload = b"intro\0".to_vec();
    chapter_payload.extend_from_slice(&1_000u32.to_be_bytes());
    chapter_payload.extend_from_slice(&2_500u32.to_be_bytes());
    chapter_payload.extend_from_slice(&u32::MAX.to_be_bytes());
    chapter_payload.extend_from_slice(&u32::MAX.to_be_bytes());
    chapter_payload.extend(id3_frame(b"TIT2", b"\x03Opening"));
    let chapter = id3_frame(b"CHAP", &chapter_payload);
    let mut apic = b"\x03image/png\0\x03\0".to_vec();
    apic.extend(tiny_png());
    let artwork = id3_frame(b"APIC", &apic);
    let mut tag = Vec::new();
    tag.extend(title);
    tag.extend(chapter);
    tag.extend(artwork);
    let mut out = b"ID3\x03\0\0".to_vec();
    out.extend_from_slice(&synchsafe(tag.len()));
    out.extend(tag);
    out.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
    out.extend(std::iter::repeat_n(0u8, 256));
    out
}
fn wav(format_tag: u16) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&format_tag.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&48_000u32.to_le_bytes());
    fmt.extend_from_slice(&192_000u32.to_le_bytes());
    fmt.extend_from_slice(&4u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let mut info = b"INFO".to_vec();
    info.extend_from_slice(b"INAM");
    info.extend_from_slice(&5u32.to_le_bytes());
    info.extend_from_slice(b"Wave\0");
    info.push(0);
    let chunks = [
        riff_chunk(b"fmt ", &fmt),
        riff_chunk(b"LIST", &info),
        riff_chunk(b"data", &[0; 64]),
    ]
    .concat();
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend(chunks);
    out
}
fn riff_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = kind.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0)
    }
    out
}
fn flac() -> Vec<u8> {
    let sample_rate = 48_000u64;
    let channels = 2u64;
    let samples = 96_000u64;
    let packed = (sample_rate << 44) | ((channels - 1) << 41) | (15u64 << 36) | samples;
    let mut info = vec![0u8; 34];
    info[0..2].copy_from_slice(&4096u16.to_be_bytes());
    info[2..4].copy_from_slice(&4096u16.to_be_bytes());
    info[10..18].copy_from_slice(&packed.to_be_bytes());
    let mut out = b"fLaC".to_vec();
    out.push(0x80);
    out.extend_from_slice(&[0, 0, 34]);
    out.extend(info);
    out
}
fn mp4() -> Vec<u8> {
    let mut ftyp = b"isom".to_vec();
    ftyp.extend_from_slice(&0u32.to_be_bytes());
    ftyp.extend_from_slice(b"isom");
    let mut mvhd = vec![0u8; 20];
    mvhd[12..16].copy_from_slice(&1_000u32.to_be_bytes());
    mvhd[16..20].copy_from_slice(&10_000u32.to_be_bytes());
    let audio = mp4_track(7, b"soun", b"mp4a");
    let unknown = mp4_track(9, b"vide", b"zzzz");
    let mut chpl = vec![0, 0, 0, 0, 1];
    chpl.extend_from_slice(&20_000_000u64.to_be_bytes());
    chpl.push(5);
    chpl.extend_from_slice(b"Intro");
    let mut moov = box_(b"mvhd", &mvhd);
    moov.extend(audio);
    moov.extend(unknown);
    moov.extend(box_(b"udta", &box_(b"chpl", &chpl)));
    [box_(b"ftyp", &ftyp), box_(b"moov", &moov)].concat()
}
fn quicktime() -> Vec<u8> {
    let mut out = mp4();
    out[8..12].copy_from_slice(b"qt  ");
    out
}

fn iso_file(major: &[u8; 4], compatible: &[[u8; 4]], moov: Vec<u8>) -> Vec<u8> {
    let mut ftyp = major.to_vec();
    ftyp.extend_from_slice(&0u32.to_be_bytes());
    for brand in compatible {
        ftyp.extend_from_slice(brand);
    }
    [box_(b"ftyp", &ftyp), box_(b"moov", &moov)].concat()
}

fn mp4_subtitle_track(id: u32, sizes: (u32, u32), offsets: &[u32]) -> Vec<u8> {
    let mut tkhd = vec![0u8; 24];
    tkhd[12..16].copy_from_slice(&id.to_be_bytes());
    let mut mdhd = vec![0u8; 24];
    mdhd[12..16].copy_from_slice(&1_000u32.to_be_bytes());
    mdhd[16..20].copy_from_slice(&10_000u32.to_be_bytes());
    let mut hdlr = vec![0u8; 12];
    hdlr[8..12].copy_from_slice(b"sbtl");
    let mut entry = (16u32).to_be_bytes().to_vec();
    entry.extend_from_slice(b"wvtt");
    entry.extend_from_slice(&[0; 8]);
    let mut stsd = vec![0; 4];
    stsd.extend_from_slice(&1u32.to_be_bytes());
    stsd.extend(entry);
    let mut stsz = vec![0; 4];
    stsz.extend_from_slice(&sizes.0.to_be_bytes());
    stsz.extend_from_slice(&sizes.1.to_be_bytes());
    let mut stco = vec![0; 4];
    stco.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
    for offset in offsets {
        stco.extend_from_slice(&offset.to_be_bytes());
    }
    let mut stts = vec![0; 4];
    stts.extend_from_slice(&1u32.to_be_bytes());
    stts.extend_from_slice(&sizes.1.to_be_bytes());
    stts.extend_from_slice(&1_000u32.to_be_bytes());
    let stbl = box_(
        b"stbl",
        &[
            box_(b"stsd", &stsd),
            box_(b"stsz", &stsz),
            box_(b"stco", &stco),
            box_(b"stts", &stts),
        ]
        .concat(),
    );
    let mut mdia = box_(b"mdhd", &mdhd);
    mdia.extend(box_(b"hdlr", &hdlr));
    mdia.extend(box_(b"minf", &stbl));
    let mut trak = box_(b"tkhd", &tkhd);
    trak.extend(box_(b"mdia", &mdia));
    box_(b"trak", &trak)
}

fn mp4_track(id: u32, handler: &[u8; 4], codec: &[u8; 4]) -> Vec<u8> {
    let mut tkhd = vec![0u8; 24];
    tkhd[12..16].copy_from_slice(&id.to_be_bytes());
    tkhd[20..24].copy_from_slice(&10_000u32.to_be_bytes());
    let mut mdhd = vec![0u8; 24];
    mdhd[12..16].copy_from_slice(&1_000u32.to_be_bytes());
    mdhd[16..20].copy_from_slice(&10_000u32.to_be_bytes());
    let mut hdlr = vec![0u8; 12];
    hdlr[8..12].copy_from_slice(handler);
    let mut entry = (16u32).to_be_bytes().to_vec();
    entry.extend_from_slice(codec);
    entry.extend_from_slice(&[0; 8]);
    let mut stsd = vec![0; 4];
    stsd.extend_from_slice(&1u32.to_be_bytes());
    stsd.extend(entry);
    let stbl = box_(b"stbl", &box_(b"stsd", &stsd));
    let mut mdia = box_(b"mdhd", &mdhd);
    mdia.extend(box_(b"hdlr", &hdlr));
    mdia.extend(box_(b"minf", &stbl));
    let mut trak = box_(b"tkhd", &tkhd);
    trak.extend(box_(b"mdia", &mdia));
    box_(b"trak", &trak)
}
fn elem(id: &[u8], data: &[u8]) -> Vec<u8> {
    assert!(data.len() < 16_383);
    let mut out = id.to_vec();
    if data.len() < 127 {
        out.push(0x80 | data.len() as u8);
    } else {
        out.push(0x40 | ((data.len() >> 8) as u8));
        out.push(data.len() as u8);
    }
    out.extend_from_slice(data);
    out
}
fn matroska() -> Vec<u8> {
    let info = elem(
        &[0x15, 0x49, 0xa9, 0x66],
        &elem(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]),
    );
    let subtitle_track = track(1, 42, 0x11, "S_TEXT/UTF8", false);
    let secret_track = track(2, 99, 2, "A_SECRET", true);
    let tracks = elem(
        &[0x16, 0x54, 0xae, 0x6b],
        &[subtitle_track, secret_track].concat(),
    );
    let display = elem(&[0x80], &elem(&[0x85], b"Start"));
    let mut atom = elem(&[0x73, 0xc4], &[7]);
    atom.extend(elem(&[0x91], &[0]));
    atom.extend(display);
    let chapters = elem(
        &[0x10, 0x43, 0xa7, 0x70],
        &elem(&[0x45, 0xb9], &elem(&[0xb6], &atom)),
    );
    let srt = b"1\n00:00:00,000 --> 00:00:01,000\nAttached caption\n";
    let mut file = elem(&[0x46, 0xae], &[5]);
    file.extend(elem(&[0x46, 0x6e], b"captions.srt"));
    file.extend(elem(&[0x46, 0x60], b"application/x-subrip"));
    file.extend(elem(&[0x46, 0x5c], srt));
    let attachments = elem(&[0x19, 0x41, 0xa4, 0x69], &elem(&[0x61, 0xa7], &file));
    let mut cluster = elem(&[0xe7], &[0]);
    let mut block = vec![0x81, 0, 0, 0];
    block.extend_from_slice(b"Block caption");
    cluster.extend(elem(&[0xa3], &block));
    let cluster = elem(&[0x1f, 0x43, 0xb6, 0x75], &cluster);
    let segment = [info, tracks, chapters, attachments, cluster].concat();
    [
        elem(&[0x1a, 0x45, 0xdf, 0xa3], &elem(&[0x42, 0x82], b"matroska")),
        elem(&[0x18, 0x53, 0x80, 0x67], &segment),
    ]
    .concat()
}
fn track(number: u8, uid: u8, kind: u8, codec: &str, encrypted: bool) -> Vec<u8> {
    let mut data = elem(&[0xd7], &[number]);
    data.extend(elem(&[0x73, 0xc5], &[uid]));
    data.extend(elem(&[0x83], &[kind]));
    data.extend(elem(&[0x86], codec.as_bytes()));
    if encrypted {
        let encryption = elem(&[0x50, 0x35], &[]);
        let encoding = elem(&[0x62, 0x40], &encryption);
        data.extend(elem(&[0x6d, 0x80], &encoding));
    }
    elem(&[0xae], &data)
}

fn id3_with_frames(version: u8, tag_flags: u8, frames: &[Vec<u8>]) -> Vec<u8> {
    let tag = frames.concat();
    let mut out = vec![b'I', b'D', b'3', version, 0, tag_flags];
    out.extend_from_slice(&synchsafe(tag.len()));
    out.extend(tag);
    out.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
    out
}

fn apic_frame(image: &[u8]) -> Vec<u8> {
    let mut payload = b"\x03image/png\0\x03\0".to_vec();
    payload.extend_from_slice(image);
    id3_frame(b"APIC", &payload)
}

fn flac_picture(image: &[u8]) -> Vec<u8> {
    let mut out = 3u32.to_be_bytes().to_vec();
    out.extend_from_slice(&9u32.to_be_bytes());
    out.extend_from_slice(b"image/png");
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&32u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(image.len() as u32).to_be_bytes());
    out.extend_from_slice(image);
    out
}

fn flac_with_two_pictures() -> Vec<u8> {
    let image = tiny_png();
    let mut out = b"fLaC".to_vec();
    out.extend_from_slice(&[0, 0, 0, 34]);
    out.extend_from_slice(&[0; 34]);
    for (last, picture) in [false, true]
        .into_iter()
        .zip([flac_picture(&image), flac_picture(&image)])
    {
        out.push(if last { 0x86 } else { 0x06 });
        let len = picture.len() as u32;
        out.extend_from_slice(&len.to_be_bytes()[1..]);
        out.extend(picture);
    }
    out
}

fn attachment_file(uid: Option<u8>, filename: &str, media_type: &str, data: &[u8]) -> Vec<u8> {
    let mut file = uid
        .map(|uid| elem(&[0x46, 0xae], &[uid]))
        .unwrap_or_default();
    file.extend(elem(&[0x46, 0x6e], filename.as_bytes()));
    file.extend(elem(&[0x46, 0x60], media_type.as_bytes()));
    file.extend(elem(&[0x46, 0x5c], data));
    elem(&[0x61, 0xa7], &file)
}

fn matroska_with_duplicate_source_ids() -> Vec<u8> {
    let info = elem(
        &[0x15, 0x49, 0xa9, 0x66],
        &elem(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]),
    );
    let atoms = [0u8, 1]
        .into_iter()
        .map(|start| {
            let mut atom = elem(&[0x73, 0xc4], &[7]);
            atom.extend(elem(&[0x91], &[start]));
            elem(&[0xb6], &atom)
        })
        .collect::<Vec<_>>()
        .concat();
    let chapters = elem(&[0x10, 0x43, 0xa7, 0x70], &elem(&[0x45, 0xb9], &atoms));
    let srt = b"1\n00:00:00,000 --> 00:00:01,000\nDuplicate\n";
    let png = tiny_png();
    let files = [
        attachment_file(Some(5), "a.srt", "application/x-subrip", srt),
        attachment_file(Some(5), "b.srt", "application/x-subrip", srt),
        attachment_file(None, "a.bin", "application/octet-stream", b"same"),
        attachment_file(None, "b.bin", "application/octet-stream", b"same"),
        attachment_file(None, "a.png", "image/png", &png),
        attachment_file(None, "b.png", "image/png", &png),
    ]
    .concat();
    let attachments = elem(&[0x19, 0x41, 0xa4, 0x69], &files);
    let segment = [info, chapters, attachments].concat();
    [
        elem(&[0x1a, 0x45, 0xdf, 0xa3], &elem(&[0x42, 0x82], b"matroska")),
        elem(&[0x18, 0x53, 0x80, 0x67], &segment),
    ]
    .concat()
}

fn matroska_with_non_monotone_blocks() -> Vec<u8> {
    let info = elem(
        &[0x15, 0x49, 0xa9, 0x66],
        &elem(&[0x2a, 0xd7, 0xb1], &[0x0f, 0x42, 0x40]),
    );
    let tracks = elem(
        &[0x16, 0x54, 0xae, 0x6b],
        &track(1, 42, 0x11, "S_TEXT/UTF8", false),
    );
    let mut cluster = elem(&[0xe7], &[0]);
    for (time, text) in [(10i16, b"later".as_slice()), (0i16, b"earlier".as_slice())] {
        let mut block = vec![0x81];
        block.extend_from_slice(&time.to_be_bytes());
        block.push(0);
        block.extend_from_slice(text);
        cluster.extend(elem(&[0xa3], &block));
    }
    let segment = [info, tracks, elem(&[0x1f, 0x43, 0xb6, 0x75], &cluster)].concat();
    [
        elem(&[0x1a, 0x45, 0xdf, 0xa3], &elem(&[0x42, 0x82], b"matroska")),
        elem(&[0x18, 0x53, 0x80, 0x67], &segment),
    ]
    .concat()
}

#[test]
fn representative_audio_containers_retain_metadata_and_unsupported_codecs() {
    let mp3 = parse_media_bytes(
        &mp3(),
        SourceInfo::stdin("sample.mp3"),
        MediaFormat::Mp3,
        &MediaOptions::default(),
    );
    assert_eq!(mp3.status, OperationStatus::Complete);
    let doc = mp3.payload().unwrap();
    assert_eq!(doc.streams[0].id, "stream:audio:0");
    assert_eq!(doc.chapters[0].id, "chapter:intro");
    assert!(
        doc.metadata
            .iter()
            .any(|entry| entry.value == "Container title")
    );
    assert!(doc.artwork[0].image.is_some());
    let wav = parse_media_bytes(
        &wav(0x1234),
        SourceInfo::stdin("sample.wav"),
        MediaFormat::Wav,
        &MediaOptions::default(),
    );
    assert_eq!(
        wav.payload().unwrap().streams[0].inspection,
        CodecInspectionStatus::Unsupported
    );
    assert!(
        wav.payload()
            .unwrap()
            .metadata
            .iter()
            .any(|entry| entry.key == "INAM")
    );
    let flac = parse_media_bytes(
        &flac(),
        SourceInfo::stdin("sample.flac"),
        MediaFormat::Flac,
        &MediaOptions::default(),
    );
    let stream = &flac.payload().unwrap().streams[0];
    assert_eq!(stream.sample_rate, Some(48_000));
    assert_eq!(stream.duration_ms, Some(2_000));
}

#[test]
fn iso_stream_and_chapter_identities_are_native_and_stable() {
    let bytes = mp4();
    let first = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("first.mp4"),
        MediaFormat::Mp4,
        &MediaOptions::default(),
    );
    let second = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("renamed.mp4"),
        MediaFormat::Mp4,
        &MediaOptions::default(),
    );
    let a = first.payload().unwrap();
    let b = second.payload().unwrap();
    assert_eq!(
        a.streams.iter().map(|s| &s.id).collect::<Vec<_>>(),
        b.streams.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
    assert_eq!(a.streams[0].id, "stream:track:7");
    assert_eq!(a.streams[1].inspection, CodecInspectionStatus::Unsupported);
    assert_eq!(a.chapters[0].title.as_deref(), Some("Intro"));
    let quicktime = parse_media_bytes(
        &quicktime(),
        SourceInfo::stdin("sample.mov"),
        MediaFormat::QuickTime,
        &MediaOptions::default(),
    );
    assert_eq!(quicktime.status, OperationStatus::Complete);
    assert_eq!(quicktime.payload().unwrap().format, MediaFormat::QuickTime);
}

#[test]
fn duplicate_native_track_ids_remain_unique_and_graph_safe() {
    let moov = [
        mp4_track(7, b"soun", b"mp4a"),
        mp4_track(7, b"vide", b"avc1"),
    ]
    .concat();
    let bytes = iso_file(b"isom", &[*b"mp42"], moov);
    let parsed = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("duplicate.mp4"),
        MediaFormat::Mp4,
        &MediaOptions::default(),
    );
    let document = parsed.payload().unwrap();
    assert_eq!(document.streams[0].native_id.as_deref(), Some("7"));
    assert_eq!(document.streams[1].native_id.as_deref(), Some("7"));
    assert_ne!(document.streams[0].id, document.streams[1].id);
    let graph = document
        .to_document_graph(DocumentGraphContext::new("media:duplicates"))
        .unwrap();
    graph.validate_contract().unwrap();
    let ids = graph
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), graph.nodes.len());
}

#[test]
fn matroska_routes_embedded_subtitles_and_inventories_encryption() {
    let bytes = matroska();
    let parsed = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("sample.mkv"),
        MediaFormat::Matroska,
        &MediaOptions::default(),
    );
    assert_eq!(parsed.status, OperationStatus::Complete);
    let doc = parsed.payload().unwrap();
    assert_eq!(doc.streams[0].id, "stream:track:42");
    assert_eq!(doc.streams[0].container_track_number, Some(1));
    assert!(doc.encrypted);
    assert_eq!(doc.streams[1].inspection, CodecInspectionStatus::Encrypted);
    assert_eq!(doc.chapters[0].id, "chapter:7");
    assert!(
        doc.attachments[0]
            .parsed_subtitle
            .as_ref()
            .is_some_and(|s| s.cues[0].text == "Attached caption")
    );
    assert!(
        doc.subtitle_tracks[0]
            .document
            .as_ref()
            .is_some_and(|s| s.cues[0].text == "Block caption")
    );
    assert!(
        doc.chapters[0]
            .locator
            .components()
            .iter()
            .any(|component| matches!(component, LocationComponent::MediaTime { .. }))
    );
    assert!(
        doc.subtitle_tracks[0]
            .locator
            .components()
            .iter()
            .any(|component| matches!(component, LocationComponent::MediaTime { .. }))
    );
    let attachment_start = bytes
        .windows(b"1\n00:00:00,000 --> 00:00:01,000\nAttached caption\n".len())
        .position(|window| window == b"1\n00:00:00,000 --> 00:00:01,000\nAttached caption\n")
        .unwrap();
    assert!(matches!(
        doc.attachments[0].locator.components().first(),
        Some(LocationComponent::ByteRange { byte_start, byte_end })
            if *byte_start == attachment_start
                && *byte_end == attachment_start
                    + b"1\n00:00:00,000 --> 00:00:01,000\nAttached caption\n".len()
    ));
    assert_eq!(doc.attachments[0].locator.precision().name(), "exact");
    assert_eq!(
        doc.subtitle_tracks[0].locator.precision().name(),
        "approximate"
    );
    assert!(matches!(
        doc.subtitle_tracks[0].locator.components().first(),
        Some(LocationComponent::MediaTime { .. })
    ));
    let synthesized_cue = &doc.subtitle_tracks[0].document.as_ref().unwrap().cues[0];
    assert_eq!(
        synthesized_cue.text_locator.precision().name(),
        "approximate"
    );
    let graph = doc
        .to_document_graph(DocumentGraphContext::new("media:test"))
        .unwrap();
    assert_eq!(graph.kind, DocumentKind::Media);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::MediaTrack)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Attachment)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| { node.kind == DocumentNodeKind::Transcript && node.text.is_none() })
    );
    assert!(graph.nodes.iter().any(|node| {
        node.kind == DocumentNodeKind::Cue
            && node.text.as_deref() == Some("Block caption")
            && node.attrs.contains_key("start_ms")
            && node.attrs.contains_key("end_ms")
            && node.attrs.contains_key("time_locator")
    }));
    graph.validate_contract().unwrap();
}

#[test]
fn non_monotone_media_samples_use_honest_time_only_locators() {
    let first_sample = box_(b"payl", b"first");
    let second_sample = box_(b"payl", b"later");
    assert_eq!(first_sample.len(), second_sample.len());
    let size = first_sample.len() as u32;
    let track = mp4_subtitle_track(3, (size, 2), &[0, 0]);
    let mut bytes = iso_file(b"isom", &[*b"mp42"], track);
    let first_offset = bytes.len();
    bytes.extend_from_slice(&first_sample);
    let second_offset = bytes.len();
    bytes.extend_from_slice(&second_sample);
    let stco = bytes
        .windows(4)
        .position(|window| window == b"stco")
        .unwrap();
    bytes[stco + 12..stco + 16].copy_from_slice(&(second_offset as u32).to_be_bytes());
    bytes[stco + 16..stco + 20].copy_from_slice(&(first_offset as u32).to_be_bytes());
    let parsed = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("reverse-offsets.mp4"),
        MediaFormat::Mp4,
        &MediaOptions::default(),
    );
    assert_ne!(
        parsed.status,
        OperationStatus::Failed,
        "{:?}",
        parsed.diagnostics
    );
    let subtitle = &parsed.payload().unwrap().subtitle_tracks[0];
    assert_eq!(subtitle.locator.precision().name(), "approximate");
    assert!(
        subtitle
            .locator
            .components()
            .iter()
            .all(|component| !matches!(component, LocationComponent::ByteRange { .. }))
    );

    let parsed = parse_media_bytes(
        &matroska_with_non_monotone_blocks(),
        SourceInfo::stdin("non-monotone.mkv"),
        MediaFormat::Matroska,
        &MediaOptions::default(),
    );
    assert_ne!(parsed.status, OperationStatus::Failed);
    let subtitle = &parsed.payload().unwrap().subtitle_tracks[0];
    assert_eq!(subtitle.locator.precision().name(), "approximate");
    assert!(matches!(
        subtitle.locator.components().first(),
        Some(LocationComponent::MediaTime { .. })
    ));
}

#[test]
fn missing_iso_subtitle_tables_are_explicit_partial_results() {
    for (name, track) in [
        ("missing-stsz.mp4", mp4_track(4, b"sbtl", b"wvtt")),
        ("missing-stco.mp4", {
            let mut track = mp4_subtitle_track(4, (1, 1), &[0]);
            let stco = track
                .windows(4)
                .position(|window| window == b"stco")
                .unwrap();
            track[stco..stco + 4].copy_from_slice(b"free");
            track
        }),
    ] {
        let bytes = iso_file(b"isom", &[*b"mp42"], track);
        let parsed = parse_media_bytes(
            &bytes,
            SourceInfo::stdin(name),
            MediaFormat::Mp4,
            &MediaOptions::default(),
        );
        assert_eq!(parsed.status, OperationStatus::Partial, "{name}");
        assert!(
            parsed
                .payload()
                .unwrap()
                .diagnostics
                .iter()
                .any(|diagnostic| {
                    diagnostic.code.as_str() == "media.subtitle.chunk_layout_unsupported"
                })
        );
    }
}

#[test]
fn duplicate_chapter_attachment_and_artwork_ids_are_disambiguated() {
    let parsed = parse_media_bytes(
        &matroska_with_duplicate_source_ids(),
        SourceInfo::stdin("duplicates.mkv"),
        MediaFormat::Matroska,
        &MediaOptions::default(),
    );
    let document = parsed.payload().unwrap();
    for ids in [
        document
            .chapters
            .iter()
            .map(|value| &value.id)
            .collect::<Vec<_>>(),
        document
            .attachments
            .iter()
            .map(|value| &value.id)
            .collect::<Vec<_>>(),
        document
            .artwork
            .iter()
            .map(|value| &value.id)
            .collect::<Vec<_>>(),
    ] {
        assert_eq!(
            ids.len(),
            ids.into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );
    }
    assert_eq!(document.chapters[0].id, "chapter:7");
    assert!(document.chapters[1].id.starts_with("chapter:7:source:"));
    assert_eq!(document.attachments[0].id, "attachment:5");
    assert!(
        document.attachments[1]
            .id
            .starts_with("attachment:5:source:")
    );
    document
        .to_document_graph(DocumentGraphContext::new("media:source-duplicates"))
        .unwrap()
        .validate_contract()
        .unwrap();
}

#[test]
fn repeated_id3_and_flac_artwork_obey_cumulative_retention_limits() {
    let image = tiny_png();
    let mut options = MediaOptions::default();
    options.max_attachment_bytes = image.len() as u64 + 1;
    for (name, format, bytes) in [
        (
            "two-pictures.mp3",
            MediaFormat::Mp3,
            id3_with_frames(3, 0, &[apic_frame(&image), apic_frame(&image)]),
        ),
        (
            "two-pictures.flac",
            MediaFormat::Flac,
            flac_with_two_pictures(),
        ),
    ] {
        let parsed = parse_media_bytes(&bytes, SourceInfo::stdin(name), format, &options);
        assert_eq!(parsed.status, OperationStatus::Partial, "{name}");
        let artwork = &parsed.payload().unwrap().artwork;
        assert_eq!(artwork.len(), 2, "{name}");
        assert!(artwork[0].bytes.is_some(), "{name}");
        assert!(artwork[1].bytes.is_none(), "{name}");
        assert_ne!(artwork[0].id, artwork[1].id, "{name}");
    }
}

#[test]
fn id3_encoding_modes_are_explicit_and_v24_encryption_is_not_decoded() {
    for (name, version, flags, expected) in [
        (
            "unsync.mp3",
            3,
            0x80,
            "media.id3.unsynchronization_unsupported",
        ),
        (
            "extended-v23.mp3",
            3,
            0x40,
            "media.id3.extended_header_unsupported",
        ),
        (
            "extended-v24.mp3",
            4,
            0x40,
            "media.id3.extended_header_unsupported",
        ),
    ] {
        let parsed = parse_media_bytes(
            &id3_with_frames(version, flags, &[id3_frame(b"TIT2", b"\x03Hidden")]),
            SourceInfo::stdin(name),
            MediaFormat::Mp3,
            &MediaOptions::default(),
        );
        assert_eq!(parsed.status, OperationStatus::Partial, "{name}");
        let document = parsed.payload().unwrap();
        assert!(document.metadata.is_empty(), "{name}");
        assert!(
            document
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == expected)
        );
    }

    let payload = b"\x03Title";
    let mut frame = b"TIT2".to_vec();
    frame.extend_from_slice(&synchsafe(payload.len()));
    frame.extend_from_slice(&[0, 0x04]);
    frame.extend_from_slice(payload);
    let parsed = parse_media_bytes(
        &id3_with_frames(4, 0, &[frame]),
        SourceInfo::stdin("encrypted-v24.mp3"),
        MediaFormat::Mp3,
        &MediaOptions::default(),
    );
    let document = parsed.payload().unwrap();
    assert!(document.encrypted);
    assert!(document.metadata.is_empty());
    assert!(
        document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "media.id3.encrypted_frame")
    );
}

#[test]
fn ebml_subtitle_retention_is_preflighted_against_cumulative_memory() {
    let bytes = matroska_with_non_monotone_blocks();
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_memory_bytes = Some(bytes.len() as u64);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let parsed = parse_media_with_operation_control(
        &bytes,
        SourceInfo::stdin("ebml-memory.mkv"),
        MediaFormat::Matroska,
        &MediaOptions::default(),
        &control,
    );
    assert_eq!(parsed.status, OperationStatus::Failed);
    assert!(
        parsed.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "grist.budget.memory_bytes.exhausted"
        })
    );
}

#[test]
fn controlled_child_images_share_the_parent_input_budget() {
    let bytes = mp3();
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_input_bytes = Some(bytes.len() as u64);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let parsed = parse_media_with_operation_control(
        &bytes,
        SourceInfo::stdin("budget-artwork.mp3"),
        MediaFormat::Mp3,
        &MediaOptions::default(),
        &control,
    );
    assert_eq!(parsed.status, OperationStatus::Partial);
    assert!(parsed.payload().unwrap().artwork[0].image.is_none());
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "grist.budget.input_bytes.exhausted")
    );
}

#[test]
fn malicious_iso_sample_tables_are_rejected_before_allocation() {
    let track = mp4_subtitle_track(3, (1, u32::MAX), &[0]);
    let bytes = iso_file(b"isom", &[*b"mp42"], track);
    let parsed = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("huge-stsz.mp4"),
        MediaFormat::Mp4,
        &MediaOptions::default(),
    );
    assert_eq!(parsed.status, OperationStatus::Partial);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "media.subtitle.limit")
    );
    assert!(parsed.payload().unwrap().subtitle_tracks.is_empty());

    let track = mp4_subtitle_track(3, (1, 1_000_000), &[0]);
    let bytes = iso_file(b"isom", &[*b"mp42"], track);
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_memory_bytes = Some(bytes.len() as u64 + 4_096);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let parsed = parse_media_with_operation_control(
        &bytes,
        SourceInfo::stdin("memory-stsz.mp4"),
        MediaFormat::Mp4,
        &MediaOptions::default(),
        &control,
    );
    assert_eq!(parsed.status, OperationStatus::Failed);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "grist.budget.memory_bytes.exhausted")
    );
}

#[test]
fn unsupported_native_subtitle_layout_is_explicit_and_downstream_safe() {
    let track = mp4_subtitle_track(4, (1, 3), &[0, 1]);
    let bytes = iso_file(b"isom", &[*b"mp42"], track);
    let parsed = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("stsc-required.mp4"),
        MediaFormat::Mp4,
        &MediaOptions::default(),
    );
    assert_eq!(parsed.status, OperationStatus::Partial);
    let document = parsed.payload().unwrap();
    assert!(document.subtitle_tracks.is_empty());
    assert!(document.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "media.subtitle.chunk_layout_unsupported"
    }));
    document
        .to_document_graph(DocumentGraphContext::new("media:native-subtitle-failure"))
        .unwrap()
        .validate_contract()
        .unwrap();
}

#[test]
fn encrypted_id3_frames_are_inventoried_without_decoding() {
    let mut frame = b"TIT2".to_vec();
    frame.extend_from_slice(&6u32.to_be_bytes());
    frame.extend_from_slice(&[0, 0x40]);
    frame.extend_from_slice(b"\x03Title");
    let mut bytes = b"ID3\x03\0\0".to_vec();
    bytes.extend_from_slice(&synchsafe(frame.len()));
    bytes.extend(frame);
    bytes.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
    let parsed = parse_media_bytes(
        &bytes,
        SourceInfo::stdin("encrypted-id3.mp3"),
        MediaFormat::Mp3,
        &MediaOptions::default(),
    );
    let document = parsed.payload().unwrap();
    assert!(document.encrypted);
    assert!(document.metadata.iter().all(|entry| entry.value != "Title"));
    assert!(
        document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "media.id3.encrypted_frame")
    );
}

#[test]
fn malformed_limits_and_child_budgets_are_explicit() {
    let malformed = parse_media_bytes(
        &[0, 0, 0, 32, b'f', b't', b'y', b'p'],
        SourceInfo::stdin("bad.mp4"),
        MediaFormat::Mp4,
        &MediaOptions::default(),
    );
    assert_eq!(malformed.status, OperationStatus::Failed);
    let mut options = MediaOptions::default();
    options.max_attachment_bytes = 4;
    let large = parse_media_bytes(
        &matroska(),
        SourceInfo::stdin("large.mkv"),
        MediaFormat::Matroska,
        &options,
    );
    assert_eq!(large.status, OperationStatus::Partial);
    assert!(
        large
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "media.attachment.limit")
    );
    let mut budget = ResourceBudget::trusted_unbounded();
    let retained = &large.payload().unwrap().attachments[0];
    assert!(retained.byte_length > options.max_attachment_bytes);
    assert!(retained.sha256.starts_with("sha256:") && retained.sha256.len() == 71);
    assert!(retained.bytes.is_none());
    budget.max_child_artifacts = Some(0);
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let limited = parse_media_with_operation_control(
        &matroska(),
        SourceInfo::stdin("budget.mkv"),
        MediaFormat::Matroska,
        &MediaOptions::default(),
        &control,
    );
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "grist.budget.child_artifacts.exhausted")
    );
}

#[test]
fn registry_and_schema_surfaces_advertise_media_contract() {
    let registry = builtin_parser_registry().unwrap();
    for format in ["mp3", "mp4", "quicktime", "wav", "flac", "matroska"] {
        let ParserSelection::Available(descriptor) = registry.select_format(format) else {
            panic!("{format} unavailable")
        };
        assert_eq!(descriptor.payload_schema.version, "grist/media/v1");
        assert!(
            descriptor
                .capabilities
                .contains(&Capability::EmbeddedArtifacts)
        );
    }
    #[cfg(feature = "schemas")]
    {
        let names = grist::schema::schema_catalog()
            .schemas
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"media".to_string()));
        assert!(names.contains(&"media-envelope".to_string()));
        assert!(names.contains(&"media-options".to_string()));
        let schema = grist::schema::schema_json("media-options").unwrap();
        for field in [
            "max_boxes",
            "max_nesting_depth",
            "max_metadata_bytes",
            "max_attachment_bytes",
            "max_subtitle_bytes",
        ] {
            assert_eq!(schema["properties"][field]["minimum"].as_f64(), Some(1.0));
        }
    }
}

#[test]
fn media_magic_detection_selects_the_container_contract() {
    let registry = builtin_parser_registry().unwrap();
    let cases = [
        ("sample.mp3", mp3(), ContentKind::Mp3, "mp3"),
        ("sample.mp4", mp4(), ContentKind::Mp4, "mp4"),
        ("sample.wav", wav(1), ContentKind::Wav, "wav"),
        ("sample.flac", flac(), ContentKind::Flac, "flac"),
        (
            "sample.mov",
            quicktime(),
            ContentKind::QuickTime,
            "quicktime",
        ),
        ("sample.mkv", matroska(), ContentKind::Matroska, "matroska"),
    ];

    for (name, bytes, expected, expected_format) in cases {
        let detection = detect_with_registry(
            Path::new(name),
            &bytes,
            None,
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detection.content_kind, expected, "{name}");
        assert_eq!(detection.candidates[0].identity.format, expected_format);
    }
}

#[test]
fn iso_brand_and_ebml_doctype_collisions_do_not_steal_other_formats() {
    let registry = builtin_parser_registry().unwrap();
    for (name, major, compatible) in [
        ("sample.heic", *b"heic", *b"mif1"),
        ("sample.avif", *b"avif", *b"mif1"),
        ("compatible.heic", *b"isom", *b"heic"),
    ] {
        let bytes = iso_file(&major, &[compatible], Vec::new());
        let detection = detect_with_registry(
            Path::new(name),
            &bytes,
            None,
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(detection.content_kind, ContentKind::Heif, "{name}");
        assert_eq!(detection.candidates[0].identity.format, "heif", "{name}");
        assert_eq!(
            parse_media_bytes(
                &bytes,
                SourceInfo::stdin(name),
                MediaFormat::Mp4,
                &MediaOptions::default(),
            )
            .status,
            OperationStatus::Failed
        );
    }

    let compatible_quicktime = iso_file(b"isom", &[*b"qt  "], Vec::new());
    let detection = detect_with_registry(
        Path::new("compatible.mov"),
        &compatible_quicktime,
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.content_kind, ContentKind::QuickTime);
    assert_eq!(
        parse_media_bytes(
            &compatible_quicktime,
            SourceInfo::stdin("compatible.mov"),
            MediaFormat::QuickTime,
            &MediaOptions::default(),
        )
        .status,
        OperationStatus::Complete
    );

    let unrelated_ebml = [
        elem(
            &[0x1a, 0x45, 0xdf, 0xa3],
            &elem(&[0x42, 0x82], b"not-matroska"),
        ),
        elem(&[0x18, 0x53, 0x80, 0x67], &[]),
    ]
    .concat();
    let detection = detect_with_registry(
        Path::new("unrelated.ebml"),
        &unrelated_ebml,
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_ne!(detection.content_kind, ContentKind::Matroska);
    assert_eq!(
        parse_media_bytes(
            &unrelated_ebml,
            SourceInfo::stdin("unrelated.ebml"),
            MediaFormat::Matroska,
            &MediaOptions::default(),
        )
        .status,
        OperationStatus::Failed
    );
}

#[test]
#[cfg(feature = "cli")]
fn cli_dispatch_projects_media_payload_to_graph() {
    let envelope = grist::cli::parse_bytes(
        "mp4",
        mp4(),
        SourceInfo::stdin("cli.mp4"),
        grist::core::RequestId::new("cli-media").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    let graph = grist::cli::project_envelope_to_graph(&envelope, "media:cli").unwrap();
    assert_eq!(graph.kind, DocumentKind::Media);
}
