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
    } else if is_tar(bytes) {
        Some(magic(
            "tar",
            "application/x-tar",
            "TAR header signature and checksum",
            0.98,
        ))
    } else {
        media_signature(bytes)
            .or_else(|| image_signature(bytes))
            .or_else(|| other_signature(bytes))
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

fn media_signature(bytes: &[u8]) -> Option<Signal> {
    if bytes.starts_with(b"ID3")
        || (!has_unicode_bom(bytes) && bytes.get(..4).is_some_and(valid_mpeg_audio_header))
    {
        Some(magic("mp3", "audio/mpeg", "ID3/MPEG audio signature", 0.97))
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        Some(magic("wav", "audio/wav", "RIFF WAVE form signature", 0.99))
    } else if bytes.starts_with(b"fLaC") {
        Some(magic("flac", "audio/flac", "FLAC stream marker", 0.99))
    } else if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        ebml_doc_type(bytes)
            .filter(|value| matches!(*value, b"matroska" | b"webm"))
            .map(|_| {
                magic(
                    "matroska",
                    "video/x-matroska",
                    "Matroska/WebM EBML DocType",
                    0.99,
                )
            })
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        match iso_bmff_family(bytes) {
            Some(IsoBmffFamily::QuickTime) => Some(magic(
                "quicktime",
                "video/quicktime",
                "QuickTime major or compatible brand",
                0.99,
            )),
            Some(IsoBmffFamily::Mp4) => Some(magic(
                "mp4",
                "video/mp4",
                "MP4 major or compatible brand",
                0.98,
            )),
            Some(IsoBmffFamily::Heif) | None => None,
        }
    } else {
        None
    }
}

fn has_unicode_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xef, 0xbb, 0xbf])
        || bytes.starts_with(&[0xff, 0xfe])
        || bytes.starts_with(&[0xfe, 0xff])
        || bytes.starts_with(&[0x00, 0x00, 0xfe, 0xff])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IsoBmffFamily {
    Heif,
    QuickTime,
    Mp4,
}

fn iso_bmff_family(bytes: &[u8]) -> Option<IsoBmffFamily> {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return None;
    }
    let size = usize::try_from(u32::from_be_bytes(bytes[..4].try_into().ok()?)).ok()?;
    if size < 16 || size > bytes.len() {
        return None;
    }
    let brands = std::iter::once(&bytes[8..12]).chain(bytes[16..size].chunks_exact(4));
    let mut family = None;
    for brand in brands {
        if matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis"
        ) {
            return Some(IsoBmffFamily::Heif);
        }
        if brand == b"qt  " {
            family = Some(IsoBmffFamily::QuickTime);
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
            family = Some(IsoBmffFamily::Mp4);
        }
    }
    family
}

fn valid_mpeg_audio_header(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && bytes[0] == 0xff
        && bytes[1] & 0xe0 == 0xe0
        && (bytes[1] >> 3) & 3 != 1
        && (bytes[1] >> 1) & 3 != 0
        && !matches!(bytes[2] >> 4, 0 | 15)
        && (bytes[2] >> 2) & 3 != 3
}

fn ebml_doc_type(bytes: &[u8]) -> Option<&[u8]> {
    if !bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return None;
    }
    let (header_size, size_len) = ebml_vint(bytes, 4, false)?;
    let header_start = 4usize.checked_add(size_len)?;
    let header_end = header_start.checked_add(usize::try_from(header_size).ok()?)?;
    if header_end > bytes.len() {
        return None;
    }
    let mut cursor = header_start;
    while cursor < header_end {
        let (id, id_len) = ebml_vint(bytes, cursor, true)?;
        let (size, value_len) = ebml_vint(bytes, cursor.checked_add(id_len)?, false)?;
        let value_start = cursor.checked_add(id_len)?.checked_add(value_len)?;
        let value_end = value_start.checked_add(usize::try_from(size).ok()?)?;
        if value_end > header_end {
            return None;
        }
        if id == 0x4282 {
            return Some(&bytes[value_start..value_end]);
        }
        cursor = value_end;
    }
    None
}

fn ebml_vint(bytes: &[u8], offset: usize, preserve_marker: bool) -> Option<(u64, usize)> {
    let first = *bytes.get(offset)?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || offset.checked_add(len)? > bytes.len() {
        return None;
    }
    let mut value = if preserve_marker {
        u64::from(first)
    } else {
        u64::from(first & (0xff >> len))
    };
    for byte in &bytes[offset + 1..offset + len] {
        value = value.checked_mul(256)?.checked_add(u64::from(*byte))?;
    }
    Some((value, len))
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
    } else if heif_signature(bytes) {
        Some(magic(
            "heif",
            "image/heif",
            "HEIF/HEIC compatible brand",
            0.99,
        ))
    } else if has_svg_root(bytes) {
        Some(magic("svg", "image/svg+xml", "SVG root element", 0.98))
    } else {
        None
    }
}

pub(crate) fn has_svg_root(bytes: &[u8]) -> bool {
    xml_root_name(bytes)
        .is_some_and(|name| name.rsplit(|byte| *byte == b':').next() == Some(b"svg"))
}

#[cfg(feature = "media")]
pub(crate) fn has_ttml_root(bytes: &[u8]) -> bool {
    let Some((start, end)) = xml_root_bounds(bytes) else {
        return false;
    };
    let name = &bytes[start..end];
    if name.rsplit(|byte| *byte == b':').next() != Some(b"tt") {
        return false;
    }
    let prefix = name
        .iter()
        .position(|byte| *byte == b':')
        .map(|colon| &name[..colon]);
    root_namespace(bytes, end, prefix).is_some_and(|namespace| {
        matches!(
            namespace,
            b"http://www.w3.org/ns/ttml" | b"http://www.w3.org/2006/10/ttaf1"
        )
    })
}

pub(crate) fn xml_root_name(bytes: &[u8]) -> Option<&[u8]> {
    let (start, end) = xml_root_bounds(bytes)?;
    Some(&bytes[start..end])
}

fn xml_root_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    std::str::from_utf8(bytes).ok()?;
    let mut cursor = usize::from(bytes.starts_with(b"\xef\xbb\xbf")) * 3;
    loop {
        cursor += bytes
            .get(cursor..)?
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        let tail = bytes.get(cursor..)?;
        if tail.starts_with(b"<?") {
            cursor += find_terminator(tail, b"?>")? + 2;
        } else if tail.starts_with(b"<!--") {
            cursor += find_terminator(tail, b"-->")? + 3;
        } else if starts_ascii_case_insensitive(tail, b"<!doctype") {
            cursor += declaration_end(tail)?;
        } else {
            break;
        }
    }
    let tail = bytes.get(cursor..)?;
    if tail.first() != Some(&b'<') || matches!(tail.get(1), Some(b'/' | b'!' | b'?')) {
        return None;
    }
    let start = cursor + 1;
    let length = bytes[start..]
        .iter()
        .take_while(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
        .count();
    (length > 0).then_some((start, start + length))
}

#[cfg(feature = "media")]
fn root_namespace<'a>(
    bytes: &'a [u8],
    mut cursor: usize,
    prefix: Option<&[u8]>,
) -> Option<&'a [u8]> {
    loop {
        cursor += bytes
            .get(cursor..)?
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        if matches!(bytes.get(cursor), Some(b'/' | b'>')) {
            return None;
        }
        let name_start = cursor;
        cursor += bytes[cursor..]
            .iter()
            .take_while(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'=' | b'/' | b'>'))
            .count();
        let attribute_name = &bytes[name_start..cursor];
        cursor += bytes[cursor..]
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        if bytes.get(cursor) != Some(&b'=') {
            return None;
        }
        cursor += 1;
        cursor += bytes[cursor..]
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        let quote = *bytes.get(cursor)?;
        if !matches!(quote, b'\'' | b'"') {
            return None;
        }
        cursor += 1;
        let value_start = cursor;
        cursor += bytes[cursor..].iter().position(|byte| *byte == quote)?;
        let value = &bytes[value_start..cursor];
        cursor += 1;
        let is_namespace = match prefix {
            Some(prefix) => attribute_name.starts_with(b"xmlns:") && &attribute_name[6..] == prefix,
            None => attribute_name == b"xmlns",
        };
        if is_namespace {
            return Some(value);
        }
    }
}

fn find_terminator(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn starts_ascii_case_insensitive(value: &[u8], prefix: &[u8]) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

fn declaration_end(bytes: &[u8]) -> Option<usize> {
    let mut quote = None;
    let mut subset_depth = 0u64;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if let Some(current) = quote {
            if byte == current {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'[' => subset_depth = subset_depth.saturating_add(1),
            b']' => subset_depth = subset_depth.saturating_sub(1),
            b'>' if subset_depth == 0 => return Some(index + 1),
            _ => {}
        }
    }
    None
}

fn heif_signature(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes(bytes[0..4].try_into().unwrap()) as usize;
    if size < 16 || size > bytes.len() {
        return false;
    }
    let supported = |brand: &[u8]| {
        matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis"
        )
    };
    supported(&bytes[8..12]) || bytes[16..size].chunks_exact(4).any(supported)
}

fn other_signature(bytes: &[u8]) -> Option<Signal> {
    if bytes.starts_with(&[0xd9, 0xd9, 0xf7]) {
        Some(magic(
            "cbor",
            "application/cbor",
            "self-described CBOR tag",
            0.99,
        ))
    } else if bytes.starts_with(b"SQLite format 3\0") {
        Some(magic(
            "sqlite",
            "application/vnd.sqlite3",
            "SQLite database header",
            0.995,
        ))
    } else if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        if contains_utf16le_ascii(bytes, "__properties_version1.0")
            && contains_utf16le_ascii(bytes, "__substg1.0_")
        {
            Some(magic(
                "msg",
                "application/vnd.ms-outlook",
                "OLE compound file contains Outlook MSG property streams",
                0.998,
            ))
        } else {
            Some(magic(
                "ole_compound",
                "application/x-ole-storage",
                "OLE compound-file signature",
                0.99,
            ))
        }
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
    } else if bytes.len() >= 12 && bytes.starts_with(b"ARROW1") && bytes.ends_with(b"ARROW1") {
        Some(magic(
            "arrow",
            "application/vnd.apache.arrow.file",
            "Arrow IPC file boundary signatures",
            0.995,
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

pub(super) fn is_tar(bytes: &[u8]) -> bool {
    if bytes.len() < 512 {
        return false;
    }
    if bytes[..512].iter().all(|byte| *byte == 0) {
        return bytes.len() >= 1024 && bytes[512..1024].iter().all(|byte| *byte == 0);
    }
    let Some(stored) = parse_tar_octal(&bytes[148..156]) else {
        return false;
    };
    let calculated = bytes[..512]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum::<u64>();
    stored == calculated
}

fn parse_tar_octal(bytes: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(bytes).ok()?.trim_matches(['\0', ' ']);
    (!text.is_empty())
        .then(|| u64::from_str_radix(text, 8).ok())
        .flatten()
}
fn contains_utf16le_ascii(bytes: &[u8], needle: &str) -> bool {
    let encoded = needle
        .bytes()
        .flat_map(|byte| [byte, 0])
        .collect::<Vec<_>>();
    bytes.windows(encoded.len()).any(|window| window == encoded)
}
