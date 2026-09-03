//! Safe XLSX and XLSM workbook parser.
//!
//! Formula source, stored calculation caches, macros, links, and embedded
//! objects are retained as inert source data. Nothing is calculated or run.

mod graph;
mod model;
mod package;
mod parse;
mod xml;

pub use model::*;

use crate::core::{
    CellAddress, Diagnostic, LocationComponent, OperationStatus, ParserInfo, SchemaVersion,
    SourceLocator,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};

pub(crate) const PARSER: &str = "grist.spreadsheet_ooxml";

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("zip + quick-xml", "4.6.1/0.37.5")
        .with_specification_version("ECMA-376 OPC and SpreadsheetML package conventions")
        .with_feature("spreadsheet-ooxml")
}

pub(crate) fn parse_xlsx_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, SpreadsheetPackageKind::Workbook)
}

pub(crate) fn parse_xlsm_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse_registered(context, SpreadsheetPackageKind::MacroEnabledWorkbook)
}

fn parse_registered(
    context: &mut ParserContext<'_>,
    expected: SpreadsheetPackageKind,
) -> Result<ParserOutput, ParserError> {
    let options: SpreadsheetOoxmlOptions = serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed(PARSER, error.to_string())))?;
    if looks_like_encrypted_ooxml(context.bytes()) {
        return Ok(package::encrypted_output());
    }
    let mut archive = package::read_package(context.bytes(), context)?;
    if archive.encrypted {
        return Ok(package::encrypted_output());
    }
    let mut diagnostics = std::mem::take(&mut archive.diagnostics);
    let manifest = package::parse_manifest(&archive.entries, &mut diagnostics)?;
    if manifest.kind != expected {
        return Err(Box::new(Diagnostic::error(
            PARSER,
            "spreadsheet_ooxml.package_kind_mismatch",
            format!(
                "requested {} but package declares {}",
                expected.format_id(),
                manifest.kind.format_id()
            ),
        )));
    }
    let relationships = package::parse_relationships(&archive.entries, &mut diagnostics);
    let office = relationships
        .iter()
        .filter(|item| {
            item.source_part.is_none() && item.relationship_type.ends_with("/officeDocument")
        })
        .collect::<Vec<_>>();
    if office.len() != 1
        || office[0].resolved_part.as_deref() != Some(&manifest.workbook_part)
        || office[0].target_exists != Some(true)
    {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "root relationships must resolve exactly one officeDocument target matching the workbook part",
        )));
    }
    let mut workbook = parse::parse_workbook(
        &archive.entries,
        &relationships,
        &manifest.workbook_part,
        context,
        &mut diagnostics,
    )?;
    let macro_projects = package::macro_projects(&archive.entries, &manifest.content_types);
    context.consume_child_artifacts(macro_projects.len() as u64)?;
    if expected == SpreadsheetPackageKind::Workbook && !macro_projects.is_empty() {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "spreadsheet_ooxml.macro.unexpected",
                "macro project found in XLSX content and quarantined",
            )
            .with_locator(macro_projects[0].locator.clone())
            .partial(),
        );
    }
    let all_parts = package::package_parts(&archive.entries, &manifest.content_types);
    for sheet in &mut workbook.sheets {
        for object in &mut sheet.objects {
            object.content_type = object
                .part
                .as_deref()
                .and_then(|path| package::content_type(&manifest.content_types, path));
        }
    }
    let parts = if options.include_parts {
        all_parts
    } else {
        Vec::new()
    };
    let properties = package::parse_properties(&archive.entries, &mut diagnostics);
    context.consume_nodes(
        parts
            .len()
            .saturating_add(relationships.len())
            .saturating_add(properties.len())
            .saturating_add(workbook.named_ranges.len())
            .saturating_add(macro_projects.len())
            .saturating_add(1) as u64,
    )?;
    let document = SpreadsheetOoxmlDocument {
        schema_version: SchemaVersion::SPREADSHEET_OOXML_V1.into(),
        package_kind: manifest.kind,
        package_media_type: manifest.kind.media_type().into(),
        workbook_part: manifest.workbook_part.clone(),
        workbook_locator: part_locator(&manifest.workbook_part),
        date_system: workbook.date_system,
        calculation: workbook.calculation,
        sheets: workbook.sheets,
        named_ranges: workbook.named_ranges,
        styles: workbook.styles,
        properties,
        relationships,
        parts,
        macro_projects,
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

pub(super) fn part_locator(path: &str) -> SourceLocator {
    SourceLocator::exact(LocationComponent::OoxmlPart {
        part: path.into(),
        paragraph: None,
        run: None,
        table: None,
        row: None,
        column: None,
        object_id: None,
    })
    .expect("valid OOXML part locator")
}

pub(super) fn xml_locator(part: &str, path: &str) -> SourceLocator {
    part_locator(part)
        .nested(LocationComponent::XmlPath { path: path.into() })
        .expect("valid XML locator")
}

pub(super) fn sheet_locator(sheet: &str, start: (u32, u32), end: (u32, u32)) -> SourceLocator {
    SourceLocator::exact(LocationComponent::SheetRange {
        sheet: sheet.into(),
        start_cell: CellAddress::a1(u64::from(start.0), u64::from(start.1)).expect("valid cell"),
        end_cell: CellAddress::a1(u64::from(end.0), u64::from(end.1)).expect("valid cell"),
    })
    .expect("valid sheet range")
}

pub(super) fn cell_locator(sheet: &str, reference: &str) -> SourceLocator {
    let split = reference
        .find(|value: char| value.is_ascii_digit())
        .unwrap_or(1);
    let (letters, digits) = reference.split_at(split);
    let column = letters
        .trim_matches('$')
        .chars()
        .fold(0u32, |value, letter| {
            value.saturating_mul(26).saturating_add(
                u32::from(letter.to_ascii_uppercase()).saturating_sub(u32::from('A')) + 1,
            )
        })
        .max(1);
    let row = digits.trim_matches('$').parse().unwrap_or(1);
    sheet_locator(sheet, (row, column), (row, column))
}

fn looks_like_encrypted_ooxml(bytes: &[u8]) -> bool {
    const OLE_MAGIC: &[u8] = &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    bytes.starts_with(OLE_MAGIC)
        && ["EncryptedPackage", "EncryptionInfo"].iter().any(|needle| {
            let encoded = needle
                .bytes()
                .flat_map(|byte| [byte, 0])
                .collect::<Vec<_>>();
            bytes.windows(encoded.len()).any(|window| window == encoded)
        })
}

pub fn parse_status_for_encrypted_compound(bytes: &[u8]) -> Option<OperationStatus> {
    looks_like_encrypted_ooxml(bytes).then_some(OperationStatus::Encrypted)
}
