use super::{CliCapabilities, DetectionReport, TextOutputManifest};
use crate::core::{
    BudgetProfile, BudgetSelection, ContentIdentity, FormatHint, Input, Limits, OperationKind,
    ParseRequest, ProviderSet, RequestId, SourceInfo,
};
use crate::detect::{DetectionOptions, detect_source};
use crate::document_graph::{DocumentGraph, DocumentGraphContext, ToDocumentGraph};
use crate::ingest::Ingestor;
use crate::registry::builtin_parser_registry;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputHints {
    pub filename: Option<String>,
    pub media_type: Option<String>,
    pub kind: Option<String>,
}

pub fn default_budget() -> BudgetSelection {
    BudgetSelection::Profile(BudgetProfile::UntrustedServiceV1)
}

pub fn read_input_bytes(
    input: &str,
    hints: &InputHints,
) -> Result<(Vec<u8>, SourceInfo), std::io::Error> {
    if input == "-" {
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes)?;
        let display_name = hints.filename.as_deref().unwrap_or("stdin");
        let mut source = SourceInfo::stdin(display_name);
        source.declared_mime_type.clone_from(&hints.media_type);
        return Ok((bytes, source));
    }
    let path = PathBuf::from(input);
    let bytes = std::fs::read(&path)?;
    let mut source = match hints.filename.as_deref() {
        Some(filename) => SourceInfo::new(filename).with_path(&path),
        None => SourceInfo::from_path(&path),
    };
    source.declared_mime_type.clone_from(&hints.media_type);
    Ok((bytes, source))
}

pub fn detect_input(
    input: &str,
    hints: &InputHints,
    request_id: RequestId,
) -> Result<DetectionReport, Box<dyn std::error::Error>> {
    let (bytes, source) = read_input_bytes(input, hints)?;
    let registry = builtin_parser_registry()?;
    let budget = default_budget().budget();
    let limits = Limits {
        max_file_bytes: budget
            .max_input_bytes
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(usize::MAX),
        max_parse_depth: budget
            .max_nesting_depth
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(usize::MAX),
        ..Limits::default()
    };
    let format_hint = format_hint(hints, None);
    let detection = detect_source(
        &source,
        &bytes,
        format_hint.as_ref(),
        &limits,
        &registry,
        &DetectionOptions::default(),
    )?;
    let identity = detection.apply_to_identity(ContentIdentity::for_raw_bytes(&bytes));
    Ok(DetectionReport {
        schema_version: DetectionReport::SCHEMA_VERSION.to_string(),
        request_id,
        source,
        identity,
        diagnostics: detection.diagnostics.clone(),
        detection,
    })
}

pub fn parse_input(
    format: &str,
    input: &str,
    hints: &InputHints,
    request_id: RequestId,
    format_options: Option<Value>,
) -> super::CliResult<crate::core::Envelope<Value>> {
    let (bytes, source) = read_input_bytes(input, hints)?;
    let request = ParseRequest::new(
        request_id,
        Input::bytes(bytes),
        source,
        default_budget(),
        ProviderSet::none(),
    );
    let request = match format_hint(hints, (format != "auto").then_some(format)) {
        Some(hint) => request.with_format_hint(hint),
        None => request,
    };
    let registry = builtin_parser_registry()?;
    if format == "auto" {
        let envelope = Ingestor::new(registry).ingest(request)?;
        return Ok(envelope.with_operation(OperationKind::Parse));
    }
    Ok(registry.dispatch(format, request, format_options)?)
}

/// In-memory equivalent of the structured CLI parse command. The executable
/// delegates to the same registry path after reading its input bytes.
pub fn parse_bytes(
    format: &str,
    bytes: Vec<u8>,
    source: SourceInfo,
    request_id: RequestId,
    format_options: Option<Value>,
) -> super::CliResult<crate::core::Envelope<Value>> {
    let request = ParseRequest::new(
        request_id,
        Input::bytes(bytes),
        source,
        default_budget(),
        ProviderSet::none(),
    );
    let registry = builtin_parser_registry()?;
    if format == "auto" {
        return Ok(Ingestor::new(registry)
            .ingest(request)?
            .with_operation(OperationKind::Parse));
    }
    Ok(registry.dispatch(
        format,
        request.with_format_hint(FormatHint::exact(format)),
        format_options,
    )?)
}

pub fn project_envelope_to_graph(
    envelope: &crate::core::Envelope<Value>,
    graph_id: impl Into<String>,
) -> super::CliResult<DocumentGraph> {
    let payload = envelope
        .payload
        .as_ref()
        .ok_or("parse operation produced no payload")?
        .clone();
    let context = DocumentGraphContext::new(graph_id).with_source(envelope.source.clone());
    match envelope.kind {
        crate::core::ArtifactKind::Text => {
            let document: crate::text::TextDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Markdown => {
            let document: crate::markdown::MarkdownDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::RestructuredText => {
            let document: crate::restructured_text::RestructuredTextDocument =
                serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::AsciiDoc => {
            let document: crate::asciidoc::AsciiDocDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Html => {
            let document: crate::html::HtmlDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Xml => {
            let document: crate::xml::XmlDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Latex => {
            let document: crate::latex::LatexDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Bibliography => {
            let document: crate::bibliography::BibliographyDocument =
                serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Pdf => {
            let document: crate::pdf::PdfDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::WordOoxml => {
            let document: crate::word_ooxml::WordOoxmlDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::PresentationOoxml => {
            let document: crate::presentation_ooxml::PresentationOoxmlDocument =
                serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::SpreadsheetOoxml => {
            let document: crate::spreadsheet_ooxml::SpreadsheetOoxmlDocument =
                serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::SpreadsheetOdf => {
            let document: crate::spreadsheet_odf::SpreadsheetOdfDocument =
                serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::PresentationOdf => {
            let document: crate::presentation_odf::OdfPresentationDocument =
                serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::OdfWord => {
            let document: crate::odf_word::OdfWordDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Rtf => {
            let document: crate::rtf::RtfDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Csv => {
            let document: crate::csv::CsvDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::StructuredBinary => {
            let document: crate::structured_binary::StructuredBinaryDocument =
                serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Columnar => {
            let document: crate::columnar::ColumnarDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Sqlite => {
            let document: crate::sqlite::SqliteDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Email => {
            let document: crate::email::EmailDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::Mbox => {
            let document: crate::mbox::MboxDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::OutlookMsg => {
            let document: crate::outlook::OutlookMsgDocument = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::PythonCode => {
            let document: crate::python::PythonFile = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::RustCode => {
            let document: crate::rust::RustFile = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        crate::core::ArtifactKind::TypeScriptCode => {
            let document: crate::typescript::TypeScriptFile = serde_json::from_value(payload)?;
            Ok(document.to_document_graph(context)?)
        }
        _ => Err(format!(
            "parser payload kind {:?} has no enabled DocumentGraph projection",
            envelope.kind
        )
        .into()),
    }
}

pub fn capabilities() -> Result<CliCapabilities, crate::capabilities::CapabilityDiscoveryError> {
    crate::capabilities::discover()
}

fn format_hint(hints: &InputHints, explicit_format: Option<&str>) -> Option<FormatHint> {
    let format = explicit_format
        .map(str::to_string)
        .or_else(|| hints.kind.clone());
    if format.is_none() && hints.media_type.is_none() && hints.filename.is_none() {
        return None;
    }
    Some(FormatHint {
        format,
        media_type: hints.media_type.clone(),
        filename: hints.filename.clone(),
    })
}

pub fn graph_input(
    input: &str,
    hints: &InputHints,
) -> super::CliResult<(DocumentGraph, ContentIdentity, SourceInfo)> {
    let (bytes, source) = read_input_bytes(input, hints)?;
    let value = serde_json::from_slice::<serde_json::Value>(&bytes)?;
    let graph_value = value
        .get("payload")
        .and_then(|payload| payload.get("graph"))
        .filter(|_| value.get("operation").and_then(serde_json::Value::as_str) == Some("transform"))
        .unwrap_or(&value);
    let graph = serde_json::from_value::<DocumentGraph>(graph_value.clone())?;
    let identity = ContentIdentity::for_raw_bytes(&bytes)
        .with_canonical_payload(graph.schema_version.as_str(), &graph)?;
    Ok((graph, identity, source))
}

pub fn output_manifest(
    request_id: RequestId,
    operation: OperationKind,
    destination: super::OutputDestination,
    result: &crate::render::RenderResult,
    graph_transform_source_map: Option<&crate::transform::GraphTransformSourceMap>,
) -> TextOutputManifest {
    TextOutputManifest {
        schema_version: TextOutputManifest::SCHEMA_VERSION.to_string(),
        request_id,
        operation,
        format: result.format,
        media_type: result.media_type.clone(),
        destination,
        byte_length: result.content.len(),
        sha256: crate::core::sha256_hex(result.content.as_bytes()),
        renderer: result.renderer.clone(),
        renderer_version: result.renderer_version.clone(),
        renderer_digest: result.renderer_digest.clone(),
        options_digest: result.options_digest.clone(),
        source_map: result.source_map.clone(),
        graph_transform_source_map: graph_transform_source_map.cloned(),
        fidelity: result.fidelity.clone(),
        diagnostics: result.diagnostics.clone(),
    }
}

pub fn path_label(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
