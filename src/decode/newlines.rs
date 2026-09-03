//! Ordered newline inventory without normalization.

use super::codecs::MappedText;
use super::model::{
    DecodedByteRange, NewlineFidelity, NewlineKind, NewlineSequence, RawByteRange, TextEncoding,
};

pub(crate) fn inventory(
    decoded: &MappedText,
    raw_bytes: &[u8],
    encoding: &TextEncoding,
) -> NewlineFidelity {
    let mut fidelity = NewlineFidelity::default();
    let extended_raw = matches!(encoding, TextEncoding::Other(_))
        .then(|| ascii_compatible_raw_newlines(raw_bytes));
    let mut extended_index = 0;
    let bytes = decoded.text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let (kind, end) = match bytes[offset] {
            b'\r' if bytes.get(offset + 1) == Some(&b'\n') => (NewlineKind::CrLf, offset + 2),
            b'\r' => (NewlineKind::Cr, offset + 1),
            b'\n' => (NewlineKind::Lf, offset + 1),
            _ => {
                offset += 1;
                continue;
            }
        };
        let raw = extended_raw
            .as_ref()
            .and_then(|ranges| ranges.get(extended_index).copied())
            .or_else(|| decoded.raw_for_decoded_range(offset, end))
            .unwrap_or((0, raw_bytes.len()));
        extended_index += usize::from(extended_raw.is_some());
        match kind {
            NewlineKind::Lf => fidelity.lf_count += 1,
            NewlineKind::CrLf => fidelity.crlf_count += 1,
            NewlineKind::Cr => fidelity.cr_count += 1,
        }
        fidelity.sequences.push(NewlineSequence {
            kind,
            raw_range: RawByteRange::from_usize(raw.0, raw.1),
            decoded_range: DecodedByteRange::from_usize(offset, end),
        });
        offset = end;
    }
    fidelity.final_line_terminated = bytes.ends_with(b"\n") || bytes.ends_with(b"\r");
    fidelity
}

fn ascii_compatible_raw_newlines(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        match bytes[offset] {
            b'\r' if bytes.get(offset + 1) == Some(&b'\n') => {
                ranges.push((offset, offset + 2));
                offset += 2;
            }
            b'\r' | b'\n' => {
                ranges.push((offset, offset + 1));
                offset += 1;
            }
            _ => offset += 1,
        }
    }
    ranges
}
