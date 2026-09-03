//! Byte-oriented RTF group and control lexer with bounded recovery.

use super::PARSER;
use super::model::{RtfBinary, RtfControl, RtfElement, RtfGroup, RtfMalformed, RtfText};
use crate::core::{Diagnostic, LocationComponent, SourceLocator};
use crate::registry::{ParserContext, ParserError};

pub(super) struct LexedRtf {
    pub root: RtfGroup,
    pub diagnostics: Vec<Diagnostic>,
}

struct GroupBuilder {
    id: String,
    start: usize,
    destination: Option<String>,
    ignorable: bool,
    saw_leading_control: bool,
    contents: Vec<RtfElement>,
}

impl GroupBuilder {
    fn finish(self, end: usize, closed: bool, index: &RawLineIndex) -> RtfGroup {
        RtfGroup {
            id: self.id,
            destination: self.destination,
            ignorable: self.ignorable,
            closed,
            contents: self.contents,
            locator: index.locator(self.start, end),
        }
    }

    fn push_control(&mut self, control: RtfControl) {
        if !self.saw_leading_control {
            if control.control_symbol && control.name == "*" {
                self.ignorable = true;
            } else if !control.control_symbol {
                if is_destination(&control.name) || self.ignorable {
                    self.destination = Some(control.name.clone());
                }
                self.saw_leading_control = true;
            } else {
                self.saw_leading_control = true;
            }
        }
        self.contents.push(RtfElement::Control { control });
    }

    fn push_text(
        &mut self,
        raw_bytes: &[u8],
        source_syntax: &[u8],
        start: usize,
        end: usize,
        index: &RawLineIndex,
    ) {
        if raw_bytes.is_empty() {
            return;
        }
        if let Some(RtfElement::Text { text }) = self.contents.last_mut()
            && locator_end(&text.locator) == Some(start)
            && (text.source_syntax == text.raw_bytes) == (source_syntax == raw_bytes)
        {
            text.raw_bytes.extend_from_slice(raw_bytes);
            text.source_syntax.extend_from_slice(source_syntax);
            text.locator = index.locator(locator_start(&text.locator).unwrap_or(start), end);
            return;
        }
        self.contents.push(RtfElement::Text {
            text: RtfText {
                decoded: String::new(),
                raw_bytes: raw_bytes.to_vec(),
                source_syntax: source_syntax.to_vec(),
                locator: index.locator(start, end),
            },
        });
    }
}

pub(super) fn lex(bytes: &[u8], context: &ParserContext<'_>) -> Result<LexedRtf, ParserError> {
    let index = RawLineIndex::new(bytes);
    let mut diagnostics = Vec::new();
    let mut stack = Vec::<GroupBuilder>::new();
    let mut roots = Vec::<RtfGroup>::new();
    let mut outside = Vec::<RtfElement>::new();
    let mut group_sequence = 0usize;
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        context.checkpoint()?;
        match bytes[cursor] {
            b'{' => {
                group_sequence = group_sequence.saturating_add(1);
                let depth = u64::try_from(stack.len().saturating_add(1)).unwrap_or(u64::MAX);
                context.observe_nesting_depth(depth)?;
                stack.push(GroupBuilder {
                    id: format!("group:{group_sequence}"),
                    start: cursor,
                    destination: None,
                    ignorable: false,
                    saw_leading_control: false,
                    contents: Vec::new(),
                });
                cursor += 1;
            }
            b'}' => {
                if let Some(group) = stack.pop() {
                    let group = group.finish(cursor + 1, true, &index);
                    append_group(group, &mut stack, &mut roots);
                } else {
                    let malformed = RtfMalformed {
                        reason: "unmatched closing brace".into(),
                        raw_bytes: vec![b'}'],
                        locator: index.locator(cursor, cursor + 1),
                    };
                    diagnostics.push(
                        Diagnostic::malformed(PARSER, "unmatched RTF closing brace")
                            .with_locator(malformed.locator.clone())
                            .partial(),
                    );
                    outside.push(RtfElement::Malformed { malformed });
                }
                cursor += 1;
            }
            b'\\' => {
                let (elements, next) = lex_control(bytes, cursor, &index, &mut diagnostics);
                for element in elements {
                    append_element(element, &mut stack, &mut outside);
                }
                cursor = next;
            }
            b'\r' | b'\n' => cursor += 1,
            _ => {
                let start = cursor;
                while cursor < bytes.len()
                    && !matches!(bytes[cursor], b'{' | b'}' | b'\\' | b'\r' | b'\n')
                {
                    cursor += 1;
                }
                append_text(
                    &bytes[start..cursor],
                    &bytes[start..cursor],
                    start,
                    cursor,
                    &index,
                    &mut stack,
                    &mut outside,
                );
            }
        }
    }

    if !stack.is_empty() {
        let count = stack.len();
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                format!("RTF ended with {count} unclosed group(s); groups were closed at EOF"),
            )
            .with_locator(index.locator(stack[0].start, bytes.len()))
            .partial(),
        );
        while let Some(group) = stack.pop() {
            let group = group.finish(bytes.len(), false, &index);
            append_group(group, &mut stack, &mut roots);
        }
    }

    let root_index = roots
        .iter()
        .position(|group| group.destination.as_deref() == Some("rtf"))
        .ok_or_else(|| {
            Box::new(
                Diagnostic::malformed(PARSER, "input has no top-level RTF header group")
                    .with_locator(index.locator(0, bytes.len().min(8))),
            ) as ParserError
        })?;
    let mut root = roots.remove(root_index);
    if !roots.is_empty() || !outside.is_empty() {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                "content outside the top-level RTF group was retained during recovery",
            )
            .with_locator(index.locator(0, bytes.len()))
            .partial(),
        );
        for group in roots {
            root.contents.push(RtfElement::Group {
                group: Box::new(group),
            });
        }
        root.contents.extend(outside);
        root.locator = index.locator(0, bytes.len());
    }
    Ok(LexedRtf { root, diagnostics })
}

fn append_group(group: RtfGroup, stack: &mut [GroupBuilder], roots: &mut Vec<RtfGroup>) {
    if let Some(parent) = stack.last_mut() {
        parent.contents.push(RtfElement::Group {
            group: Box::new(group),
        });
    } else {
        roots.push(group);
    }
}

fn append_element(element: RtfElement, stack: &mut [GroupBuilder], outside: &mut Vec<RtfElement>) {
    if let Some(group) = stack.last_mut() {
        match element {
            RtfElement::Control { control } => group.push_control(control),
            other => group.contents.push(other),
        }
    } else {
        outside.push(element);
    }
}

#[allow(clippy::too_many_arguments)]
fn append_text(
    raw_bytes: &[u8],
    source_syntax: &[u8],
    start: usize,
    end: usize,
    index: &RawLineIndex,
    stack: &mut [GroupBuilder],
    outside: &mut Vec<RtfElement>,
) {
    if let Some(group) = stack.last_mut() {
        group.push_text(raw_bytes, source_syntax, start, end, index);
    } else if !raw_bytes.iter().all(u8::is_ascii_whitespace) {
        outside.push(RtfElement::Malformed {
            malformed: RtfMalformed {
                reason: "text outside top-level group".into(),
                raw_bytes: source_syntax.to_vec(),
                locator: index.locator(start, end),
            },
        });
    }
}

fn lex_control(
    bytes: &[u8],
    start: usize,
    index: &RawLineIndex,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<RtfElement>, usize) {
    if start + 1 >= bytes.len() {
        let malformed = RtfMalformed {
            reason: "trailing backslash".into(),
            raw_bytes: vec![b'\\'],
            locator: index.locator(start, start + 1),
        };
        diagnostics.push(
            Diagnostic::malformed(PARSER, "RTF ends with a trailing backslash")
                .with_locator(malformed.locator.clone())
                .partial(),
        );
        return (vec![RtfElement::Malformed { malformed }], start + 1);
    }
    let symbol = bytes[start + 1];
    if matches!(symbol, b'\\' | b'{' | b'}') {
        let locator = index.locator(start, start + 2);
        return (
            vec![RtfElement::Text {
                text: RtfText {
                    decoded: String::new(),
                    raw_bytes: vec![symbol],
                    source_syntax: bytes[start..start + 2].to_vec(),
                    locator,
                },
            }],
            start + 2,
        );
    }
    if symbol == b'\'' {
        if start + 3 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[start + 2]), hex(bytes[start + 3]))
        {
            return (
                vec![RtfElement::Text {
                    text: RtfText {
                        decoded: String::new(),
                        raw_bytes: vec![(high << 4) | low],
                        source_syntax: bytes[start..start + 4].to_vec(),
                        locator: index.locator(start, start + 4),
                    },
                }],
                start + 4,
            );
        }
        let end = bytes.len().min(start + 4);
        let malformed = RtfMalformed {
            reason: "invalid hexadecimal escape".into(),
            raw_bytes: bytes[start..end].to_vec(),
            locator: index.locator(start, end),
        };
        diagnostics.push(
            Diagnostic::malformed(PARSER, "invalid or truncated RTF hexadecimal escape")
                .with_locator(malformed.locator.clone())
                .partial(),
        );
        return (vec![RtfElement::Malformed { malformed }], end);
    }
    if !symbol.is_ascii_alphabetic() {
        let end = start + 2;
        return (
            vec![RtfElement::Control {
                control: RtfControl {
                    name: char::from(symbol).to_string(),
                    parameter: None,
                    control_symbol: true,
                    known: known_symbol(symbol),
                    raw_bytes: bytes[start..end].to_vec(),
                    locator: index.locator(start, end),
                },
            }],
            end,
        );
    }

    let mut cursor = start + 1;
    while cursor < bytes.len() && bytes[cursor].is_ascii_alphabetic() {
        cursor += 1;
    }
    let name = String::from_utf8_lossy(&bytes[start + 1..cursor]).into_owned();
    let parameter_start = cursor;
    if cursor < bytes.len() && bytes[cursor] == b'-' {
        cursor += 1;
    }
    let digits_start = cursor;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }
    let parameter = if cursor > digits_start {
        std::str::from_utf8(&bytes[parameter_start..cursor])
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
    } else {
        None
    };
    if cursor > digits_start && parameter.is_none() {
        diagnostics.push(
            Diagnostic::malformed(
                PARSER,
                format!("control word \\{name} has an invalid parameter"),
            )
            .with_locator(index.locator(start, cursor))
            .partial(),
        );
    }
    if cursor < bytes.len() && bytes[cursor] == b' ' {
        cursor += 1;
    }
    let control = RtfControl {
        name: name.clone(),
        parameter,
        control_symbol: false,
        known: known_control(&name),
        raw_bytes: bytes[start..cursor].to_vec(),
        locator: index.locator(start, cursor),
    };
    let mut elements = vec![RtfElement::Control { control }];
    if name == "bin" {
        let declared = parameter.unwrap_or(0).max(0) as usize;
        let available = bytes.len().saturating_sub(cursor);
        let length = available.min(declared);
        let end = cursor + length;
        let truncated = length != declared;
        let binary = RtfBinary {
            declared_length: declared,
            bytes: bytes[cursor..end].to_vec(),
            truncated,
            locator: index.locator(cursor, end),
        };
        if truncated {
            diagnostics.push(
                Diagnostic::malformed(
                    PARSER,
                    format!(
                        "RTF binary payload declared {declared} bytes but only {length} remain"
                    ),
                )
                .with_locator(binary.locator.clone())
                .partial(),
            );
        }
        elements.push(RtfElement::Binary { binary });
        cursor = end;
    }
    (elements, cursor)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn is_destination(name: &str) -> bool {
    matches!(
        name,
        "rtf"
            | "fonttbl"
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
            | "field"
            | "fldinst"
            | "fldrslt"
            | "datafield"
            | "formfield"
            | "pict"
            | "shppict"
            | "nonshppict"
            | "object"
            | "objdata"
            | "objclass"
            | "objname"
            | "objtime"
            | "result"
            | "annotation"
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
            | "list"
            | "listlevel"
            | "leveltext"
            | "levelnumbers"
            | "listoverride"
            | "listoverridelevel"
            | "latentstyles"
            | "mmathPr"
            | "mmath"
            | "themedata"
            | "colorschememapping"
            | "datastore"
            | "xmlnstbl"
            | "xmlopen"
            | "xmlattrname"
            | "xmlattrvalue"
    )
}

pub(super) fn known_control(name: &str) -> bool {
    is_destination(name)
        || matches!(
            name,
            "ansi"
                | "mac"
                | "pc"
                | "pca"
                | "ansicpg"
                | "deff"
                | "deflang"
                | "adeflang"
                | "uc"
                | "u"
                | "upr"
                | "ud"
                | "f"
                | "fcharset"
                | "cpg"
                | "fprq"
                | "fnil"
                | "froman"
                | "fswiss"
                | "fmodern"
                | "fscript"
                | "fdecor"
                | "ftech"
                | "fbidi"
                | "falt"
                | "red"
                | "green"
                | "blue"
                | "ctint"
                | "cshade"
                | "s"
                | "cs"
                | "ds"
                | "ts"
                | "sbasedon"
                | "snext"
                | "slink"
                | "additive"
                | "sautoupd"
                | "shidden"
                | "slocked"
                | "spriority"
                | "sqformat"
                | "styrsid"
                | "plain"
                | "pard"
                | "sectd"
                | "b"
                | "i"
                | "ul"
                | "ulnone"
                | "uldb"
                | "uld"
                | "uldash"
                | "uldashd"
                | "uldashdd"
                | "ulwave"
                | "strike"
                | "striked"
                | "v"
                | "caps"
                | "scaps"
                | "outl"
                | "shad"
                | "embo"
                | "impr"
                | "sub"
                | "super"
                | "nosupersub"
                | "fs"
                | "cf"
                | "cb"
                | "highlight"
                | "kerning"
                | "expnd"
                | "expndtw"
                | "lang"
                | "langfe"
                | "langnp"
                | "rtlch"
                | "ltrch"
                | "rtlpar"
                | "ltrpar"
                | "ql"
                | "qr"
                | "qc"
                | "qj"
                | "fi"
                | "li"
                | "ri"
                | "lin"
                | "rin"
                | "sb"
                | "sa"
                | "sl"
                | "slmult"
                | "keep"
                | "keepn"
                | "pagebb"
                | "widctlpar"
                | "nowidctlpar"
                | "hyphpar"
                | "intbl"
                | "itap"
                | "trowd"
                | "cell"
                | "row"
                | "nestcell"
                | "nestrow"
                | "cellx"
                | "clmgf"
                | "clmrg"
                | "clvmgf"
                | "clvmrg"
                | "trhdr"
                | "trkeep"
                | "trleft"
                | "trrh"
                | "par"
                | "line"
                | "tab"
                | "page"
                | "column"
                | "sect"
                | "softline"
                | "softpage"
                | "softcol"
                | "emdash"
                | "endash"
                | "emspace"
                | "enspace"
                | "qmspace"
                | "bullet"
                | "lquote"
                | "rquote"
                | "ldblquote"
                | "rdblquote"
                | "zwj"
                | "zwnj"
                | "zwbo"
                | "zwnbo"
                | "bin"
                | "flddirty"
                | "fldlock"
                | "fldedit"
                | "fldpriv"
                | "fldalt"
                | "pngblip"
                | "jpegblip"
                | "emfblip"
                | "wmetafile"
                | "dibitmap"
                | "wbitmap"
                | "picw"
                | "pich"
                | "picwgoal"
                | "pichgoal"
                | "picscalex"
                | "picscaley"
                | "piccropl"
                | "piccropr"
                | "piccropt"
                | "piccropb"
                | "bliptag"
                | "blipuid"
                | "objemb"
                | "objlink"
                | "objautlink"
                | "objsub"
                | "objpub"
                | "objicemb"
                | "objhtml"
                | "objocx"
                | "objw"
                | "objh"
                | "objscalex"
                | "objscaley"
                | "objupdate"
                | "revised"
                | "deleted"
                | "revauth"
                | "revdttm"
                | "insrsid"
                | "delrsid"
                | "charrsid"
                | "sectrsid"
                | "pararsid"
                | "atrfstart"
                | "atrfend"
                | "atnid"
                | "listid"
                | "listtemplateid"
                | "listsimple"
                | "listhybrid"
                | "listrestarthdn"
                | "levelnfc"
                | "levelnfcn"
                | "leveljc"
                | "leveljcn"
                | "levelstartat"
                | "levelfollow"
                | "levellegal"
                | "levelnorestart"
                | "levelpicture"
                | "levelspace"
                | "levelindent"
                | "ls"
                | "ilvl"
                | "listoverridecount"
                | "listoverrideformat"
                | "listoverridestartat"
                | "nofpages"
                | "nofwords"
                | "nofchars"
                | "nofcharsws"
                | "version"
                | "vern"
                | "edmins"
                | "id"
        )
}

fn known_symbol(symbol: u8) -> bool {
    matches!(
        symbol,
        b'*' | b'~' | b'-' | b'_' | b':' | b'|' | b'\n' | b'\r'
    )
}

#[derive(Debug)]
pub(super) struct RawLineIndex {
    line_starts: Vec<usize>,
}

impl RawLineIndex {
    pub fn new(bytes: &[u8]) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        Self { line_starts }
    }

    pub fn locator(&self, start: usize, end: usize) -> SourceLocator {
        let (start_line, start_column) = self.position(start);
        let (end_line, end_column) = self.position(end);
        SourceLocator::exact(LocationComponent::TextRange {
            byte_start: start,
            byte_end: end,
            start_line,
            start_column,
            end_line,
            end_column,
        })
        .expect("RTF byte locator is ordered and one-based")
    }

    fn position(&self, offset: usize) -> (usize, usize) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        (line + 1, offset.saturating_sub(self.line_starts[line]) + 1)
    }
}

pub(super) fn locator_start(locator: &SourceLocator) -> Option<usize> {
    match locator.components().last()? {
        LocationComponent::TextRange { byte_start, .. } => Some(*byte_start),
        _ => None,
    }
}

pub(super) fn locator_end(locator: &SourceLocator) -> Option<usize> {
    match locator.components().last()? {
        LocationComponent::TextRange { byte_end, .. } => Some(*byte_end),
        _ => None,
    }
}
