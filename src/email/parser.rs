use super::decode::{decode_charset, decode_rfc2047, parse_mime_value, transfer_decode};
use super::*;
use crate::container::{
    ArtifactDisposition, ArtifactMetadata, ArtifactParent, ArtifactRelationship, EmbeddedArtifact,
};
use crate::core::{
    AutoFormatOptions, BudgetProfile, BudgetSelection, CompoundMemberInput, ContentIdentity,
    Diagnostic, FormatHint, FormatIdentity, IndexPosition, Input, LocationComponent,
    OperationControl, ParseOptions, ProviderSet, RequestId, ResolvedParseRequest, SchemaVersion,
    SourceInfo, SourceLocator, SourceRange, sha256_hex,
};
use crate::registry::{ParserSelection, builtin_parser_registry};

const PARSER: &str = "grist.email";

pub(super) fn parse_document(
    bytes: &[u8],
    source: &SourceInfo,
    options: &EmailOptions,
    control: &OperationControl,
) -> Result<EmailDocument, Box<Diagnostic>> {
    validate_options(options)?;
    let parent_identity = ContentIdentity::for_raw_bytes(bytes)
        .with_format(FormatIdentity::new("eml", Some("message/rfc822")));
    let mut state = ParseState {
        bytes,
        source,
        options,
        control,
        parent_identity,
        diagnostics: Vec::new(),
        external_references: Vec::new(),
        encrypted_parts: Vec::new(),
        signed_parts: Vec::new(),
        part_count: 0,
    };
    let (header_end, body_start, separator_found) = split_header_body(bytes, 0, bytes.len());
    if !separator_found {
        state.partial(
            "email.header.separator_missing",
            "message has no RFC 5322 header/body separator; the available header block was retained",
            Some(range_locator(bytes, 0, bytes.len(), &[], None, Some("headers"))),
        );
    }
    let structured_header_end = header_end.min(options.max_header_bytes);
    if header_end > options.max_header_bytes {
        state.partial(
            "email.limit.header_bytes",
            format!(
                "header block is {header_end} bytes and exceeds max_header_bytes {}",
                options.max_header_bytes
            ),
            Some(range_locator(
                bytes,
                0,
                structured_header_end,
                &[],
                None,
                Some("headers"),
            )),
        );
    }
    let headers = parse_headers(bytes, 0, structured_header_end, &[], &mut state.diagnostics);
    let message_id =
        first_header(&headers, "message-id").map(|header| header.decoded_value.clone());
    let mime = state.parse_part(
        0,
        bytes.len(),
        body_start.min(bytes.len()),
        headers.clone(),
        Vec::new(),
        0,
        message_id.as_deref(),
    );
    let address_fields = address_fields(&headers);
    let date_fields = date_fields(&headers);
    let subject = first_header(&headers, "subject").map(text_fact);
    let thread = thread_evidence(&headers, subject.as_ref());
    let authentication = authentication_evidence(&headers);
    let complete = state
        .diagnostics
        .iter()
        .all(|diagnostic| !diagnostic.partial);
    Ok(EmailDocument {
        schema_version: SchemaVersion::EMAIL_V1.into(),
        headers,
        address_fields,
        date_fields,
        subject,
        thread,
        authentication,
        mime,
        external_references: state.external_references,
        encrypted_parts: state.encrypted_parts,
        signed_parts: state.signed_parts,
        diagnostics: state.diagnostics,
        complete,
    })
}

fn validate_options(options: &EmailOptions) -> Result<(), Box<Diagnostic>> {
    if options.max_header_bytes == 0
        || options.max_mime_depth == 0
        || options.max_mime_parts == 0
        || options.max_decoded_part_bytes == 0
    {
        return Err(Box::new(Diagnostic::malformed(
            PARSER,
            "email options limits must all be greater than zero",
        )));
    }
    Ok(())
}

struct ParseState<'a> {
    bytes: &'a [u8],
    source: &'a SourceInfo,
    options: &'a EmailOptions,
    control: &'a OperationControl,
    parent_identity: ContentIdentity,
    diagnostics: Vec<Diagnostic>,
    external_references: Vec<EmailExternalReference>,
    encrypted_parts: Vec<Vec<usize>>,
    signed_parts: Vec<Vec<usize>>,
    part_count: usize,
}

impl ParseState<'_> {
    #[allow(clippy::too_many_arguments)]
    fn parse_part(
        &mut self,
        entity_start: usize,
        entity_end: usize,
        body_start: usize,
        headers: Vec<EmailHeader>,
        path: Vec<usize>,
        depth: usize,
        message_id: Option<&str>,
    ) -> MimePart {
        self.part_count = self.part_count.saturating_add(1);
        if self.part_count > self.options.max_mime_parts || depth > self.options.max_mime_depth {
            self.partial(
                "email.limit.mime_structure",
                "MIME entity count or nesting depth exceeded the explicit email options",
                Some(range_locator(
                    self.bytes,
                    entity_start,
                    entity_end,
                    &path,
                    message_id,
                    Some("entity"),
                )),
            );
            return self.opaque_part(
                entity_start,
                entity_end,
                body_start,
                headers,
                path,
                message_id,
            );
        }
        if let Err(error) = self.control.budget().observe_nesting_depth(depth as u64) {
            self.diagnostics.push(error.diagnostic(PARSER).partial());
            return self.opaque_part(
                entity_start,
                entity_end,
                body_start,
                headers,
                path,
                message_id,
            );
        }
        if let Err(error) = self.control.budget().consume_nodes(1) {
            self.diagnostics.push(error.diagnostic(PARSER).partial());
            return self.opaque_part(
                entity_start,
                entity_end,
                body_start,
                headers,
                path,
                message_id,
            );
        }
        let content_type = first_header(&headers, "content-type")
            .map(|header| parse_mime_value(&header.decoded_value, "text/plain"))
            .unwrap_or_else(|| parse_mime_value("text/plain; charset=us-ascii", "text/plain"));
        let content_disposition = first_header(&headers, "content-disposition")
            .map(|header| parse_mime_value(&header.decoded_value, ""));
        let transfer_encoding = first_header(&headers, "content-transfer-encoding")
            .map(|header| header.decoded_value.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "7bit".into());
        let content_id = header_text(&headers, "content-id");
        let content_location = header_text(&headers, "content-location");
        let locator = range_locator(
            self.bytes,
            entity_start,
            entity_end,
            &path,
            message_id,
            Some("entity"),
        );
        let body_locator = range_locator(
            self.bytes,
            body_start,
            entity_end,
            &path,
            message_id,
            Some("body"),
        );
        let body = &self.bytes[body_start.min(entity_end)..entity_end];
        let mut part = MimePart {
            path: path.clone(),
            headers,
            content_type,
            content_disposition,
            transfer_encoding,
            content_id,
            content_location,
            encoded_body_sha256: sha256_hex(body),
            encoded_body_bytes: body.len(),
            decoded_body_sha256: None,
            decoded_body_bytes: None,
            text: None,
            preamble: None,
            epilogue: None,
            children: Vec::new(),
            attachment: None,
            encrypted: false,
            signed: false,
            locator,
            body_locator,
        };
        let essence = part.content_type.essence.as_str();
        part.encrypted = essence == "multipart/encrypted"
            || matches!(
                essence,
                "application/pkcs7-mime" | "application/x-pkcs7-mime"
            ) && part
                .content_type
                .parameter("smime-type")
                .is_none_or(|value| value.eq_ignore_ascii_case("enveloped-data"));
        part.signed = essence == "multipart/signed"
            || matches!(
                essence,
                "application/pkcs7-signature" | "application/x-pkcs7-signature"
            );
        if part.encrypted {
            self.encrypted_parts.push(path.clone());
            self.partial(
                "email.mime.encrypted",
                "encrypted MIME content was inventoried but not decrypted",
                Some(part.locator.clone()),
            );
        }
        if part.signed {
            self.signed_parts.push(path.clone());
        }
        if essence.starts_with("multipart/") {
            self.parse_multipart(&mut part, body_start, entity_end, depth, message_id);
        } else {
            self.parse_leaf(&mut part, body, depth);
        }
        if let Some(location) = &part.content_location
            && is_remote_uri(location)
        {
            self.external_references.push(EmailExternalReference {
                uri: location.clone(),
                source: "content-location".into(),
                mime_path: path,
                resolved: false,
                locator: part.body_locator.clone(),
            });
        }
        part
    }

    fn parse_multipart(
        &mut self,
        part: &mut MimePart,
        body_start: usize,
        body_end: usize,
        depth: usize,
        message_id: Option<&str>,
    ) {
        let Some(boundary) = part.content_type.parameter("boundary").map(str::to_string) else {
            self.partial(
                "email.mime.boundary_missing",
                "multipart entity has no boundary parameter",
                Some(part.locator.clone()),
            );
            return;
        };
        let delimiters = boundary_delimiters(self.bytes, body_start, body_end, boundary.as_bytes());
        if delimiters.is_empty() {
            self.partial(
                "email.mime.boundary_not_found",
                "declared multipart boundary does not occur in the body",
                Some(part.body_locator.clone()),
            );
            return;
        }
        part.preamble =
            Some(String::from_utf8_lossy(&self.bytes[body_start..delimiters[0].0]).into_owned());
        let mut child_index = 0;
        let mut closed = false;
        for window in delimiters.windows(2) {
            let current = window[0];
            let next = window[1];
            if current.2 {
                closed = true;
                break;
            }
            child_index += 1;
            let child_start = current.1;
            let child_end = trim_framing_newline(self.bytes, child_start, next.0);
            let (header_end, child_body, separator) =
                split_header_body(self.bytes, child_start, child_end);
            let mut child_path = part.path.clone();
            child_path.push(child_index);
            if !separator {
                self.partial(
                    "email.mime.part_header_separator_missing",
                    "MIME child has no header/body separator",
                    Some(range_locator(
                        self.bytes,
                        child_start,
                        child_end,
                        &child_path,
                        message_id,
                        Some("entity"),
                    )),
                );
            }
            let child_headers = parse_headers(
                self.bytes,
                child_start,
                header_end.min(child_start.saturating_add(self.options.max_header_bytes)),
                &child_path,
                &mut self.diagnostics,
            );
            part.children.push(self.parse_part(
                child_start,
                child_end,
                child_body.min(child_end),
                child_headers,
                child_path,
                depth + 1,
                message_id,
            ));
            if next.2 {
                closed = true;
                part.epilogue =
                    Some(String::from_utf8_lossy(&self.bytes[next.1..body_end]).into_owned());
                break;
            }
        }
        if !closed {
            self.partial(
                "email.mime.closing_boundary_missing",
                "multipart entity has no closing boundary",
                Some(part.body_locator.clone()),
            );
            if let Some(last) = delimiters.last().copied()
                && !last.2
            {
                child_index += 1;
                let child_start = last.1;
                let (header_end, child_body, _) =
                    split_header_body(self.bytes, child_start, body_end);
                let mut child_path = part.path.clone();
                child_path.push(child_index);
                let child_headers = parse_headers(
                    self.bytes,
                    child_start,
                    header_end,
                    &child_path,
                    &mut self.diagnostics,
                );
                part.children.push(self.parse_part(
                    child_start,
                    body_end,
                    child_body,
                    child_headers,
                    child_path,
                    depth + 1,
                    message_id,
                ));
            }
        }
    }

    fn parse_leaf(&mut self, part: &mut MimePart, encoded_body: &[u8], depth: usize) {
        if encoded_body.len() > self.options.max_decoded_part_bytes.saturating_mul(2) {
            self.partial(
                "email.limit.decoded_part_bytes",
                format!(
                    "encoded MIME body is too large for max_decoded_part_bytes {}",
                    self.options.max_decoded_part_bytes
                ),
                Some(part.body_locator.clone()),
            );
            return;
        }
        let decoded = transfer_decode(encoded_body, &part.transfer_encoding);
        if decoded.malformed {
            self.partial(
                "email.mime.transfer_encoding_malformed",
                format!(
                    "MIME body uses malformed or unsupported content-transfer-encoding {}",
                    part.transfer_encoding
                ),
                Some(part.body_locator.clone()),
            );
        }
        if decoded.bytes.len() > self.options.max_decoded_part_bytes {
            self.partial(
                "email.limit.decoded_part_bytes",
                format!(
                    "decoded MIME body is {} bytes and exceeds max_decoded_part_bytes {}",
                    decoded.bytes.len(),
                    self.options.max_decoded_part_bytes
                ),
                Some(part.body_locator.clone()),
            );
            return;
        }
        part.decoded_body_sha256 = Some(sha256_hex(&decoded.bytes));
        part.decoded_body_bytes = Some(decoded.bytes.len());
        if part.content_type.essence.starts_with("text/") {
            let charset = part.content_type.parameter("charset").unwrap_or("us-ascii");
            let (text, used_charset, lossy) = decode_charset(&decoded.bytes, charset);
            if lossy {
                self.partial(
                    "email.mime.charset_lossy",
                    format!("MIME text declared unsupported or lossy charset {charset}"),
                    Some(part.body_locator.clone()),
                );
            }
            if let Err(error) = self
                .control
                .budget()
                .consume_decoded_characters(text.chars().count() as u64)
            {
                self.diagnostics.push(error.diagnostic(PARSER).partial());
            } else {
                let format_flowed = part
                    .content_type
                    .parameter("format")
                    .is_some_and(|value| value.eq_ignore_ascii_case("flowed"));
                let delsp = part
                    .content_type
                    .parameter("delsp")
                    .is_some_and(|value| value.eq_ignore_ascii_case("yes"));
                if part.content_type.essence == "text/html" {
                    for uri in html_remote_references(&text) {
                        self.external_references.push(EmailExternalReference {
                            uri,
                            source: "html-attribute".into(),
                            mime_path: part.path.clone(),
                            resolved: false,
                            locator: part.body_locator.clone(),
                        });
                    }
                }
                part.text = Some(MimeTextBody {
                    text,
                    charset: used_charset,
                    lossy,
                    format_flowed,
                    delsp,
                    locator: part.body_locator.clone(),
                });
            }
        }
        if should_capture_attachment(part) {
            match self.capture_attachment(part, &decoded.bytes, depth) {
                Ok(attachment) => part.attachment = Some(attachment),
                Err(diagnostic) => self.diagnostics.push((*diagnostic).partial()),
            }
        }
    }

    fn capture_attachment(
        &mut self,
        part: &MimePart,
        decoded: &[u8],
        depth: usize,
    ) -> Result<EmailAttachment, Box<Diagnostic>> {
        self.control
            .budget()
            .consume_child_artifacts(1)
            .map_err(|error| Box::new(error.diagnostic(PARSER)))?;
        let filename = part
            .content_disposition
            .as_ref()
            .and_then(|value| value.parameter("filename"))
            .or_else(|| part.content_type.parameter("name"))
            .map(str::to_string);
        let disposition_name = part
            .content_disposition
            .as_ref()
            .map(|value| value.essence.clone())
            .unwrap_or_else(|| "unspecified".into());
        let inline_resource = disposition_name == "inline" || part.content_id.is_some();
        let (relationship, disposition) = if inline_resource {
            (
                ArtifactRelationship::InlineResourceOf,
                ArtifactDisposition::Inline,
            )
        } else {
            (
                ArtifactRelationship::AttachmentOf,
                ArtifactDisposition::Attachment,
            )
        };
        let mut metadata = ArtifactMetadata::new(
            ArtifactParent::new(self.parent_identity.clone(), relationship),
            part.locator.clone(),
            disposition,
        )
        .with_media_type(part.content_type.essence.clone());
        if let Some(filename) = &filename {
            metadata = metadata.with_declared_filename(filename.clone());
        }
        let artifact = if self.options.inline_attachment_bytes {
            EmbeddedArtifact::capture_inline(metadata, decoded)
        } else {
            EmbeddedArtifact::inventory(metadata, decoded)
        }
        .map_err(|error| {
            Box::new(Diagnostic::parser_defect(
                PARSER,
                format!("embedded-artifact invariant failed: {error}"),
            ))
        })?;
        let nested = if self.options.parse_nested_attachments && !part.encrypted {
            self.parse_nested(part, decoded, filename.as_deref(), depth + 1)
        } else {
            None
        };
        Ok(EmailAttachment {
            filename,
            disposition: disposition_name,
            inline_resource,
            artifact,
            nested,
        })
    }

    fn parse_nested(
        &mut self,
        part: &MimePart,
        bytes: &[u8],
        filename: Option<&str>,
        depth: usize,
    ) -> Option<NestedEmailParse> {
        let registry = builtin_parser_registry().ok()?;
        let format = nested_format(&registry, &part.content_type.essence, filename)?;
        let descriptor = match registry.select_format(&format) {
            ParserSelection::Available(descriptor) => descriptor,
            ParserSelection::Unsupported { .. } => return None,
        };
        if self
            .control
            .budget()
            .observe_nesting_depth(depth as u64)
            .is_err()
        {
            self.partial(
                "email.nested.budget_limited",
                "nested attachment exceeded the shared nesting budget",
                Some(part.locator.clone()),
            );
            return None;
        }
        let member_name = filename
            .map(str::to_string)
            .unwrap_or_else(|| format!("mime-{}", display_path(&part.path)));
        let child_source = SourceInfo::new(member_name.clone())
            .with_declared_mime_type(part.content_type.essence.clone())
            .with_parent(self.source.clone());
        let input = Input::compound_member(CompoundMemberInput::new(
            self.source.clone(),
            member_name,
            None,
            Input::bytes(bytes.to_vec()),
        ))
        .resolve_with_control(self.control)
        .ok()?;
        let request = ResolvedParseRequest {
            request_id: RequestId::new(format!("email-nested-{}", display_path(&part.path)))
                .ok()?,
            input,
            source: child_source,
            format_hint: Some(FormatHint::exact(format.clone())),
            options: ParseOptions::default(),
            format_options: AutoFormatOptions,
            budget: BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
            control: self.control.clone(),
            providers: ProviderSet::default(),
        };
        match registry.dispatch_selected(&descriptor.id, request, None) {
            Ok(envelope) => {
                let status = envelope.status;
                if status != crate::core::OperationStatus::Complete {
                    self.partial(
                        "email.nested.incomplete",
                        format!("nested {format} attachment returned {status:?}"),
                        Some(part.locator.clone()),
                    );
                }
                Some(NestedEmailParse {
                    format,
                    status,
                    envelope: serde_json::to_value(envelope).ok()?,
                })
            }
            Err(error) => {
                self.partial(
                    "email.nested.dispatch_failed",
                    format!("nested {format} attachment could not be dispatched: {error}"),
                    Some(part.locator.clone()),
                );
                None
            }
        }
    }

    fn opaque_part(
        &self,
        entity_start: usize,
        entity_end: usize,
        body_start: usize,
        headers: Vec<EmailHeader>,
        path: Vec<usize>,
        message_id: Option<&str>,
    ) -> MimePart {
        let body = &self.bytes[body_start.min(entity_end)..entity_end];
        MimePart {
            path: path.clone(),
            headers,
            content_type: parse_mime_value("application/octet-stream", "application/octet-stream"),
            content_disposition: None,
            transfer_encoding: "unknown".into(),
            content_id: None,
            content_location: None,
            encoded_body_sha256: sha256_hex(body),
            encoded_body_bytes: body.len(),
            decoded_body_sha256: None,
            decoded_body_bytes: None,
            text: None,
            preamble: None,
            epilogue: None,
            children: Vec::new(),
            attachment: None,
            encrypted: false,
            signed: false,
            locator: range_locator(
                self.bytes,
                entity_start,
                entity_end,
                &path,
                message_id,
                Some("entity"),
            ),
            body_locator: range_locator(
                self.bytes,
                body_start,
                entity_end,
                &path,
                message_id,
                Some("body"),
            ),
        }
    }

    fn partial(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        locator: Option<SourceLocator>,
    ) {
        let mut diagnostic = Diagnostic::warning(PARSER, code, message).partial();
        if let Some(locator) = locator {
            diagnostic = diagnostic.with_locator(locator);
        }
        self.diagnostics.push(diagnostic);
    }
}

fn parse_headers(
    bytes: &[u8],
    start: usize,
    end: usize,
    path: &[usize],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<EmailHeader> {
    let mut logical: Vec<(usize, usize)> = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let next = next_line(bytes, cursor, end);
        let continuation = bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'));
        if continuation && !logical.is_empty() {
            logical.last_mut().expect("not empty").1 = next;
        } else {
            logical.push((cursor, next));
        }
        cursor = next;
    }
    logical
        .into_iter()
        .enumerate()
        .map(|(ordinal, (header_start, header_end))| {
            let content_end = trim_trailing_line_endings(bytes, header_start, header_end);
            let raw_bytes = &bytes[header_start..content_end];
            let first_end = raw_bytes
                .iter()
                .position(|byte| matches!(byte, b'\r' | b'\n'))
                .unwrap_or(raw_bytes.len());
            let colon = raw_bytes[..first_end].iter().position(|byte| *byte == b':');
            let (name, raw_value, valid) = match colon {
                Some(colon) if colon > 0 => (
                    Some(String::from_utf8_lossy(&raw_bytes[..colon]).into_owned()),
                    String::from_utf8_lossy(&raw_bytes[colon + 1..]).into_owned(),
                    raw_bytes[..colon]
                        .iter()
                        .all(|byte| byte.is_ascii_graphic() && *byte != b':'),
                ),
                _ => (None, String::from_utf8_lossy(raw_bytes).into_owned(), false),
            };
            let normalized_name = name.as_ref().map(|value| value.to_ascii_lowercase());
            let unfolded_value = unfold_header_value(&raw_value);
            let locator = range_locator(
                bytes,
                header_start,
                content_end,
                path,
                None,
                Some(normalized_name.as_deref().unwrap_or("malformed-header")),
            );
            if !valid {
                diagnostics.push(
                    Diagnostic::malformed(PARSER, "malformed RFC 5322 header line")
                        .with_locator(locator.clone())
                        .partial(),
                );
            }
            EmailHeader {
                ordinal,
                name,
                normalized_name,
                raw: String::from_utf8_lossy(raw_bytes).into_owned(),
                raw_value,
                decoded_value: decode_rfc2047(&unfolded_value),
                unfolded_value,
                valid,
                locator,
            }
        })
        .collect()
}

fn unfold_header_value(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn first_header<'a>(headers: &'a [EmailHeader], name: &str) -> Option<&'a EmailHeader> {
    headers
        .iter()
        .find(|header| header.normalized_name.as_deref() == Some(name))
}

fn header_text(headers: &[EmailHeader], name: &str) -> Option<String> {
    first_header(headers, name).map(|header| header.decoded_value.trim().to_string())
}

fn text_fact(header: &EmailHeader) -> EmailTextFact {
    EmailTextFact {
        raw: header.unfolded_value.clone(),
        decoded: header.decoded_value.clone(),
        locator: header.locator.clone(),
    }
}

fn address_fields(headers: &[EmailHeader]) -> Vec<EmailAddressField> {
    const NAMES: &[&str] = &[
        "from",
        "sender",
        "reply-to",
        "to",
        "cc",
        "bcc",
        "resent-from",
        "resent-sender",
        "resent-to",
        "resent-cc",
        "resent-bcc",
    ];
    headers
        .iter()
        .filter(|header| {
            header
                .normalized_name
                .as_deref()
                .is_some_and(|name| NAMES.contains(&name))
        })
        .map(|header| EmailAddressField {
            header_name: header.name.clone().unwrap_or_default(),
            raw: header.unfolded_value.clone(),
            decoded: header.decoded_value.clone(),
            addresses: parse_addresses(&header.decoded_value),
            locator: header.locator.clone(),
        })
        .collect()
}

fn parse_addresses(value: &str) -> Vec<EmailAddress> {
    let mut output = Vec::new();
    for token in split_address_tokens(value) {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(colon) = top_level_char(trimmed, ':')
            && let Some(semicolon) = trimmed.rfind(';')
            && semicolon > colon
        {
            let group = trimmed[..colon].trim().trim_matches('"').to_string();
            for mut address in parse_addresses(&trimmed[colon + 1..semicolon]) {
                address.group = Some(group.clone());
                output.push(address);
            }
            if trimmed[colon + 1..semicolon].trim().is_empty() {
                output.push(EmailAddress {
                    raw: trimmed.to_string(),
                    display_name: None,
                    address: None,
                    group: Some(group),
                    comments: Vec::new(),
                });
            }
            continue;
        }
        let comments = comments(trimmed);
        let (display_name, address) =
            if let (Some(open), Some(close)) = (trimmed.rfind('<'), trimmed.rfind('>')) {
                if close > open {
                    (
                        clean_display_name(&trimmed[..open]),
                        Some(trimmed[open + 1..close].trim().to_string()),
                    )
                } else {
                    (None, None)
                }
            } else if trimmed.contains('@') {
                (None, Some(strip_comments(trimmed).trim().to_string()))
            } else {
                (clean_display_name(trimmed), None)
            };
        output.push(EmailAddress {
            raw: trimmed.to_string(),
            display_name,
            address,
            group: None,
            comments,
        });
    }
    output
}

fn split_address_tokens(value: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut start = 0;
    let mut quote = false;
    let mut escape = false;
    let mut comment_depth: usize = 0;
    let mut angle_depth: usize = 0;
    for (index, ch) in value.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if quote || comment_depth > 0 => escape = true,
            '"' if comment_depth == 0 => quote = !quote,
            '(' if !quote => comment_depth += 1,
            ')' if !quote => comment_depth = comment_depth.saturating_sub(1),
            '<' if !quote && comment_depth == 0 => angle_depth += 1,
            '>' if !quote && comment_depth == 0 => angle_depth = angle_depth.saturating_sub(1),
            ',' if !quote && comment_depth == 0 && angle_depth == 0 => {
                output.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    output.push(&value[start..]);
    output
}

fn top_level_char(value: &str, needle: char) -> Option<usize> {
    let mut quote = false;
    let mut comment: usize = 0;
    for (index, ch) in value.char_indices() {
        match ch {
            '"' if comment == 0 => quote = !quote,
            '(' if !quote => comment += 1,
            ')' if !quote => comment = comment.saturating_sub(1),
            _ if ch == needle && !quote && comment == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

fn comments(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if ch == '(' {
            if depth == 0 {
                start = index + 1;
            }
            depth += 1;
        } else if ch == ')' && depth > 0 {
            depth -= 1;
            if depth == 0 {
                result.push(value[start..index].to_string());
            }
        }
    }
    result
}

fn strip_comments(value: &str) -> String {
    let mut result = String::new();
    let mut depth = 0;
    for ch in value.chars() {
        if ch == '(' {
            depth += 1;
        } else if ch == ')' && depth > 0 {
            depth -= 1;
        } else if depth == 0 {
            result.push(ch);
        }
    }
    result
}

fn clean_display_name(value: &str) -> Option<String> {
    let value = strip_comments(value);
    let value = value.trim().trim_matches('"').trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn date_fields(headers: &[EmailHeader]) -> Vec<EmailDateField> {
    headers
        .iter()
        .filter(|header| {
            matches!(
                header.normalized_name.as_deref(),
                Some("date" | "resent-date" | "delivery-date")
            )
        })
        .map(|header| {
            let timezone_text = timezone_token(&header.decoded_value);
            EmailDateField {
                header_name: header.name.clone().unwrap_or_default(),
                raw: header.unfolded_value.clone(),
                decoded: header.decoded_value.clone(),
                timezone_offset_minutes: timezone_text.as_deref().and_then(timezone_offset),
                timezone_text,
                locator: header.locator.clone(),
            }
        })
        .collect()
}

fn timezone_token(value: &str) -> Option<String> {
    let without_comment = value.split('(').next().unwrap_or(value).trim_end();
    without_comment
        .split_whitespace()
        .next_back()
        .filter(|token| {
            let upper = token.to_ascii_uppercase();
            (token.len() == 5
                && matches!(token.as_bytes()[0], b'+' | b'-')
                && token.as_bytes()[1..].iter().all(u8::is_ascii_digit))
                || matches!(
                    upper.as_str(),
                    "UT" | "UTC"
                        | "GMT"
                        | "EST"
                        | "EDT"
                        | "CST"
                        | "CDT"
                        | "MST"
                        | "MDT"
                        | "PST"
                        | "PDT"
                )
        })
        .map(str::to_string)
}

fn timezone_offset(value: &str) -> Option<i32> {
    let upper = value.to_ascii_uppercase();
    let named = match upper.as_str() {
        "UT" | "UTC" | "GMT" => Some(0),
        "EST" => Some(-5 * 60),
        "EDT" => Some(-4 * 60),
        "CST" => Some(-6 * 60),
        "CDT" => Some(-5 * 60),
        "MST" => Some(-7 * 60),
        "MDT" => Some(-6 * 60),
        "PST" => Some(-8 * 60),
        "PDT" => Some(-7 * 60),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let sign = match value.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours = value.get(1..3)?.parse::<i32>().ok()?;
    let minutes = value.get(3..5)?.parse::<i32>().ok()?;
    (hours <= 23 && minutes <= 59).then_some(sign * (hours * 60 + minutes))
}

fn thread_evidence(
    headers: &[EmailHeader],
    subject: Option<&EmailTextFact>,
) -> EmailThreadEvidence {
    let message_id = first_header(headers, "message-id").map(text_fact);
    let in_reply_to = first_header(headers, "in-reply-to")
        .map(message_ids)
        .unwrap_or_default();
    let references = first_header(headers, "references")
        .map(message_ids)
        .unwrap_or_default();
    let received = headers
        .iter()
        .filter(|header| header.normalized_name.as_deref() == Some("received"))
        .map(text_fact)
        .collect();
    EmailThreadEvidence {
        message_id,
        in_reply_to,
        references,
        received,
        thread_index: first_header(headers, "thread-index").map(text_fact),
        thread_topic: first_header(headers, "thread-topic").map(text_fact),
        normalized_subject_hint: subject.map(|subject| normalize_subject(&subject.decoded)),
    }
}

fn message_ids(header: &EmailHeader) -> Vec<EmailMessageId> {
    let mut output = Vec::new();
    let bytes = header.decoded_value.as_bytes();
    let mut cursor = 0;
    while let Some(open) = bytes[cursor..].iter().position(|byte| *byte == b'<') {
        let open = cursor + open;
        let Some(close) = bytes[open + 1..].iter().position(|byte| *byte == b'>') else {
            break;
        };
        let close = open + 1 + close;
        output.push(EmailMessageId {
            value: header.decoded_value[open..=close].to_string(),
            locator: header.locator.clone(),
        });
        cursor = close + 1;
    }
    if output.is_empty() && !header.decoded_value.trim().is_empty() {
        output.extend(
            header
                .decoded_value
                .split_whitespace()
                .map(|value| EmailMessageId {
                    value: value.to_string(),
                    locator: header.locator.clone(),
                }),
        );
    }
    output
}

fn normalize_subject(value: &str) -> String {
    let mut value = value.trim();
    loop {
        let lower = value.to_ascii_lowercase();
        let removed = ["re:", "fw:", "fwd:"]
            .iter()
            .find_map(|prefix| lower.strip_prefix(prefix).map(|_| prefix.len()));
        let Some(length) = removed else {
            break;
        };
        value = value[length..].trim_start();
    }
    value.to_string()
}

fn authentication_evidence(headers: &[EmailHeader]) -> Vec<EmailAuthenticationEvidence> {
    headers
        .iter()
        .filter(|header| {
            header.normalized_name.as_deref().is_some_and(|name| {
                matches!(
                    name,
                    "dkim-signature"
                        | "domainkey-signature"
                        | "authentication-results"
                        | "arc-seal"
                        | "arc-message-signature"
                        | "arc-authentication-results"
                )
            })
        })
        .map(|header| EmailAuthenticationEvidence {
            kind: header.normalized_name.clone().unwrap_or_default(),
            raw: header.unfolded_value.clone(),
            decoded: header.decoded_value.clone(),
            locator: header.locator.clone(),
        })
        .collect()
}

fn should_capture_attachment(part: &MimePart) -> bool {
    if part.path.is_empty() {
        return false;
    }
    let disposition = part
        .content_disposition
        .as_ref()
        .map(|value| value.essence.as_str());
    let named = part
        .content_disposition
        .as_ref()
        .and_then(|value| value.parameter("filename"))
        .or_else(|| part.content_type.parameter("name"))
        .is_some();
    matches!(disposition, Some("attachment" | "inline"))
        || named
        || part.content_id.is_some()
        || (!part.content_type.essence.starts_with("text/")
            && part.content_type.essence != "message/rfc822")
}

fn nested_format(
    registry: &crate::registry::ParserRegistry,
    media_type: &str,
    filename: Option<&str>,
) -> Option<String> {
    if media_type.eq_ignore_ascii_case("message/rfc822") {
        return Some("eml".into());
    }
    let normalized = media_type
        .split(';')
        .next()
        .unwrap_or(media_type)
        .trim()
        .to_ascii_lowercase();
    if let Some(descriptor) = registry.parsers().into_iter().find(|descriptor| {
        descriptor
            .format
            .media_types
            .iter()
            .any(|value| value.eq_ignore_ascii_case(&normalized))
    }) {
        return Some(descriptor.format.id);
    }
    let extension = filename
        .and_then(|value| std::path::Path::new(value).extension())
        .and_then(|value| value.to_str())?;
    registry.parsers().into_iter().find_map(|descriptor| {
        descriptor
            .format
            .extensions
            .iter()
            .any(|value| value.eq_ignore_ascii_case(extension))
            .then_some(descriptor.format.id)
    })
}

fn split_header_body(bytes: &[u8], start: usize, end: usize) -> (usize, usize, bool) {
    let mut cursor = start;
    while cursor < end {
        let line_end = next_line(bytes, cursor, end);
        let content_end = trim_line_ending(bytes, cursor, line_end);
        if content_end == cursor {
            return (cursor, line_end, true);
        }
        cursor = line_end;
    }
    (end, end, false)
}

fn next_line(bytes: &[u8], start: usize, end: usize) -> usize {
    match bytes[start..end].iter().position(|byte| *byte == b'\n') {
        Some(relative) => start + relative + 1,
        None => end,
    }
}

fn trim_line_ending(bytes: &[u8], start: usize, mut end: usize) -> usize {
    if end > start && bytes[end - 1] == b'\n' {
        end -= 1;
    }
    if end > start && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    end
}

fn trim_trailing_line_endings(bytes: &[u8], start: usize, mut end: usize) -> usize {
    while end > start && matches!(bytes[end - 1], b'\r' | b'\n') {
        end -= 1;
    }
    end
}

fn boundary_delimiters(
    bytes: &[u8],
    start: usize,
    end: usize,
    boundary: &[u8],
) -> Vec<(usize, usize, bool)> {
    let mut output = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let next = next_line(bytes, cursor, end);
        let content_end = trim_line_ending(bytes, cursor, next);
        let line = &bytes[cursor..content_end];
        if let Some(rest) = line
            .strip_prefix(b"--")
            .and_then(|line| line.strip_prefix(boundary))
        {
            let closing = rest.strip_prefix(b"--");
            let trailing = closing.unwrap_or(rest);
            if trailing.iter().all(|byte| matches!(byte, b' ' | b'\t')) {
                output.push((cursor, next, closing.is_some()));
            }
        }
        cursor = next;
    }
    output
}

fn trim_framing_newline(bytes: &[u8], start: usize, mut end: usize) -> usize {
    if end > start && bytes[end - 1] == b'\n' {
        end -= 1;
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
    }
    end
}

fn range_locator(
    bytes: &[u8],
    start: usize,
    end: usize,
    path: &[usize],
    message_id: Option<&str>,
    header: Option<&str>,
) -> SourceLocator {
    let start = start.min(bytes.len());
    let end = end.min(bytes.len()).max(start);
    let (start_line, start_column) = raw_line_column(bytes, start);
    let (end_line, end_column) = raw_line_column(bytes, end);
    let email = LocationComponent::EmailPart {
        message_id: message_id.map(str::to_string),
        mime_path: path
            .iter()
            .map(|value| IndexPosition::one_based(*value as u64).expect("MIME paths are positive"))
            .collect(),
        header: header.map(str::to_string),
    };
    SourceLocator::exact(email)
        .expect("email locator is valid")
        .nested(SourceRange {
            byte_start: start,
            byte_end: end,
            start_line,
            start_column,
            end_line,
            end_column,
        })
        .expect("text range is valid")
}

fn raw_line_column(bytes: &[u8], end: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    let mut cursor = 0;
    while cursor < end.min(bytes.len()) {
        if bytes[cursor] == b'\n' {
            line += 1;
            column = 1;
            cursor += 1;
            continue;
        }
        let width = std::str::from_utf8(&bytes[cursor..end.min(bytes.len())])
            .ok()
            .and_then(|text| text.chars().next())
            .map(char::len_utf8)
            .or_else(|| {
                (1..=4)
                    .filter(|width| cursor + width <= end.min(bytes.len()))
                    .find(|width| std::str::from_utf8(&bytes[cursor..cursor + width]).is_ok())
            })
            .unwrap_or(1);
        cursor += width;
        column += 1;
    }
    (line, column)
}

fn html_remote_references(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut output = Vec::new();
    let mut cursor = 0;
    while cursor < lower.len() {
        let next = ["src=", "href="]
            .iter()
            .filter_map(|marker| {
                lower[cursor..]
                    .find(marker)
                    .map(|offset| (offset, marker.len()))
            })
            .min_by_key(|value| value.0);
        let Some((offset, marker_len)) = next else {
            break;
        };
        let value_start = cursor + offset + marker_len;
        let remainder = text[value_start..].trim_start();
        let skipped = text[value_start..].len() - remainder.len();
        let (value, consumed) = if let Some(quote) = remainder
            .chars()
            .next()
            .filter(|ch| matches!(ch, '\'' | '"'))
        {
            let inner = &remainder[quote.len_utf8()..];
            let end = inner.find(quote).unwrap_or(inner.len());
            (
                &inner[..end],
                quote.len_utf8() + end + usize::from(end < inner.len()),
            )
        } else {
            let end = remainder
                .find(|ch: char| ch.is_whitespace() || ch == '>')
                .unwrap_or(remainder.len());
            (&remainder[..end], end)
        };
        if is_remote_uri(value) && !output.iter().any(|existing| existing == value) {
            output.push(value.to_string());
        }
        cursor = value_start + skipped + consumed;
    }
    output
}

fn is_remote_uri(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("//")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CancellationToken;

    fn control() -> OperationControl {
        OperationControl::new(
            &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
            CancellationToken::new(),
        )
        .unwrap()
    }

    #[test]
    fn parses_folded_headers_addresses_dates_and_threading() {
        let bytes = b"From: =?UTF-8?Q?Jos=C3=A9?= <jose@example.test>\r\nTo: Team: a@example.test, b@example.test;\r\nSubject: Re: =?UTF-8?Q?Ol=C3=A1?=\r\nDate: Fri, 7 Aug 2026 10:00:00 -0700\r\nMessage-ID: <m@example.test>\r\nReferences: <a@example.test>\r\n <b@example.test>\r\n\r\nhello";
        let document = parse_document(
            bytes,
            &SourceInfo::new("mail.eml"),
            &EmailOptions::default(),
            &control(),
        )
        .unwrap();
        assert_eq!(document.subject.unwrap().decoded, "Re: Olá");
        assert_eq!(
            document.address_fields[0].addresses[0].address.as_deref(),
            Some("jose@example.test")
        );
        assert_eq!(document.date_fields[0].timezone_offset_minutes, Some(-420));
        assert_eq!(document.thread.references.len(), 2);
    }

    #[test]
    fn malformed_headers_are_retained_and_partial() {
        let bytes = b"Broken header\r\n\r\nbody";
        let document = parse_document(
            bytes,
            &SourceInfo::new("bad.eml"),
            &EmailOptions::default(),
            &control(),
        )
        .unwrap();
        assert!(!document.headers[0].valid);
        assert!(!document.complete);
    }
}
