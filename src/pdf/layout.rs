//! Native PDF text and geometry extraction from decoded page content streams.

use super::syntax::RawObject;
use super::*;
use crate::core::{
    BoundingBox, CoordinateOrigin, CoordinateUnit, Diagnostic, IndexBase, IndexPosition,
    IndexRange, LocationComponent, LocatorConfidence, SourceLocator,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const PARSER: &str = "grist.pdf";

pub(crate) fn extract_native_layout(
    pages: &[PdfPage],
    objects: &BTreeMap<PdfReference, &RawObject>,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> PdfNativeLayout {
    let mut public_fonts = BTreeMap::<String, PdfFont>::new();
    let mut layouts = Vec::with_capacity(pages.len());
    let mut remaining_operations = options.max_content_operations;
    let mut remaining_glyphs = options.max_native_glyphs;
    for page in pages {
        let mut fonts = page_fonts(page, objects, diagnostics);
        for font in fonts.values() {
            public_fonts
                .entry(font.model.id.clone())
                .or_insert_with(|| font.model.clone());
        }
        let (streams, content_objects) = page_content_streams(page, objects, diagnostics);
        let mut interpreter = Interpreter::new(page, &mut fonts, diagnostics);
        for (reference, bytes) in streams {
            if remaining_operations == 0 || remaining_glyphs == 0 {
                interpreter.budget_hit = true;
                break;
            }
            interpreter.interpret(
                reference,
                bytes,
                &mut remaining_operations,
                &mut remaining_glyphs,
            );
        }
        let budget_hit = interpreter.budget_hit;
        let invalid = interpreter.invalid;
        let glyphs = std::mem::take(&mut interpreter.glyphs);
        drop(interpreter);
        for font in fonts.values() {
            public_fonts
                .entry(font.model.id.clone())
                .or_insert_with(|| font.model.clone());
        }
        if budget_hit {
            diagnostics.push(
                Diagnostic::budget_exhausted(
                    PARSER,
                    format!(
                        "native text extraction budget reached on page {}",
                        page.index
                    ),
                )
                .with_locator(page.locator.clone()),
            );
        }
        let (tokens, lines, blocks, reading_order) = organize_page(page, &glyphs);
        let status = if budget_hit {
            PdfNativeTextStatus::BudgetExceeded
        } else if glyphs.is_empty() && invalid {
            PdfNativeTextStatus::Unusable
        } else if glyphs.is_empty() {
            PdfNativeTextStatus::NoNativeText
        } else {
            PdfNativeTextStatus::Extracted
        };
        layouts.push(PdfPageNativeLayout {
            page_index: page.index,
            rotation_degrees: page.rotation_degrees,
            status,
            content_objects,
            glyphs,
            tokens,
            lines,
            blocks,
            reading_order,
        });
    }
    PdfNativeLayout {
        fonts: public_fonts.into_values().collect(),
        pages: layouts,
    }
}

#[derive(Debug, Clone)]
struct FontRuntime {
    model: PdfFont,
    unicode: BTreeMap<Vec<u8>, String>,
    code_lengths: Vec<usize>,
    widths: BTreeMap<u32, f64>,
    default_width: f64,
    simple: bool,
}

fn page_fonts(
    page: &PdfPage,
    objects: &BTreeMap<PdfReference, &RawObject>,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<String, FontRuntime> {
    let mut result = BTreeMap::new();
    let Some(resources) = inherited_dictionary(page.object, "Resources", objects) else {
        return result;
    };
    let Some(fonts) = resources
        .get("Font")
        .and_then(|value| resolve_value(value, objects))
        .and_then(PdfValue::as_dictionary)
    else {
        return result;
    };
    for (resource_name, value) in fonts {
        let reference = value.as_reference();
        let font_object = reference.and_then(|reference| objects.get(&reference).copied());
        let dictionary = resolve_value(value, objects).and_then(PdfValue::as_dictionary);
        let Some(dictionary) = dictionary else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.font.invalid",
                    format!("font resource /{resource_name} does not resolve to a dictionary"),
                )
                .partial()
                .with_locator(page.locator.clone()),
            );
            result.insert(
                resource_name.clone(),
                missing_font(page, resource_name, reference),
            );
            continue;
        };
        let runtime = build_font(
            page,
            resource_name,
            reference,
            font_object,
            dictionary,
            objects,
            diagnostics,
        );
        result.insert(resource_name.clone(), runtime);
    }
    result
}

fn build_font(
    page: &PdfPage,
    resource_name: &str,
    reference: Option<PdfReference>,
    font_object: Option<&RawObject>,
    dictionary: &BTreeMap<String, PdfValue>,
    objects: &BTreeMap<PdfReference, &RawObject>,
    diagnostics: &mut Vec<Diagnostic>,
) -> FontRuntime {
    let subtype = dictionary
        .get("Subtype")
        .and_then(PdfValue::as_name)
        .map(str::to_string);
    let simple = subtype.as_deref() != Some("Type0");
    let encoding = encoding_name(dictionary.get("Encoding"), objects);
    let descendant = dictionary
        .get("DescendantFonts")
        .and_then(|value| resolve_value(value, objects))
        .and_then(|value| match value {
            PdfValue::Array(values) => values.first(),
            _ => None,
        })
        .and_then(|value| resolve_value(value, objects))
        .and_then(PdfValue::as_dictionary);
    let metrics_dictionary = descendant.unwrap_or(dictionary);
    let descriptor = metrics_dictionary
        .get("FontDescriptor")
        .and_then(|value| resolve_value(value, objects))
        .and_then(PdfValue::as_dictionary);
    let flags = descriptor
        .and_then(|value| value.get("Flags"))
        .and_then(PdfValue::as_integer)
        .unwrap_or(0);
    let base_font = dictionary
        .get("BaseFont")
        .or_else(|| metrics_dictionary.get("BaseFont"))
        .and_then(PdfValue::as_name)
        .map(str::to_string);
    let italic_angle = descriptor
        .and_then(|value| value.get("ItalicAngle"))
        .and_then(number);
    let weight = descriptor
        .and_then(|value| value.get("FontWeight"))
        .and_then(number)
        .map(|value| value.clamp(1.0, u16::MAX as f64) as u16);
    let bold_name = base_font
        .as_deref()
        .is_some_and(|name| name.to_ascii_lowercase().contains("bold"));
    let italic_name = base_font.as_deref().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        name.contains("italic") || name.contains("oblique")
    });
    let embedded = descriptor.is_some_and(|value| {
        ["FontFile", "FontFile2", "FontFile3"]
            .iter()
            .any(|key| value.contains_key(*key))
    });
    let (mut unicode, mut code_lengths, to_unicode) =
        to_unicode_map(dictionary, objects, diagnostics);
    if simple && unicode.is_empty() {
        unicode.extend(encoding_differences(dictionary.get("Encoding"), objects));
        if !unicode.is_empty() {
            code_lengths = vec![1];
        }
    }
    if matches!(to_unicode, PdfUnicodeMapStatus::Missing) {
        let diagnostic = Diagnostic::warning(
            PARSER,
            "pdf.font.to_unicode_missing",
            format!("font /{resource_name} has no usable ToUnicode map"),
        )
        .with_locator(page.locator.clone());
        diagnostics.push(if simple {
            diagnostic
        } else {
            diagnostic.partial()
        });
    }
    let (widths, default_width) = font_widths(metrics_dictionary, simple, objects);
    let object = reference.unwrap_or(page.object);
    let locator = font_object
        .map(|object| object.model.locator.clone())
        .unwrap_or_else(|| {
            page.object_locator
                .at_key(format!("Resources/Font/{resource_name}"))
        });
    let direction = if encoding.as_deref().is_some_and(|name| name.ends_with("-V")) {
        PdfWritingDirection::TopToBottom
    } else {
        PdfWritingDirection::LeftToRight
    };
    FontRuntime {
        model: PdfFont {
            id: format!(
                "pdf-font-{}-{}-{resource_name}",
                object.object_number, object.generation
            ),
            resource_name: resource_name.to_string(),
            object: reference,
            subtype,
            base_font,
            encoding,
            embedded,
            to_unicode,
            direction,
            style: PdfFontStyle {
                weight,
                italic_angle,
                bold: bold_name || flags & 0x40000 != 0 || weight.is_some_and(|w| w >= 700),
                italic: italic_name || flags & 0x40 != 0 || italic_angle.is_some_and(|a| a != 0.0),
                serif: flags & 0x2 != 0,
                monospaced: flags & 0x1 != 0,
            },
            locator,
        },
        unicode,
        code_lengths,
        widths,
        default_width,
        simple,
    }
}

fn missing_font(page: &PdfPage, name: &str, reference: Option<PdfReference>) -> FontRuntime {
    let object = reference.unwrap_or(page.object);
    FontRuntime {
        model: PdfFont {
            id: format!(
                "pdf-font-missing-{}-{}-{name}",
                object.object_number, object.generation
            ),
            resource_name: name.to_string(),
            object: reference,
            subtype: None,
            base_font: None,
            encoding: None,
            embedded: false,
            to_unicode: PdfUnicodeMapStatus::Missing,
            direction: PdfWritingDirection::Unknown,
            style: PdfFontStyle {
                weight: None,
                italic_angle: None,
                bold: false,
                italic: false,
                serif: false,
                monospaced: false,
            },
            locator: page.object_locator.at_key(format!("Resources/Font/{name}")),
        },
        unicode: BTreeMap::new(),
        code_lengths: vec![1],
        widths: BTreeMap::new(),
        default_width: 500.0,
        simple: true,
    }
}

fn inherited_dictionary<'a>(
    start: PdfReference,
    key: &str,
    objects: &'a BTreeMap<PdfReference, &RawObject>,
) -> Option<&'a BTreeMap<String, PdfValue>> {
    let mut current = Some(start);
    let mut seen = HashSet::new();
    while let Some(reference) = current {
        if !seen.insert(reference) {
            return None;
        }
        let dictionary = objects.get(&reference)?.model.value.as_dictionary()?;
        if let Some(value) = dictionary.get(key) {
            return resolve_value(value, objects)?.as_dictionary();
        }
        current = dictionary.get("Parent").and_then(PdfValue::as_reference);
    }
    None
}

fn resolve_value<'a>(
    value: &'a PdfValue,
    objects: &'a BTreeMap<PdfReference, &RawObject>,
) -> Option<&'a PdfValue> {
    let mut value = value;
    let mut depth = 0;
    while let PdfValue::Reference(reference) = value {
        value = &objects.get(reference)?.model.value;
        depth += 1;
        if depth > 32 {
            return None;
        }
    }
    Some(value)
}

fn number(value: &PdfValue) -> Option<f64> {
    match value {
        PdfValue::Integer(value) => Some(*value as f64),
        PdfValue::Real(value) => Some(*value),
        _ => None,
    }
}

fn encoding_name(
    value: Option<&PdfValue>,
    objects: &BTreeMap<PdfReference, &RawObject>,
) -> Option<String> {
    match value.and_then(|value| resolve_value(value, objects)) {
        Some(PdfValue::Name(name)) => Some(name.clone()),
        Some(PdfValue::Dictionary(dictionary)) => dictionary
            .get("BaseEncoding")
            .and_then(PdfValue::as_name)
            .map(str::to_string),
        _ => None,
    }
}

fn encoding_differences(
    value: Option<&PdfValue>,
    objects: &BTreeMap<PdfReference, &RawObject>,
) -> BTreeMap<Vec<u8>, String> {
    let Some(dictionary) = value
        .and_then(|value| resolve_value(value, objects))
        .and_then(PdfValue::as_dictionary)
    else {
        return BTreeMap::new();
    };
    let Some(PdfValue::Array(values)) = dictionary.get("Differences") else {
        return BTreeMap::new();
    };
    let mut code = None;
    let mut result = BTreeMap::new();
    for value in values {
        match value {
            PdfValue::Integer(value) => code = u8::try_from(*value).ok(),
            PdfValue::Name(name) => {
                if let Some(current) = code {
                    if let Some(character) = glyph_name(name) {
                        result.insert(vec![current], character);
                    }
                    code = current.checked_add(1);
                }
            }
            _ => {}
        }
    }
    result
}

fn glyph_name(name: &str) -> Option<String> {
    if name.len() == 1 {
        return Some(name.to_string());
    }
    let named = match name {
        "space" => ' ',
        "hyphen" => '-',
        "period" => '.',
        "comma" => ',',
        "colon" => ':',
        "semicolon" => ';',
        "parenleft" => '(',
        "parenright" => ')',
        "slash" => '/',
        "backslash" => '\\',
        "quotedbl" => '"',
        "quotesingle" => '\'',
        "ampersand" => '&',
        "percent" => '%',
        "plus" => '+',
        "equal" => '=',
        "question" => '?',
        "exclam" => '!',
        "Euro" => '\u{20ac}',
        _ => {
            let hexadecimal = name
                .strip_prefix("uni")
                .filter(|value| value.len() == 4)
                .or_else(|| {
                    name.strip_prefix('u')
                        .filter(|value| (4..=6).contains(&value.len()))
                })?;
            return u32::from_str_radix(hexadecimal, 16)
                .ok()
                .and_then(char::from_u32)
                .map(|value| value.to_string());
        }
    };
    Some(named.to_string())
}

fn font_widths(
    dictionary: &BTreeMap<String, PdfValue>,
    simple: bool,
    objects: &BTreeMap<PdfReference, &RawObject>,
) -> (BTreeMap<u32, f64>, f64) {
    let mut widths = BTreeMap::new();
    if simple {
        let first = dictionary
            .get("FirstChar")
            .and_then(PdfValue::as_integer)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        if let Some(PdfValue::Array(values)) = dictionary
            .get("Widths")
            .and_then(|value| resolve_value(value, objects))
        {
            for (offset, value) in values.iter().enumerate() {
                if let Some(width) = number(value) {
                    widths.insert(first.saturating_add(offset as u32), width);
                }
            }
        }
    } else if let Some(PdfValue::Array(values)) = dictionary
        .get("W")
        .and_then(|value| resolve_value(value, objects))
    {
        let mut cursor = 0;
        while cursor < values.len() {
            let Some(first) = values[cursor]
                .as_integer()
                .and_then(|value| u32::try_from(value).ok())
            else {
                break;
            };
            cursor += 1;
            match values.get(cursor) {
                Some(PdfValue::Array(entries)) => {
                    for (offset, value) in entries.iter().enumerate() {
                        if let Some(width) = number(value) {
                            widths.insert(first.saturating_add(offset as u32), width);
                        }
                    }
                    cursor += 1;
                }
                Some(last) => {
                    let Some(last) = last
                        .as_integer()
                        .and_then(|value| u32::try_from(value).ok())
                    else {
                        break;
                    };
                    let Some(width) = values.get(cursor + 1).and_then(number) else {
                        break;
                    };
                    for code in first..=last.min(first.saturating_add(65_536)) {
                        widths.insert(code, width);
                    }
                    cursor += 2;
                }
                None => break,
            }
        }
    }
    let default = dictionary
        .get(if simple { "MissingWidth" } else { "DW" })
        .and_then(number)
        .unwrap_or(if simple { 500.0 } else { 1000.0 });
    (widths, default)
}

fn to_unicode_map(
    dictionary: &BTreeMap<String, PdfValue>,
    objects: &BTreeMap<PdfReference, &RawObject>,
    diagnostics: &mut Vec<Diagnostic>,
) -> (BTreeMap<Vec<u8>, String>, Vec<usize>, PdfUnicodeMapStatus) {
    let Some(reference) = dictionary.get("ToUnicode").and_then(PdfValue::as_reference) else {
        let subtype = dictionary.get("Subtype").and_then(PdfValue::as_name);
        let encoding = dictionary.get("Encoding").and_then(PdfValue::as_name);
        let base_font = dictionary.get("BaseFont").and_then(PdfValue::as_name);
        let standard_base_font = base_font.is_some_and(|name| {
            let name = name.split_once('+').map_or(name, |(_, suffix)| suffix);
            ["Helvetica", "Times", "Courier"]
                .iter()
                .any(|family| name.starts_with(family))
        });
        let standard_encoding = encoding.is_some_and(|name| {
            matches!(
                name,
                "WinAnsiEncoding" | "MacRomanEncoding" | "MacExpertEncoding" | "StandardEncoding"
            )
        });
        let status = if subtype.is_some_and(|name| name != "Type0")
            && (standard_encoding || standard_base_font)
        {
            PdfUnicodeMapStatus::StandardEncoding
        } else {
            PdfUnicodeMapStatus::Missing
        };
        return (BTreeMap::new(), vec![1], status);
    };
    let Some(bytes) = objects
        .get(&reference)
        .and_then(|object| object.decoded_stream.as_deref())
    else {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.font.to_unicode_invalid",
                format!("ToUnicode object {reference} is missing or undecodable"),
            )
            .partial(),
        );
        return (BTreeMap::new(), vec![1], PdfUnicodeMapStatus::Malformed);
    };
    let map = parse_cmap(bytes);
    if map.is_empty() {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.font.to_unicode_invalid",
                format!("ToUnicode object {reference} contains no usable mappings"),
            )
            .partial(),
        );
        return (map, vec![1], PdfUnicodeMapStatus::Malformed);
    }
    let mut lengths = map
        .keys()
        .map(Vec::len)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    lengths.sort_unstable_by(|left, right| right.cmp(left));
    (map, lengths, PdfUnicodeMapStatus::Present)
}

fn parse_cmap(bytes: &[u8]) -> BTreeMap<Vec<u8>, String> {
    let tokens = CmapLexer::new(bytes).collect::<Vec<_>>();
    let mut map = BTreeMap::new();
    let mut cursor = 0;
    while cursor < tokens.len() {
        match tokens.get(cursor) {
            Some(CmapToken::Word(word)) if word == "beginbfchar" => {
                cursor += 1;
                while !matches!(tokens.get(cursor), Some(CmapToken::Word(word)) if word == "endbfchar")
                {
                    let (Some(CmapToken::Hex(source)), Some(CmapToken::Hex(target))) =
                        (tokens.get(cursor), tokens.get(cursor + 1))
                    else {
                        cursor += 1;
                        if cursor >= tokens.len() {
                            break;
                        }
                        continue;
                    };
                    if let Some(text) = utf16be(target) {
                        map.insert(source.clone(), text);
                    }
                    cursor += 2;
                }
            }
            Some(CmapToken::Word(word)) if word == "beginbfrange" => {
                cursor += 1;
                while !matches!(tokens.get(cursor), Some(CmapToken::Word(word)) if word == "endbfrange")
                {
                    let (Some(CmapToken::Hex(start)), Some(CmapToken::Hex(end))) =
                        (tokens.get(cursor), tokens.get(cursor + 1))
                    else {
                        cursor += 1;
                        if cursor >= tokens.len() {
                            break;
                        }
                        continue;
                    };
                    match tokens.get(cursor + 2) {
                        Some(CmapToken::Hex(target)) => {
                            add_cmap_range(&mut map, start, end, target);
                            cursor += 3;
                        }
                        Some(CmapToken::Array(targets)) => {
                            let start_code = bytes_to_u32(start);
                            for (offset, target) in targets.iter().enumerate() {
                                if let (Some(code), Some(text)) = (
                                    start_code.map(|v| v.saturating_add(offset as u32)),
                                    utf16be(target),
                                ) {
                                    map.insert(u32_to_bytes(code, start.len()), text);
                                }
                            }
                            cursor += 3;
                        }
                        _ => cursor += 1,
                    }
                }
            }
            _ => cursor += 1,
        }
    }
    map
}

fn add_cmap_range(map: &mut BTreeMap<Vec<u8>, String>, start: &[u8], end: &[u8], target: &[u8]) {
    let (Some(first), Some(last), Some(target)) =
        (bytes_to_u32(start), bytes_to_u32(end), bytes_to_u32(target))
    else {
        return;
    };
    for code in first..=last.min(first.saturating_add(65_536)) {
        let mapped = target.saturating_add(code - first);
        let target_bytes = u32_to_bytes(mapped, target_byte_len(mapped));
        if let Some(text) = utf16be(&target_bytes) {
            map.insert(u32_to_bytes(code, start.len()), text);
        }
    }
}

fn bytes_to_u32(bytes: &[u8]) -> Option<u32> {
    (bytes.len() <= 4).then(|| {
        bytes
            .iter()
            .fold(0u32, |value, byte| (value << 8) | u32::from(*byte))
    })
}

fn u32_to_bytes(value: u32, length: usize) -> Vec<u8> {
    value.to_be_bytes()[4 - length.min(4)..].to_vec()
}

fn target_byte_len(value: u32) -> usize {
    if value <= u16::MAX as u32 { 2 } else { 4 }
}

fn utf16be(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return None;
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&units).ok()
}

#[derive(Debug, Clone)]
enum CmapToken {
    Hex(Vec<u8>),
    Array(Vec<Vec<u8>>),
    Word(String),
}

struct CmapLexer<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> CmapLexer<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn skip_space(&mut self) {
        loop {
            while self
                .bytes
                .get(self.cursor)
                .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                self.cursor += 1;
            }
            if self.bytes.get(self.cursor) == Some(&b'%') {
                while self
                    .bytes
                    .get(self.cursor)
                    .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
                {
                    self.cursor += 1;
                }
            } else {
                break;
            }
        }
    }

    fn hex(&mut self) -> Vec<u8> {
        self.cursor += 1;
        let mut nibbles = Vec::new();
        while let Some(byte) = self.bytes.get(self.cursor).copied() {
            self.cursor += 1;
            if byte == b'>' {
                break;
            }
            if let Some(value) = hex_value(byte) {
                nibbles.push(value);
            }
        }
        if nibbles.len() % 2 == 1 {
            nibbles.push(0);
        }
        nibbles
            .chunks_exact(2)
            .map(|pair| pair[0] << 4 | pair[1])
            .collect()
    }
}

impl Iterator for CmapLexer<'_> {
    type Item = CmapToken;
    fn next(&mut self) -> Option<Self::Item> {
        self.skip_space();
        match self.bytes.get(self.cursor).copied()? {
            b'<' if self.bytes.get(self.cursor + 1) != Some(&b'<') => {
                Some(CmapToken::Hex(self.hex()))
            }
            b'[' => {
                self.cursor += 1;
                let mut values = Vec::new();
                loop {
                    self.skip_space();
                    match self.bytes.get(self.cursor) {
                        Some(b']') => {
                            self.cursor += 1;
                            break;
                        }
                        Some(b'<') => values.push(self.hex()),
                        Some(_) => {
                            self.cursor += 1;
                        }
                        None => break,
                    }
                }
                Some(CmapToken::Array(values))
            }
            _ => {
                let start = self.cursor;
                while self.bytes.get(self.cursor).is_some_and(|byte| {
                    !byte.is_ascii_whitespace() && !matches!(byte, b'[' | b']' | b'<' | b'>')
                }) {
                    self.cursor += 1;
                }
                (self.cursor > start).then(|| {
                    CmapToken::Word(
                        String::from_utf8_lossy(&self.bytes[start..self.cursor]).into_owned(),
                    )
                })
            }
        }
    }
}

fn page_content_streams<'a>(
    page: &PdfPage,
    objects: &'a BTreeMap<PdfReference, &RawObject>,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<(PdfReference, &'a [u8])>, Vec<PdfReference>) {
    let Some(dictionary) = objects
        .get(&page.object)
        .and_then(|object| object.model.value.as_dictionary())
    else {
        return (Vec::new(), Vec::new());
    };
    let Some(contents) = dictionary.get("Contents") else {
        return (Vec::new(), Vec::new());
    };
    let mut references = Vec::new();
    collect_content_references(contents, objects, 0, &mut references);
    let mut streams = Vec::new();
    for reference in &references {
        match objects.get(reference) {
            Some(object) if object.model.stream.is_none() => diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.content.invalid_object",
                    format!(
                        "page {} content object {reference} is not a stream",
                        page.index
                    ),
                )
                .partial()
                .with_locator(page.locator.clone()),
            ),
            Some(object) if object.decoded_stream.is_none() => diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.content.unavailable",
                    format!(
                        "page {} content stream {reference} could not be decoded",
                        page.index
                    ),
                )
                .partial()
                .with_locator(page.locator.clone()),
            ),
            Some(object) => streams.push((*reference, object.decoded_stream.as_deref().unwrap())),
            None => diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.content.missing_object",
                    format!(
                        "page {} references missing content object {reference}",
                        page.index
                    ),
                )
                .partial()
                .with_locator(page.locator.clone()),
            ),
        }
    }
    (streams, references)
}

fn collect_content_references(
    value: &PdfValue,
    objects: &BTreeMap<PdfReference, &RawObject>,
    depth: u8,
    output: &mut Vec<PdfReference>,
) {
    if depth > 16 {
        return;
    }
    match value {
        PdfValue::Reference(reference) => {
            if objects
                .get(reference)
                .is_some_and(|object| object.model.stream.is_some())
            {
                output.push(*reference);
            } else if let Some(object) = objects.get(reference)
                && matches!(object.model.value, PdfValue::Array(_))
            {
                collect_content_references(&object.model.value, objects, depth + 1, output);
            } else {
                output.push(*reference);
            }
        }
        PdfValue::Array(values) => {
            for value in values {
                collect_content_references(value, objects, depth + 1, output);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Copy)]
struct Matrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Matrix {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };
    fn new(values: [f64; 6]) -> Self {
        Self {
            a: values[0],
            b: values[1],
            c: values[2],
            d: values[3],
            e: values[4],
            f: values[5],
        }
    }
    fn then(self, right: Self) -> Self {
        Self {
            a: self.a * right.a + self.c * right.b,
            b: self.b * right.a + self.d * right.b,
            c: self.a * right.c + self.c * right.d,
            d: self.b * right.c + self.d * right.d,
            e: self.a * right.e + self.c * right.f + self.e,
            f: self.b * right.e + self.d * right.f + self.f,
        }
    }
    fn translate(self, x: f64, y: f64) -> Self {
        self.then(Self::new([1.0, 0.0, 0.0, 1.0, x, y]))
    }
    fn point(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

#[derive(Debug, Clone)]
struct TextState {
    ctm: Matrix,
    text_matrix: Matrix,
    line_matrix: Matrix,
    font_name: Option<String>,
    font_size: f64,
    char_space: f64,
    word_space: f64,
    horizontal_scale: f64,
    leading: f64,
    rise: f64,
    rendering_mode: u8,
    in_text: bool,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            ctm: Matrix::IDENTITY,
            text_matrix: Matrix::IDENTITY,
            line_matrix: Matrix::IDENTITY,
            font_name: None,
            font_size: 0.0,
            char_space: 0.0,
            word_space: 0.0,
            horizontal_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            rendering_mode: 0,
            in_text: false,
        }
    }
}

struct Interpreter<'a, 'b> {
    page: &'a PdfPage,
    fonts: &'b mut BTreeMap<String, FontRuntime>,
    diagnostics: &'b mut Vec<Diagnostic>,
    state: TextState,
    stack: Vec<TextState>,
    glyphs: Vec<PdfGlyph>,
    operation_index: u64,
    missing_fonts: HashSet<String>,
    invalid: bool,
    budget_hit: bool,
}

impl<'a, 'b> Interpreter<'a, 'b> {
    fn new(
        page: &'a PdfPage,
        fonts: &'b mut BTreeMap<String, FontRuntime>,
        diagnostics: &'b mut Vec<Diagnostic>,
    ) -> Self {
        Self {
            page,
            fonts,
            diagnostics,
            state: TextState::default(),
            stack: Vec::new(),
            glyphs: Vec::new(),
            operation_index: 0,
            missing_fonts: HashSet::new(),
            invalid: false,
            budget_hit: false,
        }
    }

    fn interpret(
        &mut self,
        content_object: PdfReference,
        bytes: &[u8],
        remaining_operations: &mut u64,
        remaining_glyphs: &mut u64,
    ) {
        let mut lexer = ContentLexer::new(bytes);
        let mut operands = Vec::new();
        while let Some(item) = lexer.next_item() {
            match item {
                ContentItem::Value(value) => operands.push(value),
                ContentItem::Operator(operator) => {
                    if *remaining_operations == 0 {
                        self.budget_hit = true;
                        break;
                    }
                    *remaining_operations -= 1;
                    self.operation_index += 1;
                    if operator == "BI" {
                        lexer.skip_inline_image();
                        operands.clear();
                        continue;
                    }
                    self.apply_operator(&operator, &operands, content_object, remaining_glyphs);
                    operands.clear();
                    if self.budget_hit {
                        break;
                    }
                }
            }
        }
    }

    fn apply_operator(
        &mut self,
        operator: &str,
        operands: &[ContentValue],
        content_object: PdfReference,
        remaining_glyphs: &mut u64,
    ) {
        match operator {
            "q" => self.stack.push(self.state.clone()),
            "Q" => {
                if let Some(state) = self.stack.pop() {
                    self.state = state;
                } else {
                    self.invalid_operator(operator, "graphics-state stack is empty");
                }
            }
            "cm" => {
                if let Some(values) = six_numbers(operands) {
                    self.state.ctm = self.state.ctm.then(Matrix::new(values));
                } else {
                    self.invalid_operator(operator, "expected six numeric operands");
                }
            }
            "BT" => {
                self.state.in_text = true;
                self.state.text_matrix = Matrix::IDENTITY;
                self.state.line_matrix = Matrix::IDENTITY;
            }
            "ET" => self.state.in_text = false,
            "Tf" => match operands {
                [ContentValue::Name(name), ContentValue::Number(size)] if size.is_finite() => {
                    self.state.font_name = Some(name.clone());
                    self.state.font_size = size.abs();
                }
                _ => self.invalid_operator(operator, "expected a font name and size"),
            },
            "Tm" => {
                if let Some(values) = six_numbers(operands) {
                    self.state.text_matrix = Matrix::new(values);
                    self.state.line_matrix = self.state.text_matrix;
                } else {
                    self.invalid_operator(operator, "expected six numeric operands");
                }
            }
            "Td" | "TD" => {
                if let Some([x, y]) = two_numbers(operands) {
                    if operator == "TD" {
                        self.state.leading = -y;
                    }
                    self.state.line_matrix = self.state.line_matrix.translate(x, y);
                    self.state.text_matrix = self.state.line_matrix;
                } else {
                    self.invalid_operator(operator, "expected two numeric operands");
                }
            }
            "T*" => self.next_line(),
            "Tc" => self.set_number(operands, operator, |state, value| state.char_space = value),
            "Tw" => self.set_number(operands, operator, |state, value| state.word_space = value),
            "Tz" => self.set_number(operands, operator, |state, value| {
                state.horizontal_scale = value / 100.0
            }),
            "TL" => self.set_number(operands, operator, |state, value| state.leading = value),
            "Ts" => self.set_number(operands, operator, |state, value| state.rise = value),
            "Tr" => {
                if let Some(value) = one_number(operands) {
                    self.state.rendering_mode = value.clamp(0.0, 7.0) as u8;
                } else {
                    self.invalid_operator(operator, "expected a numeric rendering mode");
                }
            }
            "Tj" => {
                if let Some(ContentValue::String(bytes)) = operands.last() {
                    self.show_bytes(bytes, content_object, remaining_glyphs);
                } else {
                    self.invalid_operator(operator, "expected a string operand");
                }
            }
            "TJ" => {
                if let Some(ContentValue::Array(values)) = operands.last() {
                    for value in values {
                        match value {
                            ContentValue::String(bytes) => {
                                self.show_bytes(bytes, content_object, remaining_glyphs)
                            }
                            ContentValue::Number(adjustment) => self.adjust_text(*adjustment),
                            _ => self
                                .invalid_operator(operator, "text array contains an invalid value"),
                        }
                        if self.budget_hit {
                            break;
                        }
                    }
                } else {
                    self.invalid_operator(operator, "expected a text array");
                }
            }
            "'" => {
                self.next_line();
                if let Some(ContentValue::String(bytes)) = operands.last() {
                    self.show_bytes(bytes, content_object, remaining_glyphs);
                } else {
                    self.invalid_operator(operator, "expected a string operand");
                }
            }
            "\"" => match operands {
                [
                    ContentValue::Number(word),
                    ContentValue::Number(character),
                    ContentValue::String(bytes),
                ] => {
                    self.state.word_space = *word;
                    self.state.char_space = *character;
                    self.next_line();
                    self.show_bytes(bytes, content_object, remaining_glyphs);
                }
                _ => self.invalid_operator(
                    operator,
                    "expected word spacing, character spacing, and a string",
                ),
            },
            _ => {}
        }
    }

    fn set_number(
        &mut self,
        operands: &[ContentValue],
        operator: &str,
        setter: fn(&mut TextState, f64),
    ) {
        if let Some(value) = one_number(operands) {
            setter(&mut self.state, value);
        } else {
            self.invalid_operator(operator, "expected one numeric operand");
        }
    }

    fn next_line(&mut self) {
        self.state.line_matrix = self.state.line_matrix.translate(0.0, -self.state.leading);
        self.state.text_matrix = self.state.line_matrix;
    }

    fn adjust_text(&mut self, adjustment: f64) {
        let displacement = -adjustment / 1000.0 * self.state.font_size;
        let vertical = self
            .current_font()
            .is_some_and(|font| font.model.direction == PdfWritingDirection::TopToBottom);
        self.state.text_matrix = if vertical {
            self.state.text_matrix.translate(0.0, -displacement)
        } else {
            self.state
                .text_matrix
                .translate(displacement * self.state.horizontal_scale, 0.0)
        };
    }

    fn show_bytes(
        &mut self,
        bytes: &[u8],
        content_object: PdfReference,
        remaining_glyphs: &mut u64,
    ) {
        if !self.state.in_text {
            self.invalid_operator("Tj/TJ", "text-showing operator occurred outside BT/ET");
            return;
        }
        if self.state.font_size <= 0.0 {
            self.invalid_operator("Tj/TJ", "text-showing operator has no positive font size");
            return;
        }
        let Some(font_name) = self.state.font_name.clone() else {
            self.missing_font("<unset>");
            return;
        };
        if !self.fonts.contains_key(&font_name) {
            self.missing_font(&font_name);
            self.fonts
                .insert(font_name.clone(), missing_font(self.page, &font_name, None));
        }
        let font = self.fonts.get(&font_name).expect("font inserted").clone();
        let mut cursor = 0;
        while cursor < bytes.len() {
            if *remaining_glyphs == 0 {
                self.budget_hit = true;
                break;
            }
            let (length, text, mapping_confidence) = decode_code(&font, &bytes[cursor..]);
            let length = length.max(1).min(bytes.len() - cursor);
            let code = &bytes[cursor..cursor + length];
            let numeric_code = bytes_to_u32(code).unwrap_or(0);
            let width_known = font.widths.contains_key(&numeric_code);
            let width = font
                .widths
                .get(&numeric_code)
                .copied()
                .unwrap_or(font.default_width);
            let vertical = font.model.direction == PdfWritingDirection::TopToBottom;
            let advance = width / 1000.0 * self.state.font_size
                + self.state.char_space
                + if code == b" " {
                    self.state.word_space
                } else {
                    0.0
                };
            let scaled_advance = advance * self.state.horizontal_scale;
            let geometry_confidence = if width_known { 1.0 } else { 0.72 };
            let bbox = glyph_bbox(
                self.state.ctm.then(self.state.text_matrix),
                self.state.font_size,
                scaled_advance.abs(),
                self.state.rise,
                vertical,
            );
            let direction = text_direction(&text, font.model.direction);
            let locator = pdf_locator(
                self.page,
                bbox,
                None,
                mapping_confidence.min(geometry_confidence),
            );
            self.glyphs.push(PdfGlyph {
                index: self.glyphs.len() as u64 + 1,
                text,
                raw_code_hex: code.iter().map(|byte| format!("{byte:02X}")).collect(),
                font_id: font.model.id.clone(),
                font_size: self.state.font_size,
                direction,
                rendering_mode: self.state.rendering_mode,
                mapping_confidence,
                geometry_confidence,
                bbox,
                content_object,
                operation_index: self.operation_index,
                locator,
            });
            *remaining_glyphs -= 1;
            self.state.text_matrix = if vertical {
                self.state.text_matrix.translate(0.0, -advance)
            } else {
                self.state.text_matrix.translate(scaled_advance, 0.0)
            };
            cursor += length;
        }
    }

    fn current_font(&self) -> Option<&FontRuntime> {
        self.state
            .font_name
            .as_ref()
            .and_then(|name| self.fonts.get(name))
    }

    fn missing_font(&mut self, name: &str) {
        if self.missing_fonts.insert(name.to_string()) {
            self.diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.font.missing",
                    format!("page {} uses unresolved font /{name}", self.page.index),
                )
                .partial()
                .with_locator(self.page.locator.clone()),
            );
        }
        self.invalid = true;
    }

    fn invalid_operator(&mut self, operator: &str, reason: &str) {
        self.invalid = true;
        self.diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.content.invalid_operator",
                format!("page {} operator {operator}: {reason}", self.page.index),
            )
            .partial()
            .with_locator(self.page.locator.clone()),
        );
    }
}

fn one_number(values: &[ContentValue]) -> Option<f64> {
    match values {
        [ContentValue::Number(value)] if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn two_numbers(values: &[ContentValue]) -> Option<[f64; 2]> {
    match values {
        [ContentValue::Number(a), ContentValue::Number(b)] if a.is_finite() && b.is_finite() => {
            Some([*a, *b])
        }
        _ => None,
    }
}

fn six_numbers(values: &[ContentValue]) -> Option<[f64; 6]> {
    match values {
        [
            ContentValue::Number(a),
            ContentValue::Number(b),
            ContentValue::Number(c),
            ContentValue::Number(d),
            ContentValue::Number(e),
            ContentValue::Number(f),
        ] if [a, b, c, d, e, f].iter().all(|value| value.is_finite()) => {
            Some([*a, *b, *c, *d, *e, *f])
        }
        _ => None,
    }
}

fn decode_code(font: &FontRuntime, bytes: &[u8]) -> (usize, String, f64) {
    for length in &font.code_lengths {
        if let Some(code) = bytes.get(..*length)
            && let Some(text) = font.unicode.get(code)
        {
            return (*length, text.clone(), 1.0);
        }
    }
    if font.simple {
        let byte = bytes[0];
        let confidence = match font.model.to_unicode {
            PdfUnicodeMapStatus::StandardEncoding => {
                if byte.is_ascii() {
                    1.0
                } else {
                    0.8
                }
            }
            PdfUnicodeMapStatus::Present => 0.8,
            PdfUnicodeMapStatus::Missing | PdfUnicodeMapStatus::Malformed => 0.45,
        };
        (1, decode_single_byte(byte), confidence)
    } else {
        let length = if bytes.len() >= 2 { 2 } else { 1 };
        (length, "\u{fffd}".into(), 0.2)
    }
}

fn decode_single_byte(byte: u8) -> String {
    let character = match byte {
        0x80 => '\u{20ac}',
        0x82 => '\u{201a}',
        0x83 => '\u{0192}',
        0x84 => '\u{201e}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02c6}',
        0x89 => '\u{2030}',
        0x8a => '\u{0160}',
        0x8b => '\u{2039}',
        0x8c => '\u{0152}',
        0x8e => '\u{017d}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02dc}',
        0x99 => '\u{2122}',
        0x9a => '\u{0161}',
        0x9b => '\u{203a}',
        0x9c => '\u{0153}',
        0x9e => '\u{017e}',
        0x9f => '\u{0178}',
        value => char::from(value),
    };
    character.to_string()
}

fn glyph_bbox(
    matrix: Matrix,
    font_size: f64,
    advance: f64,
    rise: f64,
    vertical: bool,
) -> BoundingBox {
    let (x_size, y_size) = if vertical {
        (font_size, advance.max(font_size * 0.5))
    } else {
        (advance.max(font_size * 0.2), font_size)
    };
    let points = [
        matrix.point(0.0, rise - font_size * 0.2),
        matrix.point(x_size, rise - font_size * 0.2),
        matrix.point(0.0, rise - font_size * 0.2 + y_size),
        matrix.point(x_size, rise - font_size * 0.2 + y_size),
    ];
    let min_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    BoundingBox {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
        unit: CoordinateUnit::Points,
        origin: CoordinateOrigin::BottomLeft,
    }
}

fn text_direction(text: &str, fallback: PdfWritingDirection) -> PdfWritingDirection {
    let mut rtl = false;
    let mut ltr = false;
    for character in text.chars() {
        let code = character as u32;
        let character_is_rtl = matches!(
            code,
            0x0590..=0x08ff | 0xfb1d..=0xfdff | 0xfe70..=0xfeff
        );
        rtl |= character_is_rtl;
        ltr |= character.is_alphabetic() && !character_is_rtl;
    }
    match (ltr, rtl) {
        (true, true) => PdfWritingDirection::Mixed,
        (_, true) => PdfWritingDirection::RightToLeft,
        (true, false) => PdfWritingDirection::LeftToRight,
        _ => fallback,
    }
}

#[derive(Debug, Clone)]
enum ContentValue {
    Number(f64),
    Name(String),
    String(Vec<u8>),
    Array(Vec<ContentValue>),
}

enum ContentItem {
    Value(ContentValue),
    Operator(String),
}

struct ContentLexer<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> ContentLexer<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn next_item(&mut self) -> Option<ContentItem> {
        self.skip_space();
        let byte = *self.bytes.get(self.cursor)?;
        match byte {
            b'/' => Some(ContentItem::Value(ContentValue::Name(self.name()))),
            b'(' => Some(ContentItem::Value(ContentValue::String(
                self.literal_string(),
            ))),
            b'<' if self.bytes.get(self.cursor + 1) != Some(&b'<') => {
                Some(ContentItem::Value(ContentValue::String(self.hex_string())))
            }
            b'[' => Some(ContentItem::Value(ContentValue::Array(self.array()))),
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let token = self.word();
                match token.parse::<f64>().ok().filter(|value| value.is_finite()) {
                    Some(value) => Some(ContentItem::Value(ContentValue::Number(value))),
                    None => Some(ContentItem::Operator(token)),
                }
            }
            _ => Some(ContentItem::Operator(self.word())),
        }
    }

    fn skip_space(&mut self) {
        loop {
            while self
                .bytes
                .get(self.cursor)
                .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == 0)
            {
                self.cursor += 1;
            }
            if self.bytes.get(self.cursor) == Some(&b'%') {
                while self
                    .bytes
                    .get(self.cursor)
                    .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
                {
                    self.cursor += 1;
                }
            } else {
                break;
            }
        }
    }

    fn name(&mut self) -> String {
        self.cursor += 1;
        let start = self.cursor;
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(|byte| !is_content_delimiter(*byte))
        {
            self.cursor += 1;
        }
        decode_name(&self.bytes[start..self.cursor])
    }

    fn word(&mut self) -> String {
        let start = self.cursor;
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(|byte| !is_content_delimiter(*byte))
        {
            self.cursor += 1;
        }
        if self.cursor == start {
            self.cursor += 1;
        }
        String::from_utf8_lossy(&self.bytes[start..self.cursor]).into_owned()
    }

    fn literal_string(&mut self) -> Vec<u8> {
        self.cursor += 1;
        let mut result = Vec::new();
        let mut depth = 1u32;
        while let Some(byte) = self.bytes.get(self.cursor).copied() {
            self.cursor += 1;
            match byte {
                b'(' => {
                    depth += 1;
                    result.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    result.push(byte);
                }
                b'\\' => {
                    let Some(escaped) = self.bytes.get(self.cursor).copied() else {
                        break;
                    };
                    self.cursor += 1;
                    match escaped {
                        b'n' => result.push(b'\n'),
                        b'r' => result.push(b'\r'),
                        b't' => result.push(b'\t'),
                        b'b' => result.push(8),
                        b'f' => result.push(12),
                        b'(' | b')' | b'\\' => result.push(escaped),
                        b'\r' => {
                            if self.bytes.get(self.cursor) == Some(&b'\n') {
                                self.cursor += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut value = escaped - b'0';
                            for _ in 0..2 {
                                if let Some(next @ b'0'..=b'7') =
                                    self.bytes.get(self.cursor).copied()
                                {
                                    self.cursor += 1;
                                    value = value.saturating_mul(8).saturating_add(next - b'0');
                                } else {
                                    break;
                                }
                            }
                            result.push(value);
                        }
                        value => result.push(value),
                    }
                }
                value => result.push(value),
            }
        }
        result
    }

    fn hex_string(&mut self) -> Vec<u8> {
        self.cursor += 1;
        let mut nibbles = Vec::new();
        while let Some(byte) = self.bytes.get(self.cursor).copied() {
            self.cursor += 1;
            if byte == b'>' {
                break;
            }
            if let Some(value) = hex_value(byte) {
                nibbles.push(value);
            }
        }
        if nibbles.len() % 2 == 1 {
            nibbles.push(0);
        }
        nibbles
            .chunks_exact(2)
            .map(|pair| pair[0] << 4 | pair[1])
            .collect()
    }

    fn array(&mut self) -> Vec<ContentValue> {
        self.cursor += 1;
        let mut values = Vec::new();
        loop {
            self.skip_space();
            match self.bytes.get(self.cursor).copied() {
                Some(b']') => {
                    self.cursor += 1;
                    break;
                }
                Some(b'(') => values.push(ContentValue::String(self.literal_string())),
                Some(b'<') if self.bytes.get(self.cursor + 1) != Some(&b'<') => {
                    values.push(ContentValue::String(self.hex_string()))
                }
                Some(b'/') => values.push(ContentValue::Name(self.name())),
                Some(b'[') => values.push(ContentValue::Array(self.array())),
                Some(_) => {
                    let token = self.word();
                    if let Ok(value) = token.parse::<f64>() {
                        values.push(ContentValue::Number(value));
                    }
                }
                None => break,
            }
        }
        values
    }

    fn skip_inline_image(&mut self) {
        while self.cursor + 2 <= self.bytes.len() {
            if self.bytes[self.cursor..].starts_with(b"EI")
                && self
                    .cursor
                    .checked_sub(1)
                    .and_then(|index| self.bytes.get(index))
                    .is_some_and(|byte| byte.is_ascii_whitespace())
                && self
                    .bytes
                    .get(self.cursor + 2)
                    .is_none_or(|byte| byte.is_ascii_whitespace())
            {
                self.cursor += 2;
                return;
            }
            self.cursor += 1;
        }
    }
}

fn is_content_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            0 | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

fn decode_name(bytes: &[u8]) -> String {
    let mut output = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'#'
            && let (Some(high), Some(low)) = (
                bytes.get(cursor + 1).and_then(|byte| hex_value(*byte)),
                bytes.get(cursor + 2).and_then(|byte| hex_value(*byte)),
            )
        {
            output.push(high << 4 | low);
            cursor += 3;
        } else {
            output.push(bytes[cursor]);
            cursor += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug)]
struct LineGroup {
    glyphs: Vec<usize>,
    bbox: BoundingBox,
    column: usize,
}

fn organize_page(
    page: &PdfPage,
    glyphs: &[PdfGlyph],
) -> (
    Vec<PdfTextToken>,
    Vec<PdfTextLine>,
    Vec<PdfTextBlock>,
    PdfReadingOrder,
) {
    let mut groups = cluster_lines(glyphs);
    let column_count = assign_columns(&mut groups, glyphs);
    let overall_direction = aggregate_direction(glyphs.iter().map(|glyph| glyph.direction));
    groups.sort_by(|left, right| {
        let column = if overall_direction == PdfWritingDirection::RightToLeft {
            right.column.cmp(&left.column)
        } else {
            left.column.cmp(&right.column)
        };
        column
            .then_with(|| top(right.bbox).total_cmp(&top(left.bbox)))
            .then_with(|| left.bbox.x.total_cmp(&right.bbox.x))
    });
    let mut tokens = Vec::new();
    let mut lines = Vec::new();
    let mut line_columns = Vec::new();
    for group in &groups {
        let mut indexes = group.glyphs.clone();
        let direction = aggregate_direction(indexes.iter().map(|index| glyphs[*index].direction));
        indexes.sort_by(|left, right| match direction {
            PdfWritingDirection::RightToLeft => {
                glyphs[*right].bbox.x.total_cmp(&glyphs[*left].bbox.x)
            }
            PdfWritingDirection::TopToBottom | PdfWritingDirection::BottomToTop => {
                top(glyphs[*right].bbox).total_cmp(&top(glyphs[*left].bbox))
            }
            _ => glyphs[*left].bbox.x.total_cmp(&glyphs[*right].bbox.x),
        });
        let token_start = tokens.len() as u64 + 1;
        build_tokens(page, glyphs, &indexes, &mut tokens);
        let token_end = tokens.len() as u64 + 1;
        if token_start == token_end {
            continue;
        }
        let text = tokens[(token_start - 1) as usize..(token_end - 1) as usize]
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let confidence = glyph_confidence(glyphs, &indexes) * 0.95;
        let index = lines.len() as u64 + 1;
        lines.push(PdfTextLine {
            index,
            text,
            token_start,
            token_end,
            direction,
            bbox: group.bbox,
            confidence,
            evidence: vec![
                PdfReadingEvidence {
                    kind: PdfReadingEvidenceKind::TextMatrixBaseline,
                    description: "glyph boxes share a compatible text baseline".into(),
                    confidence: 0.95,
                },
                PdfReadingEvidence {
                    kind: PdfReadingEvidenceKind::GeometricLineClustering,
                    description: "glyph proximity and direction form a deterministic line".into(),
                    confidence,
                },
            ],
            locator: pdf_locator(page, group.bbox, Some((token_start, token_end)), confidence),
        });
        line_columns.push(group.column);
    }
    let blocks = build_blocks(page, &lines, &line_columns);
    let confidence = if blocks.is_empty() {
        1.0
    } else if column_count > 1 {
        0.82
    } else {
        0.9
    };
    let mut evidence = vec![
        PdfReadingEvidence {
            kind: PdfReadingEvidenceKind::ContentStreamSequence,
            description: "content operation sequence breaks otherwise equal geometry ties".into(),
            confidence: 0.7,
        },
        PdfReadingEvidence {
            kind: PdfReadingEvidenceKind::GeometricLineClustering,
            description: "baseline, proximity, and page coordinates order native text".into(),
            confidence: 0.9,
        },
    ];
    if column_count > 1 {
        evidence.push(PdfReadingEvidence {
            kind: PdfReadingEvidenceKind::ColumnSeparation,
            description: format!("detected {column_count} non-overlapping geometric text columns"),
            confidence: 0.82,
        });
    }
    if matches!(
        overall_direction,
        PdfWritingDirection::RightToLeft | PdfWritingDirection::Mixed
    ) {
        evidence.push(PdfReadingEvidence {
            kind: PdfReadingEvidenceKind::UnicodeDirectionality,
            description: "Unicode strong-direction characters influence glyph and column ordering"
                .into(),
            confidence: 0.9,
        });
    }
    let reading_order = PdfReadingOrder {
        block_order: blocks.iter().map(|block| block.index).collect(),
        confidence,
        evidence,
    };
    (tokens, lines, blocks, reading_order)
}

fn cluster_lines(glyphs: &[PdfGlyph]) -> Vec<LineGroup> {
    let mut groups = Vec::<LineGroup>::new();
    for (index, glyph) in glyphs
        .iter()
        .enumerate()
        .filter(|(_, glyph)| !glyph.text.trim().is_empty())
    {
        let center_y = glyph.bbox.y + glyph.bbox.height / 2.0;
        let matching = groups.iter().position(|group| {
            let group_center = group.bbox.y + group.bbox.height / 2.0;
            let tolerance = glyph.bbox.height.max(group.bbox.height) * 0.55 + 0.5;
            let horizontal_gap = if glyph.bbox.x > right(group.bbox) {
                glyph.bbox.x - right(group.bbox)
            } else if group.bbox.x > right(glyph.bbox) {
                group.bbox.x - right(glyph.bbox)
            } else {
                0.0
            };
            (center_y - group_center).abs() <= tolerance
                && horizontal_gap <= glyph.bbox.height.max(group.bbox.height) * 5.0
        });
        if let Some(group) = matching {
            groups[group].glyphs.push(index);
            groups[group].bbox = union_box(groups[group].bbox, glyph.bbox);
        } else {
            groups.push(LineGroup {
                glyphs: vec![index],
                bbox: glyph.bbox,
                column: 0,
            });
        }
    }
    groups
}

fn assign_columns(groups: &mut [LineGroup], _glyphs: &[PdfGlyph]) -> usize {
    let mut order = (0..groups.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| groups[*left].bbox.x.total_cmp(&groups[*right].bbox.x));
    let mut columns = Vec::<BoundingBox>::new();
    for index in order {
        let bbox = groups[index].bbox;
        let candidate = columns.iter().position(|column| {
            let overlap = (right(bbox).min(right(*column)) - bbox.x.max(column.x)).max(0.0);
            let minimum = bbox.width.min(column.width).max(1.0);
            overlap / minimum >= 0.35
                || (bbox.x - column.x).abs() <= bbox.height.max(column.height) * 2.0
        });
        let column = candidate.unwrap_or_else(|| {
            columns.push(bbox);
            columns.len() - 1
        });
        columns[column] = union_box(columns[column], bbox);
        groups[index].column = column;
    }
    columns.len()
}

fn build_tokens(
    page: &PdfPage,
    glyphs: &[PdfGlyph],
    indexes: &[usize],
    output: &mut Vec<PdfTextToken>,
) {
    let mut current = Vec::<usize>::new();
    let mut previous: Option<&PdfGlyph> = None;
    for index in indexes {
        let glyph = &glyphs[*index];
        let gap = previous
            .map(|previous| match glyph.direction {
                PdfWritingDirection::RightToLeft => previous.bbox.x - right(glyph.bbox),
                PdfWritingDirection::TopToBottom | PdfWritingDirection::BottomToTop => {
                    previous.bbox.y - top(glyph.bbox)
                }
                _ => glyph.bbox.x - right(previous.bbox),
            })
            .unwrap_or(0.0);
        let boundary = glyph.text.chars().all(char::is_whitespace)
            || gap
                > glyph
                    .bbox
                    .height
                    .max(previous.map_or(0.0, |value| value.bbox.height))
                    * 0.45;
        if boundary {
            flush_token(page, glyphs, &mut current, output);
        }
        if !glyph.text.chars().all(char::is_whitespace) {
            current.push(*index);
        }
        previous = Some(glyph);
    }
    flush_token(page, glyphs, &mut current, output);
}

fn flush_token(
    page: &PdfPage,
    glyphs: &[PdfGlyph],
    current: &mut Vec<usize>,
    output: &mut Vec<PdfTextToken>,
) {
    if current.is_empty() {
        return;
    }
    let bbox = current
        .iter()
        .skip(1)
        .fold(glyphs[current[0]].bbox, |bbox, index| {
            union_box(bbox, glyphs[*index].bbox)
        });
    let text = current
        .iter()
        .map(|index| glyphs[*index].text.as_str())
        .collect::<String>();
    let mut font_ids = current
        .iter()
        .map(|index| glyphs[*index].font_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    font_ids.sort();
    let direction = aggregate_direction(current.iter().map(|index| glyphs[*index].direction));
    let confidence = glyph_confidence(glyphs, current);
    let index = output.len() as u64 + 1;
    let glyph_start = current
        .iter()
        .map(|index| glyphs[*index].index)
        .min()
        .unwrap_or(1);
    let glyph_end = current
        .iter()
        .map(|index| glyphs[*index].index)
        .max()
        .unwrap_or(glyph_start)
        + 1;
    output.push(PdfTextToken {
        index,
        text,
        glyph_start,
        glyph_end,
        font_ids,
        direction,
        bbox,
        locator: pdf_locator(page, bbox, Some((index, index + 1)), confidence),
    });
    current.clear();
}

fn build_blocks(page: &PdfPage, lines: &[PdfTextLine], columns: &[usize]) -> Vec<PdfTextBlock> {
    let mut groups = Vec::<Vec<usize>>::new();
    for index in 0..lines.len() {
        let append = groups
            .last()
            .and_then(|group| group.last())
            .is_some_and(|previous| {
                columns.get(*previous) == columns.get(index)
                    && lines[*previous].bbox.height.min(lines[index].bbox.height)
                        / lines[*previous]
                            .bbox
                            .height
                            .max(lines[index].bbox.height)
                            .max(0.001)
                        >= 0.65
                    && (lines[*previous].bbox.y - top(lines[index].bbox)).abs()
                        <= lines[*previous].bbox.height.max(lines[index].bbox.height) * 2.2
                    && (lines[*previous].bbox.x - lines[index].bbox.x).abs()
                        <= lines[*previous].bbox.height.max(lines[index].bbox.height) * 2.5
            });
        if append {
            groups.last_mut().unwrap().push(index);
        } else {
            groups.push(vec![index]);
        }
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(ordinal, group)| {
            let first = &lines[group[0]];
            let last = &lines[*group.last().unwrap()];
            let bbox = group.iter().skip(1).fold(first.bbox, |bbox, index| {
                union_box(bbox, lines[*index].bbox)
            });
            let confidence = group
                .iter()
                .map(|index| lines[*index].confidence)
                .fold(1.0, f64::min)
                * 0.95;
            let direction = aggregate_direction(group.iter().map(|index| lines[*index].direction));
            PdfTextBlock {
                index: ordinal as u64 + 1,
                text: group
                    .iter()
                    .map(|index| lines[*index].text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                line_start: first.index,
                line_end: last.index + 1,
                token_start: first.token_start,
                token_end: last.token_end,
                direction,
                bbox,
                confidence,
                locator: pdf_locator(
                    page,
                    bbox,
                    Some((first.token_start, last.token_end)),
                    confidence,
                ),
            }
        })
        .collect()
}

fn aggregate_direction(
    directions: impl Iterator<Item = PdfWritingDirection>,
) -> PdfWritingDirection {
    let values = directions
        .filter(|value| *value != PdfWritingDirection::Unknown)
        .collect::<BTreeSet<_>>();
    if values.is_empty() {
        PdfWritingDirection::Unknown
    } else if values.len() == 1 {
        *values.first().unwrap()
    } else {
        PdfWritingDirection::Mixed
    }
}

fn glyph_confidence(glyphs: &[PdfGlyph], indexes: &[usize]) -> f64 {
    indexes
        .iter()
        .map(|index| {
            glyphs[*index]
                .mapping_confidence
                .min(glyphs[*index].geometry_confidence)
        })
        .fold(1.0, f64::min)
}

fn pdf_locator(
    page: &PdfPage,
    bbox: BoundingBox,
    tokens: Option<(u64, u64)>,
    confidence: f64,
) -> SourceLocator {
    let component = LocationComponent::PdfRegion {
        page: IndexPosition::new(page.index, IndexBase::One).expect("one-based PDF page"),
        bbox: Some(bbox),
        rotation_degrees: Some(page.rotation_degrees),
        tokens: tokens.map(|(start, end)| {
            IndexRange::new(start, end, IndexBase::One).expect("valid native token range")
        }),
    };
    if confidence >= 0.999 {
        SourceLocator::exact(component).expect("valid native PDF region")
    } else {
        SourceLocator::approximate(
            component,
            LocatorConfidence::new(confidence.clamp(0.0, 1.0)).expect("bounded confidence"),
        )
        .expect("valid approximate native PDF region")
    }
}

fn union_box(left: BoundingBox, right_box: BoundingBox) -> BoundingBox {
    let x = left.x.min(right_box.x);
    let y = left.y.min(right_box.y);
    let max_x = right(left).max(right(right_box));
    let max_y = top(left).max(top(right_box));
    BoundingBox {
        x,
        y,
        width: max_x - x,
        height: max_y - y,
        unit: CoordinateUnit::Points,
        origin: CoordinateOrigin::BottomLeft,
    }
}

fn right(bbox: BoundingBox) -> f64 {
    bbox.x + bbox.width
}
fn top(bbox: BoundingBox) -> f64 {
    bbox.y + bbox.height
}
