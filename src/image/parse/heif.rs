use super::{Builder, ImageResult, be_u16, be_u32, be_u64};
use crate::image::ImageDimensions;
use std::collections::BTreeMap;

#[derive(Debug, Default)]
struct HeifIndex {
    meta_boxes: u32,
    primary_item: Option<u32>,
    items: BTreeMap<u32, ItemInfo>,
    locations: BTreeMap<u32, ItemLocation>,
    properties: Vec<ItemProperty>,
    associations: BTreeMap<u32, Vec<u16>>,
    references: Vec<ItemReference>,
    mdat_ranges: Vec<(usize, usize)>,
}

#[derive(Debug)]
struct ItemInfo {
    item_type: [u8; 4],
    content_type: Option<String>,
}

#[derive(Debug)]
struct ItemLocation {
    construction_method: u16,
    data_reference_index: u16,
    base_offset: u64,
    extents: Vec<Extent>,
}

#[derive(Debug)]
struct Extent {
    offset: u64,
    length: u64,
}

#[derive(Debug)]
enum ItemProperty {
    Dimensions(ImageDimensions),
    Rotation(i16),
    Mirror,
    Color(String),
    Other,
}

#[derive(Debug)]
struct ItemReference {
    kind: [u8; 4],
    from: u32,
    to: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
struct BoxView {
    kind: [u8; 4],
    start: usize,
    data_start: usize,
    end: usize,
}

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

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> ImageResult<()> {
    if !matches(bytes) {
        return Err("HEIF/HEIC ftyp brand is missing".into());
    }
    let mut index = HeifIndex::default();
    scan_boxes(bytes, 0, bytes.len(), 0, builder, &mut index)?;
    normalize_mdat_ranges(&mut index.mdat_ranges);

    let primary = index
        .primary_item
        .ok_or("HEIF meta contains no pitm primary item")?;
    let primary_info = index
        .items
        .get(&primary)
        .ok_or_else(|| format!("HEIF primary item {primary} has no infe item information"))?;
    if !is_image_item(primary_info.item_type) {
        return Err(format!("HEIF primary item {primary} is not an image item").into());
    }
    validate_primary_data(bytes, primary, &index, builder)?;

    let primary_properties = index
        .associations
        .get(&primary)
        .ok_or_else(|| format!("HEIF primary item {primary} has no ipma property association"))?;
    let mut dimensions = None;
    for property_index in primary_properties {
        builder.checkpoint()?;
        let property = index
            .properties
            .get(usize::from(*property_index).saturating_sub(1))
            .ok_or_else(|| format!("HEIF ipma references missing property {property_index}"))?;
        match property {
            ItemProperty::Dimensions(value) => dimensions = Some(*value),
            ItemProperty::Rotation(value) => builder.orientation.rotation_degrees = *value,
            ItemProperty::Mirror => builder.orientation.mirrored = true,
            ItemProperty::Color(value) => builder.color.color_space = Some(value.clone()),
            ItemProperty::Other => {}
        }
    }
    let dimensions = dimensions
        .ok_or_else(|| format!("HEIF primary item {primary} has no associated ispe dimensions"))?;
    builder.dimensions = dimensions;
    builder.frame(dimensions, None)?;

    let mut metadata_for_primary = std::collections::BTreeSet::new();
    for reference in &index.references {
        builder.checkpoint()?;
        if reference.kind == *b"cdsc" && reference.to.contains(&primary) {
            metadata_for_primary.insert(reference.from);
        }
    }
    for (item_id, info) in &index.items {
        builder.checkpoint()?;
        if !is_metadata_item(info) || !metadata_for_primary.contains(item_id) {
            continue;
        }
        let (data, start) = item_data(bytes, *item_id, &index, builder)?;
        if info.item_type == *b"Exif" {
            if data.len() < 4 {
                return Err(format!("HEIF Exif item {item_id} is truncated").into());
            }
            let offset = 4usize
                .checked_add(be_u32(data, 0)? as usize)
                .ok_or("HEIF Exif TIFF offset overflows")?;
            let tiff = data
                .get(offset..)
                .ok_or_else(|| format!("HEIF Exif item {item_id} TIFF offset is out of bounds"))?;
            builder.exif(tiff, start.saturating_add(offset))?;
        } else {
            builder.xmp(data, start)?;
        }
    }
    builder
        .color
        .model
        .get_or_insert_with(|| "yuv_or_rgb".into());
    Ok(())
}

fn scan_boxes(
    bytes: &[u8],
    start: usize,
    end: usize,
    depth: u8,
    builder: &mut Builder<'_>,
    index: &mut HeifIndex,
) -> ImageResult<()> {
    if depth > 16 {
        return Err("HEIF box nesting exceeds the safety ceiling".into());
    }
    let mut offset = start;
    while offset < end {
        builder.checkpoint()?;
        let view = parse_box(bytes, offset, end)?;
        let kind = String::from_utf8_lossy(&view.kind).into_owned();
        let data = &bytes[view.data_start..view.end];
        builder.chunk(
            kind.clone(),
            is_known_box(&view.kind),
            data,
            view.start,
            view.end,
        )?;
        match &view.kind {
            b"meta" => {
                index.meta_boxes = index.meta_boxes.saturating_add(1);
                if index.meta_boxes > 1 {
                    return Err(
                        "HEIF contains multiple meta graphs; cross-meta association is forbidden"
                            .into(),
                    );
                }
                if data.len() < 4 {
                    return Err("HEIF meta full-box header is truncated".into());
                }
                scan_boxes(
                    bytes,
                    view.data_start + 4,
                    view.end,
                    depth + 1,
                    builder,
                    index,
                )?;
            }
            b"pitm" => parse_pitm(data, index)?,
            b"iinf" => parse_iinf(bytes, view, depth + 1, builder, index)?,
            b"iloc" => parse_iloc(data, index, builder)?,
            b"iref" => parse_iref(bytes, view, depth + 1, builder, index)?,
            b"ipco" => parse_ipco(bytes, view, depth + 1, builder, index)?,
            b"ipma" => parse_ipma(data, index, builder)?,
            b"mdat" => index.mdat_ranges.push((view.data_start, view.end)),
            kind if is_plain_container(kind) => {
                scan_boxes(bytes, view.data_start, view.end, depth + 1, builder, index)?;
            }
            _ => {}
        }
        offset = view.end;
    }
    Ok(())
}

fn parse_box(bytes: &[u8], offset: usize, parent_end: usize) -> Result<BoxView, String> {
    if offset + 8 > parent_end {
        return Err("HEIF box header is truncated".into());
    }
    let size32 = be_u32(bytes, offset)? as usize;
    let kind = bytes[offset + 4..offset + 8].try_into().unwrap();
    let (header, end) = if size32 == 1 {
        let size = usize::try_from(be_u64(bytes, offset + 8)?)
            .map_err(|_| "HEIF extended box size exceeds address space")?;
        (16usize, offset.checked_add(size))
    } else if size32 == 0 {
        (8usize, Some(parent_end))
    } else {
        (8usize, offset.checked_add(size32))
    };
    let end = end.ok_or_else(|| "HEIF box size overflows".to_string())?;
    if end > parent_end || end < offset.saturating_add(header) {
        return Err("HEIF box is truncated or has an invalid extent".into());
    }
    Ok(BoxView {
        kind,
        start: offset,
        data_start: offset + header,
        end,
    })
}

fn parse_pitm(data: &[u8], index: &mut HeifIndex) -> Result<(), String> {
    let version = *data
        .first()
        .ok_or("HEIF pitm full-box header is truncated")?;
    let item = if version == 0 {
        u32::from(be_u16(data, 4)?)
    } else {
        be_u32(data, 4)?
    };
    if index.primary_item.replace(item).is_some() {
        return Err("HEIF contains more than one pitm box".into());
    }
    Ok(())
}

fn parse_iinf(
    bytes: &[u8],
    view: BoxView,
    depth: u8,
    builder: &mut Builder<'_>,
    index: &mut HeifIndex,
) -> ImageResult<()> {
    let data = &bytes[view.data_start..view.end];
    let version = *data
        .first()
        .ok_or("HEIF iinf full-box header is truncated")?;
    let (declared, mut offset) = if version == 0 {
        (u32::from(be_u16(data, 4)?), view.data_start + 6)
    } else {
        (be_u32(data, 4)?, view.data_start + 8)
    };
    let mut parsed = 0u32;
    while offset < view.end {
        builder.checkpoint()?;
        if depth > 16 {
            return Err("HEIF box nesting exceeds the safety ceiling".into());
        }
        let child = parse_box(bytes, offset, view.end)?;
        let child_data = &bytes[child.data_start..child.end];
        builder.chunk(
            String::from_utf8_lossy(&child.kind).into_owned(),
            child.kind == *b"infe",
            child_data,
            child.start,
            child.end,
        )?;
        if child.kind == *b"infe" {
            let (id, info) = parse_infe(child_data)?;
            builder.charge_records(1)?;
            builder.charge_nodes(1)?;
            if index.items.insert(id, info).is_some() {
                return Err(format!("HEIF item {id} has duplicate infe records").into());
            }
            parsed = parsed.saturating_add(1);
        }
        offset = child.end;
    }
    if parsed != declared {
        return Err("HEIF iinf item count does not match its infe children".into());
    }
    Ok(())
}

fn parse_infe(data: &[u8]) -> Result<(u32, ItemInfo), String> {
    let version = *data
        .first()
        .ok_or("HEIF infe full-box header is truncated")?;
    let (id, mut offset) = match version {
        2 => (u32::from(be_u16(data, 4)?), 8usize),
        3 => (be_u32(data, 4)?, 10usize),
        _ => return Err("HEIF infe versions before 2 are not supported".into()),
    };
    let item_type: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or("HEIF infe item type is truncated")?
        .try_into()
        .unwrap();
    offset += 4;
    let (_, consumed) = c_string(&data[offset..])?;
    offset += consumed;
    let content_type = if item_type == *b"mime" {
        let (value, _) = c_string(&data[offset..])?;
        Some(value)
    } else {
        None
    };
    Ok((
        id,
        ItemInfo {
            item_type,
            content_type,
        },
    ))
}

fn parse_iloc(data: &[u8], index: &mut HeifIndex, builder: &mut Builder<'_>) -> ImageResult<()> {
    if data.len() < 8 {
        return Err("HEIF iloc full-box header is truncated".into());
    }
    let version = data[0];
    if version > 2 {
        return Err(format!("HEIF iloc version {version} is unsupported").into());
    }
    let offset_size = data[4] >> 4;
    let length_size = data[4] & 15;
    let base_offset_size = data[5] >> 4;
    let index_size = if version > 0 { data[5] & 15 } else { 0 };
    if [offset_size, length_size, base_offset_size, index_size]
        .iter()
        .any(|size| *size > 8)
    {
        return Err("HEIF iloc integer field width exceeds 8 bytes".into());
    }
    let mut cursor = 6usize;
    let count = if version < 2 {
        let count = u32::from(be_u16(data, cursor)?);
        cursor += 2;
        count
    } else {
        let count = be_u32(data, cursor)?;
        cursor += 4;
        count
    };
    for _ in 0..count {
        builder.checkpoint()?;
        builder.charge_records(1)?;
        builder.charge_nodes(1)?;
        let id = if version < 2 {
            let value = u32::from(be_u16(data, cursor)?);
            cursor += 2;
            value
        } else {
            let value = be_u32(data, cursor)?;
            cursor += 4;
            value
        };
        let construction_method = if version > 0 {
            let value = be_u16(data, cursor)? & 15;
            cursor += 2;
            value
        } else {
            0
        };
        let data_reference_index = be_u16(data, cursor)?;
        cursor += 2;
        let base_offset = read_uint(data, &mut cursor, base_offset_size)?;
        let extent_count = be_u16(data, cursor)?;
        cursor += 2;
        let mut extents = Vec::with_capacity(usize::from(extent_count));
        for _ in 0..extent_count {
            builder.checkpoint()?;
            builder.charge_nodes(1)?;
            if version > 0 && index_size > 0 {
                let _ = read_uint(data, &mut cursor, index_size)?;
            }
            extents.push(Extent {
                offset: read_uint(data, &mut cursor, offset_size)?,
                length: read_uint(data, &mut cursor, length_size)?,
            });
        }
        if index
            .locations
            .insert(
                id,
                ItemLocation {
                    construction_method,
                    data_reference_index,
                    base_offset,
                    extents,
                },
            )
            .is_some()
        {
            return Err(format!("HEIF item {id} has duplicate iloc records").into());
        }
    }
    if cursor != data.len() {
        return Err("HEIF iloc has trailing bytes".into());
    }
    Ok(())
}

fn parse_iref(
    bytes: &[u8],
    view: BoxView,
    depth: u8,
    builder: &mut Builder<'_>,
    index: &mut HeifIndex,
) -> ImageResult<()> {
    let data = &bytes[view.data_start..view.end];
    let version = *data
        .first()
        .ok_or("HEIF iref full-box header is truncated")?;
    if version > 1 {
        return Err(format!("HEIF iref version {version} is unsupported").into());
    }
    let mut offset = view.data_start + 4;
    while offset < view.end {
        builder.checkpoint()?;
        if depth > 16 {
            return Err("HEIF box nesting exceeds the safety ceiling".into());
        }
        let child = parse_box(bytes, offset, view.end)?;
        let data = &bytes[child.data_start..child.end];
        builder.chunk(
            String::from_utf8_lossy(&child.kind).into_owned(),
            true,
            data,
            child.start,
            child.end,
        )?;
        let mut cursor = 0usize;
        builder.charge_records(1)?;
        let from = read_item_id(data, &mut cursor, version)?;
        let count = be_u16(data, cursor)?;
        cursor += 2;
        let mut to = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            builder.checkpoint()?;
            builder.charge_nodes(1)?;
            to.push(read_item_id(data, &mut cursor, version)?);
        }
        if cursor != data.len() {
            return Err("HEIF item-reference box has trailing bytes".into());
        }
        index.references.push(ItemReference {
            kind: child.kind,
            from,
            to,
        });
        offset = child.end;
    }
    Ok(())
}

fn parse_ipco(
    bytes: &[u8],
    view: BoxView,
    depth: u8,
    builder: &mut Builder<'_>,
    index: &mut HeifIndex,
) -> ImageResult<()> {
    let mut offset = view.data_start;
    while offset < view.end {
        builder.checkpoint()?;
        if depth > 16 {
            return Err("HEIF box nesting exceeds the safety ceiling".into());
        }
        let child = parse_box(bytes, offset, view.end)?;
        let data = &bytes[child.data_start..child.end];
        builder.chunk(
            String::from_utf8_lossy(&child.kind).into_owned(),
            is_known_box(&child.kind),
            data,
            child.start,
            child.end,
        )?;
        builder.charge_records(1)?;
        let property = match &child.kind {
            b"ispe" if data.len() >= 12 => ItemProperty::Dimensions(ImageDimensions {
                width: be_u32(data, 4)?,
                height: be_u32(data, 8)?,
            }),
            b"irot" if !data.is_empty() => ItemProperty::Rotation(i16::from(data[0] & 3) * 90),
            b"imir" if !data.is_empty() => ItemProperty::Mirror,
            b"colr" if data.len() >= 4 => {
                ItemProperty::Color(String::from_utf8_lossy(&data[..4]).into_owned())
            }
            _ => ItemProperty::Other,
        };
        index.properties.push(property);
        offset = child.end;
    }
    Ok(())
}

fn parse_ipma(data: &[u8], index: &mut HeifIndex, builder: &mut Builder<'_>) -> ImageResult<()> {
    if data.len() < 8 {
        return Err("HEIF ipma full-box header is truncated".into());
    }
    let version = data[0];
    if version > 1 {
        return Err(format!("HEIF ipma version {version} is unsupported").into());
    }
    let wide = data[3] & 1 != 0;
    let count = be_u32(data, 4)?;
    let mut cursor = 8usize;
    for _ in 0..count {
        builder.checkpoint()?;
        builder.charge_records(1)?;
        builder.charge_nodes(1)?;
        let item = read_item_id(data, &mut cursor, version)?;
        let association_count = *data
            .get(cursor)
            .ok_or("HEIF ipma association count is truncated")?;
        cursor += 1;
        let mut properties = Vec::with_capacity(usize::from(association_count));
        for _ in 0..association_count {
            builder.checkpoint()?;
            builder.charge_nodes(1)?;
            let association = if wide {
                let value = be_u16(data, cursor)?;
                cursor += 2;
                value & 0x7fff
            } else {
                let value = u16::from(*data.get(cursor).ok_or("HEIF ipma entry is truncated")?);
                cursor += 1;
                value & 0x7f
            };
            if association != 0 {
                properties.push(association);
            }
        }
        index
            .associations
            .entry(item)
            .or_default()
            .extend(properties);
    }
    if cursor != data.len() {
        return Err("HEIF ipma has trailing bytes".into());
    }
    Ok(())
}

fn validate_primary_data(
    bytes: &[u8],
    primary: u32,
    index: &HeifIndex,
    builder: &Builder<'_>,
) -> ImageResult<()> {
    let location = index
        .locations
        .get(&primary)
        .ok_or_else(|| format!("HEIF primary item {primary} has no iloc data association"))?;
    if location.extents.is_empty() {
        return Err(format!("HEIF primary item {primary} has no data extents").into());
    }
    for extent in &location.extents {
        builder.checkpoint()?;
        let (start, end) = extent_range(location, extent, bytes.len())?;
        if !range_inside_mdat(&index.mdat_ranges, start, end) {
            return Err(format!("HEIF primary item {primary} extent is not inside mdat").into());
        }
    }
    Ok(())
}

fn item_data<'a>(
    bytes: &'a [u8],
    item: u32,
    index: &HeifIndex,
    builder: &Builder<'_>,
) -> ImageResult<(&'a [u8], usize)> {
    let location = index
        .locations
        .get(&item)
        .ok_or_else(|| format!("HEIF associated metadata item {item} has no iloc record"))?;
    if location.extents.is_empty() {
        return Err(format!("HEIF associated metadata item {item} has no extents").into());
    }
    let mut ranges = Vec::with_capacity(location.extents.len());
    for extent in &location.extents {
        builder.checkpoint()?;
        let range = extent_range(location, extent, bytes.len())?;
        if !range_inside_mdat(&index.mdat_ranges, range.0, range.1) {
            return Err(format!("HEIF metadata item {item} extent is not inside mdat").into());
        }
        ranges.push(range);
    }
    for pair in ranges.windows(2) {
        if pair[0].1 != pair[1].0 {
            return Err(format!(
                "HEIF metadata item {item} uses non-contiguous extents that cannot retain one exact locator"
            )
            .into());
        }
    }
    let start = ranges[0].0;
    let end = ranges.last().unwrap().1;
    Ok((&bytes[start..end], start))
}

fn extent_range(
    location: &ItemLocation,
    extent: &Extent,
    input_length: usize,
) -> Result<(usize, usize), String> {
    if location.construction_method != 0 || location.data_reference_index != 0 {
        return Err("HEIF external/idat item construction is unsupported".into());
    }
    if extent.length == 0 {
        return Err("HEIF zero-length item extent is not a data association".into());
    }
    let start = location
        .base_offset
        .checked_add(extent.offset)
        .ok_or("HEIF item extent offset overflows")?;
    let end = start
        .checked_add(extent.length)
        .ok_or("HEIF item extent length overflows")?;
    let start = usize::try_from(start).map_err(|_| "HEIF item extent exceeds address space")?;
    let end = usize::try_from(end).map_err(|_| "HEIF item extent exceeds address space")?;
    if end > input_length {
        return Err("HEIF item extent is outside the input".into());
    }
    Ok((start, end))
}

fn normalize_mdat_ranges(ranges: &mut Vec<(usize, usize)>) {
    ranges.sort_unstable();
    let mut write = 0usize;
    for read in 0..ranges.len() {
        if write > 0 && ranges[read].0 <= ranges[write - 1].1 {
            ranges[write - 1].1 = ranges[write - 1].1.max(ranges[read].1);
        } else {
            ranges[write] = ranges[read];
            write += 1;
        }
    }
    ranges.truncate(write);
}

fn range_inside_mdat(ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
    let candidate = ranges.partition_point(|(_, range_end)| *range_end <= start);
    ranges
        .get(candidate)
        .is_some_and(|(range_start, range_end)| start >= *range_start && end <= *range_end)
}

fn is_metadata_item(info: &ItemInfo) -> bool {
    info.item_type == *b"Exif"
        || (info.item_type == *b"mime"
            && info.content_type.as_deref().is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "application/rdf+xml" | "application/xml" | "text/xml"
                )
            }))
}

fn is_image_item(kind: [u8; 4]) -> bool {
    matches!(
        &kind,
        b"hvc1" | b"hev1" | b"av01" | b"grid" | b"iden" | b"jpeg"
    )
}

fn read_item_id(bytes: &[u8], cursor: &mut usize, version: u8) -> Result<u32, String> {
    if version == 0 {
        let value = u32::from(be_u16(bytes, *cursor)?);
        *cursor += 2;
        Ok(value)
    } else {
        let value = be_u32(bytes, *cursor)?;
        *cursor += 4;
        Ok(value)
    }
}

fn read_uint(bytes: &[u8], cursor: &mut usize, width: u8) -> Result<u64, String> {
    let width = usize::from(width);
    let slice = bytes
        .get(*cursor..cursor.saturating_add(width))
        .ok_or_else(|| "HEIF variable-width integer is truncated".to_string())?;
    *cursor += width;
    Ok(slice
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte)))
}

fn c_string(bytes: &[u8]) -> Result<(String, usize), String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| "HEIF infe string is not terminated".to_string())?;
    Ok((String::from_utf8_lossy(&bytes[..end]).into_owned(), end + 1))
}

fn is_plain_container(kind: &[u8; 4]) -> bool {
    matches!(
        kind,
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"dinf" | b"iprp"
    )
}

fn is_known_box(kind: &[u8; 4]) -> bool {
    matches!(
        kind,
        b"ftyp"
            | b"meta"
            | b"moov"
            | b"trak"
            | b"mdia"
            | b"minf"
            | b"stbl"
            | b"dinf"
            | b"iprp"
            | b"ipco"
            | b"ipma"
            | b"iinf"
            | b"iloc"
            | b"pitm"
            | b"iref"
            | b"ispe"
            | b"irot"
            | b"imir"
            | b"colr"
            | b"mdat"
            | b"hdlr"
            | b"infe"
    )
}
