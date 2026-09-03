use super::model::*;
use super::package::{PackageEntry, available, relationship};
use super::xml::{attr, descendant_text, descendants, local_name, parse_bool, parse_xml_part};
use super::{PARSER, cell_locator, part_locator, xml_locator};
use crate::core::Diagnostic;
use crate::registry::{ParserContext, ParserError};
use crate::security::{XmlSecurityPolicy, inspect_xml};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::BTreeMap;

pub(super) struct WorkbookData {
    pub date_system: SpreadsheetDateSystem,
    pub calculation: SpreadsheetCalculationProperties,
    pub sheets: Vec<SpreadsheetSheet>,
    pub named_ranges: Vec<SpreadsheetNamedRange>,
    pub styles: SpreadsheetStyles,
}

pub(super) fn parse_workbook(
    entries: &[PackageEntry],
    relationships: &[SpreadsheetRelationship],
    workbook_part: &str,
    context: &ParserContext<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<WorkbookData, ParserError> {
    let entry = available(entries, workbook_part).ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            format!("declared workbook part {workbook_part} is missing or unavailable"),
        ))
    })?;
    let nodes = parse_xml_part(
        workbook_part,
        entry.bytes.as_deref().unwrap_or_default(),
        diagnostics,
    )
    .map_err(|_| Box::new(Diagnostic::malformed(PARSER, "cannot parse workbook XML")))?;
    let _root = nodes
        .first()
        .filter(|node| local_name(&node.name) == "workbook")
        .ok_or_else(|| {
            Box::new(Diagnostic::malformed(
                PARSER,
                "workbook part has no workbook root",
            ))
        })?;
    let date_system = descendants(&nodes, 0, "workbookPr")
        .next()
        .and_then(|index| parse_bool(attr(&nodes[index], "date1904")))
        .filter(|value| *value)
        .map_or(SpreadsheetDateSystem::Excel1900, |_| {
            SpreadsheetDateSystem::Excel1904
        });
    let calculation = descendants(&nodes, 0, "calcPr").next().map_or_else(
        SpreadsheetCalculationProperties::default,
        |index| {
            let node = &nodes[index];
            SpreadsheetCalculationProperties {
                calculation_id: attr(node, "calcId").map(str::to_string),
                mode: attr(node, "calcMode").map(str::to_string),
                full_calculation_on_load: parse_bool(attr(node, "fullCalcOnLoad")),
                force_full_calculation: parse_bool(attr(node, "forceFullCalc")),
                formulas_calculated_by_grist: false,
            }
        },
    );
    let named_ranges = descendants(&nodes, 0, "definedName")
        .map(|index| {
            let node = &nodes[index];
            SpreadsheetNamedRange {
                name: attr(node, "name").unwrap_or_default().to_string(),
                formula: descendant_text(&nodes, index),
                local_sheet_id: attr(node, "localSheetId").and_then(|v| v.parse().ok()),
                hidden: parse_bool(attr(node, "hidden")).unwrap_or(false),
                comment: attr(node, "comment").map(str::to_string),
                locator: xml_locator(workbook_part, &node.path),
            }
        })
        .collect();
    let shared_strings = parse_shared_strings(entries, relationships, workbook_part, diagnostics);
    let styles = parse_styles(entries, relationships, workbook_part, diagnostics);
    let mut sheets = Vec::new();
    for (order, index) in descendants(&nodes, 0, "sheet").enumerate() {
        context.checkpoint()?;
        let node = &nodes[index];
        let id = attr(node, "id").unwrap_or_default();
        let part = relationship(relationships, Some(workbook_part), id)
            .and_then(|item| item.resolved_part.clone());
        let name = attr(node, "name").unwrap_or_default().to_string();
        let visibility = match attr(node, "state").unwrap_or("visible") {
            "visible" => SpreadsheetVisibility::Visible,
            "hidden" => SpreadsheetVisibility::Hidden,
            "veryHidden" => SpreadsheetVisibility::VeryHidden,
            _ => SpreadsheetVisibility::Unknown,
        };
        let locator = part
            .as_deref()
            .map(part_locator)
            .unwrap_or_else(|| xml_locator(workbook_part, &node.path));
        let mut sheet = SpreadsheetSheet {
            order,
            sheet_id: attr(node, "sheetId").unwrap_or_default().to_string(),
            name,
            visibility,
            part: part.clone(),
            dimension: None,
            pane: None,
            selections: Vec::new(),
            columns: Vec::new(),
            rows: Vec::new(),
            merges: Vec::new(),
            hyperlinks: Vec::new(),
            comments: Vec::new(),
            tables: Vec::new(),
            objects: Vec::new(),
            locator,
        };
        if let Some(part) = part {
            if let Some(sheet_entry) = available(entries, &part) {
                parse_sheet_xml(
                    &mut sheet,
                    &part,
                    sheet_entry.bytes.as_deref().unwrap_or_default(),
                    &SheetResources {
                        shared_strings: &shared_strings,
                        styles: &styles,
                        relationships,
                    },
                    context,
                    diagnostics,
                )?;
                attach_related_objects(&mut sheet, &part, entries, relationships, diagnostics);
            } else {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "spreadsheet_ooxml.sheet.missing_part",
                        format!("sheet {} targets missing part {part}", sheet.name),
                    )
                    .with_locator(sheet.locator.clone())
                    .partial(),
                );
            }
        }
        sheets.push(sheet);
    }
    Ok(WorkbookData {
        date_system,
        calculation,
        sheets,
        named_ranges,
        styles,
    })
}

fn parse_shared_strings(
    entries: &[PackageEntry],
    relationships: &[SpreadsheetRelationship],
    workbook_part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<String> {
    let part = relationships
        .iter()
        .find(|item| {
            item.source_part.as_deref() == Some(workbook_part)
                && item.relationship_type.ends_with("/sharedStrings")
        })
        .and_then(|item| item.resolved_part.as_deref())
        .unwrap_or("xl/sharedStrings.xml");
    let Some(entry) = available(entries, part) else {
        return Vec::new();
    };
    let Ok(nodes) = parse_xml_part(
        part,
        entry.bytes.as_deref().unwrap_or_default(),
        diagnostics,
    ) else {
        return Vec::new();
    };
    descendants(&nodes, 0, "si")
        .map(|index| descendant_text(&nodes, index))
        .collect()
}

pub(super) fn parse_styles(
    entries: &[PackageEntry],
    relationships: &[SpreadsheetRelationship],
    workbook_part: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> SpreadsheetStyles {
    let part = relationships
        .iter()
        .find(|item| {
            item.source_part.as_deref() == Some(workbook_part)
                && item.relationship_type.ends_with("/styles")
        })
        .and_then(|item| item.resolved_part.as_deref())
        .unwrap_or("xl/styles.xml");
    let Some(entry) = available(entries, part) else {
        return SpreadsheetStyles::default();
    };
    let Ok(nodes) = parse_xml_part(
        part,
        entry.bytes.as_deref().unwrap_or_default(),
        diagnostics,
    ) else {
        return SpreadsheetStyles::default();
    };
    let mut number_formats = BTreeMap::new();
    for index in descendants(&nodes, 0, "numFmt") {
        if let (Some(id), Some(code)) = (
            attr(&nodes[index], "numFmtId").and_then(|v| v.parse().ok()),
            attr(&nodes[index], "formatCode"),
        ) {
            number_formats.insert(id, code.to_string());
        }
    }
    let records = |container: &str, item: &str| -> Vec<SpreadsheetStyleRecord> {
        descendants(&nodes, 0, container)
            .next()
            .into_iter()
            .flat_map(|container_index| nodes[container_index].children.iter())
            .filter(|index| local_name(&nodes[**index].name) == item)
            .enumerate()
            .map(|(position, index)| {
                let node = &nodes[*index];
                let values = descendants(&nodes, *index, "name")
                    .chain(descendants(&nodes, *index, "color"))
                    .chain(descendants(&nodes, *index, "patternFill"))
                    .map(|child| {
                        (
                            local_name(&nodes[child].name).to_string(),
                            attr(&nodes[child], "val")
                                .or_else(|| attr(&nodes[child], "rgb"))
                                .or_else(|| attr(&nodes[child], "patternType"))
                                .unwrap_or_default()
                                .to_string(),
                        )
                    })
                    .collect();
                SpreadsheetStyleRecord {
                    index: position as u32,
                    attributes: node.attributes.clone(),
                    values,
                    locator: xml_locator(part, &node.path),
                }
            })
            .collect()
    };
    let cell_formats = descendants(&nodes, 0, "cellXfs")
        .next()
        .into_iter()
        .flat_map(|index| nodes[index].children.iter())
        .filter(|index| local_name(&nodes[**index].name) == "xf")
        .enumerate()
        .map(|(style_id, index)| {
            let node = &nodes[*index];
            let alignment = node
                .children
                .iter()
                .find(|child| local_name(&nodes[**child].name) == "alignment")
                .map(|child| &nodes[*child]);
            let protection = node
                .children
                .iter()
                .find(|child| local_name(&nodes[**child].name) == "protection")
                .map(|child| &nodes[*child]);
            let num_fmt = attr(node, "numFmtId").and_then(|value| value.parse().ok());
            SpreadsheetCellStyle {
                style_id: style_id as u32,
                number_format_id: num_fmt,
                number_format_code: num_fmt.and_then(|id| number_formats.get(&id).cloned()),
                font_id: attr(node, "fontId").and_then(|v| v.parse().ok()),
                fill_id: attr(node, "fillId").and_then(|v| v.parse().ok()),
                border_id: attr(node, "borderId").and_then(|v| v.parse().ok()),
                horizontal_alignment: alignment
                    .and_then(|n| attr(n, "horizontal"))
                    .map(str::to_string),
                vertical_alignment: alignment
                    .and_then(|n| attr(n, "vertical"))
                    .map(str::to_string),
                wrap_text: alignment.and_then(|n| parse_bool(attr(n, "wrapText"))),
                text_rotation: alignment
                    .and_then(|n| attr(n, "textRotation"))
                    .and_then(|v| v.parse().ok()),
                locked: protection.and_then(|n| parse_bool(attr(n, "locked"))),
                hidden_formula: protection.and_then(|n| parse_bool(attr(n, "hidden"))),
            }
        })
        .collect();
    SpreadsheetStyles {
        number_formats,
        cell_formats,
        fonts: records("fonts", "font"),
        fills: records("fills", "fill"),
        borders: records("borders", "border"),
    }
}

#[derive(Default)]
struct CellBuilder {
    reference: String,
    cell_type: Option<String>,
    style_id: Option<u32>,
    value: String,
    inline_text: String,
    formula: String,
    formula_type: Option<String>,
    formula_ref: Option<String>,
    shared_index: Option<u32>,
    in_value: bool,
    in_formula: bool,
    in_inline_text: bool,
}

struct SheetResources<'a> {
    shared_strings: &'a [String],
    styles: &'a SpreadsheetStyles,
    relationships: &'a [SpreadsheetRelationship],
}

fn parse_sheet_xml(
    sheet: &mut SpreadsheetSheet,
    part: &str,
    bytes: &[u8],
    resources: &SheetResources<'_>,
    context: &ParserContext<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), ParserError> {
    let findings = inspect_xml(bytes, &XmlSecurityPolicy::default());
    if !findings.is_empty() {
        diagnostics.extend(findings.into_iter().map(|finding| {
            Diagnostic::warning(PARSER, finding.code, finding.message)
                .with_locator(part_locator(part))
                .partial()
        }));
        return Ok(());
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut row = None::<SpreadsheetRow>;
    let mut cell = None::<CellBuilder>;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => match local_event_name(&start).as_str() {
                "row" => row = Some(row_from(&start, &sheet.name)),
                "c" => cell = Some(cell_from(&start)),
                "v" => {
                    if let Some(cell) = cell.as_mut() {
                        cell.in_value = true;
                    }
                }
                "f" => {
                    if let Some(cell) = cell.as_mut() {
                        cell.in_formula = true;
                        cell.formula_type = event_attr(&start, "t");
                        cell.formula_ref = event_attr(&start, "ref");
                        cell.shared_index =
                            event_attr(&start, "si").and_then(|value| value.parse().ok());
                    }
                }
                "t" => {
                    if let Some(cell) = cell.as_mut() {
                        cell.in_inline_text = true;
                    }
                }
                _ => apply_sheet_element(sheet, part, &start, resources.relationships),
            },
            Ok(Event::Empty(start)) => match local_event_name(&start).as_str() {
                "row" => sheet.rows.push(row_from(&start, &sheet.name)),
                "c" => {
                    context.consume_cells(1)?;
                    let built = build_cell(
                        cell_from(&start),
                        &sheet.name,
                        resources.shared_strings,
                        resources.styles,
                    );
                    if row.is_none() {
                        row = Some(empty_row(built.row, &sheet.name));
                    }
                    row.as_mut().unwrap().cells.push(built);
                }
                _ => apply_sheet_element(sheet, part, &start, resources.relationships),
            },
            Ok(Event::Text(text)) => {
                if let Some(cell) = cell.as_mut() {
                    append_cell_text(cell, &decode_text(&String::from_utf8_lossy(text.as_ref())));
                }
            }
            Ok(Event::CData(text)) => {
                if let Some(cell) = cell.as_mut() {
                    append_cell_text(cell, &String::from_utf8_lossy(text.as_ref()));
                }
            }
            Ok(Event::End(end)) => {
                match local_name(&String::from_utf8_lossy(end.name().as_ref())) {
                    "v" => {
                        if let Some(cell) = cell.as_mut() {
                            cell.in_value = false;
                        }
                    }
                    "f" => {
                        if let Some(cell) = cell.as_mut() {
                            cell.in_formula = false;
                        }
                    }
                    "t" => {
                        if let Some(cell) = cell.as_mut() {
                            cell.in_inline_text = false;
                        }
                    }
                    "c" => {
                        if let Some(builder) = cell.take() {
                            context.consume_cells(1)?;
                            let built = build_cell(
                                builder,
                                &sheet.name,
                                resources.shared_strings,
                                resources.styles,
                            );
                            if row.is_none() {
                                row = Some(empty_row(built.row, &sheet.name));
                            }
                            row.as_mut().unwrap().cells.push(built);
                        }
                    }
                    "row" => {
                        if let Some(mut value) = row.take() {
                            value.cells.sort_by_key(|cell| cell.column);
                            sheet.rows.push(value);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                diagnostics.push(
                    Diagnostic::malformed(
                        PARSER,
                        format!("malformed worksheet XML in {part}: {error}"),
                    )
                    .with_locator(part_locator(part))
                    .partial(),
                );
                break;
            }
            _ => {}
        }
    }
    if let Some(mut value) = row {
        value.cells.sort_by_key(|cell| cell.column);
        sheet.rows.push(value);
    }
    sheet.rows.sort_by_key(|row| row.row);
    context.consume_nodes(
        sheet
            .rows
            .iter()
            .map(|row| row.cells.len().saturating_add(1))
            .sum::<usize>() as u64,
    )?;
    Ok(())
}

fn append_cell_text(cell: &mut CellBuilder, value: &str) {
    if cell.in_value {
        cell.value.push_str(value);
    }
    if cell.in_formula {
        cell.formula.push_str(value);
    }
    if cell.in_inline_text {
        cell.inline_text.push_str(value);
    }
}

fn local_event_name(start: &BytesStart<'_>) -> String {
    local_name(&String::from_utf8_lossy(start.name().as_ref())).to_string()
}

fn event_attr(start: &BytesStart<'_>, name: &str) -> Option<String> {
    start
        .attributes()
        .with_checks(false)
        .flatten()
        .find(|item| local_name(&String::from_utf8_lossy(item.key.as_ref())) == name)
        .map(|item| decode_text(&String::from_utf8_lossy(item.value.as_ref())))
}

fn row_from(start: &BytesStart<'_>, sheet: &str) -> SpreadsheetRow {
    let row = event_attr(start, "r")
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    SpreadsheetRow {
        row,
        height: event_attr(start, "ht").and_then(|value| value.parse().ok()),
        style_id: event_attr(start, "s").and_then(|value| value.parse().ok()),
        hidden: event_attr(start, "hidden")
            .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true")),
        outline_level: event_attr(start, "outlineLevel").and_then(|value| value.parse().ok()),
        cells: Vec::new(),
        locator: row_locator(sheet, row),
    }
}

fn empty_row(row: u32, sheet: &str) -> SpreadsheetRow {
    SpreadsheetRow {
        row,
        height: None,
        style_id: None,
        hidden: false,
        outline_level: None,
        cells: Vec::new(),
        locator: row_locator(sheet, row),
    }
}

fn cell_from(start: &BytesStart<'_>) -> CellBuilder {
    CellBuilder {
        reference: event_attr(start, "r").unwrap_or_default(),
        cell_type: event_attr(start, "t"),
        style_id: event_attr(start, "s").and_then(|value| value.parse().ok()),
        ..CellBuilder::default()
    }
}

fn build_cell(
    builder: CellBuilder,
    sheet: &str,
    shared_strings: &[String],
    styles: &SpreadsheetStyles,
) -> SpreadsheetCell {
    let (row, column) = parse_cell_reference(&builder.reference).unwrap_or((1, 1));
    let cell_type = match builder.cell_type.as_deref() {
        None | Some("n") => SpreadsheetCellType::Number,
        Some("s") => SpreadsheetCellType::SharedString,
        Some("inlineStr") => SpreadsheetCellType::InlineString,
        Some("str") => SpreadsheetCellType::String,
        Some("b") => SpreadsheetCellType::Boolean,
        Some("e") => SpreadsheetCellType::Error,
        Some("d") => SpreadsheetCellType::Date,
        Some("") => SpreadsheetCellType::Blank,
        _ => SpreadsheetCellType::Unknown,
    };
    let stored = if builder.value.is_empty() {
        (!builder.inline_text.is_empty()).then(|| builder.inline_text.clone())
    } else {
        Some(builder.value.clone())
    };
    let displayed = match cell_type {
        SpreadsheetCellType::SharedString => builder
            .value
            .parse::<usize>()
            .ok()
            .and_then(|index| shared_strings.get(index).cloned()),
        SpreadsheetCellType::InlineString => {
            (!builder.inline_text.is_empty()).then_some(builder.inline_text.clone())
        }
        SpreadsheetCellType::Boolean => builder
            .value
            .parse::<u8>()
            .ok()
            .map(|value| if value == 0 { "false" } else { "true" }.into()),
        _ => stored.clone(),
    };
    let locator = cell_locator(sheet, &builder.reference);
    let formula = (!builder.formula.is_empty() || builder.formula_type.is_some()).then(|| {
        SpreadsheetFormula {
            source: builder.formula,
            formula_type: builder.formula_type,
            reference: builder.formula_ref,
            shared_index: builder.shared_index,
            calculate: false,
            locator: locator.clone(),
        }
    });
    let cached_value = formula
        .as_ref()
        .and_then(|_| stored.clone())
        .map(|stored_value| SpreadsheetCachedValue {
            stored_value,
            displayed_value: displayed.clone(),
            source: SpreadsheetCachedValueSource::WorkbookStoredFormulaCache,
        });
    let style = builder
        .style_id
        .and_then(|id| styles.cell_formats.get(id as usize).cloned());
    SpreadsheetCell {
        reference: builder.reference,
        row,
        column,
        cell_type,
        stored_value: stored,
        displayed_value: displayed,
        formula,
        cached_value,
        style_id: builder.style_id,
        style,
        locator,
    }
}

fn apply_sheet_element(
    sheet: &mut SpreadsheetSheet,
    part: &str,
    start: &BytesStart<'_>,
    relationships: &[SpreadsheetRelationship],
) {
    match local_event_name(start).as_str() {
        "dimension" => sheet.dimension = event_attr(start, "ref"),
        "pane" => {
            sheet.pane = Some(SpreadsheetPane {
                state: event_attr(start, "state"),
                top_left_cell: event_attr(start, "topLeftCell"),
                active_pane: event_attr(start, "activePane"),
                horizontal_split: event_attr(start, "xSplit").and_then(|v| v.parse().ok()),
                vertical_split: event_attr(start, "ySplit").and_then(|v| v.parse().ok()),
                locator: part_locator(part),
            })
        }
        "selection" => sheet.selections.push(SpreadsheetSelection {
            pane: event_attr(start, "pane"),
            active_cell: event_attr(start, "activeCell"),
            ranges: event_attr(start, "sqref"),
            locator: part_locator(part),
        }),
        "col" => sheet.columns.push(SpreadsheetColumn {
            min: event_attr(start, "min")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
            max: event_attr(start, "max")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
            width: event_attr(start, "width").and_then(|v| v.parse().ok()),
            style_id: event_attr(start, "style").and_then(|v| v.parse().ok()),
            hidden: event_attr(start, "hidden").as_deref() == Some("1"),
            outline_level: event_attr(start, "outlineLevel").and_then(|v| v.parse().ok()),
            locator: part_locator(part),
        }),
        "mergeCell" => {
            if let Some(range) = event_attr(start, "ref") {
                sheet.merges.push(SpreadsheetMerge {
                    locator: range_locator(&sheet.name, &range),
                    range,
                });
            }
        }
        "hyperlink" => {
            if let Some(range) = event_attr(start, "ref") {
                let rel = event_attr(start, "id")
                    .and_then(|id| relationship(relationships, Some(part), &id));
                sheet.hyperlinks.push(SpreadsheetHyperlink {
                    range: range.clone(),
                    target: rel.map(|item| item.target.clone()),
                    location: event_attr(start, "location"),
                    display: event_attr(start, "display"),
                    tooltip: event_attr(start, "tooltip"),
                    external: rel.is_some_and(|item| {
                        item.target_mode == SpreadsheetRelationshipTargetMode::External
                    }),
                    locator: range_locator(&sheet.name, &range),
                });
            }
        }
        _ => {}
    }
}

fn decode_text(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn attach_related_objects(
    sheet: &mut SpreadsheetSheet,
    sheet_part: &str,
    entries: &[PackageEntry],
    relationships: &[SpreadsheetRelationship],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for rel in relationships
        .iter()
        .filter(|item| item.source_part.as_deref() == Some(sheet_part))
    {
        let Some(part) = rel.resolved_part.as_deref() else {
            continue;
        };
        if rel.relationship_type.ends_with("/comments") {
            sheet
                .comments
                .extend(parse_comments(part, entries, diagnostics));
        } else if rel.relationship_type.ends_with("/table") {
            if let Some(table) = parse_table(part, entries, diagnostics) {
                sheet.tables.push(table);
            }
        } else if rel.relationship_type.ends_with("/drawing") {
            sheet
                .objects
                .extend(parse_drawing(part, entries, relationships, diagnostics));
        } else if rel.relationship_type.ends_with("/oleObject")
            || rel.relationship_type.ends_with("/package")
        {
            sheet.objects.push(object_from_part(
                SpreadsheetObjectKind::EmbeddedObject,
                part,
                None,
                entries,
            ));
        }
    }
}

fn parse_comments(
    part: &str,
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SpreadsheetComment> {
    let Some(entry) = available(entries, part) else {
        return Vec::new();
    };
    let Ok(nodes) = parse_xml_part(
        part,
        entry.bytes.as_deref().unwrap_or_default(),
        diagnostics,
    ) else {
        return Vec::new();
    };
    let authors = descendants(&nodes, 0, "author")
        .map(|index| descendant_text(&nodes, index))
        .collect::<Vec<_>>();
    descendants(&nodes, 0, "comment")
        .map(|index| {
            let node = &nodes[index];
            let reference = attr(node, "ref").unwrap_or_default().to_string();
            let author = attr(node, "authorId")
                .and_then(|value| value.parse::<usize>().ok())
                .and_then(|author| authors.get(author).cloned());
            SpreadsheetComment {
                reference: reference.clone(),
                author,
                text: descendant_text(&nodes, index),
                locator: xml_locator(part, &node.path),
            }
        })
        .collect()
}

fn parse_table(
    part: &str,
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SpreadsheetTable> {
    let entry = available(entries, part)?;
    let nodes = parse_xml_part(
        part,
        entry.bytes.as_deref().unwrap_or_default(),
        diagnostics,
    )
    .ok()?;
    let root = nodes
        .first()
        .filter(|node| local_name(&node.name) == "table")?;
    let columns = descendants(&nodes, 0, "tableColumn")
        .map(|index| {
            let node = &nodes[index];
            let formula = descendants(&nodes, index, "calculatedColumnFormula")
                .next()
                .map(|child| descendant_text(&nodes, child));
            SpreadsheetTableColumn {
                id: attr(node, "id").and_then(|v| v.parse().ok()),
                name: attr(node, "name").map(str::to_string),
                totals_row_function: attr(node, "totalsRowFunction").map(str::to_string),
                calculated_column_formula: formula,
            }
        })
        .collect();
    let style_name = descendants(&nodes, 0, "tableStyleInfo")
        .next()
        .and_then(|index| attr(&nodes[index], "name"))
        .map(str::to_string);
    Some(SpreadsheetTable {
        id: attr(root, "id").and_then(|v| v.parse().ok()),
        name: attr(root, "name").map(str::to_string),
        display_name: attr(root, "displayName").map(str::to_string),
        range: attr(root, "ref").unwrap_or_default().to_string(),
        header_rows: attr(root, "headerRowCount")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
        totals_rows: attr(root, "totalsRowCount")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        columns,
        style_name,
        part: part.to_string(),
        locator: part_locator(part),
    })
}

fn parse_drawing(
    part: &str,
    entries: &[PackageEntry],
    relationships: &[SpreadsheetRelationship],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SpreadsheetObject> {
    let Some(entry) = available(entries, part) else {
        return Vec::new();
    };
    let Ok(nodes) = parse_xml_part(
        part,
        entry.bytes.as_deref().unwrap_or_default(),
        diagnostics,
    ) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for anchor in descendants(&nodes, 0, "twoCellAnchor")
        .chain(descendants(&nodes, 0, "oneCellAnchor"))
        .chain(descendants(&nodes, 0, "absoluteAnchor"))
    {
        let from = anchor_cell(&nodes, anchor, "from");
        let to = anchor_cell(&nodes, anchor, "to");
        let name = descendants(&nodes, anchor, "cNvPr")
            .next()
            .and_then(|index| attr(&nodes[index], "name"))
            .map(str::to_string);
        let ids = descendants(&nodes, anchor, "blip")
            .filter_map(|index| {
                attr(&nodes[index], "embed").or_else(|| attr(&nodes[index], "link"))
            })
            .chain(
                descendants(&nodes, anchor, "chart").filter_map(|index| attr(&nodes[index], "id")),
            )
            .collect::<Vec<_>>();
        for id in ids {
            let Some(rel) = relationship(relationships, Some(part), id) else {
                continue;
            };
            let kind = if rel.relationship_type.ends_with("/image") {
                SpreadsheetObjectKind::Image
            } else if rel.relationship_type.ends_with("/chart") {
                SpreadsheetObjectKind::Chart
            } else {
                SpreadsheetObjectKind::Unknown
            };
            let mut object = rel.resolved_part.as_deref().map_or_else(
                || object_from_part(kind, part, name.clone(), entries),
                |target| object_from_part(kind, target, name.clone(), entries),
            );
            object.anchor_from = from.clone();
            object.anchor_to = to.clone();
            if kind == SpreadsheetObjectKind::Chart {
                if let Some(target) = rel.resolved_part.as_deref() {
                    populate_chart(&mut object, target, entries, diagnostics);
                }
            }
            output.push(object);
        }
    }
    if output.is_empty() {
        output.push(object_from_part(
            SpreadsheetObjectKind::Drawing,
            part,
            None,
            entries,
        ));
    }
    output
}

fn anchor_cell(nodes: &[super::xml::XmlNode], anchor: usize, endpoint: &str) -> Option<String> {
    let endpoint = descendants(nodes, anchor, endpoint).next()?;
    let column = descendants(nodes, endpoint, "col")
        .next()
        .and_then(|index| descendant_text(nodes, index).parse::<u32>().ok())?;
    let row = descendants(nodes, endpoint, "row")
        .next()
        .and_then(|index| descendant_text(nodes, index).parse::<u32>().ok())?;
    Some(format!(
        "{}{}",
        column_name(column.saturating_add(1)),
        row.saturating_add(1)
    ))
}

fn object_from_part(
    kind: SpreadsheetObjectKind,
    part: &str,
    name: Option<String>,
    entries: &[PackageEntry],
) -> SpreadsheetObject {
    let entry = available(entries, part);
    SpreadsheetObject {
        kind,
        name,
        part: Some(part.to_string()),
        content_type: None,
        identity: entry
            .and_then(|entry| entry.bytes.as_deref())
            .map(crate::core::ContentIdentity::for_raw_bytes),
        anchor_from: None,
        anchor_to: None,
        title: None,
        series_formulas: Vec::new(),
        cached_values: Vec::new(),
        locator: part_locator(part),
    }
}

fn populate_chart(
    object: &mut SpreadsheetObject,
    part: &str,
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(entry) = available(entries, part) else {
        return;
    };
    let Ok(nodes) = parse_xml_part(
        part,
        entry.bytes.as_deref().unwrap_or_default(),
        diagnostics,
    ) else {
        return;
    };
    object.title = descendants(&nodes, 0, "title")
        .next()
        .map(|index| descendant_text(&nodes, index))
        .filter(|value| !value.is_empty());
    object.series_formulas = descendants(&nodes, 0, "f")
        .map(|index| descendant_text(&nodes, index))
        .filter(|value| !value.is_empty())
        .collect();
    object.cached_values = descendants(&nodes, 0, "numCache")
        .chain(descendants(&nodes, 0, "strCache"))
        .flat_map(|index| descendants(&nodes, index, "v"))
        .map(|index| descendant_text(&nodes, index))
        .collect();
}

fn parse_cell_reference(reference: &str) -> Option<(u32, u32)> {
    let reference = reference.trim_matches('$');
    let split = reference.find(|character: char| character.is_ascii_digit())?;
    let (letters, digits) = reference.split_at(split);
    if letters.is_empty() || digits.is_empty() {
        return None;
    }
    let column = letters.chars().try_fold(0u32, |value, letter| {
        let letter = letter.to_ascii_uppercase();
        letter.is_ascii_uppercase().then(|| {
            value
                .saturating_mul(26)
                .saturating_add(u32::from(letter) - u32::from('A') + 1)
        })
    })?;
    Some((digits.parse().ok()?, column))
}

fn column_name(mut column: u32) -> String {
    let mut output = String::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        output.insert(0, char::from_u32(u32::from('A') + remainder).unwrap());
        column = (column - 1) / 26;
    }
    output
}

fn row_locator(sheet: &str, row: u32) -> crate::core::SourceLocator {
    range_locator(sheet, &format!("A{row}:XFD{row}"))
}

fn range_locator(sheet: &str, range: &str) -> crate::core::SourceLocator {
    let mut parts = range.split(':');
    let start = parts
        .next()
        .and_then(parse_cell_reference)
        .unwrap_or((1, 1));
    let end = parts.next().and_then(parse_cell_reference).unwrap_or(start);
    super::sheet_locator(sheet, start, end)
}
