//! Authoritative PresentationML slide content and inferred reading order.

use super::archive::PackageEntry;
use super::content_types::content_type_for;
use super::model::*;
use super::xml::{XmlNode, attribute, descendant_text, local_name, parse_xml_part};
use super::{part_locator, xml_locator};
use crate::core::{
    BoundingBox, ContentIdentity, CoordinateOrigin, CoordinateUnit, Diagnostic, FormatIdentity,
    IndexPosition, LocationComponent, SourceLocator,
};
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
struct Acc {
    shapes: Vec<PresentationShape>,
    tables: Vec<PresentationTable>,
    charts: Vec<PresentationChart>,
    equations: Vec<PresentationEquation>,
    images: Vec<PresentationImage>,
    links: Vec<PresentationLink>,
    objects: Vec<PresentationEmbeddedObject>,
}

#[derive(Clone, Default)]
struct Author {
    name: Option<String>,
    initials: Option<String>,
}

pub(super) fn parse_slide_contents(
    entries: &[PackageEntry],
    relationships: &[PresentationRelationship],
    types: &PresentationContentTypes,
    slides: &[PresentationSlideReference],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PresentationSlideContent> {
    let authors = parse_authors(entries, diagnostics);
    slides
        .iter()
        .filter_map(|slide| {
            let part = slide.part.as_deref()?;
            let bytes = entry_bytes(entries, part)?;
            let nodes = parse_xml_part(part, bytes, diagnostics).ok()?;
            let mut acc = Acc::default();
            if let Some(tree) = find(&nodes, 0, "spTree") {
                collect_shapes(
                    &nodes,
                    tree,
                    None,
                    slide.order,
                    part,
                    entries,
                    relationships,
                    types,
                    diagnostics,
                    &mut acc,
                );
            }
            Some(PresentationSlideContent {
                order: slide.order,
                slide_id: slide.slide_id.clone(),
                part: part.into(),
                locator: slide_loc(part, "/", slide.order, None, None),
                notes: parse_notes(
                    entries,
                    relationships,
                    types,
                    slide.order,
                    part,
                    diagnostics,
                ),
                comments: parse_comments(entries, relationships, part, &authors, diagnostics),
                transition: parse_transition(&nodes, part),
                animations: parse_animations(&nodes, part),
                reading_order: reading_order(&acc.shapes),
                shapes: acc.shapes,
                tables: acc.tables,
                charts: acc.charts,
                equations: acc.equations,
                images: acc.images,
                links: acc.links,
                embedded_objects: acc.objects,
            })
        })
        .collect()
}

fn entry_bytes<'a>(entries: &'a [PackageEntry], part: &str) -> Option<&'a [u8]> {
    entries
        .iter()
        .find(|entry| entry.path == part && entry.rejected.is_none())?
        .bytes
        .as_deref()
}

fn relation<'a>(
    rels: &'a [PresentationRelationship],
    source: &str,
    id: &str,
) -> Option<&'a PresentationRelationship> {
    rels.iter()
        .find(|item| item.source_part.as_deref() == Some(source) && item.id == id)
}

fn find(nodes: &[XmlNode], root: usize, wanted: &str) -> Option<usize> {
    if local_name(&nodes[root].name) == wanted {
        return Some(root);
    }
    nodes[root]
        .children
        .iter()
        .find_map(|child| find(nodes, *child, wanted))
}

fn direct(nodes: &[XmlNode], root: usize, wanted: &str) -> Option<usize> {
    nodes[root]
        .children
        .iter()
        .copied()
        .find(|child| local_name(&nodes[*child].name) == wanted)
}

fn shape_element(name: &str) -> bool {
    matches!(
        name,
        "sp" | "pic" | "graphicFrame" | "grpSp" | "cxnSp" | "contentPart"
    )
}

fn scoped(nodes: &[XmlNode], root: usize, wanted: &str) -> Option<usize> {
    nodes[root].children.iter().find_map(|child| {
        let name = local_name(&nodes[*child].name);
        if name == wanted {
            Some(*child)
        } else if shape_element(name) {
            None
        } else {
            scoped(nodes, *child, wanted)
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_shapes(
    nodes: &[XmlNode],
    root: usize,
    parent: Option<&str>,
    slide: usize,
    part: &str,
    entries: &[PackageEntry],
    rels: &[PresentationRelationship],
    types: &PresentationContentTypes,
    diagnostics: &mut Vec<Diagnostic>,
    acc: &mut Acc,
) {
    for child in &nodes[root].children {
        if !shape_element(local_name(&nodes[*child].name)) {
            continue;
        }
        let shape = parse_shape(nodes, *child, parent, slide, part, acc.shapes.len());
        let id = shape.shape_id.clone();
        parse_objects(
            nodes,
            *child,
            &shape,
            slide,
            part,
            entries,
            rels,
            types,
            diagnostics,
            acc,
        );
        acc.shapes.push(shape);
        if local_name(&nodes[*child].name) == "grpSp" {
            collect_shapes(
                nodes,
                *child,
                Some(&id),
                slide,
                part,
                entries,
                rels,
                types,
                diagnostics,
                acc,
            );
        }
    }
}

fn parse_shape(
    nodes: &[XmlNode],
    root: usize,
    parent: Option<&str>,
    slide: usize,
    part: &str,
    z_order: usize,
) -> PresentationShape {
    let nv = scoped(nodes, root, "cNvPr");
    let id = nv
        .and_then(|node| attribute(&nodes[node], "id"))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("xml:{}", nodes[root].path));
    let geometry = parse_geometry(nodes, root, slide, part, &id);
    let placeholder = scoped(nodes, root, "ph");
    let mut rel_ids = BTreeSet::new();
    collect_rel_ids(nodes, root, &mut rel_ids);
    PresentationShape {
        shape_id: id.clone(),
        source_element: nodes[root].name.clone(),
        kind: match local_name(&nodes[root].name) {
            "sp" => PresentationShapeKind::Shape,
            "pic" => PresentationShapeKind::Picture,
            "graphicFrame" => PresentationShapeKind::GraphicFrame,
            "grpSp" => PresentationShapeKind::Group,
            "cxnSp" => PresentationShapeKind::Connector,
            "contentPart" => PresentationShapeKind::ContentPart,
            _ => PresentationShapeKind::Unknown,
        },
        parent_shape_id: parent.map(str::to_string),
        z_order,
        name: nv
            .and_then(|n| attribute(&nodes[n], "name"))
            .map(str::to_string),
        placeholder_type: placeholder
            .and_then(|n| attribute(&nodes[n], "type"))
            .map(str::to_string),
        placeholder_index: placeholder
            .and_then(|n| attribute(&nodes[n], "idx"))
            .map(str::to_string),
        alt_text: nv
            .and_then(|n| attribute(&nodes[n], "descr"))
            .map(str::to_string),
        alt_title: nv
            .and_then(|n| attribute(&nodes[n], "title"))
            .map(str::to_string),
        decorative: scoped(nodes, root, "decorative")
            .and_then(|n| attribute(&nodes[n], "val"))
            .is_some_and(truthy),
        hidden: nv
            .and_then(|n| attribute(&nodes[n], "hidden"))
            .is_some_and(truthy),
        text_body: scoped(nodes, root, "txBody")
            .map(|n| text_body(nodes, n, part, slide, Some(&id))),
        relationship_ids: rel_ids.into_iter().collect(),
        metadata: ["nvPr", "spPr", "style", "grpSpPr"]
            .into_iter()
            .filter_map(|name| scoped(nodes, root, name))
            .flat_map(|n| metadata(nodes, n, part))
            .collect(),
        locator: slide_loc(
            part,
            &nodes[root].path,
            slide,
            Some(&id),
            geometry.as_ref().and_then(bbox),
        ),
        geometry,
    }
}

fn collect_rel_ids(nodes: &[XmlNode], root: usize, ids: &mut BTreeSet<String>) {
    for (name, value) in &nodes[root].attributes {
        if matches!(name.as_str(), "r:id" | "r:embed" | "r:link") {
            ids.insert(value.clone());
        }
    }
    for child in &nodes[root].children {
        if !shape_element(local_name(&nodes[*child].name)) {
            collect_rel_ids(nodes, *child, ids);
        }
    }
}

fn parse_geometry(
    nodes: &[XmlNode],
    root: usize,
    slide: usize,
    part: &str,
    id: &str,
) -> Option<PresentationShapeGeometry> {
    let xfrm = scoped(nodes, root, "xfrm")?;
    let num = |node: Option<usize>, name: &str| {
        node.and_then(|n| attribute(&nodes[n], name))
            .and_then(|value| value.parse::<i64>().ok())
    };
    let off = direct(nodes, xfrm, "off");
    let ext = direct(nodes, xfrm, "ext");
    let child_off = direct(nodes, xfrm, "chOff");
    let child_ext = direct(nodes, xfrm, "chExt");
    Some(PresentationShapeGeometry {
        x: num(off, "x"),
        y: num(off, "y"),
        width: num(ext, "cx"),
        height: num(ext, "cy"),
        child_x: num(child_off, "x"),
        child_y: num(child_off, "y"),
        child_width: num(child_ext, "cx"),
        child_height: num(child_ext, "cy"),
        rotation: num(Some(xfrm), "rot"),
        flip_horizontal: attribute(&nodes[xfrm], "flipH").is_some_and(truthy),
        flip_vertical: attribute(&nodes[xfrm], "flipV").is_some_and(truthy),
        preset_geometry: scoped(nodes, root, "prstGeom")
            .and_then(|n| attribute(&nodes[n], "prst"))
            .map(str::to_string),
        has_custom_geometry: scoped(nodes, root, "custGeom").is_some(),
        attributes: nodes[xfrm].attributes.clone(),
        locator: slide_loc(part, &nodes[xfrm].path, slide, Some(id), None),
    })
}

fn bbox(value: &PresentationShapeGeometry) -> Option<BoundingBox> {
    let (x, y, width, height) = (value.x?, value.y?, value.width?, value.height?);
    if width < 0 || height < 0 {
        return None;
    }
    Some(BoundingBox {
        x: x as f64 / 12_700.0,
        y: y as f64 / 12_700.0,
        width: width as f64 / 12_700.0,
        height: height as f64 / 12_700.0,
        unit: CoordinateUnit::Points,
        origin: CoordinateOrigin::TopLeft,
    })
}

fn slide_loc(
    part: &str,
    path: &str,
    slide: usize,
    id: Option<&str>,
    bbox: Option<BoundingBox>,
) -> SourceLocator {
    let base = part_locator(part).expect("validated OOXML part");
    let base = if path == "/" {
        base
    } else {
        base.nested(LocationComponent::XmlPath { path: path.into() })
            .expect("valid XML path")
    };
    let position = IndexPosition::one_based(u64::try_from(slide).unwrap_or(u64::MAX) + 1)
        .expect("one-based slide");
    base.nested(LocationComponent::SlideRegion {
        slide: position,
        shape_id: id.map(str::to_string),
        bbox,
    })
    .expect("valid slide region")
}

fn metadata(nodes: &[XmlNode], root: usize, part: &str) -> Vec<PresentationXmlMetadata> {
    let mut out = Vec::new();
    metadata_into(nodes, root, part, &mut out);
    out
}

fn metadata_into(
    nodes: &[XmlNode],
    root: usize,
    part: &str,
    out: &mut Vec<PresentationXmlMetadata>,
) {
    let text = nodes[root].text.trim();
    out.push(PresentationXmlMetadata {
        element: nodes[root].name.clone(),
        attributes: nodes[root].attributes.clone(),
        text: (!text.is_empty()).then(|| text.into()),
        locator: xml_locator(part, &nodes[root].path),
    });
    for child in &nodes[root].children {
        metadata_into(nodes, *child, part, out);
    }
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes"
    )
}

fn text_body(
    nodes: &[XmlNode],
    root: usize,
    part: &str,
    slide: usize,
    id: Option<&str>,
) -> PresentationTextBody {
    let paragraphs = nodes[root]
        .children
        .iter()
        .filter(|child| local_name(&nodes[**child].name) == "p")
        .enumerate()
        .map(|(i, child)| paragraph(nodes, *child, i, part, slide, id))
        .collect::<Vec<_>>();
    let text = paragraphs
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let properties = nodes[root]
        .children
        .iter()
        .filter(|child| matches!(local_name(&nodes[**child].name), "bodyPr" | "lstStyle"))
        .flat_map(|child| metadata(nodes, *child, part))
        .collect();
    PresentationTextBody {
        paragraphs,
        text,
        properties,
        locator: slide_loc(part, &nodes[root].path, slide, id, None),
    }
}

fn paragraph(
    nodes: &[XmlNode],
    root: usize,
    index: usize,
    part: &str,
    slide: usize,
    id: Option<&str>,
) -> PresentationTextParagraph {
    let props = direct(nodes, root, "pPr");
    let runs = nodes[root]
        .children
        .iter()
        .filter(|child| matches!(local_name(&nodes[**child].name), "r" | "fld" | "br"))
        .enumerate()
        .map(|(i, child)| text_run(nodes, *child, i, part, slide, id))
        .collect::<Vec<_>>();
    PresentationTextParagraph {
        index,
        text: runs.iter().map(|run| run.text.as_str()).collect(),
        runs,
        level: props
            .and_then(|n| attribute(&nodes[n], "lvl"))
            .and_then(|v| v.parse().ok()),
        alignment: props
            .and_then(|n| attribute(&nodes[n], "algn"))
            .map(str::to_string),
        properties: props.map(|n| metadata(nodes, n, part)).unwrap_or_default(),
        locator: slide_loc(part, &nodes[root].path, slide, id, None),
    }
}

fn text_run(
    nodes: &[XmlNode],
    root: usize,
    index: usize,
    part: &str,
    slide: usize,
    id: Option<&str>,
) -> PresentationTextRun {
    let kind = match local_name(&nodes[root].name) {
        "fld" => PresentationTextRunKind::Field,
        "br" => PresentationTextRunKind::Break,
        _ => PresentationTextRunKind::Text,
    };
    let props = direct(nodes, root, "rPr");
    let hyperlink = props.and_then(|n| find(nodes, n, "hlinkClick"));
    let text = if kind == PresentationTextRunKind::Break {
        "\n".into()
    } else {
        find(nodes, root, "t")
            .map(|n| descendant_text(nodes, n))
            .unwrap_or_default()
    };
    PresentationTextRun {
        index,
        kind,
        text,
        field_id: attribute(&nodes[root], "id").map(str::to_string),
        field_type: attribute(&nodes[root], "type").map(str::to_string),
        language: props
            .and_then(|n| attribute(&nodes[n], "lang"))
            .map(str::to_string),
        font_size: props
            .and_then(|n| attribute(&nodes[n], "sz"))
            .and_then(|v| v.parse().ok()),
        bold: props.and_then(|n| attribute(&nodes[n], "b")).map(truthy),
        italic: props.and_then(|n| attribute(&nodes[n], "i")).map(truthy),
        underline: props
            .and_then(|n| attribute(&nodes[n], "u"))
            .map(str::to_string),
        typeface: props
            .and_then(|n| find(nodes, n, "latin"))
            .and_then(|n| attribute(&nodes[n], "typeface"))
            .map(str::to_string),
        color: props.and_then(|n| run_color(nodes, n)),
        hyperlink_relationship_id: hyperlink
            .and_then(|n| attribute(&nodes[n], "r:id"))
            .map(str::to_string),
        hyperlink_action: hyperlink
            .and_then(|n| attribute(&nodes[n], "action"))
            .map(str::to_string),
        properties: props.map(|n| metadata(nodes, n, part)).unwrap_or_default(),
        locator: slide_loc(part, &nodes[root].path, slide, id, None),
    }
}

fn run_color(nodes: &[XmlNode], root: usize) -> Option<String> {
    ["srgbClr", "schemeClr", "sysClr", "prstClr"]
        .into_iter()
        .find_map(|name| find(nodes, root, name))
        .and_then(|n| attribute(&nodes[n], "val").or_else(|| attribute(&nodes[n], "lastClr")))
        .map(str::to_string)
}

#[allow(clippy::too_many_arguments)]
fn parse_objects(
    nodes: &[XmlNode],
    root: usize,
    shape: &PresentationShape,
    slide: usize,
    part: &str,
    entries: &[PackageEntry],
    rels: &[PresentationRelationship],
    types: &PresentationContentTypes,
    diagnostics: &mut Vec<Diagnostic>,
    acc: &mut Acc,
) {
    if let Some(table) = scoped(nodes, root, "tbl") {
        acc.tables
            .push(parse_table(nodes, table, shape, slide, part));
    }
    if let Some(chart) = scoped(nodes, root, "chart")
        && let Some(id) = attribute(&nodes[chart], "r:id")
    {
        acc.charts.push(parse_chart(
            nodes,
            chart,
            shape,
            part,
            id,
            entries,
            rels,
            diagnostics,
        ));
    }
    if shape.kind == PresentationShapeKind::Picture {
        acc.images
            .push(parse_image(nodes, root, shape, part, entries, rels, types));
    }
    collect_equations(nodes, root, shape, part, &mut acc.equations);
    collect_links(nodes, root, shape, part, rels, &mut acc.links);
    for name in ["oleObj", "embeddedObject"] {
        if let Some(node) = scoped(nodes, root, name) {
            acc.objects
                .push(parse_embedded(nodes, node, shape, part, rels, types));
        }
    }
    if shape.kind == PresentationShapeKind::ContentPart {
        acc.objects
            .push(parse_embedded(nodes, root, shape, part, rels, types));
    }
}

fn parse_table(
    nodes: &[XmlNode],
    root: usize,
    shape: &PresentationShape,
    slide: usize,
    part: &str,
) -> PresentationTable {
    let grid_columns = find(nodes, root, "tblGrid")
        .into_iter()
        .flat_map(|grid| nodes[grid].children.iter())
        .filter(|child| local_name(&nodes[**child].name) == "gridCol")
        .filter_map(|child| attribute(&nodes[*child], "w").and_then(|v| v.parse().ok()))
        .collect();
    let rows = nodes[root]
        .children
        .iter()
        .filter(|child| local_name(&nodes[**child].name) == "tr")
        .enumerate()
        .map(|(row, child)| table_row(nodes, *child, row, shape, slide, part))
        .collect();
    PresentationTable {
        shape_id: shape.shape_id.clone(),
        grid_columns,
        rows,
        style_id: find(nodes, root, "tableStyleId")
            .map(|n| descendant_text(nodes, n))
            .filter(|v| !v.is_empty()),
        locator: shape.locator.clone(),
    }
}

fn table_row(
    nodes: &[XmlNode],
    root: usize,
    row: usize,
    shape: &PresentationShape,
    slide: usize,
    part: &str,
) -> PresentationTableRow {
    let cells = nodes[root]
        .children
        .iter()
        .filter(|child| local_name(&nodes[**child].name) == "tc")
        .enumerate()
        .map(|(column, child)| {
            let attrs = nodes[*child].attributes.clone();
            PresentationTableCell {
                row,
                column,
                grid_span: attribute(&nodes[*child], "gridSpan")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
                row_span: attribute(&nodes[*child], "rowSpan")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
                horizontal_merge: attribute(&nodes[*child], "hMerge").is_some_and(truthy),
                vertical_merge: attribute(&nodes[*child], "vMerge").is_some_and(truthy),
                text_body: find(nodes, *child, "txBody")
                    .map(|n| text_body(nodes, n, part, slide, Some(&shape.shape_id))),
                attributes: attrs,
                locator: xml_locator(part, &nodes[*child].path),
            }
        })
        .collect();
    PresentationTableRow {
        index: row,
        height: attribute(&nodes[root], "h").and_then(|v| v.parse().ok()),
        cells,
        locator: xml_locator(part, &nodes[root].path),
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_chart(
    _slide_nodes: &[XmlNode],
    chart_node: usize,
    shape: &PresentationShape,
    slide_part: &str,
    id: &str,
    entries: &[PackageEntry],
    rels: &[PresentationRelationship],
    diagnostics: &mut Vec<Diagnostic>,
) -> PresentationChart {
    let related = relation(rels, slide_part, id);
    let part = related.and_then(|item| item.resolved_part.clone());
    let mut chart_types = Vec::new();
    let mut title = None;
    let mut series = Vec::new();
    let mut external = None;
    let mut locator = shape.locator.clone();
    if let Some(chart_part) = part.as_deref()
        && let Some(bytes) = entry_bytes(entries, chart_part)
    {
        if let Ok(nodes) = parse_xml_part(chart_part, bytes, diagnostics) {
            locator = part_locator(chart_part).expect("valid chart part");
            for (index, node) in nodes.iter().enumerate() {
                let name = local_name(&node.name);
                if name.ends_with("Chart") && name != "chart" {
                    chart_types.push(name.into());
                }
                if name == "title" && title.is_none() {
                    title = Some(descendant_text(&nodes, index)).filter(|value| !value.is_empty());
                }
                if name == "ser" {
                    series.push(chart_series(&nodes, index, series.len(), chart_part));
                }
                if name == "externalData" {
                    external = attribute(node, "r:id").map(str::to_string);
                }
            }
        }
    }
    chart_types.sort();
    chart_types.dedup();
    let output_locator = if part.is_some() {
        locator
    } else {
        xml_locator(slide_part, &_slide_nodes[chart_node].path)
    };
    PresentationChart {
        shape_id: shape.shape_id.clone(),
        relationship_id: id.into(),
        part,
        chart_types,
        title,
        series,
        external_data_relationship_id: external,
        locator: output_locator,
    }
}

fn chart_series(
    nodes: &[XmlNode],
    root: usize,
    index: usize,
    part: &str,
) -> PresentationChartSeries {
    let name = direct(nodes, root, "tx")
        .map(|n| descendant_text(nodes, n))
        .filter(|v| !v.is_empty());
    let categories = direct(nodes, root, "cat")
        .map(|n| chart_values(nodes, n))
        .unwrap_or_default();
    let values = direct(nodes, root, "val")
        .map(|n| chart_values(nodes, n))
        .unwrap_or_default();
    PresentationChartSeries {
        index,
        name,
        categories,
        values,
        locator: xml_locator(part, &nodes[root].path),
    }
}

fn chart_values(nodes: &[XmlNode], root: usize) -> Vec<String> {
    let mut values = Vec::new();
    collect_values(nodes, root, &mut values);
    values
}

fn collect_values(nodes: &[XmlNode], root: usize, values: &mut Vec<String>) {
    if local_name(&nodes[root].name) == "v" {
        values.push(descendant_text(nodes, root));
        return;
    }
    for child in &nodes[root].children {
        collect_values(nodes, *child, values);
    }
}

fn parse_image(
    nodes: &[XmlNode],
    root: usize,
    shape: &PresentationShape,
    part: &str,
    entries: &[PackageEntry],
    rels: &[PresentationRelationship],
    types: &PresentationContentTypes,
) -> PresentationImage {
    let blip = scoped(nodes, root, "blip");
    let embed = blip
        .and_then(|n| attribute(&nodes[n], "r:embed"))
        .map(str::to_string);
    let linked = blip
        .and_then(|n| attribute(&nodes[n], "r:link"))
        .map(str::to_string);
    let related = embed
        .as_deref()
        .or(linked.as_deref())
        .and_then(|id| relation(rels, part, id));
    let target = related.and_then(|item| item.resolved_part.clone());
    let content_type = target
        .as_deref()
        .and_then(|target| content_type_for(types, target));
    let part_identity = target
        .as_deref()
        .and_then(|target| entry_bytes(entries, target))
        .map(|bytes| {
            ContentIdentity::for_raw_bytes(bytes).with_format(FormatIdentity::new(
                "presentation_image",
                content_type.clone(),
            ))
        });
    let crop = scoped(nodes, root, "srcRect")
        .map(|n| nodes[n].attributes.clone())
        .unwrap_or_default();
    PresentationImage {
        shape_id: shape.shape_id.clone(),
        relationship_id: embed,
        linked_relationship_id: linked,
        part: target,
        content_type,
        part_identity,
        alt_text: shape.alt_text.clone(),
        alt_title: shape.alt_title.clone(),
        crop,
        locator: shape.locator.clone(),
    }
}

fn parse_embedded(
    nodes: &[XmlNode],
    root: usize,
    shape: &PresentationShape,
    part: &str,
    rels: &[PresentationRelationship],
    types: &PresentationContentTypes,
) -> PresentationEmbeddedObject {
    let id = attribute(&nodes[root], "r:id").map(str::to_string);
    let target = id
        .as_deref()
        .and_then(|id| relation(rels, part, id))
        .and_then(|item| item.resolved_part.clone());
    PresentationEmbeddedObject {
        shape_id: shape.shape_id.clone(),
        relationship_id: id,
        program_id: attribute(&nodes[root], "progId").map(str::to_string),
        name: attribute(&nodes[root], "name").map(str::to_string),
        show_as_icon: attribute(&nodes[root], "showAsIcon").is_some_and(truthy),
        content_type: target
            .as_deref()
            .and_then(|target| content_type_for(types, target)),
        part: target,
        attributes: nodes[root].attributes.clone(),
        locator: xml_locator(part, &nodes[root].path),
    }
}

fn collect_equations(
    nodes: &[XmlNode],
    root: usize,
    shape: &PresentationShape,
    part: &str,
    out: &mut Vec<PresentationEquation>,
) {
    let name = local_name(&nodes[root].name);
    if matches!(name, "oMath" | "oMathPara") {
        out.push(PresentationEquation {
            shape_id: shape.shape_id.clone(),
            display: name == "oMathPara",
            text: descendant_text(nodes, root),
            metadata: metadata(nodes, root, part),
            locator: xml_locator(part, &nodes[root].path),
        });
        return;
    }
    for child in &nodes[root].children {
        if !shape_element(local_name(&nodes[*child].name)) {
            collect_equations(nodes, *child, shape, part, out);
        }
    }
}

fn collect_links(
    nodes: &[XmlNode],
    root: usize,
    shape: &PresentationShape,
    part: &str,
    rels: &[PresentationRelationship],
    out: &mut Vec<PresentationLink>,
) {
    let name = local_name(&nodes[root].name);
    if matches!(name, "hlinkClick" | "hlinkHover") {
        let id = attribute(&nodes[root], "r:id").map(str::to_string);
        let related = id.as_deref().and_then(|id| relation(rels, part, id));
        out.push(PresentationLink {
            source_shape_id: shape.shape_id.clone(),
            source_run: None,
            kind: if name == "hlinkHover" {
                PresentationActionKind::Hover
            } else {
                PresentationActionKind::Click
            },
            relationship_id: id,
            action: attribute(&nodes[root], "action").map(str::to_string),
            target: related.map(|item| item.target.clone()),
            external: related.is_some_and(|item| {
                item.target_mode == PresentationRelationshipTargetMode::External
            }),
            tooltip: attribute(&nodes[root], "tooltip").map(str::to_string),
            locator: xml_locator(part, &nodes[root].path),
        });
    }
    for child in &nodes[root].children {
        if !shape_element(local_name(&nodes[*child].name)) {
            collect_links(nodes, *child, shape, part, rels, out);
        }
    }
}

fn parse_notes(
    entries: &[PackageEntry],
    rels: &[PresentationRelationship],
    types: &PresentationContentTypes,
    slide: usize,
    slide_part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PresentationNote> {
    rels.iter()
        .filter(|item| {
            item.source_part.as_deref() == Some(slide_part)
                && item
                    .relationship_type
                    .to_ascii_lowercase()
                    .ends_with("/notesslide")
        })
        .filter_map(|item| item.resolved_part.as_deref())
        .filter_map(|part| {
            let bytes = entry_bytes(entries, part)?;
            let nodes = parse_xml_part(part, bytes, diagnostics).ok()?;
            let mut acc = Acc::default();
            if let Some(tree) = find(&nodes, 0, "spTree") {
                collect_shapes(
                    &nodes,
                    tree,
                    None,
                    slide,
                    part,
                    entries,
                    rels,
                    types,
                    diagnostics,
                    &mut acc,
                );
            }
            let text = acc
                .shapes
                .iter()
                .filter_map(|shape| shape.text_body.as_ref())
                .map(|body| body.text.as_str())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            Some(PresentationNote {
                part: part.into(),
                text,
                shapes: acc.shapes,
                tables: acc.tables,
                charts: acc.charts,
                equations: acc.equations,
                images: acc.images,
                links: acc.links,
                embedded_objects: acc.objects,
                locator: part_locator(part).expect("valid notes part"),
            })
        })
        .collect()
}

fn parse_authors(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> HashMap<String, Author> {
    let mut out = HashMap::new();
    for entry in entries.iter().filter(|entry| {
        entry.path.to_ascii_lowercase().contains("author") && entry.path.ends_with(".xml")
    }) {
        let Some(bytes) = entry.bytes.as_deref() else {
            continue;
        };
        let Ok(nodes) = parse_xml_part(&entry.path, bytes, diagnostics) else {
            continue;
        };
        for node in &nodes {
            if matches!(local_name(&node.name), "cmAuthor" | "author" | "person")
                && let Some(id) = attribute(node, "id")
            {
                out.insert(
                    id.into(),
                    Author {
                        name: attribute(node, "name")
                            .or_else(|| attribute(node, "displayName"))
                            .map(str::to_string),
                        initials: attribute(node, "initials").map(str::to_string),
                    },
                );
            }
        }
    }
    out
}

fn parse_comments(
    entries: &[PackageEntry],
    rels: &[PresentationRelationship],
    slide_part: &str,
    authors: &HashMap<String, Author>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PresentationComment> {
    let mut out = Vec::new();
    for part in rels
        .iter()
        .filter(|item| {
            item.source_part.as_deref() == Some(slide_part)
                && item
                    .relationship_type
                    .to_ascii_lowercase()
                    .contains("comment")
        })
        .filter_map(|item| item.resolved_part.as_deref())
    {
        let Some(bytes) = entry_bytes(entries, part) else {
            continue;
        };
        let Ok(nodes) = parse_xml_part(part, bytes, diagnostics) else {
            continue;
        };
        for (index, node) in nodes.iter().enumerate() {
            if !matches!(local_name(&node.name), "cm" | "comment") {
                continue;
            }
            let author_id = attribute(node, "authorId").map(str::to_string);
            let author = author_id.as_ref().and_then(|id| authors.get(id));
            let pos = direct(&nodes, index, "pos");
            out.push(PresentationComment {
                comment_id: attribute(node, "idx")
                    .or_else(|| attribute(node, "id"))
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("xml:{}", node.path)),
                author_id,
                author_name: author.and_then(|a| a.name.clone()),
                author_initials: author.and_then(|a| a.initials.clone()),
                created_at: attribute(node, "dt")
                    .or_else(|| attribute(node, "created"))
                    .map(str::to_string),
                parent_comment_id: attribute(node, "parentId").map(str::to_string),
                x: pos
                    .and_then(|n| attribute(&nodes[n], "x"))
                    .and_then(|v| v.parse().ok()),
                y: pos
                    .and_then(|n| attribute(&nodes[n], "y"))
                    .and_then(|v| v.parse().ok()),
                text: find(&nodes, index, "text")
                    .or_else(|| find(&nodes, index, "t"))
                    .map(|n| descendant_text(&nodes, n))
                    .unwrap_or_default(),
                attributes: node.attributes.clone(),
                locator: xml_locator(part, &node.path),
            });
        }
    }
    out.sort_by(|a, b| a.comment_id.cmp(&b.comment_id));
    out
}

fn parse_transition(nodes: &[XmlNode], part: &str) -> Option<PresentationTransition> {
    let root = find(nodes, 0, "transition")?;
    let kind = nodes[root]
        .children
        .first()
        .map(|child| local_name(&nodes[*child].name).to_string());
    let sound = find(nodes, root, "snd")
        .and_then(|n| attribute(&nodes[n], "r:embed"))
        .or_else(|| find(nodes, root, "snd").and_then(|n| attribute(&nodes[n], "r:link")))
        .map(str::to_string);
    Some(PresentationTransition {
        kind,
        advance_on_click: attribute(&nodes[root], "advClick").map(truthy),
        advance_after_ms: attribute(&nodes[root], "advTm").and_then(|v| v.parse().ok()),
        duration_ms: attribute(&nodes[root], "dur").and_then(|v| v.parse().ok()),
        speed: attribute(&nodes[root], "spd").map(str::to_string),
        sound_relationship_id: sound,
        attributes: nodes[root].attributes.clone(),
        metadata: metadata(nodes, root, part),
        locator: xml_locator(part, &nodes[root].path),
    })
}

fn parse_animations(nodes: &[XmlNode], part: &str) -> Vec<PresentationAnimation> {
    let Some(timing) = find(nodes, 0, "timing") else {
        return Vec::new();
    };
    let mut indexes = Vec::new();
    collect_descendants(nodes, timing, &mut indexes);
    indexes
        .into_iter()
        .enumerate()
        .map(|(index, node)| {
            let mut targets = BTreeSet::new();
            target_ids(nodes, node, &mut targets);
            let relationships = nodes[node]
                .attributes
                .iter()
                .filter(|(name, _)| matches!(name.as_str(), "r:id" | "r:embed" | "r:link"))
                .map(|(_, value)| value.clone())
                .collect();
            PresentationAnimation {
                index,
                element: nodes[node].name.clone(),
                target_shape_ids: targets.into_iter().collect(),
                relationship_ids: relationships,
                attributes: nodes[node].attributes.clone(),
                text: (!nodes[node].text.trim().is_empty()).then(|| nodes[node].text.trim().into()),
                locator: xml_locator(part, &nodes[node].path),
            }
        })
        .collect()
}

fn collect_descendants(nodes: &[XmlNode], root: usize, out: &mut Vec<usize>) {
    for child in &nodes[root].children {
        out.push(*child);
        collect_descendants(nodes, *child, out);
    }
}

fn target_ids(nodes: &[XmlNode], root: usize, out: &mut BTreeSet<String>) {
    if local_name(&nodes[root].name) == "spTgt"
        && let Some(id) = attribute(&nodes[root], "spid")
    {
        out.insert(id.into());
    }
    for child in &nodes[root].children {
        target_ids(nodes, *child, out);
    }
}

fn reading_order(shapes: &[PresentationShape]) -> PresentationReadingOrder {
    let mut ordered = shapes.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| reading_key(a).cmp(&reading_key(b)));
    let entries = ordered
        .into_iter()
        .enumerate()
        .map(|(rank, shape)| {
            let title = shape.placeholder_type.as_deref().is_some_and(|value| {
                matches!(value.to_ascii_lowercase().as_str(), "title" | "ctrtitle")
            });
            let positioned = shape
                .geometry
                .as_ref()
                .is_some_and(|g| g.x.is_some() && g.y.is_some());
            let (confidence, evidence) = if title {
                (
                    0.98,
                    vec![
                        "title_placeholder_precedence".into(),
                        "presentation_geometry".into(),
                    ],
                )
            } else if positioned {
                (
                    0.84,
                    vec![
                        "top_to_bottom_geometry".into(),
                        "left_to_right_geometry".into(),
                        "shape_tree_tie_break".into(),
                    ],
                )
            } else {
                (0.55, vec!["shape_tree_fallback_missing_geometry".into()])
            };
            PresentationReadingOrderEntry {
                rank,
                shape_id: shape.shape_id.clone(),
                source_z_order: shape.z_order,
                confidence,
                evidence,
                locator: shape.locator.clone(),
            }
        })
        .collect::<Vec<_>>();
    let confidence = if entries.is_empty() {
        1.0
    } else {
        entries.iter().map(|entry| entry.confidence).sum::<f64>() / entries.len() as f64
    };
    PresentationReadingOrder {
        method: "title_then_top_to_bottom_left_to_right_v1".into(),
        confidence,
        entries,
    }
}

fn reading_key(shape: &PresentationShape) -> (u8, u8, i64, i64, usize, &str) {
    let title = shape
        .placeholder_type
        .as_deref()
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "title" | "ctrtitle"));
    let geometry = shape.geometry.as_ref();
    let positioned = geometry.is_some_and(|g| g.x.is_some() && g.y.is_some());
    (
        u8::from(!title),
        u8::from(!positioned),
        geometry.and_then(|g| g.y).unwrap_or(i64::MAX),
        geometry.and_then(|g| g.x).unwrap_or(i64::MAX),
        shape.z_order,
        &shape.shape_id,
    )
}
