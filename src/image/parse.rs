use super::metadata::{parse_iptc, parse_tiff};
use super::model::*;
use super::svg::{frame_locator, parse_svg};
use crate::core::{
    BudgetExceeded, LocationComponent, OperationControl, OperationControlError, SourceLocator,
    sha256_hex,
};
use std::collections::BTreeMap;

mod bmp;
mod gif;
mod heif;
mod jpeg;
mod png;
mod tiff;
mod webp;

#[derive(Debug)]
pub(crate) enum ImageParseError {
    Malformed(String),
    Control(OperationControlError),
}

impl std::fmt::Display for ImageParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(message) => formatter.write_str(message),
            Self::Control(error) => error.fmt(formatter),
        }
    }
}

impl From<String> for ImageParseError {
    fn from(value: String) -> Self {
        Self::Malformed(value)
    }
}

impl From<&str> for ImageParseError {
    fn from(value: &str) -> Self {
        Self::Malformed(value.into())
    }
}

impl From<OperationControlError> for ImageParseError {
    fn from(value: OperationControlError) -> Self {
        Self::Control(value)
    }
}

impl From<BudgetExceeded> for ImageParseError {
    fn from(value: BudgetExceeded) -> Self {
        Self::Control(OperationControlError::BudgetExceeded(value))
    }
}

pub(super) type ImageResult<T> = Result<T, ImageParseError>;

pub(crate) fn parse_document(bytes: &[u8], options: &ImageOptions) -> ImageResult<ImageDocument> {
    parse_document_controlled(bytes, options, None)
}

pub(crate) fn parse_document_controlled<'a>(
    bytes: &[u8],
    options: &'a ImageOptions,
    control: Option<&'a OperationControl>,
) -> ImageResult<ImageDocument> {
    if options.max_frames == 0
        || options.max_dimension == 0
        || options.max_metadata_bytes == 0
        || options.max_unknown_chunk_bytes == 0
        || options.max_chunks == 0
        || options.max_svg_elements == 0
        || options.max_svg_depth == 0
        || options.max_svg_path_bytes == 0
    {
        return Err("image limits must all be greater than zero".into());
    }
    let format =
        sniff_format(bytes).ok_or_else(|| "input has no supported image signature".to_string())?;
    let mut builder = Builder::new(format, options, control);
    builder.checkpoint()?;
    match format {
        ImageFormat::Png => png::parse(bytes, &mut builder)?,
        ImageFormat::Jpeg => jpeg::parse(bytes, &mut builder)?,
        ImageFormat::Tiff => tiff::parse(bytes, &mut builder)?,
        ImageFormat::Webp => webp::parse(bytes, &mut builder)?,
        ImageFormat::Gif => gif::parse(bytes, &mut builder)?,
        ImageFormat::Bmp => bmp::parse(bytes, &mut builder)?,
        ImageFormat::Heif => heif::parse(bytes, &mut builder)?,
        ImageFormat::Svg => parse_svg_image(bytes, &mut builder)?,
    }
    builder.finish()
}

pub(crate) fn sniff_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some(ImageFormat::Tiff)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else if heif::matches(bytes) {
        Some(ImageFormat::Heif)
    } else if looks_like_svg(bytes) {
        Some(ImageFormat::Svg)
    } else {
        None
    }
}

pub(super) struct Builder<'a> {
    pub format: ImageFormat,
    pub options: &'a ImageOptions,
    pub dimensions: ImageDimensions,
    pub frames: Vec<ImageFrame>,
    pub color: ImageColorInfo,
    pub orientation: ImageOrientation,
    pub metadata: ImageMetadata,
    metadata_bytes: u64,
    pub embedded_text: Vec<ImageText>,
    pub chunks: Vec<ImageChunk>,
    pub vector: Option<SvgContent>,
    pub links: Vec<ImageLink>,
    pub active_content: Vec<ImageActiveContent>,
    retained_unknown_bytes: u64,
    control: Option<&'a OperationControl>,
}

impl<'a> Builder<'a> {
    fn new(
        format: ImageFormat,
        options: &'a ImageOptions,
        control: Option<&'a OperationControl>,
    ) -> Self {
        Self {
            format,
            options,
            dimensions: ImageDimensions::default(),
            frames: Vec::new(),
            color: ImageColorInfo::default(),
            orientation: ImageOrientation::default(),
            metadata: ImageMetadata::default(),
            metadata_bytes: 0,
            embedded_text: Vec::new(),
            chunks: Vec::new(),
            vector: None,
            links: Vec::new(),
            active_content: Vec::new(),
            retained_unknown_bytes: 0,
            control,
        }
    }

    pub fn checkpoint(&self) -> ImageResult<()> {
        self.control
            .map_or(Ok(()), |control| control.checkpoint().map_err(Into::into))
    }

    pub fn charge_nodes(&self, count: u64) -> ImageResult<()> {
        self.control.map_or(Ok(()), |control| {
            control.budget().consume_nodes(count).map_err(Into::into)
        })
    }

    pub fn charge_records(&self, count: u64) -> ImageResult<()> {
        self.control.map_or(Ok(()), |control| {
            control.budget().consume_records(count).map_err(Into::into)
        })
    }

    pub fn charge_decoded_characters(&self, count: u64) -> ImageResult<()> {
        self.control.map_or(Ok(()), |control| {
            control
                .budget()
                .consume_decoded_characters(count)
                .map_err(Into::into)
        })
    }

    pub fn chunk(
        &mut self,
        kind: impl Into<String>,
        known: bool,
        bytes: &[u8],
        start: usize,
        end: usize,
    ) -> ImageResult<()> {
        self.checkpoint()?;
        if self.chunks.len() as u64 >= self.options.max_chunks {
            return Err("image chunk count exceeds ImageOptions::max_chunks".into());
        }
        let retain_raw = !known && self.options.retain_unknown_chunk_bytes;
        if retain_raw {
            self.retained_unknown_bytes = self
                .retained_unknown_bytes
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| "retained unknown image bytes overflow".to_string())?;
            if self.retained_unknown_bytes > self.options.max_unknown_chunk_bytes {
                return Err(
                    "unknown image chunks exceed ImageOptions::max_unknown_chunk_bytes".into(),
                );
            }
        }
        self.charge_nodes(1)?;
        self.chunks.push(ImageChunk {
            kind: kind.into(),
            known,
            byte_length: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            raw_bytes: retain_raw.then(|| bytes.to_vec()),
            locator: byte_locator(start, end),
        });
        Ok(())
    }

    pub fn metadata(
        &mut self,
        kind: ImageMetadataKind,
        bytes: &[u8],
        start: usize,
        text: Option<String>,
        fields: BTreeMap<String, String>,
    ) -> ImageResult<()> {
        self.metadata_with_charge(kind, bytes, start, text, fields, false)
    }

    pub fn metadata_precharged(
        &mut self,
        kind: ImageMetadataKind,
        bytes: &[u8],
        start: usize,
        text: Option<String>,
        fields: BTreeMap<String, String>,
    ) -> ImageResult<()> {
        self.metadata_with_charge(kind, bytes, start, text, fields, true)
    }

    fn metadata_with_charge(
        &mut self,
        kind: ImageMetadataKind,
        bytes: &[u8],
        start: usize,
        text: Option<String>,
        fields: BTreeMap<String, String>,
        decoded_precharged: bool,
    ) -> ImageResult<()> {
        self.reserve_metadata(bytes.len())?;
        self.metadata_reserved(kind, bytes, start, text, fields, true, decoded_precharged)
    }

    fn reserve_metadata(&mut self, length: usize) -> ImageResult<()> {
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(length as u64)
            .ok_or_else(|| "image metadata byte count overflows".to_string())?;
        if self.metadata_bytes > self.options.max_metadata_bytes {
            return Err("image metadata exceeds ImageOptions::max_metadata_bytes".into());
        }
        Ok(())
    }

    fn metadata_reserved(
        &mut self,
        kind: ImageMetadataKind,
        bytes: &[u8],
        start: usize,
        text: Option<String>,
        fields: BTreeMap<String, String>,
        retain_raw: bool,
        decoded_precharged: bool,
    ) -> ImageResult<()> {
        self.checkpoint()?;
        if let Some(value) = text.as_ref().filter(|value| !value.is_empty()) {
            if !decoded_precharged {
                self.charge_decoded_characters(value.chars().count() as u64)?;
            }
            self.charge_nodes(1)?;
            self.embedded_text.push(ImageText {
                kind: if kind == ImageMetadataKind::Comment {
                    ImageTextKind::Comment
                } else {
                    ImageTextKind::Metadata
                },
                text: value.clone(),
                locator: byte_locator(start, start.saturating_add(bytes.len())),
            });
        }
        self.charge_nodes(1)?;
        self.metadata.blocks.push(ImageMetadataBlock {
            kind,
            sha256: sha256_hex(bytes),
            byte_length: bytes.len() as u64,
            text,
            fields,
            raw_bytes: (retain_raw && self.options.retain_metadata_bytes).then(|| bytes.to_vec()),
            locator: byte_locator(start, start.saturating_add(bytes.len())),
        });
        Ok(())
    }

    pub fn exif(&mut self, bytes: &[u8], start: usize) -> ImageResult<()> {
        self.reserve_metadata(bytes.len())?;
        let tiff = parse_tiff(bytes, self.options.max_metadata_bytes)?;
        self.metadata_reserved(
            ImageMetadataKind::Exif,
            bytes,
            start,
            None,
            BTreeMap::new(),
            true,
            false,
        )?;
        merge_camera(&mut self.metadata.camera, tiff.camera);
        if let Some(value) = tiff.orientation {
            self.orientation = ImageOrientation::from_exif(value);
        }
        for span in tiff.xmp {
            self.xmp_embedded(span.slice(bytes)?, start.saturating_add(span.offset))?;
        }
        for span in tiff.iptc {
            self.iptc_embedded(span.slice(bytes)?, start.saturating_add(span.offset))?;
        }
        Ok(())
    }

    pub fn xmp(&mut self, bytes: &[u8], start: usize) -> ImageResult<()> {
        self.xmp_with_charge(bytes, start, false)
    }

    pub fn xmp_precharged(&mut self, bytes: &[u8], start: usize) -> ImageResult<()> {
        self.xmp_with_charge(bytes, start, true)
    }

    fn xmp_with_charge(
        &mut self,
        bytes: &[u8],
        start: usize,
        decoded_precharged: bool,
    ) -> ImageResult<()> {
        self.reserve_metadata(bytes.len())?;
        let text = String::from_utf8_lossy(bytes)
            .trim_matches(char::from(0))
            .to_string();
        self.metadata_reserved(
            ImageMetadataKind::Xmp,
            bytes,
            start,
            Some(text),
            BTreeMap::new(),
            true,
            decoded_precharged,
        )
    }

    pub fn iptc(&mut self, bytes: &[u8], start: usize) -> ImageResult<()> {
        self.iptc_with_charge(bytes, start, false)
    }

    pub fn iptc_precharged(&mut self, bytes: &[u8], start: usize) -> ImageResult<()> {
        self.iptc_with_charge(bytes, start, true)
    }

    fn iptc_with_charge(
        &mut self,
        bytes: &[u8],
        start: usize,
        decoded_precharged: bool,
    ) -> ImageResult<()> {
        self.reserve_metadata(bytes.len())?;
        let fields = parse_iptc(bytes);
        let text = fields
            .get("caption")
            .or_else(|| fields.get("headline"))
            .cloned();
        self.metadata_reserved(
            ImageMetadataKind::Iptc,
            bytes,
            start,
            text,
            fields,
            true,
            decoded_precharged,
        )
    }

    fn xmp_embedded(&mut self, bytes: &[u8], start: usize) -> ImageResult<()> {
        let text = String::from_utf8_lossy(bytes)
            .trim_matches(char::from(0))
            .to_string();
        self.metadata_reserved(
            ImageMetadataKind::Xmp,
            bytes,
            start,
            Some(text),
            BTreeMap::new(),
            false,
            false,
        )
    }

    fn iptc_embedded(&mut self, bytes: &[u8], start: usize) -> ImageResult<()> {
        let fields = parse_iptc(bytes);
        let text = fields
            .get("caption")
            .or_else(|| fields.get("headline"))
            .cloned();
        self.metadata_reserved(
            ImageMetadataKind::Iptc,
            bytes,
            start,
            text,
            fields,
            false,
            false,
        )
    }

    pub fn frame(
        &mut self,
        dimensions: ImageDimensions,
        duration_ms: Option<u64>,
    ) -> ImageResult<()> {
        self.checkpoint()?;
        if self.frames.len() as u64 >= self.options.max_frames {
            return Err("image frame/page count exceeds ImageOptions::max_frames".into());
        }
        self.charge_records(1)?;
        self.charge_nodes(1)?;
        let index = self.frames.len() as u64;
        self.frames.push(ImageFrame {
            index,
            dimensions,
            x: 0,
            y: 0,
            duration_ms,
            disposal: None,
            blend: None,
            locator: frame_locator(index, dimensions),
        });
        Ok(())
    }

    fn finish(mut self) -> ImageResult<ImageDocument> {
        self.checkpoint()?;
        if self.dimensions.width == 0 || self.dimensions.height == 0 {
            return Err(format!(
                "{} image dimensions are missing or zero",
                self.format.as_str()
            )
            .into());
        }
        if self.dimensions.width > self.options.max_dimension
            || self.dimensions.height > self.options.max_dimension
        {
            return Err("image dimensions exceed ImageOptions::max_dimension".into());
        }
        if self.frames.is_empty() {
            self.frame(self.dimensions, None)?;
        }
        if self.frames.iter().any(|frame| {
            frame.dimensions.width > self.options.max_dimension
                || frame.dimensions.height > self.options.max_dimension
        }) {
            return Err("image frame dimensions exceed ImageOptions::max_dimension".into());
        }
        Ok(ImageDocument {
            schema_version: crate::core::SchemaVersion::IMAGE_V1.into(),
            format: self.format,
            dimensions: self.dimensions,
            frames: self.frames,
            color: self.color,
            orientation: self.orientation,
            metadata: self.metadata,
            embedded_text: self.embedded_text,
            chunks: self.chunks,
            vector: self.vector,
            links: self.links,
            active_content: self.active_content,
            complete: true,
        })
    }
}

fn parse_svg_image(bytes: &[u8], builder: &mut Builder<'_>) -> ImageResult<()> {
    let parsed = parse_svg(bytes, builder.options, builder.control)?;
    builder.dimensions = parsed.dimensions;
    builder.vector = Some(parsed.vector);
    builder.embedded_text = parsed.text;
    builder.links = parsed.links;
    builder.active_content = parsed.active;
    builder.color.model = Some("vector".into());
    builder.color.alpha = Some(true);
    builder.chunk("svg_document", true, bytes, 0, bytes.len())?;
    Ok(())
}

fn merge_camera(target: &mut ImageCameraMetadata, source: ImageCameraMetadata) {
    target.make = target.make.take().or(source.make);
    target.model = target.model.take().or(source.model);
    target.lens_model = target.lens_model.take().or(source.lens_model);
    target.captured_at = target.captured_at.take().or(source.captured_at);
    target.exposure_time = target.exposure_time.take().or(source.exposure_time);
    target.f_number = target.f_number.take().or(source.f_number);
    target.iso_speed = target.iso_speed.take().or(source.iso_speed);
    target.focal_length = target.focal_length.take().or(source.focal_length);
    target.gps_latitude = target.gps_latitude.take().or(source.gps_latitude);
    target.gps_longitude = target.gps_longitude.take().or(source.gps_longitude);
    target.tags.extend(source.tags);
}

pub(super) fn byte_locator(start: usize, end: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ByteRange {
        byte_start: start,
        byte_end: end,
    })
    .expect("checked image byte range")
}
pub(super) fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (!needle.is_empty())
        .then(|| {
            haystack
                .windows(needle.len())
                .position(|window| window == needle)
        })
        .flatten()
}
fn looks_like_svg(bytes: &[u8]) -> bool {
    crate::detect::has_svg_root(bytes)
}
pub(super) fn be_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_be_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| "big-endian u16 is out of bounds".to_string())?
            .try_into()
            .unwrap(),
    ))
}
pub(super) fn be_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| "big-endian u32 is out of bounds".to_string())?
            .try_into()
            .unwrap(),
    ))
}
pub(super) fn be_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    Ok(u64::from_be_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or_else(|| "big-endian u64 is out of bounds".to_string())?
            .try_into()
            .unwrap(),
    ))
}
pub(super) fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| "little-endian u16 is out of bounds".to_string())?
            .try_into()
            .unwrap(),
    ))
}
pub(super) fn le_u24(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let raw = bytes
        .get(offset..offset + 3)
        .ok_or_else(|| "little-endian u24 is out of bounds".to_string())?;
    Ok(u32::from(raw[0]) | (u32::from(raw[1]) << 8) | (u32::from(raw[2]) << 16))
}
pub(super) fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| "little-endian u32 is out of bounds".to_string())?
            .try_into()
            .unwrap(),
    ))
}
pub(super) fn le_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    Ok(i32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| "little-endian i32 is out of bounds".to_string())?
            .try_into()
            .unwrap(),
    ))
}
