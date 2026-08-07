use super::Signal;
use crate::core::{DetectionEvidenceKind, Diagnostic};

pub(super) fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x50, 0x4b, 0x03, 0x04])
        || bytes.starts_with(&[0x50, 0x4b, 0x05, 0x06])
        || bytes.starts_with(&[0x50, 0x4b, 0x07, 0x08])
}

fn magic(format: &str, media: &str, description: &str, weight: f32) -> Signal {
    Signal::new(
        format,
        Some(media),
        weight,
        DetectionEvidenceKind::MagicBytes,
        description,
    )
    .decisive()
}

pub(super) fn signals(bytes: &[u8], diagnostics: &mut Vec<Diagnostic>) -> Vec<Signal> {
    let signal = if bytes.starts_with(&[0x25, 0x50, 0x44, 0x46, 0x2d]) {
        Some(magic(
            "pdf",
            "application/pdf",
            "PDF header signature",
            0.99,
        ))
    } else if bytes.starts_with(&[0x25, 0x50, 0x44, 0x46]) {
        truncated_pdf(diagnostics)
    } else if is_zip(bytes) {
        Some(magic(
            "zip",
            "application/zip",
            "ZIP container signature",
            0.90,
        ))
    } else {
        image_signature(bytes).or_else(|| other_signature(bytes))
    };
    signal.into_iter().collect()
}

fn truncated_pdf(diagnostics: &mut Vec<Diagnostic>) -> Option<Signal> {
    diagnostics.push(
        Diagnostic::warning(
            "grist.detect",
            "detect.signature.pdf_truncated",
            "PDF signature is present without a complete version header",
        )
        .partial(),
    );
    Some(magic(
        "pdf",
        "application/pdf",
        "truncated PDF header signature",
        0.88,
    ))
}

fn image_signature(bytes: &[u8]) -> Option<Signal> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(magic("png", "image/png", "PNG signature", 0.995))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(magic(
            "jpeg",
            "image/jpeg",
            "JPEG start-of-image signature",
            0.99,
        ))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(magic("gif", "image/gif", "GIF signature", 0.99))
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some(magic("tiff", "image/tiff", "TIFF signature", 0.99))
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(magic(
            "webp",
            "image/webp",
            "RIFF WEBP form signature",
            0.99,
        ))
    } else if bytes.starts_with(b"BM") {
        Some(magic("bmp", "image/bmp", "BMP signature", 0.96))
    } else {
        None
    }
}

fn other_signature(bytes: &[u8]) -> Option<Signal> {
    if bytes.starts_with(b"SQLite format 3\0") {
        Some(magic(
            "sqlite",
            "application/vnd.sqlite3",
            "SQLite database header",
            0.995,
        ))
    } else if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        Some(magic(
            "ole_compound",
            "application/x-ole-storage",
            "OLE compound-file signature",
            0.99,
        ))
    } else if bytes.starts_with(b"{\\rtf") {
        Some(magic(
            "rtf",
            "application/rtf",
            "RTF control-word signature",
            0.98,
        ))
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        Some(magic("gzip", "application/gzip", "GZIP signature", 0.99))
    } else if bytes.starts_with(b"BZh") {
        Some(magic(
            "bzip2",
            "application/x-bzip2",
            "BZIP2 signature",
            0.99,
        ))
    } else if bytes.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        Some(magic("xz", "application/x-xz", "XZ signature", 0.99))
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Some(magic(
            "zstd",
            "application/zstd",
            "Zstandard frame signature",
            0.99,
        ))
    } else if bytes.starts_with(&[0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c]) {
        Some(magic(
            "seven_zip",
            "application/x-7z-compressed",
            "7z signature",
            0.99,
        ))
    } else if bytes.len() >= 8 && bytes.starts_with(b"PAR1") && bytes.ends_with(b"PAR1") {
        Some(magic(
            "parquet",
            "application/vnd.apache.parquet",
            "Parquet boundary signatures",
            0.99,
        ))
    } else {
        None
    }
}
