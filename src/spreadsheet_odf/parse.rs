//! OpenDocument spreadsheet package interpretation.

use super::archive::{PackageEntry, find_entry};
use super::model::*;
use super::xml::{
    XmlContent, XmlDocument, XmlElement, attr, descendant_text, element_locator, local_name,
    parse_xml, raw_xml,
};
use super::{PARSER, cell_reference, member_locator, sheet_locator};
use crate::core::{ContentIdentity, Diagnostic, OperationStatus, SchemaVersion};
use crate::registry::{ParserContext, ParserError, ParserOutput};
use crate::security::ArchiveEntryKind;
use std::collections::{BTreeMap, HashMap};

const CONTENT: &str = "content.xml";
const STYLES: &str = "styles.xml";
const META: &str = "meta.xml";
const SETTINGS: &str = "settings.xml";
const MANIFEST: &str = "META-INF/manifest.xml";

type ParsedWorkbook = (
    SpreadsheetOdfCalculation,
    Vec<SpreadsheetOdfNamedRange>,
    Vec<SpreadsheetOdfSheet>,
    Vec<SpreadsheetOdfRawElement>,
);

pub(super) fn parse_registered(
    context: &mut ParserContext<'_>,
    expected: SpreadsheetOdfPackageKind,
) -> Result<ParserOutput, ParserError> {
    let options: SpreadsheetOdfOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(PARSER, error.to_string())))?;
    let mut package = super::archive::read_package(context.bytes(), context)?;
    let mut diagnostics = std::mem::take(&mut package.diagnostics);
    super::xml::observe_xml_nesting(&package.entries, context)?;
    if package
        .entries
        .iter()
        .any(|entry| entry.encrypted && matches!(entry.path.as_str(), "mimetype" | CONTENT))
    {
        return Ok(encrypted_output(
            "an OpenDocument spreadsheet core member is encrypted",
        ));
    }
    validate_mimetype(&package.entries, expected)?;
    let manifest = parse_manifest(&package.entries, &mut diagnostics);
    if manifest
        .iter()
        .any(|entry| entry.full_path == CONTENT && entry.encrypted)
    {
        return Ok(encrypted_output(
            "manifest declares content.xml as encrypted",
        ));
    }
    for item in manifest
        .iter()
        .filter(|item| item.encrypted && item.full_path != CONTENT)
    {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "spreadsheet_odf.encrypted.member",
                format!(
                    "encrypted member {} is inventoried but unavailable",
                    item.full_path
                ),
            )
            .with_locator(item.locator.clone())
            .partial(),
        );
    }
    let content_entry = find_entry(&package.entries, CONTENT).ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "OpenDocument spreadsheet has no safe content.xml member",
        )) as ParserError
    })?;
    let content = parse_xml(content_entry, &mut diagnostics).ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "content.xml is unavailable or malformed",
        )) as ParserError
    })?;
    let version = content
        .nodes
        .first()
        .and_then(|node| attr(node, "version"))
        .map(str::to_string);
    let styles_document =
        find_entry(&package.entries, STYLES).and_then(|entry| parse_xml(entry, &mut diagnostics));
    let styles = parse_styles(&package.entries, &content, styles_document.as_ref());
    let style_map = styles
        .iter()
        .map(|style| (style.name.clone(), style.clone()))
        .collect::<HashMap<_, _>>();
    let metadata = parse_metadata(&package.entries, &mut diagnostics);
    let (active_table, panes) = parse_settings(&package.entries, &mut diagnostics);
    let (calculation, named_ranges, sheets, raw_elements) = parse_workbook(
        content_entry,
        &content,
        &package.entries,
        &manifest,
        &style_map,
        context,
        &mut diagnostics,
    )?;
    let all_parts = package_parts(&package.entries, &manifest);
    context.consume_child_artifacts(
        sheets
            .iter()
            .map(|sheet| sheet.objects.len())
            .sum::<usize>() as u64,
    )?;
    context.consume_nodes(document_node_count(
        &sheets,
        named_ranges.len(),
        styles.len(),
        metadata.len(),
        manifest.len(),
        raw_elements.len(),
        if options.include_parts {
            all_parts.len()
        } else {
            0
        },
        panes.len(),
    ))?;
    let document = SpreadsheetOdfDocument {
        schema_version: SchemaVersion::SPREADSHEET_ODF_V1.into(),
        package_kind: expected,
        package_media_type: expected.media_type().into(),
        version,
        workbook_locator: member_locator(content_entry).expect("safe content member"),
        calculation,
        active_table,
        panes,
        sheets,
        named_ranges,
        styles,
        metadata,
        manifest,
        raw_elements,
        parts: if options.include_parts {
            all_parts
        } else {
            Vec::new()
        },
    };
    let value = serde_json::to_value(document)
        .map_err(|error| Box::new(Diagnostic::parser_defect(PARSER, error.to_string())))?;
    if diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        Ok(ParserOutput::partial(Some(value), diagnostics))
    } else {
        let mut output = ParserOutput::complete(value);
        output.diagnostics = diagnostics;
        Ok(output)
    }
}

fn encrypted_output(message: &str) -> ParserOutput {
    ParserOutput::terminal(
        OperationStatus::Encrypted,
        vec![Diagnostic::error(
            PARSER,
            "spreadsheet_odf.encrypted",
            message,
        )],
    )
}

fn validate_mimetype(
    entries: &[PackageEntry],
    expected: SpreadsheetOdfPackageKind,
) -> Result<(), ParserError> {
    let entry = find_entry(entries, "mimetype").ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "package has no safe mimetype member",
        )) as ParserError
    })?;
    let actual = entry
        .bytes
        .as_deref()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(str::trim)
        .unwrap_or_default();
    if actual != expected.media_type() {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            format!(
                "requested {} but mimetype declares {actual:?}",
                expected.format_id()
            ),
        )));
    }
    Ok(())
}

fn parse_manifest(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SpreadsheetOdfManifestEntry> {
    let Some(entry) = find_entry(entries, MANIFEST) else {
        diagnostics.push(Diagnostic::malformed(PARSER, "package has no manifest.xml").partial());
        return Vec::new();
    };
    let Some(document) = parse_xml(entry, diagnostics) else {
        return Vec::new();
    };
    document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| local_name(&node.name) == "file-entry")
        .map(|(index, node)| SpreadsheetOdfManifestEntry {
            full_path: attr(node, "full-path").unwrap_or_default().to_string(),
            media_type: attr(node, "media-type")
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            version: attr(node, "version").map(str::to_string),
            encrypted: descendants(&document, index)
                .any(|child| local_name(&child.name) == "encryption-data"),
            locator: element_locator(entry, &document, node),
        })
        .collect()
}

fn parse_styles(
    entries: &[PackageEntry],
    content: &XmlDocument,
    external: Option<&XmlDocument>,
) -> Vec<SpreadsheetOdfStyle> {
    let mut output = Vec::new();
    for (entry, document) in [
        (find_entry(entries, CONTENT), Some(content)),
        (find_entry(entries, STYLES), external),
    ]
    .into_iter()
    .filter_map(|(entry, document)| entry.zip(document))
    {
        for (index, node) in document
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(local_name(&node.name), "style" | "default-style"))
        {
            let name = attr(node, "name")
                .or_else(|| {
                    attr(node, "family").map(|family| {
                        if local_name(&node.name) == "default-style" {
                            family
                        } else {
                            ""
                        }
                    })
                })
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let mut properties = BTreeMap::new();
            for property in descendants(document, index)
                .filter(|item| local_name(&item.name).ends_with("properties"))
            {
                properties.extend(
                    property
                        .attributes
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            }
            output.push(SpreadsheetOdfStyle {
                name: name.to_string(),
                family: attr(node, "family").map(str::to_string),
                parent_style_name: attr(node, "parent-style-name").map(str::to_string),
                data_style_name: attr(node, "data-style-name").map(str::to_string),
                origin_part: entry.path.clone(),
                properties,
                locator: element_locator(entry, document, node),
            });
        }
    }
    output
}

fn parse_metadata(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SpreadsheetOdfMetadata> {
    let Some(entry) = find_entry(entries, META) else {
        return Vec::new();
    };
    let Some(document) = parse_xml(entry, diagnostics) else {
        return Vec::new();
    };
    let Some(root) = document
        .nodes
        .iter()
        .position(|node| local_name(&node.name) == "meta")
    else {
        return Vec::new();
    };
    child_indexes(&document.nodes[root])
        .map(|index| {
            let node = &document.nodes[index];
            SpreadsheetOdfMetadata {
                name: node.name.clone(),
                value: descendant_text(&document, index).trim().to_string(),
                value_type: attr(node, "value-type").map(str::to_string),
                locator: element_locator(entry, &document, node),
            }
        })
        .collect()
}

fn parse_settings(
    entries: &[PackageEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> (Option<String>, Vec<SpreadsheetOdfPane>) {
    let Some(entry) = find_entry(entries, SETTINGS) else {
        return (None, Vec::new());
    };
    let Some(document) = parse_xml(entry, diagnostics) else {
        return (None, Vec::new());
    };
    let mut values = BTreeMap::<String, (String, crate::core::SourceLocator)>::new();
    for (index, node) in document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| local_name(&node.name) == "config-item")
    {
        if let Some(name) = attr(node, "name") {
            values.insert(
                name.to_string(),
                (
                    descendant_text(&document, index).trim().to_string(),
                    element_locator(entry, &document, node),
                ),
            );
        }
    }
    let active = values.get("ActiveTable").map(|value| value.0.clone());
    let h_mode = values
        .get("HorizontalSplitMode")
        .map(|value| value.0.clone());
    let v_mode = values.get("VerticalSplitMode").map(|value| value.0.clone());
    let locator = values
        .values()
        .next()
        .map(|value| value.1.clone())
        .unwrap_or_else(|| member_locator(entry).expect("safe settings member"));
    let pane = SpreadsheetOdfPane {
        view_name: values.get("ViewId").map(|v| v.0.clone()),
        active_table: active.clone(),
        horizontal_split_mode: h_mode.clone(),
        vertical_split_mode: v_mode.clone(),
        horizontal_split_position: values
            .get("HorizontalSplitPosition")
            .and_then(|v| v.0.parse().ok()),
        vertical_split_position: values
            .get("VerticalSplitPosition")
            .and_then(|v| v.0.parse().ok()),
        frozen: h_mode.as_deref() == Some("2")
            || v_mode.as_deref() == Some("2")
            || h_mode.as_deref() == Some("freeze")
            || v_mode.as_deref() == Some("freeze"),
        locator,
    };
    (
        active,
        (!values.is_empty()).then_some(pane).into_iter().collect(),
    )
}

fn parse_workbook(
    entry: &PackageEntry,
    document: &XmlDocument,
    entries: &[PackageEntry],
    manifest: &[SpreadsheetOdfManifestEntry],
    styles: &HashMap<String, SpreadsheetOdfStyle>,
    context: &ParserContext<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<ParsedWorkbook, ParserError> {
    let spreadsheet = document
        .nodes
        .iter()
        .position(|node| local_name(&node.name) == "spreadsheet")
        .ok_or_else(|| {
            Box::new(Diagnostic::malformed(
                PARSER,
                "content.xml has no office:spreadsheet",
            )) as ParserError
        })?;
    let calculation = descendants(document, spreadsheet)
        .find(|node| local_name(&node.name) == "calculation-settings")
        .map(parse_calculation)
        .unwrap_or_default();
    let named_ranges = descendants(document, spreadsheet)
        .filter(|node| matches!(local_name(&node.name), "named-range" | "named-expression"))
        .map(|node| SpreadsheetOdfNamedRange {
            name: attr(node, "name").unwrap_or_default().to_string(),
            expression: attr(node, "cell-range-address")
                .or_else(|| attr(node, "expression"))
                .unwrap_or_default()
                .to_string(),
            base_cell_address: attr(node, "base-cell-address").map(str::to_string),
            range_usable_as: attr(node, "range-usable-as").map(str::to_string),
            locator: element_locator(entry, document, node),
        })
        .collect();
    let mut sheets = Vec::new();
    for (index, node) in document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| local_name(&node.name) == "table")
        .filter(|(_, node)| attr(node, "name").is_some())
        .enumerate()
    {
        let (_, table) = node;
        sheets.push(parse_sheet(
            index,
            table,
            entry,
            document,
            entries,
            manifest,
            styles,
            context,
            diagnostics,
        )?);
    }
    let raw_elements = child_indexes(&document.nodes[spreadsheet])
        .filter(|index| {
            !matches!(
                local_name(&document.nodes[*index].name),
                "calculation-settings" | "named-expressions" | "table"
            )
        })
        .map(|index| raw_element(entry, document, index))
        .collect();
    Ok((calculation, named_ranges, sheets, raw_elements))
}

fn parse_calculation(node: &XmlElement) -> SpreadsheetOdfCalculation {
    SpreadsheetOdfCalculation {
        case_sensitive: bool_attr(node, "case-sensitive"),
        precision_as_shown: bool_attr(node, "precision-as-shown"),
        search_criteria_must_apply_to_whole_cell: bool_attr(
            node,
            "search-criteria-must-apply-to-whole-cell",
        ),
        automatic_find_labels: bool_attr(node, "automatic-find-labels"),
        null_year: attr(node, "null-year").and_then(|value| value.parse().ok()),
        iteration_enabled: bool_attr(node, "iteration"),
        iteration_steps: attr(node, "iteration-steps").and_then(|value| value.parse().ok()),
        iteration_maximum_difference: attr(node, "iteration-maximum-difference")
            .map(str::to_string),
        formulas_calculated_by_grist: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_sheet(
    order: usize,
    table: &XmlElement,
    entry: &PackageEntry,
    document: &XmlDocument,
    entries: &[PackageEntry],
    manifest: &[SpreadsheetOdfManifestEntry],
    styles: &HashMap<String, SpreadsheetOdfStyle>,
    context: &ParserContext<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<SpreadsheetOdfSheet, ParserError> {
    let name = attr(table, "name").unwrap_or("Sheet").to_string();
    let style_name = attr(table, "style-name").map(str::to_string);
    let sheet_visibility = visibility(
        table,
        style_name.as_deref().and_then(|name| styles.get(name)),
    );
    let mut sheet = SpreadsheetOdfSheet {
        order,
        name: name.clone(),
        style_name,
        visibility: sheet_visibility,
        protected: bool_attr(table, "protected").unwrap_or(false),
        print_ranges: attr(table, "print-ranges").map(str::to_string),
        columns: Vec::new(),
        rows: Vec::new(),
        merges: Vec::new(),
        comments: Vec::new(),
        links: Vec::new(),
        objects: Vec::new(),
        raw_elements: Vec::new(),
        locator: sheet_locator(&name, (1, 1), (1, 1)),
    };
    let table_index = node_index(document, table);
    let mut row = 1u32;
    let mut column = 1u32;
    collect_table_children(document, table_index, &mut |node_index| {
        let node = &document.nodes[node_index];
        match local_name(&node.name) {
            "table-column" => {
                let repeated = repeat(node, "number-columns-repeated");
                let style = attr(node, "style-name").map(str::to_string);
                sheet.columns.push(SpreadsheetOdfColumn {
                    column,
                    repeated,
                    style_name: style.clone(),
                    default_cell_style_name: attr(node, "default-cell-style-name")
                        .map(str::to_string),
                    visibility: visibility(
                        node,
                        style.as_deref().and_then(|name| styles.get(name)),
                    ),
                    width: style
                        .as_deref()
                        .and_then(|name| styles.get(name))
                        .and_then(|style| style_property(style, "column-width")),
                    locator: sheet_locator(
                        &name,
                        (1, column),
                        (1, column.saturating_add(repeated - 1)),
                    ),
                });
                column = column.saturating_add(repeated);
            }
            "table-row" => {
                let repeated = repeat(node, "number-rows-repeated");
                let style = attr(node, "style-name").map(str::to_string);
                let mut output = SpreadsheetOdfRow {
                    row,
                    repeated,
                    style_name: style.clone(),
                    default_cell_style_name: attr(node, "default-cell-style-name")
                        .map(str::to_string),
                    visibility: visibility(
                        node,
                        style.as_deref().and_then(|name| styles.get(name)),
                    ),
                    height: style
                        .as_deref()
                        .and_then(|name| styles.get(name))
                        .and_then(|style| style_property(style, "row-height")),
                    cells: Vec::new(),
                    locator: sheet_locator(&name, (row, 1), (row.saturating_add(repeated - 1), 1)),
                };
                let mut cell_column = 1u32;
                for cell_index in child_indexes(node) {
                    let cell_node = &document.nodes[cell_index];
                    if !matches!(
                        local_name(&cell_node.name),
                        "table-cell" | "covered-table-cell"
                    ) {
                        continue;
                    }
                    let cell = parse_cell(
                        &name,
                        row,
                        cell_column,
                        repeated,
                        cell_index,
                        document,
                        entries,
                        manifest,
                        diagnostics,
                        &mut sheet,
                    );
                    let width = cell.repeated;
                    context.consume_cells(u64::from(width).saturating_mul(u64::from(repeated)))?;
                    output.cells.push(cell);
                    cell_column = cell_column.saturating_add(width);
                }
                let last_column = output
                    .cells
                    .last()
                    .map(|cell| cell.column.saturating_add(cell.repeated - 1))
                    .unwrap_or(1);
                output.locator = sheet_locator(
                    &name,
                    (row, 1),
                    (row.saturating_add(repeated - 1), last_column),
                );
                sheet.rows.push(output);
                row = row.saturating_add(repeated);
            }
            _ => {}
        }
        Ok(())
    })?;
    collect_sheet_objects(
        &mut sheet,
        table_index,
        document,
        entries,
        manifest,
        diagnostics,
    );
    sheet.raw_elements = child_indexes(&document.nodes[table_index])
        .filter(|index| {
            !matches!(
                local_name(&document.nodes[*index].name),
                "table-row"
                    | "table-column"
                    | "table-header-rows"
                    | "table-row-group"
                    | "table-rows"
                    | "table-header-columns"
                    | "table-column-group"
                    | "table-columns"
            )
        })
        .map(|index| raw_element(entry, document, index))
        .collect();
    let last_column = sheet
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .map(|cell| cell.column.saturating_add(cell.repeated - 1))
        .max()
        .unwrap_or(1);
    sheet.locator = sheet_locator(&name, (1, 1), (row.saturating_sub(1).max(1), last_column));
    Ok(sheet)
}

#[allow(clippy::too_many_arguments)]
fn parse_cell(
    sheet_name: &str,
    row: u32,
    column: u32,
    row_repeat: u32,
    index: usize,
    document: &XmlDocument,
    entries: &[PackageEntry],
    manifest: &[SpreadsheetOdfManifestEntry],
    diagnostics: &mut Vec<Diagnostic>,
    sheet: &mut SpreadsheetOdfSheet,
) -> SpreadsheetOdfCell {
    let node = &document.nodes[index];
    let repeated = repeat(node, "number-columns-repeated");
    let columns_spanned = repeat(node, "number-columns-spanned");
    let rows_spanned = repeat(node, "number-rows-spanned");
    let reference = cell_reference(row, column);
    let locator = sheet_locator(
        sheet_name,
        (row, column),
        (
            row.saturating_add(row_repeat - 1)
                .saturating_add(rows_spanned - 1),
            column
                .saturating_add(repeated - 1)
                .saturating_add(columns_spanned - 1),
        ),
    );
    let formula_source = attr(node, "formula").map(str::to_string);
    let value_type = attr(node, "value-type").map(str::to_string);
    let stored_value = stored_value(node);
    let displayed_value = visible_text(document, index);
    if columns_spanned > 1 || rows_spanned > 1 {
        sheet.merges.push(SpreadsheetOdfMerge {
            range: format!(
                "{}:{}",
                reference,
                cell_reference(
                    row.saturating_add(rows_spanned - 1),
                    column.saturating_add(columns_spanned - 1)
                )
            ),
            locator: locator.clone(),
        });
    }
    for child in descendants_with_indexes(document, index) {
        let child_node = &document.nodes[child];
        match local_name(&child_node.name) {
            "annotation" => {
                let creator = descendants(document, child)
                    .find(|item| local_name(&item.name) == "creator")
                    .map(|item| {
                        descendant_text(document, node_index(document, item))
                            .trim()
                            .to_string()
                    });
                let date = descendants(document, child)
                    .find(|item| local_name(&item.name) == "date")
                    .map(|item| {
                        descendant_text(document, node_index(document, item))
                            .trim()
                            .to_string()
                    });
                let text = annotation_text(document, child);
                sheet.comments.push(SpreadsheetOdfComment {
                    reference: reference.clone(),
                    creator,
                    date,
                    text,
                    locator: locator.clone(),
                });
            }
            "a" => {
                if let Some(target) = attr(child_node, "href") {
                    sheet.links.push(SpreadsheetOdfLink {
                        reference: reference.clone(),
                        target: target.to_string(),
                        label: Some(descendant_text(document, child).trim().to_string())
                            .filter(|value| !value.is_empty()),
                        external: is_external(target),
                        locator: locator.clone(),
                    });
                }
            }
            "image" | "object" | "object-ole" => {
                sheet.objects.push(parse_object(
                    child_node,
                    sheet_name,
                    &reference,
                    entries,
                    manifest,
                    diagnostics,
                ));
            }
            _ => {}
        }
    }
    let formula = formula_source.map(|source| SpreadsheetOdfFormula {
        namespace_prefix: source.split_once(':').map(|value| value.0.to_string()),
        source,
        calculate: false,
        locator: locator.clone(),
    });
    let cached_value = formula.as_ref().and_then(|_| {
        stored_value.clone().map(|value| SpreadsheetOdfCachedValue {
            stored_value: value,
            displayed_value: displayed_value.clone(),
            value_type: value_type.clone(),
            source: SpreadsheetOdfCachedValueSource::PackageStoredFormulaResult,
        })
    });
    SpreadsheetOdfCell {
        source_xml: raw_xml(document, node).to_string(),
        reference,
        row,
        column,
        repeated,
        kind: if local_name(&node.name) == "covered-table-cell" {
            SpreadsheetOdfCellKind::Covered
        } else {
            SpreadsheetOdfCellKind::Cell
        },
        value_type,
        stored_value,
        displayed_value,
        currency: attr(node, "currency").map(str::to_string),
        formula,
        cached_value,
        style_name: attr(node, "style-name").map(str::to_string),
        validation_name: attr(node, "content-validation-name").map(str::to_string),
        columns_spanned,
        rows_spanned,
        locator,
    }
}

fn parse_object(
    node: &XmlElement,
    sheet: &str,
    reference: &str,
    entries: &[PackageEntry],
    manifest: &[SpreadsheetOdfManifestEntry],
    diagnostics: &mut Vec<Diagnostic>,
) -> SpreadsheetOdfObject {
    let href = attr(node, "href").map(str::to_string);
    let path = href.as_deref().map(normalize_href);
    let media_type = path.as_deref().and_then(|path| {
        media_type_for(manifest, path).or_else(|| media_type_for(manifest, &format!("{path}/")))
    });
    let mut kind = match local_name(&node.name) {
        "image" => SpreadsheetOdfObjectKind::Image,
        "object" | "object-ole" => SpreadsheetOdfObjectKind::EmbeddedObject,
        _ => SpreadsheetOdfObjectKind::Unknown,
    };
    let target = path.as_deref().and_then(|path| {
        find_entry(entries, path).or_else(|| find_entry(entries, &format!("{path}/content.xml")))
    });
    let mut source_ranges = Vec::new();
    let mut cached_values = Vec::new();
    let mut title = None;
    if let Some(target) = target {
        if target.path.ends_with("content.xml") {
            if let Some(xml) = parse_xml(target, diagnostics) {
                if xml
                    .nodes
                    .iter()
                    .any(|item| matches!(local_name(&item.name), "chart" | "plot-area" | "series"))
                {
                    kind = SpreadsheetOdfObjectKind::Chart;
                }
                for item in &xml.nodes {
                    for (key, value) in &item.attributes {
                        if local_name(key).contains("range-address") {
                            source_ranges.push(value.clone());
                        }
                    }
                    if matches!(local_name(&item.name), "table-cell" | "data-point") {
                        if let Some(value) = stored_value(item) {
                            cached_values.push(value);
                        }
                    }
                    if title.is_none() && local_name(&item.name) == "title" {
                        let value = descendant_text(&xml, node_index(&xml, item))
                            .trim()
                            .to_string();
                        if !value.is_empty() {
                            title = Some(value);
                        }
                    }
                }
            }
        }
    }
    source_ranges.sort();
    source_ranges.dedup();
    cached_values.sort();
    cached_values.dedup();
    SpreadsheetOdfObject {
        kind,
        name: attr(node, "name").map(str::to_string),
        title,
        href,
        media_type,
        identity: target
            .and_then(|target| target.bytes.as_deref())
            .map(ContentIdentity::for_raw_bytes),
        anchor_cell: reference.to_string(),
        end_cell: attr(node, "end-cell-address").map(str::to_string),
        source_ranges,
        cached_values,
        locator: sheet_locator(
            sheet,
            parse_reference(reference),
            parse_reference(reference),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_sheet_objects(
    sheet: &mut SpreadsheetOdfSheet,
    table_index: usize,
    document: &XmlDocument,
    entries: &[PackageEntry],
    manifest: &[SpreadsheetOdfManifestEntry],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for index in descendants_with_indexes(document, table_index).filter(|index| {
        matches!(
            local_name(&document.nodes[*index].name),
            "image" | "object" | "object-ole"
        ) && nearest_ancestor(document, *index, |name| {
            matches!(name, "table-cell" | "covered-table-cell")
        })
        .is_none()
    }) {
        let node = &document.nodes[index];
        let frame_index = nearest_ancestor(document, index, |name| name == "frame");
        let anchor_node = frame_index.map_or(node, |frame| &document.nodes[frame]);
        let anchor = attr(anchor_node, "anchor-cell-address")
            .or_else(|| attr(node, "anchor-cell-address"))
            .unwrap_or("A1");
        let reference = {
            let (row, column) = parse_reference(anchor);
            cell_reference(row, column)
        };
        let mut object = parse_object(
            node,
            &sheet.name,
            &reference,
            entries,
            manifest,
            diagnostics,
        );
        object.name = attr(anchor_node, "name")
            .map(str::to_string)
            .or(object.name);
        object.end_cell = attr(anchor_node, "end-cell-address")
            .map(str::to_string)
            .or(object.end_cell);
        if object.title.is_none() {
            object.title = frame_index.and_then(|frame| {
                descendants(document, frame)
                    .find(|element| local_name(&element.name) == "title")
                    .map(|element| {
                        descendant_text(document, node_index(document, element))
                            .trim()
                            .to_string()
                    })
                    .filter(|value| !value.is_empty())
            });
        }
        sheet.objects.push(object);
    }
}

fn nearest_ancestor<F>(document: &XmlDocument, index: usize, mut matches: F) -> Option<usize>
where
    F: FnMut(&str) -> bool,
{
    let node = &document.nodes[index];
    document
        .nodes
        .iter()
        .enumerate()
        .filter(|(candidate_index, candidate)| {
            *candidate_index != index
                && candidate.start <= node.start
                && candidate.end >= node.end
                && matches(local_name(&candidate.name))
        })
        .max_by_key(|(_, candidate)| candidate.start)
        .map(|(index, _)| index)
}

fn package_parts(
    entries: &[PackageEntry],
    manifest: &[SpreadsheetOdfManifestEntry],
) -> Vec<SpreadsheetOdfPart> {
    entries
        .iter()
        .map(|entry| SpreadsheetOdfPart {
            package_index: entry.index,
            path: entry.path.clone(),
            media_type: media_type_for(manifest, &entry.path),
            compressed_size: entry.compressed_size,
            uncompressed_size: entry.uncompressed_size,
            crc32: entry.crc32,
            compression: entry.compression.clone(),
            status: if entry.rejected.is_some() {
                "rejected"
            } else if entry.kind == ArchiveEntryKind::Directory {
                "directory"
            } else if entry.encrypted {
                "encrypted"
            } else {
                "available"
            }
            .into(),
            encrypted: entry.encrypted,
            rejection_code: entry.rejected.as_ref().map(|value| value.0.clone()),
            rejection_message: entry.rejected.as_ref().map(|value| value.1.clone()),
            identity: entry.bytes.as_deref().map(ContentIdentity::for_raw_bytes),
            locator: member_locator(entry)
                .unwrap_or_else(|_| super::fallback_member_locator(entry.index)),
        })
        .collect()
}

fn raw_element(
    entry: &PackageEntry,
    document: &XmlDocument,
    index: usize,
) -> SpreadsheetOdfRawElement {
    let node = &document.nodes[index];
    SpreadsheetOdfRawElement {
        name: node.name.clone(),
        raw_xml: raw_xml(document, node).to_string(),
        locator: element_locator(entry, document, node),
    }
}

#[allow(clippy::too_many_arguments)]
fn document_node_count(
    sheets: &[SpreadsheetOdfSheet],
    named_ranges: usize,
    styles: usize,
    metadata: usize,
    manifest: usize,
    raw_elements: usize,
    parts: usize,
    panes: usize,
) -> u64 {
    let sheet_nodes = sheets.iter().fold(0usize, |count, sheet| {
        count
            .saturating_add(1)
            .saturating_add(sheet.columns.len())
            .saturating_add(sheet.rows.len())
            .saturating_add(sheet.rows.iter().map(|row| row.cells.len()).sum::<usize>())
            .saturating_add(sheet.merges.len())
            .saturating_add(sheet.comments.len())
            .saturating_add(sheet.links.len())
            .saturating_add(sheet.objects.len())
            .saturating_add(sheet.raw_elements.len())
    });
    u64::try_from(
        1usize
            .saturating_add(sheet_nodes)
            .saturating_add(named_ranges)
            .saturating_add(styles)
            .saturating_add(metadata)
            .saturating_add(manifest)
            .saturating_add(raw_elements)
            .saturating_add(parts)
            .saturating_add(panes),
    )
    .unwrap_or(u64::MAX)
}

fn media_type_for(manifest: &[SpreadsheetOdfManifestEntry], path: &str) -> Option<String> {
    manifest
        .iter()
        .find(|item| item.full_path.trim_end_matches('/') == path.trim_end_matches('/'))
        .and_then(|item| item.media_type.clone())
}

fn stored_value(node: &XmlElement) -> Option<String> {
    [
        "value",
        "string-value",
        "date-value",
        "time-value",
        "boolean-value",
    ]
    .into_iter()
    .find_map(|name| attr(node, name).map(str::to_string))
}

fn visible_text(document: &XmlDocument, index: usize) -> Option<String> {
    fn collect(document: &XmlDocument, index: usize, output: &mut String) {
        let node = &document.nodes[index];
        if matches!(
            local_name(&node.name),
            "annotation" | "object" | "object-ole" | "image"
        ) {
            return;
        }
        match local_name(&node.name) {
            "s" => output.push(' '),
            "tab" => output.push('\t'),
            "line-break" => output.push('\n'),
            _ => {}
        }
        for content in &node.content {
            match content {
                XmlContent::Text { value, .. } => output.push_str(value),
                XmlContent::Child(child) => collect(document, *child, output),
            }
        }
        if matches!(local_name(&node.name), "p" | "h") && !output.ends_with('\n') {
            output.push('\n');
        }
    }
    let mut output = String::new();
    collect(document, index, &mut output);
    let output = output.trim_end_matches('\n').to_string();
    (!output.is_empty()).then_some(output)
}

fn annotation_text(document: &XmlDocument, index: usize) -> String {
    descendants_with_indexes(document, index)
        .filter(|child| matches!(local_name(&document.nodes[*child].name), "p" | "h"))
        .map(|child| descendant_text(document, child).trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn visibility(node: &XmlElement, style: Option<&SpreadsheetOdfStyle>) -> SpreadsheetOdfVisibility {
    let value = attr(node, "visibility")
        .or_else(|| attr(node, "display"))
        .map(str::to_ascii_lowercase)
        .or_else(|| style.and_then(|style| style_property(style, "display")))
        .unwrap_or_else(|| "visible".into());
    match value.as_str() {
        "visible" | "true" => SpreadsheetOdfVisibility::Visible,
        "hidden" | "false" => SpreadsheetOdfVisibility::Hidden,
        "filter" | "filtered" => SpreadsheetOdfVisibility::Filtered,
        "collapse" | "collapsed" => SpreadsheetOdfVisibility::Collapsed,
        _ => SpreadsheetOdfVisibility::Unknown,
    }
}

fn style_property(style: &SpreadsheetOdfStyle, wanted: &str) -> Option<String> {
    style
        .properties
        .iter()
        .find(|(key, _)| local_name(key) == wanted)
        .map(|(_, value)| value.clone())
}

fn repeat(node: &XmlElement, name: &str) -> u32 {
    attr(node, name)
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
        .max(1)
}
fn bool_attr(node: &XmlElement, name: &str) -> Option<bool> {
    attr(node, name).and_then(|value| match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    })
}
fn is_external(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:") || target.starts_with("file:")
}
fn normalize_href(href: &str) -> String {
    href.trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

fn parse_reference(reference: &str) -> (u32, u32) {
    let value = reference
        .rsplit('.')
        .next()
        .unwrap_or(reference)
        .trim_matches('$');
    let split = value
        .find(|character: char| character.is_ascii_digit())
        .unwrap_or(1);
    let (letters, digits) = value.split_at(split);
    let column = letters
        .trim_matches('$')
        .chars()
        .fold(0u32, |total, letter| {
            total.saturating_mul(26).saturating_add(
                u32::from(letter.to_ascii_uppercase()).saturating_sub(u32::from('A')) + 1,
            )
        })
        .max(1);
    (digits.trim_matches('$').parse().unwrap_or(1), column)
}

fn collect_table_children<F>(
    document: &XmlDocument,
    index: usize,
    visitor: &mut F,
) -> Result<(), ParserError>
where
    F: FnMut(usize) -> Result<(), ParserError>,
{
    for child in child_indexes(&document.nodes[index]) {
        let name = local_name(&document.nodes[child].name);
        if matches!(name, "table-row" | "table-column") {
            visitor(child)?;
        } else if matches!(
            name,
            "table-header-rows"
                | "table-row-group"
                | "table-rows"
                | "table-header-columns"
                | "table-column-group"
                | "table-columns"
        ) {
            collect_table_children(document, child, visitor)?;
        }
    }
    Ok(())
}

fn child_indexes(node: &XmlElement) -> impl Iterator<Item = usize> + '_ {
    node.content.iter().filter_map(|content| match content {
        XmlContent::Child(index) => Some(*index),
        XmlContent::Text { .. } => None,
    })
}

fn descendants(document: &XmlDocument, index: usize) -> impl Iterator<Item = &XmlElement> {
    descendants_with_indexes(document, index).map(|index| &document.nodes[index])
}

fn descendants_with_indexes(
    document: &XmlDocument,
    index: usize,
) -> impl Iterator<Item = usize> + '_ {
    let mut stack = child_indexes(&document.nodes[index]).collect::<Vec<_>>();
    stack.reverse();
    std::iter::from_fn(move || {
        let index = stack.pop()?;
        let mut children = child_indexes(&document.nodes[index]).collect::<Vec<_>>();
        children.reverse();
        stack.extend(children);
        Some(index)
    })
}

fn node_index(document: &XmlDocument, node: &XmlElement) -> usize {
    document
        .nodes
        .iter()
        .position(|candidate| std::ptr::eq(candidate, node))
        .expect("node belongs to document")
}
