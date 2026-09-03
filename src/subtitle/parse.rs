use super::model::*;
use crate::core::{
    BudgetExceeded, Diagnostic, DiagnosticClass, DiagnosticCode, IndexPosition, LineIndex,
    LocationComponent, OperationControl, OperationControlError, SourceLocator, SourceRange,
};
use crate::decode::DecodedText;
use quick_xml::{
    events::{BytesStart, Event},
    name::{Namespace, ResolveResult},
    reader::NsReader,
};
use std::cell::Cell;
use std::collections::BTreeMap;

const PARSER: &str = "grist.subtitle";
const TTML_NAMESPACE: &str = "http://www.w3.org/ns/ttml";
const TTML_LEGACY_NAMESPACE: &str = "http://www.w3.org/2006/10/ttaf1";
const TTML_PARAMETER_NAMESPACE: &str = "http://www.w3.org/ns/ttml#parameter";
const TTML_LEGACY_PARAMETER_NAMESPACE: &str = "http://www.w3.org/2006/10/ttaf1#parameter";
const TTML_METADATA_NAMESPACE: &str = "http://www.w3.org/ns/ttml#metadata";
const TTML_LEGACY_METADATA_NAMESPACE: &str = "http://www.w3.org/2006/10/ttaf1#metadata";
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

#[derive(Debug)]
pub(crate) enum SubtitleParseError {
    Control(OperationControlError),
}

impl From<OperationControlError> for SubtitleParseError {
    fn from(value: OperationControlError) -> Self {
        Self::Control(value)
    }
}

impl From<BudgetExceeded> for SubtitleParseError {
    fn from(value: BudgetExceeded) -> Self {
        Self::Control(OperationControlError::BudgetExceeded(value))
    }
}

type ParseResult<T> = Result<T, SubtitleParseError>;

struct Guard<'a> {
    control: &'a OperationControl,
    retained_memory: Cell<u64>,
}

impl Guard<'_> {
    fn new(control: &OperationControl) -> Guard<'_> {
        Guard {
            control,
            retained_memory: Cell::new(0),
        }
    }

    fn checkpoint(&self) -> ParseResult<()> {
        self.control.checkpoint().map_err(Into::into)
    }

    fn node(&self) -> ParseResult<()> {
        self.control.budget().consume_nodes(1).map_err(Into::into)
    }

    fn record(&self) -> ParseResult<()> {
        self.control.budget().consume_records(1).map_err(Into::into)
    }

    fn depth(&self, depth: usize) -> ParseResult<()> {
        self.control
            .budget()
            .observe_nesting_depth(depth as u64)
            .map_err(Into::into)
    }

    fn retain(&self, bytes: usize) -> ParseResult<()> {
        let total = self
            .retained_memory
            .get()
            .checked_add(u64::try_from(bytes).unwrap_or(u64::MAX))
            .unwrap_or(u64::MAX);
        self.control
            .budget()
            .observe_memory_bytes(total)
            .map_err(SubtitleParseError::from)?;
        self.retained_memory.set(total);
        Ok(())
    }

    fn memory(&self, bytes: usize) -> ParseResult<()> {
        let total = self
            .retained_memory
            .get()
            .checked_add(u64::try_from(bytes).unwrap_or(u64::MAX))
            .unwrap_or(u64::MAX);
        self.control
            .budget()
            .observe_memory_bytes(total)
            .map_err(Into::into)
    }
}

#[derive(Clone, Copy)]
struct Line<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

struct PhysicalLines<'a> {
    text: &'a str,
    cursor: usize,
    emitted_empty: bool,
}

impl<'a> PhysicalLines<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            cursor: 0,
            emitted_empty: false,
        }
    }

    fn next_checked(&mut self, guard: &Guard<'_>) -> ParseResult<Option<Line<'a>>> {
        if self.cursor >= self.text.len() {
            if self.text.is_empty() && !self.emitted_empty {
                self.emitted_empty = true;
                return Ok(Some(Line {
                    start: 0,
                    end: 0,
                    text: "",
                }));
            }
            return Ok(None);
        }
        let start = self.cursor;
        let bytes = self.text.as_bytes();
        let mut end = start;
        while end < bytes.len() {
            let chunk_end = end.saturating_add(64 * 1024).min(bytes.len());
            if let Some(relative) = bytes[end..chunk_end]
                .iter()
                .position(|byte| matches!(byte, b'\r' | b'\n'))
            {
                end += relative;
                break;
            }
            end = chunk_end;
            guard.checkpoint()?;
        }
        self.cursor = end;
        if self.cursor < self.text.len() {
            let first = self.text.as_bytes()[self.cursor];
            self.cursor += 1;
            if first == b'\r' && self.text.as_bytes().get(self.cursor) == Some(&b'\n') {
                self.cursor += 1;
            }
        }
        Ok(Some(Line {
            start,
            end,
            text: &self.text[start..end],
        }))
    }
}

#[derive(Default)]
struct Parsed {
    metadata: BTreeMap<String, String>,
    tracks: Vec<SubtitleTrack>,
    regions: Vec<SubtitleRegion>,
    styles: Vec<SubtitleStyle>,
    cues: Vec<SubtitleCue>,
    diagnostics: Vec<Diagnostic>,
}

impl Parsed {
    fn new(text: &str, index: &LineIndex, guard: &Guard<'_>) -> ParseResult<Self> {
        guard.node()?;
        let track = default_track(text, index);
        guard.retain(track_memory(&track))?;
        Ok(Self {
            tracks: vec![track],
            ..Self::default()
        })
    }
}

pub(crate) fn parse_decoded(
    decoded: &DecodedText,
    format: SubtitleFormat,
    options: &SubtitleOptions,
    control: &OperationControl,
) -> ParseResult<SubtitleDocument> {
    let guard = Guard::new(control);
    guard.checkpoint()?;
    guard.node()?;
    guard.retain(decoded.raw_bytes().len())?;
    guard.retain(decoded.text.len())?;
    guard.retain(decode_report_memory(&decoded.report))?;
    let index = LineIndex::new(&decoded.text);
    guard.retain(decoded.text.len())?;
    let range = SourceRange::new(0, decoded.text.len(), &index);
    let mut parsed = match format {
        SubtitleFormat::Srt => parse_blocks(&decoded.text, &index, options, false, &guard)?,
        SubtitleFormat::WebVtt => parse_blocks(&decoded.text, &index, options, true, &guard)?,
        SubtitleFormat::Ttml => parse_ttml(&decoded.text, &index, options, &guard)?,
    };
    guard.retain(string_map_memory(&parsed.metadata))?;
    diagnose_overlaps(&parsed.cues, &mut parsed.diagnostics, &guard)?;
    let transcript = transcript(&parsed.cues, &guard)?;
    for diagnostic in &decoded.report.diagnostics {
        guard.retain(diagnostic_memory(diagnostic))?;
    }
    let mut diagnostics = decoded.report.diagnostics.clone();
    diagnostics.extend(parsed.diagnostics);
    let complete = diagnostics.is_empty();
    guard.retain(decoded.raw_bytes().len())?;
    guard.retain(decoded.text.len())?;
    guard.retain(decode_report_memory(&decoded.report))?;
    Ok(SubtitleDocument {
        schema_version: crate::core::SchemaVersion::SUBTITLE_V1.into(),
        format,
        raw_bytes: decoded.raw_bytes().to_vec(),
        raw_range: crate::decode::RawByteRange {
            start: 0,
            end: decoded.raw_bytes().len() as u64,
        },
        decoded_text: decoded.text.clone(),
        decoded_range: range.clone(),
        locator: exact(range),
        encoding: decoded.report.encoding.clone(),
        decoding: decoded.report.clone(),
        metadata: parsed.metadata,
        tracks: parsed.tracks,
        regions: parsed.regions,
        styles: parsed.styles,
        cues: parsed.cues,
        transcript,
        diagnostics,
        complete,
    })
}

fn parse_blocks(
    text: &str,
    index: &LineIndex,
    options: &SubtitleOptions,
    webvtt: bool,
    guard: &Guard<'_>,
) -> ParseResult<Parsed> {
    let mut out = Parsed::new(text, index, guard)?;
    let mut lines = PhysicalLines::new(text);
    let first = lines.next_checked(guard)?;
    let header_valid = !webvtt || first.is_some_and(|line| valid_webvtt_header(line.text));
    if webvtt && !header_valid {
        let end = first.map_or(0, |line| line.end);
        push_diagnostic(
            &mut out.diagnostics,
            malformed(
                "subtitle.webvtt.missing_header",
                "WebVTT input must begin with an exact WEBVTT header",
                0,
                end,
                index,
            ),
            guard,
        )?;
    } else if webvtt {
        let line = first.expect("validated WebVTT header exists");
        let header = line.text.strip_prefix('\u{feff}').unwrap_or(line.text);
        let note = header.strip_prefix("WEBVTT").unwrap_or("").trim();
        if !note.is_empty() {
            out.metadata.insert("header".into(), note.into());
        }
    }

    let mut lines = PhysicalLines::new(text);
    let mut block_index = 0usize;
    let mut seen_cue = false;
    while let Some(block) = next_block(&mut lines, guard)? {
        guard.checkpoint()?;
        let first = block[0];
        let last = *block.last().expect("blocks are non-empty");
        if webvtt && first.start == 0 && valid_webvtt_header(first.text) {
            continue;
        }
        if webvtt && first.text.trim() == "STYLE" {
            if seen_cue {
                push_diagnostic(
                    &mut out.diagnostics,
                    malformed(
                        "subtitle.webvtt.late_style",
                        "WebVTT STYLE blocks must precede every cue",
                        first.start,
                        last.end,
                        index,
                    ),
                    guard,
                )?;
            }
            if out.styles.len() >= options.max_styles {
                push_diagnostic(
                    &mut out.diagnostics,
                    limit(
                        "subtitle.limit.styles",
                        "WebVTT style limit exceeded",
                        first.start,
                        last.end,
                        index,
                    ),
                    guard,
                )?;
                continue;
            }
            guard.node()?;
            let raw = block.get(1).map_or("", |line| &text[line.start..last.end]);
            guard.retain(webvtt_style_memory(raw, out.styles.len()))?;
            let style = SubtitleStyle {
                id: format!("style-{:06}", out.styles.len()),
                selector: raw.lines().next().map(|line| line.trim().into()),
                properties: css_properties(&raw, guard)?,
                raw: raw.into(),
                locator: locator(first.start, last.end, index),
            };
            out.styles.push(style);
            continue;
        }
        if webvtt && first.text.trim() == "REGION" {
            if seen_cue {
                push_diagnostic(
                    &mut out.diagnostics,
                    malformed(
                        "subtitle.webvtt.late_region",
                        "WebVTT REGION blocks must precede every cue",
                        first.start,
                        last.end,
                        index,
                    ),
                    guard,
                )?;
            }
            if out.regions.len() >= options.max_regions {
                push_diagnostic(
                    &mut out.diagnostics,
                    limit(
                        "subtitle.limit.regions",
                        "WebVTT region limit exceeded",
                        first.start,
                        last.end,
                        index,
                    ),
                    guard,
                )?;
                continue;
            }
            guard.node()?;
            guard.retain(webvtt_region_memory(&block, out.regions.len()))?;
            let settings = block
                .iter()
                .skip(1)
                .filter_map(|line| line.text.split_once(':'))
                .map(|(key, value)| (key.trim().into(), value.trim().into()))
                .collect::<BTreeMap<_, _>>();
            let id = settings
                .get("id")
                .cloned()
                .unwrap_or_else(|| format!("region-{:06}", out.regions.len()));
            let region = SubtitleRegion {
                id,
                settings,
                locator: locator(first.start, last.end, index),
            };
            out.regions.push(region);
            continue;
        }
        if webvtt && is_webvtt_note(first.text) {
            guard.node()?;
            out.metadata.insert(
                format!("note-{:06}", out.metadata.len()),
                text[first.start..last.end].into(),
            );
            continue;
        }
        seen_cue = true;
        if out.cues.len() >= options.max_cues {
            push_diagnostic(
                &mut out.diagnostics,
                limit(
                    "subtitle.limit.cues",
                    "subtitle cue limit exceeded",
                    first.start,
                    last.end,
                    index,
                ),
                guard,
            )?;
            break;
        }
        let source_index = block_index;
        block_index = block_index.saturating_add(1);
        let Some(time_at) = block.iter().position(|line| line.text.contains("-->")) else {
            push_diagnostic(
                &mut out.diagnostics,
                malformed(
                    if webvtt {
                        "subtitle.webvtt.missing_timing"
                    } else {
                        "subtitle.srt.missing_timing"
                    },
                    "cue block has no timing line",
                    first.start,
                    last.end,
                    index,
                ),
                guard,
            )?;
            continue;
        };
        let time_line = block[time_at];
        let source_id = (time_at > 0).then(|| first.text.trim().to_string());
        let text_lines = &block[time_at + 1..];
        let (text_start, text_end, raw_text) = match (text_lines.first(), text_lines.last()) {
            (Some(first), Some(last)) => (
                first.start,
                last.end,
                text[first.start..last.end].to_string(),
            ),
            _ => (time_line.end, time_line.end, String::new()),
        };
        let parsed_timing = timing_line(time_line.text, webvtt);
        let timing_valid = parsed_timing.is_some();
        let (start_ms, end_ms, settings) = parsed_timing.unwrap_or_default();
        if !timing_valid || start_ms.is_none() || end_ms.is_none() || end_ms < start_ms {
            push_diagnostic(
                &mut out.diagnostics,
                malformed(
                    "subtitle.timing.malformed",
                    "cue has malformed, format-incompatible, or reversed timing",
                    time_line.start,
                    time_line.end,
                    index,
                ),
                guard,
            )?;
        }
        let text_locator = locator(text_start, text_end, index);
        let (plain, speaker, runs, payload_diagnostics) = if webvtt {
            vtt_payload(&raw_text, text_start, index, guard)?
        } else {
            let (speaker, plain) = srt_speaker(&raw_text);
            guard.node()?;
            let kind = if speaker.is_some() {
                SubtitleTextRunKind::Voice
            } else {
                SubtitleTextRunKind::Text
            };
            (
                plain.clone(),
                speaker.clone(),
                vec![SubtitleTextRun {
                    kind,
                    text: plain,
                    speaker,
                    style: None,
                    locator: text_locator.clone(),
                }],
                Vec::new(),
            )
        };
        out.diagnostics.extend(payload_diagnostics);
        let region_id = settings.get("region").cloned();
        if webvtt
            && region_id
                .as_ref()
                .is_some_and(|id| !out.regions.iter().any(|region| &region.id == id))
        {
            push_diagnostic(
                &mut out.diagnostics,
                malformed(
                    "subtitle.webvtt.unknown_region",
                    "WebVTT cue refers to an undeclared region",
                    time_line.start,
                    time_line.end,
                    index,
                ),
                guard,
            )?;
        }
        guard.record()?;
        guard.node()?;
        let cue = SubtitleCue {
            id: format!("cue-{source_index:06}"),
            source_index,
            source_id,
            track_id: "track-0".into(),
            region_id,
            timing: SubtitleTiming {
                raw: time_line.text.into(),
                start_ms,
                end_ms,
                locator: time_locator(start_ms, end_ms, 0),
            },
            settings,
            speaker,
            text: plain,
            raw_text,
            runs,
            locator: locator(first.start, last.end, index),
            text_locator,
            structure_locator: None,
        };
        if webvtt {
            guard.retain(cue_shell_memory(&cue))?;
        } else {
            guard.retain(cue_memory(&cue))?;
        }
        out.cues.push(cue);
    }
    Ok(out)
}

fn next_block<'a>(
    lines: &mut PhysicalLines<'a>,
    guard: &Guard<'_>,
) -> ParseResult<Option<Vec<Line<'a>>>> {
    let mut block = Vec::new();
    let mut bytes = 0usize;
    while let Some(line) = lines.next_checked(guard)? {
        guard.checkpoint()?;
        if line.text.trim().is_empty() {
            if block.is_empty() {
                continue;
            }
            break;
        }
        bytes = bytes.saturating_add(line.end.saturating_sub(line.start));
        guard.memory(bytes)?;
        block.push(line);
    }
    Ok((!block.is_empty()).then_some(block))
}

fn valid_webvtt_header(value: &str) -> bool {
    let value = value.strip_prefix('\u{feff}').unwrap_or(value);
    value == "WEBVTT"
        || value.strip_prefix("WEBVTT").is_some_and(|tail| {
            (tail.starts_with(' ') || tail.starts_with('\t')) && !tail.contains("-->")
        })
}

fn is_webvtt_note(value: &str) -> bool {
    let value = value.trim_end();
    value == "NOTE"
        || value
            .strip_prefix("NOTE")
            .is_some_and(|tail| tail.starts_with(' ') || tail.starts_with('\t'))
}

#[derive(Clone)]
struct VttContext {
    name: String,
    speaker: Option<String>,
    style: Option<String>,
    kind: SubtitleTextRunKind,
}

type VttPayload = (
    String,
    Option<String>,
    Vec<SubtitleTextRun>,
    Vec<Diagnostic>,
);

fn vtt_payload(
    raw: &str,
    absolute_start: usize,
    index: &LineIndex,
    guard: &Guard<'_>,
) -> ParseResult<VttPayload> {
    let mut plain = String::new();
    let mut runs = Vec::new();
    let mut diagnostics = Vec::new();
    let mut first_speaker = None;
    let mut contexts = Vec::<VttContext>::new();
    let mut cursor = 0usize;
    while cursor < raw.len() {
        guard.checkpoint()?;
        if raw.as_bytes()[cursor] == b'<' {
            let Some(relative_close) = raw[cursor..].find('>') else {
                push_diagnostic(
                    &mut diagnostics,
                    malformed(
                        "subtitle.webvtt.unclosed_tag",
                        "WebVTT cue contains an unclosed tag",
                        absolute_start + cursor,
                        absolute_start + raw.len(),
                        index,
                    ),
                    guard,
                )?;
                push_vtt_run(
                    &mut runs,
                    SubtitleTextRunKind::RawMarkup,
                    &raw[cursor..],
                    &contexts,
                    absolute_start + cursor,
                    absolute_start + raw.len(),
                    index,
                    guard,
                )?;
                break;
            };
            let close = cursor + relative_close;
            let raw_tag = &raw[cursor..=close];
            let tag = raw[cursor + 1..close].trim();
            if let Some(name) = tag.strip_prefix('/') {
                let name = name.trim();
                if contexts.last().is_some_and(|context| context.name == name) {
                    contexts.pop();
                } else {
                    push_diagnostic(
                        &mut diagnostics,
                        malformed(
                            "subtitle.webvtt.mismatched_tag",
                            "WebVTT cue contains a mismatched closing tag",
                            absolute_start + cursor,
                            absolute_start + close + 1,
                            index,
                        ),
                        guard,
                    )?;
                    if let Some(position) =
                        contexts.iter().rposition(|context| context.name == name)
                    {
                        contexts.truncate(position);
                    }
                }
            } else if vtt_clock(tag).is_some() {
                push_vtt_run(
                    &mut runs,
                    SubtitleTextRunKind::Timestamp,
                    tag,
                    &contexts,
                    absolute_start + cursor,
                    absolute_start + close + 1,
                    index,
                    guard,
                )?;
            } else if tag.eq_ignore_ascii_case("br") {
                guard.retain(1)?;
                plain.push('\n');
                push_vtt_run(
                    &mut runs,
                    SubtitleTextRunKind::LineBreak,
                    "\n",
                    &contexts,
                    absolute_start + cursor,
                    absolute_start + close + 1,
                    index,
                    guard,
                )?;
            } else if let Some(context) = vtt_context(tag, guard)? {
                if first_speaker.is_none() {
                    guard.retain(context.speaker.as_ref().map_or(0, String::len))?;
                    first_speaker = context.speaker.clone();
                }
                contexts.push(context);
            } else {
                push_diagnostic(
                    &mut diagnostics,
                    unsupported(
                        "subtitle.webvtt.unknown_tag",
                        "unknown WebVTT cue markup was retained inertly",
                        absolute_start + cursor,
                        absolute_start + close + 1,
                        index,
                    ),
                    guard,
                )?;
                push_vtt_run(
                    &mut runs,
                    SubtitleTextRunKind::RawMarkup,
                    raw_tag,
                    &contexts,
                    absolute_start + cursor,
                    absolute_start + close + 1,
                    index,
                    guard,
                )?;
            }
            cursor = close + 1;
            continue;
        }
        let next = raw[cursor..]
            .find('<')
            .map_or(raw.len(), |relative| cursor + relative);
        let value = decode_entities(&raw[cursor..next], guard)?;
        guard.retain(value.len())?;
        plain.push_str(&value);
        if !value.is_empty() {
            let kind = contexts
                .last()
                .map_or(SubtitleTextRunKind::Text, |context| context.kind);
            push_vtt_run(
                &mut runs,
                kind,
                &value,
                &contexts,
                absolute_start + cursor,
                absolute_start + next,
                index,
                guard,
            )?;
        }
        cursor = next;
    }
    if !contexts.is_empty() {
        push_diagnostic(
            &mut diagnostics,
            malformed(
                "subtitle.webvtt.unclosed_tag",
                "WebVTT cue contains unclosed nested markup",
                absolute_start,
                absolute_start + raw.len(),
                index,
            ),
            guard,
        )?;
    }
    if runs.is_empty() {
        guard.node()?;
        guard.retain(std::mem::size_of::<SubtitleTextRun>().saturating_add(plain.len()))?;
        runs.push(SubtitleTextRun {
            kind: SubtitleTextRunKind::Text,
            text: plain.clone(),
            speaker: None,
            style: None,
            locator: locator(absolute_start, absolute_start + raw.len(), index),
        });
    }
    Ok((plain, first_speaker, runs, diagnostics))
}

#[allow(clippy::too_many_arguments)]
fn push_vtt_run(
    runs: &mut Vec<SubtitleTextRun>,
    kind: SubtitleTextRunKind,
    text: &str,
    contexts: &[VttContext],
    start: usize,
    end: usize,
    index: &LineIndex,
    guard: &Guard<'_>,
) -> ParseResult<()> {
    guard.node()?;
    let speaker = active_speaker(contexts);
    let style_len = active_style_len(contexts);
    guard.retain(
        std::mem::size_of::<SubtitleTextRun>()
            .saturating_add(text.len())
            .saturating_add(speaker.map_or(0, str::len))
            .saturating_add(style_len),
    )?;
    runs.push(SubtitleTextRun {
        kind,
        text: text.into(),
        speaker: speaker.map(str::to_owned),
        style: active_style(contexts, style_len),
        locator: locator(start, end, index),
    });
    Ok(())
}

fn vtt_context(tag: &str, guard: &Guard<'_>) -> ParseResult<Option<VttContext>> {
    let (head, tail) = tag
        .split_once(char::is_whitespace)
        .map_or((tag, ""), |(head, tail)| (head, tail.trim()));
    let name = head.split('.').next().unwrap_or(head);
    let (speaker, style, language, kind) = match name {
        "v" if !tail.is_empty() => (Some(tail), None, None, SubtitleTextRunKind::Voice),
        "c" => (
            None,
            head.strip_prefix("c."),
            None,
            SubtitleTextRunKind::Span,
        ),
        "ruby" | "rt" => (None, Some(name), None, SubtitleTextRunKind::Ruby),
        "b" | "i" | "u" => (None, Some(name), None, SubtitleTextRunKind::Span),
        "lang" if !tail.is_empty() => (None, None, Some(tail), SubtitleTextRunKind::Span),
        _ => return Ok(None),
    };
    let style_len = style.map_or(0, str::len).saturating_add(
        language.map_or(0, |language| "lang:".len().saturating_add(language.len())),
    );
    guard.retain(
        std::mem::size_of::<VttContext>()
            .saturating_add(name.len())
            .saturating_add(speaker.map_or(0, str::len))
            .saturating_add(style_len),
    )?;
    Ok(Some(VttContext {
        name: name.into(),
        speaker: speaker.map(str::to_owned),
        style: style
            .map(str::to_owned)
            .or_else(|| language.map(|language| format!("lang:{language}"))),
        kind,
    }))
}

fn active_speaker(contexts: &[VttContext]) -> Option<&str> {
    contexts
        .iter()
        .rev()
        .find_map(|context| context.speaker.as_deref())
}

fn active_style_len(contexts: &[VttContext]) -> usize {
    let mut count = 0usize;
    let bytes = contexts
        .iter()
        .filter_map(|context| context.style.as_deref())
        .fold(0usize, |bytes, style| {
            count = count.saturating_add(1);
            bytes.saturating_add(style.len())
        });
    bytes.saturating_add(count.saturating_sub(1))
}

fn active_style(contexts: &[VttContext], capacity: usize) -> Option<String> {
    let mut output = String::with_capacity(capacity);
    for style in contexts
        .iter()
        .filter_map(|context| context.style.as_deref())
    {
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(style);
    }
    (!output.is_empty()).then_some(output)
}

#[derive(Debug, Clone, Copy, Default)]
struct ResolvedTiming {
    start_ms: Option<u64>,
    end_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TimeContainer {
    #[default]
    Par,
    Seq,
}

#[derive(Clone, Default)]
struct TtmlAttributes {
    raw: BTreeMap<String, String>,
    resolved: BTreeMap<(Option<String>, String), String>,
}

struct TtmlContext {
    name: String,
    native: bool,
    path: String,
    child_counts: BTreeMap<String, u64>,
    style: Option<String>,
    speaker: Option<String>,
    language: Option<String>,
    timing: ResolvedTiming,
    container: TimeContainer,
    sequential_cursor_ms: Option<u64>,
}

struct TtmlCue {
    start: usize,
    attrs: TtmlAttributes,
    source_index: usize,
    language: Option<String>,
    timing: ResolvedTiming,
    path: String,
    runs: Vec<SubtitleTextRun>,
    text: String,
    text_start: Option<usize>,
    text_end: usize,
    speaker: Option<String>,
}

impl TtmlCue {
    fn new(
        start: usize,
        attrs: TtmlAttributes,
        source_index: usize,
        language: Option<String>,
        timing: ResolvedTiming,
        path: String,
    ) -> Self {
        Self {
            start,
            attrs,
            source_index,
            language,
            timing,
            path,
            runs: Vec::new(),
            text: String::new(),
            text_start: None,
            text_end: start,
            speaker: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        text: String,
        kind: SubtitleTextRunKind,
        speaker: Option<String>,
        style: Option<String>,
        locator: SourceLocator,
        start: usize,
        end: usize,
        guard: &Guard<'_>,
    ) -> ParseResult<()> {
        guard.node()?;
        guard.retain(
            std::mem::size_of::<SubtitleTextRun>()
                .saturating_add(text.len())
                .saturating_add(text.len())
                .saturating_add(speaker.as_ref().map_or(0, String::len))
                .saturating_add(style.as_ref().map_or(0, String::len)),
        )?;
        if self.speaker.is_none() {
            guard.retain(speaker.as_ref().map_or(0, String::len))?;
            self.speaker = speaker.clone();
        }
        self.text_start.get_or_insert(start);
        self.text_end = end;
        self.text.push_str(&text);
        self.runs.push(SubtitleTextRun {
            kind,
            text,
            speaker,
            style,
            locator,
        });
        Ok(())
    }
}

fn parse_ttml(
    text: &str,
    index: &LineIndex,
    options: &SubtitleOptions,
    guard: &Guard<'_>,
) -> ParseResult<Parsed> {
    let mut out = Parsed::new(text, index, guard)?;
    let mut reader = NsReader::from_str(text);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut previous = 0usize;
    let mut contexts = Vec::<TtmlContext>::new();
    let mut cue: Option<TtmlCue> = None;
    let mut cue_depth = None;
    let mut root_seen = false;
    let mut root_closed = false;
    let mut valid_root = false;
    let mut parameters = TimeParameters::default();
    loop {
        guard.checkpoint()?;
        let event = reader
            .read_resolved_event()
            .map(|(namespace, event)| (native_ttml_namespace(&namespace), event));
        let end = usize::try_from(reader.buffer_position())
            .unwrap_or(text.len())
            .min(text.len());
        match event {
            Ok((namespace, Event::Start(start))) => {
                let depth = contexts.len() + 1;
                guard.depth(depth)?;
                if depth > options.max_nesting_depth {
                    push_diagnostic(
                        &mut out.diagnostics,
                        limit(
                            "subtitle.ttml.nesting",
                            "TTML nesting limit exceeded",
                            previous,
                            end,
                            index,
                        ),
                        guard,
                    )?;
                    break;
                }
                let name = local_name(start.name().as_ref());
                let native = namespace && contexts.iter().all(|context| context.native);
                let attrs = match attributes(&reader, &start, guard)? {
                    Ok(attrs) => attrs,
                    Err(message) => {
                        push_diagnostic(
                            &mut out.diagnostics,
                            malformed("subtitle.ttml.attributes", &message, previous, end, index),
                            guard,
                        )?;
                        break;
                    }
                };
                let path = next_xml_path(&mut contexts, &name, guard)?;
                if depth == 1 {
                    if root_seen {
                        push_diagnostic(
                            &mut out.diagnostics,
                            malformed(
                                "subtitle.ttml.multiple_roots",
                                "TTML input contains more than one root element",
                                previous,
                                end,
                                index,
                            ),
                            guard,
                        )?;
                    }
                    root_seen = true;
                    valid_root = name == "tt" && native;
                    if !valid_root {
                        push_diagnostic(
                            &mut out.diagnostics,
                            malformed(
                                "subtitle.ttml.root",
                                "TTML requires a tt root in a recognized TTML namespace",
                                previous,
                                end,
                                index,
                            ),
                            guard,
                        )?;
                    }
                    parameters = root_time_parameters(
                        &attrs,
                        &mut out.metadata,
                        &mut out.diagnostics,
                        previous,
                        end,
                        index,
                        guard,
                    )?;
                    out.tracks[0].language = attr(&attrs, "lang").cloned();
                } else if root_closed {
                    push_diagnostic(
                        &mut out.diagnostics,
                        malformed(
                            "subtitle.ttml.multiple_roots",
                            "TTML input contains content after the root element",
                            previous,
                            end,
                            index,
                        ),
                        guard,
                    )?;
                }
                let timing = if native {
                    match resolve_timing(&attrs, contexts.last(), &parameters) {
                        Ok(timing) => timing,
                        Err(message) => {
                            push_diagnostic(
                                &mut out.diagnostics,
                                malformed(
                                    "subtitle.timing.malformed",
                                    &message,
                                    previous,
                                    end,
                                    index,
                                ),
                                guard,
                            )?;
                            ResolvedTiming::default()
                        }
                    }
                } else {
                    contexts
                        .last()
                        .map_or(ResolvedTiming::default(), |context| context.timing)
                };
                let inherited_language =
                    contexts.last().and_then(|context| context.language.clone());
                let inherited_style = contexts.last().and_then(|context| context.style.clone());
                let inherited_speaker = contexts.last().and_then(|context| context.speaker.clone());
                let language = if native {
                    attr(&attrs, "lang").cloned().or(inherited_language)
                } else {
                    inherited_language
                };
                let style = if native {
                    attr(&attrs, "style").cloned().or(inherited_style)
                } else {
                    inherited_style
                };
                let speaker = if native {
                    attr(&attrs, "agent").cloned().or(inherited_speaker)
                } else {
                    inherited_speaker
                };
                let container = if native
                    && attr(&attrs, "timeContainer").is_some_and(|value| value == "seq")
                {
                    TimeContainer::Seq
                } else {
                    TimeContainer::Par
                };
                if valid_root && native {
                    handle_ttml_declaration(
                        &name,
                        &attrs,
                        text,
                        previous,
                        end,
                        index,
                        language.clone(),
                        options,
                        &mut out,
                        guard,
                    )?;
                    if name == "p" {
                        if cue.is_some() {
                            push_diagnostic(
                                &mut out.diagnostics,
                                malformed(
                                    "subtitle.ttml.nested_cue",
                                    "TTML p elements cannot be nested",
                                    previous,
                                    end,
                                    index,
                                ),
                                guard,
                            )?;
                        } else if out.cues.len() >= options.max_cues {
                            push_diagnostic(
                                &mut out.diagnostics,
                                limit(
                                    "subtitle.limit.cues",
                                    "TTML cue limit exceeded",
                                    previous,
                                    end,
                                    index,
                                ),
                                guard,
                            )?;
                        } else {
                            guard.retain(
                                ttml_attributes_memory(&attrs).saturating_add(path.len()),
                            )?;
                            cue = Some(TtmlCue::new(
                                previous,
                                attrs.clone(),
                                out.cues.len(),
                                language.clone(),
                                timing,
                                path.clone(),
                            ));
                            cue_depth = Some(depth);
                        }
                    }
                }
                guard.retain(
                    std::mem::size_of::<TtmlContext>()
                        .saturating_add(name.len())
                        .saturating_add(style.as_ref().map_or(0, String::len))
                        .saturating_add(speaker.as_ref().map_or(0, String::len))
                        .saturating_add(language.as_ref().map_or(0, String::len)),
                )?;
                contexts.push(TtmlContext {
                    name,
                    native,
                    path,
                    child_counts: BTreeMap::new(),
                    style,
                    speaker,
                    language,
                    timing,
                    container,
                    sequential_cursor_ms: timing.start_ms,
                });
            }
            Ok((namespace, Event::Empty(start))) => {
                let depth = contexts.len() + 1;
                guard.depth(depth)?;
                if depth > options.max_nesting_depth {
                    push_diagnostic(
                        &mut out.diagnostics,
                        limit(
                            "subtitle.ttml.nesting",
                            "TTML nesting limit exceeded",
                            previous,
                            end,
                            index,
                        ),
                        guard,
                    )?;
                    break;
                }
                let name = local_name(start.name().as_ref());
                let native = namespace && contexts.iter().all(|context| context.native);
                let attrs = match attributes(&reader, &start, guard)? {
                    Ok(attrs) => attrs,
                    Err(message) => {
                        push_diagnostic(
                            &mut out.diagnostics,
                            malformed("subtitle.ttml.attributes", &message, previous, end, index),
                            guard,
                        )?;
                        break;
                    }
                };
                let path = next_xml_path(&mut contexts, &name, guard)?;
                if depth == 1 {
                    if root_seen {
                        push_diagnostic(
                            &mut out.diagnostics,
                            malformed(
                                "subtitle.ttml.multiple_roots",
                                "TTML input contains more than one root element",
                                previous,
                                end,
                                index,
                            ),
                            guard,
                        )?;
                    }
                    root_seen = true;
                    root_closed = true;
                    valid_root = name == "tt" && native;
                    if !valid_root {
                        push_diagnostic(
                            &mut out.diagnostics,
                            malformed(
                                "subtitle.ttml.root",
                                "TTML requires a tt root in a recognized TTML namespace",
                                previous,
                                end,
                                index,
                            ),
                            guard,
                        )?;
                    }
                    parameters = root_time_parameters(
                        &attrs,
                        &mut out.metadata,
                        &mut out.diagnostics,
                        previous,
                        end,
                        index,
                        guard,
                    )?;
                    out.tracks[0].language = attr(&attrs, "lang").cloned();
                }
                let timing = if native {
                    match resolve_timing(&attrs, contexts.last(), &parameters) {
                        Ok(timing) => timing,
                        Err(message) => {
                            push_diagnostic(
                                &mut out.diagnostics,
                                malformed(
                                    "subtitle.timing.malformed",
                                    &message,
                                    previous,
                                    end,
                                    index,
                                ),
                                guard,
                            )?;
                            ResolvedTiming::default()
                        }
                    }
                } else {
                    contexts
                        .last()
                        .map_or(ResolvedTiming::default(), |context| context.timing)
                };
                let inherited_language =
                    contexts.last().and_then(|context| context.language.clone());
                let language = if native {
                    attr(&attrs, "lang").cloned().or(inherited_language)
                } else {
                    inherited_language
                };
                if valid_root && native {
                    handle_ttml_declaration(
                        &name,
                        &attrs,
                        text,
                        previous,
                        end,
                        index,
                        language.clone(),
                        options,
                        &mut out,
                        guard,
                    )?;
                    if name == "br" {
                        if let Some(cue) = cue.as_mut() {
                            cue.push(
                                "\n".into(),
                                SubtitleTextRunKind::LineBreak,
                                current_speaker(&contexts),
                                current_style(&contexts),
                                locator(previous, end, index),
                                previous,
                                end,
                                guard,
                            )?;
                        }
                    } else if name == "p" {
                        if out.cues.len() >= options.max_cues {
                            push_diagnostic(
                                &mut out.diagnostics,
                                limit(
                                    "subtitle.limit.cues",
                                    "TTML cue limit exceeded",
                                    previous,
                                    end,
                                    index,
                                ),
                                guard,
                            )?;
                        } else {
                            let cue = TtmlCue::new(
                                previous,
                                attrs,
                                out.cues.len(),
                                language,
                                timing,
                                path,
                            );
                            finish_ttml(cue, end, text, index, &mut out, guard)?;
                        }
                    }
                }
                update_sequential_parent(&mut contexts, timing);
            }
            Ok((_, Event::Text(value))) => {
                if cue_depth.is_some_and(|depth| contexts.len() >= depth)
                    && contexts.iter().all(|context| context.native)
                    && let Some(cue) = cue.as_mut()
                {
                    let value = match value.unescape() {
                        Ok(value) => value.into_owned(),
                        Err(error) => {
                            push_diagnostic(
                                &mut out.diagnostics,
                                malformed(
                                    "subtitle.ttml.entity",
                                    &format!("TTML text contains an invalid entity: {error}"),
                                    previous,
                                    end,
                                    index,
                                ),
                                guard,
                            )?;
                            String::from_utf8_lossy(value.as_ref()).into_owned()
                        }
                    };
                    let speaker = current_speaker(&contexts);
                    let kind = if speaker.is_some() {
                        SubtitleTextRunKind::Voice
                    } else if contexts
                        .iter()
                        .any(|context| context.native && context.name == "ruby")
                    {
                        SubtitleTextRunKind::Ruby
                    } else if contexts
                        .iter()
                        .any(|context| context.native && context.name == "span")
                    {
                        SubtitleTextRunKind::Span
                    } else {
                        SubtitleTextRunKind::Text
                    };
                    cue.push(
                        value,
                        kind,
                        speaker,
                        current_style(&contexts),
                        locator(previous, end, index),
                        previous,
                        end,
                        guard,
                    )?;
                }
            }
            Ok((_, Event::CData(value))) => {
                if cue_depth.is_some_and(|depth| contexts.len() >= depth)
                    && contexts.iter().all(|context| context.native)
                    && let Some(cue) = cue.as_mut()
                {
                    let value = reader
                        .decoder()
                        .decode(value.as_ref())
                        .map(|value| value.into_owned())
                        .unwrap_or_default();
                    cue.push(
                        value,
                        SubtitleTextRunKind::Text,
                        current_speaker(&contexts),
                        current_style(&contexts),
                        locator(previous, end, index),
                        previous,
                        end,
                        guard,
                    )?;
                }
            }
            Ok((namespace, Event::End(close))) => {
                let name = local_name(close.name().as_ref());
                let native = namespace && contexts.last().is_some_and(|context| context.native);
                if native && name == "p" && cue_depth == Some(contexts.len()) {
                    if let Some(cue) = cue.take() {
                        finish_ttml(cue, end, text, index, &mut out, guard)?;
                    }
                    cue_depth = None;
                }
                if let Some(context) = contexts.pop() {
                    update_sequential_parent(&mut contexts, context.timing);
                }
                if contexts.is_empty() {
                    root_closed = true;
                }
            }
            Ok((_, Event::DocType(_))) => {
                push_diagnostic(
                    &mut out.diagnostics,
                    security(
                        "subtitle.ttml.doctype",
                        "TTML document type declarations and entities are retained inertly and not resolved",
                        previous,
                        end,
                        index,
                    ),
                    guard,
                )?;
            }
            Ok((_, Event::Eof)) => break,
            Err(error) => {
                push_diagnostic(
                    &mut out.diagnostics,
                    malformed(
                        "subtitle.ttml.xml",
                        &format!("malformed TTML XML: {error}"),
                        previous,
                        end.max(previous),
                        index,
                    ),
                    guard,
                )?;
                break;
            }
            _ => {}
        }
        previous = end;
    }
    if !root_seen {
        push_diagnostic(
            &mut out.diagnostics,
            malformed(
                "subtitle.ttml.root",
                "TTML input has no document root",
                0,
                text.len(),
                index,
            ),
            guard,
        )?;
    } else if !root_closed {
        push_diagnostic(
            &mut out.diagnostics,
            malformed(
                "subtitle.ttml.unclosed_root",
                "TTML document root is not closed",
                0,
                text.len(),
                index,
            ),
            guard,
        )?;
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn handle_ttml_declaration(
    name: &str,
    attrs: &TtmlAttributes,
    source: &str,
    start: usize,
    end: usize,
    index: &LineIndex,
    language: Option<String>,
    options: &SubtitleOptions,
    out: &mut Parsed,
    guard: &Guard<'_>,
) -> ParseResult<()> {
    if name == "style" {
        if out.styles.len() >= options.max_styles {
            push_diagnostic(
                &mut out.diagnostics,
                limit(
                    "subtitle.limit.styles",
                    "TTML style limit exceeded",
                    start,
                    end,
                    index,
                ),
                guard,
            )?;
        } else {
            guard.node()?;
            guard.retain(ttml_style_memory(
                attrs,
                end.saturating_sub(start),
                out.styles.len(),
            ))?;
            let style = ttml_style(attrs, source, start, end, index, out.styles.len());
            out.styles.push(style);
        }
    } else if name == "region" {
        if out.regions.len() >= options.max_regions {
            push_diagnostic(
                &mut out.diagnostics,
                limit(
                    "subtitle.limit.regions",
                    "TTML region limit exceeded",
                    start,
                    end,
                    index,
                ),
                guard,
            )?;
        } else {
            guard.node()?;
            guard.retain(ttml_region_memory(
                attrs,
                out.regions.len(),
                language.as_deref(),
            ))?;
            let region = ttml_region(attrs, start, end, index, out.regions.len(), language);
            out.regions.push(region);
        }
    }
    Ok(())
}

fn finish_ttml(
    mut cue: TtmlCue,
    end: usize,
    source: &str,
    index: &LineIndex,
    out: &mut Parsed,
    guard: &Guard<'_>,
) -> ParseResult<()> {
    let start_ms = cue.timing.start_ms;
    let end_ms = cue.timing.end_ms;
    if start_ms.is_none() || end_ms.is_none() || end_ms < start_ms {
        push_diagnostic(
            &mut out.diagnostics,
            malformed(
                "subtitle.timing.malformed",
                "TTML cue has malformed, missing, or reversed resolved timing",
                cue.start,
                end,
                index,
            ),
            guard,
        )?;
    }
    if cue.runs.is_empty() {
        guard.node()?;
        guard.retain(std::mem::size_of::<SubtitleTextRun>())?;
        cue.runs.push(SubtitleTextRun {
            kind: SubtitleTextRunKind::Text,
            text: String::new(),
            speaker: None,
            style: None,
            locator: locator(end, end, index),
        });
    }
    let raw_begin = attr(&cue.attrs, "begin").cloned().unwrap_or_default();
    let raw_end = attr(&cue.attrs, "end").cloned();
    let duration = attr(&cue.attrs, "dur").cloned();
    let raw = raw_end.as_ref().map_or_else(
        || {
            format!(
                "{raw_begin} + {}",
                duration.as_deref().unwrap_or("inherited")
            )
        },
        |value| format!("{raw_begin} --> {value}"),
    );
    let region_id = attr(&cue.attrs, "region").cloned();
    if region_id
        .as_ref()
        .is_some_and(|id| !out.regions.iter().any(|region| &region.id == id))
    {
        push_diagnostic(
            &mut out.diagnostics,
            malformed(
                "subtitle.ttml.unknown_region",
                "TTML cue refers to an undeclared region",
                cue.start,
                end,
                index,
            ),
            guard,
        )?;
    }
    let text_start = cue.text_start.unwrap_or(end);
    let text_end = cue.text_end.max(text_start);
    let source_id = attr(&cue.attrs, "id").cloned();
    let id = source_id
        .clone()
        .unwrap_or_else(|| format!("cue-{:06}", cue.source_index));
    let settings = cue
        .attrs
        .raw
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "begin" | "end" | "dur" | "id" | "xml:id"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    guard.record()?;
    guard.node()?;
    let parsed_cue = SubtitleCue {
        id,
        source_index: cue.source_index,
        source_id,
        track_id: "track-0".into(),
        region_id,
        timing: SubtitleTiming {
            raw,
            start_ms,
            end_ms,
            locator: time_locator(start_ms, end_ms, 0),
        },
        settings,
        speaker: cue.speaker,
        text: cue.text,
        raw_text: source[text_start.min(source.len())..text_end.min(source.len())].into(),
        runs: cue.runs,
        locator: locator(cue.start, end, index),
        text_locator: locator(text_start, text_end, index),
        structure_locator: xml_locator(cue.path),
    };
    guard.retain(cue_shell_memory(&parsed_cue))?;
    out.cues.push(parsed_cue);
    if let Some(language) = cue.language
        && out.tracks[0].language.is_none()
    {
        out.tracks[0].language = Some(language);
    }
    Ok(())
}

fn update_sequential_parent(contexts: &mut [TtmlContext], timing: ResolvedTiming) {
    if let Some(parent) = contexts.last_mut()
        && parent.container == TimeContainer::Seq
    {
        parent.sequential_cursor_ms = timing.end_ms.or(timing.start_ms);
    }
}

fn next_xml_path(
    contexts: &mut [TtmlContext],
    name: &str,
    guard: &Guard<'_>,
) -> ParseResult<String> {
    let path = if let Some(parent) = contexts.last_mut() {
        let ordinal = parent.child_counts.entry(name.into()).or_insert(0);
        *ordinal = ordinal.saturating_add(1);
        format!("{}/{}[{}]", parent.path, name, *ordinal)
    } else {
        format!("/{name}[1]")
    };
    guard.retain(
        path.len()
            .saturating_add(name.len())
            .saturating_add(std::mem::size_of::<(String, u64)>()),
    )?;
    Ok(path)
}

fn ttml_style(
    attrs: &TtmlAttributes,
    source: &str,
    start: usize,
    end: usize,
    index: &LineIndex,
    ordinal: usize,
) -> SubtitleStyle {
    SubtitleStyle {
        id: attr(attrs, "id")
            .cloned()
            .unwrap_or_else(|| format!("style-{ordinal:06}")),
        selector: attr(attrs, "style").cloned(),
        properties: attrs
            .raw
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "id" | "xml:id"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        raw: source[start..end].into(),
        locator: locator(start, end, index),
    }
}

fn ttml_region(
    attrs: &TtmlAttributes,
    start: usize,
    end: usize,
    index: &LineIndex,
    ordinal: usize,
    language: Option<String>,
) -> SubtitleRegion {
    let mut settings = attrs.raw.clone();
    if let Some(language) = language {
        settings.entry("xml:lang".into()).or_insert(language);
    }
    SubtitleRegion {
        id: attr(attrs, "id")
            .cloned()
            .unwrap_or_else(|| format!("region-{ordinal:06}")),
        settings,
        locator: locator(start, end, index),
    }
}

#[derive(Debug, Clone, Copy)]
struct Rational {
    numerator: u128,
    denominator: u128,
}

impl Rational {
    const fn integer(value: u128) -> Self {
        Self {
            numerator: value,
            denominator: 1,
        }
    }

    fn multiply(self, other: Self) -> Option<Self> {
        Some(Self {
            numerator: self.numerator.checked_mul(other.numerator)?,
            denominator: self.denominator.checked_mul(other.denominator)?,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct TimeParameters {
    frame_rate: Rational,
    sub_frame_rate: u128,
    tick_rate: Rational,
    frame_clock_supported: bool,
    tick_clock_supported: bool,
    time_expressions_supported: bool,
}

enum TimeParameterError {
    Invalid(String),
    UnsupportedTimeBase(String),
}

impl Default for TimeParameters {
    fn default() -> Self {
        Self {
            frame_rate: Rational::integer(30),
            sub_frame_rate: 1,
            tick_rate: Rational::integer(1),
            frame_clock_supported: true,
            tick_clock_supported: true,
            time_expressions_supported: true,
        }
    }
}

impl TimeParameters {
    fn invalid() -> Self {
        Self {
            frame_clock_supported: false,
            tick_clock_supported: false,
            time_expressions_supported: false,
            ..Self::default()
        }
    }

    fn from_root(
        attrs: &TtmlAttributes,
        metadata: &mut BTreeMap<String, String>,
    ) -> Result<Self, TimeParameterError> {
        for key in [
            "frameRate",
            "frameRateMultiplier",
            "subFrameRate",
            "tickRate",
            "timeBase",
            "dropMode",
        ] {
            if let Some(value) = attr(attrs, key) {
                metadata.insert(key.into(), value.clone());
            }
        }
        match attr(attrs, "timeBase").map(String::as_str) {
            None | Some("media") => {}
            Some("clock" | "smpte") => {
                return Err(TimeParameterError::UnsupportedTimeBase(
                    attr(attrs, "timeBase").cloned().unwrap_or_default(),
                ));
            }
            Some(value) => {
                return Err(TimeParameterError::Invalid(format!(
                    "invalid TTML timeBase {value:?}"
                )));
            }
        }
        let base = match attr(attrs, "frameRate") {
            Some(value) => Rational::integer(
                value
                    .parse::<u128>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        TimeParameterError::Invalid(format!("invalid TTML frameRate {value:?}"))
                    })?,
            ),
            None => Rational::integer(30),
        };
        let multiplier = match attr(attrs, "frameRateMultiplier") {
            Some(value) => parse_ratio_pair(value)
                .filter(|value| value.numerator > 0)
                .ok_or_else(|| {
                    TimeParameterError::Invalid(format!(
                        "invalid TTML frameRateMultiplier {value:?}"
                    ))
                })?,
            None => Rational::integer(1),
        };
        let frame_rate = base
            .multiply(multiplier)
            .filter(|value| value.numerator > 0 && value.denominator > 0)
            .ok_or_else(|| {
                TimeParameterError::Invalid("TTML effective frame rate overflows".into())
            })?;
        let sub_frame_rate = match attr(attrs, "subFrameRate") {
            Some(value) => value
                .parse::<u128>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    TimeParameterError::Invalid(format!("invalid TTML subFrameRate {value:?}"))
                })?,
            None => 1,
        };
        let default_tick = frame_rate
            .multiply(Rational::integer(sub_frame_rate))
            .ok_or_else(|| {
                TimeParameterError::Invalid("TTML default tick rate overflows".into())
            })?;
        let tick_rate = match attr(attrs, "tickRate") {
            Some(value) => Rational::integer(
                value
                    .parse::<u128>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        TimeParameterError::Invalid(format!("invalid TTML tickRate {value:?}"))
                    })?,
            ),
            None => default_tick,
        };
        let frame_clock_supported = match attr(attrs, "dropMode").map(String::as_str) {
            None | Some("nonDrop") => true,
            Some("dropNTSC" | "dropPAL") => false,
            Some(value) => {
                return Err(TimeParameterError::Invalid(format!(
                    "invalid TTML dropMode {value:?}"
                )));
            }
        };
        Ok(Self {
            frame_rate,
            sub_frame_rate,
            tick_rate,
            frame_clock_supported,
            tick_clock_supported: true,
            time_expressions_supported: true,
        })
    }
}

fn root_time_parameters(
    attrs: &TtmlAttributes,
    metadata: &mut BTreeMap<String, String>,
    diagnostics: &mut Vec<Diagnostic>,
    start: usize,
    end: usize,
    index: &LineIndex,
    guard: &Guard<'_>,
) -> ParseResult<TimeParameters> {
    match TimeParameters::from_root(attrs, metadata) {
        Ok(parameters) if parameters.frame_clock_supported => Ok(parameters),
        Ok(parameters) => {
            push_diagnostic(
                diagnostics,
                malformed(
                    "subtitle.ttml.drop_frame_unsupported",
                    "TTML drop-frame clock times are retained but not interpreted",
                    start,
                    end,
                    index,
                ),
                guard,
            )?;
            Ok(parameters)
        }
        Err(TimeParameterError::UnsupportedTimeBase(time_base)) => {
            push_diagnostic(
                diagnostics,
                unsupported(
                    "subtitle.ttml.time_base_unsupported",
                    &format!(
                        "TTML timeBase {time_base:?} is retained but unsupported for standalone media-time interpretation"
                    ),
                    start,
                    end,
                    index,
                ),
                guard,
            )?;
            Ok(TimeParameters::invalid())
        }
        Err(TimeParameterError::Invalid(message)) => {
            push_diagnostic(
                diagnostics,
                malformed("subtitle.ttml.time_parameters", &message, start, end, index),
                guard,
            )?;
            Ok(TimeParameters::invalid())
        }
    }
}

fn resolve_timing(
    attrs: &TtmlAttributes,
    parent: Option<&TtmlContext>,
    parameters: &TimeParameters,
) -> Result<ResolvedTiming, String> {
    let parent_start = parent
        .and_then(|parent| parent.timing.start_ms)
        .unwrap_or(0);
    let implicit_start = parent.map_or(0, |parent| {
        if parent.container == TimeContainer::Seq {
            parent.sequential_cursor_ms.unwrap_or(parent_start)
        } else {
            parent_start
        }
    });
    let start_ms = match attr(attrs, "begin") {
        Some(value) => Some(
            implicit_start
                .checked_add(
                    ttml_time(value, parameters)
                        .ok_or_else(|| format!("invalid TTML begin time {value:?}"))?,
                )
                .ok_or_else(|| "TTML begin time overflows milliseconds".to_string())?,
        ),
        None => Some(implicit_start),
    };
    let end_ms = if let Some(value) = attr(attrs, "end") {
        Some(
            parent_start
                .checked_add(
                    ttml_time(value, parameters)
                        .ok_or_else(|| format!("invalid TTML end time {value:?}"))?,
                )
                .ok_or_else(|| "TTML end time overflows milliseconds".to_string())?,
        )
    } else if let Some(value) = attr(attrs, "dur") {
        Some(
            start_ms
                .expect("resolved TTML start exists")
                .checked_add(
                    ttml_time(value, parameters)
                        .ok_or_else(|| format!("invalid TTML duration {value:?}"))?,
                )
                .ok_or_else(|| "TTML duration overflows milliseconds".to_string())?,
        )
    } else {
        parent.and_then(|parent| parent.timing.end_ms)
    };
    if end_ms.is_some_and(|end| end < start_ms.unwrap_or(0)) {
        return Err("TTML resolved end precedes resolved begin".into());
    }
    Ok(ResolvedTiming { start_ms, end_ms })
}

type ParsedTimingLine = (Option<u64>, Option<u64>, BTreeMap<String, String>);

fn timing_line(value: &str, webvtt: bool) -> Option<ParsedTimingLine> {
    let (start, rest) = value.split_once("-->")?;
    if rest.contains("-->") {
        return None;
    }
    let mut pieces = rest.split_whitespace();
    let end_token = pieces.next()?;
    let start_ms = if webvtt {
        vtt_clock(start.trim())
    } else {
        srt_clock(start.trim())
    };
    let end_ms = if webvtt {
        vtt_clock(end_token)
    } else {
        srt_clock(end_token)
    };
    let mut settings = BTreeMap::new();
    for piece in pieces {
        let (key, value) = piece.split_once(':')?;
        if key.is_empty() || value.is_empty() || settings.insert(key.into(), value.into()).is_some()
        {
            return None;
        }
    }
    Some((start_ms, end_ms, settings))
}

fn srt_clock(value: &str) -> Option<u64> {
    strict_clock(value, ',', false)
}

fn vtt_clock(value: &str) -> Option<u64> {
    strict_clock(value, '.', true)
}

fn strict_clock(value: &str, separator: char, allow_short: bool) -> Option<u64> {
    let (clock, millis) = value.split_once(separator)?;
    if millis.len() != 3 || !millis.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let parts = clock.split(':').collect::<Vec<_>>();
    if parts.len() != 3 && !(allow_short && parts.len() == 2) {
        return None;
    }
    let (hours, minutes, seconds) = if parts.len() == 3 {
        if parts[0].len() < 2 || parts[1].len() != 2 || parts[2].len() != 2 {
            return None;
        }
        (parts[0].parse::<u64>().ok()?, parts[1], parts[2])
    } else {
        if parts[0].len() != 2 || parts[1].len() != 2 {
            return None;
        }
        (0, parts[0], parts[1])
    };
    let minutes = minutes.parse::<u64>().ok()?;
    let seconds = seconds.parse::<u64>().ok()?;
    let millis = millis.parse::<u64>().ok()?;
    if minutes > 59 || seconds > 59 {
        return None;
    }
    hours
        .checked_mul(3_600_000)?
        .checked_add(minutes.checked_mul(60_000)?)?
        .checked_add(seconds.checked_mul(1_000)?)?
        .checked_add(millis)
}

fn ttml_time(value: &str, parameters: &TimeParameters) -> Option<u64> {
    if !parameters.time_expressions_supported {
        return None;
    }
    let value = value.trim();
    if value.contains(':') {
        return ttml_clock(value, parameters);
    }
    for (suffix, factor) in [("ms", 1u128), ("h", 3_600_000), ("m", 60_000), ("s", 1_000)] {
        if let Some(number) = value.strip_suffix(suffix) {
            let value = parse_decimal(number)?;
            return rational_millis(value, factor);
        }
    }
    if let Some(number) = value.strip_suffix('f') {
        if !parameters.frame_clock_supported {
            return None;
        }
        let frames = parse_decimal(number)?;
        return rate_units_to_millis(frames, parameters.frame_rate);
    }
    if let Some(number) = value.strip_suffix('t') {
        if !parameters.tick_clock_supported {
            return None;
        }
        let ticks = parse_decimal(number)?;
        return rate_units_to_millis(ticks, parameters.tick_rate);
    }
    None
}

fn ttml_clock(value: &str, parameters: &TimeParameters) -> Option<u64> {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    if parts[0].len() < 2 || parts[1].len() != 2 {
        return None;
    }
    let hours = parts[0].parse::<u64>().ok()?;
    let minutes = parts[1].parse::<u64>().ok()?;
    if minutes > 59 {
        return None;
    }
    let seconds = if parts.len() == 3 {
        parse_decimal(parts[2])?
    } else {
        if parts[2].len() != 2 {
            return None;
        }
        Rational::integer(parts[2].parse::<u128>().ok()?)
    };
    if seconds.numerator >= seconds.denominator.checked_mul(60)? {
        return None;
    }
    let base = hours
        .checked_mul(3_600_000)?
        .checked_add(minutes.checked_mul(60_000)?)?
        .checked_add(rational_millis(seconds, 1_000)?)?;
    if parts.len() == 3 {
        return Some(base);
    }
    if !parameters.frame_clock_supported {
        return None;
    }
    let frames = parse_frame_component(parts[3], parameters.sub_frame_rate)?;
    if frames
        .numerator
        .checked_mul(parameters.frame_rate.denominator)?
        >= parameters
            .frame_rate
            .numerator
            .checked_mul(frames.denominator)?
    {
        return None;
    }
    base.checked_add(rate_units_to_millis(frames, parameters.frame_rate)?)
}

fn parse_frame_component(value: &str, sub_frame_rate: u128) -> Option<Rational> {
    let (frames, sub_frames) = value
        .split_once('.')
        .map_or((value, None), |(frames, sub)| (frames, Some(sub)));
    if frames.is_empty()
        || !frames.bytes().all(|byte| byte.is_ascii_digit())
        || sub_frames.is_some_and(|value| {
            value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    let frames = frames.parse::<u128>().ok()?;
    let sub_frames = match sub_frames {
        Some(value) => value.parse::<u128>().ok()?,
        None => 0,
    };
    if sub_frame_rate == 0 || sub_frames >= sub_frame_rate {
        return None;
    }
    Some(Rational {
        numerator: frames
            .checked_mul(sub_frame_rate)?
            .checked_add(sub_frames)?,
        denominator: sub_frame_rate,
    })
}

fn parse_decimal(value: &str) -> Option<Rational> {
    if value.is_empty() || value.starts_with('+') || value.starts_with('-') {
        return None;
    }
    let (whole, fraction) = match value.split_once('.') {
        Some((_, "")) => return None,
        Some((whole, fraction)) => (whole, fraction),
        None => (value, ""),
    };
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 18
    {
        return None;
    }
    let denominator = 10u128.checked_pow(fraction.len() as u32)?;
    let whole = whole.parse::<u128>().ok()?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u128>().ok()?
    };
    Some(Rational {
        numerator: whole.checked_mul(denominator)?.checked_add(fraction)?,
        denominator,
    })
}

fn parse_ratio_pair(value: &str) -> Option<Rational> {
    let mut parts = value.split_whitespace();
    let numerator = parts.next()?.parse::<u128>().ok()?;
    let denominator = parts.next()?.parse::<u128>().ok()?;
    (parts.next().is_none() && denominator > 0).then_some(Rational {
        numerator,
        denominator,
    })
}

fn rational_millis(value: Rational, factor: u128) -> Option<u64> {
    divide_round(value.numerator.checked_mul(factor)?, value.denominator)
}

fn rate_units_to_millis(units: Rational, rate: Rational) -> Option<u64> {
    if rate.numerator == 0 || units.denominator == 0 || rate.denominator == 0 {
        return None;
    }
    let numerator = units
        .numerator
        .checked_mul(1_000)?
        .checked_mul(rate.denominator)?;
    let denominator = units.denominator.checked_mul(rate.numerator)?;
    divide_round(numerator, denominator)
}

fn divide_round(numerator: u128, denominator: u128) -> Option<u64> {
    if denominator == 0 {
        return None;
    }
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let rounded = if remainder >= denominator / 2 + denominator % 2 {
        quotient.checked_add(1)?
    } else {
        quotient
    };
    u64::try_from(rounded).ok()
}

fn diagnose_overlaps(
    cues: &[SubtitleCue],
    diagnostics: &mut Vec<Diagnostic>,
    guard: &Guard<'_>,
) -> ParseResult<()> {
    let mut ordered = cues.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| cue_time_order(left, right));
    let mut ends = BTreeMap::<String, (u64, String)>::new();
    for cue in ordered {
        guard.checkpoint()?;
        let (Some(start), Some(end)) = (cue.timing.start_ms, cue.timing.end_ms) else {
            continue;
        };
        if let Some((previous_end, previous_id)) = ends.get(&cue.track_id)
            && start < *previous_end
        {
            let mut diagnostic = Diagnostic::warning(
                PARSER,
                "subtitle.timing.overlap",
                format!(
                    "cue {} overlaps cue {} on track {}",
                    cue.id, previous_id, cue.track_id
                ),
            )
            .partial()
            .with_affected_ids(vec![previous_id.clone(), cue.id.clone()]);
            diagnostic.class = DiagnosticClass::MalformedInput;
            if let Some(locator) = cue.timing.locator.clone() {
                diagnostic = diagnostic.with_locator(locator);
            }
            push_diagnostic(diagnostics, diagnostic, guard)?;
        }
        if ends
            .get(&cue.track_id)
            .is_none_or(|(previous_end, _)| end > *previous_end)
        {
            ends.insert(cue.track_id.clone(), (end, cue.id.clone()));
        }
    }
    Ok(())
}

fn transcript(cues: &[SubtitleCue], guard: &Guard<'_>) -> ParseResult<TranscriptProjection> {
    guard.retain(
        cues.len()
            .saturating_mul(std::mem::size_of::<&SubtitleCue>()),
    )?;
    let mut ordered = cues.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| cue_time_order(left, right));
    let mut entries = Vec::new();
    for cue in ordered {
        guard.checkpoint()?;
        let (Some(start_ms), Some(end_ms), Some(time_locator)) = (
            cue.timing.start_ms,
            cue.timing.end_ms,
            cue.timing.locator.clone(),
        ) else {
            continue;
        };
        guard.retain(transcript_entry_memory_from_cue(cue))?;
        let entry = TranscriptEntry {
            cue_id: cue.id.clone(),
            source_index: cue.source_index,
            track_id: cue.track_id.clone(),
            start_ms,
            end_ms,
            speaker: cue.speaker.clone(),
            text: cue.text.clone(),
            text_locator: cue.text_locator.clone(),
            time_locator,
        };
        entries.push(entry);
    }
    let rendered_bytes = entries.iter().fold(0usize, |total, entry| {
        total.saturating_add(entry.text.len()).saturating_add(
            entry
                .speaker
                .as_ref()
                .map_or(0, |speaker| speaker.len().saturating_add(2)),
        )
    });
    let separators = entries.len().saturating_sub(1);
    let joined_bytes = rendered_bytes.saturating_add(separators);
    guard.retain(
        entries
            .len()
            .saturating_mul(std::mem::size_of::<String>())
            .saturating_add(rendered_bytes),
    )?;
    let rendered = entries
        .iter()
        .map(|entry| {
            entry.speaker.as_ref().map_or_else(
                || entry.text.clone(),
                |speaker| format!("{speaker}: {}", entry.text),
            )
        })
        .collect::<Vec<_>>();
    guard.retain(joined_bytes)?;
    let text = rendered.join("\n");
    Ok(TranscriptProjection { text, entries })
}

fn cue_time_order(left: &SubtitleCue, right: &SubtitleCue) -> std::cmp::Ordering {
    left.timing
        .start_ms
        .unwrap_or(u64::MAX)
        .cmp(&right.timing.start_ms.unwrap_or(u64::MAX))
        .then_with(|| {
            left.timing
                .end_ms
                .unwrap_or(u64::MAX)
                .cmp(&right.timing.end_ms.unwrap_or(u64::MAX))
        })
        .then_with(|| left.track_id.cmp(&right.track_id))
        .then_with(|| left.source_index.cmp(&right.source_index))
        .then_with(|| left.id.cmp(&right.id))
}

fn string_map_memory(values: &BTreeMap<String, String>) -> usize {
    values.iter().fold(0usize, |total, (key, value)| {
        total
            .saturating_add(std::mem::size_of::<(String, String)>())
            .saturating_add(key.len())
            .saturating_add(value.len())
    })
}

fn ttml_attributes_memory(attrs: &TtmlAttributes) -> usize {
    let resolved = attrs
        .resolved
        .iter()
        .fold(0usize, |total, ((namespace, local), value)| {
            total
                .saturating_add(std::mem::size_of::<((Option<String>, String), String)>())
                .saturating_add(namespace.as_ref().map_or(0, String::len))
                .saturating_add(local.len())
                .saturating_add(value.len())
        });
    string_map_memory(&attrs.raw).saturating_add(resolved)
}

fn run_memory(run: &SubtitleTextRun) -> usize {
    std::mem::size_of::<SubtitleTextRun>()
        .saturating_add(run.text.len())
        .saturating_add(run.speaker.as_ref().map_or(0, String::len))
        .saturating_add(run.style.as_ref().map_or(0, String::len))
}

fn cue_memory(cue: &SubtitleCue) -> usize {
    cue.runs
        .iter()
        .fold(cue_shell_memory(cue), |total, run| {
            total.saturating_add(run_memory(run))
        })
        .saturating_add(cue.speaker.as_ref().map_or(0, String::len))
        .saturating_add(cue.text.len())
}

fn cue_shell_memory(cue: &SubtitleCue) -> usize {
    std::mem::size_of::<SubtitleCue>()
        .saturating_add(cue.id.len())
        .saturating_add(cue.source_id.as_ref().map_or(0, String::len))
        .saturating_add(cue.track_id.len())
        .saturating_add(cue.region_id.as_ref().map_or(0, String::len))
        .saturating_add(cue.timing.raw.len())
        .saturating_add(string_map_memory(&cue.settings))
        .saturating_add(cue.raw_text.len())
}

fn generated_identifier_len(prefix: &str, ordinal: usize) -> usize {
    let digits = if ordinal == 0 {
        1
    } else {
        ordinal.ilog10() as usize + 1
    };
    prefix.len().saturating_add(1).saturating_add(digits.max(6))
}

fn map_entries_memory<'a>(entries: impl Iterator<Item = (&'a str, &'a str)>) -> usize {
    entries.fold(0usize, |total, (key, value)| {
        total
            .saturating_add(std::mem::size_of::<(String, String)>())
            .saturating_add(key.len())
            .saturating_add(value.len())
    })
}

fn webvtt_style_memory(raw: &str, ordinal: usize) -> usize {
    let selector = raw.lines().next().map(str::trim);
    let properties = map_entries_memory(raw.split([';', '{', '}']).filter_map(|item| {
        item.split_once(':')
            .map(|(key, value)| (key.trim(), value.trim()))
    }));
    std::mem::size_of::<SubtitleStyle>()
        .saturating_add(generated_identifier_len("style", ordinal))
        .saturating_add(selector.map_or(0, str::len))
        .saturating_add(properties)
        .saturating_add(raw.len())
}

fn webvtt_region_memory(block: &[Line<'_>], ordinal: usize) -> usize {
    let entries = block.iter().skip(1).filter_map(|line| {
        line.text
            .split_once(':')
            .map(|(key, value)| (key.trim(), value.trim()))
    });
    let id_len = entries
        .clone()
        .find_map(|(key, value)| (key == "id").then_some(value.len()))
        .unwrap_or_else(|| generated_identifier_len("region", ordinal));
    std::mem::size_of::<SubtitleRegion>()
        .saturating_add(id_len)
        .saturating_add(map_entries_memory(entries))
}

fn ttml_style_memory(attrs: &TtmlAttributes, raw_len: usize, ordinal: usize) -> usize {
    let id_len =
        attr(attrs, "id").map_or_else(|| generated_identifier_len("style", ordinal), String::len);
    let properties = map_entries_memory(
        attrs
            .raw
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "id" | "xml:id"))
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    std::mem::size_of::<SubtitleStyle>()
        .saturating_add(id_len)
        .saturating_add(attr(attrs, "style").map_or(0, String::len))
        .saturating_add(properties)
        .saturating_add(raw_len)
}

fn ttml_region_memory(attrs: &TtmlAttributes, ordinal: usize, language: Option<&str>) -> usize {
    let id_len =
        attr(attrs, "id").map_or_else(|| generated_identifier_len("region", ordinal), String::len);
    let language = language.filter(|_| !attrs.raw.contains_key("xml:lang"));
    std::mem::size_of::<SubtitleRegion>()
        .saturating_add(id_len)
        .saturating_add(string_map_memory(&attrs.raw))
        .saturating_add(language.map_or(0, |language| {
            std::mem::size_of::<(String, String)>()
                .saturating_add("xml:lang".len())
                .saturating_add(language.len())
        }))
}

fn track_memory(track: &SubtitleTrack) -> usize {
    std::mem::size_of::<SubtitleTrack>()
        .saturating_add(track.id.len())
        .saturating_add(track.label.as_ref().map_or(0, String::len))
        .saturating_add(track.language.as_ref().map_or(0, String::len))
        .saturating_add(track.kind.as_ref().map_or(0, String::len))
        .saturating_add(string_map_memory(&track.settings))
}

fn transcript_entry_memory_from_cue(cue: &SubtitleCue) -> usize {
    std::mem::size_of::<TranscriptEntry>()
        .saturating_add(cue.id.len())
        .saturating_add(cue.track_id.len())
        .saturating_add(cue.speaker.as_ref().map_or(0, String::len))
        .saturating_add(cue.text.len())
}

fn diagnostic_memory(diagnostic: &Diagnostic) -> usize {
    diagnostic
        .cause
        .iter()
        .chain(diagnostic.affected_ids.iter())
        .fold(std::mem::size_of::<Diagnostic>(), |total, value| {
            total.saturating_add(value.len())
        })
        .saturating_add(diagnostic.code.as_str().len())
        .saturating_add(diagnostic.message.len())
        .saturating_add(diagnostic.parser.len())
        .saturating_add(diagnostic.module.len())
        .saturating_add(diagnostic.source.as_ref().map_or(0, String::len))
        .saturating_add(diagnostic.documentation_uri.as_ref().map_or(0, String::len))
        .saturating_add(diagnostic.explanation_key.as_ref().map_or(0, String::len))
}

fn push_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    diagnostic: Diagnostic,
    guard: &Guard<'_>,
) -> ParseResult<()> {
    guard.retain(diagnostic_memory(&diagnostic))?;
    diagnostics.push(diagnostic);
    Ok(())
}

fn decode_report_memory(report: &crate::decode::DecodeReport) -> usize {
    let declarations = report
        .declarations
        .iter()
        .fold(0usize, |total, declaration| {
            total
                .saturating_add(std::mem::size_of_val(declaration))
                .saturating_add(declaration.label.len())
                .saturating_add(
                    declaration
                        .encoding
                        .as_ref()
                        .map_or(0, |encoding| encoding.label().len()),
                )
        });
    let issues = report.issues.iter().fold(0usize, |total, issue| {
        total
            .saturating_add(std::mem::size_of_val(issue))
            .saturating_add(issue.message.len())
    });
    let diagnostics = report.diagnostics.iter().fold(0usize, |total, diagnostic| {
        total.saturating_add(diagnostic_memory(diagnostic))
    });
    std::mem::size_of::<crate::decode::DecodeReport>()
        .saturating_add(report.schema_version.len())
        .saturating_add(report.encoding.label().len())
        .saturating_add(report.raw_identity.sha256.len())
        .saturating_add(report.decoded_identity.encoding.len())
        .saturating_add(report.decoded_identity.sha256.len())
        .saturating_add(declarations)
        .saturating_add(issues)
        .saturating_add(
            report
                .newlines
                .sequences
                .len()
                .saturating_mul(std::mem::size_of::<crate::decode::NewlineSequence>()),
        )
        .saturating_add(diagnostics)
}

fn srt_speaker(raw: &str) -> (Option<String>, String) {
    let first = raw.lines().next().unwrap_or(raw);
    if let Some((speaker, value)) = first.split_once(": ") {
        let speaker = speaker.trim();
        if !speaker.is_empty()
            && speaker.len() <= 64
            && speaker.chars().any(|character| character.is_alphabetic())
            && speaker.chars().all(|character| {
                !character.is_alphabetic()
                    || character.is_uppercase()
                    || character.is_whitespace()
                    || matches!(character, '_' | '-')
            })
        {
            return (
                Some(speaker.into()),
                format!("{}{}", value, raw.strip_prefix(first).unwrap_or_default()),
            );
        }
    }
    (None, raw.into())
}

fn decode_entities(value: &str, guard: &Guard<'_>) -> ParseResult<String> {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0usize;
    while cursor < value.len() {
        if cursor % (64 * 1024) == 0 {
            guard.checkpoint()?;
        }
        if value.as_bytes()[cursor] == b'&'
            && let Some(relative_end) = value[cursor..].find(';')
        {
            let end = cursor + relative_end;
            let entity = &value[cursor + 1..end];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "nbsp" => Some('\u{a0}'),
                "lrm" => Some('\u{200e}'),
                "rlm" => Some('\u{200f}'),
                _ => entity
                    .strip_prefix("#x")
                    .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                    .or_else(|| {
                        entity
                            .strip_prefix('#')
                            .and_then(|digits| digits.parse::<u32>().ok())
                    })
                    .and_then(char::from_u32),
            };
            if let Some(decoded) = decoded {
                output.push(decoded);
                cursor = end + 1;
                continue;
            }
        }
        let character = value[cursor..]
            .chars()
            .next()
            .expect("cursor remains at a character boundary");
        output.push(character);
        cursor += character.len_utf8();
    }
    Ok(output)
}

fn default_track(text: &str, index: &LineIndex) -> SubtitleTrack {
    SubtitleTrack {
        id: "track-0".into(),
        label: None,
        language: None,
        kind: Some("subtitles".into()),
        settings: BTreeMap::new(),
        locator: locator(0, text.len(), index),
    }
}

fn time_locator(start: Option<u64>, end: Option<u64>, track: u64) -> Option<SourceLocator> {
    let (Some(start_ms), Some(end_ms)) = (start, end) else {
        return None;
    };
    if end_ms < start_ms {
        return None;
    }
    SourceLocator::exact(LocationComponent::MediaTime {
        start_ms,
        end_ms,
        track: Some(IndexPosition::zero_based(track)),
    })
    .ok()
}

fn xml_locator(path: String) -> Option<SourceLocator> {
    SourceLocator::exact(LocationComponent::XmlPath { path }).ok()
}

fn exact(range: SourceRange) -> SourceLocator {
    SourceLocator::exact(range).expect("valid subtitle text range")
}

fn locator(start: usize, end: usize, index: &LineIndex) -> SourceLocator {
    exact(SourceRange::new(start, end.max(start), index))
}

fn malformed(code: &str, message: &str, start: usize, end: usize, index: &LineIndex) -> Diagnostic {
    let range = SourceRange::new(start, end.max(start), index);
    let mut diagnostic = Diagnostic::warning(PARSER, code, message)
        .partial()
        .with_range(range.clone());
    diagnostic.class = DiagnosticClass::MalformedInput;
    diagnostic.with_locator(exact(range))
}

fn unsupported(
    code: &str,
    message: &str,
    start: usize,
    end: usize,
    index: &LineIndex,
) -> Diagnostic {
    let range = SourceRange::new(start, end.max(start), index);
    let mut diagnostic = Diagnostic::warning(PARSER, code, message)
        .partial()
        .with_range(range.clone());
    diagnostic.class = DiagnosticClass::UnsupportedContent;
    diagnostic.with_locator(exact(range))
}

fn security(code: &str, message: &str, start: usize, end: usize, index: &LineIndex) -> Diagnostic {
    let range = SourceRange::new(start, end.max(start), index);
    let mut diagnostic = Diagnostic::warning(PARSER, code, message)
        .partial()
        .with_range(range.clone());
    diagnostic.class = DiagnosticClass::SecurityRejection;
    diagnostic.with_locator(exact(range))
}

fn limit(code: &str, message: &str, start: usize, end: usize, index: &LineIndex) -> Diagnostic {
    let mut diagnostic = Diagnostic::budget_exhausted(PARSER, message)
        .with_range(SourceRange::new(start, end.max(start), index))
        .with_locator(locator(start, end, index));
    diagnostic.code = DiagnosticCode::new(code);
    diagnostic
}

fn css_properties(raw: &str, guard: &Guard<'_>) -> ParseResult<BTreeMap<String, String>> {
    let mut properties = BTreeMap::new();
    for item in raw.split([';', '{', '}']) {
        guard.checkpoint()?;
        if let Some((key, value)) = item.split_once(':') {
            properties.insert(key.trim().into(), value.trim().into());
        }
    }
    Ok(properties)
}

fn local_name(value: &[u8]) -> String {
    let value = String::from_utf8_lossy(value);
    value.rsplit(':').next().unwrap_or(&value).into()
}

fn native_ttml_namespace(namespace: &ResolveResult<'_>) -> bool {
    matches!(
        namespace,
        ResolveResult::Bound(Namespace(value))
            if *value == TTML_NAMESPACE.as_bytes()
                || *value == TTML_LEGACY_NAMESPACE.as_bytes()
    )
}

fn attributes(
    reader: &NsReader<&[u8]>,
    start: &BytesStart<'_>,
    guard: &Guard<'_>,
) -> ParseResult<Result<TtmlAttributes, String>> {
    let mut output = TtmlAttributes::default();
    for attribute in start.attributes().with_checks(true) {
        let attribute = match attribute {
            Ok(attribute) => attribute,
            Err(error) => return Ok(Err(format!("malformed TTML attribute: {error}"))),
        };
        let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
        let value = match attribute.unescape_value() {
            Ok(value) => value.into_owned(),
            Err(error) => {
                return Ok(Err(format!("malformed TTML attribute value: {error}")));
            }
        };
        guard.retain(
            std::mem::size_of::<(String, String)>()
                .saturating_add(key.len())
                .saturating_add(value.len()),
        )?;
        if output.raw.insert(key.clone(), value.clone()).is_some() {
            return Ok(Err(format!("duplicate TTML attribute {key:?}")));
        }
        let (namespace, local) = reader.resolve_attribute(attribute.key);
        let namespace = match namespace {
            ResolveResult::Unbound => None,
            ResolveResult::Bound(namespace) => {
                Some(String::from_utf8_lossy(namespace.as_ref()).into_owned())
            }
            ResolveResult::Unknown(prefix) => {
                return Ok(Err(format!(
                    "TTML attribute uses undeclared namespace prefix {:?}",
                    String::from_utf8_lossy(&prefix)
                )));
            }
        };
        let local = String::from_utf8_lossy(local.as_ref()).into_owned();
        guard.retain(
            std::mem::size_of::<((Option<String>, String), String)>()
                .saturating_add(namespace.as_ref().map_or(0, String::len))
                .saturating_add(local.len())
                .saturating_add(value.len()),
        )?;
        if output.resolved.insert((namespace, local), value).is_some() {
            return Ok(Err(format!(
                "duplicate TTML expanded-name attribute {key:?}"
            )));
        }
    }
    Ok(Ok(output))
}

fn attr<'a>(attrs: &'a TtmlAttributes, name: &str) -> Option<&'a String> {
    attrs
        .resolved
        .iter()
        .find_map(|((namespace, local), value)| {
            if local != name {
                return None;
            }
            let namespace = namespace.as_deref();
            let recognized = match name {
                "id" | "lang" => namespace.is_none() || namespace == Some(XML_NAMESPACE),
                "frameRate"
                | "frameRateMultiplier"
                | "subFrameRate"
                | "tickRate"
                | "timeBase"
                | "dropMode" => matches!(
                    namespace,
                    Some(TTML_PARAMETER_NAMESPACE | TTML_LEGACY_PARAMETER_NAMESPACE)
                ),
                "agent" => matches!(
                    namespace,
                    Some(TTML_METADATA_NAMESPACE | TTML_LEGACY_METADATA_NAMESPACE)
                ),
                _ => namespace.is_none(),
            };
            recognized.then_some(value)
        })
}

fn current_speaker(contexts: &[TtmlContext]) -> Option<String> {
    contexts
        .iter()
        .rev()
        .filter(|context| context.native)
        .find_map(|context| context.speaker.clone())
}

fn current_style(contexts: &[TtmlContext]) -> Option<String> {
    let styles = contexts
        .iter()
        .filter(|context| context.native)
        .filter_map(|context| context.style.as_deref())
        .collect::<Vec<_>>();
    (!styles.is_empty()).then(|| styles.join(" "))
}
