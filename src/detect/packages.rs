use super::Signal;
use crate::core::{DetectionEvidenceKind, Diagnostic};
#[cfg(any(
    feature = "epub",
    feature = "word-ooxml",
    feature = "presentation-ooxml",
    feature = "spreadsheet-ooxml",
    feature = "spreadsheet-odf",
    feature = "presentation-odf",
    feature = "odf-word"
))]
use std::io::{Cursor, Read};

const MAX_ENTRIES: usize = 16_384;

struct ZipEntry<'a> {
    name: String,
    stored_data: Option<&'a [u8]>,
}

pub(super) fn zip_signals(bytes: &[u8], diagnostics: &mut Vec<Diagnostic>) -> Vec<Signal> {
    let entries = match central_entries(bytes) {
        Some(entries) => entries,
        None => {
            diagnostics.push(
                Diagnostic::warning(
                    "grist.detect",
                    "detect.package.zip_directory_unavailable",
                    "ZIP signature is present but the bounded central directory could not be read",
                )
                .partial(),
            );
            return Vec::new();
        }
    };
    let names = entries
        .iter()
        .map(|entry| entry.name.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let has = |name: &str| names.iter().any(|entry| entry == name);
    let package = |format: &str, media: &str, description: &str| {
        Signal::new(
            format,
            Some(media),
            0.995,
            DetectionEvidenceKind::ContainerManifest,
            description,
        )
        .decisive()
    };
    let content_types = stored_text(&entries, "[Content_Types].xml")
        .map(str::to_string)
        .or_else(|| bounded_content_types(bytes))
        .map(|manifest| manifest.to_ascii_lowercase());
    let signal = if content_types.as_deref().is_some_and(|manifest| {
        manifest.contains("application/vnd.ms-word.document.macroenabled.main+xml")
    }) {
        Some(package(
            "docm",
            "application/vnd.ms-word.document.macroEnabled.12",
            "ZIP [Content_Types].xml declares a macro-enabled OOXML Word document",
        ))
    } else if content_types.as_deref().is_some_and(|manifest| {
        manifest.contains("application/vnd.ms-word.template.macroenabledtemplate.main+xml")
    }) {
        Some(package(
            "dotm",
            "application/vnd.ms-word.template.macroEnabled.12",
            "ZIP [Content_Types].xml declares a macro-enabled OOXML Word template",
        ))
    } else if content_types.as_deref().is_some_and(|manifest| {
        manifest.contains(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
        )
    }) {
        Some(package(
            "dotx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
            "ZIP [Content_Types].xml declares an OOXML Word template",
        ))
    } else if content_types.as_deref().is_some_and(|manifest| {
        manifest.contains("application/vnd.ms-powerpoint.presentation.macroenabled.main+xml")
    }) {
        Some(package(
            "pptm",
            "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
            "ZIP [Content_Types].xml declares a macro-enabled OOXML presentation",
        ))
    } else if content_types.as_deref().is_some_and(|manifest| {
        manifest.contains(
            "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml",
        )
    }) {
        Some(package(
            "potx",
            "application/vnd.openxmlformats-officedocument.presentationml.template",
            "ZIP [Content_Types].xml declares an OOXML presentation template",
        ))
    } else if content_types.as_deref().is_some_and(|manifest| {
        manifest.contains(
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml",
        )
    }) {
        Some(package(
            "ppsx",
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
            "ZIP [Content_Types].xml declares an OOXML presentation slideshow",
        ))
    } else if content_types.as_deref().is_some_and(|manifest| {
        manifest.contains("application/vnd.ms-excel.sheet.macroenabled.main+xml")
    }) {
        Some(package(
            "xlsm",
            "application/vnd.ms-excel.sheet.macroEnabled.12",
            "ZIP [Content_Types].xml declares a macro-enabled OOXML workbook",
        ))
    } else if content_types
        .as_deref()
        .is_some_and(|manifest| manifest.contains("wordprocessingml.document.main+xml"))
    {
        Some(package(
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "ZIP [Content_Types].xml declares an OOXML Word document",
        ))
    } else if content_types
        .as_deref()
        .is_some_and(|manifest| manifest.contains("presentationml.presentation.main+xml"))
    {
        Some(package(
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "ZIP [Content_Types].xml declares an OOXML presentation",
        ))
    } else if content_types
        .as_deref()
        .is_some_and(|manifest| manifest.contains("spreadsheetml.sheet.main+xml"))
    {
        Some(package(
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "ZIP [Content_Types].xml declares an OOXML spreadsheet",
        ))
    } else if has("word/document.xml") {
        Some(package(
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "ZIP package contains word/document.xml",
        ))
    } else if has("ppt/presentation.xml") {
        Some(package(
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "ZIP package contains ppt/presentation.xml",
        ))
    } else if has("xl/workbook.xml") {
        Some(package(
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "ZIP package contains xl/workbook.xml",
        ))
    } else if let Some(media) = stored_mimetype(&entries) {
        odf_or_epub_signal(media, &package)
    } else if has("meta-inf/container.xml") {
        Some(package(
            "epub",
            "application/epub+zip",
            "ZIP package contains META-INF/container.xml",
        ))
    } else {
        None
    };
    signal.into_iter().collect()
}

fn odf_or_epub_signal(
    media: &str,
    package: &impl Fn(&str, &str, &str) -> Signal,
) -> Option<Signal> {
    Some(match media {
        "application/epub+zip" => package("epub", media, "ZIP mimetype manifest declares EPUB"),
        "application/vnd.oasis.opendocument.text" => package(
            "odt",
            media,
            "ZIP mimetype manifest declares OpenDocument text",
        ),
        "application/vnd.oasis.opendocument.text-template" => package(
            "ott",
            media,
            "ZIP mimetype manifest declares OpenDocument text template",
        ),
        "application/vnd.oasis.opendocument.presentation" => package(
            "odp",
            media,
            "ZIP mimetype manifest declares OpenDocument presentation",
        ),
        "application/vnd.oasis.opendocument.presentation-template" => package(
            "otp",
            media,
            "ZIP mimetype manifest declares OpenDocument presentation template",
        ),
        "application/vnd.oasis.opendocument.spreadsheet" => package(
            "ods",
            media,
            "ZIP mimetype manifest declares OpenDocument spreadsheet",
        ),
        "application/vnd.oasis.opendocument.spreadsheet-template" => package(
            "ots",
            media,
            "ZIP mimetype manifest declares OpenDocument spreadsheet template",
        ),
        _ => return None,
    })
}

fn stored_mimetype<'a>(entries: &'a [ZipEntry<'a>]) -> Option<&'a str> {
    stored_text(entries, "mimetype").map(str::trim)
}

fn stored_text<'a>(entries: &'a [ZipEntry<'a>], name: &str) -> Option<&'a str> {
    let data = entries
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(name))?
        .stored_data?;
    std::str::from_utf8(data).ok()
}

#[cfg(any(
    feature = "epub",
    feature = "word-ooxml",
    feature = "presentation-ooxml",
    feature = "spreadsheet-ooxml",
    feature = "spreadsheet-odf",
    feature = "presentation-odf",
    feature = "odf-word"
))]
fn bounded_content_types(bytes: &[u8]) -> Option<String> {
    const MAX_CONTENT_TYPES_BYTES: u64 = 1_048_576;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let file = archive.by_name("[Content_Types].xml").ok()?;
    if file.encrypted() || file.is_dir() || file.size() > MAX_CONTENT_TYPES_BYTES {
        return None;
    }
    let size = file.size();
    let mut bounded = file.take(MAX_CONTENT_TYPES_BYTES + 1);
    let mut output = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    bounded.read_to_end(&mut output).ok()?;
    (u64::try_from(output.len()).ok()? <= MAX_CONTENT_TYPES_BYTES)
        .then(|| String::from_utf8(output).ok())
        .flatten()
}

#[cfg(not(any(
    feature = "epub",
    feature = "word-ooxml",
    feature = "presentation-ooxml",
    feature = "spreadsheet-ooxml",
    feature = "spreadsheet-odf",
    feature = "presentation-odf",
    feature = "odf-word"
)))]
fn bounded_content_types(_bytes: &[u8]) -> Option<String> {
    None
}

fn central_entries(bytes: &[u8]) -> Option<Vec<ZipEntry<'_>>> {
    let search_start = bytes.len().saturating_sub(65_557);
    let eocd = (search_start..bytes.len().saturating_sub(3))
        .rev()
        .find(|offset| bytes.get(*offset..*offset + 4) == Some(b"PK\x05\x06"))?;
    let count = usize::from(le_u16(bytes, eocd + 10)?).min(MAX_ENTRIES);
    let mut offset = usize::try_from(le_u32(bytes, eocd + 16)?).ok()?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if bytes.get(offset..offset + 4) != Some(b"PK\x01\x02") {
            return None;
        }
        let compression = le_u16(bytes, offset + 10)?;
        let compressed_size = usize::try_from(le_u32(bytes, offset + 20)?).ok()?;
        let name_len = usize::from(le_u16(bytes, offset + 28)?);
        let extra_len = usize::from(le_u16(bytes, offset + 30)?);
        let comment_len = usize::from(le_u16(bytes, offset + 32)?);
        let local_offset = usize::try_from(le_u32(bytes, offset + 42)?).ok()?;
        let name_start = offset.checked_add(46)?;
        let name_end = name_start.checked_add(name_len)?;
        let name = String::from_utf8_lossy(bytes.get(name_start..name_end)?).replace('\\', "/");
        let stored_data = (compression == 0)
            .then(|| stored_entry(bytes, local_offset, compressed_size))
            .flatten();
        entries.push(ZipEntry { name, stored_data });
        offset = name_end.checked_add(extra_len)?.checked_add(comment_len)?;
    }
    Some(entries)
}

fn stored_entry(bytes: &[u8], offset: usize, size: usize) -> Option<&[u8]> {
    if bytes.get(offset..offset + 4) != Some(b"PK\x03\x04") {
        return None;
    }
    let name_len = usize::from(le_u16(bytes, offset + 26)?);
    let extra_len = usize::from(le_u16(bytes, offset + 28)?);
    let start = offset
        .checked_add(30)?
        .checked_add(name_len)?
        .checked_add(extra_len)?;
    bytes.get(start..start.checked_add(size)?)
}

fn le_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
