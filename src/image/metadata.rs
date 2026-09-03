use super::model::{ImageCameraMetadata, ImageDimensions};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy)]
enum Endian {
    Little,
    Big,
}

impl Endian {
    fn u16(self, bytes: &[u8], offset: usize) -> Option<u16> {
        let raw: [u8; 2] = bytes.get(offset..offset + 2)?.try_into().ok()?;
        Some(match self {
            Self::Little => u16::from_le_bytes(raw),
            Self::Big => u16::from_be_bytes(raw),
        })
    }

    fn u32(self, bytes: &[u8], offset: usize) -> Option<u32> {
        let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
        Some(match self {
            Self::Little => u32::from_le_bytes(raw),
            Self::Big => u32::from_be_bytes(raw),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TiffData {
    pub pages: Vec<TiffPage>,
    pub camera: ImageCameraMetadata,
    pub orientation: Option<u16>,
    pub ifd_spans: Vec<ByteSpan>,
    pub xmp: Vec<ByteSpan>,
    pub iptc: Vec<ByteSpan>,
    pub unknown_tags: Vec<(u16, ByteSpan)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ByteSpan {
    pub offset: usize,
    pub length: usize,
}

impl ByteSpan {
    pub(crate) fn slice<'a>(self, bytes: &'a [u8]) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(self.length)
            .ok_or_else(|| "TIFF metadata range overflows".to_string())?;
        bytes
            .get(self.offset..end)
            .ok_or_else(|| "TIFF metadata range is out of bounds".to_string())
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TiffPage {
    pub dimensions: ImageDimensions,
    pub bits_per_component: Option<u8>,
    pub samples_per_pixel: Option<u8>,
    pub photometric: Option<u16>,
}

#[derive(Debug, Clone)]
struct Entry {
    tag: u16,
    field_type: u16,
    data_offset: usize,
    data_length: usize,
}

pub(crate) fn parse_tiff(bytes: &[u8], max_metadata_bytes: u64) -> Result<TiffData, String> {
    if bytes.len() < 8 {
        return Err("TIFF header is truncated".into());
    }
    let endian = match &bytes[..2] {
        b"II" => Endian::Little,
        b"MM" => Endian::Big,
        _ => return Err("TIFF byte-order marker is invalid".into()),
    };
    if endian.u16(bytes, 2) != Some(42) {
        return Err(
            "BigTIFF and invalid TIFF magic are not accepted by the classic TIFF parser".into(),
        );
    }
    let mut next = endian
        .u32(bytes, 4)
        .ok_or_else(|| "TIFF first IFD offset is truncated".to_string())?
        as usize;
    let mut visited = HashSet::new();
    let mut result = TiffData::default();
    let mut metadata_bytes = 0u64;
    while next != 0 {
        if !visited.insert(next) {
            return Err("TIFF IFD chain contains a cycle".into());
        }
        if result.pages.len() >= 10_000 {
            return Err("TIFF IFD count exceeds the structural safety ceiling".into());
        }
        let (entries, following, span) = parse_ifd(bytes, endian, next)?;
        reserve_metadata(&mut metadata_bytes, span.length, max_metadata_bytes)?;
        result.ifd_spans.push(span);
        let mut page = TiffPage::default();
        apply_entries(
            bytes,
            endian,
            &entries,
            &mut page,
            &mut result,
            true,
            &mut metadata_bytes,
            max_metadata_bytes,
        )?;
        result.pages.push(page);
        next = following;
    }
    if result.pages.is_empty() {
        return Err("TIFF contains no image file directory".into());
    }
    Ok(result)
}

fn parse_ifd(
    bytes: &[u8],
    endian: Endian,
    offset: usize,
) -> Result<(Vec<Entry>, usize, ByteSpan), String> {
    let count = endian
        .u16(bytes, offset)
        .ok_or_else(|| "TIFF IFD entry count is out of bounds".to_string())?
        as usize;
    if count > 65_535 {
        return Err("TIFF IFD entry count exceeds the structural safety ceiling".into());
    }
    let table_end = offset
        .checked_add(2)
        .and_then(|value| value.checked_add(count.saturating_mul(12)))
        .ok_or_else(|| "TIFF IFD size overflow".to_string())?;
    if table_end.checked_add(4).is_none_or(|end| end > bytes.len()) {
        return Err("TIFF IFD table is truncated".into());
    }
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let at = offset + 2 + index * 12;
        let tag = endian.u16(bytes, at).unwrap();
        let field_type = endian.u16(bytes, at + 2).unwrap();
        let item_count = endian.u32(bytes, at + 4).unwrap();
        let unit = type_size(field_type)
            .ok_or_else(|| format!("TIFF tag {tag} uses invalid field type {field_type}"))?;
        let size = (item_count as usize)
            .checked_mul(unit)
            .ok_or_else(|| format!("TIFF tag {tag} byte count overflows"))?;
        let value_offset = at + 8;
        let data_offset = if size <= 4 {
            value_offset
        } else {
            endian.u32(bytes, value_offset).unwrap() as usize
        };
        let end = data_offset
            .checked_add(size)
            .ok_or_else(|| format!("TIFF tag {tag} data range overflows"))?;
        if end > bytes.len() {
            return Err(format!("TIFF tag {tag} data is out of bounds"));
        }
        entries.push(Entry {
            tag,
            field_type,
            data_offset,
            data_length: size,
        });
    }
    let following = endian.u32(bytes, table_end).unwrap() as usize;
    Ok((
        entries,
        following,
        ByteSpan {
            offset,
            length: table_end + 4 - offset,
        },
    ))
}

fn apply_entries(
    bytes: &[u8],
    endian: Endian,
    entries: &[Entry],
    page: &mut TiffPage,
    result: &mut TiffData,
    allow_nested: bool,
    metadata_bytes: &mut u64,
    max_metadata_bytes: u64,
) -> Result<(), String> {
    for entry in entries {
        let data = entry_data(bytes, entry)?;
        let first = entry_u32(data, entry.field_type, endian);
        match entry.tag {
            256 => page.dimensions.width = first.unwrap_or(0),
            257 => page.dimensions.height = first.unwrap_or(0),
            258 => page.bits_per_component = first.and_then(|value| u8::try_from(value).ok()),
            262 => page.photometric = first.and_then(|value| u16::try_from(value).ok()),
            274 => {
                let orientation = first.and_then(|value| u16::try_from(value).ok());
                result.orientation = result.orientation.or(orientation);
            }
            277 => page.samples_per_pixel = first.and_then(|value| u8::try_from(value).ok()),
            271 => {
                reserve_metadata(metadata_bytes, entry.data_length, max_metadata_bytes)?;
                set_ascii(
                    &mut result.camera.make,
                    data,
                    "make",
                    &mut result.camera.tags,
                )
            }
            272 => {
                reserve_metadata(metadata_bytes, entry.data_length, max_metadata_bytes)?;
                set_ascii(
                    &mut result.camera.model,
                    data,
                    "model",
                    &mut result.camera.tags,
                )
            }
            306 | 36867 => {
                reserve_metadata(metadata_bytes, entry.data_length, max_metadata_bytes)?;
                set_ascii(
                    &mut result.camera.captured_at,
                    data,
                    "captured_at",
                    &mut result.camera.tags,
                )
            }
            42036 => {
                reserve_metadata(metadata_bytes, entry.data_length, max_metadata_bytes)?;
                set_ascii(
                    &mut result.camera.lens_model,
                    data,
                    "lens_model",
                    &mut result.camera.tags,
                )
            }
            33434 => set_rational(
                &mut result.camera.exposure_time,
                data,
                endian,
                "exposure_time",
                &mut result.camera.tags,
            ),
            33437 => set_rational(
                &mut result.camera.f_number,
                data,
                endian,
                "f_number",
                &mut result.camera.tags,
            ),
            34855 => set_number(
                &mut result.camera.iso_speed,
                data,
                entry.field_type,
                endian,
                "iso_speed",
                &mut result.camera.tags,
            ),
            37386 => set_rational(
                &mut result.camera.focal_length,
                data,
                endian,
                "focal_length",
                &mut result.camera.tags,
            ),
            700 => {
                reserve_metadata(metadata_bytes, entry.data_length, max_metadata_bytes)?;
                result.xmp.push(entry_span(entry));
            }
            33723 => {
                reserve_metadata(metadata_bytes, entry.data_length, max_metadata_bytes)?;
                result.iptc.push(entry_span(entry));
            }
            34665 if allow_nested => {
                let nested_offset = first.unwrap_or(0) as usize;
                if nested_offset != 0 {
                    let (nested, _, span) = parse_ifd(bytes, endian, nested_offset)?;
                    reserve_metadata(metadata_bytes, span.length, max_metadata_bytes)?;
                    result.ifd_spans.push(span);
                    let mut nested_page = TiffPage::default();
                    apply_entries(
                        bytes,
                        endian,
                        &nested,
                        &mut nested_page,
                        result,
                        false,
                        metadata_bytes,
                        max_metadata_bytes,
                    )?;
                }
            }
            34853 if allow_nested => {
                let nested_offset = first.unwrap_or(0) as usize;
                if nested_offset != 0 {
                    let (nested, _, span) = parse_ifd(bytes, endian, nested_offset)?;
                    reserve_metadata(metadata_bytes, span.length, max_metadata_bytes)?;
                    result.ifd_spans.push(span);
                    apply_gps_entries(bytes, endian, &nested, &mut result.camera)?;
                }
            }
            _ if !is_baseline_tag(entry.tag) => {
                result.unknown_tags.push((entry.tag, entry_span(entry)));
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_baseline_tag(tag: u16) -> bool {
    matches!(
        tag,
        254 | 255
            | 256
            | 257
            | 258
            | 259
            | 262
            | 263
            | 266
            | 269
            | 270
            | 271
            | 272
            | 273
            | 274
            | 277
            | 278
            | 279
            | 282
            | 283
            | 284
            | 296
            | 305
            | 306
            | 315
            | 318
            | 319
            | 320
            | 322
            | 323
            | 324
            | 325
            | 338
            | 339
    )
}

fn entry_u32(data: &[u8], field_type: u16, endian: Endian) -> Option<u32> {
    match field_type {
        1 | 6 | 7 => data.first().copied().map(u32::from),
        3 | 8 => endian.u16(data, 0).map(u32::from),
        4 | 9 | 13 => endian.u32(data, 0),
        _ => None,
    }
}

fn set_ascii(
    target: &mut Option<String>,
    data: &[u8],
    name: &str,
    tags: &mut BTreeMap<String, String>,
) {
    let value = String::from_utf8_lossy(data)
        .trim_matches(char::from(0))
        .trim()
        .to_string();
    if !value.is_empty() {
        tags.insert(name.into(), value.clone());
        *target = Some(value);
    }
}

fn set_number(
    target: &mut Option<String>,
    data: &[u8],
    field_type: u16,
    endian: Endian,
    name: &str,
    tags: &mut BTreeMap<String, String>,
) {
    if let Some(value) = entry_u32(data, field_type, endian) {
        let value = value.to_string();
        tags.insert(name.into(), value.clone());
        *target = Some(value);
    }
}

fn set_rational(
    target: &mut Option<String>,
    data: &[u8],
    endian: Endian,
    name: &str,
    tags: &mut BTreeMap<String, String>,
) {
    if data.len() >= 8 {
        let numerator = endian.u32(data, 0).unwrap();
        let denominator = endian.u32(data, 4).unwrap();
        let value = format!("{numerator}/{denominator}");
        tags.insert(name.into(), value.clone());
        *target = Some(value);
    }
}

fn entry_span(entry: &Entry) -> ByteSpan {
    ByteSpan {
        offset: entry.data_offset,
        length: entry.data_length,
    }
}

fn entry_data<'a>(bytes: &'a [u8], entry: &Entry) -> Result<&'a [u8], String> {
    entry_span(entry).slice(bytes)
}

fn reserve_metadata(total: &mut u64, length: usize, maximum: u64) -> Result<(), String> {
    *total = total
        .checked_add(length as u64)
        .ok_or_else(|| "TIFF metadata byte count overflows".to_string())?;
    if *total > maximum {
        return Err("TIFF metadata exceeds ImageOptions::max_metadata_bytes".into());
    }
    Ok(())
}

fn apply_gps_entries(
    bytes: &[u8],
    endian: Endian,
    entries: &[Entry],
    camera: &mut ImageCameraMetadata,
) -> Result<(), String> {
    let mut latitude_ref = None;
    let mut latitude = None;
    let mut longitude_ref = None;
    let mut longitude = None;
    for entry in entries {
        let data = entry_data(bytes, entry)?;
        match entry.tag {
            1 => latitude_ref = data.first().copied().map(char::from),
            2 => latitude = gps_coordinate(data, endian),
            3 => longitude_ref = data.first().copied().map(char::from),
            4 => longitude = gps_coordinate(data, endian),
            _ => {}
        }
    }
    camera.gps_latitude = signed_coordinate(latitude, latitude_ref, 'S');
    camera.gps_longitude = signed_coordinate(longitude, longitude_ref, 'W');
    if let Some(value) = &camera.gps_latitude {
        camera.tags.insert("gps_latitude".into(), value.clone());
    }
    if let Some(value) = &camera.gps_longitude {
        camera.tags.insert("gps_longitude".into(), value.clone());
    }
    Ok(())
}

fn gps_coordinate(data: &[u8], endian: Endian) -> Option<f64> {
    if data.len() < 24 {
        return None;
    }
    let rational = |offset| {
        let numerator = endian.u32(data, offset)? as f64;
        let denominator = endian.u32(data, offset + 4)?;
        (denominator != 0).then_some(numerator / f64::from(denominator))
    };
    Some(rational(0)? + rational(8)? / 60.0 + rational(16)? / 3600.0)
}

fn signed_coordinate(
    value: Option<f64>,
    reference: Option<char>,
    negative: char,
) -> Option<String> {
    let mut value = value?;
    if reference?.to_ascii_uppercase() == negative {
        value = -value;
    }
    Some(format!("{value:.8}"))
}

fn type_size(field_type: u16) -> Option<usize> {
    Some(match field_type {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => return None,
    })
}

pub(crate) fn parse_iptc(bytes: &[u8]) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let mut offset = 0usize;
    while offset + 5 <= bytes.len() {
        if bytes[offset] != 0x1c {
            offset += 1;
            continue;
        }
        let record = bytes[offset + 1];
        let dataset = bytes[offset + 2];
        let length = u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]) as usize;
        let start = offset + 5;
        let Some(end) = start.checked_add(length).filter(|end| *end <= bytes.len()) else {
            break;
        };
        let name = match (record, dataset) {
            (2, 5) => "object_name".into(),
            (2, 25) => "keywords".into(),
            (2, 80) => "byline".into(),
            (2, 90) => "city".into(),
            (2, 95) => "province_state".into(),
            (2, 101) => "country".into(),
            (2, 105) => "headline".into(),
            (2, 116) => "copyright".into(),
            (2, 120) => "caption".into(),
            _ => format!("{record}:{dataset}"),
        };
        let value = String::from_utf8_lossy(&bytes[start..end]).to_string();
        fields
            .entry(name)
            .and_modify(|existing: &mut String| {
                existing.push_str("; ");
                existing.push_str(&value);
            })
            .or_insert(value);
        offset = end;
    }
    fields
}
