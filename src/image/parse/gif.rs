use super::{Builder, find_bytes, le_u16};
use crate::image::{ImageDimensions, ImageMetadataKind};
use std::collections::BTreeMap;

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> Result<(), String> {
    if bytes.len() < 13 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Err("GIF header or logical screen descriptor is truncated".into());
    }
    builder.dimensions = ImageDimensions {
        width: u32::from(le_u16(bytes, 6)?),
        height: u32::from(le_u16(bytes, 8)?),
    };
    builder.color.bits_per_component = Some(((bytes[10] >> 4) & 7) + 1);
    builder.color.model = Some("indexed_rgb".into());
    builder.color.component_count = Some(3);
    builder.color.alpha = Some(false);
    let mut offset = 13usize;
    if bytes[10] & 0x80 != 0 {
        offset = offset
            .checked_add(3usize << ((bytes[10] & 7) as usize + 1))
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "GIF global color table is truncated".to_string())?;
    }
    let mut duration = None;
    let mut disposal = None;
    let mut saw_trailer = false;
    while offset < bytes.len() {
        builder.checkpoint()?;
        match bytes[offset] {
            0x2c => {
                if offset + 10 > bytes.len() {
                    return Err("GIF image descriptor is truncated".into());
                }
                let dimensions = ImageDimensions {
                    width: u32::from(le_u16(bytes, offset + 5)?),
                    height: u32::from(le_u16(bytes, offset + 7)?),
                };
                let mut data = offset + 10;
                let packed = bytes[offset + 9];
                if packed & 0x80 != 0 {
                    data = data
                        .checked_add(3usize << ((packed & 7) as usize + 1))
                        .filter(|end| *end <= bytes.len())
                        .ok_or_else(|| "GIF local color table is truncated".to_string())?;
                }
                let code_size = *bytes
                    .get(data)
                    .ok_or_else(|| "GIF LZW code size is missing".to_string())?;
                if !(2..=8).contains(&code_size) {
                    return Err("GIF LZW minimum code size is invalid".into());
                }
                data += 1;
                let end = sub_blocks_end(bytes, data)?;
                builder.chunk("image_descriptor", true, &bytes[offset..end], offset, end)?;
                builder.frame(dimensions, duration.take())?;
                let frame = builder.frames.last_mut().unwrap();
                frame.x = u32::from(le_u16(bytes, offset + 1)?);
                frame.y = u32::from(le_u16(bytes, offset + 3)?);
                frame.disposal = disposal.take();
                if frame
                    .x
                    .checked_add(frame.dimensions.width)
                    .is_none_or(|end| end > builder.dimensions.width)
                    || frame
                        .y
                        .checked_add(frame.dimensions.height)
                        .is_none_or(|end| end > builder.dimensions.height)
                {
                    return Err("GIF image descriptor is outside the logical screen".into());
                }
                offset = end;
            }
            0x21 => {
                let label = *bytes
                    .get(offset + 1)
                    .ok_or_else(|| "GIF extension label is truncated".to_string())?;
                let end = sub_blocks_end(bytes, offset + 2)?;
                let payload = &bytes[offset + 2..end];
                builder.chunk(
                    format!("extension-{label:02x}"),
                    matches!(label, 0xf9 | 0xfe | 0xff | 0x01),
                    payload,
                    offset,
                    end,
                )?;
                if label == 0xf9 {
                    if payload.len() < 6 || payload[0] != 4 {
                        return Err("GIF graphics control extension is malformed".into());
                    }
                    duration = Some(u64::from(le_u16(payload, 2)?) * 10);
                    let method = (payload[1] >> 2) & 7;
                    if method > 3 {
                        return Err("GIF disposal method is reserved".into());
                    }
                    disposal = Some(
                        match method {
                            0 => "unspecified",
                            1 => "none",
                            2 => "background",
                            3 => "previous",
                            _ => unreachable!(),
                        }
                        .into(),
                    );
                    if payload[1] & 1 != 0 {
                        builder.color.alpha = Some(true);
                    }
                } else if matches!(label, 0xfe | 0x01) {
                    let value = collect_sub_blocks(payload, if label == 0x01 { 13 } else { 0 })?;
                    builder.metadata(
                        if label == 0xfe {
                            ImageMetadataKind::Comment
                        } else {
                            ImageMetadataKind::Text
                        },
                        &value,
                        offset + 2,
                        Some(String::from_utf8_lossy(&value).into_owned()),
                        BTreeMap::new(),
                    )?;
                } else if label == 0xff {
                    let value = collect_sub_blocks(payload, 0)?;
                    if find_bytes(&value, b"XMP DataXMP").is_some() {
                        builder.xmp(&value, offset + 2)?;
                    }
                }
                offset = end;
            }
            0x3b => {
                saw_trailer = true;
                offset += 1;
                break;
            }
            value => {
                return Err(format!(
                    "GIF contains unknown top-level block 0x{value:02x}"
                ));
            }
        }
    }
    if !saw_trailer {
        return Err("GIF trailer is missing".into());
    }
    if offset != bytes.len() {
        return Err("GIF contains trailing bytes after the trailer".into());
    }
    Ok(())
}

fn sub_blocks_end(bytes: &[u8], mut offset: usize) -> Result<usize, String> {
    loop {
        let size = *bytes
            .get(offset)
            .ok_or_else(|| "GIF data sub-block chain is truncated".to_string())?
            as usize;
        offset += 1;
        if size == 0 {
            return Ok(offset);
        }
        offset = offset
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "GIF data sub-block is truncated".to_string())?;
    }
}

fn collect_sub_blocks(bytes: &[u8], mut offset: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    while offset < bytes.len() {
        let size = bytes[offset] as usize;
        offset += 1;
        if size == 0 {
            return Ok(out);
        }
        let end = offset
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "GIF extension sub-block is truncated".to_string())?;
        out.extend_from_slice(&bytes[offset..end]);
        offset = end;
    }
    Err("GIF extension has no sub-block terminator".into())
}
