use super::decode::transfer_decode;
use super::{
    EmailDocument, MimePart, NestedEmailParse, OpaqueEmailContent, SmimeDecryption, SmimeKind,
    SmimePart, TnefAttribute, TnefDocument,
};
use crate::core::{
    DeclaredLoss, Diagnostic, LocationComponent, OperationControl, OperationKind, OperationStatus,
    ProvenanceStep, ProviderInvocation, ProviderKind, RawContentIdentity, SourceInfo,
    SourceLocator,
};
use crate::provider::{
    DecryptionOptions, DecryptionRequest, ProviderRequest, ProviderRequestContext, ProviderResult,
};
use crate::registry::ParserContext;
use serde_json::json;

const TNEF_SIGNATURE: u32 = 0x223e_9f78;
const PARSER: &str = "grist.email";

pub(super) struct SecureProviderEvidence {
    pub providers: Vec<ProviderInvocation>,
    pub provenance: Vec<ProvenanceStep>,
}

pub(super) fn apply_smime_decryption(
    context: &mut ParserContext<'_>,
    document: &mut EmailDocument,
) -> SecureProviderEvidence {
    let mut evidence = SecureProviderEvidence {
        providers: Vec::new(),
        provenance: Vec::new(),
    };
    let Some(network_access) = context.selected_provider_network(ProviderKind::Decryption) else {
        return evidence;
    };
    let mut pending = Vec::new();
    collect_encrypted_parts(&document.mime, context.bytes(), &mut pending);
    for pending_part in pending {
        let configuration = json!({
            "adapter": "grist.email.smime",
            "version": 1,
            "mime_path": pending_part.path,
            "scheme": "smime-cms-enveloped-data",
        });
        let request_context = match ProviderRequestContext::new(
            &pending_part.ciphertext,
            network_access,
            &configuration,
        ) {
            Ok(value) => value,
            Err(error) => {
                document.diagnostics.push(
                    Diagnostic::provider_failure(PARSER, error.to_string())
                        .with_locator(pending_part.locator)
                        .partial(),
                );
                continue;
            }
        };
        let request = match DecryptionRequest::with_provider_credentials(
            request_context,
            DecryptionOptions {
                scheme: "smime-cms-enveloped-data".into(),
                key_id: None,
                expected_media_type: Some("message/rfc822".into()),
            },
        ) {
            Ok(value) => ProviderRequest::Decryption(value),
            Err(error) => {
                document.diagnostics.push(
                    Diagnostic::provider_failure(PARSER, error.to_string())
                        .with_locator(pending_part.locator)
                        .partial(),
                );
                continue;
            }
        };
        let response = match context.run_provider(&request) {
            Ok(value) => value,
            Err(diagnostic) => {
                document.diagnostics.push(
                    (*diagnostic)
                        .with_locator(pending_part.locator.clone())
                        .partial(),
                );
                continue;
            }
        };
        evidence.providers.push(response.envelope_invocation());
        let provider_name = response.metadata.provider.name.clone();
        let request_digest = response.metadata.request_digest.clone();
        let output_identity = response.metadata.output_identity.clone();
        let mut attempt_diagnostics = response
            .metadata
            .diagnostics
            .iter()
            .cloned()
            .map(|diagnostic| {
                diagnostic
                    .with_locator(pending_part.locator.clone())
                    .partial()
            })
            .collect::<Vec<_>>();
        let (status, content_identity, media_type, parsed) = if let Some(
            ProviderResult::Decryption(result),
        ) = response.result()
        {
            let content_identity = result.identity.clone();
            let media_type = result.media_type.clone();
            if let Ok(step) = ProvenanceStep::new(
                OperationKind::Parse,
                "grist.email.smime-decryption",
                response.metadata.input_identity.sha256.clone(),
                content_identity.sha256.clone(),
                response.metadata.configuration_digest.clone(),
                DeclaredLoss::Lossless,
            ) {
                evidence
                    .provenance
                    .push(step.with_provider(provider_name.clone()));
            }
            let resident_bytes = context.bytes().len().saturating_add(result.bytes.len()) as u64;
            if let Err(diagnostic) = context.observe_memory_bytes(resident_bytes) {
                attempt_diagnostics.push(
                    (*diagnostic)
                        .with_locator(pending_part.locator.clone())
                        .partial(),
                );
                (
                    OperationStatus::Partial,
                    Some(content_identity),
                    media_type,
                    None,
                )
            } else {
                let nested_source = SourceInfo::new(format!(
                    "decrypted-smime-{}.eml",
                    display_path(&pending_part.path)
                ))
                .with_declared_mime_type(
                    media_type
                        .clone()
                        .unwrap_or_else(|| "message/rfc822".into()),
                )
                .with_parent(context.source().clone());
                let nested = super::parse_email_with_operation_control(
                    &result.bytes,
                    nested_source,
                    &super::EmailOptions::default(),
                    context.control(),
                );
                let nested_status = nested.status;
                let parsed = serde_json::to_value(&nested)
                    .ok()
                    .map(|envelope| NestedEmailParse {
                        format: "eml".into(),
                        status: nested_status,
                        envelope,
                    });
                if nested_status != OperationStatus::Complete {
                    attempt_diagnostics.push(
                        partial(
                            "email.smime.decrypted_parse_incomplete",
                            format!(
                                "decryption succeeded but the decrypted MIME parse returned {nested_status:?}"
                            ),
                            pending_part.locator.clone(),
                        ),
                    );
                }
                (nested_status, Some(content_identity), media_type, parsed)
            }
        } else {
            (OperationStatus::Failed, None, None, None)
        };
        if status == OperationStatus::Complete {
            document.diagnostics.retain(|diagnostic| {
                diagnostic.code != "email.mime.encrypted"
                    || diagnostic.locator.as_deref() != Some(&pending_part.locator)
            });
        }
        document.diagnostics.extend(attempt_diagnostics.clone());
        if let Some(part) = find_part_mut(&mut document.mime, &pending_part.path)
            && let Some(smime) = &mut part.smime
        {
            smime.decryption = Some(SmimeDecryption {
                status,
                provider: provider_name,
                request_digest,
                output_identity,
                content_identity,
                media_type,
                parsed,
                diagnostics: attempt_diagnostics,
            });
        }
    }
    document.complete = document
        .diagnostics
        .iter()
        .all(|diagnostic| !diagnostic.partial);
    evidence
}

struct PendingDecryption {
    path: Vec<usize>,
    ciphertext: Vec<u8>,
    locator: SourceLocator,
}

fn collect_encrypted_parts(part: &MimePart, source: &[u8], output: &mut Vec<PendingDecryption>) {
    if part.encrypted
        && matches!(
            part.content_type.essence.as_str(),
            "application/pkcs7-mime" | "application/x-pkcs7-mime"
        )
        && let Some((start, end)) = byte_range(&part.body_locator)
        && let Some(encoded) = source.get(start..end)
    {
        let decoded = transfer_decode(encoded, &part.transfer_encoding);
        output.push(PendingDecryption {
            path: part.path.clone(),
            ciphertext: decoded.bytes,
            locator: part.locator.clone(),
        });
    }
    for child in &part.children {
        collect_encrypted_parts(child, source, output);
    }
}

fn find_part_mut<'a>(part: &'a mut MimePart, path: &[usize]) -> Option<&'a mut MimePart> {
    if part.path == path {
        return Some(part);
    }
    part.children
        .iter_mut()
        .find_map(|child| find_part_mut(child, path))
}

fn byte_range(locator: &SourceLocator) -> Option<(usize, usize)> {
    locator.components().iter().rev().find_map(|component| {
        component
            .as_text_range()
            .map(|range| (range.byte_start, range.byte_end))
    })
}

fn display_path(path: &[usize]) -> String {
    if path.is_empty() {
        "root".into()
    } else {
        path.iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }
}

pub(super) fn is_tnef_media_type(essence: &str) -> bool {
    matches!(
        essence,
        "application/ms-tnef" | "application/vnd.ms-tnef" | "application/x-tnef"
    )
}

pub(super) fn parse_tnef(
    bytes: &[u8],
    locator: SourceLocator,
    control: &OperationControl,
) -> (TnefDocument, Vec<Diagnostic>) {
    let signature = read_u32(bytes, 0).unwrap_or_default();
    let mut document = TnefDocument {
        signature,
        signature_valid: signature == TNEF_SIGNATURE,
        key: read_u16(bytes, 4),
        identity: RawContentIdentity::new(bytes),
        attributes: Vec::new(),
        trailing_bytes: 0,
        complete: true,
        locator: locator.clone(),
    };
    let mut diagnostics = Vec::new();
    if bytes.len() < 6 {
        document.complete = false;
        document.trailing_bytes = bytes.len();
        diagnostics.push(partial(
            "email.tnef.header_truncated",
            "TNEF content is shorter than its six-byte header",
            locator,
        ));
        return (document, diagnostics);
    }
    if !document.signature_valid {
        document.complete = false;
        diagnostics.push(partial(
            "email.tnef.signature_invalid",
            "TNEF content does not begin with the little-endian TNEF signature",
            locator.clone(),
        ));
    }

    let mut cursor = 6usize;
    while cursor < bytes.len() {
        if bytes.len().saturating_sub(cursor) < 11 {
            document.complete = false;
            document.trailing_bytes = bytes.len() - cursor;
            diagnostics.push(partial(
                "email.tnef.attribute_truncated",
                "TNEF attribute header or checksum is truncated",
                relative_locator(&locator, cursor, bytes.len()),
            ));
            break;
        }
        if let Err(error) = control.budget().consume_nodes(1) {
            document.complete = false;
            document.trailing_bytes = bytes.len() - cursor;
            diagnostics.push(error.diagnostic(PARSER).with_locator(relative_locator(
                &locator,
                cursor,
                bytes.len(),
            )));
            break;
        }
        let start = cursor;
        let level = bytes[cursor];
        let name = read_u16(bytes, cursor + 1).expect("bounded TNEF header");
        let attribute_type = read_u16(bytes, cursor + 3).expect("bounded TNEF header");
        let length = read_u32(bytes, cursor + 5).expect("bounded TNEF header") as usize;
        cursor += 9;
        let Some(value_end) = cursor.checked_add(length) else {
            document.complete = false;
            document.trailing_bytes = bytes.len() - start;
            diagnostics.push(partial(
                "email.tnef.attribute_length_overflow",
                "TNEF attribute length overflows the addressable input",
                relative_locator(&locator, start, bytes.len()),
            ));
            break;
        };
        if value_end.saturating_add(2) > bytes.len() {
            document.complete = false;
            document.trailing_bytes = bytes.len() - start;
            diagnostics.push(partial(
                "email.tnef.attribute_truncated",
                "TNEF attribute value exceeds the available bytes",
                relative_locator(&locator, start, bytes.len()),
            ));
            break;
        }
        let value = &bytes[cursor..value_end];
        let checksum = read_u16(bytes, value_end).expect("bounded TNEF checksum");
        let computed = value
            .iter()
            .fold(0u16, |sum, byte| sum.wrapping_add(u16::from(*byte)));
        let checksum_valid = checksum == computed;
        let ordinal = document.attributes.len();
        document.attributes.push(TnefAttribute {
            ordinal,
            level,
            name,
            known_name: tnef_attribute_name(name).map(str::to_string),
            attribute_type,
            value: RawContentIdentity::new(value),
            checksum,
            checksum_valid,
            locator: relative_locator(&locator, start, value_end + 2),
        });
        if !checksum_valid && name != 0x8008 {
            document.complete = false;
            diagnostics.push(partial(
                "email.tnef.checksum_mismatch",
                format!("TNEF attribute {ordinal} checksum does not match its value bytes"),
                relative_locator(&locator, start, value_end + 2),
            ));
        }
        cursor = value_end + 2;
    }
    (document, diagnostics)
}

pub(super) fn represent_smime(part: &mut MimePart, encoded_body: &[u8]) {
    let essence = part.content_type.essence.as_str();
    let kind = if essence == "multipart/signed" {
        Some(SmimeKind::MultipartSigned)
    } else if essence == "multipart/encrypted" {
        Some(SmimeKind::MultipartEncrypted)
    } else if matches!(
        essence,
        "application/pkcs7-signature" | "application/x-pkcs7-signature"
    ) {
        Some(SmimeKind::DetachedSignature)
    } else if matches!(
        essence,
        "application/pkcs7-mime" | "application/x-pkcs7-mime"
    ) {
        if part
            .content_type
            .parameter("smime-type")
            .is_some_and(|value| value.eq_ignore_ascii_case("signed-data"))
        {
            Some(SmimeKind::SignedData)
        } else {
            Some(SmimeKind::EnvelopedData)
        }
    } else {
        None
    };
    let Some(kind) = kind else {
        return;
    };
    let (signed_content_path, signature_path) = if kind == SmimeKind::MultipartSigned {
        (
            part.children.first().map(|child| child.path.clone()),
            part.children.get(1).map(|child| child.path.clone()),
        )
    } else {
        (None, None)
    };
    part.smime = Some(SmimePart {
        kind,
        smime_type: part
            .content_type
            .parameter("smime-type")
            .map(str::to_string),
        protocol: part.content_type.parameter("protocol").map(str::to_string),
        micalg: part.content_type.parameter("micalg").map(str::to_string),
        native: OpaqueEmailContent {
            identity: RawContentIdentity::new(encoded_body),
            locator: part.body_locator.clone(),
        },
        signed_content_path,
        signature_path,
        decryption: None,
    });
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn relative_locator(parent: &SourceLocator, start: usize, end: usize) -> SourceLocator {
    parent
        .clone()
        .nested(LocationComponent::ByteRange {
            byte_start: start,
            byte_end: end,
        })
        .expect("bounded TNEF byte range is a valid nested locator")
}

fn partial(code: &'static str, message: impl Into<String>, locator: SourceLocator) -> Diagnostic {
    Diagnostic::warning(PARSER, code, message)
        .with_locator(locator)
        .partial()
}

fn tnef_attribute_name(name: u16) -> Option<&'static str> {
    match name {
        0x8000 => Some("from"),
        0x8004 => Some("subject"),
        0x8005 => Some("date-sent"),
        0x8006 => Some("date-received"),
        0x8007 => Some("message-status"),
        0x8008 => Some("message-class"),
        0x8009 => Some("message-id"),
        0x800a => Some("parent-id"),
        0x800b => Some("conversation-id"),
        0x800c => Some("body"),
        0x800d => Some("priority"),
        0x800f => Some("attachment-data"),
        0x8010 => Some("attachment-title"),
        0x8011 => Some("attachment-metafile"),
        0x8012 => Some("attachment-create-date"),
        0x8013 => Some("attachment-modify-date"),
        0x8020 => Some("date-modified"),
        0x9001 => Some("attachment-transport-filename"),
        0x9002 => Some("attachment-render-data"),
        0x9003 => Some("mapi-properties"),
        0x9004 => Some("recipient-table"),
        0x9005 => Some("attachment"),
        0x9006 => Some("tnef-version"),
        0x9007 => Some("oem-codepage"),
        _ => None,
    }
}
