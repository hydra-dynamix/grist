use super::{Builder, be_u16, find_bytes};
use crate::core::sha256_hex;
use crate::image::{ImageDimensions, ImageMetadataKind};
use std::collections::BTreeMap;

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> Result<(), String> {
    if !bytes.starts_with(b"\xff\xd8") {
        return Err("JPEG start-of-image marker is missing".into());
    }
    let mut offset = 2usize;
    let mut saw_end = false;
    'segments: while offset < bytes.len() {
        builder.checkpoint()?;
        if bytes[offset] != 0xff {
            offset += 1;
            continue;
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        let marker = *bytes
            .get(offset)
            .ok_or_else(|| "JPEG marker is truncated".to_string())?;
        offset += 1;
        match marker {
            0x00 => {}
            0xd8 => return Err("JPEG contains an unexpected nested start-of-image marker".into()),
            0xd9 => {
                saw_end = true;
                break 'segments;
            }
            0x01 | 0xd0..=0xd7 => {}
            0xda => {
                let length = be_u16(bytes, offset)? as usize;
                if length < 2 || offset + length > bytes.len() {
                    return Err("JPEG SOS segment is truncated".into());
                }
                builder.chunk(
                    "SOS",
                    true,
                    &bytes[offset + 2..offset + length],
                    offset - 2,
                    offset + length,
                )?;
                offset += length;
            }
            _ => {
                let length = be_u16(bytes, offset)? as usize;
                if length < 2 {
                    return Err(format!("JPEG marker {marker:02x} has invalid length"));
                }
                let end = offset
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| format!("JPEG marker {marker:02x} is truncated"))?;
                let data_start = offset + 2;
                let data = &bytes[data_start..end];
                let known = is_sof(marker)
                    || matches!(
                        marker,
                        0xdb | 0xc4 | 0xdd | 0xe0 | 0xe1 | 0xe2 | 0xed | 0xfe
                    );
                builder.chunk(marker_name(marker), known, data, offset - 2, end)?;
                inspect(marker, data, data_start, builder)?;
                offset = end;
            }
        }
    }
    if !saw_end {
        return Err("JPEG end-of-image marker is missing".into());
    }
    if bytes[offset..].iter().any(|byte| *byte != 0) {
        return Err("JPEG contains non-padding bytes after end-of-image".into());
    }
    if builder.dimensions.width == 0 {
        return Err("JPEG contains no supported start-of-frame segment".into());
    }
    builder.frame(builder.dimensions, None)?;
    Ok(())
}

fn inspect(marker: u8, data: &[u8], start: usize, builder: &mut Builder<'_>) -> Result<(), String> {
    if is_sof(marker) {
        if data.len() < 6 {
            return Err("JPEG SOF segment is truncated".into());
        }
        builder.color.bits_per_component = data.first().copied();
        builder.dimensions = ImageDimensions {
            width: u32::from(be_u16(data, 3)?),
            height: u32::from(be_u16(data, 1)?),
        };
        builder.color.component_count = data.get(5).copied();
        builder.color.model = Some(
            match data[5] {
                1 => "grayscale",
                3 => "ycbcr_or_rgb",
                4 => "cmyk",
                _ => "unknown",
            }
            .into(),
        );
    } else if marker == 0xe1 {
        if data.starts_with(b"Exif\0\0") {
            builder.exif(&data[6..], start + 6)?;
        } else if data.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
            builder.xmp(&data[29..], start + 29)?;
        }
    } else if marker == 0xed {
        if let Some(position) = find_bytes(data, b"\x1c\x02") {
            builder.iptc(&data[position..], start + position)?;
        }
    } else if marker == 0xe2 && data.starts_with(b"ICC_PROFILE\0") {
        builder.color.icc_profile_sha256 = Some(sha256_hex(data));
        builder.metadata(ImageMetadataKind::Icc, data, start, None, BTreeMap::new())?;
    } else if marker == 0xfe {
        builder.metadata(
            ImageMetadataKind::Comment,
            data,
            start,
            Some(String::from_utf8_lossy(data).into_owned()),
            BTreeMap::new(),
        )?;
    }
    Ok(())
}

fn marker_name(marker: u8) -> String {
    match marker {
        0xe0 => "APP0".into(),
        0xe1 => "APP1".into(),
        0xe2 => "APP2".into(),
        0xed => "APP13".into(),
        0xfe => "COM".into(),
        value if is_sof(value) => format!("SOF{value:02x}"),
        value => format!("marker-{value:02x}"),
    }
}
fn is_sof(marker: u8) -> bool {
    matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf)
}
