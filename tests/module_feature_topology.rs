#![allow(unused_imports)]

use grist::{
    container, core, detect, document_graph, formats, ingest, provider, registry, render, schema,
    segment, transform,
};

const MANIFEST: &str = include_str!("../Cargo.toml");

#[test]
fn target_module_namespaces_are_public_in_the_minimal_library() {
    assert_eq!(grist::version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn family_features_and_selection_profiles_are_declared() {
    let families = [
        "text-publishing",
        "scholarly",
        "pdf",
        "word-processing",
        "presentations",
        "spreadsheets",
        "structured-data",
        "email-message",
        "notebooks",
        "code",
        "archives",
        "media",
        "model-output",
    ];
    for feature in ["default", "cli", "full"] {
        assert!(
            MANIFEST
                .lines()
                .any(|line| line.starts_with(&format!("{feature} ="))),
            "Cargo feature `{feature}` is missing"
        );
    }
    for family in families {
        assert!(
            MANIFEST
                .lines()
                .any(|line| line.starts_with(&format!("{family} ="))),
            "Cargo family feature `{family}` is missing"
        );
        let full = MANIFEST
            .split_once("full = [")
            .and_then(|(_, tail)| tail.split_once(']'))
            .map(|(members, _)| members)
            .expect("full feature block");
        assert!(
            full.contains(&format!("\"{family}\"")),
            "full does not enable `{family}`"
        );
    }
    assert!(MANIFEST.contains(r#"cli = ["dep:clap", "full"]"#));
}

#[test]
fn plain_text_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::text::TextDocument> = None;
    let _: Option<grist::text::TextDocument> = canonical;
}

#[cfg(feature = "markdown")]
#[test]
fn markdown_compatibility_path_has_identical_types() {
    fn accepts_canonical(_: grist::formats::markdown::MarkdownDocument) {}
    let envelope = grist::markdown::parse_markdown(
        "# topology",
        grist::core::SourceInfo::stdin("compatibility.md"),
    );
    accepts_canonical(envelope.payload.expect("complete Markdown payload"));
}

#[cfg(feature = "restructured-text")]
#[test]
fn restructured_text_compatibility_path_has_identical_types() {
    fn accepts_canonical(_: grist::formats::restructured_text::RestructuredTextDocument) {}
    let envelope = grist::restructured_text::parse_restructured_text(
        "Topology\n========\n",
        grist::core::SourceInfo::stdin("compatibility.rst"),
    );
    accepts_canonical(envelope.payload.expect("complete reStructuredText payload"));
}

#[cfg(feature = "asciidoc")]
#[test]
fn asciidoc_compatibility_path_has_identical_types() {
    fn accepts_canonical(_: grist::formats::asciidoc::AsciiDocDocument) {}
    let envelope = grist::asciidoc::parse_asciidoc(
        "= Topology\n",
        grist::core::SourceInfo::stdin("compatibility.adoc"),
    );
    accepts_canonical(envelope.payload.expect("complete AsciiDoc payload"));
}
#[cfg(feature = "csv")]
#[test]
fn delimited_compatibility_path_has_identical_types() {
    fn accepts_canonical(_: grist::formats::delimited::CsvDocument) {}
    let envelope = grist::csv::parse_csv(
        "name\nGrist\n",
        grist::core::SourceInfo::stdin("compatibility.csv"),
        &grist::formats::delimited::CsvOptions::default(),
    );
    accepts_canonical(envelope.payload.expect("complete CSV payload"));
}

#[cfg(feature = "serialization")]
#[test]
fn structured_text_compatibility_path_has_identical_types() {
    fn accepts_legacy(_: grist::serialization::SerializationPayload) {}
    let envelope = grist::formats::structured_text::parse_serialization(
        r#"{"topology":true}"#,
        grist::formats::structured_text::SerializationFormat::Json,
        grist::core::SourceInfo::stdin("compatibility.json"),
    );
    accepts_legacy(envelope.payload.expect("complete serialization payload"));
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::sqlite::SqliteDocument> = None;
    let _: Option<grist::sqlite::SqliteDocument> = canonical;
}

#[cfg(feature = "email-message")]
#[test]
fn mbox_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::mbox::MboxDocument> = None;
    let _: Option<grist::mbox::MboxDocument> = canonical;
}

#[cfg(feature = "email-message")]
#[test]
fn outlook_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::outlook::OutlookMsgDocument> = None;
    let _: Option<grist::outlook::OutlookMsgDocument> = canonical;
}

#[cfg(feature = "email-message")]
#[test]
fn calendar_contact_compatibility_path_has_identical_types() {
    let calendar: Option<grist::formats::calendar_contact::ICalendarDocument> = None;
    let _: Option<grist::calendar_contact::ICalendarDocument> = calendar;
    let contact: Option<grist::formats::calendar_contact::VCardDocument> = None;
    let _: Option<grist::calendar_contact::VCardDocument> = contact;
}

#[cfg(feature = "html")]
#[test]
fn html_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::html::HtmlDocument> = None;
    let _: Option<grist::html::HtmlDocument> = canonical;
}

#[cfg(feature = "latex")]
#[test]
fn latex_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::latex::LatexDocument> = None;
    let _: Option<grist::latex::LatexDocument> = canonical;
}

#[cfg(feature = "word-ooxml")]
#[test]
fn word_ooxml_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::word_ooxml::WordOoxmlDocument> = None;
    let _: Option<grist::word_ooxml::WordOoxmlDocument> = canonical;
}

#[cfg(feature = "presentation-ooxml")]
#[test]
fn presentation_ooxml_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::presentation_ooxml::PresentationOoxmlDocument> = None;
    let _: Option<grist::presentation_ooxml::PresentationOoxmlDocument> = canonical;
}

#[cfg(feature = "presentation-odf")]
#[test]
fn presentation_odf_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::presentation_odf::OdfPresentationDocument> = None;
    let _: Option<grist::presentation_odf::OdfPresentationDocument> = canonical;
}

#[cfg(feature = "odf-word")]
#[test]
fn odf_word_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::odf_word::OdfWordDocument> = None;
    let _: Option<grist::odf_word::OdfWordDocument> = canonical;
}

#[cfg(feature = "rtf")]
#[test]
fn rtf_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::rtf::RtfDocument> = None;
    let _: Option<grist::rtf::RtfDocument> = canonical;
}

#[cfg(feature = "rust")]
#[test]
fn rust_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::rust::RustFile> = None;
    let _: Option<grist::rust::RustFile> = canonical;
}

#[cfg(feature = "python")]
#[test]
fn python_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::python::PythonFile> = None;
    let _: Option<grist::python::PythonFile> = canonical;
}

#[cfg(feature = "typescript")]
#[test]
fn typescript_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::typescript::TypeScriptFile> = None;
    let _: Option<grist::typescript::TypeScriptFile> = canonical;
}

#[cfg(feature = "ldgr-projection")]
#[test]
fn ldgr_projection_compatibility_path_has_identical_types() {
    let canonical: Option<grist::formats::ldgr_projection::LdgrProjectionDocument> = None;
    let _: Option<grist::ldgr_projection::LdgrProjectionDocument> = canonical;
}
