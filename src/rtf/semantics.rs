//! Semantic interpretation over the loss-retaining RTF group tree.

use super::PARSER;
use super::lexer::is_destination;
use super::model::*;
use crate::container::{
    ArtifactDisposition, ArtifactMetadata, ArtifactParent, ArtifactRelationship, EmbeddedArtifact,
};
use crate::core::{ContentIdentity, Diagnostic, FormatIdentity, LocationComponent, SourceLocator};
use crate::registry::ParserError;
use encoding_rs::{Encoding, WINDOWS_1252};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct SemanticResult {
    pub charset: RtfCharset,
    pub ansi_code_page: u16,
    pub default_font: Option<i32>,
    pub generator: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub destinations: Vec<RtfDestinationOccurrence>,
    pub fonts: Vec<RtfFont>,
    pub colors: Vec<RtfColor>,
    pub styles: Vec<RtfStyleDefinition>,
    pub lists: Vec<RtfListDefinition>,
    pub list_overrides: Vec<RtfListOverride>,
    pub paragraphs: Vec<RtfParagraph>,
    pub list_items: Vec<RtfListItem>,
    pub tables: Vec<RtfTable>,
    pub fields: Vec<RtfField>,
    pub images: Vec<RtfImage>,
    pub objects: Vec<RtfObject>,
    pub revisions: Vec<RtfRevision>,
    pub comments: Vec<RtfComment>,
    pub unknown_controls: Vec<RtfControl>,
    pub embedded_artifacts: Vec<EmbeddedArtifact>,
    pub views: RtfTextViews,
}

#[derive(Debug, Clone)]
struct State {
    style: RtfCharacterStyle,
    paragraph_style: Option<i32>,
    list_override: Option<i32>,
    list_level: Option<i32>,
    unicode_fallback_length: usize,
    revision: Option<RtfRevisionKind>,
    revision_author: Option<i32>,
    revision_timestamp: Option<i64>,
    in_table: bool,
    table_depth: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            style: RtfCharacterStyle::default(),
            paragraph_style: None,
            list_override: None,
            list_level: None,
            unicode_fallback_length: 1,
            revision: None,
            revision_author: None,
            revision_timestamp: None,
            in_table: false,
            table_depth: 0,
        }
    }
}

struct Builder {
    paragraphs: Vec<RtfParagraph>,
    current_runs: Vec<RtfRun>,
    accepted: String,
    original: String,
    fields: Vec<RtfField>,
    comments: Vec<RtfComment>,
    skip_fallback: usize,
    pending_high: Option<(u16, SourceLocator, State)>,
    next_run: usize,
    next_paragraph: usize,
    next_table: usize,
    active_table: Option<usize>,
    table_row: usize,
    table_cell: usize,
    row_started: bool,
    cell_boundaries: Vec<i32>,
    paragraph_start: Option<SourceLocator>,
}

impl Builder {
    fn new() -> Self {
        Self {
            paragraphs: Vec::new(),
            current_runs: Vec::new(),
            accepted: String::new(),
            original: String::new(),
            fields: Vec::new(),
            comments: Vec::new(),
            skip_fallback: 0,
            pending_high: None,
            next_run: 0,
            next_paragraph: 0,
            next_table: 0,
            active_table: None,
            table_row: 0,
            table_cell: 0,
            row_started: false,
            cell_boundaries: Vec::new(),
            paragraph_start: None,
        }
    }

    fn add_text(&mut self, text: String, locator: SourceLocator, state: &State) {
        if text.is_empty() {
            return;
        }
        self.next_run += 1;
        self.paragraph_start.get_or_insert_with(|| locator.clone());
        self.current_runs.push(RtfRun {
            id: format!("run:{}", self.next_run),
            text: text.clone(),
            style: state.style.clone(),
            revision: state.revision,
            revision_author: state.revision_author,
            revision_timestamp: state.revision_timestamp,
            locator,
        });
        if !state.style.hidden {
            match state.revision {
                Some(RtfRevisionKind::Inserted) => self.accepted.push_str(&text),
                Some(RtfRevisionKind::Deleted) => self.original.push_str(&text),
                None => {
                    self.accepted.push_str(&text);
                    self.original.push_str(&text);
                }
            }
        }
    }

    fn flush_surrogate(&mut self) {
        if let Some((_, locator, state)) = self.pending_high.take() {
            self.add_text("�".into(), locator, &state);
        }
    }

    fn unicode(&mut self, value: i32, locator: SourceLocator, state: &State) {
        let unit = value as i16 as u16;
        if (0xd800..=0xdbff).contains(&unit) {
            self.flush_surrogate();
            self.pending_high = Some((unit, locator, state.clone()));
        } else if (0xdc00..=0xdfff).contains(&unit) {
            if let Some((high, first, first_state)) = self.pending_high.take() {
                let scalar =
                    0x1_0000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00);
                self.add_text(
                    char::from_u32(scalar).unwrap_or('�').to_string(),
                    span_locator(&first, &locator),
                    &first_state,
                );
            } else {
                self.add_text("�".into(), locator, state);
            }
        } else {
            self.flush_surrogate();
            self.add_text(
                char::from_u32(u32::from(unit)).unwrap_or('�').to_string(),
                locator,
                state,
            );
        }
        self.skip_fallback = state.unicode_fallback_length;
    }

    fn paragraph(&mut self, state: &State, terminal: &SourceLocator, force: bool) {
        self.flush_surrogate();
        if self.current_runs.is_empty() && !force {
            return;
        }
        self.next_paragraph += 1;
        let start = self
            .paragraph_start
            .take()
            .unwrap_or_else(|| terminal.clone());
        let table_position = self.active_table.map(|table_index| RtfTablePosition {
            table_index,
            row_index: self.table_row,
            cell_index: self.table_cell,
            nesting_level: state.table_depth.max(u32::from(state.in_table)),
            cell_right_twips: self.cell_boundaries.get(self.table_cell).copied(),
        });
        self.paragraphs.push(RtfParagraph {
            id: format!("paragraph:{}", self.next_paragraph),
            runs: std::mem::take(&mut self.current_runs),
            paragraph_style: state.paragraph_style,
            list_override: state.list_override,
            list_level: state.list_level,
            table_position,
            locator: span_locator(&start, terminal),
        });
    }

    fn paragraph_break(&mut self, locator: &SourceLocator, state: &State) {
        self.paragraph(state, locator, true);
        push_once(&mut self.accepted, '\n');
        push_once(&mut self.original, '\n');
    }

    fn row_start(&mut self) {
        if self.active_table.is_none() {
            self.active_table = Some(self.next_table);
            self.next_table += 1;
            self.table_row = 0;
        } else if !self.row_started {
            self.table_row += 1;
        }
        self.table_cell = 0;
        self.row_started = true;
        self.cell_boundaries.clear();
    }
}

pub(super) fn interpret(
    root: &mut RtfGroup,
    source: &[u8],
    options: &RtfOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<SemanticResult, ParserError> {
    let charset = header_charset(root);
    let ansi_code_page = header_parameter(root, "ansicpg")
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or_else(|| default_code_page(charset));
    let default_font = header_parameter(root, "deff");
    decode_group(root, ansi_code_page, diagnostics, &mut BTreeSet::new());

    let destinations = collect_destinations(root);
    let fonts = parse_fonts(root);
    let colors = parse_colors(root);
    let styles = parse_styles(root);
    let lists = parse_lists(root);
    let list_overrides = parse_list_overrides(root);
    let generator = first_destination(root, "generator")
        .map(plain_group_text)
        .map(|value| clean_text(&value));
    let metadata = parse_metadata(root);
    let unknown_controls = collect_unknown_controls(root);

    let mut builder = Builder::new();
    walk_group(root, State::default(), &mut builder);
    builder.flush_surrogate();
    if let Some(terminal) = builder.current_runs.last().map(|run| run.locator.clone()) {
        builder.paragraph(&State::default(), &terminal, false);
    }
    let paragraphs = std::mem::take(&mut builder.paragraphs);
    let list_items = build_list_items(&paragraphs);
    let tables = build_tables(&paragraphs);
    let revisions = paragraphs
        .iter()
        .flat_map(|paragraph| &paragraph.runs)
        .filter_map(|run| {
            Some(RtfRevision {
                run_id: run.id.clone(),
                kind: run.revision?,
                author_index: run.revision_author,
                timestamp: run.revision_timestamp,
                text: run.text.clone(),
                locator: run.locator.clone(),
            })
        })
        .collect();
    let accepted = normalize_view(std::mem::take(&mut builder.accepted));
    let original = normalize_view(std::mem::take(&mut builder.original));
    let parent = ContentIdentity::for_raw_bytes(source)
        .with_format(FormatIdentity::new("rtf", Some("application/rtf")));
    let (mut images, image_data) = parse_images(root);
    let (mut objects, object_data) = parse_objects(root);
    let mut embedded_artifacts = Vec::new();
    for (index, (image, bytes)) in images.iter_mut().zip(image_data).enumerate() {
        if bytes.is_empty() {
            continue;
        }
        let mut metadata = ArtifactMetadata::new(
            ArtifactParent::new(parent.clone(), ArtifactRelationship::EmbeddedIn),
            image.locator.clone(),
            ArtifactDisposition::Inline,
        )
        .with_declared_filename(format!(
            "rtf-image-{}.{}",
            index + 1,
            image_extension(image.media_type.as_deref())
        ));
        if let Some(media_type) = &image.media_type {
            metadata = metadata.with_media_type(media_type.clone());
        }
        let artifact = capture(metadata, &bytes, options)?;
        image.artifact_id = Some(artifact.identity.artifact_id.clone());
        embedded_artifacts.push(artifact);
    }
    for (index, (object, bytes)) in objects.iter_mut().zip(object_data).enumerate() {
        if bytes.is_empty() {
            continue;
        }
        let metadata = ArtifactMetadata::new(
            ArtifactParent::new(parent.clone(), ArtifactRelationship::EmbeddedIn),
            object.locator.clone(),
            ArtifactDisposition::Attachment,
        )
        .with_declared_filename(format!("rtf-object-{}.bin", index + 1))
        .with_media_type("application/x-ole-storage");
        let artifact = capture(metadata, &bytes, options)?;
        object.artifact_id = Some(artifact.identity.artifact_id.clone());
        embedded_artifacts.push(artifact);
    }
    Ok(SemanticResult {
        charset,
        ansi_code_page,
        default_font,
        generator,
        metadata,
        destinations,
        fonts,
        colors,
        styles,
        lists,
        list_overrides,
        paragraphs,
        list_items,
        tables,
        fields: builder.fields,
        images,
        objects,
        revisions,
        comments: builder.comments,
        unknown_controls,
        embedded_artifacts,
        views: RtfTextViews {
            visible: accepted.clone(),
            accepted,
            original: original.clone(),
            rejected: original,
        },
    })
}

fn capture(
    metadata: ArtifactMetadata,
    bytes: &[u8],
    options: &RtfOptions,
) -> Result<EmbeddedArtifact, ParserError> {
    let result = if options.inline_embedded_artifact_bytes {
        EmbeddedArtifact::capture_inline(metadata, bytes)
    } else {
        EmbeddedArtifact::inventory(metadata, bytes)
    };
    result.map_err(|error| {
        Box::new(Diagnostic::parser_defect(
            PARSER,
            format!("embedded-artifact invariant failed: {error}"),
        )) as ParserError
    })
}

fn decode_group(
    group: &mut RtfGroup,
    inherited: u16,
    diagnostics: &mut Vec<Diagnostic>,
    reported: &mut BTreeSet<String>,
) {
    let mut code_page = inherited;
    for element in &mut group.contents {
        match element {
            RtfElement::Control { control }
                if matches!(control.name.as_str(), "ansicpg" | "cpg") =>
            {
                if let Some(value) = control
                    .parameter
                    .and_then(|value| u16::try_from(value).ok())
                {
                    code_page = value;
                }
            }
            RtfElement::Text { text } => {
                let (decoded, problem) = decode_bytes(&text.raw_bytes, code_page);
                text.decoded = decoded;
                if let Some(problem) = problem
                    && reported.insert(format!("{code_page}:{problem}"))
                {
                    diagnostics.push(
                        Diagnostic::warning(PARSER, "rtf.encoding.recovery", problem)
                            .with_locator(text.locator.clone())
                            .partial(),
                    );
                }
            }
            RtfElement::Group { group } => {
                decode_group(group, code_page, diagnostics, reported);
            }
            _ => {}
        }
    }
}

fn decode_bytes(bytes: &[u8], code_page: u16) -> (String, Option<String>) {
    let label = match code_page {
        65001 => "utf-8".to_string(),
        932 => "shift_jis".to_string(),
        936 => "gbk".to_string(),
        949 => "euc-kr".to_string(),
        950 => "big5".to_string(),
        _ => format!("windows-{code_page}"),
    };
    let (encoding, unsupported) = Encoding::for_label(label.as_bytes())
        .map_or((WINDOWS_1252, true), |encoding| (encoding, false));
    let (value, had_errors) = encoding.decode_without_bom_handling(bytes);
    let problem = if unsupported {
        Some(format!(
            "unsupported RTF code page {code_page}; recovered as Windows-1252"
        ))
    } else if had_errors {
        Some(format!(
            "invalid byte sequence for RTF code page {code_page}; replacement retained"
        ))
    } else {
        None
    };
    (value.into_owned(), problem)
}

fn walk_group(group: &RtfGroup, mut state: State, builder: &mut Builder) {
    match group.destination.as_deref() {
        Some("field") => {
            builder.fields.push(parse_field(group));
            if let Some(result) = child_destination(group, "fldrslt") {
                walk_group(result, state, builder);
            }
            return;
        }
        Some("object") => {
            if let Some(result) = child_destination(group, "result") {
                walk_group(result, state, builder);
            }
            return;
        }
        Some("annotation") => {
            builder.comments.push(parse_comment(group));
            return;
        }
        Some(destination) if hidden_destination(destination) => return,
        Some(destination) if group.ignorable && !is_destination(destination) => return,
        _ => {}
    }
    for element in &group.contents {
        match element {
            RtfElement::Control { control } => handle_control(control, &mut state, builder),
            RtfElement::Text { text } => {
                let value = skip_fallback(&text.decoded, &mut builder.skip_fallback);
                if !value.is_empty() {
                    builder.flush_surrogate();
                    builder.add_text(value, text.locator.clone(), &state);
                }
            }
            RtfElement::Group { group } => walk_group(group, state.clone(), builder),
            RtfElement::Binary { .. } | RtfElement::Malformed { .. } => {}
        }
    }
}

fn handle_control(control: &RtfControl, state: &mut State, builder: &mut Builder) {
    if control.name != "u" {
        builder.flush_surrogate();
    }
    let enabled = control.parameter != Some(0);
    match control.name.as_str() {
        "uc" => state.unicode_fallback_length = control.parameter.unwrap_or(1).max(0) as usize,
        "u" => {
            if let Some(value) = control.parameter {
                builder.unicode(value, control.locator.clone(), state);
            }
        }
        "b" => state.style.bold = enabled,
        "i" => state.style.italic = enabled,
        "ul" | "uldb" | "uld" | "uldash" | "uldashd" | "uldashdd" | "ulwave" => {
            state.style.underline = enabled;
        }
        "ulnone" => state.style.underline = false,
        "strike" | "striked" => state.style.strike = enabled,
        "v" => state.style.hidden = enabled,
        "super" => {
            state.style.superscript = enabled;
            state.style.subscript = false;
        }
        "sub" => {
            state.style.subscript = enabled;
            state.style.superscript = false;
        }
        "nosupersub" => {
            state.style.subscript = false;
            state.style.superscript = false;
        }
        "f" => state.style.font = control.parameter,
        "fs" => state.style.font_size_half_points = control.parameter,
        "cf" => state.style.foreground_color = control.parameter,
        "cb" | "highlight" => state.style.background_color = control.parameter,
        "cs" => state.style.character_style = control.parameter,
        "s" => state.paragraph_style = control.parameter,
        "ls" => state.list_override = control.parameter,
        "ilvl" => state.list_level = control.parameter,
        "plain" => state.style = RtfCharacterStyle::default(),
        "pard" => {
            state.paragraph_style = None;
            state.list_override = None;
            state.list_level = None;
            state.in_table = false;
            state.table_depth = 0;
            if !builder.row_started {
                builder.active_table = None;
            }
        }
        "revised" => state.revision = enabled.then_some(RtfRevisionKind::Inserted),
        "deleted" => state.revision = enabled.then_some(RtfRevisionKind::Deleted),
        "revauth" => state.revision_author = control.parameter,
        "revdttm" => state.revision_timestamp = control.parameter.map(i64::from),
        "par" => builder.paragraph_break(&control.locator, state),
        "line" => builder.add_text("\n".into(), control.locator.clone(), state),
        "tab" => builder.add_text("\t".into(), control.locator.clone(), state),
        "page" | "column" | "sect" => builder.paragraph_break(&control.locator, state),
        "emdash" => builder.add_text("—".into(), control.locator.clone(), state),
        "endash" => builder.add_text("–".into(), control.locator.clone(), state),
        "emspace" => builder.add_text(" ".into(), control.locator.clone(), state),
        "enspace" => builder.add_text(" ".into(), control.locator.clone(), state),
        "qmspace" => builder.add_text(" ".into(), control.locator.clone(), state),
        "bullet" => builder.add_text("•".into(), control.locator.clone(), state),
        "lquote" => builder.add_text("‘".into(), control.locator.clone(), state),
        "rquote" => builder.add_text("’".into(), control.locator.clone(), state),
        "ldblquote" => builder.add_text("“".into(), control.locator.clone(), state),
        "rdblquote" => builder.add_text("”".into(), control.locator.clone(), state),
        "~" => builder.add_text("\u{a0}".into(), control.locator.clone(), state),
        "_" => builder.add_text("‑".into(), control.locator.clone(), state),
        "intbl" => state.in_table = enabled,
        "itap" => state.table_depth = control.parameter.unwrap_or(1).max(0) as u32,
        "trowd" => {
            state.in_table = true;
            state.table_depth = state.table_depth.max(1);
            builder.row_start();
        }
        "cellx" => {
            if let Some(value) = control.parameter {
                builder.cell_boundaries.push(value);
            }
        }
        "cell" | "nestcell" => {
            builder.paragraph(state, &control.locator, true);
            builder.table_cell += 1;
            builder.accepted.push('\t');
            builder.original.push('\t');
        }
        "row" | "nestrow" => {
            if !builder.current_runs.is_empty() {
                builder.paragraph(state, &control.locator, true);
            }
            trim_terminal(&mut builder.accepted, '\t');
            trim_terminal(&mut builder.original, '\t');
            push_once(&mut builder.accepted, '\n');
            push_once(&mut builder.original, '\n');
            builder.row_started = false;
        }
        _ => {}
    }
}

fn skip_fallback(value: &str, remaining: &mut usize) -> String {
    let skipped = value.chars().count().min(*remaining);
    *remaining -= skipped;
    value.chars().skip(skipped).collect()
}

fn hidden_destination(destination: &str) -> bool {
    matches!(
        destination,
        "fonttbl"
            | "filetbl"
            | "colortbl"
            | "stylesheet"
            | "listtable"
            | "listoverridetable"
            | "revtbl"
            | "rsidtbl"
            | "generator"
            | "info"
            | "title"
            | "subject"
            | "author"
            | "manager"
            | "company"
            | "operator"
            | "category"
            | "keywords"
            | "comment"
            | "doccomm"
            | "hlinkbase"
            | "creatim"
            | "revtim"
            | "printim"
            | "buptim"
            | "header"
            | "headerl"
            | "headerr"
            | "headerf"
            | "footer"
            | "footerl"
            | "footerr"
            | "footerf"
            | "footnote"
            | "ftnsep"
            | "ftnsepc"
            | "aftnsep"
            | "aftnsepc"
            | "fldinst"
            | "datafield"
            | "formfield"
            | "pict"
            | "shppict"
            | "nonshppict"
            | "objdata"
            | "objclass"
            | "objname"
            | "objtime"
            | "atnauthor"
            | "atnid"
            | "atnref"
            | "atntime"
            | "xe"
            | "tc"
            | "bkmkstart"
            | "bkmkend"
            | "pn"
            | "pntext"
            | "listtext"
            | "leveltext"
            | "levelnumbers"
            | "themedata"
            | "colorschememapping"
            | "datastore"
            | "xmlnstbl"
            | "xmlopen"
            | "xmlattrname"
            | "xmlattrvalue"
    )
}

fn header_charset(root: &RtfGroup) -> RtfCharset {
    for (name, charset) in [
        ("ansi", RtfCharset::Ansi),
        ("mac", RtfCharset::Mac),
        ("pc", RtfCharset::Pc),
        ("pca", RtfCharset::Pca),
    ] {
        if find_control(root, name).is_some() {
            return charset;
        }
    }
    RtfCharset::Unknown
}

const fn default_code_page(charset: RtfCharset) -> u16 {
    match charset {
        RtfCharset::Mac => 10000,
        RtfCharset::Pc => 437,
        RtfCharset::Pca => 850,
        RtfCharset::Ansi | RtfCharset::Unknown => 1252,
    }
}

fn header_parameter(root: &RtfGroup, name: &str) -> Option<i32> {
    root.contents.iter().find_map(|element| match element {
        RtfElement::Control { control } if control.name == name => control.parameter,
        _ => None,
    })
}

fn collect_destinations(root: &RtfGroup) -> Vec<RtfDestinationOccurrence> {
    let mut output = Vec::new();
    walk_groups(root, &mut |group| {
        if let Some(name) = &group.destination {
            output.push(RtfDestinationOccurrence {
                group_id: group.id.clone(),
                name: name.clone(),
                ignorable: group.ignorable,
                recognized: is_destination(name),
                locator: group.locator.clone(),
            });
        }
    });
    output
}

fn collect_unknown_controls(root: &RtfGroup) -> Vec<RtfControl> {
    let mut output = Vec::new();
    walk_controls(root, &mut |control| {
        if !control.known {
            output.push(control.clone());
        }
    });
    output
}

fn parse_fonts(root: &RtfGroup) -> Vec<RtfFont> {
    let Some(table) = first_destination(root, "fonttbl") else {
        return Vec::new();
    };
    let mut fonts = Vec::new();
    walk_groups(table, &mut |group| {
        let Some(index) = direct_parameter(group, "f") else {
            return;
        };
        if fonts.iter().any(|font: &RtfFont| font.index == index) {
            return;
        }
        let family = [
            "fnil", "froman", "fswiss", "fmodern", "fscript", "fdecor", "ftech", "fbidi",
        ]
        .into_iter()
        .find(|name| find_control(group, name).is_some())
        .map(str::to_string);
        fonts.push(RtfFont {
            index,
            name: clean_text(&plain_group_text(group)),
            family,
            charset: direct_parameter(group, "fcharset"),
            code_page: direct_parameter(group, "cpg"),
            pitch: direct_parameter(group, "fprq"),
            alternate_name: child_destination(group, "falt")
                .map(plain_group_text)
                .map(|value| clean_text(&value)),
            locator: group.locator.clone(),
        });
    });
    fonts.sort_by_key(|font| font.index);
    fonts
}

fn parse_colors(root: &RtfGroup) -> Vec<RtfColor> {
    let Some(table) = first_destination(root, "colortbl") else {
        return Vec::new();
    };
    let (mut red, mut green, mut blue) = (None, None, None);
    let mut output = Vec::new();
    for element in &table.contents {
        match element {
            RtfElement::Control { control } => match control.name.as_str() {
                "red" => red = control.parameter.and_then(|value| u8::try_from(value).ok()),
                "green" => green = control.parameter.and_then(|value| u8::try_from(value).ok()),
                "blue" => blue = control.parameter.and_then(|value| u8::try_from(value).ok()),
                _ => {}
            },
            RtfElement::Text { text } => {
                for _ in text.decoded.chars().filter(|value| *value == ';') {
                    output.push(RtfColor {
                        index: output.len(),
                        red,
                        green,
                        blue,
                        auto: red.is_none() && green.is_none() && blue.is_none(),
                    });
                    (red, green, blue) = (None, None, None);
                }
            }
            _ => {}
        }
    }
    output
}

fn parse_styles(root: &RtfGroup) -> Vec<RtfStyleDefinition> {
    let Some(table) = first_destination(root, "stylesheet") else {
        return Vec::new();
    };
    let mut styles = Vec::new();
    walk_groups(table, &mut |group| {
        let definition = [
            ("s", RtfStyleKind::Paragraph),
            ("cs", RtfStyleKind::Character),
            ("ds", RtfStyleKind::Section),
            ("ts", RtfStyleKind::Table),
        ]
        .into_iter()
        .find_map(|(name, kind)| direct_parameter(group, name).map(|number| (number, kind)));
        let Some((number, kind)) = definition else {
            return;
        };
        if styles
            .iter()
            .any(|style: &RtfStyleDefinition| style.number == number && style.kind == kind)
        {
            return;
        }
        styles.push(RtfStyleDefinition {
            number,
            kind,
            name: clean_text(&plain_group_text(group)),
            based_on: direct_parameter(group, "sbasedon"),
            next_style: direct_parameter(group, "snext"),
            additive: direct_find_control(group, "additive").is_some(),
            controls: direct_controls(group),
            locator: group.locator.clone(),
        });
    });
    styles.sort_by_key(|style| style.number);
    styles
}

fn parse_lists(root: &RtfGroup) -> Vec<RtfListDefinition> {
    let Some(table) = first_destination(root, "listtable") else {
        return Vec::new();
    };
    let mut lists = Vec::new();
    walk_groups(table, &mut |group| {
        let Some(list_id) = direct_parameter(group, "listid") else {
            return;
        };
        if lists
            .iter()
            .any(|item: &RtfListDefinition| item.list_id == list_id)
        {
            return;
        }
        let mut levels = Vec::new();
        walk_groups(group, &mut |level| {
            if level.destination.as_deref() != Some("listlevel") {
                return;
            }
            levels.push(RtfListLevel {
                level: levels.len(),
                number_format: direct_parameter(level, "levelnfc")
                    .or_else(|| direct_parameter(level, "levelnfcn")),
                start_at: direct_parameter(level, "levelstartat"),
                alignment: direct_parameter(level, "leveljc")
                    .or_else(|| direct_parameter(level, "leveljcn")),
                follow: direct_parameter(level, "levelfollow"),
                level_text: child_destination(level, "leveltext")
                    .map(plain_group_text)
                    .map(|value| clean_text(&value))
                    .unwrap_or_default(),
                level_numbers: child_destination(level, "levelnumbers")
                    .map(group_payload_bytes)
                    .unwrap_or_default(),
                controls: direct_controls(level),
                locator: level.locator.clone(),
            });
        });
        lists.push(RtfListDefinition {
            list_id,
            template_id: direct_parameter(group, "listtemplateid"),
            simple: direct_find_control(group, "listsimple").is_some(),
            hybrid: direct_find_control(group, "listhybrid").is_some(),
            levels,
            locator: group.locator.clone(),
        });
    });
    lists.sort_by_key(|list| list.list_id);
    lists
}

fn parse_list_overrides(root: &RtfGroup) -> Vec<RtfListOverride> {
    let Some(table) = first_destination(root, "listoverridetable") else {
        return Vec::new();
    };
    let mut output = Vec::new();
    walk_groups(table, &mut |group| {
        let Some(override_id) = direct_parameter(group, "ls") else {
            return;
        };
        if output
            .iter()
            .any(|item: &RtfListOverride| item.override_id == override_id)
        {
            return;
        }
        output.push(RtfListOverride {
            list_id: direct_parameter(group, "listid"),
            override_id,
            override_count: direct_parameter(group, "listoverridecount"),
            locator: group.locator.clone(),
        });
    });
    output.sort_by_key(|item| item.override_id);
    output
}

fn parse_metadata(root: &RtfGroup) -> BTreeMap<String, String> {
    let Some(info) = first_destination(root, "info") else {
        return BTreeMap::new();
    };
    info.contents
        .iter()
        .filter_map(|element| match element {
            RtfElement::Group { group } => {
                let name = group.destination.clone()?;
                let value = clean_text(&plain_group_text(group));
                (!value.is_empty()).then_some((name, value))
            }
            _ => None,
        })
        .collect()
}

fn parse_field(group: &RtfGroup) -> RtfField {
    let instruction = child_destination(group, "fldinst")
        .map(plain_group_text)
        .map(|value| clean_text(&value))
        .unwrap_or_default();
    let result = child_destination(group, "fldrslt")
        .map(plain_group_text)
        .map(|value| clean_text(&value))
        .unwrap_or_default();
    let field_type = instruction
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let target = (field_type == "HYPERLINK")
        .then(|| hyperlink_target(&instruction))
        .flatten();
    RtfField {
        group_id: group.id.clone(),
        instruction,
        result,
        field_type,
        target,
        locked: find_control(group, "fldlock").is_some(),
        dirty: find_control(group, "flddirty").is_some(),
        locator: group.locator.clone(),
    }
}

fn hyperlink_target(instruction: &str) -> Option<String> {
    let rest = instruction
        .trim_start()
        .strip_prefix("HYPERLINK")
        .or_else(|| instruction.trim_start().strip_prefix("hyperlink"))?
        .trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        return quoted.split('"').next().map(str::to_string);
    }
    rest.split_whitespace()
        .next()
        .filter(|value| !value.starts_with('\\'))
        .map(str::to_string)
}

fn parse_comment(group: &RtfGroup) -> RtfComment {
    RtfComment {
        group_id: group.id.clone(),
        annotation_id: parameter(group, "atnid"),
        author: child_destination(group, "atnauthor")
            .map(plain_group_text)
            .map(|value| clean_text(&value)),
        initials: child_destination(group, "atnid")
            .map(plain_group_text)
            .map(|value| clean_text(&value))
            .filter(|value| !value.is_empty()),
        text: clean_text(&direct_text(group)),
        range_start: parameter(group, "atrfstart"),
        range_end: parameter(group, "atrfend"),
        locator: group.locator.clone(),
    }
}

fn parse_images(root: &RtfGroup) -> (Vec<RtfImage>, Vec<Vec<u8>>) {
    let mut images = Vec::new();
    let mut payloads = Vec::new();
    walk_groups(root, &mut |group| {
        if group.destination.as_deref() != Some("pict") {
            return;
        }
        let media_type = [
            ("pngblip", "image/png"),
            ("jpegblip", "image/jpeg"),
            ("emfblip", "image/emf"),
            ("wmetafile", "image/wmf"),
            ("dibitmap", "image/bmp"),
        ]
        .into_iter()
        .find(|(name, _)| find_control(group, name).is_some())
        .map(|(_, media_type)| media_type.to_string());
        let bytes = group_payload_bytes(group);
        images.push(RtfImage {
            group_id: group.id.clone(),
            media_type,
            width_pixels: parameter(group, "picw"),
            height_pixels: parameter(group, "pich"),
            width_goal_twips: parameter(group, "picwgoal"),
            height_goal_twips: parameter(group, "pichgoal"),
            scale_x_percent: parameter(group, "picscalex"),
            scale_y_percent: parameter(group, "picscaley"),
            binary_length: bytes.len(),
            artifact_id: None,
            locator: group.locator.clone(),
        });
        payloads.push(bytes);
    });
    (images, payloads)
}

fn parse_objects(root: &RtfGroup) -> (Vec<RtfObject>, Vec<Vec<u8>>) {
    let mut objects = Vec::new();
    let mut payloads = Vec::new();
    walk_groups(root, &mut |group| {
        if group.destination.as_deref() != Some("object") {
            return;
        }
        let object_type = [
            "objemb",
            "objlink",
            "objautlink",
            "objsub",
            "objpub",
            "objicemb",
            "objhtml",
            "objocx",
        ]
        .into_iter()
        .find(|name| find_control(group, name).is_some())
        .map(str::to_string);
        let bytes = child_destination(group, "objdata")
            .map(group_payload_bytes)
            .unwrap_or_default();
        objects.push(RtfObject {
            group_id: group.id.clone(),
            object_type,
            class_name: child_destination(group, "objclass")
                .map(plain_group_text)
                .map(|value| clean_text(&value)),
            object_name: child_destination(group, "objname")
                .map(plain_group_text)
                .map(|value| clean_text(&value)),
            result_text: child_destination(group, "result")
                .map(plain_group_text)
                .map(|value| clean_text(&value))
                .unwrap_or_default(),
            binary_length: bytes.len(),
            artifact_id: None,
            locator: group.locator.clone(),
        });
        payloads.push(bytes);
    });
    (objects, payloads)
}

fn group_payload_bytes(group: &RtfGroup) -> Vec<u8> {
    let mut output = Vec::new();
    let mut high_nibble = None;
    collect_payload(group, &mut output, &mut high_nibble);
    output
}

fn collect_payload(group: &RtfGroup, output: &mut Vec<u8>, high_nibble: &mut Option<u8>) {
    for element in &group.contents {
        match element {
            RtfElement::Binary { binary } => {
                *high_nibble = None;
                output.extend_from_slice(&binary.bytes);
            }
            RtfElement::Text { text } => {
                if text.source_syntax.starts_with(&[b'\\', 39]) {
                    *high_nibble = None;
                    output.extend_from_slice(&text.raw_bytes);
                } else {
                    for nibble in text.raw_bytes.iter().copied().filter_map(hex_value) {
                        if let Some(high) = high_nibble.take() {
                            output.push((high << 4) | nibble);
                        } else {
                            *high_nibble = Some(nibble);
                        }
                    }
                }
            }
            RtfElement::Group { group } => collect_payload(group, output, high_nibble),
            _ => {}
        }
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn image_extension(media_type: Option<&str>) -> &'static str {
    match media_type {
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/emf") => "emf",
        Some("image/wmf") => "wmf",
        Some("image/bmp") => "bmp",
        _ => "bin",
    }
}

fn build_list_items(paragraphs: &[RtfParagraph]) -> Vec<RtfListItem> {
    paragraphs
        .iter()
        .filter_map(|paragraph| {
            Some(RtfListItem {
                paragraph_id: paragraph.id.clone(),
                override_id: paragraph.list_override?,
                level: paragraph.list_level.unwrap_or(0),
                text: paragraph_text(paragraph),
                locator: paragraph.locator.clone(),
            })
        })
        .collect()
}

fn paragraph_text(paragraph: &RtfParagraph) -> String {
    paragraph
        .runs
        .iter()
        .filter(|run| !run.style.hidden && run.revision != Some(RtfRevisionKind::Deleted))
        .map(|run| run.text.as_str())
        .collect()
}

fn build_tables(paragraphs: &[RtfParagraph]) -> Vec<RtfTable> {
    type Cells<'a> = BTreeMap<usize, Vec<&'a RtfParagraph>>;
    type Rows<'a> = BTreeMap<usize, Cells<'a>>;
    let mut grouped = BTreeMap::<usize, Rows<'_>>::new();
    let mut depths = BTreeMap::new();
    for paragraph in paragraphs {
        let Some(position) = &paragraph.table_position else {
            continue;
        };
        depths.insert(position.table_index, position.nesting_level);
        grouped
            .entry(position.table_index)
            .or_default()
            .entry(position.row_index)
            .or_default()
            .entry(position.cell_index)
            .or_default()
            .push(paragraph);
    }
    grouped
        .into_iter()
        .map(|(table_index, rows)| {
            let rows = rows
                .into_iter()
                .map(|(row_index, cells)| {
                    let cells = cells
                        .into_iter()
                        .map(|(cell_index, paragraphs)| {
                            let first = paragraphs.first().expect("non-empty grouped cell");
                            let last = paragraphs.last().expect("non-empty grouped cell");
                            RtfTableCell {
                                index: cell_index,
                                right_boundary_twips: first
                                    .table_position
                                    .as_ref()
                                    .and_then(|position| position.cell_right_twips),
                                paragraph_ids: paragraphs
                                    .iter()
                                    .map(|paragraph| paragraph.id.clone())
                                    .collect(),
                                text: paragraphs
                                    .iter()
                                    .map(|paragraph| paragraph_text(paragraph))
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                                locator: span_locator(&first.locator, &last.locator),
                            }
                        })
                        .collect::<Vec<_>>();
                    let locator = span_locator(
                        &cells.first().expect("non-empty grouped row").locator,
                        &cells.last().expect("non-empty grouped row").locator,
                    );
                    RtfTableRow {
                        index: row_index,
                        cells,
                        locator,
                    }
                })
                .collect::<Vec<_>>();
            let locator = span_locator(
                &rows.first().expect("non-empty grouped table").locator,
                &rows.last().expect("non-empty grouped table").locator,
            );
            RtfTable {
                id: format!("table:{}", table_index + 1),
                nesting_level: depths.get(&table_index).copied().unwrap_or(1),
                rows,
                locator,
            }
        })
        .collect()
}

fn first_destination<'a>(group: &'a RtfGroup, name: &str) -> Option<&'a RtfGroup> {
    if group.destination.as_deref() == Some(name) {
        return Some(group);
    }
    group.contents.iter().find_map(|element| match element {
        RtfElement::Group { group } => first_destination(group, name),
        _ => None,
    })
}

fn child_destination<'a>(group: &'a RtfGroup, name: &str) -> Option<&'a RtfGroup> {
    group.contents.iter().find_map(|element| match element {
        RtfElement::Group { group } if group.destination.as_deref() == Some(name) => {
            Some(group.as_ref())
        }
        _ => None,
    })
}

fn find_control<'a>(group: &'a RtfGroup, name: &str) -> Option<&'a RtfControl> {
    group.contents.iter().find_map(|element| match element {
        RtfElement::Control { control } if control.name == name => Some(control),
        RtfElement::Group { group } => find_control(group, name),
        _ => None,
    })
}

fn parameter(group: &RtfGroup, name: &str) -> Option<i32> {
    find_control(group, name).and_then(|control| control.parameter)
}

fn direct_find_control<'a>(group: &'a RtfGroup, name: &str) -> Option<&'a RtfControl> {
    group.contents.iter().find_map(|element| match element {
        RtfElement::Control { control } if control.name == name => Some(control),
        _ => None,
    })
}

fn direct_parameter(group: &RtfGroup, name: &str) -> Option<i32> {
    direct_find_control(group, name).and_then(|control| control.parameter)
}

fn direct_controls(group: &RtfGroup) -> Vec<RtfControl> {
    group
        .contents
        .iter()
        .filter_map(|element| match element {
            RtfElement::Control { control } => Some(control.clone()),
            _ => None,
        })
        .collect()
}

fn walk_groups(group: &RtfGroup, callback: &mut impl FnMut(&RtfGroup)) {
    callback(group);
    for element in &group.contents {
        if let RtfElement::Group { group } = element {
            walk_groups(group, callback);
        }
    }
}

fn walk_controls(group: &RtfGroup, callback: &mut impl FnMut(&RtfControl)) {
    for element in &group.contents {
        match element {
            RtfElement::Control { control } => callback(control),
            RtfElement::Group { group } => walk_controls(group, callback),
            _ => {}
        }
    }
}

fn plain_group_text(group: &RtfGroup) -> String {
    let mut output = String::new();
    collect_plain_text(group, &mut output);
    output
}

fn direct_text(group: &RtfGroup) -> String {
    group
        .contents
        .iter()
        .filter_map(|element| match element {
            RtfElement::Text { text } => Some(text.decoded.as_str()),
            _ => None,
        })
        .collect()
}

fn collect_plain_text(group: &RtfGroup, output: &mut String) {
    let mut skip = 0usize;
    let mut fallback_length = 1usize;
    for element in &group.contents {
        match element {
            RtfElement::Text { text } => output.push_str(&skip_fallback(&text.decoded, &mut skip)),
            RtfElement::Control { control } => match control.name.as_str() {
                "uc" => fallback_length = control.parameter.unwrap_or(1).max(0) as usize,
                "u" => {
                    if let Some(value) = control.parameter {
                        let unit = value as i16 as u16;
                        output.push(char::from_u32(u32::from(unit)).unwrap_or('�'));
                        skip = fallback_length;
                    }
                }
                "tab" => output.push('\t'),
                "line" | "par" => output.push('\n'),
                _ => {}
            },
            RtfElement::Group { group } => collect_plain_text(group, output),
            _ => {}
        }
    }
}

fn clean_text(value: &str) -> String {
    value.trim().trim_end_matches(';').trim().to_string()
}

fn normalize_view(value: String) -> String {
    value
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn push_once(value: &mut String, character: char) {
    if !value.ends_with(character) {
        value.push(character);
    }
}

fn trim_terminal(value: &mut String, character: char) {
    while value.ends_with(character) {
        value.pop();
    }
}

fn span_locator(start: &SourceLocator, end: &SourceLocator) -> SourceLocator {
    let (byte_start, start_line, start_column) = match start.components().last() {
        Some(LocationComponent::TextRange {
            byte_start,
            start_line,
            start_column,
            ..
        }) => (*byte_start, *start_line, *start_column),
        _ => return start.clone(),
    };
    let (byte_end, end_line, end_column) = match end.components().last() {
        Some(LocationComponent::TextRange {
            byte_end,
            end_line,
            end_column,
            ..
        }) => (*byte_end, *end_line, *end_column),
        _ => return start.clone(),
    };
    if byte_end < byte_start {
        return start.clone();
    }
    SourceLocator::exact(LocationComponent::TextRange {
        byte_start,
        byte_end,
        start_line,
        start_column,
        end_line,
        end_column,
    })
    .unwrap_or_else(|_| start.clone())
}
