use super::model::*;
use super::parse::ImageResult;
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
    next_child: u64,
}

pub(crate) fn parse_svg(
    bytes: &[u8],
    options: &ImageOptions,
    control: Option<&OperationControl>,
) -> ImageResult<SvgParsed> {
    let source = std::str::from_utf8(bytes).map_err(|_| "SVG source is not valid UTF-8")?;
    let mut reader = Reader::from_str(source);
    reader.config_mut().trim_text(false);
    let mut dimensions = ImageDimensions::default();
    let mut view_box = None;
    let mut element_count = 0u64;
    let mut saw_root = false;
    let mut locator_index = 0u64;
    let mut unknown = BTreeSet::new();
    let mut stack = Vec::<SvgStackEntry>::new();
    let mut text = Vec::new();
    let mut links = Vec::new();
    let mut active = Vec::new();
    loop {
        if let Some(control) = control {
            control.checkpoint()?;
        }
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                charge_nodes(control, 1)?;
                let name = local_name(event.name().as_ref());
                element_count = element_count.saturating_add(1);
                if element_count > options.max_svg_elements {
                    return Err("SVG element count exceeds ImageOptions::max_svg_elements".into());
                }
                if stack.is_empty() {
                    if saw_root {
                        return Err("SVG contains more than one root element".into());
                    }
                    if name != "svg" {
                        return Err("SVG root element is not <svg>".into());
                    }
                    saw_root = true;
                }
                let depth = stack.len().saturating_add(1) as u64;
                if depth > options.max_svg_depth {
                    return Err("SVG nesting exceeds ImageOptions::max_svg_depth".into());
                }
                observe_depth(control, depth)?;
                let path =
                    next_element_path(&mut stack, &name, depth, &mut locator_index, options)?;
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
                    control,
                )?;
                if !known_element(&name) {
                    unknown.insert(name.clone());
                }
                if name == "script" {
                    charge_nodes(control, 1)?;
                    active.push(active_item("script", None, b"", locator.clone()));
                }
                if is_smil_element(&name) {
                    charge_nodes(control, 1)?;
                    active.push(active_item(
                        "smil_animation",
                        Some(name.clone()),
                        name.as_bytes(),
                        locator.clone(),
                    ));
                }
                stack.push(SvgStackEntry {
                    name,
                    next_child: 0,
                });
            }
            Ok(Event::Empty(event)) => {
                charge_nodes(control, 1)?;
                let name = local_name(event.name().as_ref());
                element_count = element_count.saturating_add(1);
                if element_count > options.max_svg_elements {
                    return Err("SVG element count exceeds ImageOptions::max_svg_elements".into());
                }
                if stack.is_empty() {
                    if saw_root {
                        return Err("SVG contains more than one root element".into());
                    }
                    if name != "svg" {
                        return Err("SVG root element is not <svg>".into());
                    }
                    saw_root = true;
                }
                let depth = stack.len().saturating_add(1) as u64;
                if depth > options.max_svg_depth {
                    return Err("SVG nesting exceeds ImageOptions::max_svg_depth".into());
                }
                observe_depth(control, depth)?;
                let path =
                    next_element_path(&mut stack, &name, depth, &mut locator_index, options)?;
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
                    control,
                )?;
                if !known_element(&name) {
                    unknown.insert(name.clone());
                }
                if name == "script" {
                    charge_nodes(control, 1)?;
                    active.push(active_item("script", None, b"", locator.clone()));
                }
                if is_smil_element(&name) {
                    charge_nodes(control, 1)?;
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
                let raw: &[u8] = event.as_ref();
                if raw.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                charge_decoded_characters(control, raw.len() as u64)?;
                let path = text_path(&stack, &mut locator_index, options)?;
                let locator = event_locator(&reader, event.len(), &path);
                let value = event
                    .unescape()
                    .map_err(|error| error.to_string())?
                    .into_owned();
                inspect_text(
                    value,
                    locator,
                    &stack,
                    &mut text,
                    &mut links,
                    &mut active,
                    control,
                )?;
            }
            Ok(Event::CData(event)) => {
                let raw: &[u8] = event.as_ref();
                if raw.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                charge_decoded_characters(control, raw.len() as u64)?;
                let path = text_path(&stack, &mut locator_index, options)?;
                let locator = event_locator(&reader, event.len(), &path);
                let value = reader
                    .decoder()
                    .decode(event.as_ref())
                    .map_err(|error| error.to_string())?
                    .into_owned();
                inspect_text(
                    value,
                    locator,
                    &stack,
                    &mut text,
                    &mut links,
                    &mut active,
                    control,
                )?;
            }
            Ok(Event::DocType(event)) => {
                charge_nodes(control, 1)?;
                let locator = event_locator(&reader, event.len() + 3, "/doctype()");
                active.push(active_item("doctype", None, event.as_ref(), locator));
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(format!("malformed SVG XML: {error}").into()),
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("SVG contains an unclosed element".into());
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
    control: Option<&OperationControl>,
) -> ImageResult<()> {
    if value.trim().is_empty() {
        return Ok(());
    }
    let current = stack.last().map(|entry| entry.name.as_str()).unwrap_or("");
    if current == "script" {
        charge_nodes(control, 1)?;
        active.push(active_item("script_body", None, value.as_bytes(), locator));
    } else if current == "style" {
        charge_nodes(control, 1)?;
        active.push(active_item(
            "style_block",
            None,
            value.as_bytes(),
            locator.clone(),
        ));
        inventory_css(&value, &locator, links, active, control)?;
    } else if matches!(current, "text" | "tspan" | "textPath" | "title" | "desc") {
        let kind = match current {
            "title" => ImageTextKind::VectorTitle,
            "desc" => ImageTextKind::VectorDescription,
            _ => ImageTextKind::VectorText,
        };
        charge_nodes(control, 1)?;
        text.push(ImageText {
            kind,
            text: value,
            locator,
        });
    }
    Ok(())
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
    control: Option<&OperationControl>,
) -> ImageResult<()> {
    for attribute in event.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| format!("malformed SVG attribute: {error}"))?;
        let key = local_name(attribute.key.as_ref());
        charge_decoded_characters(control, attribute.value.as_ref().len() as u64)?;
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
            charge_nodes(control, 1)?;
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
                charge_nodes(control, 1)?;
                active.push(active_item(
                    "javascript_uri",
                    Some(key.clone()),
                    value.as_bytes(),
                    locator.clone(),
                ));
            }
        }
        if key == "style" {
            inventory_css(&value, locator, links, active, control)?;
        }
        if key.to_ascii_lowercase().starts_with("on") {
            charge_nodes(control, 1)?;
            active.push(active_item(
                "event_handler",
                Some(key),
                value.as_bytes(),
                locator.clone(),
            ));
        } else if is_smil_timing_attribute(&key) {
            charge_nodes(control, 1)?;
            active.push(active_item(
                "smil_timing",
                Some(key),
                value.as_bytes(),
                locator.clone(),
            ));
        }
    }
    if matches!(name, "foreignObject" | "iframe" | "audio" | "video") {
        charge_nodes(control, 1)?;
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

fn next_element_path(
    stack: &mut [SvgStackEntry],
    name: &str,
    depth: u64,
    locator_index: &mut u64,
    options: &ImageOptions,
) -> ImageResult<String> {
    let ordinal = if let Some(parent) = stack.last_mut() {
        parent.next_child = parent.next_child.saturating_add(1);
        parent.next_child
    } else {
        1
    };
    *locator_index = locator_index.saturating_add(1);
    let path_bytes = "/element()[;name=;sibling=;depth=]"
        .len()
        .saturating_add(decimal_digits(*locator_index))
        .saturating_add(name.len())
        .saturating_add(decimal_digits(ordinal))
        .saturating_add(decimal_digits(depth));
    if path_bytes as u64 > options.max_svg_path_bytes {
        return Err("SVG locator path exceeds ImageOptions::max_svg_path_bytes".into());
    }
    let mut path = String::with_capacity(path_bytes);
    use std::fmt::Write as _;
    write!(
        path,
        "/element()[{};name={name};sibling={ordinal};depth={depth}]",
        *locator_index
    )
    .expect("writing to a String cannot fail");
    Ok(path)
}

fn text_path(
    stack: &[SvgStackEntry],
    locator_index: &mut u64,
    options: &ImageOptions,
) -> ImageResult<String> {
    *locator_index = locator_index.saturating_add(1);
    let name = stack.last().map_or("document", |entry| entry.name.as_str());
    let path_bytes = "/text()[;parent=]"
        .len()
        .saturating_add(decimal_digits(*locator_index))
        .saturating_add(name.len());
    if path_bytes as u64 > options.max_svg_path_bytes {
        return Err("SVG locator path exceeds ImageOptions::max_svg_path_bytes".into());
    }
    let mut path = String::with_capacity(path_bytes);
    use std::fmt::Write as _;
    write!(path, "/text()[{};parent={name}]", *locator_index)
        .expect("writing to a String cannot fail");
    Ok(path)
}

fn decimal_digits(mut value: u64) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

fn inventory_css(
    css: &str,
    locator: &SourceLocator,
    links: &mut Vec<ImageLink>,
    active: &mut Vec<ImageActiveContent>,
    control: Option<&OperationControl>,
) -> ImageResult<()> {
    let mut cursor = 0usize;
    while let Some(relative) = find_ascii_case_insensitive(&css.as_bytes()[cursor..], b"url(") {
        if let Some(control) = control {
            control.checkpoint()?;
        }
        let start = cursor + relative + 4;
        let Some(relative_end) = css[start..].find(')') else {
            break;
        };
        let end = start + relative_end;
        let target = css[start..end].trim().trim_matches(['\'', '"']);
        if !target.is_empty() {
            emit_css_resource("css_url", target, locator, links, active, control)?;
        }
        cursor = end.saturating_add(1);
    }
    cursor = 0;
    while let Some(relative) = find_ascii_case_insensitive(&css.as_bytes()[cursor..], b"@import") {
        if let Some(control) = control {
            control.checkpoint()?;
        }
        let start = cursor + relative + "@import".len();
        let tail = css[start..].trim_start();
        let target = if tail
            .as_bytes()
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"url("))
        {
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
            emit_css_resource("css_import", target, locator, links, active, control)?;
        }
        cursor = start.saturating_add(1);
    }
    Ok(())
}

fn emit_css_resource(
    kind: &'static str,
    target: &str,
    locator: &SourceLocator,
    links: &mut Vec<ImageLink>,
    active: &mut Vec<ImageActiveContent>,
    control: Option<&OperationControl>,
) -> ImageResult<()> {
    if let Some(control) = control {
        control.checkpoint()?;
    }
    charge_nodes(control, 2)?;
    links.push(ImageLink {
        external: !target.starts_with('#'),
        target: target.to_string(),
        locator: locator.clone(),
        disposition: ImageActiveContentDisposition::InventoriedNotExecuted,
    });
    active.push(active_item(kind, None, target.as_bytes(), locator.clone()));
    Ok(())
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

fn charge_nodes(control: Option<&OperationControl>, count: u64) -> ImageResult<()> {
    control.map_or(Ok(()), |control| {
        control.budget().consume_nodes(count).map_err(Into::into)
    })
}

fn charge_decoded_characters(control: Option<&OperationControl>, count: u64) -> ImageResult<()> {
    control.map_or(Ok(()), |control| {
        control
            .budget()
            .consume_decoded_characters(count)
            .map_err(Into::into)
    })
}

fn observe_depth(control: Option<&OperationControl>, depth: u64) -> ImageResult<()> {
    control.map_or(Ok(()), |control| {
        control
            .budget()
            .observe_nesting_depth(depth)
            .map_err(Into::into)
    })
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
