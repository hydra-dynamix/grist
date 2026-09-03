use super::*;
use crate::core::{
    ContentIdentity, Diagnostic, FormatIdentity, IndexBase, IndexRange, LocationComponent,
    OperationControl, OperationStatus, RequestId, SourceInfo, SourceLocator, SourceRange,
    StreamEvent, StreamItem, StreamTerminal, sha256_hex,
};
use std::collections::BTreeMap;

const PARSER: &str = "grist.mbox";

pub type MboxStreamEvent = StreamEvent<MboxMessage>;

pub fn stream_mbox<'a>(
    bytes: &'a [u8],
    source: SourceInfo,
    options: &'a MboxOptions,
    request_id: RequestId,
    control: OperationControl,
) -> MboxStream<'a> {
    let invalid_options = options.max_messages == 0 || options.max_separator_bytes == 0;
    let diagnostics = invalid_options
        .then(|| Diagnostic::malformed(PARSER, "mbox limits must be greater than zero"))
        .into_iter()
        .collect();
    MboxStream {
        bytes,
        source,
        options,
        request_id,
        control,
        offset: 0,
        sequence: 0,
        duplicate_counts: BTreeMap::new(),
        diagnostics,
        invalid_options,
        finished: false,
    }
}

pub struct MboxStream<'a> {
    bytes: &'a [u8],
    source: SourceInfo,
    options: &'a MboxOptions,
    request_id: RequestId,
    control: OperationControl,
    offset: usize,
    sequence: u64,
    duplicate_counts: BTreeMap<String, usize>,
    diagnostics: Vec<Diagnostic>,
    invalid_options: bool,
    finished: bool,
}

impl Iterator for MboxStream<'_> {
    type Item = MboxStreamEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if self.invalid_options {
            self.finished = true;
            return Some(StreamEvent::terminal(StreamTerminal {
                status: OperationStatus::Failed,
                emitted_items: 0,
                diagnostics: std::mem::take(&mut self.diagnostics),
                budget_usage: self.control.usage(),
            }));
        }
        if let Err(error) = self.control.checkpoint() {
            self.finished = true;
            return Some(StreamEvent::terminal(StreamTerminal::from_control(
                PARSER,
                self.sequence,
                &self.control,
                error,
            )));
        }
        if self.offset >= self.bytes.len() {
            self.finished = true;
            let status = if self.diagnostics.iter().any(|diagnostic| diagnostic.partial) {
                OperationStatus::Partial
            } else {
                OperationStatus::Complete
            };
            return Some(StreamEvent::terminal(StreamTerminal {
                status,
                emitted_items: self.sequence,
                diagnostics: std::mem::take(&mut self.diagnostics),
                budget_usage: self.control.usage(),
            }));
        }
        if self.sequence as usize >= self.options.max_messages {
            self.finished = true;
            self.diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "mbox.limit.messages",
                    format!(
                        "mailbox exceeded max_messages {}",
                        self.options.max_messages
                    ),
                )
                .partial(),
            );
            return Some(StreamEvent::terminal(StreamTerminal {
                status: OperationStatus::Partial,
                emitted_items: self.sequence,
                diagnostics: std::mem::take(&mut self.diagnostics),
                budget_usage: self.control.usage(),
            }));
        }
        if let Err(error) = self.control.budget().consume_records(1) {
            self.finished = true;
            return Some(StreamEvent::terminal(StreamTerminal::from_control(
                PARSER,
                self.sequence,
                &self.control,
                error.into(),
            )));
        }

        let record_start = self.offset;
        let separator = separator_at(self.bytes, record_start, self.options.max_separator_bytes);
        let (message_start, separator) = if let Some(found) = separator {
            (
                found.line_end,
                Some(materialize_separator(self.bytes, found)),
            )
        } else {
            if self.sequence == 0 && !self.options.recover_missing_initial_separator {
                self.finished = true;
                return Some(StreamEvent::terminal(StreamTerminal {
                    status: OperationStatus::Failed,
                    emitted_items: 0,
                    diagnostics: vec![Diagnostic::malformed(
                        PARSER,
                        "mailbox does not begin with a valid mbox envelope separator",
                    )],
                    budget_usage: self.control.usage(),
                }));
            }
            let end = find_next_separator(
                self.bytes,
                record_start.saturating_add(1),
                self.options.max_separator_bytes,
            )
            .unwrap_or(self.bytes.len());
            self.diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "mbox.separator.missing",
                    "recovered bytes without an envelope separator as a message",
                )
                .with_locator(range_locator(self.bytes, record_start, end))
                .partial(),
            );
            (record_start, None)
        };

        let boundary = locate_message_end(self.bytes, message_start, self.options);
        let mut message_end = boundary.end.max(message_start).min(self.bytes.len());
        if separator.is_none() {
            message_end = find_next_separator(
                self.bytes,
                message_start.saturating_add(1),
                self.options.max_separator_bytes,
            )
            .unwrap_or(self.bytes.len());
        }
        if message_end == record_start && message_end < self.bytes.len() {
            message_end += 1;
        }
        self.offset = message_end;

        let ordinal = usize::try_from(self.sequence).unwrap_or(usize::MAX) + 1;
        let raw = &self.bytes[message_start..message_end];
        let (decoded, escaped_from_lines) = if self.options.unescape_from_lines {
            unescape_from_lines(self.bytes, message_start, message_end)
        } else {
            (raw.to_vec(), Vec::new())
        };
        let decoded_sha256 = sha256_hex(&decoded);
        let duplicate_occurrence = self
            .duplicate_counts
            .entry(decoded_sha256.clone())
            .and_modify(|count| *count += 1)
            .or_insert(1);
        let stable_id = format!(
            "mbox-message:{}:{}",
            decoded_sha256
                .strip_prefix("sha256:")
                .unwrap_or(&decoded_sha256),
            duplicate_occurrence
        );
        let locator = message_locator(self.bytes, message_start, message_end, ordinal);
        let child_source = SourceInfo::new(stable_id.clone())
            .with_declared_mime_type("message/rfc822")
            .with_parent(self.source.clone());
        let envelope = crate::email::parse_email_with_operation_control(
            &decoded,
            child_source,
            &self.options.email,
            &self.control,
        );
        let status = envelope.status;
        let mut email = envelope.payload;
        let mut message_diagnostics = envelope.diagnostics;
        if let Some(document) = &mut email {
            prefix_email_locators(document, &locator);
        }
        for diagnostic in &mut message_diagnostics {
            prefix_diagnostic_locator(diagnostic, &locator);
        }
        if status != OperationStatus::Complete {
            self.diagnostics.push(
                Diagnostic::warning(
                    PARSER,
                    "mbox.message.incomplete",
                    format!("message {stable_id} parsed with status {status:?}"),
                )
                .with_locator(locator.clone())
                .partial(),
            );
        }
        let mut boundary_diagnostics = boundary.diagnostics;
        for diagnostic in &mut boundary_diagnostics {
            if diagnostic.locator.is_none() {
                diagnostic.locator = Some(Box::new(locator.clone()));
            }
        }
        self.diagnostics.extend(boundary_diagnostics.clone());
        message_diagnostics.extend(boundary_diagnostics);
        let content_length = boundary.content_length;
        let duplicate_occurrence = *duplicate_occurrence;
        let message = MboxMessage {
            ordinal,
            stable_id,
            duplicate_occurrence,
            raw_sha256: sha256_hex(raw),
            decoded_sha256,
            raw_bytes: raw.len(),
            decoded_bytes: decoded.len(),
            separator,
            escaped_from_lines,
            content_length,
            locator,
            status,
            email,
            diagnostics: message_diagnostics,
        };
        let identity = ContentIdentity::for_raw_bytes(&decoded)
            .with_format(FormatIdentity::new("eml", Some("message/rfc822")));
        let event = StreamEvent::item(StreamItem::new(
            self.sequence,
            self.request_id.clone(),
            identity,
            message,
        ));
        self.sequence += 1;
        Some(event)
    }
}

#[derive(Debug, Clone, Copy)]
struct SeparatorLine {
    start: usize,
    content_end: usize,
    line_end: usize,
    ending: MboxLineEnding,
}

fn separator_at(bytes: &[u8], start: usize, maximum: usize) -> Option<SeparatorLine> {
    if start >= bytes.len() || (start > 0 && !matches!(bytes[start - 1], b'\n' | b'\r')) {
        return None;
    }
    let line_end = next_line(bytes, start);
    if line_end.saturating_sub(start) > maximum {
        return None;
    }
    let (content_end, ending) = line_content_end(bytes, start, line_end);
    let line = &bytes[start..content_end];
    if !line.starts_with(b"From ") {
        return None;
    }
    let text = String::from_utf8_lossy(&line[5..]);
    let mut fields = text.split_ascii_whitespace();
    let sender = fields.next()?;
    let tail = fields.collect::<Vec<_>>();
    if sender.is_empty()
        || tail.len() < 3
        || !tail
            .iter()
            .any(|field| field.bytes().any(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    Some(SeparatorLine {
        start,
        content_end,
        line_end,
        ending,
    })
}

fn find_next_separator(bytes: &[u8], from: usize, maximum: usize) -> Option<usize> {
    let mut offset = from.min(bytes.len());
    if offset > 0 && !matches!(bytes[offset - 1], b'\n' | b'\r') {
        offset = next_line(bytes, offset);
    }
    while offset < bytes.len() {
        if separator_at(bytes, offset, maximum).is_some() {
            return Some(offset);
        }
        let next = next_line(bytes, offset);
        if next <= offset {
            break;
        }
        offset = next;
    }
    None
}

fn materialize_separator(bytes: &[u8], separator: SeparatorLine) -> MboxSeparator {
    let raw = String::from_utf8_lossy(&bytes[separator.start..separator.content_end]).into_owned();
    let rest = raw.strip_prefix("From ").unwrap_or(&raw);
    let (sender, date) = rest
        .split_once(char::is_whitespace)
        .map(|(sender, date)| (sender.to_string(), date.trim_start().to_string()))
        .unwrap_or_else(|| (rest.to_string(), String::new()));
    MboxSeparator {
        raw,
        sender,
        date,
        line_ending: separator.ending,
        locator: range_locator(bytes, separator.start, separator.line_end),
    }
}

struct MessageBoundary {
    end: usize,
    content_length: Option<MboxContentLengthEvidence>,
    diagnostics: Vec<Diagnostic>,
}

fn locate_message_end(bytes: &[u8], start: usize, options: &MboxOptions) -> MessageBoundary {
    let fallback = find_next_separator(bytes, start.saturating_add(1), options.max_separator_bytes)
        .unwrap_or(bytes.len());
    let Some(header) = find_content_length(bytes, start, fallback) else {
        return MessageBoundary {
            end: fallback,
            content_length: None,
            diagnostics: Vec::new(),
        };
    };
    let declared = header.value.trim().parse::<usize>().ok();
    let mut evidence = MboxContentLengthEvidence {
        raw: header.value.clone(),
        declared_body_bytes: declared,
        observed_body_bytes: fallback.saturating_sub(header.body_start),
        honored: false,
        exact_boundary: false,
        locator: range_locator(bytes, header.start, header.end),
    };
    let mut diagnostics = Vec::new();
    let Some(declared) = declared else {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "mbox.content_length.invalid",
                "Content-Length is not a decimal byte count; separator recovery was used",
            )
            .with_locator(evidence.locator.clone())
            .partial(),
        );
        return MessageBoundary {
            end: fallback,
            content_length: Some(evidence),
            diagnostics,
        };
    };
    if !options.honor_content_length {
        return MessageBoundary {
            end: fallback,
            content_length: Some(evidence),
            diagnostics,
        };
    }
    let Some(expected) = header.body_start.checked_add(declared) else {
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "mbox.content_length.overflow",
                "Content-Length overflowed the source range; separator recovery was used",
            )
            .partial(),
        );
        return MessageBoundary {
            end: fallback,
            content_length: Some(evidence),
            diagnostics,
        };
    };
    if expected > bytes.len() {
        evidence.honored = true;
        evidence.observed_body_bytes = bytes.len().saturating_sub(header.body_start);
        diagnostics.push(
            Diagnostic::warning(
                PARSER,
                "mbox.content_length.truncated",
                format!(
                    "Content-Length declares {declared} bytes but only {} remain",
                    evidence.observed_body_bytes
                ),
            )
            .partial(),
        );
        return MessageBoundary {
            end: bytes.len(),
            content_length: Some(evidence),
            diagnostics,
        };
    }
    let exact = separator_at(bytes, expected, options.max_separator_bytes).is_some()
        || expected == bytes.len();
    let after_newline = if expected < bytes.len() && matches!(bytes[expected], b'\n' | b'\r') {
        let candidate = next_line(bytes, expected);
        separator_at(bytes, candidate, options.max_separator_bytes).map(|_| candidate)
    } else {
        None
    };
    if exact || after_newline.is_some() {
        evidence.honored = true;
        evidence.exact_boundary = true;
        evidence.observed_body_bytes = declared;
        return MessageBoundary {
            end: after_newline.unwrap_or(expected),
            content_length: Some(evidence),
            diagnostics,
        };
    }
    diagnostics.push(
        Diagnostic::warning(
            PARSER,
            "mbox.content_length.mismatch",
            "Content-Length did not end at a mailbox boundary; separator recovery was used",
        )
        .partial(),
    );
    MessageBoundary {
        end: fallback,
        content_length: Some(evidence),
        diagnostics,
    }
}

struct ContentLengthHeader {
    start: usize,
    end: usize,
    body_start: usize,
    value: String,
}

fn find_content_length(bytes: &[u8], start: usize, end: usize) -> Option<ContentLengthHeader> {
    let mut offset = start;
    let mut found: Option<ContentLengthHeader> = None;
    while offset < end {
        let line_end = next_line_bounded(bytes, offset, end);
        let (content_end, _) = line_content_end(bytes, offset, line_end);
        if content_end == offset {
            if let Some(header) = &mut found {
                header.body_start = line_end;
            }
            return found;
        }
        let line = &bytes[offset..content_end];
        if let Some(colon) = line.iter().position(|byte| *byte == b':')
            && line[..colon].eq_ignore_ascii_case(b"content-length")
        {
            found = Some(ContentLengthHeader {
                start: offset,
                end: line_end,
                body_start: line_end,
                value: String::from_utf8_lossy(&line[colon + 1..])
                    .trim()
                    .to_string(),
            });
        }
        offset = line_end;
    }
    found
}

fn unescape_from_lines(
    source: &[u8],
    start: usize,
    end: usize,
) -> (Vec<u8>, Vec<MboxEscapedFromLine>) {
    let body_start = header_body_start(source, start, end);
    let mut decoded = Vec::with_capacity(end.saturating_sub(start));
    let mut evidence = Vec::new();
    let mut offset = start;
    while offset < end {
        let line_end = next_line_bounded(source, offset, end);
        let line = &source[offset..line_end];
        let quotes = line.iter().take_while(|byte| **byte == b'>').count();
        if offset >= body_start && quotes > 0 && line[quotes..].starts_with(b"From ") {
            evidence.push(MboxEscapedFromLine {
                raw_prefix_length: quotes,
                decoded_offset: decoded.len(),
                locator: range_locator(source, offset, line_end),
            });
            decoded.extend_from_slice(&line[1..]);
        } else {
            decoded.extend_from_slice(line);
        }
        offset = line_end;
    }
    (decoded, evidence)
}

fn header_body_start(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut offset = start;
    while offset < end {
        let next = next_line_bounded(bytes, offset, end);
        let (content_end, _) = line_content_end(bytes, offset, next);
        if content_end == offset {
            return next;
        }
        offset = next;
    }
    end
}

fn next_line(bytes: &[u8], start: usize) -> usize {
    next_line_bounded(bytes, start, bytes.len())
}

fn next_line_bounded(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut offset = start;
    while offset < end {
        match bytes[offset] {
            b'\n' => return offset + 1,
            b'\r' if offset + 1 < end && bytes[offset + 1] == b'\n' => return offset + 2,
            b'\r' => return offset + 1,
            _ => offset += 1,
        }
    }
    end
}

fn line_content_end(bytes: &[u8], start: usize, line_end: usize) -> (usize, MboxLineEnding) {
    if line_end <= start {
        return (line_end, MboxLineEnding::None);
    }
    if bytes[line_end - 1] == b'\n' {
        if line_end >= start + 2 && bytes[line_end - 2] == b'\r' {
            (line_end - 2, MboxLineEnding::CrLf)
        } else {
            (line_end - 1, MboxLineEnding::Lf)
        }
    } else if bytes[line_end - 1] == b'\r' {
        (line_end - 1, MboxLineEnding::Cr)
    } else {
        (line_end, MboxLineEnding::None)
    }
}

fn message_locator(bytes: &[u8], start: usize, end: usize, ordinal: usize) -> SourceLocator {
    let records = IndexRange::new(ordinal as u64, ordinal as u64, IndexBase::One)
        .expect("one-based message ordinal is valid");
    range_locator(bytes, start, end)
        .nested(LocationComponent::RecordRange {
            collection: "mbox.messages".into(),
            records,
            field: None,
        })
        .expect("message record locator is valid")
}

fn range_locator(bytes: &[u8], start: usize, end: usize) -> SourceLocator {
    let (start_line, start_column) = raw_line_column(bytes, start);
    let (end_line, end_column) = raw_line_column(bytes, end);
    SourceLocator::exact(SourceRange {
        byte_start: start,
        byte_end: end,
        start_line,
        start_column,
        end_line,
        end_column,
    })
    .expect("byte-derived source range is valid")
}

fn raw_line_column(bytes: &[u8], end: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for byte in bytes.iter().take(end.min(bytes.len())) {
        if *byte == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn nest_locator(outer: &SourceLocator, inner: &SourceLocator) -> SourceLocator {
    let mut components = outer.components().to_vec();
    components.extend_from_slice(inner.components());
    SourceLocator::new(components, inner.precision().clone())
        .expect("nested mailbox locator remains valid")
}

fn prefix_diagnostic_locator(diagnostic: &mut Diagnostic, outer: &SourceLocator) {
    if let Some(locator) = &mut diagnostic.locator {
        **locator = nest_locator(outer, locator);
    }
}

fn prefix_email_locators(document: &mut crate::email::EmailDocument, outer: &SourceLocator) {
    for header in &mut document.headers {
        header.locator = nest_locator(outer, &header.locator);
    }
    for field in &mut document.address_fields {
        field.locator = nest_locator(outer, &field.locator);
    }
    for field in &mut document.date_fields {
        field.locator = nest_locator(outer, &field.locator);
    }
    if let Some(subject) = &mut document.subject {
        subject.locator = nest_locator(outer, &subject.locator);
    }
    if let Some(message_id) = &mut document.thread.message_id {
        message_id.locator = nest_locator(outer, &message_id.locator);
    }
    for message_id in document
        .thread
        .in_reply_to
        .iter_mut()
        .chain(document.thread.references.iter_mut())
    {
        message_id.locator = nest_locator(outer, &message_id.locator);
    }
    for received in &mut document.thread.received {
        received.locator = nest_locator(outer, &received.locator);
    }
    for fact in [
        &mut document.thread.thread_index,
        &mut document.thread.thread_topic,
    ]
    .into_iter()
    .flatten()
    {
        fact.locator = nest_locator(outer, &fact.locator);
    }
    for evidence in &mut document.authentication {
        evidence.locator = nest_locator(outer, &evidence.locator);
    }
    prefix_mime_locators(&mut document.mime, outer);
    for reference in &mut document.external_references {
        reference.locator = nest_locator(outer, &reference.locator);
    }
    for diagnostic in &mut document.diagnostics {
        prefix_diagnostic_locator(diagnostic, outer);
    }
}

fn prefix_mime_locators(part: &mut crate::email::MimePart, outer: &SourceLocator) {
    part.locator = nest_locator(outer, &part.locator);
    part.body_locator = nest_locator(outer, &part.body_locator);
    for header in &mut part.headers {
        header.locator = nest_locator(outer, &header.locator);
    }
    if let Some(text) = &mut part.text {
        text.locator = nest_locator(outer, &text.locator);
    }
    if let Some(attachment) = &mut part.attachment {
        attachment.artifact.locator = nest_locator(outer, &attachment.artifact.locator);
    }
    for child in &mut part.children {
        prefix_mime_locators(child, outer);
    }
}
