use super::model::*;
use crate::core::{
    BoundingBox, CoordinateOrigin, CoordinateUnit, IndexPosition, LocationComponent,
    OperationControl, SourceLocator, sha256_hex,
};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::BTreeSet;

pub(crate) struct SvgParsed {
    pub dimensions: ImageDimensions,
    pub vector: SvgContent,
    pub text: Vec<ImageText>,
    pub links: Vec<ImageLink>,
    pub active: Vec<ImageActiveContent>,
}

#[derive(Debug)]
struct SvgStackEntry {
    name: String,
    path: String,
    next_child: u64,
}

pub(crate) fn parse_svg(
    bytes: &[u8],
    options: &ImageOptions,
    control: Option<&OperationControl>,
) -> Result<SvgParsed, String> {
    let source = std::str::from_utf8(bytes).map_err(|_| "SVG source is not valid UTF-8")?;
    let mut reader = Reader::from_str(source);
    reader.config_mut().trim_text(false);
    let mut dimensions = ImageDimensions::default();
    let mut view_box = None;
    let mut element_count = 0u64;
    let mut unknown = BTreeSet::new();
    let mut stack = Vec::<SvgStackEntry>::new();
    let mut text = Vec::new();
    let mut links = Vec::new();
    let mut active = Vec::new();
    loop {
        if let Some(control) = control {
            control.checkpoint().map_err(|error| error.to_string())?;
        }
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = local_name(event.name().as_ref());
                element_count = element_count.saturating_add(1);
                if element_count > options.max_svg_elements {
                    return Err("SVG element count exceeds ImageOptions::max_svg_elements".into());
                }
                if stack.is_empty() && name != "svg" {
                    return Err("SVG root element is not <svg>".into());
                }
                let path = next_element_path(&mut stack, &name);
                let locator = event_locator(&reader, event.len() + 2, &path);
                inspect_element(
                    &event,
                    &name,
                    &path,
                    &locator,
                    &mut dimensions,
                    &mut view_box,
                    &mut links,
                    &mut active,
                )?;
                if !known_element(&name) {
                    unknown.insert(name.clone());
                }
                if name == "script" {
                    active.push(active_item("script", None, b"", locator.clone()));
                }
                if is_smil_element(&name) {
                    active.push(active_item(
                        "smil_animation",
                        Some(name.clone()),
                        name.as_bytes(),
                        locator.clone(),
                    ));
                }
                stack.push(SvgStackEntry {
                    name,
                    path,
                    next_child: 0,
                });
            }
            Ok(Event::Empty(event)) => {
                let name = local_name(event.name().as_ref());
                element_count = element_count.saturating_add(1);
                if element_count > options.max_svg_elements {
                    return Err("SVG element count exceeds ImageOptions::max_svg_elements".into());
                }
                if stack.is_empty() && name != "svg" {
                    return Err("SVG root element is not <svg>".into());
                }
                let path = next_element_path(&mut stack, &name);
                let locator = event_locator(&reader, event.len() + 2, &path);
                inspect_element(
                    &event,
                    &name,
                    &path,
                    &locator,
                    &mut dimensions,
                    &mut view_box,
                    &mut links,
                    &mut active,
                )?;
                if !known_element(&name) {
                    unknown.insert(name.clone());
                }
                if name == "script" {
                    active.push(active_item("script", None, b"", locator.clone()));
                }
                if is_smil_element(&name) {
                    active.push(active_item(
                        "smil_animation",
                        Some(name.clone()),
                        name.as_bytes(),
                        locator,
                    ));
                }
            }
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Text(event)) => {
                let path = stack
                    .last()
                    .map(|entry| format!("{}/text()", entry.path))
                    .unwrap_or_else(|| "/text()".into());
                let locator = event_locator(&reader, event.len(), &path);
                let value = event
                    .unescape()
                    .map_err(|error| error.to_string())?
                    .into_owned();
                inspect_text(value, locator, &stack, &mut text, &mut links, &mut active);
            }
            Ok(Event::CData(event)) => {
                let path = stack
                    .last()
                    .map(|entry| format!("{}/text()", entry.path))
                    .unwrap_or_else(|| "/text()".into());
                let locator = event_locator(&reader, event.len(), &path);
                let value = reader
                    .decoder()
                    .decode(event.as_ref())
                    .map_err(|error| error.to_string())?
                    .into_owned();
                inspect_text(value, locator, &stack, &mut text, &mut links, &mut active);
            }
            Ok(Event::DocType(event)) => {
                let locator = event_locator(&reader, event.len() + 3, "/doctype()");
                active.push(active_item("doctype", None, event.as_ref(), locator));
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("malformed SVG XML: {error}")),
            _ => {}
        }
    }
    if element_count == 0 {
        return Err("SVG contains no elements".into());
    }
    if dimensions.width == 0 || dimensions.height == 0 {
        if let Some([_, _, width, height]) = view_box {
            dimensions.width = positive_u32(width);
            dimensions.height = positive_u32(height);
        }
    }
    if dimensions.width == 0 || dimensions.height == 0 {
        return Err("SVG has neither positive width/height nor a usable viewBox".into());
    }
    Ok(SvgParsed {
        dimensions,
        vector: SvgContent {
            view_box,
            element_count,
            unknown_elements: unknown.into_iter().collect(),
        },
        text,
        links,
        active,
    })
}

fn inspect_text(
    value: String,
    locator: SourceLocator,
    stack: &[SvgStackEntry],
    text: &mut Vec<ImageText>,
    links: &mut Vec<ImageLink>,
    active: &mut Vec<ImageActiveContent>,
) {
    if value.trim().is_empty() {
        return;
    }
    let current = stack.last().map(|entry| entry.name.as_str()).unwrap_or("");
    if current == "script" {
        active.push(active_item("script_body", None, value.as_bytes(), locator));
    } else if current == "style" {
        active.push(active_item(
            "style_block",
            None,
            value.as_bytes(),
            locator.clone(),
        ));
        inventory_css(&value, &locator, links, active);
    } else if matches!(current, "text" | "tspan" | "textPath" | "title" | "desc") {
        let kind = match current {
            "title" => ImageTextKind::VectorTitle,
            "desc" => ImageTextKind::VectorDescription,
            _ => ImageTextKind::VectorText,
        };
        text.push(ImageText {
            kind,
            text: value,
            locator,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn inspect_element(
    event: &BytesStart<'_>,
    name: &str,
    path: &str,
    locator: &SourceLocator,
    dimensions: &mut ImageDimensions,
    view_box: &mut Option<[f64; 4]>,
    links: &mut Vec<ImageLink>,
    active: &mut Vec<ImageActiveContent>,
) -> Result<(), String> {
    for attribute in event.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| format!("malformed SVG attribute: {error}"))?;
        let key = local_name(attribute.key.as_ref());
        let value = attribute
            .unescape_value()
            .map_err(|error| format!("malformed SVG attribute value: {error}"))?
            .into_owned();
        if name == "svg" {
            match key.as_str() {
                "width" => dimensions.width = parse_length(&value),
                "height" => dimensions.height = parse_length(&value),
                "viewBox" | "viewbox" => *view_box = parse_view_box(&value),
                _ => {}
            }
        }
        if matches!(key.as_str(), "href" | "src" | "poster") {
            links.push(ImageLink {
                external: !value.starts_with('#'),
                target: value.clone(),
                locator: locator.clone(),
                disposition: ImageActiveContentDisposition::InventoriedNotExecuted,
            });
            if value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("javascript:")
            {
                active.push(active_item(
                    "javascript_uri",
                    Some(key.clone()),
                    value.as_bytes(),
                    locator.clone(),
                ));
            }
        }
        if key == "style" {
            inventory_css(&value, locator, links, active);
        }
        if key.to_ascii_lowercase().starts_with("on") {
            active.push(active_item(
                "event_handler",
                Some(key),
                value.as_bytes(),
                locator.clone(),
            ));
        } else if is_smil_timing_attribute(&key) {
            active.push(active_item(
                "smil_timing",
                Some(key),
                value.as_bytes(),
                locator.clone(),
            ));
        }
    }
    if matches!(name, "foreignObject" | "iframe" | "audio" | "video") {
        active.push(active_item(
            "active_element",
            Some(name.into()),
            name.as_bytes(),
            locator.clone(),
        ));
    }
    let _ = path;
    Ok(())
}

fn active_item(
    kind: &str,
    name: Option<String>,
    value: &[u8],
    locator: SourceLocator,
) -> ImageActiveContent {
    ImageActiveContent {
        kind: kind.into(),
        name,
        value_sha256: sha256_hex(value),
        locator,
        disposition: ImageActiveContentDisposition::InventoriedNotExecuted,
    }
}

fn svg_locator(start: usize, end: usize, path: &str) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ByteRange {
        byte_start: start,
        byte_end: end,
    })
    .and_then(|locator| {
        locator.nested(LocationComponent::XmlPath {
            path: path.to_string(),
        })
    })
    .expect("SVG locator is structurally valid")
}

fn event_locator(reader: &Reader<&[u8]>, event_length: usize, path: &str) -> SourceLocator {
    let end = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
    svg_locator(end.saturating_sub(event_length), end, path)
}

fn next_element_path(stack: &mut [SvgStackEntry], name: &str) -> String {
    let ordinal = if let Some(parent) = stack.last_mut() {
        parent.next_child = parent.next_child.saturating_add(1);
        parent.next_child
    } else {
        1
    };
    stack.last().map_or_else(
        || format!("/{name}[{ordinal}]"),
        |parent| format!("{}/{name}[{ordinal}]", parent.path),
    )
}

fn inventory_css(
    css: &str,
    locator: &SourceLocator,
    links: &mut Vec<ImageLink>,
    active: &mut Vec<ImageActiveContent>,
) {
    for (kind, target) in css_resource_targets(css) {
        links.push(ImageLink {
            external: !target.starts_with('#'),
            target: target.clone(),
            locator: locator.clone(),
            disposition: ImageActiveContentDisposition::InventoriedNotExecuted,
        });
        active.push(active_item(kind, None, target.as_bytes(), locator.clone()));
    }
}

fn css_resource_targets(css: &str) -> Vec<(&'static str, String)> {
    let lower = css.to_ascii_lowercase();
    let mut found = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = lower[cursor..].find("url(") {
        let start = cursor + relative + 4;
        let Some(relative_end) = css[start..].find(')') else {
            break;
        };
        let end = start + relative_end;
        let target = css[start..end].trim().trim_matches(['\'', '"']).to_string();
        if !target.is_empty() {
            found.push(("css_url", target));
        }
        cursor = end + 1;
    }
    cursor = 0;
    while let Some(relative) = lower[cursor..].find("@import") {
        let start = cursor + relative + "@import".len();
        let tail = css[start..].trim_start();
        let target = if tail.to_ascii_lowercase().starts_with("url(") {
            tail[4..]
                .split(')')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['\'', '"'])
        } else {
            tail.trim_start_matches(['\'', '"'])
                .split(['\'', '"', ';', ' ', '\t', '\r', '\n'])
                .next()
                .unwrap_or("")
        };
        if !target.is_empty() {
            found.push(("css_import", target.to_string()));
        }
        cursor = start.saturating_add(1);
    }
    found
}

fn is_smil_element(name: &str) -> bool {
    matches!(
        name,
        "animate" | "animateMotion" | "animateTransform" | "set" | "discard"
    )
}

fn is_smil_timing_attribute(name: &str) -> bool {
    matches!(
        name,
        "begin" | "end" | "dur" | "min" | "max" | "repeatCount" | "repeatDur" | "restart"
    )
}

pub(crate) fn frame_locator(index: u64, dimensions: ImageDimensions) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ImageRegion {
        frame: IndexPosition::zero_based(index),
        bbox: Some(BoundingBox {
            x: 0.0,
            y: 0.0,
            width: f64::from(dimensions.width),
            height: f64::from(dimensions.height),
            unit: CoordinateUnit::Pixels,
            origin: CoordinateOrigin::TopLeft,
        }),
    })
    .expect("image dimensions form a valid pixel rectangle")
}

fn local_name(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    raw.rsplit(':').next().unwrap_or(&raw).to_string()
}

fn parse_length(value: &str) -> u32 {
    let number: String = value
        .trim()
        .chars()
        .take_while(|character| character.is_ascii_digit() || matches!(character, '.' | '+' | '-'))
        .collect();
    number.parse::<f64>().ok().map(positive_u32).unwrap_or(0)
}

fn positive_u32(value: f64) -> u32 {
    if value.is_finite() && value > 0.0 && value <= f64::from(u32::MAX) {
        value.ceil() as u32
    } else {
        0
    }
}

fn parse_view_box(value: &str) -> Option<[f64; 4]> {
    let numbers = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|item| !item.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (numbers.len() == 4).then(|| [numbers[0], numbers[1], numbers[2], numbers[3]])
}

fn known_element(name: &str) -> bool {
    matches!(
        name,
        "svg"
            | "g"
            | "defs"
            | "symbol"
            | "use"
            | "switch"
            | "a"
            | "path"
            | "rect"
            | "circle"
            | "ellipse"
            | "line"
            | "polyline"
            | "polygon"
            | "text"
            | "tspan"
            | "textPath"
            | "title"
            | "desc"
            | "image"
            | "marker"
            | "pattern"
            | "clipPath"
            | "mask"
            | "linearGradient"
            | "radialGradient"
            | "stop"
            | "filter"
            | "feBlend"
            | "feColorMatrix"
            | "feComponentTransfer"
            | "feComposite"
            | "feConvolveMatrix"
            | "feDiffuseLighting"
            | "feDisplacementMap"
            | "feFlood"
            | "feGaussianBlur"
            | "feImage"
            | "feMerge"
            | "feMorphology"
            | "feOffset"
            | "feSpecularLighting"
            | "feTile"
            | "feTurbulence"
            | "style"
            | "script"
            | "metadata"
            | "view"
            | "animate"
            | "animateMotion"
            | "animateTransform"
            | "set"
            | "discard"
    )
}
