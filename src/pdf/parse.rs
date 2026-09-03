use super::filters::{decode_stream, filter_names};
use super::syntax::{
    RawObject, SyntaxIssue, object_stream_locator, parse_dictionary_at, parse_unsigned,
    parse_value_at, scan_indirect_objects, skip_space_and_comments,
};
use super::*;
use crate::core::{
    ArtifactKind, BoundingBox, ContentIdentity, CoordinateOrigin, CoordinateUnit, Diagnostic,
    DiagnosticDetails, Envelope, FormatIdentity, IndexBase, IndexPosition, LocationComponent,
    OperationKind, OperationStatus, ParserInfo, RecoveryAction, RecoveryKind, SchemaVersion,
    SourceInfo, SourceLocator, options_digest, sha256_hex,
};
use crate::registry::{ParserContext, ParserError, ParserOutput};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};

type ParsedXrefTable = (Vec<PdfXrefEntry>, BTreeMap<String, PdfValue>, usize);

const PARSER: &str = "grist.pdf";

pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_implementation("grist-safe-pdf-native-semantic", env!("CARGO_PKG_VERSION"))
        .with_specification_version("ISO 32000-2:2020 structural, text, and graphics subset")
        .with_feature("pdf")
}

pub fn parse_pdf_bytes(bytes: &[u8], source: SourceInfo, options: &PdfOptions) -> PdfEnvelope {
    let digest =
        options_digest(&serializable_options(options)).expect("PDF options are serializable");
    match parse_core(bytes, source.clone(), options) {
        Ok(outcome) if outcome.document.encryption.encrypted => {
            let encryption = &outcome.document.encryption;
            let details = DiagnosticDetails::from_value(json!({
                "handler": encryption.handler,
                "revision": encryption.revision,
                "algorithm_version": encryption.algorithm_version,
                "key_length_bits": encryption.key_length_bits,
                "credential_supplied": encryption.credential_supplied,
            }))
            .expect("safe encryption details");
            let diagnostic = Diagnostic::error(
                PARSER,
                "pdf.encrypted",
                "PDF requires successful in-memory decryption before content can be returned",
            )
            .with_details(details)
            .with_recovery(RecoveryAction::new(
                RecoveryKind::SupplyCredentials,
                "supply a valid password through PdfOptions::set_password",
                true,
            ));
            Envelope::without_payload(
                OperationKind::Parse,
                ArtifactKind::Pdf,
                OperationStatus::Encrypted,
                source,
                parser_info(),
                digest,
                SchemaVersion::PDF_V1,
            )
            .expect("encrypted status permits no payload")
            .with_identity(pdf_identity(bytes))
            .with_diagnostics(vec![diagnostic])
        }
        Ok(outcome) => finish_envelope(bytes, source, digest, outcome),
        Err(diagnostic) => Envelope::without_payload(
            OperationKind::Parse,
            ArtifactKind::Pdf,
            OperationStatus::Failed,
            source,
            parser_info(),
            digest,
            SchemaVersion::PDF_V1,
        )
        .expect("failed status permits no payload")
        .with_identity(pdf_identity(bytes))
        .with_diagnostics(vec![*diagnostic]),
    }
}

pub(crate) fn parse_registered(
    context: &mut ParserContext<'_>,
) -> Result<ParserOutput, ParserError> {
    let options: PdfOptions =
        serde_json::from_value(context.options().clone()).map_err(|error| {
            Box::new(Diagnostic::malformed(PARSER, error.to_string())) as ParserError
        })?;
    let mut outcome = parse_core(context.bytes(), context.source().clone(), &options)?;
    if outcome.document.encryption.encrypted {
        let details = DiagnosticDetails::from_value(json!({
            "handler": outcome.document.encryption.handler,
            "revision": outcome.document.encryption.revision,
            "algorithm_version": outcome.document.encryption.algorithm_version,
            "key_length_bits": outcome.document.encryption.key_length_bits,
            "credential_supplied": false,
        }))
        .expect("safe encryption details");
        return Ok(ParserOutput::terminal(
            OperationStatus::Encrypted,
            vec![
                Diagnostic::error(
                    PARSER,
                    "pdf.encrypted",
                    "PDF requires an in-memory password",
                )
                .with_details(details)
                .with_recovery(RecoveryAction::new(
                    RecoveryKind::SupplyCredentials,
                    "use the typed PDF API to supply a password without serialization",
                    true,
                )),
            ],
        ));
    }
    let ocr = super::ocr::apply_selected_ocr(context, &mut outcome.document, &options);
    outcome.diagnostics.extend(ocr.diagnostics);
    context.consume_child_artifacts(embedded_file_count(
        &outcome.document.interactive.embedded_files,
    ))?;
    context.consume_nodes(
        outcome
            .document
            .objects
            .len()
            .saturating_add(outcome.document.page_tree.len())
            .saturating_add(outcome.document.pages.len())
            .saturating_add(outcome.document.native_layout.fonts.len())
            .saturating_add(
                outcome
                    .document
                    .native_layout
                    .pages
                    .iter()
                    .map(|page| {
                        page.glyphs
                            .len()
                            .saturating_add(page.tokens.len())
                            .saturating_add(page.lines.len())
                            .saturating_add(page.blocks.len())
                    })
                    .sum::<usize>(),
            )
            .saturating_add(
                outcome
                    .document
                    .semantic_structure
                    .pages
                    .iter()
                    .map(|page| {
                        page.columns
                            .len()
                            .saturating_add(page.blocks.len())
                            .saturating_add(page.lists.len())
                            .saturating_add(page.tables.len())
                            .saturating_add(
                                page.tables
                                    .iter()
                                    .map(|table| {
                                        table.rows.len()
                                            + table
                                                .rows
                                                .iter()
                                                .map(|row| row.cells.len())
                                                .sum::<usize>()
                                    })
                                    .sum::<usize>(),
                            )
                            .saturating_add(page.graphics.len())
                            .saturating_add(page.figures.len())
                    })
                    .sum::<usize>(),
            )
            .saturating_add(outcome.document.metadata.fields.len())
            .saturating_add(text_node_count(&outcome.document.text))
            .saturating_add(interactive_node_count(&outcome.document.interactive)) as u64,
    )?;
    context.consume_records(outcome.document.pages.len() as u64)?;
    context.observe_memory_bytes(
        outcome
            .document
            .objects
            .iter()
            .filter_map(|object| object.stream.as_ref())
            .filter_map(|stream| stream.decoded_length)
            .sum(),
    )?;
    let value = serde_json::to_value(&outcome.document).map_err(|error| {
        Box::new(Diagnostic::parser_defect(PARSER, error.to_string())) as ParserError
    })?;
    let mut output = if outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.partial)
    {
        ParserOutput::partial(Some(value), outcome.diagnostics)
    } else {
        let mut output = ParserOutput::complete(value);
        output.diagnostics = outcome.diagnostics;
        output
    };
    output
        .provenance
        .extend(repair_provenance(&outcome.document));
    output.provenance.extend(ocr.provenance);
    output.providers.extend(ocr.invocations);
    Ok(output)
}

fn serializable_options(options: &PdfOptions) -> serde_json::Value {
    serde_json::to_value(options).expect("PdfOptions serialize without secrets")
}

fn pdf_identity(bytes: &[u8]) -> ContentIdentity {
    ContentIdentity::for_raw_bytes(bytes)
        .with_format(FormatIdentity::new("pdf", Some("application/pdf")))
}

fn finish_envelope(
    bytes: &[u8],
    source: SourceInfo,
    digest: String,
    outcome: CoreOutcome,
) -> PdfEnvelope {
    let mut envelope = if outcome
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.partial)
    {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Pdf,
            source,
            parser_info(),
            digest,
            SchemaVersion::PDF_V1,
            Some(outcome.document),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Pdf,
            source,
            parser_info(),
            digest,
            SchemaVersion::PDF_V1,
            outcome.document,
        )
    };
    envelope.diagnostics = outcome.diagnostics;
    if let Some(document) = envelope.payload.as_ref() {
        envelope.provenance.extend(repair_provenance(document));
    }
    envelope
        .with_identity(pdf_identity(bytes))
        .with_canonical_payload_identity()
        .expect("canonical PDF payload")
}

fn repair_provenance(document: &PdfDocument) -> Vec<crate::core::ProvenanceStep> {
    let semantic_digest = sha256_hex(
        &serde_json::to_vec(&document.semantic_structure).expect("semantic structure serializes"),
    );
    let interactive_digest = sha256_hex(
        &serde_json::to_vec(&document.interactive).expect("interactive content serializes"),
    );
    let mut output = vec![
        crate::core::ProvenanceStep::new(
            OperationKind::Parse,
            "grist.pdf.semantic-structure-inference.v1",
            "pdf:native-layout-and-graphics",
            "pdf:confidence-bearing-semantic-structure",
            semantic_digest,
            crate::core::DeclaredLoss::Lossless,
        )
        .expect("semantic provenance is valid"),
        crate::core::ProvenanceStep::new(
            OperationKind::Parse,
            "grist.pdf.interactive-embedded-inventory.v1",
            "pdf:object-graph-and-embedded-streams",
            "pdf:inert-interactive-and-nested-artifact-data",
            interactive_digest,
            crate::core::DeclaredLoss::Lossless,
        )
        .expect("interactive provenance is valid"),
    ];
    if !document.repairs.is_empty() {
        let digest = sha256_hex(&serde_json::to_vec(&document.repairs).expect("repairs serialize"));
        output.push(
            crate::core::ProvenanceStep::new(
                OperationKind::Parse,
                "grist.pdf.xref-object-recovery.v1",
                "pdf:source-structure",
                "pdf:repaired-structure",
                digest,
                crate::core::DeclaredLoss::Lossy(crate::core::LossClass::REPAIR_APPLIED.into()),
            )
            .expect("repair provenance is valid")
            .with_warning("pdf.repaired"),
        );
    }
    output
}

fn embedded_file_count(files: &[PdfEmbeddedFile]) -> u64 {
    files.iter().fold(0u64, |count, file| {
        count
            .saturating_add(1)
            .saturating_add(embedded_file_count(&file.children))
    })
}

fn interactive_node_count(interactive: &PdfInteractiveContent) -> usize {
    interactive
        .destinations
        .len()
        .saturating_add(interactive.outlines.len())
        .saturating_add(interactive.links.len())
        .saturating_add(interactive.annotations.len())
        .saturating_add(interactive.comments.len())
        .saturating_add(
            interactive
                .form
                .as_ref()
                .map_or(0, |form| 1usize.saturating_add(form.fields.len())),
        )
        .saturating_add(interactive.signatures.len())
        .saturating_add(interactive.layers.len())
        .saturating_add(embedded_file_count(&interactive.embedded_files) as usize)
}

fn text_node_count(text: &PdfTextContent) -> usize {
    text.pages.iter().fold(0usize, |count, page| {
        count
            .saturating_add(1)
            .saturating_add(page.native.value.regions.len())
            .saturating_add(page.ocr_attempts.len())
            .saturating_add(
                page.ocr_attempts
                    .iter()
                    .map(|attempt| attempt.regions.len())
                    .sum::<usize>(),
            )
            .saturating_add(
                page.reconciled
                    .as_ref()
                    .map_or(0, |value| value.value.items.len().saturating_add(1)),
            )
    })
}

struct CoreOutcome {
    document: PdfDocument,
    diagnostics: Vec<Diagnostic>,
}

fn parse_core(
    bytes: &[u8],
    source: SourceInfo,
    options: &PdfOptions,
) -> Result<CoreOutcome, ParserError> {
    validate_options(options)?;
    let (header, mut diagnostics, mut repairs) = parse_header(bytes)?;
    let (mut objects, syntax_issues, object_limit_hit) = scan_indirect_objects(bytes, options);
    diagnostics.extend(syntax_issues.into_iter().map(issue_diagnostic));
    if object_limit_hit {
        diagnostics.push(Diagnostic::budget_exhausted(
            PARSER,
            format!("PDF exceeds max_objects={}", options.max_objects),
        ));
    }
    if objects.is_empty() {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "PDF contains no parseable indirect objects",
        )));
    }

    decode_streams(&mut objects, options, false, &mut diagnostics);
    expand_object_streams(&mut objects, options, &mut diagnostics, &mut repairs);
    objects.sort_by_key(|object| {
        (
            object.model.locator_start(),
            object.model.object.object_number,
            object.model.object.generation,
        )
    });
    assign_revisions(&mut objects);

    let startxref = parse_startxref(bytes);
    let (mut xref, trailer_dictionary, trailer_locator) = parse_xref(
        bytes,
        startxref,
        &objects,
        options,
        &mut diagnostics,
        &mut repairs,
    );
    validate_xref(&mut xref, &objects, &mut diagnostics, &mut repairs);
    let latest = latest_objects(&objects);
    let (root_reference, root_repaired) = root_reference(&trailer_dictionary, &latest);
    let root = root_reference.ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            "PDF trailer and object scan contain no catalog",
        )) as ParserError
    })?;
    if root_repaired {
        repairs.push(PdfRepair {
            code: "pdf.catalog.recovered".into(),
            description: "catalog reference recovered from object scan".into(),
            object: Some(root),
            original_offset: None,
            recovered_offset: latest
                .get(&root)
                .map(|object| object.model.locator_start() as u64),
        });
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.catalog.recovered",
                "catalog reference recovered from indirect objects",
            )
            .partial(),
        );
    }
    let catalog = build_catalog(root, &latest)?;
    let trailer = build_trailer(&trailer_dictionary, trailer_locator, root);
    let encryption = build_encryption(&trailer, &latest, options.password().is_some());

    let (mut page_tree, mut pages) = build_pages(&catalog, &latest, options, &mut diagnostics)?;
    let labels = build_page_labels(&catalog, pages.len(), &latest, options, &mut diagnostics);
    let labels_by_page = labels
        .iter()
        .map(|label| (label.page_index, label.label.clone()))
        .collect::<HashMap<_, _>>();
    for page in &mut pages {
        page.label = labels_by_page.get(&page.index).cloned();
    }
    reconcile_page_counts(&mut page_tree, &pages, &mut diagnostics, &mut repairs);
    let metadata = build_metadata(&catalog, &trailer, &latest, &objects, &mut diagnostics);
    let native_layout =
        super::layout::extract_native_layout(&pages, &latest, options, &mut diagnostics);
    let text = super::ocr::native_text_content(&native_layout)?;
    let semantic_structure = super::semantic::extract_semantic_structure(
        &pages,
        &native_layout,
        &latest,
        catalog.structure_tree_root,
        options,
        &mut diagnostics,
    );
    let filters = build_filter_inventory(&objects);
    let interactive = super::interactive::extract_interactive_content(
        bytes,
        &catalog,
        &pages,
        &latest,
        options,
        &mut diagnostics,
    );
    let active_content = inventory_active_content(&objects);
    if !active_content.is_empty() {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.active_content.inert",
                format!(
                    "inventoried {} active-content reference(s) without execution",
                    active_content.len()
                ),
            )
            .partial(),
        );
    }
    if encryption.encrypted && options.password().is_some() {
        diagnostics.push(Diagnostic::warning(PARSER, "pdf.encryption.credentials_unverified", "a password was supplied only in memory, but this structural backend does not authenticate or decrypt encrypted strings/streams"));
    }
    for repair in &repairs {
        if !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == repair.code)
        {
            diagnostics.push(
                Diagnostic::warning(PARSER, repair.code.clone(), repair.description.clone())
                    .partial(),
            );
        }
    }
    let document = PdfDocument {
        schema_version: SchemaVersion::PDF_V1.into(),
        source,
        header,
        catalog,
        trailer,
        xref,
        objects: objects.into_iter().map(|object| object.model).collect(),
        page_tree,
        pages,
        native_layout,
        text,
        semantic_structure,
        page_labels: labels,
        metadata,
        filters,
        encryption,
        repairs,
        active_content,
        interactive,
    };
    Ok(CoreOutcome {
        document,
        diagnostics,
    })
}

fn validate_options(options: &PdfOptions) -> Result<(), ParserError> {
    for (name, value) in [
        (
            "ocr.duplicate_text_similarity",
            options.ocr.duplicate_text_similarity,
        ),
        ("ocr.duplicate_overlap", options.ocr.duplicate_overlap),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(Box::new(Diagnostic::malformed(
                PARSER,
                format!("PDF option {name} must be finite and in [0, 1]"),
            )));
        }
    }
    Ok(())
}

trait ObjectOffset {
    fn locator_start(&self) -> usize;
}
impl ObjectOffset for PdfIndirectObject {
    fn locator_start(&self) -> usize {
        match self.locator.location {
            PdfObjectLocation::Direct { byte_start, .. } => {
                usize::try_from(byte_start).unwrap_or(usize::MAX)
            }
            PdfObjectLocation::ObjectStream {
                decoded_byte_start, ..
            } => usize::try_from(decoded_byte_start).unwrap_or(usize::MAX),
        }
    }
}

fn parse_header(bytes: &[u8]) -> Result<(PdfHeader, Vec<Diagnostic>, Vec<PdfRepair>), ParserError> {
    let offset = bytes[..bytes.len().min(1024)]
        .windows(5)
        .position(|window| window == b"%PDF-")
        .ok_or_else(|| {
            Box::new(Diagnostic::malformed(
                PARSER,
                "missing PDF header within the first 1024 bytes",
            )) as ParserError
        })?;
    let version_start = offset + 5;
    let version_end = bytes[version_start..]
        .iter()
        .position(|byte| matches!(*byte, b'\r' | b'\n' | b' '))
        .map(|relative| version_start + relative)
        .unwrap_or(bytes.len())
        .min(version_start + 8);
    let version = String::from_utf8_lossy(&bytes[version_start..version_end]).into_owned();
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "invalid PDF version header",
        )));
    }
    let mut diagnostics = Vec::new();
    let mut repairs = Vec::new();
    if offset != 0 {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.header.relocated",
                format!("PDF header begins at byte {offset}"),
            )
            .partial(),
        );
        repairs.push(PdfRepair {
            code: "pdf.header.relocated".into(),
            description: "accepted a header after leading bytes".into(),
            object: None,
            original_offset: Some(0),
            recovered_offset: Some(offset as u64),
        });
    }
    let line_end = bytes[version_end..]
        .iter()
        .position(|byte| matches!(*byte, b'\r' | b'\n'))
        .map(|relative| version_end + relative)
        .unwrap_or(version_end);
    let marker_search_end = bytes.len().min(line_end.saturating_add(128));
    let binary_marker = bytes[line_end..marker_search_end]
        .split(|byte| matches!(*byte, b'\r' | b'\n'))
        .any(|line| {
            line.starts_with(b"%") && line.iter().skip(1).filter(|byte| **byte >= 128).count() >= 4
        });
    let eof_marker_offset = rfind(bytes, b"%%EOF").map(|offset| offset as u64);
    let trailing_bytes = eof_marker_offset
        .map(|offset| bytes.len() as u64 - offset - 5)
        .unwrap_or(0);
    if eof_marker_offset.is_none() {
        diagnostics.push(
            Diagnostic::warning(PARSER, "pdf.eof.missing", "PDF has no %%EOF marker").partial(),
        );
        repairs.push(PdfRepair {
            code: "pdf.eof.missing".into(),
            description: "parsed a file without its EOF marker".into(),
            object: None,
            original_offset: None,
            recovered_offset: None,
        });
    } else if trailing_bytes > 0
        && eof_marker_offset.is_some_and(|offset| {
            bytes[(offset as usize + 5)..]
                .iter()
                .any(|byte| !byte.is_ascii_whitespace())
        })
    {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.eof.trailing_bytes",
                format!("PDF contains {trailing_bytes} byte(s) after %%EOF"),
            )
            .partial(),
        );
    }
    Ok((
        PdfHeader {
            version,
            byte_offset: offset as u64,
            binary_marker,
            eof_marker_offset,
            trailing_bytes,
        },
        diagnostics,
        repairs,
    ))
}

fn parse_startxref(bytes: &[u8]) -> Option<u64> {
    let offset = rfind(bytes, b"startxref")? + 9;
    let mut cursor = offset;
    skip_space_and_comments(bytes, &mut cursor);
    parse_unsigned(bytes, &mut cursor)
}

fn rfind(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes
        .windows(needle.len())
        .rposition(|window| window == needle)
}

fn issue_diagnostic(issue: SyntaxIssue) -> Diagnostic {
    let details = DiagnosticDetails::from_value(json!({ "byte_offset": issue.offset, "object": issue.object.map(|object| object.to_string()) })).expect("safe syntax details");
    Diagnostic::warning(PARSER, issue.code, issue.message)
        .with_details(details)
        .partial()
}

fn decode_streams(
    objects: &mut [RawObject],
    options: &PdfOptions,
    encrypted: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for object in objects {
        let (Some(encoded), Some(dictionary)) = (
            object.stream_bytes.as_deref(),
            object.model.value.as_dictionary(),
        ) else {
            continue;
        };
        let decoded = decode_stream(
            encoded,
            dictionary,
            options.max_decoded_stream_bytes,
            encrypted,
        );
        debug_assert_eq!(decoded.filters, filter_names(dictionary));
        let supported = decoded.supported;
        if let Some(stream) = object.model.stream.as_mut() {
            stream.decode_status = decoded.status.clone();
            stream.decoded_length = decoded.bytes.as_ref().map(|bytes| bytes.len() as u64);
            stream.decoded_sha256 = decoded.bytes.as_ref().map(|bytes| sha256_hex(bytes));
        }
        object.decoded_stream = decoded.bytes;
        if let Some(message) = decoded.error {
            let diagnostic = if decoded.status == PdfStreamDecodeStatus::BudgetExceeded {
                Diagnostic::budget_exhausted(PARSER, message)
            } else if !supported {
                Diagnostic::unsupported(PARSER, message).partial()
            } else {
                Diagnostic::warning(PARSER, "pdf.stream.decode_failed", message).partial()
            };
            diagnostics.push(diagnostic);
        }
    }
}

fn expand_object_streams(
    objects: &mut Vec<RawObject>,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
    _repairs: &mut Vec<PdfRepair>,
) {
    let mut expanded = Vec::new();
    for container in objects.iter() {
        let Some(dictionary) = container.model.value.as_dictionary() else {
            continue;
        };
        if dictionary.get("Type").and_then(PdfValue::as_name) != Some("ObjStm") {
            continue;
        }
        let Some(decoded) = container.decoded_stream.as_deref() else {
            diagnostics.push(
                Diagnostic::unsupported(
                    PARSER,
                    format!(
                        "object stream {} could not be decoded",
                        container.model.object
                    ),
                )
                .partial(),
            );
            continue;
        };
        let Some(count) = dictionary
            .get("N")
            .and_then(PdfValue::as_integer)
            .and_then(|value| usize::try_from(value).ok())
        else {
            continue;
        };
        let Some(first) = dictionary
            .get("First")
            .and_then(PdfValue::as_integer)
            .and_then(|value| usize::try_from(value).ok())
        else {
            continue;
        };
        if count > options.max_objects as usize || first > decoded.len() {
            diagnostics.push(Diagnostic::budget_exhausted(
                PARSER,
                "object stream declaration exceeds configured bounds",
            ));
            continue;
        }
        let mut cursor = 0usize;
        let mut headers = Vec::new();
        for _ in 0..count {
            skip_space_and_comments(decoded, &mut cursor);
            let Some(object_number) =
                parse_unsigned(decoded, &mut cursor).and_then(|value| u32::try_from(value).ok())
            else {
                break;
            };
            skip_space_and_comments(decoded, &mut cursor);
            let Some(relative) =
                parse_unsigned(decoded, &mut cursor).and_then(|value| usize::try_from(value).ok())
            else {
                break;
            };
            headers.push((object_number, relative));
        }
        if headers.len() != count {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.object_stream.header_invalid",
                    format!(
                        "object stream {} declares {count} objects but has {} headers",
                        container.model.object,
                        headers.len()
                    ),
                )
                .partial(),
            );
        }
        for (index, (object_number, relative)) in headers.iter().copied().enumerate() {
            if objects.len().saturating_add(expanded.len()) >= options.max_objects as usize {
                break;
            }
            let start = first.saturating_add(relative);
            let end = headers
                .get(index + 1)
                .map(|(_, next)| first.saturating_add(*next))
                .unwrap_or(decoded.len())
                .min(decoded.len());
            if start >= end {
                continue;
            }
            match parse_value_at(decoded, start, options.max_object_depth) {
                Ok((value, parsed_end)) => {
                    let reference = PdfReference {
                        object_number,
                        generation: 0,
                    };
                    expanded.push(RawObject {
                        model: PdfIndirectObject {
                            object: reference,
                            revision: 0,
                            locator: object_stream_locator(
                                reference,
                                container.model.object,
                                start,
                                parsed_end.min(end),
                            ),
                            value,
                            stream: None,
                            raw_sha256: sha256_hex(&decoded[start..parsed_end.min(end)]),
                            repaired: false,
                        },
                        stream_bytes: None,
                        decoded_stream: None,
                    });
                }
                Err(issue) => diagnostics.push(issue_diagnostic(SyntaxIssue {
                    object: Some(container.model.object),
                    ..issue
                })),
            }
        }
        // Object streams are a standard representation, not a repair.
    }
    objects.extend(expanded);
}

fn assign_revisions(objects: &mut [RawObject]) {
    let mut revisions = HashMap::<PdfReference, u32>::new();
    for object in objects {
        let revision = revisions.entry(object.model.object).or_default();
        object.model.revision = *revision;
        *revision = revision.saturating_add(1);
    }
}

fn parse_xref(
    bytes: &[u8],
    startxref: Option<u64>,
    objects: &[RawObject],
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
    repairs: &mut Vec<PdfRepair>,
) -> (
    PdfCrossReference,
    BTreeMap<String, PdfValue>,
    PdfObjectLocator,
) {
    let mut sections = Vec::new();
    let mut entries = Vec::new();
    let mut trailer = None;
    let mut trailer_locator = None;
    let mut pending = startxref.and_then(|offset| usize::try_from(offset).ok());
    let mut visited = HashSet::new();
    while let Some(offset) = pending {
        if !visited.insert(offset) || sections.len() >= 1024 {
            break;
        }
        let mut cursor = offset;
        skip_space_and_comments(bytes, &mut cursor);
        if bytes.get(cursor..cursor.saturating_add(4)) == Some(b"xref") {
            match parse_xref_table(bytes, cursor, options.max_object_depth) {
                Ok((section_entries, dictionary, dictionary_offset)) => {
                    let previous = dictionary
                        .get("Prev")
                        .and_then(PdfValue::as_integer)
                        .and_then(|value| u64::try_from(value).ok());
                    sections.push(PdfXrefSection {
                        byte_offset: offset as u64,
                        kind: PdfXrefKind::Table,
                        entry_count: section_entries.len() as u64,
                        previous,
                    });
                    entries.extend(section_entries);
                    if trailer.is_none() {
                        trailer_locator =
                            Some(trailer_object_locator(dictionary_offset, bytes.len()));
                        trailer = Some(dictionary);
                    }
                    pending = previous.and_then(|value| usize::try_from(value).ok());
                    continue;
                }
                Err(issue) => diagnostics.push(issue_diagnostic(issue)),
            }
        } else if let Some(object) = objects
            .iter()
            .find(|object| object.model.locator_start() == cursor)
            && object
                .model
                .value
                .as_dictionary()
                .and_then(|dictionary| dictionary.get("Type"))
                .and_then(PdfValue::as_name)
                == Some("XRef")
        {
            if let Some((section_entries, previous)) = parse_xref_stream(object, diagnostics) {
                sections.push(PdfXrefSection {
                    byte_offset: offset as u64,
                    kind: PdfXrefKind::Stream,
                    entry_count: section_entries.len() as u64,
                    previous,
                });
                entries.extend(section_entries);
                if trailer.is_none() {
                    trailer = object.model.value.as_dictionary().cloned();
                    trailer_locator = Some(object.model.locator.clone());
                }
                pending = previous.and_then(|value| usize::try_from(value).ok());
                continue;
            }
        }
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.xref.invalid_start",
                format!("startxref points to invalid byte offset {offset}"),
            )
            .partial(),
        );
        repairs.push(PdfRepair {
            code: "pdf.xref.invalid_start".into(),
            description: "recovered cross-reference information by scanning indirect objects"
                .into(),
            object: None,
            original_offset: Some(offset as u64),
            recovered_offset: None,
        });
        break;
    }
    if trailer.is_none()
        && let Some(offset) = rfind(bytes, b"trailer")
    {
        let mut cursor = offset + 7;
        skip_space_and_comments(bytes, &mut cursor);
        if let Ok((dictionary, end)) = parse_dictionary_at(bytes, cursor, options.max_object_depth)
        {
            trailer = Some(dictionary);
            trailer_locator = Some(trailer_object_locator(cursor, end));
            repairs.push(PdfRepair {
                code: "pdf.trailer.recovered".into(),
                description: "recovered trailer dictionary independently of xref".into(),
                object: None,
                original_offset: startxref,
                recovered_offset: Some(cursor as u64),
            });
        }
    }
    if trailer.is_none()
        && let Some(object) = objects.iter().rev().find(|object| {
            object
                .model
                .value
                .as_dictionary()
                .and_then(|dictionary| dictionary.get("Type"))
                .and_then(PdfValue::as_name)
                == Some("XRef")
        })
    {
        trailer = object.model.value.as_dictionary().cloned();
        trailer_locator = Some(object.model.locator.clone());
    }
    let repaired = sections.is_empty() || !repairs.is_empty();
    if sections.is_empty() {
        entries = recovered_xref_entries(objects);
        sections.push(PdfXrefSection {
            byte_offset: 0,
            kind: PdfXrefKind::RecoveredObjectScan,
            entry_count: entries.len() as u64,
            previous: None,
        });
        repairs.push(PdfRepair {
            code: "pdf.xref.reconstructed".into(),
            description: "reconstructed xref entries from bounded indirect-object scan".into(),
            object: None,
            original_offset: startxref,
            recovered_offset: None,
        });
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.xref.reconstructed",
                "cross-reference structure was reconstructed from indirect objects",
            )
            .partial(),
        );
    }
    entries.sort_by_key(|entry| {
        (
            entry.object_number,
            entry.generation,
            entry.byte_offset,
            entry.object_stream_index,
        )
    });
    let dictionary = trailer.unwrap_or_default();
    let locator = trailer_locator.unwrap_or_else(|| trailer_object_locator(0, 0));
    (
        PdfCrossReference {
            startxref,
            sections,
            entries,
            repaired,
        },
        dictionary,
        locator,
    )
}

fn parse_xref_table(
    bytes: &[u8],
    offset: usize,
    max_depth: u16,
) -> Result<ParsedXrefTable, SyntaxIssue> {
    let mut cursor = offset + 4;
    let mut entries = Vec::new();
    loop {
        skip_space_and_comments(bytes, &mut cursor);
        if bytes.get(cursor..cursor.saturating_add(7)) == Some(b"trailer") {
            cursor += 7;
            skip_space_and_comments(bytes, &mut cursor);
            let dictionary_offset = cursor;
            let (dictionary, _) = parse_dictionary_at(bytes, cursor, max_depth)?;
            return Ok((entries, dictionary, dictionary_offset));
        }
        let subsection_offset = cursor;
        let Some(first) =
            parse_unsigned(bytes, &mut cursor).and_then(|value| u32::try_from(value).ok())
        else {
            return Err(SyntaxIssue {
                code: "pdf.xref.subsection_invalid",
                message: "invalid xref subsection header".into(),
                offset: subsection_offset,
                object: None,
            });
        };
        skip_space_and_comments(bytes, &mut cursor);
        let Some(count) =
            parse_unsigned(bytes, &mut cursor).and_then(|value| u32::try_from(value).ok())
        else {
            return Err(SyntaxIssue {
                code: "pdf.xref.subsection_invalid",
                message: "invalid xref subsection count".into(),
                offset: subsection_offset,
                object: None,
            });
        };
        for index in 0..count {
            skip_space_and_comments(bytes, &mut cursor);
            let entry_offset = cursor;
            let Some(byte_offset) = parse_unsigned(bytes, &mut cursor) else {
                return Err(SyntaxIssue {
                    code: "pdf.xref.entry_invalid",
                    message: "truncated xref entry offset".into(),
                    offset: entry_offset,
                    object: None,
                });
            };
            skip_space_and_comments(bytes, &mut cursor);
            let Some(generation) =
                parse_unsigned(bytes, &mut cursor).and_then(|value| u16::try_from(value).ok())
            else {
                return Err(SyntaxIssue {
                    code: "pdf.xref.entry_invalid",
                    message: "truncated xref generation".into(),
                    offset: entry_offset,
                    object: None,
                });
            };
            skip_space_and_comments(bytes, &mut cursor);
            let flag = bytes.get(cursor).copied().unwrap_or_default();
            if matches!(flag, b'n' | b'f') {
                cursor += 1;
            }
            entries.push(PdfXrefEntry {
                object_number: first.saturating_add(index),
                generation,
                kind: if flag == b'f' {
                    PdfXrefEntryKind::Free
                } else {
                    PdfXrefEntryKind::InUse
                },
                byte_offset: (flag != b'f').then_some(byte_offset),
                object_stream: None,
                object_stream_index: None,
                valid: matches!(flag, b'n' | b'f'),
            });
        }
    }
}

fn parse_xref_stream(
    object: &RawObject,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<(Vec<PdfXrefEntry>, Option<u64>)> {
    let dictionary = object.model.value.as_dictionary()?;
    let bytes = object.decoded_stream.as_deref()?;
    let widths = integer_array(dictionary.get("W")?)
        .into_iter()
        .filter_map(|value| usize::try_from(value).ok())
        .collect::<Vec<_>>();
    if widths.len() != 3 || widths.iter().sum::<usize>() == 0 {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.xref_stream.widths_invalid",
                format!("xref stream {} has invalid W array", object.model.object),
            )
            .partial(),
        );
        return None;
    }
    let indexes = dictionary
        .get("Index")
        .map(integer_array)
        .unwrap_or_else(|| {
            vec![
                0,
                dictionary
                    .get("Size")
                    .and_then(PdfValue::as_integer)
                    .unwrap_or(0),
            ]
        });
    if indexes.len() % 2 != 0 {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.xref_stream.index_invalid",
                format!(
                    "xref stream {} has invalid Index array",
                    object.model.object
                ),
            )
            .partial(),
        );
        return None;
    }
    let row_width = widths.iter().sum::<usize>();
    let mut cursor = 0usize;
    let mut entries = Vec::new();
    for pair in indexes.chunks_exact(2) {
        let (Ok(first), Ok(count)) = (u32::try_from(pair[0]), u32::try_from(pair[1])) else {
            return None;
        };
        for index in 0..count {
            let row = bytes.get(cursor..cursor.saturating_add(row_width))?;
            cursor += row_width;
            let field0 = if widths[0] == 0 {
                1
            } else {
                read_be(&row[..widths[0]])
            };
            let field1_start = widths[0];
            let field1_end = field1_start + widths[1];
            let field1 = read_be(&row[field1_start..field1_end]);
            let field2 = read_be(&row[field1_end..]);
            let (kind, byte_offset, object_stream, object_stream_index, generation) = match field0 {
                0 => (
                    PdfXrefEntryKind::Free,
                    None,
                    None,
                    None,
                    u16::try_from(field2).unwrap_or(u16::MAX),
                ),
                1 => (
                    PdfXrefEntryKind::InUse,
                    Some(field1),
                    None,
                    None,
                    u16::try_from(field2).unwrap_or(u16::MAX),
                ),
                2 => (
                    PdfXrefEntryKind::Compressed,
                    None,
                    Some(PdfReference {
                        object_number: u32::try_from(field1).ok()?,
                        generation: 0,
                    }),
                    u32::try_from(field2).ok(),
                    0,
                ),
                _ => (PdfXrefEntryKind::InUse, None, None, None, 0),
            };
            entries.push(PdfXrefEntry {
                object_number: first.saturating_add(index),
                generation,
                kind,
                byte_offset,
                object_stream,
                object_stream_index,
                valid: field0 <= 2,
            });
        }
    }
    let previous = dictionary
        .get("Prev")
        .and_then(PdfValue::as_integer)
        .and_then(|value| u64::try_from(value).ok());
    Some((entries, previous))
}

fn read_be(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
}

fn integer_array(value: &PdfValue) -> Vec<i64> {
    match value {
        PdfValue::Array(values) => values.iter().filter_map(PdfValue::as_integer).collect(),
        _ => Vec::new(),
    }
}

fn recovered_xref_entries(objects: &[RawObject]) -> Vec<PdfXrefEntry> {
    objects
        .iter()
        .map(|object| match object.model.locator.location {
            PdfObjectLocation::Direct { byte_start, .. } => PdfXrefEntry {
                object_number: object.model.object.object_number,
                generation: object.model.object.generation,
                kind: PdfXrefEntryKind::InUse,
                byte_offset: Some(byte_start),
                object_stream: None,
                object_stream_index: None,
                valid: true,
            },
            PdfObjectLocation::ObjectStream { container, .. } => PdfXrefEntry {
                object_number: object.model.object.object_number,
                generation: object.model.object.generation,
                kind: PdfXrefEntryKind::Compressed,
                byte_offset: None,
                object_stream: Some(container),
                object_stream_index: None,
                valid: true,
            },
        })
        .collect()
}

fn validate_xref(
    xref: &mut PdfCrossReference,
    objects: &[RawObject],
    diagnostics: &mut Vec<Diagnostic>,
    repairs: &mut Vec<PdfRepair>,
) {
    let offsets = objects
        .iter()
        .filter_map(|object| match object.model.locator.location {
            PdfObjectLocation::Direct { byte_start, .. } => Some((object.model.object, byte_start)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    for entry in &mut xref.entries {
        if entry.kind != PdfXrefEntryKind::InUse {
            continue;
        }
        let reference = PdfReference {
            object_number: entry.object_number,
            generation: entry.generation,
        };
        if let Some(actual) = offsets.get(&reference).copied()
            && entry.byte_offset != Some(actual)
        {
            entry.valid = false;
            xref.repaired = true;
            repairs.push(PdfRepair {
                code: "pdf.xref.offset_repaired".into(),
                description: format!("xref offset for {reference} was repaired"),
                object: Some(reference),
                original_offset: entry.byte_offset,
                recovered_offset: Some(actual),
            });
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.xref.offset_repaired",
                    format!("xref offset for {reference} disagrees with the object scan"),
                )
                .partial(),
            );
        }
    }
}

fn trailer_object_locator(start: usize, end: usize) -> PdfObjectLocator {
    let mut locator = PdfObjectLocator::direct(
        PdfReference {
            object_number: 0,
            generation: 0,
        },
        start,
        end,
    );
    locator.stable_id = "pdf-trailer".into();
    locator
}

fn latest_objects(objects: &[RawObject]) -> BTreeMap<PdfReference, &RawObject> {
    let mut latest = BTreeMap::new();
    for object in objects {
        latest.insert(object.model.object, object);
    }
    latest
}

fn root_reference(
    trailer: &BTreeMap<String, PdfValue>,
    objects: &BTreeMap<PdfReference, &RawObject>,
) -> (Option<PdfReference>, bool) {
    if let Some(root) = trailer.get("Root").and_then(PdfValue::as_reference) {
        return (Some(root), false);
    }
    (
        objects
            .iter()
            .find(|(_, object)| {
                object
                    .model
                    .value
                    .as_dictionary()
                    .and_then(|dictionary| dictionary.get("Type"))
                    .and_then(PdfValue::as_name)
                    == Some("Catalog")
            })
            .map(|(reference, _)| *reference),
        true,
    )
}

fn build_catalog(
    reference: PdfReference,
    objects: &BTreeMap<PdfReference, &RawObject>,
) -> Result<PdfCatalog, ParserError> {
    let object = objects.get(&reference).ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            format!("catalog object {reference} is missing"),
        )) as ParserError
    })?;
    let dictionary = object.model.value.as_dictionary().ok_or_else(|| {
        Box::new(Diagnostic::malformed(
            PARSER,
            format!("catalog object {reference} is not a dictionary"),
        )) as ParserError
    })?;
    let pages = dictionary
        .get("Pages")
        .and_then(PdfValue::as_reference)
        .ok_or_else(|| {
            Box::new(Diagnostic::malformed(
                PARSER,
                "catalog has no Pages reference",
            )) as ParserError
        })?;
    Ok(PdfCatalog {
        object: reference,
        locator: object.model.locator.clone(),
        pages,
        version: dictionary
            .get("Version")
            .and_then(PdfValue::as_name)
            .map(str::to_string),
        page_labels: dictionary
            .get("PageLabels")
            .and_then(PdfValue::as_reference),
        metadata: dictionary.get("Metadata").and_then(PdfValue::as_reference),
        outlines: dictionary.get("Outlines").and_then(PdfValue::as_reference),
        names: dictionary.get("Names").and_then(PdfValue::as_reference),
        acro_form: dictionary.get("AcroForm").and_then(PdfValue::as_reference),
        destinations: dictionary.get("Dests").and_then(PdfValue::as_reference),
        optional_content: dictionary
            .get("OCProperties")
            .and_then(PdfValue::as_reference),
        structure_tree_root: dictionary
            .get("StructTreeRoot")
            .and_then(PdfValue::as_reference),
        language: dictionary.get("Lang").and_then(pdf_text),
    })
}

fn build_trailer(
    dictionary: &BTreeMap<String, PdfValue>,
    locator: PdfObjectLocator,
    root: PdfReference,
) -> PdfTrailer {
    PdfTrailer {
        locator,
        size: dictionary
            .get("Size")
            .and_then(PdfValue::as_integer)
            .and_then(|value| u64::try_from(value).ok()),
        root,
        info: dictionary.get("Info").and_then(PdfValue::as_reference),
        encrypt: dictionary.get("Encrypt").and_then(PdfValue::as_reference),
        id: dictionary
            .get("ID")
            .and_then(|value| match value {
                PdfValue::Array(values) => Some(
                    values
                        .iter()
                        .filter_map(|value| match value {
                            PdfValue::String(value) => Some(value.clone()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default(),
        previous_xref: dictionary
            .get("Prev")
            .and_then(PdfValue::as_integer)
            .and_then(|value| u64::try_from(value).ok()),
    }
}

fn build_encryption(
    trailer: &PdfTrailer,
    objects: &BTreeMap<PdfReference, &RawObject>,
    credential_supplied: bool,
) -> PdfEncryption {
    let Some(reference) = trailer.encrypt else {
        return PdfEncryption::unencrypted();
    };
    let object = objects.get(&reference);
    let dictionary = object.and_then(|object| object.model.value.as_dictionary());
    PdfEncryption {
        encrypted: true,
        object: Some(reference),
        handler: dictionary
            .and_then(|dictionary| dictionary.get("Filter"))
            .and_then(PdfValue::as_name)
            .map(str::to_string),
        sub_filter: dictionary
            .and_then(|dictionary| dictionary.get("SubFilter"))
            .and_then(PdfValue::as_name)
            .map(str::to_string),
        algorithm_version: dictionary
            .and_then(|dictionary| dictionary.get("V"))
            .and_then(PdfValue::as_integer),
        revision: dictionary
            .and_then(|dictionary| dictionary.get("R"))
            .and_then(PdfValue::as_integer),
        key_length_bits: dictionary
            .and_then(|dictionary| dictionary.get("Length"))
            .and_then(PdfValue::as_integer)
            .and_then(|value| u64::try_from(value).ok()),
        permissions: dictionary
            .and_then(|dictionary| dictionary.get("P"))
            .and_then(PdfValue::as_integer),
        encrypt_metadata: dictionary
            .and_then(|dictionary| dictionary.get("EncryptMetadata"))
            .and_then(|value| match value {
                PdfValue::Boolean(value) => Some(*value),
                _ => None,
            }),
        string_filter: dictionary
            .and_then(|dictionary| dictionary.get("StrF"))
            .and_then(PdfValue::as_name)
            .map(str::to_string),
        stream_filter: dictionary
            .and_then(|dictionary| dictionary.get("StmF"))
            .and_then(PdfValue::as_name)
            .map(str::to_string),
        embedded_file_filter: dictionary
            .and_then(|dictionary| dictionary.get("EFF"))
            .and_then(PdfValue::as_name)
            .map(str::to_string),
        credential_supplied,
        locator: object.map(|object| object.model.locator.clone()),
    }
}

#[derive(Clone, Copy, Default)]
struct InheritedPage {
    media_box: Option<PdfRectangle>,
    crop_box: Option<PdfRectangle>,
    rotation: i64,
    user_unit: Option<f64>,
}

fn build_pages(
    catalog: &PdfCatalog,
    objects: &BTreeMap<PdfReference, &RawObject>,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(Vec<PdfPageTreeNode>, Vec<PdfPage>), ParserError> {
    let mut tree = Vec::new();
    let mut pages = Vec::new();
    let mut stack = HashSet::new();
    walk_page_tree(
        catalog.pages,
        None,
        InheritedPage::default(),
        0,
        objects,
        options,
        diagnostics,
        &mut stack,
        &mut tree,
        &mut pages,
    )?;
    if pages.is_empty() {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "PDF page tree contains no pages",
        )));
    }
    Ok((tree, pages))
}

#[allow(clippy::too_many_arguments)]
fn walk_page_tree(
    reference: PdfReference,
    expected_parent: Option<PdfReference>,
    inherited: InheritedPage,
    depth: u16,
    objects: &BTreeMap<PdfReference, &RawObject>,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
    stack: &mut HashSet<PdfReference>,
    tree: &mut Vec<PdfPageTreeNode>,
    pages: &mut Vec<PdfPage>,
) -> Result<u64, ParserError> {
    if depth > options.max_page_tree_depth {
        diagnostics.push(Diagnostic::budget_exhausted(
            PARSER,
            format!("page tree depth exceeds {}", options.max_page_tree_depth),
        ));
        return Ok(0);
    }
    if !stack.insert(reference) {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.page_tree.cycle",
                format!("page tree cycle at {reference}"),
            )
            .partial(),
        );
        return Ok(0);
    }
    let Some(object) = objects.get(&reference) else {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.page_tree.missing_object",
                format!("page tree references missing object {reference}"),
            )
            .partial(),
        );
        stack.remove(&reference);
        return Ok(0);
    };
    let Some(dictionary) = object.model.value.as_dictionary() else {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.page_tree.invalid_object",
                format!("page tree object {reference} is not a dictionary"),
            )
            .partial(),
        );
        stack.remove(&reference);
        return Ok(0);
    };
    let parent = dictionary.get("Parent").and_then(PdfValue::as_reference);
    if expected_parent.is_some() && parent != expected_parent {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.page_tree.parent_mismatch",
                format!("page tree parent mismatch for {reference}"),
            )
            .partial(),
        );
    }
    let inherited = inherit_page_values(dictionary, inherited, objects);
    let kids = reference_array(dictionary.get("Kids"));
    let kind = dictionary.get("Type").and_then(PdfValue::as_name);
    let is_page = kind == Some("Page") || (kind.is_none() && kids.is_empty());
    let count = if is_page {
        if pages.len() >= options.max_pages as usize {
            diagnostics.push(Diagnostic::budget_exhausted(
                PARSER,
                format!("PDF page limit {} was reached", options.max_pages),
            ));
            0
        } else {
            pages.push(build_page(
                reference,
                parent,
                object,
                inherited,
                pages.len() as u64 + 1,
                diagnostics,
            ));
            1
        }
    } else {
        let node_index = tree.len();
        tree.push(PdfPageTreeNode {
            object: reference,
            parent,
            kids: kids.clone(),
            declared_count: dictionary
                .get("Count")
                .and_then(PdfValue::as_integer)
                .and_then(|value| u64::try_from(value).ok()),
            actual_descendant_pages: 0,
            locator: object.model.locator.clone(),
        });
        let mut descendant_pages = 0u64;
        for kid in kids {
            descendant_pages = descendant_pages.saturating_add(walk_page_tree(
                kid,
                Some(reference),
                inherited,
                depth.saturating_add(1),
                objects,
                options,
                diagnostics,
                stack,
                tree,
                pages,
            )?);
            if pages.len() >= options.max_pages as usize {
                break;
            }
        }
        if pages.len() >= options.max_pages as usize
            && tree[node_index]
                .declared_count
                .is_some_and(|declared| declared > descendant_pages)
        {
            diagnostics.push(Diagnostic::budget_exhausted(
                PARSER,
                format!("PDF page limit {} was reached", options.max_pages),
            ));
        }
        tree[node_index].actual_descendant_pages = descendant_pages;
        descendant_pages
    };
    stack.remove(&reference);
    Ok(count)
}

fn inherit_page_values(
    dictionary: &BTreeMap<String, PdfValue>,
    mut inherited: InheritedPage,
    objects: &BTreeMap<PdfReference, &RawObject>,
) -> InheritedPage {
    if let Some(value) = dictionary
        .get("MediaBox")
        .and_then(|value| rectangle(value, objects))
    {
        inherited.media_box = Some(value);
    }
    if let Some(value) = dictionary
        .get("CropBox")
        .and_then(|value| rectangle(value, objects))
    {
        inherited.crop_box = Some(value);
    }
    if let Some(value) = dictionary
        .get("Rotate")
        .and_then(|value| resolve(value, objects))
        .and_then(PdfValue::as_integer)
    {
        inherited.rotation = value;
    }
    if let Some(value) = dictionary
        .get("UserUnit")
        .and_then(|value| number(resolve(value, objects)?))
    {
        inherited.user_unit = Some(value);
    }
    inherited
}

fn build_page(
    reference: PdfReference,
    parent: Option<PdfReference>,
    object: &RawObject,
    inherited: InheritedPage,
    index: u64,
    diagnostics: &mut Vec<Diagnostic>,
) -> PdfPage {
    let media_box = inherited.media_box.unwrap_or_else(|| {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.page.media_box_missing",
                format!("page {index} has no inherited MediaBox"),
            )
            .partial(),
        );
        PdfRectangle {
            left: 0.0,
            bottom: 0.0,
            right: 0.0,
            top: 0.0,
        }
    });
    let crop_box = inherited.crop_box.unwrap_or(media_box);
    let rotation = inherited.rotation.rem_euclid(360);
    let rotation_degrees = if matches!(rotation, 0 | 90 | 180 | 270) {
        rotation as i16
    } else {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.page.rotation_invalid",
                format!(
                    "page {index} rotation {} is not a multiple of 90",
                    inherited.rotation
                ),
            )
            .partial(),
        );
        0
    };
    let user_unit = inherited
        .user_unit
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    let mut width = crop_box.width() * user_unit;
    let mut height = crop_box.height() * user_unit;
    if matches!(rotation_degrees, 90 | 270) {
        std::mem::swap(&mut width, &mut height);
    }
    let bbox = BoundingBox {
        x: crop_box.left,
        y: crop_box.bottom,
        width: crop_box.width(),
        height: crop_box.height(),
        unit: CoordinateUnit::Points,
        origin: CoordinateOrigin::BottomLeft,
    };
    let locator = SourceLocator::exact(LocationComponent::PdfRegion {
        page: IndexPosition::new(index, IndexBase::One).expect("one-based PDF page"),
        bbox: Some(bbox),
        rotation_degrees: Some(rotation_degrees),
        tokens: None,
    })
    .expect("valid PDF page locator");
    PdfPage {
        index,
        object: reference,
        parent,
        media_box,
        crop_box,
        width_points: width,
        height_points: height,
        rotation_degrees,
        user_unit,
        label: None,
        locator,
        object_locator: object.model.locator.clone(),
    }
}

fn resolve<'a>(
    value: &'a PdfValue,
    objects: &'a BTreeMap<PdfReference, &RawObject>,
) -> Option<&'a PdfValue> {
    match value {
        PdfValue::Reference(reference) => objects.get(reference).map(|object| &object.model.value),
        value => Some(value),
    }
}

fn rectangle(
    value: &PdfValue,
    objects: &BTreeMap<PdfReference, &RawObject>,
) -> Option<PdfRectangle> {
    let PdfValue::Array(values) = resolve(value, objects)? else {
        return None;
    };
    let coordinates = values
        .iter()
        .filter_map(|value| number(resolve(value, objects)?))
        .collect::<Vec<_>>();
    (coordinates.len() == 4 && coordinates.iter().all(|value| value.is_finite())).then(|| {
        PdfRectangle {
            left: coordinates[0],
            bottom: coordinates[1],
            right: coordinates[2],
            top: coordinates[3],
        }
    })
}

fn number(value: &PdfValue) -> Option<f64> {
    match value {
        PdfValue::Integer(value) => Some(*value as f64),
        PdfValue::Real(value) => Some(*value),
        _ => None,
    }
}

fn reference_array(value: Option<&PdfValue>) -> Vec<PdfReference> {
    match value {
        Some(PdfValue::Array(values)) => values.iter().filter_map(PdfValue::as_reference).collect(),
        _ => Vec::new(),
    }
}

fn reconcile_page_counts(
    tree: &mut [PdfPageTreeNode],
    pages: &[PdfPage],
    diagnostics: &mut Vec<Diagnostic>,
    repairs: &mut Vec<PdfRepair>,
) {
    for node in tree.iter_mut() {
        if node.declared_count != Some(node.actual_descendant_pages) {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.page_tree.count_mismatch",
                    format!(
                        "page tree {} declares {:?} pages but contains {}",
                        node.object, node.declared_count, node.actual_descendant_pages
                    ),
                )
                .partial(),
            );
            repairs.push(PdfRepair {
                code: "pdf.page_tree.count_mismatch".into(),
                description: format!("used traversed page count for {}", node.object),
                object: Some(node.object),
                original_offset: node.declared_count,
                recovered_offset: Some(node.actual_descendant_pages),
            });
        }
    }
    if let Some(root) = tree.first()
        && root.actual_descendant_pages != pages.len() as u64
    {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "pdf.page_tree.traversal_partial",
                "page-tree traversal did not retain every declared descendant",
            )
            .partial(),
        );
    }
}

#[derive(Clone)]
struct LabelRule {
    start_page: u64,
    style: Option<PdfPageLabelStyle>,
    prefix: Option<String>,
    start: u64,
    locator: PdfObjectLocator,
}

fn build_page_labels(
    catalog: &PdfCatalog,
    page_count: usize,
    objects: &BTreeMap<PdfReference, &RawObject>,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PdfPageLabel> {
    let Some(root) = catalog.page_labels else {
        return Vec::new();
    };
    let mut rules = Vec::new();
    let mut visited = HashSet::new();
    collect_label_rules(
        root,
        objects,
        options,
        diagnostics,
        &mut visited,
        &mut rules,
    );
    rules.sort_by_key(|rule| rule.start_page);
    let mut labels = Vec::with_capacity(page_count);
    for page_zero in 0..page_count as u64 {
        let Some(rule) = rules.iter().rev().find(|rule| rule.start_page <= page_zero) else {
            continue;
        };
        let sequence = rule
            .start
            .saturating_add(page_zero.saturating_sub(rule.start_page));
        let label = format!(
            "{}{}",
            rule.prefix.as_deref().unwrap_or_default(),
            format_label(rule.style, sequence)
        );
        labels.push(PdfPageLabel {
            page_index: page_zero + 1,
            label,
            style: rule.style,
            prefix: rule.prefix.clone(),
            start: sequence,
            rule_locator: rule.locator.clone(),
        });
    }
    labels
}

fn collect_label_rules(
    reference: PdfReference,
    objects: &BTreeMap<PdfReference, &RawObject>,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
    visited: &mut HashSet<PdfReference>,
    rules: &mut Vec<LabelRule>,
) {
    if !visited.insert(reference) || visited.len() > options.max_page_label_nodes as usize {
        diagnostics.push(Diagnostic::budget_exhausted(
            PARSER,
            "page-label number tree is cyclic or exceeds its node budget",
        ));
        return;
    }
    let Some(object) = objects.get(&reference) else {
        return;
    };
    let Some(dictionary) = object.model.value.as_dictionary() else {
        return;
    };
    if let Some(PdfValue::Array(nums)) = dictionary.get("Nums") {
        for pair in nums.chunks_exact(2) {
            let Some(page) = pair[0]
                .as_integer()
                .and_then(|value| u64::try_from(value).ok())
            else {
                continue;
            };
            let Some(rule_dictionary) =
                resolve(&pair[1], objects).and_then(PdfValue::as_dictionary)
            else {
                continue;
            };
            let style = match rule_dictionary.get("S").and_then(PdfValue::as_name) {
                Some("D") => Some(PdfPageLabelStyle::Decimal),
                Some("R") => Some(PdfPageLabelStyle::UpperRoman),
                Some("r") => Some(PdfPageLabelStyle::LowerRoman),
                Some("A") => Some(PdfPageLabelStyle::UpperLetters),
                Some("a") => Some(PdfPageLabelStyle::LowerLetters),
                None => Some(PdfPageLabelStyle::None),
                _ => None,
            };
            let prefix = rule_dictionary.get("P").and_then(pdf_text);
            let start = rule_dictionary
                .get("St")
                .and_then(PdfValue::as_integer)
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(1)
                .max(1);
            rules.push(LabelRule {
                start_page: page,
                style,
                prefix,
                start,
                locator: object.model.locator.at_key("Nums"),
            });
        }
    }
    for kid in reference_array(dictionary.get("Kids")) {
        collect_label_rules(kid, objects, options, diagnostics, visited, rules);
    }
}

fn format_label(style: Option<PdfPageLabelStyle>, value: u64) -> String {
    match style.unwrap_or(PdfPageLabelStyle::None) {
        PdfPageLabelStyle::Decimal => value.to_string(),
        PdfPageLabelStyle::UpperRoman => roman(value).to_ascii_uppercase(),
        PdfPageLabelStyle::LowerRoman => roman(value),
        PdfPageLabelStyle::UpperLetters => letters(value).to_ascii_uppercase(),
        PdfPageLabelStyle::LowerLetters => letters(value),
        PdfPageLabelStyle::None => String::new(),
    }
}

fn roman(mut value: u64) -> String {
    if value == 0 || value > 3999 {
        return value.to_string();
    }
    let mut output = String::new();
    for (amount, digits) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while value >= amount {
            output.push_str(digits);
            value -= amount;
        }
    }
    output
}

fn letters(mut value: u64) -> String {
    if value == 0 {
        return String::new();
    }
    let mut output = Vec::new();
    while value > 0 {
        value -= 1;
        output.push(char::from(b'a' + (value % 26) as u8));
        value /= 26;
    }
    output.into_iter().rev().collect()
}

fn build_metadata(
    catalog: &PdfCatalog,
    trailer: &PdfTrailer,
    latest: &BTreeMap<PdfReference, &RawObject>,
    objects: &[RawObject],
    diagnostics: &mut Vec<Diagnostic>,
) -> PdfMetadata {
    let mut fields = Vec::new();
    if let Some(reference) = trailer.info
        && let Some(object) = latest.get(&reference)
        && let Some(dictionary) = object.model.value.as_dictionary()
    {
        fields.extend(dictionary.iter().map(|(name, value)| PdfMetadataField {
            name: name.clone(),
            value: value.clone(),
            locator: object.model.locator.at_key(name),
        }));
    }
    let xmp_object = catalog.metadata;
    let mut xmp = None;
    let mut xmp_sha256 = None;
    if let Some(reference) = xmp_object
        && let Some(object) = objects
            .iter()
            .rev()
            .find(|object| object.model.object == reference)
    {
        if let Some(bytes) = object.decoded_stream.as_deref() {
            xmp_sha256 = Some(sha256_hex(bytes));
            match std::str::from_utf8(bytes) {
                Ok(value) => xmp = Some(value.to_string()),
                Err(_) => diagnostics.push(
                    Diagnostic::warning(
                        PARSER,
                        "pdf.metadata.xmp_encoding",
                        "XMP metadata stream is not valid UTF-8",
                    )
                    .partial(),
                ),
            }
        } else {
            diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "pdf.metadata.xmp_unavailable",
                    "XMP metadata stream could not be decoded",
                )
                .partial(),
            );
        }
    }
    PdfMetadata {
        info_object: trailer.info,
        fields,
        xmp_object,
        xmp,
        xmp_sha256,
    }
}

fn build_filter_inventory(objects: &[RawObject]) -> Vec<PdfFilterUsage> {
    let mut filters = objects
        .iter()
        .filter_map(|object| {
            let dictionary = object.model.value.as_dictionary()?;
            let names = filter_names(dictionary);
            if names.is_empty() {
                return None;
            }
            let status = object.model.stream.as_ref()?.decode_status.clone();
            Some(PdfFilterUsage {
                object: object.model.object,
                filters: names,
                supported: !matches!(status, PdfStreamDecodeStatus::Unsupported),
                status,
                decoded_bytes: object
                    .model
                    .stream
                    .as_ref()
                    .and_then(|stream| stream.decoded_length),
                locator: object.model.locator.at_key("Filter"),
            })
        })
        .collect::<Vec<_>>();
    filters.sort_by_key(|usage| usage.object);
    filters
}

fn inventory_active_content(objects: &[RawObject]) -> Vec<PdfActiveContent> {
    let mut inventory = Vec::new();
    for object in objects {
        if let Some(dictionary) = object.model.value.as_dictionary() {
            scan_active_dictionary(
                object.model.object,
                &object.model.locator,
                dictionary,
                0,
                &mut inventory,
            );
        }
    }
    inventory.sort_by(|left, right| {
        (left.object, &left.key, &left.locator.key_path).cmp(&(
            right.object,
            &right.key,
            &right.locator.key_path,
        ))
    });
    inventory
}

fn scan_active_dictionary(
    reference: PdfReference,
    locator: &PdfObjectLocator,
    dictionary: &BTreeMap<String, PdfValue>,
    depth: u16,
    inventory: &mut Vec<PdfActiveContent>,
) {
    if depth > 64 {
        return;
    }
    const ACTIVE_KEYS: &[&str] = &[
        "OpenAction",
        "AA",
        "A",
        "JS",
        "Launch",
        "SubmitForm",
        "ImportData",
        "GoToR",
        "URI",
    ];
    for (key, value) in dictionary {
        let child_locator = locator.at_key(key);
        if ACTIVE_KEYS.contains(&key.as_str()) {
            let containing_action_type = dictionary
                .get("S")
                .and_then(PdfValue::as_name)
                .map(str::to_string);

            let action_type = value
                .as_dictionary()
                .and_then(|dictionary| dictionary.get("S"))
                .and_then(PdfValue::as_name)
                .map(str::to_string)
                .or(containing_action_type)
                .or_else(|| value.as_name().map(str::to_string));
            inventory.push(PdfActiveContent {
                object: reference,
                key: key.clone(),
                action_type,
                locator: child_locator.clone(),
                disposition: PdfActiveContentDisposition::InventoriedNotExecuted,
            });
        }
        match value {
            PdfValue::Dictionary(child) => {
                scan_active_dictionary(reference, &child_locator, child, depth + 1, inventory)
            }
            PdfValue::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    if let PdfValue::Dictionary(child) = value {
                        scan_active_dictionary(
                            reference,
                            &child_locator.at_key(index.to_string()),
                            child,
                            depth + 1,
                            inventory,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

fn pdf_text(value: &PdfValue) -> Option<String> {
    match value {
        PdfValue::String(value) => Some(value.text.clone()),
        PdfValue::Name(value) | PdfValue::Keyword(value) => Some(value.clone()),
        _ => None,
    }
}
