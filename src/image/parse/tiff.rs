use super::{Builder, ImageResult};
use crate::image::ImageMetadataKind;
use crate::image::metadata::parse_tiff;
use std::collections::BTreeMap;

pub(super) fn parse(bytes: &[u8], builder: &mut Builder<'_>) -> ImageResult<()> {
    let parsed = parse_tiff(bytes, builder.options.max_metadata_bytes)?;
    let mut camera_fields = parsed.camera.tags.clone();
    if let Some(make) = &parsed.camera.make {
        camera_fields.insert("make".into(), make.clone());
    }
    if let Some(model) = &parsed.camera.model {
        camera_fields.insert("model".into(), model.clone());
    }
    for (index, span) in parsed.ifd_spans.iter().copied().enumerate() {
        builder.metadata(
            ImageMetadataKind::Exif,
            span.slice(bytes)?,
            span.offset,
            None,
            if index == 0 {
                camera_fields.clone()
            } else {
                BTreeMap::new()
            },
        )?;
    }
    builder.metadata.camera = parsed.camera;
    if let Some(value) = parsed.orientation {
        builder.orientation = crate::image::ImageOrientation::from_exif(value);
    }
    for page in parsed.pages {
        if builder.dimensions.width == 0 {
            builder.dimensions = page.dimensions;
        }
        builder.color.bits_per_component =
            builder.color.bits_per_component.or(page.bits_per_component);
        builder.color.component_count = builder.color.component_count.or(page.samples_per_pixel);
        builder.color.model = builder
            .color
            .model
            .clone()
            .or_else(|| page.photometric.map(color).map(str::to_string));
        builder.frame(page.dimensions, None)?;
    }
    for span in parsed.xmp {
        builder.xmp(span.slice(bytes)?, span.offset)?;
    }
    for span in parsed.iptc {
        builder.iptc(span.slice(bytes)?, span.offset)?;
    }
    for (tag, span) in parsed.unknown_tags {
        builder.chunk(
            format!("tiff-tag-{tag}"),
            false,
            span.slice(bytes)?,
            span.offset,
            span.offset.saturating_add(span.length),
        )?;
    }
    Ok(())
}

fn color(value: u16) -> &'static str {
    match value {
        0 | 1 => "grayscale",
        2 => "rgb",
        3 => "indexed",
        5 => "cmyk",
        6 => "ycbcr",
        8 => "cielab",
        _ => "unknown",
    }
}
