use super::{MimeParameter, MimeValue};
use std::collections::BTreeMap;

pub(super) struct DecodedBytes {
    pub bytes: Vec<u8>,
    pub malformed: bool,
}

pub(super) fn transfer_decode(input: &[u8], encoding: &str) -> DecodedBytes {
    match encoding.trim().to_ascii_lowercase().as_str() {
        "base64" => decode_base64(input),
        "quoted-printable" => decode_quoted_printable(input, false),
        "7bit" | "8bit" | "binary" | "" => DecodedBytes {
            bytes: input.to_vec(),
            malformed: false,
        },
        _ => DecodedBytes {
            bytes: input.to_vec(),
            malformed: true,
        },
    }
}

pub(super) fn decode_charset(bytes: &[u8], label: &str) -> (String, String, bool) {
    let normalized = label.trim().trim_matches(['\'', '"']).to_ascii_lowercase();
    match normalized.as_str() {
        "utf-8" | "utf8" => match std::str::from_utf8(bytes) {
            Ok(value) => (value.to_string(), "utf-8".into(), false),
            Err(_) => (
                String::from_utf8_lossy(bytes).into_owned(),
                "utf-8".into(),
                true,
            ),
        },
        "us-ascii" | "ascii" if bytes.is_ascii() => (
            String::from_utf8_lossy(bytes).into_owned(),
            "us-ascii".into(),
            false,
        ),
        "us-ascii" | "ascii" => (decode_windows_1252(bytes), "us-ascii".into(), true),
        "iso-8859-1" | "iso8859-1" | "latin1" => (
            bytes.iter().map(|byte| char::from(*byte)).collect(),
            "iso-8859-1".into(),
            false,
        ),
        "windows-1252" | "cp1252" => (decode_windows_1252(bytes), "windows-1252".into(), false),
        "utf-16" | "utf-16le" => decode_utf16(bytes, true),
        "utf-16be" => decode_utf16(bytes, false),
        _ => (
            String::from_utf8_lossy(bytes).into_owned(),
            normalized,
            true,
        ),
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> (String, String, bool) {
    let malformed = bytes.len() % 2 != 0;
    let units = bytes.chunks_exact(2).map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    let mut had_errors = malformed;
    let value = char::decode_utf16(units)
        .map(|value| {
            value.unwrap_or_else(|_| {
                had_errors = true;
                char::REPLACEMENT_CHARACTER
            })
        })
        .collect();
    (
        value,
        if little_endian {
            "utf-16le"
        } else {
            "utf-16be"
        }
        .into(),
        had_errors,
    )
}

fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| match byte {
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
            value => char::from(*value),
        })
        .collect()
}

pub(super) fn decode_rfc2047(value: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    let mut previous_encoded = false;
    while let Some(relative) = value[cursor..].find("=?") {
        let start = cursor + relative;
        let prefix = &value[cursor..start];
        if !(previous_encoded && prefix.chars().all(char::is_whitespace)) {
            output.push_str(prefix);
        }
        let Some(relative_end) = value[start + 2..].find("?=") else {
            output.push_str(&value[start..]);
            return output;
        };
        let end = start + 2 + relative_end + 2;
        let token = &value[start + 2..end - 2];
        let mut fields = token.splitn(3, '?');
        let (Some(charset), Some(kind), Some(encoded)) =
            (fields.next(), fields.next(), fields.next())
        else {
            output.push_str(&value[start..end]);
            cursor = end;
            previous_encoded = false;
            continue;
        };
        let decoded = match kind.to_ascii_lowercase().as_str() {
            "b" => decode_base64(encoded.as_bytes()),
            "q" => decode_quoted_printable(encoded.replace('_', " ").as_bytes(), true),
            _ => DecodedBytes {
                bytes: Vec::new(),
                malformed: true,
            },
        };
        if decoded.malformed {
            output.push_str(&value[start..end]);
            previous_encoded = false;
        } else {
            output.push_str(&decode_charset(&decoded.bytes, charset).0);
            previous_encoded = true;
        }
        cursor = end;
    }
    output.push_str(&value[cursor..]);
    output
}

pub(super) fn parse_mime_value(raw: &str, default_essence: &str) -> MimeValue {
    let fields = split_quoted(raw, ';');
    let essence = fields
        .first()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_essence.to_string());
    let mut grouped: BTreeMap<String, Vec<ParameterFragment>> = BTreeMap::new();
    for field in fields.into_iter().skip(1) {
        let Some((raw_name, raw_value)) = field.split_once('=') else {
            continue;
        };
        let raw_name = raw_name.trim().to_string();
        let raw_value = raw_value.trim().to_string();
        let (name, index, extended) = parameter_name(&raw_name);
        grouped.entry(name).or_default().push(ParameterFragment {
            index,
            extended,
            raw_name,
            raw_value,
        });
    }
    let mut parameters = Vec::new();
    for (name, mut fragments) in grouped {
        fragments.sort_by_key(|fragment| fragment.index.unwrap_or(0));
        let segmented = fragments.iter().any(|fragment| fragment.index.is_some());
        let extended = fragments.iter().any(|fragment| fragment.extended);
        let segments = fragments
            .iter()
            .map(|fragment| unquote(&fragment.raw_value))
            .collect::<Vec<_>>();
        let joined = if segmented {
            segments.join("")
        } else {
            segments.first().cloned().unwrap_or_default()
        };
        let (value, charset, language) = if extended {
            decode_extended_parameter(&joined)
        } else {
            (decode_rfc2047(&joined), None, None)
        };
        parameters.push(MimeParameter {
            name,
            raw_name: fragments
                .iter()
                .map(|fragment| fragment.raw_name.as_str())
                .collect::<Vec<_>>()
                .join(";"),
            raw_value: fragments
                .iter()
                .map(|fragment| fragment.raw_value.as_str())
                .collect::<Vec<_>>()
                .join(""),
            value,
            extended,
            charset,
            language,
            segments,
        });
    }
    MimeValue {
        raw: raw.to_string(),
        essence,
        parameters,
    }
}

struct ParameterFragment {
    index: Option<usize>,
    extended: bool,
    raw_name: String,
    raw_value: String,
}

fn parameter_name(raw: &str) -> (String, Option<usize>, bool) {
    let lower = raw.to_ascii_lowercase();
    let Some(star) = lower.find('*') else {
        return (lower, None, false);
    };
    let name = lower[..star].to_string();
    let suffix = &lower[star + 1..];
    let extended = suffix.ends_with('*') || suffix.is_empty();
    let digits = suffix.trim_end_matches('*');
    let index = (!digits.is_empty())
        .then(|| digits.parse::<usize>().ok())
        .flatten();
    (name, index, extended)
}

fn decode_extended_parameter(value: &str) -> (String, Option<String>, Option<String>) {
    let mut pieces = value.splitn(3, '\'');
    let first = pieces.next().unwrap_or_default();
    let second = pieces.next();
    let third = pieces.next();
    let (charset, language, encoded) = match (second, third) {
        (Some(language), Some(encoded)) => (
            Some(first.to_string()),
            (!language.is_empty()).then(|| language.to_string()),
            encoded,
        ),
        _ => (None, None, value),
    };
    let (bytes, _) = percent_decode(encoded.as_bytes());
    let decoded = if let Some(charset) = charset.as_deref() {
        decode_charset(&bytes, charset).0
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    (decoded, charset, language)
}

fn split_quoted(value: &str, delimiter: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut quote = false;
    let mut escape = false;
    for (index, ch) in value.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && quote {
            escape = true;
        } else if ch == '"' {
            quote = !quote;
        } else if ch == delimiter && !quote {
            result.push(&value[start..index]);
            start = index + ch.len_utf8();
        }
    }
    result.push(&value[start..]);
    result
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(trimmed);
    let mut output = String::new();
    let mut escape = false;
    for ch in inner.chars() {
        if escape {
            output.push(ch);
            escape = false;
        } else if ch == '\\' {
            escape = true;
        } else {
            output.push(ch);
        }
    }
    if escape {
        output.push('\\');
    }
    output
}

fn decode_base64(input: &[u8]) -> DecodedBytes {
    let mut output = Vec::with_capacity(input.len().saturating_mul(3) / 4);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    let mut padding = false;
    let mut malformed = false;
    for &byte in input {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            padding = true;
            continue;
        }
        if padding {
            malformed = true;
            continue;
        }
        let Some(value) = base64_value(byte) else {
            malformed = true;
            continue;
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((accumulator >> bits) & 0xff) as u8);
            accumulator &= (1_u32 << bits).saturating_sub(1);
        }
    }
    if bits == 6 || (bits > 0 && accumulator != 0) {
        malformed = true;
    }
    DecodedBytes {
        bytes: output,
        malformed,
    }
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn decode_quoted_printable(input: &[u8], encoded_word: bool) -> DecodedBytes {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    let mut malformed = false;
    while index < input.len() {
        if input[index] != b'=' {
            output.push(input[index]);
            index += 1;
            continue;
        }
        if !encoded_word && input.get(index + 1) == Some(&b'\n') {
            index += 2;
            continue;
        }
        if !encoded_word
            && input.get(index + 1) == Some(&b'\r')
            && input.get(index + 2) == Some(&b'\n')
        {
            index += 3;
            continue;
        }
        match (input.get(index + 1), input.get(index + 2)) {
            (Some(high), Some(low)) => match (hex(*high), hex(*low)) {
                (Some(high), Some(low)) => {
                    output.push((high << 4) | low);
                    index += 3;
                }
                _ => {
                    malformed = true;
                    output.push(b'=');
                    index += 1;
                }
            },
            _ => {
                malformed = true;
                output.push(b'=');
                index += 1;
            }
        }
    }
    DecodedBytes {
        bytes: output,
        malformed,
    }
}

fn percent_decode(input: &[u8]) -> (Vec<u8>, bool) {
    let mut output = Vec::with_capacity(input.len());
    let mut malformed = false;
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' && index + 2 < input.len() {
            if let (Some(high), Some(low)) = (hex(input[index + 1]), hex(input[index + 2])) {
                output.push((high << 4) | low);
                index += 3;
                continue;
            }
            malformed = true;
        }
        output.push(input[index]);
        index += 1;
    }
    (output, malformed)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_encoded_words_and_rfc2231_parameters() {
        assert_eq!(decode_rfc2047("=?UTF-8?Q?Ol=C3=A1?="), "Olá");
        let value = parse_mime_value(
            "attachment; filename*0*=UTF-8''report%20; filename*1*=final.pdf",
            "attachment",
        );
        assert_eq!(value.parameter("filename"), Some("report final.pdf"));
    }

    #[test]
    fn transfer_decoders_report_malformed_input_without_panicking() {
        assert_eq!(transfer_decode(b"SGVsbG8=", "base64").bytes, b"Hello");
        assert!(transfer_decode(b"SGV$", "base64").malformed);
        assert_eq!(
            transfer_decode(b"hello=20world", "quoted-printable").bytes,
            b"hello world"
        );
    }
}
