#![cfg(feature = "media")]

use flate2::Compression;
use flate2::write::ZlibEncoder;
use grist::core::{
    BudgetProfile, BudgetSelection, CancellationToken, Input, Limits, OperationStatus,
    ParseRequest, ProviderSet, RequestId, ResourceBudget, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentKind, DocumentNodeKind, ToDocumentGraph,
};
use grist::image::{ImageFormat, ImageOptions, parse_image_bytes};
use grist::registry::{Capability, ParserSelection, builtin_parser_registry};
use std::io::Write;
use std::path::Path;

fn source(name: &str) -> SourceInfo {
    SourceInfo::stdin(name)
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let mut value = index as u32;
        for _ in 0..8 {
            let mask = (value & 1).wrapping_neg();
            value = (value >> 1) ^ (0xedb8_8320 & mask);
        }
        *entry = value;
    }
    let mut crc = u32::MAX;
    for byte in bytes {
        crc = table[((crc as u8) ^ *byte) as usize] ^ (crc >> 8);
    }
    !crc
}

fn ztxt(keyword: &str, text: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(text).unwrap();
    let mut data = keyword.as_bytes().to_vec();
    data.extend_from_slice(&[0, 0]);
    data.extend(encoder.finish().unwrap());
    png_chunk(b"zTXt", &data)
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&png_crc32(&out[4..]).to_be_bytes());
    out
}

fn png(width: u32, height: u32, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut chunks = extra.to_vec();
    chunks.push(png_chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1],
    ));
    png_with_color_and_chunks(width, height, 6, &chunks)
}

fn png_with_color_and_chunks(
    width: u32,
    height: u32,
    color_type: u8,
    chunks: &[Vec<u8>],
) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, color_type, 0, 0, 0]);
    out.extend(png_chunk(b"IHDR", &header));
    for chunk in chunks {
        out.extend_from_slice(chunk);
    }
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn animated_png() -> Vec<u8> {
    animated_png_with_fdat_sequence(2)
}

fn animated_png_with_fdat_sequence(fdat_sequence: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&2u32.to_be_bytes());
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    out.extend(png_chunk(b"IHDR", &header));
    let mut control = Vec::new();
    control.extend_from_slice(&2u32.to_be_bytes());
    control.extend_from_slice(&0u32.to_be_bytes());
    out.extend(png_chunk(b"acTL", &control));
    for sequence in 0..2u32 {
        let mut frame = Vec::new();
        frame.extend_from_slice(&sequence.to_be_bytes());
        frame.extend_from_slice(&1u32.to_be_bytes());
        frame.extend_from_slice(&1u32.to_be_bytes());
        frame.extend_from_slice(&sequence.to_be_bytes());
        frame.extend_from_slice(&0u32.to_be_bytes());
        frame.extend_from_slice(&1u16.to_be_bytes());
        frame.extend_from_slice(&1u16.to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        out.extend(png_chunk(b"fcTL", &frame));
        if sequence == 0 {
            out.extend(png_chunk(
                b"IDAT",
                &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1],
            ));
        } else {
            let mut data = fdat_sequence.to_be_bytes().to_vec();
            data.extend_from_slice(&[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1]);
            out.extend(png_chunk(b"fdAT", &data));
        }
    }
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn jpeg() -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 2, 0, 3, 1, 1, 0x11, 0];
    out.extend_from_slice(&[0xff, 0xfe, 0, 7]);
    out.extend_from_slice(b"hello");
    out.extend_from_slice(&[0xff, 0xda, 0, 8, 1, 1, 0, 0, 63, 0, 0]);
    out.extend_from_slice(&[0xff, 0xd9]);
    out
}

fn gif_two_frames() -> Vec<u8> {
    let mut out = b"GIF89a\x01\0\x01\0\x80\0\0".to_vec();
    out.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
    for _ in 0..2 {
        out.extend_from_slice(&[0x21, 0xf9, 4, 0, 1, 0, 0, 0]);
        out.extend_from_slice(&[0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
        out.extend_from_slice(&[2, 2, 0x44, 0x01, 0]);
    }
    out.push(0x3b);
    out
}

fn riff_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(kind);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() & 1 == 1 {
        out.push(0);
    }
    out
}

fn animated_webp() -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    body.extend(riff_chunk(b"VP8X", &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    for _ in 0..2 {
        let mut frame = vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
        frame.extend(riff_chunk(b"VP8L", &[0x2f, 0, 0, 0, 0]));
        body.extend(riff_chunk(b"ANMF", &frame));
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

fn bmp() -> Vec<u8> {
    let mut out = vec![0u8; 58];
    out[0..2].copy_from_slice(b"BM");
    out[2..6].copy_from_slice(&58u32.to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&1i32.to_le_bytes());
    out[22..26].copy_from_slice(&1i32.to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&24u16.to_le_bytes());
    out[54..58].copy_from_slice(&[0x11, 0x22, 0x33, 0]);
    out
}

fn bmff_box(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    out.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out
}

fn infe(id: u16, item_type: &[u8; 4], name: &str, content_type: Option<&str>) -> Vec<u8> {
    let mut data = vec![2, 0, 0, 0];
    data.extend_from_slice(&id.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(item_type);
    data.extend_from_slice(name.as_bytes());
    data.push(0);
    if let Some(content_type) = content_type {
        data.extend_from_slice(content_type.as_bytes());
        data.push(0);
    }
    bmff_box(b"infe", &data)
}

fn iloc(extents: &[(u16, u32, u32)]) -> Vec<u8> {
    let mut data = vec![0, 0, 0, 0, 0x44, 0];
    data.extend_from_slice(&(extents.len() as u16).to_be_bytes());
    for (id, offset, length) in extents {
        data.extend_from_slice(&id.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&offset.to_be_bytes());
        data.extend_from_slice(&length.to_be_bytes());
    }
    bmff_box(b"iloc", &data)
}

fn heif_meta(location_box: Vec<u8>, include_primary: bool) -> Vec<u8> {
    let pitm = bmff_box(b"pitm", &[0, 0, 0, 0, 0, 1]);
    let mut iinf_data = vec![0, 0, 0, 0, 0, 3];
    iinf_data.extend(infe(1, b"hvc1", "primary", None));
    iinf_data.extend(infe(2, b"Exif", "camera", None));
    iinf_data.extend(infe(3, b"mime", "xmp", Some("application/rdf+xml")));
    let iinf = bmff_box(b"iinf", &iinf_data);

    let reference = |from: u16| {
        let mut data = Vec::new();
        data.extend_from_slice(&from.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        bmff_box(b"cdsc", &data)
    };
    let mut iref_data = vec![0, 0, 0, 0];
    iref_data.extend(reference(2));
    iref_data.extend(reference(3));
    let iref = bmff_box(b"iref", &iref_data);

    let property = |width: u32, height: u32| {
        let mut data = vec![0, 0, 0, 0];
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&height.to_be_bytes());
        bmff_box(b"ispe", &data)
    };
    let mut ipco_data = property(99, 88);
    ipco_data.extend(property(4, 3));
    let ipco = bmff_box(b"ipco", &ipco_data);
    let mut ipma_data = vec![0, 0, 0, 0];
    ipma_data.extend_from_slice(&1u32.to_be_bytes());
    ipma_data.extend_from_slice(&1u16.to_be_bytes());
    ipma_data.extend_from_slice(&[1, 2]);
    let ipma = bmff_box(b"ipma", &ipma_data);
    let mut iprp_data = ipco;
    iprp_data.extend(ipma);
    let iprp = bmff_box(b"iprp", &iprp_data);

    let mut meta_data = vec![0, 0, 0, 0];
    if include_primary {
        meta_data.extend(pitm);
    }
    meta_data.extend(iinf);
    meta_data.extend(location_box);
    meta_data.extend(iref);
    meta_data.extend(iprp);
    bmff_box(b"meta", &meta_data)
}

fn heif() -> Vec<u8> {
    let ftyp = bmff_box(b"ftyp", b"heic\0\0\0\0mif1heic");
    let primary = [0xaa, 0xbb, 0xcc, 0xdd];
    let mut exif = 0u32.to_be_bytes().to_vec();
    exif.extend(tiff_with_gps());
    let xmp = b"<x:xmpmeta>associated</x:xmpmeta>";
    let placeholder = iloc(&[
        (1, 0, primary.len() as u32),
        (2, 0, exif.len() as u32),
        (3, 0, xmp.len() as u32),
    ]);
    let placeholder_meta = heif_meta(placeholder, true);
    let data_start = (ftyp.len() + placeholder_meta.len() + 8) as u32;
    let locations = iloc(&[
        (1, data_start, primary.len() as u32),
        (2, data_start + primary.len() as u32, exif.len() as u32),
        (
            3,
            data_start + primary.len() as u32 + exif.len() as u32,
            xmp.len() as u32,
        ),
    ]);
    let meta = heif_meta(locations, true);
    assert_eq!(meta.len(), placeholder_meta.len());
    let mut mdat = primary.to_vec();
    mdat.extend(exif);
    mdat.extend(xmp);
    let mut out = ftyp;
    out.extend(meta);
    out.extend(bmff_box(b"mdat", &mdat));
    out
}

fn tiff_with_gps() -> Vec<u8> {
    let mut out = vec![0u8; 152];
    out[0..2].copy_from_slice(b"II");
    out[2..4].copy_from_slice(&42u16.to_le_bytes());
    out[4..8].copy_from_slice(&8u32.to_le_bytes());
    out[8..10].copy_from_slice(&3u16.to_le_bytes());
    tiff_entry(&mut out, 10, 256, 3, 1, 10);
    tiff_entry(&mut out, 22, 257, 3, 1, 20);
    tiff_entry(&mut out, 34, 34853, 4, 1, 50);
    out[50..52].copy_from_slice(&4u16.to_le_bytes());
    tiff_entry(&mut out, 52, 1, 2, 2, u32::from_le_bytes([b'N', 0, 0, 0]));
    tiff_entry(&mut out, 64, 2, 5, 3, 104);
    tiff_entry(&mut out, 76, 3, 2, 2, u32::from_le_bytes([b'W', 0, 0, 0]));
    tiff_entry(&mut out, 88, 4, 5, 3, 128);
    write_rationals(&mut out, 104, &[(49, 1), (30, 1), (0, 1)]);
    write_rationals(&mut out, 128, &[(123, 1), (15, 1), (0, 1)]);
    out
}

fn tiff_entry(out: &mut [u8], at: usize, tag: u16, kind: u16, count: u32, value: u32) {
    out[at..at + 2].copy_from_slice(&tag.to_le_bytes());
    out[at + 2..at + 4].copy_from_slice(&kind.to_le_bytes());
    out[at + 4..at + 8].copy_from_slice(&count.to_le_bytes());
    out[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
}

fn write_rationals(out: &mut [u8], mut at: usize, values: &[(u32, u32)]) {
    for (numerator, denominator) in values {
        out[at..at + 4].copy_from_slice(&numerator.to_le_bytes());
        out[at + 4..at + 8].copy_from_slice(&denominator.to_le_bytes());
        at += 8;
    }
}

#[test]
fn native_formats_cover_multiframe_dimensions_and_camera_metadata() {
    let cases = [
        (animated_png(), ImageFormat::Png, 2),
        (jpeg(), ImageFormat::Jpeg, 1),
        (tiff_with_gps(), ImageFormat::Tiff, 1),
        (animated_webp(), ImageFormat::Webp, 2),
        (gif_two_frames(), ImageFormat::Gif, 2),
        (bmp(), ImageFormat::Bmp, 1),
        (heif(), ImageFormat::Heif, 1),
    ];
    for (bytes, format, frames) in cases {
        let envelope = parse_image_bytes(&bytes, source(format.as_str()), &ImageOptions::default());
        assert_eq!(
            envelope.status,
            OperationStatus::Complete,
            "{format:?}: {:?}",
            envelope.diagnostics
        );
        let document = envelope.payload.unwrap();
        assert_eq!(document.format, format);
        assert_eq!(document.frames.len(), frames);
        assert!(
            document
                .frames
                .iter()
                .all(|frame| !frame.locator.components().is_empty())
        );
    }
    let tiff = parse_image_bytes(
        &tiff_with_gps(),
        source("gps.tiff"),
        &ImageOptions::default(),
    )
    .payload
    .unwrap();
    assert_eq!(
        tiff.metadata.camera.gps_latitude.as_deref(),
        Some("49.50000000")
    );
    assert_eq!(
        tiff.metadata.camera.gps_longitude.as_deref(),
        Some("-123.25000000")
    );
    assert!(
        tiff.metadata
            .blocks
            .iter()
            .all(|block| block.byte_length < 100)
    );
    let heif_document =
        parse_image_bytes(&heif(), source("associated.heic"), &ImageOptions::default())
            .payload
            .unwrap();
    assert_eq!(
        (
            heif_document.dimensions.width,
            heif_document.dimensions.height
        ),
        (4, 3)
    );
    assert!(
        heif_document
            .metadata
            .blocks
            .iter()
            .any(|block| { block.kind == grist::image::ImageMetadataKind::Exif })
    );
    assert!(heif_document.metadata.blocks.iter().any(|block| {
        block.kind == grist::image::ImageMetadataKind::Xmp
            && block
                .text
                .as_deref()
                .is_some_and(|text| text.contains("associated"))
    }));
    let mut many_mdat = heif();
    for _ in 0..512 {
        many_mdat.extend(bmff_box(b"mdat", &[0]));
    }
    assert_eq!(
        parse_image_bytes(
            &many_mdat,
            source("many-mdat.heic"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Complete
    );
}

#[test]
fn malformed_oversized_and_unknown_chunks_fail_closed_under_limits() {
    let mut corrupt = png(1, 1, &[]);
    corrupt[29] ^= 1;
    assert_eq!(
        parse_image_bytes(&corrupt, source("bad.png"), &ImageOptions::default()).status,
        OperationStatus::Failed
    );

    let limited = ImageOptions {
        max_dimension: 1,
        ..ImageOptions::default()
    };
    assert_eq!(
        parse_image_bytes(&png(2, 1, &[]), source("large.png"), &limited).status,
        OperationStatus::Failed
    );

    let unknown = png(1, 1, &[png_chunk(b"aaAA", b"retain-me")]);
    let limited = ImageOptions {
        max_unknown_chunk_bytes: 4,
        ..ImageOptions::default()
    };
    let envelope = parse_image_bytes(&unknown, source("unknown.png"), &limited);
    assert_eq!(envelope.status, OperationStatus::Failed);
    assert!(
        envelope.diagnostics[0]
            .message
            .contains("max_unknown_chunk_bytes")
    );

    let metadata_limited = ImageOptions {
        max_metadata_bytes: 40,
        ..ImageOptions::default()
    };
    assert_eq!(
        parse_image_bytes(&tiff_with_gps(), source("metadata.tiff"), &metadata_limited).status,
        OperationStatus::Failed
    );

    let compressed = png(1, 1, &[ztxt("Comment", &vec![b'a'; 8 * 1024])]);
    let compressed_limited = ImageOptions {
        max_metadata_bytes: 1024,
        ..ImageOptions::default()
    };
    assert_eq!(
        parse_image_bytes(&compressed, source("deflate-bomb.png"), &compressed_limited).status,
        OperationStatus::Failed
    );
}

#[test]
fn structurally_incomplete_containers_are_not_reported_complete() {
    let mut png_without_data = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&1u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_without_data.extend(png_chunk(b"IHDR", &header));
    png_without_data.extend(png_chunk(b"IEND", &[]));
    assert_eq!(
        parse_image_bytes(
            &png_without_data,
            source("header-only.png"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Failed
    );
    let image_data = png_chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1]);
    let palette = png_chunk(b"PLTE", &[0, 0, 0]);
    for malformed in [
        png_with_color_and_chunks(1, 1, 3, std::slice::from_ref(&image_data)),
        png_with_color_and_chunks(
            1,
            1,
            3,
            &[palette.clone(), palette.clone(), image_data.clone()],
        ),
        png_with_color_and_chunks(1, 1, 3, &[image_data.clone(), palette]),
        png_with_color_and_chunks(
            1,
            1,
            6,
            &[
                image_data.clone(),
                png_chunk(b"tEXt", b"key\0value"),
                image_data,
            ],
        ),
        animated_png_with_fdat_sequence(1),
    ] {
        assert_eq!(
            parse_image_bytes(
                &malformed,
                source("bad-order.png"),
                &ImageOptions::default()
            )
            .status,
            OperationStatus::Failed
        );
    }

    let jpeg_without_scan = vec![
        0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 2, 0, 3, 1, 1, 0x11, 0, 0xff, 0xd9,
    ];
    assert_eq!(
        parse_image_bytes(
            &jpeg_without_scan,
            source("header-only.jpg"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Failed
    );
    let mut jpeg_without_entropy = jpeg_without_scan[..jpeg_without_scan.len() - 2].to_vec();
    jpeg_without_entropy.extend_from_slice(&[0xff, 0xda, 0, 8, 1, 1, 0, 0, 63, 0, 0xff, 0xd9]);
    assert_eq!(
        parse_image_bytes(
            &jpeg_without_entropy,
            source("empty-scan.jpg"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Failed
    );
    let mut malformed_sof = jpeg();
    malformed_sof[11] = 2;
    let mut malformed_sos = jpeg();
    let sos = malformed_sos
        .windows(2)
        .position(|window| window == [0xff, 0xda])
        .unwrap();
    malformed_sos[sos + 4] = 2;
    let mut invalid_tables = jpeg();
    let sos = invalid_tables
        .windows(2)
        .position(|window| window == [0xff, 0xda])
        .unwrap();
    invalid_tables[sos + 6] = 0x44;
    for malformed in [malformed_sof, malformed_sos, invalid_tables] {
        assert_eq!(
            parse_image_bytes(
                &malformed,
                source("bad-structure.jpg"),
                &ImageOptions::default()
            )
            .status,
            OperationStatus::Failed
        );
    }
    let mut incomplete_components = vec![
        0xff, 0xd8, 0xff, 0xc0, 0, 17, 8, 0, 2, 0, 3, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0,
    ];
    incomplete_components.extend_from_slice(&[0xff, 0xda, 0, 8, 1, 1, 0, 0, 63, 0, 0]);
    incomplete_components.extend_from_slice(&[0xff, 0xd9]);
    assert_eq!(
        parse_image_bytes(
            &incomplete_components,
            source("missing-components.jpg"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Failed
    );
    let mut empty_later_scan = vec![
        0xff, 0xd8, 0xff, 0xc0, 0, 17, 8, 0, 2, 0, 3, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0,
    ];
    empty_later_scan.extend_from_slice(&[0xff, 0xda, 0, 8, 1, 1, 0, 0, 63, 0, 0]);
    empty_later_scan.extend_from_slice(&[0xff, 0xda, 0, 8, 1, 2, 0, 0, 63, 0]);
    empty_later_scan.extend_from_slice(&[0xff, 0xd9]);
    assert_eq!(
        parse_image_bytes(
            &empty_later_scan,
            source("empty-later-scan.jpg"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Failed
    );

    let mut unassociated = bmff_box(b"ftyp", b"heic\0\0\0\0mif1heic");
    let ispe = {
        let mut data = vec![0, 0, 0, 0];
        data.extend_from_slice(&9u32.to_be_bytes());
        data.extend_from_slice(&8u32.to_be_bytes());
        bmff_box(b"ispe", &data)
    };
    unassociated.extend(bmff_box(
        b"meta",
        &[vec![0, 0, 0, 0], bmff_box(b"ipco", &ispe)].concat(),
    ));
    assert_eq!(
        parse_image_bytes(
            &unassociated,
            source("unassociated.heic"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Failed
    );
    let ftyp = bmff_box(b"ftyp", b"heic\0\0\0\0mif1heic");
    let pitm = bmff_box(b"pitm", &[0, 0, 0, 0, 0, 1]);
    let mut primary_only = vec![0, 0, 0, 0];
    primary_only.extend(pitm);
    let mut split_meta = ftyp;
    split_meta.extend(bmff_box(b"meta", &primary_only));
    split_meta.extend(heif_meta(iloc(&[(1, 0, 4)]), false));
    split_meta.extend(bmff_box(b"mdat", &[1, 2, 3, 4]));
    assert_eq!(
        parse_image_bytes(
            &split_meta,
            source("split-meta.heic"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Failed
    );

    let mut invalid_extent = heif();
    let iloc_kind = invalid_extent
        .windows(4)
        .position(|window| window == b"iloc")
        .unwrap();
    invalid_extent[iloc_kind + 18..iloc_kind + 22].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        parse_image_bytes(
            &invalid_extent,
            source("bad-extent.heic"),
            &ImageOptions::default()
        )
        .status,
        OperationStatus::Failed
    );

    let metadata_limited = ImageOptions {
        max_metadata_bytes: 32,
        ..ImageOptions::default()
    };
    assert_eq!(
        parse_image_bytes(&heif(), source("large-exif.heic"), &metadata_limited).status,
        OperationStatus::Failed
    );
}

#[test]
fn svg_active_content_is_inventoried_inertly_with_distinct_locators() {
    let svg = br#"<svg width="20" height="10">
      <style>@import 'https://example.invalid/a.css'; .x{fill:url(https://example.invalid/p.png)}</style>
      <image src="https://example.invalid/source.png"/>
      <animate begin="click" dur="2s" attributeName="x"/>
      <script>fetch('https://example.invalid/run')</script>
      <text>first</text><text>second</text>
    </svg>"#;
    let document = parse_image_bytes(svg, source("active.svg"), &ImageOptions::default())
        .payload
        .expect("SVG remains parseable without executing active content");
    assert_eq!(document.format, ImageFormat::Svg);
    for required in [
        "css_import",
        "css_url",
        "smil_animation",
        "smil_timing",
        "script",
        "script_body",
    ] {
        assert!(
            document
                .active_content
                .iter()
                .any(|item| item.kind == required),
            "missing {required}"
        );
    }
    assert!(
        document
            .links
            .iter()
            .any(|link| link.target.ends_with("source.png"))
    );
    assert!(document.links.iter().all(|link| matches!(
        link.disposition,
        grist::image::ImageActiveContentDisposition::InventoriedNotExecuted
    )));
    assert_ne!(
        document.embedded_text[0].locator,
        document.embedded_text[1].locator
    );
    let graph = document
        .to_document_graph(DocumentGraphContext::new("image-svg"))
        .expect("image graph");
    assert_eq!(graph.kind, DocumentKind::Image);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Link)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.name.as_deref() == Some("script"))
    );
}

#[test]
fn svg_detection_uses_only_the_actual_root_and_caps_depth_and_path_size() {
    let registry = builtin_parser_registry().unwrap();
    let true_svg = br#"<?xml version="1.0"?><!-- lead --><!DOCTYPE svg><s:svg xmlns:s="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
    let detection = detect_with_registry(
        Path::new("root.bin"),
        true_svg,
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.content_kind, ContentKind::Svg);

    let nested = br#"<?xml version="1.0"?><document><svg width="1" height="1"/></document>"#;
    let detection = detect_with_registry(
        Path::new("nested.xml"),
        nested,
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_ne!(detection.content_kind, ContentKind::Svg);
    assert_eq!(
        parse_image_bytes(nested, source("nested.xml"), &ImageOptions::default()).status,
        OperationStatus::Failed
    );

    let deep = br#"<svg width="1" height="1"><g><g><path/></g></g></svg>"#;
    let depth_limited = ImageOptions {
        max_svg_depth: 3,
        ..ImageOptions::default()
    };
    assert_eq!(
        parse_image_bytes(deep, source("deep.svg"), &depth_limited).status,
        OperationStatus::Failed
    );
    let path_limited = ImageOptions {
        max_svg_path_bytes: 12,
        ..ImageOptions::default()
    };
    assert_eq!(
        parse_image_bytes(deep, source("long-path.svg"), &path_limited).status,
        OperationStatus::Failed
    );

    let mut within_cap = String::from(r#"<svg width="1" height="1">"#);
    for _ in 0..200 {
        within_cap.push_str(r#"<g onload="x">"#);
    }
    for _ in 0..200 {
        within_cap.push_str("</g>");
    }
    within_cap.push_str("</svg>");
    let options = ImageOptions {
        max_svg_depth: 256,
        max_svg_path_bytes: 128,
        ..ImageOptions::default()
    };
    let document = parse_image_bytes(
        within_cap.as_bytes(),
        source("deep-within-cap.svg"),
        &options,
    )
    .payload
    .expect("bounded deep SVG parses with constant-size locator construction");
    let locators = document
        .active_content
        .iter()
        .filter(|item| item.kind == "event_handler")
        .map(|item| serde_json::to_string(&item.locator).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(locators.len(), 200);
}

fn dispatch_svg_with_budget(
    bytes: &[u8],
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("image-budget").unwrap(),
        Input::bytes(bytes.to_vec()),
        source("budget.svg"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch("svg", request, None)
        .unwrap()
}

#[test]
fn image_parser_preserves_cancellation_and_incremental_shared_budget_diagnostics() {
    let mut node_budget = ResourceBudget::trusted_unbounded();
    node_budget.max_nodes = Some(2);
    let node_limited = dispatch_svg_with_budget(
        br#"<svg width="1" height="1"><g/><g/><g/></svg>"#,
        node_budget,
    );
    assert_eq!(node_limited.status, OperationStatus::Failed);
    assert!(
        node_limited
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.budget.nodes.exhausted" })
    );

    let mut decoded_budget = ResourceBudget::trusted_unbounded();
    decoded_budget.max_decoded_characters = Some(2);
    let decoded_limited = dispatch_svg_with_budget(
        br#"<svg width="1" height="1"><text>long text</text></svg>"#,
        decoded_budget,
    );
    assert_eq!(decoded_limited.status, OperationStatus::Failed);
    assert!(decoded_limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.decoded_characters.exhausted"
    }));

    let mut compressed_budget = ResourceBudget::trusted_unbounded();
    compressed_budget.max_decoded_characters = Some(8);
    let compressed = png(1, 1, &[ztxt("Comment", &vec![b'x'; 4_096])]);
    let compressed_request = ParseRequest::new(
        RequestId::new("image-compressed-budget").unwrap(),
        Input::bytes(compressed),
        source("budget.png"),
        BudgetSelection::custom(compressed_budget),
        ProviderSet::none(),
    );
    let compressed_limited = builtin_parser_registry()
        .unwrap()
        .dispatch("png", compressed_request, None)
        .unwrap();
    assert_eq!(compressed_limited.status, OperationStatus::Failed);
    assert!(compressed_limited.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_str() == "grist.budget.decoded_characters.exhausted"
    }));

    let mut css_budget = ResourceBudget::trusted_unbounded();
    css_budget.max_nodes = Some(10);
    let repeated_imports = format!(
        r#"<svg width="1" height="1"><style>{}</style></svg>"#,
        "@import 'x.css';".repeat(100)
    );
    let css_limited = dispatch_svg_with_budget(repeated_imports.as_bytes(), css_budget);
    assert_eq!(css_limited.status, OperationStatus::Failed);
    assert!(
        css_limited
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.budget.nodes.exhausted" })
    );

    let mut heif_budget = ResourceBudget::trusted_unbounded();
    heif_budget.max_records = Some(1);
    let request = ParseRequest::new(
        RequestId::new("heif-budget").unwrap(),
        Input::bytes(heif()),
        source("budget.heic"),
        BudgetSelection::custom(heif_budget),
        ProviderSet::none(),
    );
    let heif_limited = builtin_parser_registry()
        .unwrap()
        .dispatch("heif", request, None)
        .unwrap();
    assert_eq!(heif_limited.status, OperationStatus::Failed);
    assert!(
        heif_limited
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.budget.records.exhausted" })
    );

    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let mut large_svg = String::from(r#"<svg width="1" height="1"><style>"#);
    large_svg.push_str(&"@import 'x.css';".repeat(300_000));
    large_svg.push_str("</style></svg>");
    let request = ParseRequest::new(
        RequestId::new("image-cancelled").unwrap(),
        Input::bytes(large_svg.into_bytes()),
        source("cancelled.svg"),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    )
    .with_cancellation(cancellation);
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        trigger.cancel();
    });
    let cancelled = builtin_parser_registry()
        .unwrap()
        .dispatch("svg", request, None)
        .unwrap();
    canceller.join().unwrap();
    assert_eq!(cancelled.status, OperationStatus::Cancelled);
    assert!(
        cancelled
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.operation.cancelled" })
    );
}

#[test]
fn detection_registry_schema_and_explicit_format_gate_are_integrated() {
    let registry = builtin_parser_registry().unwrap();
    match registry.select_format("png") {
        ParserSelection::Available(descriptor) => {
            assert_eq!(
                descriptor.options.default,
                serde_json::to_value(ImageOptions::default()).unwrap()
            );
            assert_eq!(descriptor.options.schema.name, "image-options");
            assert_eq!(descriptor.options.schema.version, "grist/image-options/v1");
            assert!(descriptor.allowed_providers.is_empty());
            assert!(
                !descriptor
                    .capabilities
                    .contains(&Capability::ProviderDerivedContent)
            );
        }
        selection => panic!("PNG descriptor is unavailable: {selection:?}"),
    }
    let bytes = animated_png();
    let detection = detect_with_registry(
        Path::new("camera.bin"),
        &bytes,
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Png);

    let request = ParseRequest::new(
        RequestId::new("image-registry").unwrap(),
        Input::bytes(bytes),
        source("animation.png"),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );
    let envelope = registry.dispatch("png", request, None).unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert_eq!(envelope.kind, grist::core::ArtifactKind::Image);

    let mismatch = ParseRequest::new(
        RequestId::new("image-mismatch").unwrap(),
        Input::bytes(jpeg()),
        source("wrong.png"),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );
    assert_eq!(
        registry.dispatch("png", mismatch, None).unwrap().status,
        OperationStatus::Failed
    );

    #[cfg(feature = "schemas")]
    for schema in ["image", "image-envelope", "image-options"] {
        assert!(
            grist::schema::schema_json(schema).is_some(),
            "missing {schema}"
        );
    }
}
