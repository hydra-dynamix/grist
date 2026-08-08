use super::{
    Capability, FormatMetadata, OptionsMetadata, Parser, ParserContext, ParserDescriptor,
    ParserError, ParserOrigin, ParserOutput, ParserRegistry, ParserRegistryError, SchemaMetadata,
    UnavailableParser, UnavailableReason,
};
use crate::core::{ArtifactKind, Diagnostic, ParserInfo, ProviderKind};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;

struct BuiltinParser {
    parse: fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
}

impl Parser for BuiltinParser {
    fn parse(&self, context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
        (self.parse)(context)
    }
}

#[allow(dead_code)]
fn decode_options<T>(context: &ParserContext<'_>) -> Result<T, ParserError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(context.options().clone())
        .map_err(|error| Box::new(Diagnostic::malformed("grist.registry", error.to_string())))
}

#[allow(dead_code)]
fn output<T: Serialize>(envelope: crate::core::Envelope<T>) -> Result<ParserOutput, ParserError> {
    ParserOutput::from_envelope(envelope).map_err(|error| {
        Box::new(Diagnostic::parser_defect(
            "grist.registry",
            error.to_string(),
        ))
    })
}

fn descriptor(
    id: &str,
    format: FormatMetadata,
    parser: ParserInfo,
    payload_schema: &str,
    feature: Option<&str>,
    default_options: Value,
) -> ParserDescriptor {
    let mut capabilities = BTreeSet::from([Capability::NativeExtraction, Capability::TypedPayload]);
    if matches!(
        format.artifact_kind,
        ArtifactKind::Text
            | ArtifactKind::Markdown
            | ArtifactKind::RestructuredText
            | ArtifactKind::AsciiDoc
            | ArtifactKind::Html
            | ArtifactKind::Epub
            | ArtifactKind::Pdf
            | ArtifactKind::WordOoxml
            | ArtifactKind::PresentationOoxml
            | ArtifactKind::SpreadsheetOoxml
            | ArtifactKind::SpreadsheetOdf
            | ArtifactKind::PresentationOdf
            | ArtifactKind::OdfWord
            | ArtifactKind::Rtf
            | ArtifactKind::Xml
            | ArtifactKind::Latex
            | ArtifactKind::Bibliography
            | ArtifactKind::Serialization
            | ArtifactKind::StructuredBinary
            | ArtifactKind::Columnar
            | ArtifactKind::Sqlite
            | ArtifactKind::Email
            | ArtifactKind::Mbox
            | ArtifactKind::OutlookMsg
            | ArtifactKind::RustCode
            | ArtifactKind::PythonCode
            | ArtifactKind::TypeScriptCode
    ) {
        capabilities.insert(Capability::DocumentGraphProjection);
    }
    if matches!(
        format.artifact_kind,
        ArtifactKind::ModelOutput | ArtifactKind::Columnar | ArtifactKind::Mbox
    ) {
        capabilities.insert(Capability::Streaming);
    }
    if matches!(
        format.artifact_kind,
        ArtifactKind::PresentationOoxml
            | ArtifactKind::SpreadsheetOoxml
            | ArtifactKind::SpreadsheetOdf
            | ArtifactKind::PresentationOdf
            | ArtifactKind::OdfWord
            | ArtifactKind::Rtf
            | ArtifactKind::Email
            | ArtifactKind::Mbox
            | ArtifactKind::OutlookMsg
    ) {
        capabilities.insert(Capability::EmbeddedArtifacts);
    }
    ParserDescriptor {
        id: id.to_string(),
        origin: ParserOrigin::BuiltIn,
        priority: ParserDescriptor::BUILTIN_PRIORITY,
        options: OptionsMetadata::new(
            SchemaMetadata::new(
                format.id.clone() + "-options",
                "grist/".to_string() + &format.id + "-options/v1",
            ),
            default_options,
        ),
        payload_schema: SchemaMetadata::new(format.id.clone(), payload_schema),
        format,
        parser,
        required_features: feature.into_iter().map(str::to_string).collect(),
        capabilities,
        allowed_providers: BTreeSet::new(),
        required_providers: BTreeSet::new(),
    }
}

fn register(
    registry: &mut ParserRegistry,
    descriptor: ParserDescriptor,
    parse: fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
) -> Result<(), ParserRegistryError> {
    registry.register_builtin(descriptor, Arc::new(BuiltinParser { parse }))
}

pub fn builtin_parser_registry() -> Result<ParserRegistry, ParserRegistryError> {
    let mut registry = ParserRegistry::empty();
    let text_format = FormatMetadata::new("text", ArtifactKind::Text)
        .with_aliases(["plain_text"])
        .with_media_types(["text/plain"])
        .with_extensions(["txt"]);
    register(
        &mut registry,
        descriptor(
            "grist.text",
            text_format,
            crate::text::parser_info(),
            crate::core::SchemaVersion::TEXT_V2,
            None,
            serde_json::to_value(crate::text::TextOptions::default())
                .expect("text options serialize"),
        ),
        parse_text,
    )?;
    register_feature_parsers(&mut registry)?;
    register_unimplemented_formats(&mut registry)?;
    Ok(registry)
}

fn parse_text(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options: crate::text::TextOptions = decode_options(context)?;
    let mut decode_options = crate::decode::DecodeOptions::for_media_type(
        context.source().declared_mime_type.as_deref(),
        Some("text"),
    );
    decode_options.context = crate::decode::DecodeContext::PlainText;
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    let decoded = context.decoded_text_with_options(decode_options)?;
    context.consume_decoded_characters(decoded.text.chars().count() as u64)?;
    let document = crate::text::document_from_decoded(decoded);
    context.consume_nodes(document.blocks.len().saturating_add(1) as u64)?;
    let provenance = crate::text::decoding_provenance(&decoded.report);
    let mut output =
        ParserOutput::complete(serde_json::to_value(document).map_err(|error| {
            Box::new(Diagnostic::parser_defect("grist.text", error.to_string()))
        })?);
    output.provenance.push(provenance);
    Ok(output)
}

fn register_feature_parsers(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    register_markdown(registry)?;
    register_restructured_text(registry)?;
    register_asciidoc(registry)?;
    register_html(registry)?;
    register_epub(registry)?;
    register_pdf(registry)?;
    register_word_ooxml(registry)?;
    register_presentation_ooxml(registry)?;
    register_spreadsheet_ooxml(registry)?;
    register_spreadsheet_odf(registry)?;
    register_presentation_odf(registry)?;
    register_odf_word(registry)?;
    register_rtf(registry)?;
    register_xml(registry)?;
    register_csv(registry)?;
    register_latex(registry)?;
    register_bibliography(registry)?;
    register_rust(registry)?;
    register_python(registry)?;
    register_typescript(registry)?;
    register_serialization(registry)?;
    register_columnar(registry)?;
    register_sqlite(registry)?;
    register_email(registry)?;
    register_mbox(registry)?;
    register_outlook_msg(registry)?;
    register_structured_binary(registry)?;
    register_model_output(registry)?;
    register_ldgr_projection(registry)?;
    Ok(())
}

#[allow(dead_code)]
fn register_disabled(
    registry: &mut ParserRegistry,
    descriptor: ParserDescriptor,
    feature: &str,
) -> Result<(), ParserRegistryError> {
    registry.register_unavailable(UnavailableParser {
        descriptor,
        reason: UnavailableReason::FeatureDisabled {
            feature: feature.to_string(),
        },
    })
}

#[cfg(feature = "markdown")]
fn register_markdown(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("markdown", ArtifactKind::Markdown)
        .with_aliases(["md"])
        .with_media_types(["text/markdown"])
        .with_extensions(["md", "markdown"]);
    register(
        registry,
        descriptor(
            "grist.markdown",
            format,
            crate::markdown::parser_info(),
            crate::core::SchemaVersion::MARKDOWN_V2,
            Some("markdown"),
            serde_json::to_value(crate::markdown::MarkdownOptions::default())
                .expect("Markdown options serialize"),
        ),
        parse_markdown,
    )
}

#[cfg(feature = "markdown")]
fn parse_markdown(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options: crate::markdown::MarkdownOptions = decode_options(context)?;
    let mut decode_options = crate::decode::DecodeOptions::for_media_type(
        context.source().declared_mime_type.as_deref(),
        Some("markdown"),
    );
    decode_options.context = crate::decode::DecodeContext::PlainText;
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    let decoded = context.decoded_bytes_with_options(decode_options)?;
    context.consume_decoded_characters(decoded.text.chars().count() as u64)?;
    let (document, diagnostics) = crate::markdown::document_from_decoded(decoded, &options);
    context.consume_nodes(document.nodes.len().saturating_add(1) as u64)?;
    let value = serde_json::to_value(document).map_err(|error| {
        Box::new(Diagnostic::parser_defect(
            "grist.markdown",
            error.to_string(),
        ))
    })?;
    let mut output = if diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        ParserOutput::partial(Some(value), diagnostics)
    } else {
        let mut output = ParserOutput::complete(value);
        output.diagnostics = diagnostics;
        output
    };
    output
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    Ok(output)
}

#[cfg(not(feature = "markdown"))]
fn register_markdown(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("markdown", ArtifactKind::Markdown)
        .with_aliases(["md"])
        .with_extensions(["md", "markdown"]);
    let metadata = descriptor(
        "grist.markdown",
        format,
        ParserInfo::new("grist.markdown").with_feature("markdown"),
        crate::core::SchemaVersion::MARKDOWN_V2,
        Some("markdown"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "markdown")
}

#[cfg(feature = "restructured-text")]
fn register_restructured_text(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("restructured-text", ArtifactKind::RestructuredText)
        .with_aliases(["restructured_text", "rst", "rest"])
        .with_media_types(["text/x-rst", "text/restructuredtext"])
        .with_extensions(["rst", "rest"]);
    register(
        registry,
        descriptor(
            "grist.restructured_text",
            format,
            crate::restructured_text::parser_info(),
            crate::core::SchemaVersion::RESTRUCTURED_TEXT_V1,
            Some("restructured-text"),
            serde_json::to_value(crate::restructured_text::RestructuredTextOptions::default())
                .expect("reStructuredText options serialize"),
        ),
        parse_restructured_text,
    )
}

#[cfg(feature = "restructured-text")]
fn parse_restructured_text(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options: crate::restructured_text::RestructuredTextOptions = decode_options(context)?;
    let mut decode_options = crate::decode::DecodeOptions::for_media_type(
        context.source().declared_mime_type.as_deref(),
        Some("restructured_text"),
    );
    decode_options.context = crate::decode::DecodeContext::PlainText;
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    let decoded = context.decoded_bytes_with_options(decode_options)?;
    let (document, diagnostics) =
        crate::restructured_text::document_from_decoded(decoded, context.source(), &options);
    context.consume_decoded_characters(crate::restructured_text::payload_decoded_char_count(
        &document,
    ) as u64)?;
    context.consume_nodes(crate::restructured_text::payload_node_count(&document) as u64)?;
    let value = serde_json::to_value(document).map_err(|error| {
        Box::new(Diagnostic::parser_defect(
            "grist.restructured_text",
            error.to_string(),
        ))
    })?;
    let mut output = if diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        ParserOutput::partial(Some(value), diagnostics)
    } else {
        let mut output = ParserOutput::complete(value);
        output.diagnostics = diagnostics;
        output
    };
    output
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    Ok(output)
}

#[cfg(not(feature = "restructured-text"))]
fn register_restructured_text(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("restructured-text", ArtifactKind::RestructuredText)
        .with_aliases(["restructured_text", "rst", "rest"])
        .with_extensions(["rst", "rest"]);
    let metadata = descriptor(
        "grist.restructured_text",
        format,
        ParserInfo::new("grist.restructured_text").with_feature("restructured-text"),
        crate::core::SchemaVersion::RESTRUCTURED_TEXT_V1,
        Some("restructured-text"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "restructured-text")
}

#[cfg(feature = "asciidoc")]
fn register_asciidoc(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("asciidoc", ArtifactKind::AsciiDoc)
        .with_aliases(["adoc", "asc"])
        .with_media_types(["text/asciidoc", "text/x-asciidoc"])
        .with_extensions(["adoc", "asciidoc", "asc"]);
    register(
        registry,
        descriptor(
            "grist.asciidoc",
            format,
            crate::asciidoc::parser_info(),
            crate::core::SchemaVersion::ASCIIDOC_V1,
            Some("asciidoc"),
            serde_json::to_value(crate::asciidoc::AsciiDocOptions::default())
                .expect("AsciiDoc options serialize"),
        ),
        parse_asciidoc,
    )
}
#[cfg(feature = "asciidoc")]
fn parse_asciidoc(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options: crate::asciidoc::AsciiDocOptions = decode_options(context)?;
    let mut decode_options = crate::decode::DecodeOptions::for_media_type(
        context.source().declared_mime_type.as_deref(),
        Some("asciidoc"),
    );
    decode_options.context = crate::decode::DecodeContext::PlainText;
    if let Some(encoding) = &options.encoding {
        decode_options.transport_encoding = Some(encoding.clone());
    }
    let decoded = context.decoded_bytes_with_options(decode_options)?;
    let (document, diagnostics) =
        crate::asciidoc::document_from_decoded(decoded, context.source(), &options);
    context
        .consume_decoded_characters(crate::asciidoc::payload_decoded_char_count(&document) as u64)?;
    context.consume_nodes(crate::asciidoc::payload_node_count(&document) as u64)?;
    let value = serde_json::to_value(document).map_err(|error| {
        Box::new(Diagnostic::parser_defect(
            "grist.asciidoc",
            error.to_string(),
        ))
    })?;
    let mut output = if diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        ParserOutput::partial(Some(value), diagnostics)
    } else {
        let mut output = ParserOutput::complete(value);
        output.diagnostics = diagnostics;
        output
    };
    output
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    Ok(output)
}

#[cfg(not(feature = "asciidoc"))]
fn register_asciidoc(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("asciidoc", ArtifactKind::AsciiDoc)
        .with_aliases(["adoc", "asc"])
        .with_extensions(["adoc", "asciidoc", "asc"]);
    let metadata = descriptor(
        "grist.asciidoc",
        format,
        ParserInfo::new("grist.asciidoc").with_feature("asciidoc"),
        crate::core::SchemaVersion::ASCIIDOC_V1,
        Some("asciidoc"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "asciidoc")
}

#[cfg(feature = "html")]
fn register_html(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("html", ArtifactKind::Html)
        .with_aliases(["htm", "xhtml"])
        .with_media_types(["text/html", "application/xhtml+xml"])
        .with_extensions(["html", "htm", "xhtml"]);
    register(
        registry,
        descriptor(
            "grist.html",
            format,
            crate::html::parser_info(),
            crate::core::SchemaVersion::HTML_V2,
            Some("html"),
            serde_json::to_value(crate::html::HtmlOptions::default()).unwrap_or_default(),
        ),
        parse_html,
    )
}

#[cfg(feature = "html")]
fn parse_html(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::html::HtmlOptions>(context)?;
    let decoded = context
        .decoded_bytes_with_options(crate::html::decode_options(context.source(), &options))?;
    context.consume_decoded_characters(decoded.text.chars().count() as u64)?;
    let (document, diagnostics) =
        crate::html::document_from_decoded(decoded, context.source(), &options);
    context.consume_nodes(crate::html::payload_node_count(&document) as u64)?;
    let value = serde_json::to_value(document)
        .map_err(|error| Box::new(Diagnostic::parser_defect("grist.html", error.to_string())))?;
    let mut output = if diagnostics.iter().any(|diagnostic| diagnostic.partial) {
        ParserOutput::partial(Some(value), diagnostics)
    } else {
        let mut output = ParserOutput::complete(value);
        output.diagnostics = diagnostics;
        output
    };
    output
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    Ok(output)
}

#[cfg(feature = "epub")]
fn register_epub(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("epub", ArtifactKind::Epub)
        .with_media_types(["application/epub+zip"])
        .with_extensions(["epub"]);
    register(
        registry,
        descriptor(
            "grist.epub",
            format,
            crate::epub::parser_info(),
            crate::core::SchemaVersion::EPUB_V1,
            Some("epub"),
            serde_json::to_value(crate::epub::EpubOptions::default()).unwrap_or_default(),
        ),
        crate::epub::parse_registered,
    )
}

#[cfg(not(feature = "epub"))]
fn register_epub(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("epub", ArtifactKind::Epub)
        .with_media_types(["application/epub+zip"])
        .with_extensions(["epub"]);
    let metadata = descriptor(
        "grist.epub",
        format,
        ParserInfo::new("grist.epub").with_feature("epub"),
        crate::core::SchemaVersion::EPUB_V1,
        Some("epub"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "epub")
}

#[cfg(feature = "pdf")]
fn register_pdf(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("pdf", ArtifactKind::Pdf)
        .with_media_types(["application/pdf"])
        .with_extensions(["pdf"]);
    let mut metadata = descriptor(
        "grist.pdf",
        format,
        crate::pdf::parser_info(),
        crate::core::SchemaVersion::PDF_V1,
        Some("pdf"),
        serde_json::to_value(crate::pdf::PdfOptions::default()).unwrap_or_default(),
    );
    metadata
        .allowed_providers
        .insert(crate::core::ProviderKind::Ocr);
    metadata
        .capabilities
        .insert(Capability::ProviderDerivedContent);
    register(registry, metadata, crate::pdf::parse_registered)
}

#[cfg(feature = "word-ooxml")]
fn register_word_ooxml(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let definitions = [
        (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            crate::word_ooxml::parse_docx_registered
                as fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
        ),
        (
            "docm",
            "application/vnd.ms-word.document.macroEnabled.12",
            crate::word_ooxml::parse_docm_registered,
        ),
        (
            "dotx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
            crate::word_ooxml::parse_dotx_registered,
        ),
        (
            "dotm",
            "application/vnd.ms-word.template.macroEnabled.12",
            crate::word_ooxml::parse_dotm_registered,
        ),
    ];
    for (id, media_type, parse) in definitions {
        let format = FormatMetadata::new(id, ArtifactKind::WordOoxml)
            .with_media_types([media_type])
            .with_extensions([id]);
        register(
            registry,
            descriptor(
                &format!("grist.{id}"),
                format,
                crate::word_ooxml::parser_info(),
                crate::core::SchemaVersion::WORD_OOXML_V1,
                Some("word-ooxml"),
                serde_json::to_value(crate::word_ooxml::WordOoxmlOptions::default())
                    .unwrap_or_default(),
            ),
            parse,
        )?;
    }
    Ok(())
}

#[cfg(feature = "presentation-ooxml")]
fn register_presentation_ooxml(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let definitions = [
        (
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            crate::presentation_ooxml::parse_pptx_registered
                as fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
        ),
        (
            "pptm",
            "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
            crate::presentation_ooxml::parse_pptm_registered,
        ),
        (
            "potx",
            "application/vnd.openxmlformats-officedocument.presentationml.template",
            crate::presentation_ooxml::parse_potx_registered,
        ),
        (
            "ppsx",
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
            crate::presentation_ooxml::parse_ppsx_registered,
        ),
    ];
    for (id, media_type, parse) in definitions {
        let format = FormatMetadata::new(id, ArtifactKind::PresentationOoxml)
            .with_media_types([media_type])
            .with_extensions([id]);
        register(
            registry,
            descriptor(
                &format!("grist.{id}"),
                format,
                crate::presentation_ooxml::parser_info(),
                crate::core::SchemaVersion::PRESENTATION_OOXML_V1,
                Some("presentation-ooxml"),
                serde_json::to_value(
                    crate::presentation_ooxml::PresentationOoxmlOptions::default(),
                )
                .unwrap_or_default(),
            ),
            parse,
        )?;
    }
    Ok(())
}

#[cfg(feature = "presentation-odf")]
fn register_presentation_odf(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let definitions = [
        (
            "odp",
            "application/vnd.oasis.opendocument.presentation",
            crate::presentation_odf::parse_odp_registered
                as fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
        ),
        (
            "otp",
            "application/vnd.oasis.opendocument.presentation-template",
            crate::presentation_odf::parse_otp_registered,
        ),
    ];
    for (id, media_type, parse) in definitions {
        let format = FormatMetadata::new(id, ArtifactKind::PresentationOdf)
            .with_media_types([media_type])
            .with_extensions([id]);
        register(
            registry,
            descriptor(
                &format!("grist.{id}"),
                format,
                crate::presentation_odf::parser_info(),
                crate::core::SchemaVersion::PRESENTATION_ODF_V1,
                Some("presentation-odf"),
                serde_json::to_value(crate::presentation_odf::OdfPresentationOptions::default())
                    .unwrap_or_default(),
            ),
            parse,
        )?;
    }
    Ok(())
}

#[cfg(feature = "spreadsheet-ooxml")]
fn register_spreadsheet_ooxml(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let definitions = [
        (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            crate::spreadsheet_ooxml::parse_xlsx_registered
                as fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
        ),
        (
            "xlsm",
            "application/vnd.ms-excel.sheet.macroEnabled.12",
            crate::spreadsheet_ooxml::parse_xlsm_registered,
        ),
    ];
    for (id, media_type, parse) in definitions {
        let format = FormatMetadata::new(id, ArtifactKind::SpreadsheetOoxml)
            .with_media_types([media_type])
            .with_extensions([id]);
        register(
            registry,
            descriptor(
                &format!("grist.{id}"),
                format,
                crate::spreadsheet_ooxml::parser_info(),
                crate::core::SchemaVersion::SPREADSHEET_OOXML_V1,
                Some("spreadsheet-ooxml"),
                serde_json::to_value(crate::spreadsheet_ooxml::SpreadsheetOoxmlOptions::default())
                    .unwrap_or_default(),
            ),
            parse,
        )?;
    }
    Ok(())
}

#[cfg(feature = "spreadsheet-odf")]
fn register_spreadsheet_odf(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let definitions = [
        (
            "ods",
            "application/vnd.oasis.opendocument.spreadsheet",
            crate::spreadsheet_odf::parse_ods_registered
                as fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
        ),
        (
            "ots",
            "application/vnd.oasis.opendocument.spreadsheet-template",
            crate::spreadsheet_odf::parse_ots_registered,
        ),
    ];
    for (id, media_type, parse) in definitions {
        let format = FormatMetadata::new(id, ArtifactKind::SpreadsheetOdf)
            .with_media_types([media_type])
            .with_extensions([id]);
        register(
            registry,
            descriptor(
                &format!("grist.{id}"),
                format,
                crate::spreadsheet_odf::parser_info(),
                crate::core::SchemaVersion::SPREADSHEET_ODF_V1,
                Some("spreadsheet-odf"),
                serde_json::to_value(crate::spreadsheet_odf::SpreadsheetOdfOptions::default())
                    .unwrap_or_default(),
            ),
            parse,
        )?;
    }
    Ok(())
}

#[cfg(not(feature = "spreadsheet-odf"))]
fn register_spreadsheet_odf(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, media_type) in [
        ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
        (
            "ots",
            "application/vnd.oasis.opendocument.spreadsheet-template",
        ),
    ] {
        let format = FormatMetadata::new(id, ArtifactKind::SpreadsheetOdf)
            .with_media_types([media_type])
            .with_extensions([id]);
        let metadata = descriptor(
            &format!("grist.{id}"),
            format,
            ParserInfo::new("grist.spreadsheet_odf").with_feature("spreadsheet-odf"),
            crate::core::SchemaVersion::SPREADSHEET_ODF_V1,
            Some("spreadsheet-odf"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "spreadsheet-odf")?;
    }
    Ok(())
}

#[cfg(not(feature = "spreadsheet-ooxml"))]
fn register_spreadsheet_ooxml(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, media_type) in [
        (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        ("xlsm", "application/vnd.ms-excel.sheet.macroEnabled.12"),
    ] {
        let format = FormatMetadata::new(id, ArtifactKind::SpreadsheetOoxml)
            .with_media_types([media_type])
            .with_extensions([id]);
        let metadata = descriptor(
            &format!("grist.{id}"),
            format,
            ParserInfo::new("grist.spreadsheet_ooxml").with_feature("spreadsheet-ooxml"),
            crate::core::SchemaVersion::SPREADSHEET_OOXML_V1,
            Some("spreadsheet-ooxml"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "spreadsheet-ooxml")?;
    }
    Ok(())
}

#[cfg(not(feature = "presentation-odf"))]
fn register_presentation_odf(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, media_type) in [
        ("odp", "application/vnd.oasis.opendocument.presentation"),
        (
            "otp",
            "application/vnd.oasis.opendocument.presentation-template",
        ),
    ] {
        let format = FormatMetadata::new(id, ArtifactKind::PresentationOdf)
            .with_media_types([media_type])
            .with_extensions([id]);
        let metadata = descriptor(
            &format!("grist.{id}"),
            format,
            ParserInfo::new("grist.presentation_odf").with_feature("presentation-odf"),
            crate::core::SchemaVersion::PRESENTATION_ODF_V1,
            Some("presentation-odf"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "presentation-odf")?;
    }
    Ok(())
}

#[cfg(feature = "odf-word")]
fn register_odf_word(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let definitions = [
        (
            "odt",
            "application/vnd.oasis.opendocument.text",
            crate::odf_word::parse_odt_registered
                as fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
        ),
        (
            "ott",
            "application/vnd.oasis.opendocument.text-template",
            crate::odf_word::parse_ott_registered,
        ),
    ];
    for (id, media_type, parse) in definitions {
        let format = FormatMetadata::new(id, ArtifactKind::OdfWord)
            .with_media_types([media_type])
            .with_extensions([id]);
        register(
            registry,
            descriptor(
                &format!("grist.{id}"),
                format,
                crate::odf_word::parser_info(),
                crate::core::SchemaVersion::ODF_WORD_V1,
                Some("odf-word"),
                serde_json::to_value(crate::odf_word::OdfWordOptions::default())
                    .unwrap_or_default(),
            ),
            parse,
        )?;
    }
    Ok(())
}

#[cfg(feature = "rtf")]
fn register_rtf(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("rtf", ArtifactKind::Rtf)
        .with_aliases(["rich_text_format"])
        .with_media_types(["application/rtf", "text/rtf"])
        .with_extensions(["rtf"]);
    register(
        registry,
        descriptor(
            "grist.rtf",
            format,
            crate::rtf::parser_info(),
            crate::core::SchemaVersion::RTF_V1,
            Some("rtf"),
            serde_json::to_value(crate::rtf::RtfOptions::default()).unwrap_or_default(),
        ),
        crate::rtf::parse_registered,
    )
}

#[cfg(not(feature = "rtf"))]
fn register_rtf(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("rtf", ArtifactKind::Rtf)
        .with_aliases(["rich_text_format"])
        .with_media_types(["application/rtf", "text/rtf"])
        .with_extensions(["rtf"]);
    let metadata = descriptor(
        "grist.rtf",
        format,
        ParserInfo::new("grist.rtf").with_feature("rtf"),
        crate::core::SchemaVersion::RTF_V1,
        Some("rtf"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "rtf")
}

#[cfg(not(feature = "odf-word"))]
fn register_odf_word(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, media_type) in [
        ("odt", "application/vnd.oasis.opendocument.text"),
        ("ott", "application/vnd.oasis.opendocument.text-template"),
    ] {
        let format = FormatMetadata::new(id, ArtifactKind::OdfWord)
            .with_media_types([media_type])
            .with_extensions([id]);
        let metadata = descriptor(
            &format!("grist.{id}"),
            format,
            ParserInfo::new("grist.odf_word").with_feature("odf-word"),
            crate::core::SchemaVersion::ODF_WORD_V1,
            Some("odf-word"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "odf-word")?;
    }
    Ok(())
}

#[cfg(not(feature = "word-ooxml"))]
fn register_word_ooxml(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, media_type) in [
        (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        ("docm", "application/vnd.ms-word.document.macroEnabled.12"),
        (
            "dotx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
        ),
        ("dotm", "application/vnd.ms-word.template.macroEnabled.12"),
    ] {
        let format = FormatMetadata::new(id, ArtifactKind::WordOoxml)
            .with_media_types([media_type])
            .with_extensions([id]);
        let metadata = descriptor(
            &format!("grist.{id}"),
            format,
            ParserInfo::new("grist.word_ooxml").with_feature("word-ooxml"),
            crate::core::SchemaVersion::WORD_OOXML_V1,
            Some("word-ooxml"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "word-ooxml")?;
    }
    Ok(())
}

#[cfg(not(feature = "presentation-ooxml"))]
fn register_presentation_ooxml(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, media_type) in [
        (
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        (
            "pptm",
            "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
        ),
        (
            "potx",
            "application/vnd.openxmlformats-officedocument.presentationml.template",
        ),
        (
            "ppsx",
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
        ),
    ] {
        let format = FormatMetadata::new(id, ArtifactKind::PresentationOoxml)
            .with_media_types([media_type])
            .with_extensions([id]);
        let metadata = descriptor(
            &format!("grist.{id}"),
            format,
            ParserInfo::new("grist.presentation_ooxml").with_feature("presentation-ooxml"),
            crate::core::SchemaVersion::PRESENTATION_OOXML_V1,
            Some("presentation-ooxml"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "presentation-ooxml")?;
    }
    Ok(())
}

#[cfg(not(feature = "pdf"))]
fn register_pdf(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("pdf", ArtifactKind::Pdf)
        .with_media_types(["application/pdf"])
        .with_extensions(["pdf"]);
    let mut metadata = descriptor(
        "grist.pdf",
        format,
        ParserInfo::new("grist.pdf").with_feature("pdf"),
        crate::core::SchemaVersion::PDF_V1,
        Some("pdf"),
        serde_json::json!({}),
    );
    metadata
        .allowed_providers
        .insert(crate::core::ProviderKind::Ocr);
    metadata
        .capabilities
        .insert(Capability::ProviderDerivedContent);
    register_disabled(registry, metadata, "pdf")
}
#[cfg(not(feature = "html"))]
fn register_html(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("html", ArtifactKind::Html)
        .with_aliases(["htm", "xhtml"])
        .with_extensions(["html", "htm", "xhtml"]);
    let metadata = descriptor(
        "grist.html",
        format,
        ParserInfo::new("grist.html").with_feature("html"),
        crate::core::SchemaVersion::HTML_V2,
        Some("html"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "html")
}

#[cfg(feature = "xml")]
fn register_xml(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("xml", ArtifactKind::Xml)
        .with_aliases(["jats", "nxml"])
        .with_media_types(["application/xml", "text/xml", "application/jats+xml"])
        .with_extensions(["xml", "jats", "nxml"]);
    register(
        registry,
        descriptor(
            "grist.xml",
            format,
            crate::xml::parser_info(),
            crate::core::SchemaVersion::XML_V1,
            Some("xml"),
            serde_json::to_value(crate::xml::XmlOptions::default()).unwrap_or_default(),
        ),
        parse_xml,
    )
}
#[cfg(feature = "xml")]
fn parse_xml(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::xml::XmlOptions>(context)?;
    let decoded = context
        .decoded_bytes_with_options(crate::xml::decode_options(context.source(), &options))?;
    context.consume_decoded_characters(decoded.text.chars().count() as u64)?;
    let (document, diagnostics) =
        crate::xml::document_from_decoded(decoded, context.source(), &options);
    context.consume_nodes(crate::xml::payload_node_count(&document) as u64)?;
    let value = serde_json::to_value(document)
        .map_err(|error| Box::new(Diagnostic::parser_defect("grist.xml", error.to_string())))?;
    let mut output = if diagnostics.iter().any(|d| d.partial) {
        ParserOutput::partial(Some(value), diagnostics)
    } else {
        let mut output = ParserOutput::complete(value);
        output.diagnostics = diagnostics;
        output
    };
    output
        .provenance
        .push(crate::text::decoding_provenance(&decoded.report));
    Ok(output)
}
#[cfg(not(feature = "xml"))]
fn register_xml(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("xml", ArtifactKind::Xml)
        .with_aliases(["jats", "nxml"])
        .with_media_types(["application/xml", "text/xml", "application/jats+xml"])
        .with_extensions(["xml", "jats", "nxml"]);
    let metadata = descriptor(
        "grist.xml",
        format,
        ParserInfo::new("grist.xml").with_feature("xml"),
        crate::core::SchemaVersion::XML_V1,
        Some("xml"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "xml")
}
#[cfg(feature = "csv")]
fn register_csv(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("csv", ArtifactKind::Csv)
        .with_aliases(["tsv"])
        .with_media_types(["text/csv", "text/tab-separated-values"])
        .with_extensions(["csv", "tsv"]);
    register(
        registry,
        descriptor(
            "grist.csv",
            format,
            ParserInfo::new("grist.csv")
                .with_implementation("grist-delimited-scanner", crate::version())
                .with_specification_version("RFC 4180-compatible")
                .with_feature("csv"),
            crate::core::SchemaVersion::CSV_V2,
            Some("csv"),
            serde_json::to_value(crate::csv::CsvOptions::default()).unwrap_or_default(),
        ),
        parse_csv,
    )
}

#[cfg(feature = "csv")]
fn parse_csv(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::csv::CsvOptions>(context)?;
    let text = context.utf8_text()?;
    context.consume_decoded_characters(text.chars().count() as u64)?;
    output(crate::csv::parse_csv_with_control(
        text,
        context.source().clone(),
        &options,
        context.control(),
    ))
}

#[cfg(not(feature = "csv"))]
fn register_csv(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("csv", ArtifactKind::Csv)
        .with_aliases(["tsv"])
        .with_extensions(["csv", "tsv"]);
    let metadata = descriptor(
        "grist.csv",
        format,
        ParserInfo::new("grist.csv").with_feature("csv"),
        crate::core::SchemaVersion::CSV_V2,
        Some("csv"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "csv")
}

#[cfg(feature = "latex")]
fn register_latex(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("latex", ArtifactKind::Latex)
        .with_aliases(["tex"])
        .with_extensions(["tex", "latex"])
        .with_media_types(["application/x-latex", "text/x-tex"]);
    register(
        registry,
        descriptor(
            "grist.latex",
            format,
            crate::latex::parser_info(),
            crate::core::SchemaVersion::LATEX_V1,
            Some("latex"),
            serde_json::to_value(crate::latex::LatexOptions::default()).unwrap_or_default(),
        ),
        parse_latex,
    )
}

#[cfg(feature = "bibliography")]
fn register_bibliography(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("bibtex", ArtifactKind::Bibliography)
        .with_aliases(["biblatex", "bibliography", "bib"])
        .with_extensions(["bib"])
        .with_media_types(["application/x-bibtex", "text/x-bibtex"]);
    register(
        registry,
        descriptor(
            "grist.bibliography",
            format,
            crate::bibliography::parser_info(),
            crate::core::SchemaVersion::BIBLIOGRAPHY_V1,
            Some("bibliography"),
            serde_json::to_value(crate::bibliography::BibliographyOptions::default())
                .unwrap_or_default(),
        ),
        parse_bibliography,
    )
}

#[cfg(feature = "bibliography")]
fn parse_bibliography(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::bibliography::BibliographyOptions>(context)?;
    let envelope = crate::bibliography::parse_bibliography_bytes(
        context.bytes(),
        context.source().clone(),
        &options,
    );
    if let Some(payload) = envelope.payload.as_ref() {
        context.consume_decoded_characters(payload.decoded_text.chars().count() as u64)?;
        context.consume_nodes(
            payload
                .entries
                .iter()
                .map(|entry| entry.fields.len().saturating_add(1))
                .sum::<usize>()
                .saturating_add(payload.constructs.len()) as u64,
        )?;
    }
    output(envelope)
}

#[cfg(not(feature = "bibliography"))]
fn register_bibliography(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("bibtex", ArtifactKind::Bibliography)
        .with_aliases(["biblatex", "bibliography", "bib"])
        .with_extensions(["bib"]);
    let metadata = descriptor(
        "grist.bibliography",
        format,
        ParserInfo::new("grist.bibliography").with_feature("bibliography"),
        crate::core::SchemaVersion::BIBLIOGRAPHY_V1,
        Some("bibliography"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "bibliography")
}

#[cfg(feature = "latex")]
fn parse_latex(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::latex::LatexOptions>(context)?;
    let envelope =
        crate::latex::parse_latex_bytes(context.bytes(), context.source().clone(), &options);
    let decoded_characters = envelope
        .payload
        .as_ref()
        .map(|payload| payload.decoded_text.chars().count() as u64)
        .unwrap_or(0);
    context.consume_decoded_characters(decoded_characters)?;
    output(envelope)
}

#[cfg(not(feature = "latex"))]
fn register_latex(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("latex", ArtifactKind::Latex).with_extensions(["tex"]);
    let metadata = descriptor(
        "grist.latex",
        format,
        ParserInfo::new("grist.latex").with_feature("latex"),
        crate::core::SchemaVersion::LATEX_V1,
        Some("latex"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "latex")
}

#[cfg(feature = "rust")]
fn register_rust(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("rust", ArtifactKind::RustCode)
        .with_media_types(["text/x-rust"])
        .with_extensions(["rs"]);
    register(
        registry,
        descriptor(
            "tree-sitter-rust",
            format,
            ParserInfo::new("tree-sitter-rust")
                .with_implementation("tree-sitter-rust", "0.23.3")
                .with_grammar_version("0.23.3")
                .with_feature("rust"),
            crate::core::SchemaVersion::RUST_CODE_V1,
            Some("rust"),
            serde_json::to_value(crate::rust::RustIngestOptions::default()).unwrap_or_default(),
        ),
        parse_rust,
    )
}

#[cfg(feature = "rust")]
fn parse_rust(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::rust::RustIngestOptions>(context)?;
    let text = context.utf8_text()?;
    context.consume_decoded_characters(text.chars().count() as u64)?;
    output(crate::rust::parse_rust(
        text,
        context.source().clone(),
        &options,
    ))
}

#[cfg(not(feature = "rust"))]
fn register_rust(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("rust", ArtifactKind::RustCode).with_extensions(["rs"]);
    let metadata = descriptor(
        "tree-sitter-rust",
        format,
        ParserInfo::new("tree-sitter-rust").with_feature("rust"),
        crate::core::SchemaVersion::RUST_CODE_V1,
        Some("rust"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "rust")
}

#[cfg(feature = "python")]
fn register_python(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("python", ArtifactKind::PythonCode)
        .with_media_types(["text/x-python"])
        .with_extensions(["py", "pyi"]);
    register(
        registry,
        descriptor(
            "tree-sitter-python",
            format,
            ParserInfo::new("tree-sitter-python")
                .with_implementation("tree-sitter-python", "0.23.6")
                .with_grammar_version("0.23.6")
                .with_feature("python"),
            crate::core::SchemaVersion::PYTHON_CODE_V1,
            Some("python"),
            serde_json::to_value(crate::python::PythonIngestOptions::default()).unwrap_or_default(),
        ),
        parse_python,
    )
}

#[cfg(feature = "python")]
fn parse_python(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::python::PythonIngestOptions>(context)?;
    let text = context.utf8_text()?;
    context.consume_decoded_characters(text.chars().count() as u64)?;
    output(crate::python::parse_python(
        text,
        context.source().clone(),
        &options,
    ))
}

#[cfg(not(feature = "python"))]
fn register_python(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("python", ArtifactKind::PythonCode).with_extensions(["py"]);
    let metadata = descriptor(
        "tree-sitter-python",
        format,
        ParserInfo::new("tree-sitter-python").with_feature("python"),
        crate::core::SchemaVersion::PYTHON_CODE_V1,
        Some("python"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "python")
}

#[cfg(feature = "typescript")]
fn register_typescript(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    register_typescript_dialect(
        registry,
        "typescript",
        "tree-sitter-typescript",
        "ts",
        crate::typescript::TypeScriptDialect::TypeScript,
    )?;
    register_typescript_dialect(
        registry,
        "tsx",
        "tree-sitter-tsx",
        "tsx",
        crate::typescript::TypeScriptDialect::Tsx,
    )?;
    register_typescript_dialect(
        registry,
        "jsx",
        "tree-sitter-jsx",
        "jsx",
        crate::typescript::TypeScriptDialect::Jsx,
    )
}

#[cfg(feature = "typescript")]
fn register_typescript_dialect(
    registry: &mut ParserRegistry,
    format_id: &str,
    parser_id: &str,
    extension: &str,
    dialect: crate::typescript::TypeScriptDialect,
) -> Result<(), ParserRegistryError> {
    let format =
        FormatMetadata::new(format_id, ArtifactKind::TypeScriptCode).with_extensions([extension]);
    let options = crate::typescript::TypeScriptIngestOptions {
        dialect,
        ..Default::default()
    };
    register(
        registry,
        descriptor(
            parser_id,
            format,
            ParserInfo::new(parser_id)
                .with_implementation(parser_id, "0.23.2")
                .with_grammar_version("0.23.2")
                .with_feature("typescript"),
            crate::core::SchemaVersion::TYPESCRIPT_CODE_V1,
            Some("typescript"),
            serde_json::to_value(options).unwrap_or_default(),
        ),
        parse_typescript,
    )
}

#[cfg(feature = "typescript")]
fn parse_typescript(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::typescript::TypeScriptIngestOptions>(context)?;
    let text = context.utf8_text()?;
    context.consume_decoded_characters(text.chars().count() as u64)?;
    output(crate::typescript::parse_typescript(
        text,
        context.source().clone(),
        &options,
    ))
}

#[cfg(not(feature = "typescript"))]
fn register_typescript(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (format_id, parser_id, extension) in [
        ("typescript", "tree-sitter-typescript", "ts"),
        ("tsx", "tree-sitter-tsx", "tsx"),
        ("jsx", "tree-sitter-jsx", "jsx"),
    ] {
        let format = FormatMetadata::new(format_id, ArtifactKind::TypeScriptCode)
            .with_extensions([extension]);
        let metadata = descriptor(
            parser_id,
            format,
            ParserInfo::new(parser_id).with_feature("typescript"),
            crate::core::SchemaVersion::TYPESCRIPT_CODE_V1,
            Some("typescript"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "typescript")?;
    }
    Ok(())
}

#[cfg(feature = "columnar")]
fn register_columnar(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, format, extensions, media, parser) in [
        (
            "arrow",
            crate::columnar::ColumnarFormat::ArrowIpcStream,
            vec!["arrow", "feather"],
            vec![
                "application/vnd.apache.arrow.stream",
                "application/vnd.apache.arrow.file",
            ],
            parse_arrow as fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
        ),
        (
            "parquet",
            crate::columnar::ColumnarFormat::Parquet,
            vec!["parquet"],
            vec!["application/vnd.apache.parquet"],
            parse_parquet,
        ),
    ] {
        let metadata = FormatMetadata::new(id, ArtifactKind::Columnar)
            .with_extensions(extensions)
            .with_media_types(media);
        register(
            registry,
            descriptor(
                &format!("grist.columnar.{id}"),
                metadata,
                crate::columnar::parser_info(format),
                crate::core::SchemaVersion::COLUMNAR_V1,
                Some("columnar"),
                serde_json::to_value(crate::columnar::ColumnarOptions::default())
                    .unwrap_or_default(),
            ),
            parser,
        )?;
    }
    Ok(())
}

#[cfg(feature = "sqlite")]
fn register_sqlite(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("sqlite", ArtifactKind::Sqlite)
        .with_aliases(["sqlite3"])
        .with_extensions(["sqlite", "sqlite3", "db"])
        .with_media_types(["application/vnd.sqlite3", "application/x-sqlite3"]);
    register(
        registry,
        descriptor(
            "grist.sqlite",
            format,
            crate::sqlite::parser_info(),
            crate::core::SchemaVersion::SQLITE_V1,
            Some("sqlite"),
            serde_json::to_value(crate::sqlite::SqliteOptions::default()).unwrap_or_default(),
        ),
        parse_sqlite,
    )
}

#[cfg(feature = "sqlite")]
fn parse_sqlite(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::sqlite::SqliteOptions>(context)?;
    output(crate::sqlite::parse_sqlite_with_operation_control(
        context.bytes(),
        context.source().clone(),
        &options,
        context.control(),
    ))
}

#[cfg(feature = "email-message")]
fn register_email(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("eml", ArtifactKind::Email)
        .with_aliases(["email", "rfc5322", "message_rfc822"])
        .with_extensions(["eml"])
        .with_media_types(["message/rfc822"]);
    register(
        registry,
        descriptor(
            "grist.email",
            format,
            crate::email::parser_info(),
            crate::core::SchemaVersion::EMAIL_V1,
            Some("email-message"),
            serde_json::to_value(crate::email::EmailOptions::default()).unwrap_or_default(),
        ),
        parse_email,
    )
}

#[cfg(feature = "email-message")]
fn parse_email(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::email::EmailOptions>(context)?;
    output(crate::email::parse_email_with_operation_control(
        context.bytes(),
        context.source().clone(),
        &options,
        context.control(),
    ))
}

#[cfg(feature = "email-message")]
fn register_mbox(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("mbox", ArtifactKind::Mbox)
        .with_aliases(["mailbox", "application_mbox"])
        .with_extensions(["mbox"])
        .with_media_types(["application/mbox"]);
    register(
        registry,
        descriptor(
            "grist.mbox",
            format,
            crate::mbox::parser_info(),
            crate::core::SchemaVersion::MBOX_V1,
            Some("email-message"),
            serde_json::to_value(crate::mbox::MboxOptions::default()).unwrap_or_default(),
        ),
        parse_mbox,
    )
}

#[cfg(feature = "email-message")]
fn parse_mbox(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::mbox::MboxOptions>(context)?;
    output(crate::mbox::parse_mbox_with_operation_control(
        context.bytes(),
        context.source().clone(),
        &options,
        context.control(),
    ))
}

#[cfg(feature = "email-message")]
fn register_outlook_msg(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("msg", ArtifactKind::OutlookMsg)
        .with_aliases(["outlook_msg", "application_vnd_ms_outlook"])
        .with_extensions(["msg"])
        .with_media_types(["application/vnd.ms-outlook"]);
    register(
        registry,
        descriptor(
            "grist.outlook.msg",
            format,
            crate::outlook::parser_info(),
            crate::core::SchemaVersion::OUTLOOK_MSG_V1,
            Some("email-message"),
            serde_json::to_value(crate::outlook::OutlookMsgOptions::default()).unwrap_or_default(),
        ),
        parse_outlook_msg,
    )
}

#[cfg(feature = "email-message")]
fn parse_outlook_msg(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::outlook::OutlookMsgOptions>(context)?;
    output(crate::outlook::parse_outlook_msg_with_operation_control(
        context.bytes(),
        context.source().clone(),
        &options,
        context.control(),
    ))
}

#[cfg(not(feature = "email-message"))]
fn register_outlook_msg(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let metadata = descriptor(
        "grist.outlook.msg",
        FormatMetadata::new("msg", ArtifactKind::OutlookMsg)
            .with_aliases(["outlook_msg", "application_vnd_ms_outlook"])
            .with_extensions(["msg"])
            .with_media_types(["application/vnd.ms-outlook"]),
        ParserInfo::new("grist.outlook.msg").with_feature("email-message"),
        crate::core::SchemaVersion::OUTLOOK_MSG_V1,
        Some("email-message"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "email-message")
}

#[cfg(not(feature = "email-message"))]
fn register_mbox(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let metadata = descriptor(
        "grist.mbox",
        FormatMetadata::new("mbox", ArtifactKind::Mbox)
            .with_aliases(["mailbox", "application_mbox"])
            .with_extensions(["mbox"])
            .with_media_types(["application/mbox"]),
        ParserInfo::new("grist.mbox").with_feature("email-message"),
        crate::core::SchemaVersion::MBOX_V1,
        Some("email-message"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "email-message")
}

#[cfg(not(feature = "email-message"))]
fn register_email(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let metadata = descriptor(
        "grist.email",
        FormatMetadata::new("eml", ArtifactKind::Email)
            .with_aliases(["email", "rfc5322", "message_rfc822"])
            .with_extensions(["eml"])
            .with_media_types(["message/rfc822"]),
        ParserInfo::new("grist.email").with_feature("email-message"),
        crate::core::SchemaVersion::EMAIL_V1,
        Some("email-message"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "email-message")
}

#[cfg(not(feature = "sqlite"))]
fn register_sqlite(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let metadata = descriptor(
        "grist.sqlite",
        FormatMetadata::new("sqlite", ArtifactKind::Sqlite)
            .with_aliases(["sqlite3"])
            .with_extensions(["sqlite", "sqlite3", "db"]),
        ParserInfo::new("grist.sqlite").with_feature("sqlite"),
        crate::core::SchemaVersion::SQLITE_V1,
        Some("sqlite"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "sqlite")
}
#[cfg(feature = "columnar")]
fn parse_arrow(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::columnar::ColumnarOptions>(context)?;
    output(crate::columnar::parse_columnar_with_operation_control(
        context.bytes(),
        None,
        context.source().clone(),
        &options,
        context.control(),
    ))
}
#[cfg(feature = "columnar")]
fn parse_parquet(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::columnar::ColumnarOptions>(context)?;
    output(crate::columnar::parse_columnar_with_operation_control(
        context.bytes(),
        Some(crate::columnar::ColumnarFormat::Parquet),
        context.source().clone(),
        &options,
        context.control(),
    ))
}
#[cfg(not(feature = "columnar"))]
fn register_columnar(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, extensions) in [
        ("arrow", vec!["arrow", "feather"]),
        ("parquet", vec!["parquet"]),
    ] {
        let metadata = descriptor(
            &format!("grist.columnar.{id}"),
            FormatMetadata::new(id, ArtifactKind::Columnar).with_extensions(extensions),
            ParserInfo::new("grist.columnar").with_feature("columnar"),
            crate::core::SchemaVersion::COLUMNAR_V1,
            Some("columnar"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "columnar")?;
    }
    Ok(())
}

#[cfg(feature = "serialization")]
fn register_serialization(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    register_serialization_format(registry, "json", "json", parse_json)?;
    register_serialization_format(registry, "jsonl", "jsonl", parse_jsonl)?;
    register_serialization_format(registry, "yaml", "yaml", parse_yaml)?;
    register_serialization_format(registry, "toml", "toml", parse_toml)?;
    Ok(())
}

#[cfg(feature = "structured-binary")]
fn register_structured_binary(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let formats = [
        (
            "cbor",
            FormatMetadata::new("cbor", ArtifactKind::StructuredBinary)
                .with_media_types(["application/cbor"])
                .with_extensions(["cbor"]),
            crate::structured_binary::StructuredBinaryFormat::Cbor,
        ),
        (
            "messagepack",
            FormatMetadata::new("messagepack", ArtifactKind::StructuredBinary)
                .with_aliases(["msgpack", "message-pack"])
                .with_media_types(["application/msgpack", "application/x-msgpack"])
                .with_extensions(["msgpack", "mpk"]),
            crate::structured_binary::StructuredBinaryFormat::MessagePack,
        ),
        (
            "protobuf",
            FormatMetadata::new("protobuf", ArtifactKind::StructuredBinary)
                .with_aliases(["protocol-buffers", "proto-binary"])
                .with_media_types(["application/x-protobuf", "application/protobuf"])
                .with_extensions(["pb", "protobuf"]),
            crate::structured_binary::StructuredBinaryFormat::Protobuf,
        ),
    ];
    for (id, format, binary_format) in formats {
        let parse = match binary_format {
            crate::structured_binary::StructuredBinaryFormat::Cbor => parse_cbor,
            crate::structured_binary::StructuredBinaryFormat::MessagePack => parse_messagepack,
            crate::structured_binary::StructuredBinaryFormat::Protobuf => parse_protobuf,
        };
        register(
            registry,
            descriptor(
                &format!("grist.structured-binary.{id}"),
                format,
                crate::structured_binary::parser_info(binary_format),
                crate::core::SchemaVersion::STRUCTURED_BINARY_V1,
                Some("structured-binary"),
                serde_json::to_value(crate::structured_binary::StructuredBinaryOptions::default())
                    .unwrap_or_default(),
            ),
            parse,
        )?;
    }
    Ok(())
}

#[cfg(feature = "structured-binary")]
fn parse_structured_binary(
    context: &mut ParserContext<'_>,
    format: crate::structured_binary::StructuredBinaryFormat,
) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::structured_binary::StructuredBinaryOptions>(context)?;
    output(
        crate::structured_binary::parse_structured_binary_with_operation_control(
            context.bytes(),
            format,
            context.source().clone(),
            &options,
            context.control(),
        ),
    )
}

#[cfg(feature = "structured-binary")]
fn parse_cbor(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    parse_structured_binary(
        context,
        crate::structured_binary::StructuredBinaryFormat::Cbor,
    )
}

#[cfg(feature = "structured-binary")]
fn parse_messagepack(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    parse_structured_binary(
        context,
        crate::structured_binary::StructuredBinaryFormat::MessagePack,
    )
}

#[cfg(feature = "structured-binary")]
fn parse_protobuf(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    parse_structured_binary(
        context,
        crate::structured_binary::StructuredBinaryFormat::Protobuf,
    )
}

#[cfg(not(feature = "structured-binary"))]
fn register_structured_binary(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for (id, aliases, extensions) in [
        ("cbor", Vec::<&str>::new(), vec!["cbor"]),
        (
            "messagepack",
            vec!["msgpack", "message-pack"],
            vec!["msgpack", "mpk"],
        ),
        (
            "protobuf",
            vec!["protocol-buffers", "proto-binary"],
            vec!["pb", "protobuf"],
        ),
    ] {
        let format = FormatMetadata::new(id, ArtifactKind::StructuredBinary)
            .with_aliases(aliases)
            .with_extensions(extensions);
        let metadata = descriptor(
            &format!("grist.structured-binary.{id}"),
            format,
            ParserInfo::new("grist.structured-binary").with_feature("structured-binary"),
            crate::core::SchemaVersion::STRUCTURED_BINARY_V1,
            Some("structured-binary"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "structured-binary")?;
    }
    Ok(())
}

#[cfg(feature = "serialization")]
fn register_serialization_format(
    registry: &mut ParserRegistry,
    id: &str,
    extension: &str,
    parse: fn(&mut ParserContext<'_>) -> Result<ParserOutput, ParserError>,
) -> Result<(), ParserRegistryError> {
    let mut format =
        FormatMetadata::new(id, ArtifactKind::Serialization).with_extensions([extension]);
    if id == "jsonl" {
        format = format
            .with_aliases(["ndjson"])
            .with_extensions(["jsonl", "ndjson"]);
    }
    register(
        registry,
        descriptor(
            &("grist.serialization.".to_string() + id),
            format,
            ParserInfo::new("grist.structured-text").with_feature("serialization"),
            crate::core::SchemaVersion::STRUCTURED_TEXT_V2,
            Some("serialization"),
            serde_json::to_value(crate::serialization::SerializationOptions::default())
                .unwrap_or_default(),
        ),
        parse,
    )
}

#[cfg(feature = "serialization")]
fn parse_serialization(
    context: &mut ParserContext<'_>,
    format: crate::serialization::SerializationFormat,
) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::serialization::SerializationOptions>(context)?;
    let text = context.utf8_text()?;
    context.consume_decoded_characters(text.chars().count() as u64)?;
    output(crate::serialization::parse_serialization_with_control(
        text,
        format,
        context.source().clone(),
        &options,
        context.control(),
    ))
}

#[cfg(feature = "serialization")]
fn parse_json(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    parse_serialization(context, crate::serialization::SerializationFormat::Json)
}

#[cfg(feature = "serialization")]
fn parse_jsonl(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    parse_serialization(context, crate::serialization::SerializationFormat::Jsonl)
}

#[cfg(feature = "serialization")]
fn parse_yaml(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    parse_serialization(context, crate::serialization::SerializationFormat::Yaml)
}

#[cfg(feature = "serialization")]
fn parse_toml(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    parse_serialization(context, crate::serialization::SerializationFormat::Toml)
}

#[cfg(not(feature = "serialization"))]
fn register_serialization(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    for id in ["json", "jsonl", "yaml", "toml"] {
        let format = FormatMetadata::new(id, ArtifactKind::Serialization).with_extensions([id]);
        let metadata = descriptor(
            &("grist.serialization.".to_string() + id),
            format,
            ParserInfo::new("grist.structured-text").with_feature("serialization"),
            crate::core::SchemaVersion::STRUCTURED_TEXT_V2,
            Some("serialization"),
            serde_json::json!({}),
        );
        register_disabled(registry, metadata, "serialization")?;
    }
    Ok(())
}

#[cfg(feature = "model-output")]
fn register_model_output(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("model_output", ArtifactKind::ModelOutput);
    register(
        registry,
        descriptor(
            "grist.model_output",
            format,
            ParserInfo::new("grist.model_output").with_feature("model-output"),
            crate::core::SchemaVersion::MODEL_OUTPUT_V1,
            Some("model-output"),
            serde_json::to_value(crate::model_output::ModelOutputOptions::default())
                .unwrap_or_default(),
        ),
        parse_model_output,
    )
}

#[cfg(feature = "model-output")]
fn parse_model_output(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::model_output::ModelOutputOptions>(context)?;
    let text = context.utf8_text()?;
    context.consume_decoded_characters(text.chars().count() as u64)?;
    output(crate::model_output::parse_model_output(
        text,
        context.source().clone(),
        &options,
    ))
}

#[cfg(not(feature = "model-output"))]
fn register_model_output(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format = FormatMetadata::new("model_output", ArtifactKind::ModelOutput);
    let metadata = descriptor(
        "grist.model_output",
        format,
        ParserInfo::new("grist.model_output").with_feature("model-output"),
        crate::core::SchemaVersion::MODEL_OUTPUT_V1,
        Some("model-output"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "model-output")
}

#[cfg(feature = "ldgr-projection")]
fn register_ldgr_projection(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format =
        FormatMetadata::new("ldgr_projection", ArtifactKind::LdgrProjection).with_aliases(["ldgr"]);
    register(
        registry,
        descriptor(
            "grist.ldgr_projection",
            format,
            ParserInfo::new("grist.ldgr_projection").with_feature("ldgr-projection"),
            crate::core::SchemaVersion::LDGR_PROJECTION_V1,
            Some("ldgr-projection"),
            serde_json::to_value(crate::ldgr_projection::LdgrProjectionOptions::default())
                .unwrap_or_default(),
        ),
        parse_ldgr_projection,
    )
}

#[cfg(feature = "ldgr-projection")]
fn parse_ldgr_projection(context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
    let options = decode_options::<crate::ldgr_projection::LdgrProjectionOptions>(context)?;
    let text = context.utf8_text()?;
    context.consume_decoded_characters(text.chars().count() as u64)?;
    output(crate::ldgr_projection::parse_ldgr_projection(
        text,
        context.source().clone(),
        options,
    ))
}

#[cfg(not(feature = "ldgr-projection"))]
fn register_ldgr_projection(registry: &mut ParserRegistry) -> Result<(), ParserRegistryError> {
    let format =
        FormatMetadata::new("ldgr_projection", ArtifactKind::LdgrProjection).with_aliases(["ldgr"]);
    let metadata = descriptor(
        "grist.ldgr_projection",
        format,
        ParserInfo::new("grist.ldgr_projection").with_feature("ldgr-projection"),
        crate::core::SchemaVersion::LDGR_PROJECTION_V1,
        Some("ldgr-projection"),
        serde_json::json!({}),
    );
    register_disabled(registry, metadata, "ldgr-projection")
}

fn register_unimplemented_formats(
    registry: &mut ParserRegistry,
) -> Result<(), ParserRegistryError> {
    for &(id, extension, feature) in unimplemented_formats() {
        let mut format = FormatMetadata::new(id, ArtifactKind::Unsupported);
        if !extension.is_empty() {
            format = format.with_extensions([extension]);
        }
        let mut metadata = descriptor(
            &("grist.".to_string() + id),
            format,
            ParserInfo::new("grist.registry").with_feature(feature),
            &("grist/".to_string() + id + "/v1"),
            Some(feature),
            serde_json::json!({}),
        );
        let reason = configure_unavailable_provider(id, &mut metadata);
        registry.register_unavailable(UnavailableParser {
            descriptor: metadata,
            reason,
        })?;
    }
    Ok(())
}

fn unimplemented_formats() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        // PDF is registered by register_pdf above.
        ("doc", "doc", "word-processing"),
        ("wordprocessingml", "wml", "word-processing"),
        ("flat_opc", "fopc", "word-processing"),
        ("ppt", "ppt", "presentations"),
        ("xlsb", "xlsb", "spreadsheets"),
        ("xls", "xls", "spreadsheets"),
        ("spreadsheetml", "xmlss", "spreadsheets"),
        ("cbor", "cbor", "structured-data"),
        ("messagepack", "msgpack", "structured-data"),
        ("protobuf", "pb", "structured-data"),
        ("pst", "pst", "email-message"),
        ("ost", "ost", "email-message"),
        ("icalendar", "ics", "email-message"),
        ("vcard", "vcf", "email-message"),
        ("tnef", "dat", "email-message"),
        ("smime", "p7m", "email-message"),
        ("ipynb", "ipynb", "notebooks"),
        ("r_markdown", "rmd", "notebooks"),
        ("quarto", "qmd", "notebooks"),
        ("javascript", "js", "code"),
        ("go", "go", "code"),
        ("java", "java", "code"),
        ("kotlin", "kt", "code"),
        ("c", "c", "code"),
        ("cpp", "cpp", "code"),
        ("csharp", "cs", "code"),
        ("ruby", "rb", "code"),
        ("php", "php", "code"),
        ("swift", "swift", "code"),
        ("bash", "sh", "code"),
        ("sql", "sql", "code"),
        ("css", "css", "code"),
        ("zip", "zip", "archives"),
        ("tar", "tar", "archives"),
        ("gzip", "gz", "archives"),
        ("bzip2", "bz2", "archives"),
        ("xz", "xz", "archives"),
        ("zstandard", "zst", "archives"),
        ("7z", "7z", "archives"),
        ("png", "png", "media"),
        ("jpeg", "jpg", "media"),
        ("tiff", "tiff", "media"),
        ("webp", "webp", "media"),
        ("gif", "gif", "media"),
        ("bmp", "bmp", "media"),
        ("heif", "heif", "media"),
        ("svg", "svg", "media"),
        ("srt", "srt", "media"),
        ("webvtt", "vtt", "media"),
        ("ttml", "ttml", "media"),
        ("mp3", "mp3", "media"),
        ("mp4", "mp4", "media"),
        ("wav", "wav", "media"),
        ("flac", "flac", "media"),
        ("matroska", "mkv", "media"),
        ("quicktime", "mov", "media"),
    ]
}

fn configure_unavailable_provider(id: &str, metadata: &mut ParserDescriptor) -> UnavailableReason {
    if matches!(
        id,
        "pdf" | "png" | "jpeg" | "tiff" | "webp" | "gif" | "bmp" | "heif" | "svg"
    ) {
        metadata.allowed_providers.insert(ProviderKind::Ocr);
        metadata
            .capabilities
            .insert(Capability::ProviderDerivedContent);
    }
    if matches!(
        id,
        "mp3" | "mp4" | "wav" | "flac" | "matroska" | "quicktime"
    ) {
        metadata
            .allowed_providers
            .insert(ProviderKind::Transcription);
        metadata
            .capabilities
            .insert(Capability::ProviderDerivedContent);
    }
    if id == "smime" {
        metadata.allowed_providers.insert(ProviderKind::Decryption);
        metadata
            .capabilities
            .insert(Capability::ProviderDerivedContent);
    }
    if matches!(id, "doc" | "ppt" | "xls" | "msg" | "pst" | "ost") {
        metadata
            .allowed_providers
            .insert(ProviderKind::IsolatedParserBackend);
        metadata
            .required_providers
            .insert(ProviderKind::IsolatedParserBackend);
        metadata.capabilities.insert(Capability::IsolatedBackend);
        return UnavailableReason::BackendUnavailable {
            backend: "caller-isolated-backend".to_string(),
        };
    }
    UnavailableReason::NotImplemented
}
