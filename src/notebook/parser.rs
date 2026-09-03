use super::{
    NotebookAttachment, NotebookCell, NotebookDocument, NotebookOptions, NotebookOutput,
    NotebookWorksheet,
};
use crate::core::{
    Diagnostic, IndexPosition, LocationComponent, OperationControl, SchemaVersion, SourceLocator,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const PARSER: &str = "grist.ipynb";

pub(crate) fn parse_document(
    bytes: &[u8],
    options: &NotebookOptions,
    control: &OperationControl,
) -> Result<NotebookDocument, Box<Diagnostic>> {
    let (structural_nodes, observed_depth) = scan_json_structure(bytes, options.max_json_depth)?;
    control
        .budget()
        .observe_nesting_depth(observed_depth as u64)
        .and_then(|_| control.budget().consume_nodes(structural_nodes as u64))
        .and_then(|_| {
            control
                .budget()
                .observe_memory_bytes(bytes.len().saturating_mul(2) as u64)
        })
        .map_err(|error| Box::new(error.diagnostic(PARSER)))?;
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        Box::new(Diagnostic::error(
            PARSER,
            "ipynb.json.malformed",
            format!("invalid notebook JSON: {error}"),
        ))
    })?;
    let root = value.as_object().ok_or_else(|| {
        Box::new(Diagnostic::error(
            PARSER,
            "ipynb.root.not_object",
            "a Jupyter notebook must be a JSON object",
        ))
    })?;
    let nbformat = required_u32(root, "nbformat")?;
    if !matches!(nbformat, 3 | 4) {
        return Err(Box::new(Diagnostic::unsupported(
            PARSER,
            format!("nbformat {nbformat} is not supported; expected 3 or 4"),
        )));
    }
    let nbformat_minor = optional_u32(root.get("nbformat_minor")).unwrap_or(0);
    let metadata = root.get("metadata").cloned().unwrap_or_else(empty_object);
    let widgets = metadata.get("widgets").cloned();
    let locator = json_pointer("");
    let mut diagnostics = Vec::new();
    let mut cells = Vec::new();
    let mut worksheets = Vec::new();

    if nbformat == 4 {
        let raw_cells = root.get("cells").and_then(Value::as_array).ok_or_else(|| {
            Box::new(Diagnostic::error(
                PARSER,
                "ipynb.cells.missing",
                "nbformat 4 notebook has no cells array",
            ))
        })?;
        parse_cells(
            raw_cells,
            None,
            options,
            control,
            &mut cells,
            &mut diagnostics,
        )?;
    } else {
        let raw_worksheets = root
            .get("worksheets")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Box::new(Diagnostic::error(
                    PARSER,
                    "ipynb.worksheets.missing",
                    "nbformat 3 notebook has no worksheets array",
                ))
            })?;
        for (worksheet_index, raw_worksheet) in raw_worksheets.iter().enumerate() {
            control
                .checkpoint()
                .map_err(|error| Box::new(error.diagnostic(PARSER)))?;
            let Some(worksheet) = raw_worksheet.as_object() else {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "ipynb.worksheet.not_object",
                        format!("worksheet {worksheet_index} is not an object"),
                    )
                    .partial()
                    .with_locator(json_pointer(&format!("/worksheets/{worksheet_index}"))),
                );
                continue;
            };
            let first = cells.len();
            if let Some(raw_cells) = worksheet.get("cells").and_then(Value::as_array) {
                parse_cells(
                    raw_cells,
                    Some(worksheet_index),
                    options,
                    control,
                    &mut cells,
                    &mut diagnostics,
                )?;
            } else {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "ipynb.worksheet.cells_missing",
                        format!("worksheet {worksheet_index} has no cells array"),
                    )
                    .partial()
                    .with_locator(json_pointer(&format!("/worksheets/{worksheet_index}"))),
                );
            }
            worksheets.push(NotebookWorksheet {
                ordinal: worksheet_index,
                metadata: worksheet
                    .get("metadata")
                    .cloned()
                    .unwrap_or_else(empty_object),
                cell_stable_ids: cells[first..]
                    .iter()
                    .map(|cell| cell.stable_id.clone())
                    .collect(),
                locator: json_pointer(&format!("/worksheets/{worksheet_index}")),
            });
        }
    }

    let mut extra = root.clone();
    for key in [
        "nbformat",
        "nbformat_minor",
        "metadata",
        "cells",
        "worksheets",
    ] {
        extra.remove(key);
    }
    Ok(NotebookDocument {
        schema_version: SchemaVersion::IPYNB_V1.to_string(),
        nbformat,
        nbformat_minor,
        metadata,
        widgets,
        cells,
        worksheets,
        extra,
        complete: diagnostics.is_empty(),
        diagnostics,
        locator,
    })
}

fn parse_cells(
    raw_cells: &[Value],
    worksheet_index: Option<usize>,
    options: &NotebookOptions,
    control: &OperationControl,
    cells: &mut Vec<NotebookCell>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), Box<Diagnostic>> {
    for (local_index, raw_cell) in raw_cells.iter().enumerate() {
        if cells.len() >= options.max_cells {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "ipynb.limit.cells",
                    format!("cell limit {} reached", options.max_cells),
                )
                .partial(),
            );
            break;
        }
        control
            .checkpoint()
            .map_err(|error| Box::new(error.diagnostic(PARSER)))?;
        let ordinal = cells.len();
        let Some(cell) = raw_cell.as_object() else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "ipynb.cell.not_object",
                    format!("cell {ordinal} is not an object"),
                )
                .partial(),
            );
            continue;
        };
        let cell_type = match cell.get("cell_type").and_then(Value::as_str) {
            Some(cell_type) => cell_type.to_string(),
            None => {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "ipynb.cell.cell_type_invalid",
                        format!("cell {ordinal} has no string cell_type"),
                    )
                    .partial()
                    .with_locator(json_pointer(&cell_pointer(worksheet_index, local_index))),
                );
                "unknown".to_string()
            }
        };
        let id = cell.get("id").and_then(Value::as_str).map(str::to_string);
        if cell.get("id").is_some() && id.is_none() {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "ipynb.cell.id_invalid",
                    format!("cell {ordinal} id is not a string"),
                )
                .partial()
                .with_locator(json_pointer(&cell_pointer(worksheet_index, local_index))),
            );
        }
        let source_raw = cell
            .get("source")
            .or_else(|| cell.get("input"))
            .cloned()
            .unwrap_or_else(|| Value::String(String::new()));
        if !matches!(&source_raw, Value::String(_) | Value::Array(_))
            || source_raw
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| !part.is_string()))
        {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "ipynb.cell.source_invalid",
                    format!("cell {ordinal} source/input must be a string or string array"),
                )
                .partial()
                .with_locator(json_pointer(&cell_pointer(worksheet_index, local_index))),
            );
        }
        let source = source_text(&source_raw);
        let stable_material = serde_json::to_vec(&if let Some(native_id) = &id {
            serde_json::json!({
                "domain": "grist.ipynb.native-cell-id.v1",
                "native_id": native_id,
            })
        } else {
            serde_json::json!({
                "domain": "grist.ipynb.generated-cell-id.v1",
                "ordinal": ordinal,
                "worksheet": worksheet_index,
                "cell_type": &cell_type,
                "source": &source_raw,
            })
        })
        .expect("cell identity material serializes");
        let stable_id = format!("cell:{}", crate::core::sha256_hex(&stable_material));
        let pointer = cell_pointer(worksheet_index, local_index);
        let locator =
            notebook_locator(ordinal, id.as_deref().unwrap_or(&stable_id), None, &pointer);
        let attachments = parse_attachments(
            cell.get("attachments"),
            ordinal,
            id.as_deref().unwrap_or(&stable_id),
            &pointer,
            options,
            diagnostics,
        );
        let outputs = parse_outputs(
            cell.get("outputs"),
            ordinal,
            id.as_deref().unwrap_or(&stable_id),
            &stable_id,
            &pointer,
            options,
            control,
            diagnostics,
        )?;
        let mut extra = cell.clone();
        for key in [
            "id",
            "cell_type",
            "source",
            "input",
            "metadata",
            "execution_count",
            "prompt_number",
            "attachments",
            "outputs",
        ] {
            extra.remove(key);
        }
        cells.push(NotebookCell {
            ordinal,
            worksheet_index,
            id,
            stable_id,
            cell_type,
            source,
            source_raw,
            metadata: cell.get("metadata").cloned().unwrap_or_else(empty_object),
            execution_count: cell
                .get("execution_count")
                .or_else(|| cell.get("prompt_number"))
                .cloned(),
            attachments,
            outputs,
            extra,
            locator,
        });
    }
    Ok(())
}

fn parse_attachments(
    raw: Option<&Value>,
    cell_index: usize,
    cell_id: &str,
    cell_pointer: &str,
    options: &NotebookOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<NotebookAttachment> {
    let attachments = match raw {
        None => return Vec::new(),
        Some(Value::Object(attachments)) => attachments,
        Some(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "ipynb.attachments.not_object",
                    format!("attachments for cell {cell_index} are not an object"),
                )
                .partial()
                .with_locator(json_pointer(&format!("{cell_pointer}/attachments"))),
            );
            return Vec::new();
        }
    };
    attachments
        .iter()
        .map(|(name, bundle)| {
            let data = mime_bundle(bundle, options, diagnostics, "attachment", cell_index);
            let pointer = format!("{cell_pointer}/attachments/{}", escape_json_pointer(name));
            NotebookAttachment {
                name: name.clone(),
                data,
                locator: notebook_locator(cell_index, cell_id, None, &pointer),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn parse_outputs(
    raw: Option<&Value>,
    cell_index: usize,
    cell_id: &str,
    cell_stable_id: &str,
    cell_pointer: &str,
    options: &NotebookOptions,
    control: &OperationControl,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<NotebookOutput>, Box<Diagnostic>> {
    let outputs = match raw {
        None => return Ok(Vec::new()),
        Some(Value::Array(outputs)) => outputs,
        Some(_) => {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "ipynb.outputs.not_array",
                    format!("outputs for cell {cell_index} are not an array"),
                )
                .partial()
                .with_locator(json_pointer(&format!("{cell_pointer}/outputs"))),
            );
            return Ok(Vec::new());
        }
    };
    let mut parsed = Vec::new();
    for (ordinal, raw_output) in outputs.iter().enumerate() {
        if parsed.len() >= options.max_outputs_per_cell {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "ipynb.limit.outputs",
                    format!(
                        "output limit {} reached for cell {cell_index}",
                        options.max_outputs_per_cell
                    ),
                )
                .partial()
                .with_locator(notebook_locator(
                    cell_index,
                    cell_id,
                    None,
                    cell_pointer,
                )),
            );
            break;
        }
        control
            .checkpoint()
            .map_err(|error| Box::new(error.diagnostic(PARSER)))?;
        let Some(output) = raw_output.as_object() else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "ipynb.output.not_object",
                    format!("output {ordinal} for cell {cell_index} is not an object"),
                )
                .partial()
                .with_locator(notebook_locator(
                    cell_index,
                    cell_id,
                    Some(ordinal),
                    &format!("{cell_pointer}/outputs/{ordinal}"),
                )),
            );
            continue;
        };
        let output_type = match output.get("output_type").and_then(Value::as_str) {
            Some(output_type) => output_type.to_string(),
            None => {
                diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "ipynb.output.output_type_invalid",
                        format!("output {ordinal} for cell {cell_index} has no string output_type"),
                    )
                    .partial()
                    .with_locator(json_pointer(&format!(
                        "{cell_pointer}/outputs/{ordinal}/output_type"
                    ))),
                );
                "unknown".to_string()
            }
        };
        let normalized_output_type = match output_type.as_str() {
            "pyout" => "execute_result",
            "pyerr" => "error",
            other => other,
        }
        .to_string();
        let mut data = output
            .get("data")
            .map(|value| mime_bundle(value, options, diagnostics, "output", cell_index))
            .unwrap_or_default();
        for (legacy, mime) in [
            ("text", "text/plain"),
            ("html", "text/html"),
            ("svg", "image/svg+xml"),
            ("png", "image/png"),
            ("jpeg", "image/jpeg"),
            ("latex", "text/latex"),
            ("json", "application/json"),
            ("javascript", "application/javascript"),
        ] {
            if let Some(value) = output.get(legacy) {
                data.entry(mime.to_string())
                    .or_insert_with(|| value.clone());
            }
        }
        let text = output
            .get("text")
            .map(source_text)
            .or_else(|| data.get("text/plain").map(source_text));
        let mut extra = output.clone();
        for key in [
            "output_type",
            "execution_count",
            "prompt_number",
            "name",
            "text",
            "data",
            "metadata",
            "transient",
            "ename",
            "evalue",
            "traceback",
            "html",
            "svg",
            "png",
            "jpeg",
            "latex",
            "json",
            "javascript",
        ] {
            extra.remove(key);
        }
        let widget_view = data
            .get("application/vnd.jupyter.widget-view+json")
            .cloned();
        parsed.push(NotebookOutput {
            ordinal,
            cell_stable_id: cell_stable_id.to_string(),
            output_type,
            normalized_output_type,
            execution_count: output
                .get("execution_count")
                .or_else(|| output.get("prompt_number"))
                .cloned(),
            stream_name: output
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            text,
            data,
            metadata: output.get("metadata").cloned().unwrap_or_else(empty_object),
            transient: output
                .get("transient")
                .cloned()
                .unwrap_or_else(empty_object),
            error_name: output
                .get("ename")
                .and_then(Value::as_str)
                .map(str::to_string),
            error_value: output
                .get("evalue")
                .and_then(Value::as_str)
                .map(str::to_string),
            traceback: output
                .get("traceback")
                .and_then(Value::as_array)
                .map(|lines| lines.iter().map(source_text).collect())
                .unwrap_or_default(),
            extra,
            widget_view,
            locator: notebook_locator(
                cell_index,
                cell_id,
                Some(ordinal),
                &format!("{cell_pointer}/outputs/{ordinal}"),
            ),
        });
    }
    Ok(parsed)
}

fn mime_bundle(
    value: &Value,
    options: &NotebookOptions,
    diagnostics: &mut Vec<Diagnostic>,
    owner: &str,
    cell_index: usize,
) -> BTreeMap<String, Value> {
    let Some(bundle) = value.as_object() else {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "ipynb.mime.not_object",
                format!("{owner} MIME bundle for cell {cell_index} is not an object"),
            )
            .partial(),
        );
        return BTreeMap::new();
    };
    if bundle.len() > options.max_mime_entries {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "ipynb.limit.mime_entries",
                format!(
                    "{owner} MIME bundle for cell {cell_index} exceeds {} entries",
                    options.max_mime_entries
                ),
            )
            .partial(),
        );
    }
    bundle
        .iter()
        .take(options.max_mime_entries)
        .map(|(mime, value)| (mime.clone(), value.clone()))
        .collect()
}

fn required_u32(root: &Map<String, Value>, key: &str) -> Result<u32, Box<Diagnostic>> {
    optional_u32(root.get(key)).ok_or_else(|| {
        Box::new(Diagnostic::error(
            PARSER,
            "ipynb.version.invalid",
            format!("{key} must be an unsigned integer"),
        ))
    })
}

fn optional_u32(value: Option<&Value>) -> Option<u32> {
    value?.as_u64()?.try_into().ok()
}

fn source_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| {
                part.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| part.to_string())
            })
            .collect(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn notebook_locator(
    cell_index: usize,
    cell_id: &str,
    output: Option<usize>,
    pointer: &str,
) -> SourceLocator {
    SourceLocator::exact(LocationComponent::NotebookCell {
        index: IndexPosition::one_based(cell_index.saturating_add(1) as u64)
            .expect("one-based notebook cell index is valid"),
        cell_id: Some(cell_id.to_string()),
        output_index: output.map(|value| {
            IndexPosition::one_based(value.saturating_add(1) as u64)
                .expect("one-based notebook output index is valid")
        }),
    })
    .expect("notebook locator is valid")
    .nested(LocationComponent::JsonPointer {
        pointer: pointer.to_string(),
    })
    .expect("notebook JSON pointer is valid")
}

fn cell_pointer(worksheet_index: Option<usize>, local_index: usize) -> String {
    worksheet_index.map_or_else(
        || format!("/cells/{local_index}"),
        |worksheet| format!("/worksheets/{worksheet}/cells/{local_index}"),
    )
}

fn json_pointer(pointer: &str) -> SourceLocator {
    SourceLocator::exact(LocationComponent::JsonPointer {
        pointer: pointer.to_string(),
    })
    .expect("JSON pointer locator is valid")
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

fn scan_json_structure(bytes: &[u8], max_depth: usize) -> Result<(usize, usize), Box<Diagnostic>> {
    let mut depth = 0usize;
    let mut observed_depth = 0usize;
    let mut structural_nodes = 1usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                observed_depth = observed_depth.max(depth);
                structural_nodes = structural_nodes.saturating_add(1);
                if depth > max_depth {
                    return Err(Box::new(Diagnostic::error(
                        PARSER,
                        "ipynb.limit.json_depth",
                        format!("notebook JSON exceeds maximum depth {max_depth}"),
                    )));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            b':' | b',' => structural_nodes = structural_nodes.saturating_add(1),
            _ => {}
        }
    }
    Ok((structural_nodes, observed_depth))
}
