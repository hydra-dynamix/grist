//! Safe ODT and OTT package parser.

mod archive;
mod graph;
mod model;
mod parse;
mod xml;

pub use model::*;

use crate::core::{
    IndexBase, IndexPosition, LocationComponent, ParserInfo, SourceLocator, SourceLocatorError,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};

pub(crate) const PARSER: &str = "grist.odf_word";

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("zip + quick-xml", "4.6.1/0.37.5")
        .with_specification_version("OASIS OpenDocument 1.2/1.3 text package conventions")
        .with_feature("odf-word")
}

pub(crate) fn parse_odt_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse::parse_registered(context, OdfPackageKind::Document)
}

pub(crate) fn parse_ott_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    parse::parse_registered(context, OdfPackageKind::Template)
}

fn member_locator(entry: &archive::PackageEntry) -> Result<SourceLocator, SourceLocatorError> {
    SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: entry.path.clone(),
        member_index: IndexPosition::new(entry.index as u64, IndexBase::Zero)
            .expect("zero-based archive member index is valid"),
    })
}

fn fallback_member_locator(index: usize) -> SourceLocator {
    SourceLocator::exact(LocationComponent::ArchiveMember {
        member_path: "[rejected-package-member]".into(),
        member_index: IndexPosition::new(index as u64, IndexBase::Zero)
            .expect("zero-based archive member index is valid"),
    })
    .expect("fallback archive member locator")
}
