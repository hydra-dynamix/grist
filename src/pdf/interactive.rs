use super::filters::decode_stream;
use super::syntax::{RawObject, scan_indirect_objects};
use super::*;
use crate::container::{
    ArtifactDisposition, ArtifactExtraction, ArtifactExtractionStatus, ArtifactMetadata,
    ArtifactParent, ArtifactRelationship, ArtifactSafetyClassification, EmbeddedArtifact,
};
use crate::core::{
    BoundingBox, ContentIdentity, CoordinateOrigin, CoordinateUnit, Diagnostic, FormatIdentity,
    IndexBase, IndexPosition, LocationComponent, SourceLocator, sha256_hex,
};
use std::collections::{BTreeMap, HashMap, HashSet};

type Latest<'a> = BTreeMap<PdfReference, &'a RawObject>;

pub(super) fn extract_interactive_content(
    bytes: &[u8],
    catalog: &PdfCatalog,
    pages: &[PdfPage],
    latest: &Latest<'_>,
    options: &PdfOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> PdfInteractiveContent {
    let mut state = State::new(options);
    let destinations = destinations(catalog, pages, latest, &mut state);
    let outlines = outlines(catalog, pages, latest, &mut state);
    let mut output = PdfInteractiveContent {
        destinations,
        outlines,
        ..PdfInteractiveContent::default()
    };
    annotations(pages, latest, &mut state, &mut output);
    let (form, signatures, form_relationships) = form(catalog, latest, &mut state);
    output.form = form;
    output.signatures = signatures;
    output.relationships.extend(form_relationships);
    output.layers = layers(catalog, latest, &mut state);
    output.embedded_files = embedded_files(bytes, catalog, latest, options, &mut state);
    relationships(&mut output);
    if state.limit_hit {
        diagnostics.push(
            Diagnostic::budget_exhausted(
                "grist.pdf",
                format!(
                    "PDF interactive object limit {} was reached",
                    options.max_interactive_objects
                ),
            )
            .partial(),
        );
    }
    if state.embedded_limit_hit {
        diagnostics.push(
            Diagnostic::budget_exhausted(
                "grist.pdf",
                "PDF embedded-file count, byte, or recursion-depth budget was reached",
            )
            .partial(),
        );
    }
    output
}

struct State {
    remaining: u64,
    files: u64,
    bytes: u64,
    limit_hit: bool,
    embedded_limit_hit: bool,
}

impl State {
    fn new(options: &PdfOptions) -> Self {
        Self {
            remaining: options.max_interactive_objects,
            files: 0,
            bytes: 0,
            limit_hit: false,
            embedded_limit_hit: false,
        }
    }

    fn take(&mut self) -> bool {
        if self.remaining == 0 {
            self.limit_hit = true;
            false
        } else {
            self.remaining -= 1;
            true
        }
    }

    fn take_file(&mut self, bytes: u64, depth: u16, options: &PdfOptions) -> bool {
        let allowed = depth <= options.max_embedded_depth
            && self.files < options.max_embedded_files
            && self.bytes.saturating_add(bytes) <= options.max_embedded_bytes;
        if allowed {
            self.files += 1;
            self.bytes = self.bytes.saturating_add(bytes);
        } else {
            self.embedded_limit_hit = true;
        }
        allowed
    }
}

fn destinations(
    catalog: &PdfCatalog,
    pages: &[PdfPage],
    latest: &Latest<'_>,
    state: &mut State,
) -> Vec<PdfNamedDestination> {
    let mut pairs = Vec::new();
    if let Some(catalog_dictionary) = dictionary(latest, catalog.object) {
        if let Some(dests) = catalog_dictionary
            .get("Dests")
            .and_then(|value| resolve_dictionary(value, latest))
        {
            let locator = object_locator(latest, catalog.object).at_key("Dests");
            for (name, value) in dests {
                pairs.push((name.clone(), value.clone(), locator.at_key(name)));
            }
        }
        if let Some(tree) = catalog_names(catalog, latest).and_then(|value| value.get("Dests")) {
            collect_name_tree(tree, latest, &mut pairs, &mut HashSet::new());
        }
    }
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    pairs
        .into_iter()
        .filter_map(|(name, value, locator)| {
            state.take().then(|| PdfNamedDestination {
                id: format!("pdf:destination:{name}"),
                name,
                destination: parse_destination(&value, pages, latest),
                locator,
            })
        })
        .collect()
}

fn outlines(
    catalog: &PdfCatalog,
    pages: &[PdfPage],
    latest: &Latest<'_>,
    state: &mut State,
) -> Vec<PdfOutlineItem> {
    let Some(first) = catalog
        .outlines
        .and_then(|root| dictionary(latest, root))
        .and_then(|value| value.get("First"))
        .and_then(PdfValue::as_reference)
    else {
        return Vec::new();
    };
    let mut output = Vec::new();
    let mut stack = vec![first];
    let mut visited = HashSet::new();
    while let Some(reference) = stack.pop() {
        if !visited.insert(reference) || !state.take() {
            continue;
        }
        let Some(value) = dictionary(latest, reference) else {
            continue;
        };
        let locator = object_locator(latest, reference);
        let first_child = reference_value(value, "First");
        let next_sibling = reference_value(value, "Next");
        stack.extend(next_sibling);
        stack.extend(first_child);
        let (destination, named_destination) = destination_value(value.get("Dest"), pages, latest);
        output.push(PdfOutlineItem {
            id: object_id("outline", reference),
            object: reference,
            title: text_value(value.get("Title")),
            parent: reference_value(value, "Parent"),
            first_child,
            next_sibling,
            previous_sibling: reference_value(value, "Prev"),
            destination,
            named_destination,
            action: parse_action(value.get("A"), pages, latest, &locator.at_key("A")),
            open: value
                .get("Count")
                .and_then(PdfValue::as_integer)
                .map(|count| count >= 0),
            locator,
        });
    }
    output.sort_by_key(|item| (item.object.object_number, item.object.generation));
    output
}

fn annotations(
    pages: &[PdfPage],
    latest: &Latest<'_>,
    state: &mut State,
    output: &mut PdfInteractiveContent,
) {
    for page in pages {
        let Some(page_dictionary) = dictionary(latest, page.object) else {
            continue;
        };
        for reference in references(page_dictionary.get("Annots")) {
            if !state.take() {
                return;
            }
            let Some(value) = dictionary(latest, reference) else {
                continue;
            };
            let subtype = name_value(value.get("Subtype")).unwrap_or_else(|| "Unknown".into());
            let rectangle = rectangle(value.get("Rect"));
            let locator = page_locator(page, rectangle);
            let id = object_id("annotation", reference);
            let action = parse_action(
                value.get("A"),
                pages,
                latest,
                &object_locator(latest, reference).at_key("A"),
            );
            let contents = text_value(value.get("Contents"));
            let author = text_value(value.get("T"));
            let subject = text_value(value.get("Subj"));
            let in_reply_to = reference_value(value, "IRT");
            output.annotations.push(PdfAnnotation {
                id: id.clone(),
                object: reference,
                page_index: page.index,
                subtype: subtype.clone(),
                rectangle,
                contents: contents.clone(),
                author: author.clone(),
                subject: subject.clone(),
                modified: text_value(value.get("M")),
                in_reply_to,
                popup: reference_value(value, "Popup"),
                action: action.clone(),
                locator: locator.clone(),
            });
            output.relationships.push(PdfInteractiveRelationship {
                source_id: id.clone(),
                relation: PdfInteractiveRelation::AnnotationFor,
                target_id: format!("pdf:page:{}", page.index),
                locator: object_locator(latest, reference),
            });
            if subtype == "Widget" {
                let field = reference_value(value, "Parent").unwrap_or(reference);
                output.relationships.push(PdfInteractiveRelationship {
                    source_id: object_id("field", field),
                    relation: PdfInteractiveRelation::FieldWidget,
                    target_id: id.clone(),
                    locator: object_locator(latest, reference),
                });
            }
            if let Some(parent) = in_reply_to {
                output.relationships.push(PdfInteractiveRelationship {
                    source_id: id.clone(),
                    relation: PdfInteractiveRelation::ReplyTo,
                    target_id: object_id("annotation", parent),
                    locator: object_locator(latest, reference).at_key("IRT"),
                });
            }
            if let Some(text) = contents.filter(|value| !value.is_empty())
                && matches!(subtype.as_str(), "Text" | "FreeText" | "Stamp" | "Caret")
            {
                output.comments.push(PdfComment {
                    id: object_id("comment", reference),
                    annotation_id: id.clone(),
                    text,
                    author,
                    subject,
                    in_reply_to,
                    locator: locator.clone(),
                });
            }
            if subtype == "Link" {
                let (destination, named_destination) =
                    destination_value(value.get("Dest"), pages, latest);
                output.links.push(PdfLink {
                    id: object_id("link", reference),
                    annotation_object: reference,
                    page_index: page.index,
                    rectangle,
                    destination,
                    named_destination,
                    action,
                    locator,
                });
            }
        }
    }
}

fn form(
    catalog: &PdfCatalog,
    latest: &Latest<'_>,
    state: &mut State,
) -> (
    Option<PdfForm>,
    Vec<PdfSignature>,
    Vec<PdfInteractiveRelationship>,
) {
    let Some(form_value) =
        dictionary(latest, catalog.object).and_then(|value| value.get("AcroForm"))
    else {
        return (None, Vec::new(), Vec::new());
    };
    let (reference, root, form_locator) = match form_value {
        PdfValue::Reference(reference) => {
            let Some(root) = dictionary(latest, *reference) else {
                return (None, Vec::new(), Vec::new());
            };
            (*reference, root, object_locator(latest, *reference))
        }
        PdfValue::Dictionary(root) => (
            catalog.object,
            root,
            object_locator(latest, catalog.object).at_key("AcroForm"),
        ),
        _ => return (None, Vec::new(), Vec::new()),
    };
    let mut fields = Vec::new();
    let mut signatures = Vec::new();
    let mut relationships = Vec::new();
    let mut stack = references(root.get("Fields"));
    let mut visited = HashSet::new();
    while let Some(field_reference) = stack.pop() {
        if !visited.insert(field_reference) || !state.take() {
            continue;
        }
        let Some(value) = dictionary(latest, field_reference) else {
            continue;
        };
        let children = references(value.get("Kids"));
        stack.extend(children.iter().copied());
        let locator = object_locator(latest, field_reference);
        let id = object_id("field", field_reference);
        let parent = reference_value(value, "Parent");
        if let Some(parent) = parent {
            relationships.push(PdfInteractiveRelationship {
                source_id: object_id("field", parent),
                relation: PdfInteractiveRelation::ParentOf,
                target_id: id.clone(),
                locator: locator.at_key("Parent"),
            });
        }
        let field_type = inherited_name(value, parent, "FT", latest);
        if field_type.as_deref() == Some("Sig") {
            signatures.push(signature(field_reference, value, latest));
        }
        fields.push(PdfFormField {
            id,
            object: field_reference,
            parent,
            children,
            field_type,
            partial_name: text_value(value.get("T")),
            alternate_name: text_value(value.get("TU")),
            mapping_name: text_value(value.get("TM")),
            value: value.get("V").cloned(),
            default_value: value.get("DV").cloned(),
            flags: value.get("Ff").and_then(PdfValue::as_integer),
            action: parse_action(value.get("A"), &[], latest, &locator.at_key("A")),
            locator,
        });
    }
    fields.sort_by_key(|field| (field.object.object_number, field.object.generation));
    signatures.sort_by(|left, right| left.id.cmp(&right.id));
    (
        Some(PdfForm {
            object: reference,
            need_appearances: boolean_value(root.get("NeedAppearances")),
            signature_flags: root.get("SigFlags").and_then(PdfValue::as_integer),
            calculation_order: references(root.get("CO")),
            fields,
            locator: form_locator,
        }),
        signatures,
        relationships,
    )
}

fn signature(
    field_reference: PdfReference,
    field: &BTreeMap<String, PdfValue>,
    latest: &Latest<'_>,
) -> PdfSignature {
    let signature_reference = field.get("V").and_then(PdfValue::as_reference);
    let value = signature_reference
        .and_then(|reference| dictionary(latest, reference))
        .or_else(|| field.get("V").and_then(PdfValue::as_dictionary));
    let locator = signature_reference
        .map(|reference| object_locator(latest, reference))
        .unwrap_or_else(|| object_locator(latest, field_reference).at_key("V"));
    PdfSignature {
        id: object_id("signature", field_reference),
        field_object: field_reference,
        signature_object: signature_reference,
        filter: value.and_then(|value| name_value(value.get("Filter"))),
        sub_filter: value.and_then(|value| name_value(value.get("SubFilter"))),
        signer_name: value.and_then(|value| text_value(value.get("Name"))),
        reason: value.and_then(|value| text_value(value.get("Reason"))),
        location: value.and_then(|value| text_value(value.get("Location"))),
        contact_info: value.and_then(|value| text_value(value.get("ContactInfo"))),
        signing_time: value.and_then(|value| text_value(value.get("M"))),
        byte_range: value
            .and_then(|value| value.get("ByteRange"))
            .map(integers)
            .unwrap_or_default(),
        contents_sha256: value.and_then(|value| value.get("Contents")).and_then(
            |value| match value {
                PdfValue::String(value) => Some(sha256_hex(value.raw_hex.as_bytes())),
                _ => None,
            },
        ),
        locator,
    }
}

fn inherited_name(
    value: &BTreeMap<String, PdfValue>,
    parent: Option<PdfReference>,
    key: &str,
    latest: &Latest<'_>,
) -> Option<String> {
    name_value(value.get(key)).or_else(|| {
        parent
            .and_then(|parent| dictionary(latest, parent))
            .and_then(|parent| name_value(parent.get(key)))
    })
}

fn layers(catalog: &PdfCatalog, latest: &Latest<'_>, state: &mut State) -> Vec<PdfLayer> {
    let Some(root_value) =
        dictionary(latest, catalog.object).and_then(|value| value.get("OCProperties"))
    else {
        return Vec::new();
    };
    let Some(root) = resolve_dictionary(root_value, latest) else {
        return Vec::new();
    };
    let default = root
        .get("D")
        .and_then(|value| resolve_dictionary(value, latest));
    let on = default
        .and_then(|value| value.get("ON"))
        .map(|value| references(Some(value)))
        .unwrap_or_default();
    let off = default
        .and_then(|value| value.get("OFF"))
        .map(|value| references(Some(value)))
        .unwrap_or_default();
    let mut output = references(root.get("OCGs"))
        .into_iter()
        .filter_map(|reference| {
            if !state.take() {
                return None;
            }
            let value = dictionary(latest, reference)?;
            Some(PdfLayer {
                id: object_id("layer", reference),
                object: reference,
                name: text_value(value.get("Name")),
                intent: names(value.get("Intent")),
                usage: value.get("Usage").cloned(),
                initially_visible: if off.contains(&reference) {
                    Some(false)
                } else if on.contains(&reference) {
                    Some(true)
                } else {
                    None
                },
                locator: object_locator(latest, reference),
            })
        })
        .collect::<Vec<_>>();
    output.sort_by_key(|layer| (layer.object.object_number, layer.object.generation));
    output
}

fn embedded_files(
    bytes: &[u8],
    catalog: &PdfCatalog,
    latest: &Latest<'_>,
    options: &PdfOptions,
    state: &mut State,
) -> Vec<PdfEmbeddedFile> {
    let mut pairs = catalog_names(catalog, latest)
        .and_then(|value| value.get("EmbeddedFiles"))
        .map(|tree| {
            let mut pairs = Vec::new();
            collect_name_tree(tree, latest, &mut pairs, &mut HashSet::new());
            pairs
        })
        .unwrap_or_default();
    let mut known = pairs
        .iter()
        .filter_map(|(_, value, _)| value.as_reference())
        .collect::<HashSet<_>>();
    for (reference, object) in latest {
        let Some(file_spec) = object.model.value.as_dictionary() else {
            continue;
        };
        if file_spec.get("Type").and_then(PdfValue::as_name) == Some("Filespec")
            && file_spec.contains_key("EF")
            && known.insert(*reference)
        {
            let name = text_value(file_spec.get("UF"))
                .or_else(|| text_value(file_spec.get("F")))
                .unwrap_or_else(|| format!("embedded-{}", reference.object_number));
            pairs.push((
                name,
                PdfValue::Reference(*reference),
                object.model.locator.clone(),
            ));
        }
    }
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    let parent = ContentIdentity::for_raw_bytes(bytes)
        .with_format(FormatIdentity::new("pdf", Some("application/pdf")));
    build_files(pairs, parent, latest, options, state, 1)
}

fn build_files(
    pairs: Vec<(String, PdfValue, PdfObjectLocator)>,
    parent: ContentIdentity,
    latest: &Latest<'_>,
    options: &PdfOptions,
    state: &mut State,
    depth: u16,
) -> Vec<PdfEmbeddedFile> {
    let mut output = Vec::new();
    for (tree_name, value, tree_locator) in pairs {
        if !state.take() {
            break;
        }
        let Some(file_reference) = value.as_reference() else {
            continue;
        };
        let Some(file_spec) = dictionary(latest, file_reference) else {
            continue;
        };
        let locator = object_locator(latest, file_reference);
        let filename = text_value(file_spec.get("UF"))
            .or_else(|| text_value(file_spec.get("F")))
            .unwrap_or(tree_name);
        let stream_reference = file_spec
            .get("EF")
            .and_then(|value| resolve_dictionary(value, latest))
            .and_then(|ef| ef.get("UF").or_else(|| ef.get("F")))
            .and_then(PdfValue::as_reference);
        let stream = stream_reference.and_then(|reference| latest.get(&reference).copied());
        let child_bytes = stream.and_then(|object| object.decoded_stream.as_deref());
        let media_type = stream_reference
            .and_then(|reference| dictionary(latest, reference))
            .and_then(|value| name_value(value.get("Subtype")));
        let child_locator = SourceLocator::exact(LocationComponent::JsonPointer {
            pointer: format!(
                "/pdf/objects/{}/embedded_file",
                file_reference.object_number
            ),
        })
        .expect("PDF embedded-file locator is valid");
        let metadata = ArtifactMetadata::new(
            ArtifactParent::new(parent.clone(), ArtifactRelationship::AttachmentOf),
            child_locator,
            ArtifactDisposition::Attachment,
        )
        .with_declared_filename(filename)
        .with_safety_hint(ArtifactSafetyClassification::Unknown);
        let metadata = media_type.map_or(metadata.clone(), |media_type| {
            metadata.with_media_type(media_type)
        });
        let length = child_bytes.map(|bytes| bytes.len() as u64).unwrap_or(0);
        let allowed = state.take_file(length, depth, options);
        let (artifact, child_status, children) = if !allowed {
            let extraction = ArtifactExtraction::new(
                ArtifactExtractionStatus::BudgetLimited,
                "pdf.embedded_file.budget_limited",
            );
            (
                if let Some(bytes) = child_bytes {
                    EmbeddedArtifact::record_known_unavailable(metadata, bytes, extraction)
                } else {
                    EmbeddedArtifact::record_unavailable(metadata, extraction)
                },
                PdfEmbeddedChildStatus::BudgetLimited,
                Vec::new(),
            )
        } else if let Some(bytes) = child_bytes {
            if bytes.starts_with(b"%PDF-") {
                nested_pdf(metadata, bytes, options, state, depth)
            } else {
                (
                    EmbeddedArtifact::record_known_unavailable(
                        metadata,
                        bytes,
                        ArtifactExtraction::new(
                            ArtifactExtractionStatus::Unsupported,
                            "pdf.embedded_file.child_parser_unsupported",
                        ),
                    ),
                    PdfEmbeddedChildStatus::Unsupported,
                    Vec::new(),
                )
            }
        } else {
            let encrypted = stream
                .and_then(|object| object.model.stream.as_ref())
                .is_some_and(|stream| stream.decode_status == PdfStreamDecodeStatus::Encrypted);
            let (status, code, child_status) = if encrypted {
                (
                    ArtifactExtractionStatus::Encrypted,
                    "pdf.embedded_file.encrypted",
                    PdfEmbeddedChildStatus::Encrypted,
                )
            } else {
                (
                    ArtifactExtractionStatus::Unsupported,
                    "pdf.embedded_file.filter_unsupported",
                    PdfEmbeddedChildStatus::Unsupported,
                )
            };
            (
                EmbeddedArtifact::record_unavailable(
                    metadata,
                    ArtifactExtraction::new(status, code),
                ),
                child_status,
                Vec::new(),
            )
        };
        if let Ok(artifact) = artifact {
            output.push(PdfEmbeddedFile {
                id: object_id("embedded-file", file_reference),
                file_spec_object: file_reference,
                stream_object: stream_reference,
                description: text_value(file_spec.get("Desc")),
                relationship: name_value(file_spec.get("AFRelationship")),
                artifact,
                child_status,
                children,
                locator: if stream_reference.is_some() {
                    locator
                } else {
                    tree_locator
                },
            });
        }
    }
    output.sort_by(|left, right| left.id.cmp(&right.id));
    output
}

fn nested_pdf(
    metadata: ArtifactMetadata,
    bytes: &[u8],
    options: &PdfOptions,
    state: &mut State,
    depth: u16,
) -> (
    Result<EmbeddedArtifact, crate::container::EmbeddedArtifactError>,
    PdfEmbeddedChildStatus,
    Vec<PdfEmbeddedFile>,
) {
    let child_media_type = metadata.media_type.clone();
    if bytes.windows(8).any(|window| window == b"/Encrypt") {
        return (
            EmbeddedArtifact::record_known_unavailable(
                metadata,
                bytes,
                ArtifactExtraction::new(
                    ArtifactExtractionStatus::Encrypted,
                    "pdf.embedded_file.child_encrypted",
                ),
            ),
            PdfEmbeddedChildStatus::Encrypted,
            Vec::new(),
        );
    }
    let (mut objects, _, _) = scan_indirect_objects(bytes, options);
    for object in &mut objects {
        let (Some(encoded), Some(dictionary)) = (
            object.stream_bytes.as_deref(),
            object.model.value.as_dictionary(),
        ) else {
            continue;
        };
        object.decoded_stream =
            decode_stream(encoded, dictionary, options.max_decoded_stream_bytes, false).bytes;
    }
    let nested_latest = latest_objects(&objects);
    let catalog = nested_latest.iter().find_map(|(reference, object)| {
        (object
            .model
            .value
            .as_dictionary()
            .and_then(|value| value.get("Type"))
            .and_then(PdfValue::as_name)
            == Some("Catalog"))
        .then_some(*reference)
    });
    let Some(catalog) = catalog else {
        return (
            EmbeddedArtifact::record_known_unavailable(
                metadata,
                bytes,
                ArtifactExtraction::new(
                    ArtifactExtractionStatus::Failed,
                    "pdf.embedded_file.child_malformed",
                ),
            ),
            PdfEmbeddedChildStatus::Malformed,
            Vec::new(),
        );
    };
    let mut pairs = Vec::new();
    if let Some(tree) = dictionary(&nested_latest, catalog)
        .and_then(|value| value.get("Names"))
        .and_then(PdfValue::as_reference)
        .and_then(|reference| dictionary(&nested_latest, reference))
        .and_then(|value| value.get("EmbeddedFiles"))
    {
        collect_name_tree(tree, &nested_latest, &mut pairs, &mut HashSet::new());
    }
    let parent = ContentIdentity::for_raw_bytes(bytes)
        .with_format(FormatIdentity::new("embedded_artifact", child_media_type));
    let children = build_files(
        pairs,
        parent,
        &nested_latest,
        options,
        state,
        depth.saturating_add(1),
    );
    (
        EmbeddedArtifact::inventory(metadata, bytes),
        PdfEmbeddedChildStatus::Parsed,
        children,
    )
}

fn latest_objects(objects: &[RawObject]) -> Latest<'_> {
    let mut output = BTreeMap::new();
    for object in objects {
        output.insert(object.model.object, object);
    }
    output
}

fn collect_name_tree(
    value: &PdfValue,
    latest: &Latest<'_>,
    output: &mut Vec<(String, PdfValue, PdfObjectLocator)>,
    visited: &mut HashSet<PdfReference>,
) {
    let (dictionary, locator) = match value {
        PdfValue::Reference(reference) if visited.insert(*reference) => (
            dictionary(latest, *reference),
            Some(object_locator(latest, *reference)),
        ),
        PdfValue::Dictionary(dictionary) => (Some(dictionary), None),
        _ => (None, None),
    };
    let Some(dictionary) = dictionary else {
        return;
    };
    let locator = locator.unwrap_or_else(|| {
        PdfObjectLocator::direct(
            PdfReference {
                object_number: 0,
                generation: 0,
            },
            0,
            0,
        )
    });
    if let Some(PdfValue::Array(names)) = dictionary.get("Names") {
        for pair in names.chunks_exact(2) {
            if let PdfValue::String(name) = &pair[0] {
                output.push((name.text.clone(), pair[1].clone(), locator.at_key("Names")));
            }
        }
    }
    for child in references(dictionary.get("Kids")) {
        collect_name_tree(&PdfValue::Reference(child), latest, output, visited);
    }
}

fn parse_action(
    value: Option<&PdfValue>,
    pages: &[PdfPage],
    latest: &Latest<'_>,
    fallback: &PdfObjectLocator,
) -> Option<PdfAction> {
    let (value, locator) = match value? {
        PdfValue::Reference(reference) => (
            dictionary(latest, *reference)?,
            object_locator(latest, *reference),
        ),
        PdfValue::Dictionary(value) => (value, fallback.clone()),
        _ => return None,
    };
    Some(PdfAction {
        action_type: name_value(value.get("S")).unwrap_or_else(|| "Unknown".into()),
        destination: value
            .get("D")
            .filter(|value| matches!(value, PdfValue::Array(_)))
            .map(|value| parse_destination(value, pages, latest)),
        uri: text_value(value.get("URI")),
        file: text_value(value.get("F")),
        named_action: name_value(value.get("N")),
        script_sha256: value.get("JS").and_then(|value| match value {
            PdfValue::String(value) => Some(sha256_hex(value.raw_hex.as_bytes())),
            PdfValue::Reference(reference) => latest
                .get(reference)
                .and_then(|object| object.decoded_stream.as_deref())
                .map(sha256_hex),
            _ => None,
        }),
        disposition: PdfActiveContentDisposition::InventoriedNotExecuted,
        locator,
    })
}

fn parse_destination(value: &PdfValue, pages: &[PdfPage], latest: &Latest<'_>) -> PdfDestination {
    let raw = value.clone();
    let resolved = match value {
        PdfValue::Dictionary(dictionary) => dictionary.get("D").unwrap_or(value),
        PdfValue::Reference(reference) => dictionary(latest, *reference)
            .and_then(|value| value.get("D"))
            .unwrap_or(value),
        _ => value,
    };
    let values = match resolved {
        PdfValue::Array(values) => values.as_slice(),
        _ => &[],
    };
    let page_object = values.first().and_then(PdfValue::as_reference);
    let page_index = page_object.and_then(|object| {
        pages
            .iter()
            .find(|page| page.object == object)
            .map(|page| page.index)
    });
    let view = match values
        .get(1)
        .and_then(PdfValue::as_name)
        .unwrap_or("Unknown")
    {
        "XYZ" => PdfDestinationView::Xyz,
        "Fit" => PdfDestinationView::Fit,
        "FitH" => PdfDestinationView::FitHorizontal,
        "FitV" => PdfDestinationView::FitVertical,
        "FitR" => PdfDestinationView::FitRectangle,
        "FitB" => PdfDestinationView::FitBoundingBox,
        "FitBH" => PdfDestinationView::FitBoundingBoxHorizontal,
        "FitBV" => PdfDestinationView::FitBoundingBoxVertical,
        other => PdfDestinationView::Unknown(other.into()),
    };
    PdfDestination {
        page_object,
        page_index,
        view,
        parameters: values.iter().skip(2).map(number).collect(),
        raw,
    }
}

fn destination_value(
    value: Option<&PdfValue>,
    pages: &[PdfPage],
    latest: &Latest<'_>,
) -> (Option<PdfDestination>, Option<String>) {
    match value {
        Some(PdfValue::String(value)) => (None, Some(value.text.clone())),
        Some(PdfValue::Name(value)) => (None, Some(value.clone())),
        Some(value) => (Some(parse_destination(value, pages, latest)), None),
        None => (None, None),
    }
}

fn relationships(output: &mut PdfInteractiveContent) {
    let names = output
        .destinations
        .iter()
        .map(|value| (value.name.as_str(), value.id.as_str()))
        .collect::<HashMap<_, _>>();
    for outline in &output.outlines {
        for (target, relation, key) in [
            (
                outline.first_child,
                PdfInteractiveRelation::ParentOf,
                "First",
            ),
            (
                outline.next_sibling,
                PdfInteractiveRelation::NextSibling,
                "Next",
            ),
            (
                outline.previous_sibling,
                PdfInteractiveRelation::PreviousSibling,
                "Prev",
            ),
        ] {
            if let Some(target) = target {
                output.relationships.push(PdfInteractiveRelationship {
                    source_id: outline.id.clone(),
                    relation,
                    target_id: object_id("outline", target),
                    locator: outline.locator.at_key(key),
                });
            }
        }
        if let Some(target) = outline
            .named_destination
            .as_deref()
            .and_then(|name| names.get(name))
        {
            output.relationships.push(PdfInteractiveRelationship {
                source_id: outline.id.clone(),
                relation: PdfInteractiveRelation::ResolvesTo,
                target_id: (*target).to_string(),
                locator: outline.locator.at_key("Dest"),
            });
        }
    }
    for link in &output.links {
        if let Some(target) = link
            .named_destination
            .as_deref()
            .and_then(|name| names.get(name))
        {
            output.relationships.push(PdfInteractiveRelationship {
                source_id: link.id.clone(),
                relation: PdfInteractiveRelation::ResolvesTo,
                target_id: (*target).to_string(),
                locator: PdfObjectLocator::direct(link.annotation_object, 0, 0).at_key("Dest"),
            });
        }
    }
    for file in &output.embedded_files {
        output.relationships.push(PdfInteractiveRelationship {
            source_id: "pdf:document".into(),
            relation: PdfInteractiveRelation::AttachmentOf,
            target_id: file.id.clone(),
            locator: file.locator.clone(),
        });
    }
    output.relationships.sort_by(|left, right| {
        (&left.source_id, &left.target_id).cmp(&(&right.source_id, &right.target_id))
    });
}

fn dictionary<'a>(
    latest: &'a Latest<'_>,
    reference: PdfReference,
) -> Option<&'a BTreeMap<String, PdfValue>> {
    latest.get(&reference)?.model.value.as_dictionary()
}

fn catalog_names<'a>(
    catalog: &PdfCatalog,
    latest: &'a Latest<'_>,
) -> Option<&'a BTreeMap<String, PdfValue>> {
    dictionary(latest, catalog.object)
        .and_then(|value| value.get("Names"))
        .and_then(|value| resolve_dictionary(value, latest))
}

fn resolve_dictionary<'a>(
    value: &'a PdfValue,
    latest: &'a Latest<'_>,
) -> Option<&'a BTreeMap<String, PdfValue>> {
    match value {
        PdfValue::Dictionary(value) => Some(value),
        PdfValue::Reference(reference) => dictionary(latest, *reference),
        _ => None,
    }
}

fn object_locator(latest: &Latest<'_>, reference: PdfReference) -> PdfObjectLocator {
    latest
        .get(&reference)
        .map(|object| object.model.locator.clone())
        .unwrap_or_else(|| PdfObjectLocator::direct(reference, 0, 0))
}

fn reference_value(dictionary: &BTreeMap<String, PdfValue>, key: &str) -> Option<PdfReference> {
    dictionary.get(key).and_then(PdfValue::as_reference)
}

fn references(value: Option<&PdfValue>) -> Vec<PdfReference> {
    match value {
        Some(PdfValue::Reference(reference)) => vec![*reference],
        Some(PdfValue::Array(values)) => values.iter().filter_map(PdfValue::as_reference).collect(),
        _ => Vec::new(),
    }
}

fn text_value(value: Option<&PdfValue>) -> Option<String> {
    match value {
        Some(PdfValue::String(value)) => Some(value.text.clone()),
        _ => None,
    }
}

fn name_value(value: Option<&PdfValue>) -> Option<String> {
    value.and_then(PdfValue::as_name).map(str::to_string)
}

fn names(value: Option<&PdfValue>) -> Vec<String> {
    match value {
        Some(PdfValue::Name(value)) => vec![value.clone()],
        Some(PdfValue::Array(values)) => values
            .iter()
            .filter_map(PdfValue::as_name)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn boolean_value(value: Option<&PdfValue>) -> Option<bool> {
    match value {
        Some(PdfValue::Boolean(value)) => Some(*value),
        _ => None,
    }
}

fn number(value: &PdfValue) -> Option<f64> {
    match value {
        PdfValue::Integer(value) => Some(*value as f64),
        PdfValue::Real(value) => Some(*value),
        _ => None,
    }
}

fn integers(value: &PdfValue) -> Vec<i64> {
    match value {
        PdfValue::Array(values) => values.iter().filter_map(PdfValue::as_integer).collect(),
        _ => Vec::new(),
    }
}

fn rectangle(value: Option<&PdfValue>) -> Option<PdfRectangle> {
    let PdfValue::Array(values) = value? else {
        return None;
    };
    if values.len() != 4 {
        return None;
    }
    Some(PdfRectangle {
        left: number(&values[0])?,
        bottom: number(&values[1])?,
        right: number(&values[2])?,
        top: number(&values[3])?,
    })
}

fn page_locator(page: &PdfPage, rectangle: Option<PdfRectangle>) -> SourceLocator {
    SourceLocator::exact(LocationComponent::PdfRegion {
        page: IndexPosition::new(page.index, IndexBase::One).expect("valid PDF page"),
        bbox: rectangle.map(|value| BoundingBox {
            x: value.left,
            y: value.bottom,
            width: value.width(),
            height: value.height(),
            unit: CoordinateUnit::Points,
            origin: CoordinateOrigin::BottomLeft,
        }),
        rotation_degrees: Some(page.rotation_degrees),
        tokens: None,
    })
    .expect("valid annotation locator")
}

fn object_id(kind: &str, reference: PdfReference) -> String {
    format!(
        "pdf:{kind}:{}:{}",
        reference.object_number, reference.generation
    )
}
