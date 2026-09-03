use super::{Builder, ImageResult, be_u16, be_u32};
use crate::core::sha256_hex;
use crate::image::{ImageDimensions, ImageMetadataKind};
use flate2::read::ZlibDecoder;
use std::collections::BTreeMap;
use std::io::Read;

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> ImageResult<()> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return Err("PNG signature is invalid".into());
    }
    let mut offset = SIGNATURE.len();
    let mut saw_header = false;
    let mut saw_end = false;
    let mut saw_image_data = false;
    let mut idat_ended = false;
    let mut color_type = None;
    let mut saw_palette = false;
    let mut animation_frames = None;
    let mut frame_controls = 0u32;
    let mut current_frame_has_data = false;
    let mut current_frame_data_kind = None;
    let mut data_bearing_frames = 0u32;
    let mut expected_animation_sequence = 0u32;
    while offset < bytes.len() {
        builder.checkpoint()?;
        if offset + 12 > bytes.len() {
            return Err("PNG chunk header or CRC is truncated".into());
        }
        let length = be_u32(bytes, offset)? as usize;
        let kind = String::from_utf8_lossy(&bytes[offset + 4..offset + 8]).into_owned();
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| "PNG chunk length overflows".to_string())?;
        let chunk_end = data_end
            .checked_add(4)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| format!("PNG {kind} chunk is truncated"))?;
        let data = &bytes[data_start..data_end];
        let expected_crc = be_u32(bytes, data_end)?;
        let actual_crc = png_crc32(&bytes[offset + 4..data_end], builder)?;
        if actual_crc != expected_crc {
            return Err(format!("PNG {kind} chunk CRC does not match its contents").into());
        }
        let known = matches!(
            kind.as_str(),
            "IHDR"
                | "PLTE"
                | "IDAT"
                | "IEND"
                | "tRNS"
                | "gAMA"
                | "cHRM"
                | "sRGB"
                | "iCCP"
                | "pHYs"
                | "tIME"
                | "eXIf"
                | "tEXt"
                | "zTXt"
                | "iTXt"
                | "acTL"
                | "fcTL"
                | "fdAT"
                | "sBIT"
                | "bKGD"
                | "hIST"
                | "sPLT"
        );
        if !known && bytes[offset + 4].is_ascii_uppercase() {
            return Err(format!("PNG contains unsupported critical chunk {kind}").into());
        }
        if kind == "IDAT" {
            if idat_ended {
                return Err("PNG IDAT chunks must be contiguous".into());
            }
        } else if saw_image_data {
            idat_ended = true;
        }
        builder.chunk(kind.clone(), known, data, offset, chunk_end)?;
        match kind.as_str() {
            "IHDR" => color_type = Some(parse_header(data, offset, saw_header, builder)?),
            "PLTE" => {
                if saw_palette || saw_image_data {
                    return Err("PNG PLTE must be unique and precede IDAT".into());
                }
                if data.is_empty() || data.len() % 3 != 0 || data.len() > 768 {
                    return Err("PNG PLTE has an invalid palette length".into());
                }
                if matches!(color_type, Some(0 | 4)) {
                    return Err("PNG grayscale color types must not contain PLTE".into());
                }
                saw_palette = true;
            }
            "acTL" => {
                if data.len() != 8 {
                    return Err("PNG acTL chunk must contain frame and play counts".into());
                }
                let declared = be_u32(data, 0)?;
                if animation_frames.replace(declared).is_some() || declared == 0 || saw_image_data {
                    return Err("PNG acTL frame count must be nonzero and unique".into());
                }
                if u64::from(declared) > builder.options.max_frames {
                    return Err("PNG animation frame count exceeds ImageOptions::max_frames".into());
                }
            }
            "fcTL" => {
                if animation_frames.is_none() {
                    return Err("PNG fcTL appears without an acTL animation declaration".into());
                }
                if frame_controls > 0 {
                    if !current_frame_has_data {
                        return Err("PNG animation frame has no IDAT/fdAT image data".into());
                    }
                    data_bearing_frames = data_bearing_frames.saturating_add(1);
                }
                check_animation_sequence(data, &mut expected_animation_sequence)?;
                parse_frame(data, builder)?;
                frame_controls = frame_controls.saturating_add(1);
                current_frame_has_data = false;
                current_frame_data_kind = None;
            }
            "IDAT" => {
                if data.is_empty() {
                    return Err("PNG IDAT image data must not be empty".into());
                }
                saw_image_data = true;
                if frame_controls > 0 {
                    if current_frame_data_kind == Some("fdAT") {
                        return Err("PNG animation frame cannot mix IDAT and fdAT data".into());
                    }
                    current_frame_has_data = true;
                    current_frame_data_kind = Some("IDAT");
                }
            }
            "fdAT" => {
                if animation_frames.is_none()
                    || frame_controls == 0
                    || !saw_image_data
                    || data.len() <= 4
                    || current_frame_data_kind == Some("IDAT")
                {
                    return Err("PNG fdAT appears without a current animation frame".into());
                }
                check_animation_sequence(data, &mut expected_animation_sequence)?;
                current_frame_has_data = true;
                current_frame_data_kind = Some("fdAT");
            }
            "eXIf" => builder.exif(data, data_start)?,
            "iCCP" => {
                builder.color.icc_profile_sha256 = Some(sha256_hex(data));
                builder.metadata(
                    ImageMetadataKind::Icc,
                    data,
                    data_start,
                    None,
                    BTreeMap::new(),
                )?;
            }
            "tEXt" | "zTXt" | "iTXt" => parse_text(&kind, data, data_start, builder)?,
            "IEND" => {
                if length != 0 {
                    return Err("PNG IEND chunk must be empty".into());
                }
                saw_end = true;
                offset = chunk_end;
                break;
            }
            _ => {}
        }
        saw_header |= kind == "IHDR";
        offset = chunk_end;
    }
    if !saw_header || !saw_end {
        return Err("PNG is missing IHDR or IEND".into());
    }
    if !saw_image_data {
        return Err("PNG contains no IDAT image data".into());
    }
    if color_type == Some(3) && !saw_palette {
        return Err("indexed-color PNG requires a PLTE before IDAT".into());
    }
    if let Some(declared) = animation_frames {
        if frame_controls > 0 && current_frame_has_data {
            data_bearing_frames = data_bearing_frames.saturating_add(1);
        }
        if frame_controls != declared || data_bearing_frames != declared {
            return Err("PNG animation frame controls/data do not match acTL".into());
        }
    }
    if offset != bytes.len() {
        return Err("PNG contains trailing bytes after IEND".into());
    }
    Ok(())
}

fn parse_header(
    data: &[u8],
    offset: usize,
    saw_header: bool,
    builder: &mut Builder<'_>,
) -> ImageResult<u8> {
    if saw_header || data.len() != 13 || offset != 8 {
        return Err("PNG IHDR must be the first unique 13-byte chunk".into());
    }
    builder.dimensions = ImageDimensions {
        width: be_u32(data, 0)?,
        height: be_u32(data, 4)?,
    };
    builder.color.bits_per_component = data.get(8).copied();
    let color_type = *data
        .get(9)
        .ok_or_else(|| "PNG IHDR is truncated".to_string())?;
    let bit_depth = data[8];
    let valid_depth = match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 | 4 | 6 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        _ => false,
    };
    if !valid_depth || data[10] != 0 || data[11] != 0 || data[12] > 1 {
        return Err(
            "PNG IHDR bit depth, compression, filter, or interlace method is invalid".into(),
        );
    }
    let (model, components, alpha) = match color_type {
        0 => ("grayscale", 1, false),
        2 => ("rgb", 3, false),
        3 => ("indexed", 1, false),
        4 => ("grayscale_alpha", 2, true),
        6 => ("rgba", 4, true),
        _ => return Err(format!("PNG color type {color_type} is invalid").into()),
    };
    builder.color.model = Some(model.into());
    builder.color.component_count = Some(components);
    builder.color.alpha = Some(alpha);
    Ok(color_type)
}

fn check_animation_sequence(data: &[u8], expected: &mut u32) -> ImageResult<()> {
    let sequence = be_u32(data, 0)?;
    if sequence != *expected {
        return Err("PNG APNG sequence numbers must be monotonic from zero".into());
    }
    *expected = expected.saturating_add(1);
    Ok(())
}

fn parse_frame(data: &[u8], builder: &mut Builder<'_>) -> ImageResult<()> {
    if data.len() != 26 {
        return Err("PNG fcTL chunk has an invalid length".into());
    }
    let dimensions = ImageDimensions {
        width: be_u32(data, 4)?,
        height: be_u32(data, 8)?,
    };
    let x = be_u32(data, 12)?;
    let y = be_u32(data, 16)?;
    if dimensions.width == 0
        || dimensions.height == 0
        || x.checked_add(dimensions.width)
            .is_none_or(|end| end > builder.dimensions.width)
        || y.checked_add(dimensions.height)
            .is_none_or(|end| end > builder.dimensions.height)
    {
        return Err("PNG animation frame is outside the image canvas".into());
    }
    let numerator = u64::from(be_u16(data, 20)?);
    let denominator = match u64::from(be_u16(data, 22)?) {
        0 => 100,
        value => value,
    };
    builder.frame(
        dimensions,
        Some(numerator.saturating_mul(1000) / denominator),
    )?;
    let frame = builder.frames.last_mut().unwrap();
    frame.x = x;
    frame.y = y;
    frame.disposal = Some(
        match data[24] {
            0 => "none",
            1 => "background",
            2 => "previous",
            _ => return Err("PNG frame disposal operation is invalid".into()),
        }
        .into(),
    );
    frame.blend = Some(
        match data[25] {
            0 => "source",
            1 => "over",
            _ => return Err("PNG frame blend operation is invalid".into()),
        }
        .into(),
    );
    Ok(())
}

fn parse_text(kind: &str, data: &[u8], start: usize, builder: &mut Builder<'_>) -> ImageResult<()> {
    let separator = data
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| format!("PNG {kind} text chunk has no keyword terminator"))?;
    let keyword = String::from_utf8_lossy(&data[..separator]).to_string();
    let value = if kind == "tEXt" {
        let encoded = &data[separator + 1..];
        builder.charge_decoded_characters(encoded.len() as u64)?;
        String::from_utf8_lossy(encoded).to_string()
    } else if kind == "zTXt" {
        let tail = &data[separator + 1..];
        if tail.first() != Some(&0) {
            return Err("PNG zTXt compression method is invalid".into());
        }
        decode_zlib_text(&tail[1..], builder.options.max_metadata_bytes, builder)?
    } else {
        let tail = &data[separator + 1..];
        if tail.len() < 4 {
            return Err("PNG iTXt control fields are truncated".into());
        }
        if tail[0] > 1 || tail[1] != 0 {
            return Err("PNG iTXt compression fields are invalid".into());
        }
        let language_end = tail[2..]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "PNG iTXt language tag is truncated".to_string())?
            + 2;
        let translated_end = tail[language_end + 1..]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "PNG iTXt translated keyword is truncated".to_string())?
            + language_end
            + 1;
        let encoded = &tail[translated_end + 1..];
        if tail[0] == 1 {
            decode_zlib_text(encoded, builder.options.max_metadata_bytes, builder)?
        } else {
            builder.charge_decoded_characters(encoded.len() as u64)?;
            String::from_utf8_lossy(encoded).to_string()
        }
    };
    let lower = keyword.to_ascii_lowercase();
    if lower.contains("xmp") || value.contains("<x:xmpmeta") || value.contains("<rdf:RDF") {
        builder.xmp_precharged(value.as_bytes(), start)
    } else if lower.contains("iptc") {
        builder.iptc_precharged(value.as_bytes(), start)
    } else {
        builder.metadata_precharged(
            ImageMetadataKind::Text,
            data,
            start,
            Some(value),
            BTreeMap::from([("keyword".into(), keyword)]),
        )
    }
}

fn decode_zlib_text(bytes: &[u8], maximum: u64, builder: &Builder<'_>) -> ImageResult<String> {
    let mut decoder = ZlibDecoder::new(bytes);
    let mut decoded = Vec::new();
    let mut buffer = [0u8; 8 * 1024];
    loop {
        builder.checkpoint()?;
        let read = decoder
            .read(&mut buffer)
            .map_err(|error| format!("PNG compressed text cannot be decoded: {error}"))?;
        if read == 0 {
            break;
        }
        let next = decoded
            .len()
            .checked_add(read)
            .ok_or("PNG compressed text length overflows")?;
        if next as u64 > maximum {
            return Err("PNG compressed text exceeds ImageOptions::max_metadata_bytes".into());
        }
        builder.charge_decoded_characters(read as u64)?;
        decoded.extend_from_slice(&buffer[..read]);
    }
    Ok(String::from_utf8_lossy(&decoded).into_owned())
}

fn png_crc32(bytes: &[u8], builder: &Builder<'_>) -> ImageResult<u32> {
    let mut crc = u32::MAX;
    for chunk in bytes.chunks(64 * 1024) {
        builder.checkpoint()?;
        for byte in chunk {
            crc = CRC32_TABLE[((crc as u8) ^ *byte) as usize] ^ (crc >> 8);
        }
    }
    Ok(!crc)
}

const CRC32_TABLE: [u32; 256] = crc32_table();

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < table.len() {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = (value & 1).wrapping_neg();
            value = (value >> 1) ^ (0xedb8_8320 & mask);
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}
