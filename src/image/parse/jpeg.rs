use super::{Builder, ImageResult, be_u16, find_bytes};
use crate::core::sha256_hex;
use crate::image::{ImageDimensions, ImageMetadataKind};
use std::collections::BTreeMap;

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> ImageResult<()> {
    if !bytes.starts_with(b"\xff\xd8") {
        return Err("JPEG start-of-image marker is missing".into());
    }
    let mut offset = 2usize;
    let mut saw_end = false;
    let mut saw_scan = false;
    let mut in_scan = false;
    let mut active_scan_has_data = false;
    let mut frame_marker = None;
    let mut frame_components = None;
    let mut scanned_components = std::collections::BTreeSet::new();
    'segments: while offset < bytes.len() {
        builder.checkpoint()?;
        if bytes[offset] != 0xff {
            if !in_scan {
                return Err("JPEG contains entropy bytes before SOS".into());
            }
            active_scan_has_data = true;
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
        if in_scan && !matches!(marker, 0x00 | 0xd0..=0xd7) {
            if !active_scan_has_data {
                return Err("JPEG scan contains no entropy data".into());
            }
            in_scan = false;
        }
        match marker {
            0x00 => {
                if !in_scan {
                    return Err("JPEG stuffed zero appears outside entropy data".into());
                }
                active_scan_has_data = true;
            }
            0xd8 => return Err("JPEG contains an unexpected nested start-of-image marker".into()),
            0xd9 => {
                saw_end = true;
                break 'segments;
            }
            0x01 => {}
            0xd0..=0xd7 if in_scan => {}
            0xd0..=0xd7 => return Err("JPEG restart marker appears before SOS".into()),
            0xda => {
                let length = be_u16(bytes, offset)? as usize;
                if length < 2 || offset + length > bytes.len() {
                    return Err("JPEG SOS segment is truncated".into());
                }
                validate_sos(
                    &bytes[offset + 2..offset + length],
                    frame_marker,
                    frame_components.as_ref(),
                    &mut scanned_components,
                )?;
                builder.chunk(
                    "SOS",
                    true,
                    &bytes[offset + 2..offset + length],
                    offset - 2,
                    offset + length,
                )?;
                saw_scan = true;
                in_scan = true;
                active_scan_has_data = false;
                offset += length;
            }
            _ => {
                let length = be_u16(bytes, offset)? as usize;
                if length < 2 {
                    return Err(format!("JPEG marker {marker:02x} has invalid length").into());
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
                if is_sof(marker) {
                    if frame_marker.replace(marker).is_some() {
                        return Err("JPEG contains more than one start-of-frame segment".into());
                    }
                }
                inspect(marker, data, data_start, builder)?;
                if is_sof(marker) {
                    frame_components = Some(
                        data[6..]
                            .chunks_exact(3)
                            .map(|component| component[0])
                            .collect::<std::collections::BTreeSet<_>>(),
                    );
                }
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
    if !saw_scan {
        return Err("JPEG contains no non-empty start-of-scan image data".into());
    }
    if frame_components
        .as_ref()
        .is_some_and(|components| !components.is_subset(&scanned_components))
    {
        return Err("JPEG scans do not cover every frame component".into());
    }
    builder.frame(builder.dimensions, None)?;
    Ok(())
}

fn inspect(marker: u8, data: &[u8], start: usize, builder: &mut Builder<'_>) -> ImageResult<()> {
    if is_sof(marker) {
        if data.len() < 6 {
            return Err("JPEG SOF segment is truncated".into());
        }
        let component_count = usize::from(data[5]);
        if component_count == 0 || data.len() != 6usize.saturating_add(3 * component_count) {
            return Err("JPEG SOF component count does not match its component records".into());
        }
        let mut selectors = std::collections::BTreeSet::new();
        for component in data[6..].chunks_exact(3) {
            if !selectors.insert(component[0]) || component[1] >> 4 == 0 || component[1] & 15 == 0 {
                return Err("JPEG SOF component selectors or sampling factors are invalid".into());
            }
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

fn validate_sos(
    data: &[u8],
    frame_marker: Option<u8>,
    frame_components: Option<&std::collections::BTreeSet<u8>>,
    scanned_components: &mut std::collections::BTreeSet<u8>,
) -> ImageResult<()> {
    let frame_marker = frame_marker.ok_or("JPEG SOS appears before a start-of-frame segment")?;
    let frame_components = frame_components.ok_or("JPEG SOS has no frame component table")?;
    let component_count = usize::from(
        *data
            .first()
            .ok_or("JPEG SOS component count is truncated")?,
    );
    if component_count == 0
        || component_count > 4
        || data.len() != 1usize.saturating_add(2 * component_count).saturating_add(3)
    {
        return Err("JPEG SOS component count does not match its selector table".into());
    }
    let mut selectors = std::collections::BTreeSet::new();
    for selector in data[1..1 + 2 * component_count].chunks_exact(2) {
        if !selectors.insert(selector[0])
            || !frame_components.contains(&selector[0])
            || selector[1] >> 4 > 3
            || selector[1] & 15 > 3
        {
            return Err("JPEG SOS component selectors or table indices are invalid".into());
        }
        scanned_components.insert(selector[0]);
    }
    let parameters = &data[1 + 2 * component_count..];
    let (spectral_start, spectral_end, approximation) =
        (parameters[0], parameters[1], parameters[2]);
    if spectral_start > spectral_end
        || spectral_end > 63
        || approximation >> 4 > 13
        || approximation & 15 > 13
    {
        return Err("JPEG SOS spectral selection or approximation is invalid".into());
    }
    if matches!(frame_marker, 0xc0 | 0xc1)
        && (spectral_start != 0 || spectral_end != 63 || approximation != 0)
    {
        return Err("sequential JPEG SOS parameters are invalid".into());
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
