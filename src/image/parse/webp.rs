use super::{Builder, le_u16, le_u24, le_u32};
use crate::core::sha256_hex;
use crate::image::{ImageDimensions, ImageMetadataKind};
use std::collections::BTreeMap;

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> Result<(), String> {
    if bytes.len() < 12 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return Err("WebP RIFF signature is invalid".into());
    }
    let declared = le_u32(bytes, 4)? as usize + 8;
    if declared > bytes.len() {
        return Err("WebP RIFF length exceeds the input".into());
    }
    if declared != bytes.len() {
        return Err("WebP contains bytes outside the declared RIFF container".into());
    }
    let mut offset = 12usize;
    while offset + 8 <= declared {
        builder.checkpoint()?;
        let kind = String::from_utf8_lossy(&bytes[offset..offset + 4]).into_owned();
        let length = le_u32(bytes, offset + 4)? as usize;
        let start = offset + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= declared)
            .ok_or_else(|| format!("WebP {kind} chunk is truncated"))?;
        let data = &bytes[start..end];
        let known = matches!(
            kind.as_str(),
            "VP8X" | "VP8 " | "VP8L" | "ALPH" | "ANIM" | "ANMF" | "ICCP" | "EXIF" | "XMP "
        );
        builder.chunk(kind.clone(), known, data, offset, end)?;
        match kind.as_str() {
            "VP8X" if data.len() >= 10 => {
                builder.dimensions = ImageDimensions {
                    width: le_u24(data, 4)?.saturating_add(1),
                    height: le_u24(data, 7)?.saturating_add(1),
                };
                builder.color.alpha = Some(data[0] & 0x10 != 0);
            }
            "VP8L" if data.len() >= 5 && data[0] == 0x2f => {
                let bits = le_u32(data, 1)?;
                builder.dimensions = ImageDimensions {
                    width: (bits & 0x3fff) + 1,
                    height: ((bits >> 14) & 0x3fff) + 1,
                };
                builder.color.model = Some("rgba".into());
                builder.color.alpha = Some(true);
            }
            "VP8 " if data.len() >= 10 && data[3..6] == [0x9d, 0x01, 0x2a] => {
                builder.dimensions = ImageDimensions {
                    width: u32::from(le_u16(data, 6)? & 0x3fff),
                    height: u32::from(le_u16(data, 8)? & 0x3fff),
                };
                builder.color.model = Some("yuv".into());
            }
            "ANMF" if data.len() >= 16 => {
                validate_frame_payload(&data[16..])?;
                let dimensions = ImageDimensions {
                    width: le_u24(data, 6)?.saturating_add(1),
                    height: le_u24(data, 9)?.saturating_add(1),
                };
                builder.frame(dimensions, Some(u64::from(le_u24(data, 12)?)))?;
                let frame = builder.frames.last_mut().unwrap();
                frame.x = le_u24(data, 0)?.saturating_mul(2);
                frame.y = le_u24(data, 3)?.saturating_mul(2);
                frame.blend = Some(if data[15] & 2 == 0 { "over" } else { "source" }.into());
                frame.disposal = Some(
                    if data[15] & 1 == 0 {
                        "none"
                    } else {
                        "background"
                    }
                    .into(),
                );
            }
            "EXIF" => {
                let prefix = usize::from(data.starts_with(b"Exif\0\0")) * 6;
                builder.exif(&data[prefix..], start + prefix)?;
            }
            "XMP " => builder.xmp(data, start)?,
            "ICCP" => {
                builder.color.icc_profile_sha256 = Some(sha256_hex(data));
                builder.metadata(ImageMetadataKind::Icc, data, start, None, BTreeMap::new())?;
            }
            _ => {}
        }
        offset = end + (length & 1);
    }
    if offset != declared && offset + 1 != declared {
        return Err("WebP chunk table has trailing or misaligned bytes".into());
    }
    Ok(())
}

fn validate_frame_payload(mut bytes: &[u8]) -> Result<(), String> {
    let mut saw_image = false;
    while !bytes.is_empty() {
        if bytes.len() < 8 {
            return Err("WebP animation frame subchunk header is truncated".into());
        }
        let kind = &bytes[..4];
        let length = le_u32(bytes, 4)? as usize;
        let end = 8usize
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "WebP animation frame subchunk is truncated".to_string())?;
        if matches!(kind, b"VP8 " | b"VP8L") {
            saw_image = true;
        } else if kind != b"ALPH" {
            return Err("WebP animation frame contains an unsupported subchunk".into());
        }
        let padded = end.saturating_add(length & 1);
        if padded > bytes.len() {
            return Err("WebP animation frame subchunk padding is truncated".into());
        }
        bytes = &bytes[padded..];
    }
    if !saw_image {
        return Err("WebP animation frame has no VP8/VP8L image payload".into());
    }
    Ok(())
}
