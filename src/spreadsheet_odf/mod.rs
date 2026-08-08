//! Safe, bounded ODS and OTS parser.
//!
//! Formula text, stored results, links, and embedded objects are retained as
//! inert package data. The parser performs no calculation or external access.

mod archive;
mod graph;
mod model;
mod parse;
mod xml;

pub use model::*;

use crate::core::{
    CellAddress, IndexBase, IndexPosition, LocationComponent, ParserInfo, SourceLocator,
    SourceLocatorError,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};

pub(crate) const PARSER: &str = "grist.spreadsheet_odf";

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("zip + quick-xml", "4.6.1/0.37.5")
        .with_specification_version("OASIS OpenDocument 1.2/1.3 spreadsheet package conventions")
        .with_feature("spreadsheet-odf")
}

pub(crate) fn parse_ods_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse::parse_registered(context, SpreadsheetOdfPackageKind::Workbook)
}

pub(crate) fn parse_ots_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse::parse_registered(context, SpreadsheetOdfPackageKind::Template)
}

fn member_locator(entry: &archive::PackageEntry) -> Result<SourceLocator, SourceLocatorError> {
    SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: entry.path.clone(),
        member_index: IndexPosition::new(entry.index as u64, IndexBase::Zero)
            .expect("valid package index"),
    })
}

fn fallback_member_locator(index: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: "[rejected-package-member]".into(),
        member_index: IndexPosition::new(index as u64, IndexBase::Zero)
            .expect("valid package index"),
    })
    .expect("fallback package locator")
}

pub(super) fn sheet_locator(sheet: &str, start: (u32, u32), end: (u32, u32)) -> SourceLocator {
    SourceLocator::exact(LocationComponent::SheetRange {
        sheet: sheet.into(),
        start_cell: CellAddress::a1(u64::from(start.0), u64::from(start.1)).expect("valid cell"),
        end_cell: CellAddress::a1(u64::from(end.0), u64::from(end.1)).expect("valid cell"),
    })
    .expect("valid sheet range")
}

pub(super) fn cell_reference(row: u32, column: u32) -> String {
    let mut value = column.max(1);
    let mut letters = String::new();
    while value > 0 {
        let digit = ((value - 1) % 26) as u8;
        letters.insert(0, char::from(b'A' + digit));
        value = (value - 1) / 26;
    }
    format!("{letters}{}", row.max(1))
}
