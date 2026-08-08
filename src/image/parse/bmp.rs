use super::{Builder, ImageResult, le_i32, le_u16, le_u32};
use crate::core::sha256_hex;
use crate::image::{ImageDimensions, ImageMetadataKind};
use std::collections::BTreeMap;

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> ImageResult<()> {
    if bytes.len() < 26 || !bytes.starts_with(b"BM") {
        return Err("BMP file or DIB header is truncated".into());
    }
    let declared = le_u32(bytes, 2)? as usize;
    if declared != 0 && declared > bytes.len() {
        return Err("BMP declared file size exceeds the input".into());
    }
    if declared != 0 && declared != bytes.len() {
        return Err("BMP declared file size does not match the input".into());
    }
    let dib = le_u32(bytes, 14)?;
    let pixel_offset = le_u32(bytes, 10)? as usize;
    let mut compression = 0u32;
    let bits;
    if dib == 12 {
        if le_u16(bytes, 22)? != 1 {
            return Err("BMP color plane count must be one".into());
        }
        builder.dimensions = ImageDimensions {
            width: u32::from(le_u16(bytes, 18)?),
            height: u32::from(le_u16(bytes, 20)?),
        };
        bits = le_u16(bytes, 24)?;
        builder.color.bits_per_component = Some(u8::try_from(bits).unwrap_or(u8::MAX));
    } else if dib >= 40 {
        if bytes.len() < 14 + dib as usize {
            return Err("BMP DIB header is truncated".into());
        }
        let width = le_i32(bytes, 18)?;
        let height = le_i32(bytes, 22)?;
        if width <= 0 || height == i32::MIN || height == 0 {
            return Err("BMP dimensions are invalid".into());
        }
        builder.dimensions = ImageDimensions {
            width: width as u32,
            height: height.unsigned_abs(),
        };
        if le_u16(bytes, 26)? != 1 {
            return Err("BMP color plane count must be one".into());
        }
        bits = le_u16(bytes, 28)?;
        compression = le_u32(bytes, 30)?;
        builder.color.bits_per_component = Some(u8::try_from(bits.min(255)).unwrap());
        builder.color.component_count = Some(if bits >= 24 { 3 } else { 1 });
        builder.color.alpha = Some(bits == 32);
        builder.color.model = Some(if bits >= 24 { "bgr" } else { "indexed_rgb" }.into());
        if dib >= 108 {
            builder.color.color_space = Some(String::from_utf8_lossy(&bytes[70..74]).into_owned());
        }
        if dib >= 124 {
            let profile_offset = le_u32(bytes, 126)? as usize + 14;
            let profile_size = le_u32(bytes, 130)? as usize;
            if profile_size > 0 {
                let end = profile_offset
                    .checked_add(profile_size)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| "BMP embedded color profile is out of bounds".to_string())?;
                let profile = &bytes[profile_offset..end];
                builder.color.icc_profile_sha256 = Some(sha256_hex(profile));
                builder.metadata(
                    ImageMetadataKind::Icc,
                    profile,
                    profile_offset,
                    None,
                    BTreeMap::new(),
                )?;
            }
        }
    } else {
        return Err(format!("BMP DIB header size {dib} is unsupported").into());
    }
    if !matches!(bits, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(format!("BMP bit depth {bits} is unsupported").into());
    }
    if !matches!(compression, 0 | 1 | 2 | 3 | 6) {
        return Err(format!("BMP compression method {compression} is unsupported").into());
    }
    let end = 14usize
        .checked_add(dib as usize)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "BMP DIB range is invalid".to_string())?;
    if pixel_offset < end || pixel_offset > bytes.len() {
        return Err("BMP pixel array offset is outside the file".into());
    }
    if compression == 0 {
        let row_bits = u64::from(builder.dimensions.width)
            .checked_mul(u64::from(bits))
            .ok_or_else(|| "BMP row size overflows".to_string())?;
        let row_bytes = row_bits.saturating_add(31) / 32 * 4;
        let pixel_bytes = row_bytes
            .checked_mul(u64::from(builder.dimensions.height))
            .ok_or_else(|| "BMP pixel array size overflows".to_string())?;
        let available = bytes.len().saturating_sub(pixel_offset) as u64;
        if pixel_bytes > available {
            return Err("BMP uncompressed pixel array is truncated".into());
        }
    }
    builder.chunk(format!("dib-{dib}"), true, &bytes[14..end], 14, end)?;
    Ok(())
}
