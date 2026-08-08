use super::{Builder, be_u32, be_u64, find_bytes};
use crate::image::ImageDimensions;

pub(super) fn matches(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    let size = u32::from_be_bytes(bytes[0..4].try_into().unwrap()) as usize;
    if size < 16 || size > bytes.len() {
        return false;
    }
    let supported = |brand: &[u8]| {
        matches!(
            brand,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis"
        )
    };
    supported(&bytes[8..12]) || bytes[16..size].chunks_exact(4).any(supported)
}

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> Result<(), String> {
    if !matches(bytes) {
        return Err("HEIF/HEIC ftyp brand is missing".into());
    }
    let mut dimensions = Vec::new();
    walk_boxes(bytes, 0, bytes.len(), 0, builder, &mut dimensions)?;
    builder.dimensions = dimensions
        .first()
        .copied()
        .ok_or_else(|| "HEIF contains no ispe image dimensions".to_string())?;
    for dimension in dimensions {
        builder.frame(dimension, None)?;
    }
    builder
        .color
        .model
        .get_or_insert_with(|| "yuv_or_rgb".into());
    Ok(())
}

fn walk_boxes(
    bytes: &[u8],
    start: usize,
    end: usize,
    depth: u8,
    builder: &mut Builder<'_>,
    dimensions: &mut Vec<ImageDimensions>,
) -> Result<(), String> {
    if depth > 16 {
        return Err("HEIF box nesting exceeds the safety ceiling".into());
    }
    let mut offset = start;
    while offset + 8 <= end {
        builder.checkpoint()?;
        let size32 = be_u32(bytes, offset)? as usize;
        let kind = String::from_utf8_lossy(&bytes[offset + 4..offset + 8]).into_owned();
        let (header, box_end) = if size32 == 1 {
            let size = usize::try_from(be_u64(bytes, offset + 8)?)
                .map_err(|_| "HEIF extended box size exceeds address space")?;
            (
                16,
                offset
                    .checked_add(size)
                    .ok_or_else(|| "HEIF box size overflows".to_string())?,
            )
        } else if size32 == 0 {
            (8, end)
        } else {
            (
                8,
                offset
                    .checked_add(size32)
                    .ok_or_else(|| "HEIF box size overflows".to_string())?,
            )
        };
        if box_end > end || box_end < offset + header {
            return Err(format!("HEIF {kind} box is truncated or invalid"));
        }
        let data_start = offset + header;
        let data = &bytes[data_start..box_end];
        let known = matches!(
            kind.as_str(),
            "ftyp"
                | "meta"
                | "moov"
                | "trak"
                | "mdia"
                | "minf"
                | "stbl"
                | "dinf"
                | "iprp"
                | "ipco"
                | "iinf"
                | "iloc"
                | "pitm"
                | "iref"
                | "ispe"
                | "irot"
                | "imir"
                | "colr"
                | "Exif"
                | "mime"
                | "infe"
                | "mdat"
                | "hdlr"
        );
        builder.chunk(kind.clone(), known, data, offset, box_end)?;
        match kind.as_str() {
            "ispe" if data.len() >= 12 => dimensions.push(ImageDimensions {
                width: be_u32(data, 4)?,
                height: be_u32(data, 8)?,
            }),
            "irot" if !data.is_empty() => {
                builder.orientation.rotation_degrees = i16::from(data[0] & 3) * 90
            }
            "imir" if !data.is_empty() => builder.orientation.mirrored = true,
            "colr" if data.len() >= 4 => {
                builder.color.color_space = Some(String::from_utf8_lossy(&data[..4]).into_owned())
            }
            "Exif" => {
                let tiff_offset = if data.len() >= 4 {
                    be_u32(data, 0).unwrap_or(0) as usize + 4
                } else {
                    0
                };
                if tiff_offset < data.len() {
                    builder.exif(&data[tiff_offset..], data_start + tiff_offset)?;
                }
            }
            "mime"
                if find_bytes(data, b"<x:xmpmeta").is_some()
                    || find_bytes(data, b"<rdf:RDF").is_some() =>
            {
                builder.xmp(data, data_start)?
            }
            _ if is_container(&kind) => {
                let child_start = if kind == "meta" {
                    data_start.saturating_add(4)
                } else {
                    data_start
                };
                if child_start <= box_end {
                    walk_boxes(bytes, child_start, box_end, depth + 1, builder, dimensions)?;
                }
            }
            _ => {}
        }
        offset = box_end;
    }
    if offset != end && bytes[offset..end].iter().any(|byte| *byte != 0) {
        return Err("HEIF box table has non-padding trailing bytes".into());
    }
    Ok(())
}

fn is_container(kind: &str) -> bool {
    matches!(
        kind,
        "meta" | "moov" | "trak" | "mdia" | "minf" | "stbl" | "dinf" | "iprp" | "ipco"
    )
}
