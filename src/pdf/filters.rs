use super::{PdfStreamDecodeStatus, PdfValue};
use flate2::read::ZlibDecoder;
use std::collections::BTreeMap;
use std::io::Read;

#[derive(Debug)]
pub(crate) struct FilterDecode {
    pub bytes: Option<Vec<u8>>,
    pub filters: Vec<String>,
    pub supported: bool,
    pub status: PdfStreamDecodeStatus,
    pub error: Option<String>,
}

pub(crate) fn filter_names(dictionary: &BTreeMap<String, PdfValue>) -> Vec<String> {
    match dictionary.get("Filter") {
        None => Vec::new(),
        Some(PdfValue::Name(name)) => vec![canonical_filter(name)],
        Some(PdfValue::Array(values)) => values
            .iter()
            .filter_map(PdfValue::as_name)
            .map(canonical_filter)
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn decode_stream(
    encoded: &[u8],
    dictionary: &BTreeMap<String, PdfValue>,
    max_output: u64,
    encrypted: bool,
) -> FilterDecode {
    let filters = filter_names(dictionary);
    if encrypted {
        return FilterDecode {
            bytes: None,
            filters,
            supported: false,
            status: PdfStreamDecodeStatus::Encrypted,
            error: Some("stream bytes are encrypted".into()),
        };
    }
    if filters.is_empty() {
        return FilterDecode {
            bytes: Some(encoded.to_vec()),
            filters,
            supported: true,
            status: PdfStreamDecodeStatus::NotFiltered,
            error: None,
        };
    }
    let mut bytes = encoded.to_vec();
    for filter in filters.clone() {
        let result = match filter.as_str() {
            "FlateDecode" => decode_flate(&bytes, max_output),
            "ASCIIHexDecode" => decode_ascii_hex(&bytes),
            "ASCII85Decode" => decode_ascii85(&bytes),
            "RunLengthDecode" => decode_run_length(&bytes, max_output),
            "DCTDecode" | "JPXDecode" | "CCITTFaxDecode" | "JBIG2Decode" => {
                return FilterDecode {
                    bytes: None,
                    filters,
                    supported: true,
                    status: PdfStreamDecodeStatus::NativeEncoded,
                    error: None,
                };
            }
            other => {
                return FilterDecode {
                    bytes: None,
                    filters,
                    supported: false,
                    status: PdfStreamDecodeStatus::Unsupported,
                    error: Some(format!("unsupported PDF stream filter {other}")),
                };
            }
        };
        match result {
            Ok(decoded) if decoded.len() as u64 <= max_output => bytes = decoded,
            Ok(_) => {
                return FilterDecode {
                    bytes: None,
                    filters,
                    supported: true,
                    status: PdfStreamDecodeStatus::BudgetExceeded,
                    error: Some(format!("decoded stream exceeds {max_output} bytes")),
                };
            }
            Err(FilterError::Budget) => {
                return FilterDecode {
                    bytes: None,
                    filters,
                    supported: true,
                    status: PdfStreamDecodeStatus::BudgetExceeded,
                    error: Some(format!("decoded stream exceeds {max_output} bytes")),
                };
            }
            Err(FilterError::Malformed(message)) => {
                return FilterDecode {
                    bytes: None,
                    filters,
                    supported: true,
                    status: PdfStreamDecodeStatus::Malformed,
                    error: Some(message),
                };
            }
        }
    }
    if predictor(dictionary) > 1 {
        return FilterDecode {
            bytes: None,
            filters,
            supported: false,
            status: PdfStreamDecodeStatus::Unsupported,
            error: Some(
                "PDF stream predictor parameters are not supported by the core decoder".into(),
            ),
        };
    }
    FilterDecode {
        bytes: Some(bytes),
        filters,
        supported: true,
        status: PdfStreamDecodeStatus::Decoded,
        error: None,
    }
}

fn predictor(dictionary: &BTreeMap<String, PdfValue>) -> i64 {
    match dictionary.get("DecodeParms") {
        Some(PdfValue::Dictionary(parameters)) => parameters
            .get("Predictor")
            .and_then(PdfValue::as_integer)
            .unwrap_or(1),
        Some(PdfValue::Array(parameters)) => parameters
            .iter()
            .filter_map(PdfValue::as_dictionary)
            .filter_map(|parameters| parameters.get("Predictor"))
            .filter_map(PdfValue::as_integer)
            .max()
            .unwrap_or(1),
        _ => 1,
    }
}

fn canonical_filter(name: &str) -> String {
    match name {
        "Fl" => "FlateDecode",
        "AHx" => "ASCIIHexDecode",
        "A85" => "ASCII85Decode",
        "LZW" => "LZWDecode",
        "RL" => "RunLengthDecode",
        "CCF" => "CCITTFaxDecode",
        "DCT" => "DCTDecode",
        other => other,
    }
    .to_string()
}

#[derive(Debug)]
enum FilterError {
    Budget,
    Malformed(String),
}

fn decode_flate(bytes: &[u8], max_output: u64) -> Result<Vec<u8>, FilterError> {
    let mut decoder = ZlibDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .by_ref()
        .take(max_output.saturating_add(1))
        .read_to_end(&mut output)
        .map_err(|error| FilterError::Malformed(format!("invalid FlateDecode stream: {error}")))?;
    if output.len() as u64 > max_output {
        Err(FilterError::Budget)
    } else {
        Ok(output)
    }
}

fn decode_ascii_hex(bytes: &[u8]) -> Result<Vec<u8>, FilterError> {
    let mut nibbles = Vec::new();
    for &byte in bytes {
        if byte == b'>' {
            break;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }
        nibbles.push(
            hex(byte)
                .ok_or_else(|| FilterError::Malformed("invalid ASCIIHexDecode digit".into()))?,
        );
    }
    if nibbles.len() % 2 == 1 {
        nibbles.push(0);
    }
    Ok(nibbles
        .chunks_exact(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect())
}

fn decode_ascii85(bytes: &[u8]) -> Result<Vec<u8>, FilterError> {
    let mut output = Vec::new();
    let mut group = Vec::with_capacity(5);
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        cursor += 1;
        if byte.is_ascii_whitespace() || (byte == b'<' && bytes.get(cursor) == Some(&b'~')) {
            if byte == b'<' {
                cursor += 1;
            }
            continue;
        }
        if byte == b'~' && bytes.get(cursor) == Some(&b'>') {
            break;
        }
        if byte == b'z' {
            if !group.is_empty() {
                return Err(FilterError::Malformed("ASCII85 z inside a group".into()));
            }
            output.extend_from_slice(&[0; 4]);
            continue;
        }
        if !(b'!'..=b'u').contains(&byte) {
            return Err(FilterError::Malformed("invalid ASCII85Decode byte".into()));
        }
        group.push(u32::from(byte - b'!'));
        if group.len() == 5 {
            let value = group.iter().fold(0u32, |value, digit| {
                value.saturating_mul(85).saturating_add(*digit)
            });
            output.extend_from_slice(&value.to_be_bytes());
            group.clear();
        }
    }
    if group.len() == 1 {
        return Err(FilterError::Malformed("invalid final ASCII85 group".into()));
    }
    if !group.is_empty() {
        let retained = group.len() - 1;
        while group.len() < 5 {
            group.push(84);
        }
        let value = group.iter().fold(0u32, |value, digit| {
            value.saturating_mul(85).saturating_add(*digit)
        });
        output.extend_from_slice(&value.to_be_bytes()[..retained]);
    }
    Ok(output)
}

fn decode_run_length(bytes: &[u8], max_output: u64) -> Result<Vec<u8>, FilterError> {
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while let Some(&length) = bytes.get(cursor) {
        cursor += 1;
        match length {
            128 => break,
            0..=127 => {
                let count = usize::from(length) + 1;
                let end = cursor
                    .checked_add(count)
                    .ok_or_else(|| FilterError::Malformed("RunLengthDecode overflow".into()))?;
                let run = bytes.get(cursor..end).ok_or_else(|| {
                    FilterError::Malformed("truncated RunLengthDecode literal".into())
                })?;
                output.extend_from_slice(run);
                cursor = end;
            }
            129..=255 => {
                let byte = *bytes.get(cursor).ok_or_else(|| {
                    FilterError::Malformed("truncated RunLengthDecode repeat".into())
                })?;
                cursor += 1;
                output.extend(std::iter::repeat_n(byte, 257 - usize::from(length)));
            }
        }
        if output.len() as u64 > max_output {
            return Err(FilterError::Budget);
        }
    }
    Ok(output)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
