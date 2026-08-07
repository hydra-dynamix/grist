//! Exact UTF and Windows-1252 decoders with raw-to-decoded span mapping.

use super::model::{DecodeIssue, DecodeIssueKind, DecodedByteRange, RawByteRange, TextEncoding};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapSegment {
    pub raw_start: usize,
    pub raw_end: usize,
    pub decoded_start: usize,
    pub decoded_end: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MappedText {
    pub text: String,
    pub map: Vec<MapSegment>,
    pub issues: Vec<DecodeIssue>,
}

impl MappedText {
    fn push_char(&mut self, character: char, raw_start: usize, raw_end: usize) {
        let decoded_start = self.text.len();
        self.text.push(character);
        self.map.push(MapSegment {
            raw_start,
            raw_end,
            decoded_start,
            decoded_end: self.text.len(),
        });
    }

    fn push_replacement(
        &mut self,
        raw_start: usize,
        raw_end: usize,
        kind: DecodeIssueKind,
        message: impl Into<String>,
    ) {
        let decoded_start = self.text.len();
        self.push_char(char::REPLACEMENT_CHARACTER, raw_start, raw_end);
        self.issues.push(DecodeIssue {
            kind,
            raw_range: RawByteRange::from_usize(raw_start, raw_end),
            decoded_range: Some(DecodedByteRange::from_usize(decoded_start, self.text.len())),
            conflicting_raw_range: None,
            message: message.into(),
        });
    }

    pub fn raw_for_decoded_range(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        let first = self
            .map
            .iter()
            .find(|segment| segment.decoded_end > start || segment.decoded_start == start)?;
        let last = self
            .map
            .iter()
            .rev()
            .find(|segment| segment.decoded_start < end || segment.decoded_end == end)?;
        Some((first.raw_start, last.raw_end))
    }
}

pub(crate) fn decode(bytes: &[u8], encoding: &TextEncoding, content_start: usize) -> MappedText {
    match encoding {
        TextEncoding::Utf8 => decode_utf8(bytes, content_start),
        TextEncoding::Utf16Le => decode_utf16(bytes, content_start, false),
        TextEncoding::Utf16Be => decode_utf16(bytes, content_start, true),
        TextEncoding::Utf32Le => decode_utf32(bytes, content_start, false),
        TextEncoding::Utf32Be => decode_utf32(bytes, content_start, true),
        TextEncoding::Windows1252 => decode_windows_1252(bytes, content_start),
        TextEncoding::Other(label) => decode_extended(bytes, content_start, label),
    }
}

fn decode_utf8(bytes: &[u8], raw_offset: usize) -> MappedText {
    let mut output = MappedText::default();
    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(valid) => {
                push_utf8_valid(&mut output, valid, raw_offset + offset);
                break;
            }
            Err(error) => {
                let valid_end = offset + error.valid_up_to();
                let valid = std::str::from_utf8(&bytes[offset..valid_end])
                    .expect("Utf8Error valid prefix invariant");
                push_utf8_valid(&mut output, valid, raw_offset + offset);
                let malformed_len = error
                    .error_len()
                    .unwrap_or_else(|| bytes.len().saturating_sub(valid_end));
                let malformed_end = valid_end.saturating_add(malformed_len).min(bytes.len());
                output.push_replacement(
                    raw_offset + valid_end,
                    raw_offset + malformed_end,
                    if error.error_len().is_some() {
                        DecodeIssueKind::UndecodableSequence
                    } else {
                        DecodeIssueKind::TruncatedCodeUnit
                    },
                    "invalid UTF-8 byte sequence was replaced",
                );
                offset = malformed_end;
            }
        }
    }
    output
}

fn push_utf8_valid(output: &mut MappedText, text: &str, raw_start: usize) {
    for (relative, character) in text.char_indices() {
        output.push_char(
            character,
            raw_start + relative,
            raw_start + relative + character.len_utf8(),
        );
    }
}

fn decode_utf16(bytes: &[u8], raw_offset: usize, big_endian: bool) -> MappedText {
    let mut output = MappedText::default();
    let mut offset = 0;
    while offset + 1 < bytes.len() {
        let unit = read_u16(&bytes[offset..offset + 2], big_endian);
        if (0xd800..=0xdbff).contains(&unit) {
            if offset + 3 < bytes.len() {
                let low = read_u16(&bytes[offset + 2..offset + 4], big_endian);
                if (0xdc00..=0xdfff).contains(&low) {
                    let scalar =
                        0x1_0000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
                    output.push_char(
                        char::from_u32(scalar).expect("valid surrogate pair"),
                        raw_offset + offset,
                        raw_offset + offset + 4,
                    );
                    offset += 4;
                    continue;
                }
            }
            output.push_replacement(
                raw_offset + offset,
                raw_offset + offset + 2,
                DecodeIssueKind::UndecodableSequence,
                "unpaired UTF-16 high surrogate was replaced",
            );
        } else if (0xdc00..=0xdfff).contains(&unit) {
            output.push_replacement(
                raw_offset + offset,
                raw_offset + offset + 2,
                DecodeIssueKind::UndecodableSequence,
                "unpaired UTF-16 low surrogate was replaced",
            );
        } else {
            output.push_char(
                char::from_u32(u32::from(unit)).expect("non-surrogate UTF-16 unit"),
                raw_offset + offset,
                raw_offset + offset + 2,
            );
        }
        offset += 2;
    }
    if offset < bytes.len() {
        output.push_replacement(
            raw_offset + offset,
            raw_offset + bytes.len(),
            DecodeIssueKind::TruncatedCodeUnit,
            "trailing partial UTF-16 code unit was replaced",
        );
    }
    output
}

fn read_u16(bytes: &[u8], big_endian: bool) -> u16 {
    let pair = [bytes[0], bytes[1]];
    if big_endian {
        u16::from_be_bytes(pair)
    } else {
        u16::from_le_bytes(pair)
    }
}

fn decode_utf32(bytes: &[u8], raw_offset: usize, big_endian: bool) -> MappedText {
    let mut output = MappedText::default();
    let complete = bytes.len() / 4 * 4;
    for offset in (0..complete).step_by(4) {
        let word: [u8; 4] = bytes[offset..offset + 4]
            .try_into()
            .expect("four-byte UTF-32 unit");
        let scalar = if big_endian {
            u32::from_be_bytes(word)
        } else {
            u32::from_le_bytes(word)
        };
        if let Some(character) = char::from_u32(scalar) {
            output.push_char(character, raw_offset + offset, raw_offset + offset + 4);
        } else {
            output.push_replacement(
                raw_offset + offset,
                raw_offset + offset + 4,
                DecodeIssueKind::UndecodableSequence,
                "invalid UTF-32 scalar value was replaced",
            );
        }
    }
    if complete < bytes.len() {
        output.push_replacement(
            raw_offset + complete,
            raw_offset + bytes.len(),
            DecodeIssueKind::TruncatedCodeUnit,
            "trailing partial UTF-32 code unit was replaced",
        );
    }
    output
}

fn decode_windows_1252(bytes: &[u8], raw_offset: usize) -> MappedText {
    let mut output = MappedText::default();
    for (offset, byte) in bytes.iter().copied().enumerate() {
        if let Some(character) = windows_1252_character(byte) {
            output.push_char(character, raw_offset + offset, raw_offset + offset + 1);
        } else {
            output.push_replacement(
                raw_offset + offset,
                raw_offset + offset + 1,
                DecodeIssueKind::UndecodableSequence,
                "undefined Windows-1252 byte was replaced",
            );
        }
    }
    output
}

fn windows_1252_character(byte: u8) -> Option<char> {
    Some(match byte {
        0x80 => '\u{20ac}',
        0x81 | 0x8d | 0x8f | 0x90 | 0x9d => return None,
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
    })
}

#[cfg(feature = "extended-encodings")]
fn decode_extended(bytes: &[u8], raw_offset: usize, label: &str) -> MappedText {
    use encoding_rs::DecoderResult;

    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
        .expect("extended labels are normalized through encoding_rs");
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let mut output = MappedText {
        text: String::with_capacity(bytes.len().saturating_mul(2).saturating_add(32)),
        map: Vec::new(),
        issues: Vec::new(),
    };
    let mut input_offset = 0;
    loop {
        let (result, read) = decoder.decode_to_string_without_replacement(
            &bytes[input_offset..],
            &mut output.text,
            true,
        );
        input_offset += read;
        match result {
            DecoderResult::InputEmpty => break,
            DecoderResult::OutputFull => output.text.reserve(bytes.len().saturating_add(32)),
            DecoderResult::Malformed(length, after) => {
                let raw_end = input_offset.saturating_sub(usize::from(after));
                let raw_start = raw_end.saturating_sub(usize::from(length));
                let decoded_start = output.text.len();
                output.text.push(char::REPLACEMENT_CHARACTER);
                output.issues.push(DecodeIssue {
                    kind: DecodeIssueKind::UndecodableSequence,
                    raw_range: RawByteRange::from_usize(
                        raw_offset + raw_start,
                        raw_offset + raw_end,
                    ),
                    decoded_range: Some(DecodedByteRange::from_usize(
                        decoded_start,
                        output.text.len(),
                    )),
                    conflicting_raw_range: None,
                    message: format!("invalid {label} byte sequence was replaced"),
                });
            }
        }
    }
    if !output.text.is_empty() {
        output.map.push(MapSegment {
            raw_start: raw_offset,
            raw_end: raw_offset + bytes.len(),
            decoded_start: 0,
            decoded_end: output.text.len(),
        });
    }
    output
}

#[cfg(not(feature = "extended-encodings"))]
fn decode_extended(_bytes: &[u8], _raw_offset: usize, label: &str) -> MappedText {
    unreachable!("unsupported extended encoding {label} must be rejected before decoding")
}

pub(crate) fn contains_valid_utf8_multibyte(bytes: &[u8]) -> bool {
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset].is_ascii() {
            offset += 1;
            continue;
        }
        for length in 2..=4 {
            let end = offset + length;
            if end <= bytes.len()
                && std::str::from_utf8(&bytes[offset..end])
                    .ok()
                    .is_some_and(|text| text.chars().count() == 1)
            {
                return true;
            }
        }
        offset += 1;
    }
    false
}
