//! BOM, transport, HTML, XML, signature, and heuristic encoding selection.

use super::codecs::{MappedText, contains_valid_utf8_multibyte, decode};
use super::model::{
    BomKind, DecodeContext, DecodeError, DecodeIssue, DecodeIssueKind, DecodeOptions,
    DecodedByteRange, EncodingDeclaration, EncodingDeclarationSource, RawByteRange, TextEncoding,
};

pub(crate) struct Selection {
    pub encoding: TextEncoding,
    pub bom: Option<BomKind>,
    pub content_start: usize,
    pub declarations: Vec<EncodingDeclaration>,
    pub conflict_issues: Vec<DecodeIssue>,
}

pub(crate) fn select(bytes: &[u8], options: &DecodeOptions) -> Result<Selection, DecodeError> {
    let bom = detect_bom(bytes);
    let signature = bom
        .as_ref()
        .map(|(_, encoding, _)| encoding.clone())
        .or_else(|| xml_signature(bytes).map(|(encoding, _)| encoding));
    let content_start = bom.as_ref().map_or(0, |(_, _, length)| *length);
    let preliminary_encoding = signature.clone().unwrap_or(TextEncoding::Windows1252);
    let preliminary = decode(
        &bytes[content_start..],
        &preliminary_encoding,
        content_start,
    );
    let context = resolve_context(options.context, &preliminary.text);

    let mut declarations = Vec::new();
    if let Some((kind, encoding, length)) = &bom {
        declarations.push(EncodingDeclaration {
            source: EncodingDeclarationSource::ByteOrderMark,
            label: encoding.label().into(),
            encoding: Some(encoding.clone()),
            raw_range: Some(RawByteRange::from_usize(0, *length)),
            decoded_range: None,
            selected: false,
        });
        let _ = kind;
    }
    if let Some(label) = options.transport_encoding.as_deref() {
        declarations.push(declaration(
            EncodingDeclarationSource::Transport,
            label,
            None,
            None,
            context,
            signature.as_ref(),
        ));
    }
    if context == DecodeContext::Xml
        && bom.is_none()
        && let Some((encoding, length)) = xml_signature(bytes)
    {
        declarations.push(EncodingDeclaration {
            source: EncodingDeclarationSource::XmlSignature,
            label: encoding.label().into(),
            encoding: Some(encoding),
            raw_range: Some(RawByteRange::from_usize(0, length)),
            decoded_range: Some(DecodedByteRange::from_usize(
                0,
                preliminary.text.len().min(4),
            )),
            selected: false,
        });
    }
    if let Some((label, decoded_start, decoded_end)) =
        internal_declaration(context, &preliminary.text)
    {
        let raw_range = preliminary
            .raw_for_decoded_range(decoded_start, decoded_end)
            .map(|(start, end)| RawByteRange::from_usize(start, end));
        declarations.push(declaration(
            match context {
                DecodeContext::Html => EncodingDeclarationSource::HtmlMeta,
                DecodeContext::Xml => EncodingDeclarationSource::XmlDeclaration,
                DecodeContext::Auto | DecodeContext::PlainText => unreachable!(),
            },
            &label,
            raw_range,
            Some(DecodedByteRange::from_usize(decoded_start, decoded_end)),
            context,
            signature.as_ref(),
        ));
    }

    let selected_index = declarations
        .iter()
        .position(|declaration| declaration.source == EncodingDeclarationSource::ByteOrderMark)
        .or_else(|| {
            declarations
                .iter()
                .position(|declaration| declaration.source == EncodingDeclarationSource::Transport)
        })
        .or_else(|| {
            declarations.iter().position(|declaration| {
                declaration.source == EncodingDeclarationSource::XmlSignature
            })
        })
        .or_else(|| {
            declarations.iter().position(|declaration| {
                matches!(
                    declaration.source,
                    EncodingDeclarationSource::HtmlMeta | EncodingDeclarationSource::XmlDeclaration
                )
            })
        });

    let encoding = if let Some(index) = selected_index {
        declarations[index].selected = true;
        declarations[index]
            .encoding
            .clone()
            .ok_or_else(|| DecodeError::UnsupportedEncoding {
                label: declarations[index].label.clone(),
                raw_range: declarations[index].raw_range,
            })?
    } else {
        let encoding = default_encoding(context, bytes);
        declarations.push(EncodingDeclaration {
            source: if std::str::from_utf8(bytes).is_ok() {
                EncodingDeclarationSource::Default
            } else {
                EncodingDeclarationSource::Heuristic
            },
            label: encoding.label().into(),
            encoding: Some(encoding.clone()),
            raw_range: None,
            decoded_range: None,
            selected: true,
        });
        encoding
    };

    let selected_range = declarations
        .iter()
        .find(|declaration| declaration.selected)
        .and_then(|declaration| declaration.raw_range);
    let mut conflict_issues = Vec::new();
    for declaration in &declarations {
        if declaration.selected {
            continue;
        }
        let Some(declared) = declaration.encoding.as_ref() else {
            if let Some(raw_range) = declaration.raw_range.or(selected_range) {
                conflict_issues.push(DecodeIssue {
                    kind: DecodeIssueKind::UnsupportedDeclaration,
                    raw_range,
                    decoded_range: declaration.decoded_range,
                    conflicting_raw_range: selected_range,
                    message: format!(
                        "unsupported encoding declaration {:?} was ignored because {} was selected",
                        declaration.label,
                        encoding.label()
                    ),
                });
            }
            continue;
        };
        if !encoding.same_family(declared)
            && let Some(raw_range) = declaration.raw_range.or(selected_range)
        {
            conflict_issues.push(DecodeIssue {
                kind: DecodeIssueKind::EncodingConflict,
                raw_range,
                decoded_range: declaration.decoded_range,
                conflicting_raw_range: selected_range,
                message: format!(
                    "{} declaration conflicts with selected {} encoding",
                    declaration.label,
                    encoding.label()
                ),
            });
        }
    }

    Ok(Selection {
        encoding,
        bom: bom.map(|(kind, _, _)| kind),
        content_start,
        declarations,
        conflict_issues,
    })
}

fn declaration(
    source: EncodingDeclarationSource,
    label: &str,
    raw_range: Option<RawByteRange>,
    decoded_range: Option<DecodedByteRange>,
    context: DecodeContext,
    endian_hint: Option<&TextEncoding>,
) -> EncodingDeclaration {
    EncodingDeclaration {
        source,
        label: label.trim().to_ascii_lowercase(),
        encoding: normalize_label(label, context, endian_hint),
        raw_range,
        decoded_range,
        selected: false,
    }
}

fn normalize_label(
    label: &str,
    context: DecodeContext,
    endian_hint: Option<&TextEncoding>,
) -> Option<TextEncoding> {
    let compact = label
        .trim()
        .trim_matches(['\'', '"'])
        .to_ascii_lowercase()
        .replace('_', "-");
    let encoding = match compact.as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => TextEncoding::Utf8,
        "utf-16" | "utf16" => match endian_hint {
            Some(TextEncoding::Utf16Le) => TextEncoding::Utf16Le,
            Some(TextEncoding::Utf16Be) => TextEncoding::Utf16Be,
            _ => TextEncoding::Utf16Be,
        },
        "utf-16le" | "utf16le" => TextEncoding::Utf16Le,
        "utf-16be" | "utf16be" => TextEncoding::Utf16Be,
        "utf-32" | "utf32" => match endian_hint {
            Some(TextEncoding::Utf32Le) => TextEncoding::Utf32Le,
            Some(TextEncoding::Utf32Be) => TextEncoding::Utf32Be,
            _ => TextEncoding::Utf32Be,
        },
        "utf-32le" | "utf32le" => TextEncoding::Utf32Le,
        "utf-32be" | "utf32be" => TextEncoding::Utf32Be,
        "windows-1252" | "cp1252" | "x-cp1252" | "iso-8859-1" | "latin1" | "latin-1"
        | "us-ascii" => TextEncoding::Windows1252,
        _ => normalize_extended(&compact)?,
    };
    if context == DecodeContext::Html
        && matches!(
            encoding,
            TextEncoding::Utf16Le
                | TextEncoding::Utf16Be
                | TextEncoding::Utf32Le
                | TextEncoding::Utf32Be
        )
    {
        Some(TextEncoding::Utf8)
    } else {
        Some(encoding)
    }
}

#[cfg(feature = "extended-encodings")]
fn normalize_extended(label: &str) -> Option<TextEncoding> {
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())?;
    Some(TextEncoding::Other(encoding.name().to_ascii_lowercase()))
}

#[cfg(not(feature = "extended-encodings"))]
fn normalize_extended(_label: &str) -> Option<TextEncoding> {
    None
}

fn default_encoding(context: DecodeContext, bytes: &[u8]) -> TextEncoding {
    if context == DecodeContext::Html {
        TextEncoding::Windows1252
    } else if context == DecodeContext::Xml
        || std::str::from_utf8(bytes).is_ok()
        || contains_valid_utf8_multibyte(bytes)
    {
        TextEncoding::Utf8
    } else {
        TextEncoding::Windows1252
    }
}

fn resolve_context(context: DecodeContext, text: &str) -> DecodeContext {
    if context != DecodeContext::Auto {
        return context;
    }
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let lower = trimmed
        .chars()
        .take(128)
        .collect::<String>()
        .to_ascii_lowercase();
    if lower.starts_with("<?xml") {
        DecodeContext::Xml
    } else if ["<!doctype html", "<html", "<head", "<body", "<meta"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        DecodeContext::Html
    } else {
        DecodeContext::PlainText
    }
}

fn internal_declaration(context: DecodeContext, text: &str) -> Option<(String, usize, usize)> {
    match context {
        DecodeContext::Html => html_declaration(text),
        DecodeContext::Xml => xml_declaration(text),
        DecodeContext::Auto | DecodeContext::PlainText => None,
    }
}

fn html_declaration(text: &str) -> Option<(String, usize, usize)> {
    let limit = text.len().min(4096);
    let prefix = &text[..floor_char_boundary(text, limit)];
    let lower = prefix.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find("<meta") {
        let meta_start = search_from + relative;
        let meta_end = lower[meta_start..]
            .find('>')
            .map_or(lower.len(), |end| meta_start + end + 1);
        if let Some(found) = attribute_encoding(&prefix[meta_start..meta_end], "charset") {
            return Some((found.0, meta_start + found.1, meta_start + found.2));
        }
        search_from = meta_end;
    }
    None
}

fn xml_declaration(text: &str) -> Option<(String, usize, usize)> {
    let prefix_end = floor_char_boundary(text, text.len().min(1024));
    let prefix = &text[..prefix_end];
    let trimmed_start = prefix.len() - prefix.trim_start_matches('\u{feff}').len();
    let declaration = &prefix[trimmed_start..];
    if !declaration
        .get(..5)
        .is_some_and(|value| value.eq_ignore_ascii_case("<?xml"))
    {
        return None;
    }
    let end = declaration.find("?>").unwrap_or(declaration.len());
    attribute_encoding(&declaration[..end], "encoding")
        .map(|(label, start, end)| (label, trimmed_start + start, trimmed_start + end))
}

fn attribute_encoding(text: &str, name: &str) -> Option<(String, usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(relative) = lower[offset..].find(name) {
        let name_start = offset + relative;
        let mut cursor = name_start + name.len();
        while lower
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        if lower.as_bytes().get(cursor) != Some(&b'=') {
            offset = cursor;
            continue;
        }
        cursor += 1;
        while lower
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        let quote = lower.as_bytes().get(cursor).copied();
        if matches!(quote, Some(b'\'' | b'"')) {
            cursor += 1;
        }
        let value_start = cursor;
        while let Some(byte) = lower.as_bytes().get(cursor) {
            if byte.is_ascii_whitespace() || matches!(*byte, b'\'' | b'"' | b';' | b'>' | b'?') {
                break;
            }
            cursor += 1;
        }
        if cursor > value_start {
            return Some((text[value_start..cursor].to_string(), value_start, cursor));
        }
        offset = cursor.saturating_add(1);
    }
    None
}

fn floor_char_boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn detect_bom(bytes: &[u8]) -> Option<(BomKind, TextEncoding, usize)> {
    if bytes.starts_with(&[0x00, 0x00, 0xfe, 0xff]) {
        Some((BomKind::Utf32Be, TextEncoding::Utf32Be, 4))
    } else if bytes.starts_with(&[0xff, 0xfe, 0x00, 0x00]) {
        Some((BomKind::Utf32Le, TextEncoding::Utf32Le, 4))
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        Some((BomKind::Utf8, TextEncoding::Utf8, 3))
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some((BomKind::Utf16Be, TextEncoding::Utf16Be, 2))
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        Some((BomKind::Utf16Le, TextEncoding::Utf16Le, 2))
    } else {
        None
    }
}

fn xml_signature(bytes: &[u8]) -> Option<(TextEncoding, usize)> {
    if bytes.starts_with(&[0x00, 0x00, 0x00, 0x3c]) {
        Some((TextEncoding::Utf32Be, 4))
    } else if bytes.starts_with(&[0x3c, 0x00, 0x00, 0x00]) {
        Some((TextEncoding::Utf32Le, 4))
    } else if bytes.starts_with(&[0x00, 0x3c, 0x00, 0x3f]) {
        Some((TextEncoding::Utf16Be, 4))
    } else if bytes.starts_with(&[0x3c, 0x00, 0x3f, 0x00]) {
        Some((TextEncoding::Utf16Le, 4))
    } else {
        None
    }
}

#[allow(dead_code)]
fn _mapping_is_intentionally_private(_: &MappedText) {}
