#![cfg(feature = "media")]

use grist::core::{
    BudgetProfile, BudgetSelection, Input, Limits, OperationStatus, ParseRequest, ProviderSet,
    RequestId, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentKind, DocumentNodeKind, ToDocumentGraph,
};
use grist::image::{ImageFormat, ImageOptions, parse_image_bytes};
use grist::registry::builtin_parser_registry;
use std::path::Path;

fn source(name: &str) -> SourceInfo {
    SourceInfo::stdin(name)
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
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
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    out.extend(png_chunk(b"IHDR", &header));
    for chunk in extra {
        out.extend_from_slice(chunk);
    }
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn animated_png() -> Vec<u8> {
    let mut animation = Vec::new();
    let mut control = Vec::new();
    control.extend_from_slice(&2u32.to_be_bytes());
    control.extend_from_slice(&0u32.to_be_bytes());
    animation.push(png_chunk(b"acTL", &control));
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
        animation.push(png_chunk(b"fcTL", &frame));
    }
    png(2, 1, &animation)
}

fn jpeg() -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0, 2, 0, 3, 1, 1, 0x11, 0];
    out.extend_from_slice(&[0xff, 0xfe, 0, 7]);
    out.extend_from_slice(b"hello");
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

fn heif() -> Vec<u8> {
    let mut ftyp = Vec::new();
    ftyp.extend_from_slice(&24u32.to_be_bytes());
    ftyp.extend_from_slice(b"ftypheic");
    ftyp.extend_from_slice(&0u32.to_be_bytes());
    ftyp.extend_from_slice(b"mif1heic");

    let mut ispe = Vec::new();
    ispe.extend_from_slice(&20u32.to_be_bytes());
    ispe.extend_from_slice(b"ispe");
    ispe.extend_from_slice(&0u32.to_be_bytes());
    ispe.extend_from_slice(&4u32.to_be_bytes());
    ispe.extend_from_slice(&3u32.to_be_bytes());
    let mut ipco = Vec::new();
    ipco.extend_from_slice(&28u32.to_be_bytes());
    ipco.extend_from_slice(b"ipco");
    ipco.extend(ispe);
    let mut iprp = Vec::new();
    iprp.extend_from_slice(&36u32.to_be_bytes());
    iprp.extend_from_slice(b"iprp");
    iprp.extend(ipco);
    let mut meta = Vec::new();
    meta.extend_from_slice(&48u32.to_be_bytes());
    meta.extend_from_slice(b"meta");
    meta.extend_from_slice(&0u32.to_be_bytes());
    meta.extend(iprp);
    ftyp.extend(meta);
    ftyp
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
fn detection_registry_schema_and_explicit_format_gate_are_integrated() {
    let registry = builtin_parser_registry().unwrap();
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
