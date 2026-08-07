//! Safe namespace-aware XML and JATS structural parser.
//!
//! Parsing is local and inert: no entity resolver, schema loader, XInclude
//! processor, network callback, or active-content execution is installed.
use crate::core::{
    ArtifactKind, ContentIdentity, Diagnostic, DiagnosticClass, Envelope, FormatIdentity,
    LineIndex, LocationComponent, OperationKind, OperationStatus, ParserInfo, SchemaVersion,
    SourceInfo, SourceLocator, SourceRange, options_digest,
};
use crate::decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeReport, DecodedByteRange, DecodedText,
    RawByteRange, TextEncoding, decode_text,
};
use crate::security::{XmlSecurityFinding, XmlSecurityFindingKind, inspect_xml};
use quick_xml::{Reader, events::Event};
#[cfg(feature = "schemas")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlDocument {
    pub schema_version: String,
    pub raw_bytes: Vec<u8>,
    pub raw_range: RawByteRange,
    pub decoded_text: String,
    pub decoded_range: SourceRange,
    pub locator: SourceLocator,
    pub encoding: TextEncoding,
    pub decoding: DecodeReport,
    pub dialect: XmlDialect,
    pub declaration: Option<XmlDeclaration>,
    pub nodes: Vec<XmlNode>,
    pub root_element_ids: Vec<String>,
    pub metadata: Vec<XmlMetadata>,
    pub links: Vec<XmlLink>,
    pub tables: Vec<XmlTable>,
    pub media: Vec<XmlMediaReference>,
    pub sections: Vec<XmlSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scholarly_links: Option<crate::jats::JatsScholarlyLinks>,
    pub entities: Vec<XmlEntity>,
    pub security_findings: Vec<XmlSecurityFinding>,
    pub well_formed: bool,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum XmlDialect {
    #[default]
    Auto,
    Xml,
    Jats,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlDeclaration {
    pub version: Option<String>,
    pub encoding: Option<String>,
    pub standalone: Option<String>,
    pub raw: String,
    pub range: SourceRange,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlNode {
    pub id: String,
    pub kind: XmlNodeKind,
    pub range: SourceRange,
    pub locator: SourceLocator,
    pub raw_range: Option<RawByteRange>,
    pub xml_path: String,
    pub depth: usize,
    pub parent_id: Option<String>,
    pub children: Vec<String>,
    pub qualified_name: Option<String>,
    pub local_name: Option<String>,
    pub prefix: Option<String>,
    pub namespace_uri: Option<String>,
    pub namespace_bindings: Vec<XmlNamespaceBinding>,
    pub attributes: Vec<XmlAttribute>,
    pub text: Option<String>,
    pub raw: String,
    pub self_closing: bool,
    pub known_jats_element: bool,
    pub recovered: bool,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum XmlNodeKind {
    Element,
    Text,
    Cdata,
    Comment,
    ProcessingInstruction,
    Doctype,
    EntityReference,
    RawUnknown,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XmlNamespaceBinding {
    pub prefix: Option<String>,
    pub uri: String,
    pub declared_here: bool,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlAttribute {
    pub qualified_name: String,
    pub local_name: String,
    pub prefix: Option<String>,
    pub namespace_uri: Option<String>,
    pub value: String,
    pub raw_value: String,
    pub quote: Option<char>,
    pub range: SourceRange,
    pub name_range: SourceRange,
    pub value_range: Option<SourceRange>,
    pub locator: SourceLocator,
    pub namespace_declaration: bool,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlMetadata {
    pub node_id: String,
    pub kind: String,
    pub name: Option<String>,
    pub value: String,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlLink {
    pub node_id: String,
    pub kind: String,
    pub attribute: String,
    pub destination: String,
    pub remote: bool,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlTable {
    pub node_id: String,
    pub rows: Vec<XmlTableRow>,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlTableRow {
    pub node_id: String,
    pub cells: Vec<XmlTableCell>,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlTableCell {
    pub node_id: String,
    pub text: String,
    pub header: bool,
    pub row_span: usize,
    pub column_span: usize,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlMediaReference {
    pub node_id: String,
    pub kind: String,
    pub destination: Option<String>,
    pub media_type: Option<String>,
    pub description: Option<String>,
    pub remote: bool,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlSection {
    pub node_id: String,
    pub kind: String,
    pub title: Option<String>,
    pub label: Option<String>,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XmlEntity {
    pub kind: XmlEntityKind,
    pub name: String,
    pub raw: String,
    pub value: Option<String>,
    pub disposition: XmlEntityDisposition,
    pub locator: SourceLocator,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum XmlEntityKind {
    Predefined,
    Numeric,
    Named,
    Declaration,
    ExternalDeclaration,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum XmlEntityDisposition {
    DecodedSafe,
    Inert,
    RejectedExternalResolution,
}
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct XmlOptions {
    pub dialect: XmlDialect,
    pub encoding: Option<String>,
    pub retain_comments: bool,
}
impl Default for XmlOptions {
    fn default() -> Self {
        Self {
            dialect: XmlDialect::Auto,
            encoding: None,
            retain_comments: true,
        }
    }
}
impl crate::core::FormatOptions for XmlOptions {
    const FORMAT: &'static str = "xml";
}
pub type XmlEnvelope = Envelope<XmlDocument>;
pub fn parse_xml(text: &str, source: SourceInfo, options: &XmlOptions) -> XmlEnvelope {
    parse_xml_bytes(text.as_bytes(), source, options)
}
pub fn parse_xml_bytes(bytes: &[u8], source: SourceInfo, options: &XmlOptions) -> XmlEnvelope {
    match decode_text(bytes, &decode_options(&source, options)) {
        Ok(d) => envelope_from_decoded(&d, source, options),
        Err(e) => failed_decode(bytes, source, options, e),
    }
}
pub(crate) fn decode_options(source: &SourceInfo, options: &XmlOptions) -> DecodeOptions {
    let mut d = DecodeOptions::for_media_type(source.declared_mime_type.as_deref(), Some("xml"));
    d.context = DecodeContext::Xml;
    if let Some(e) = &options.encoding {
        d.transport_encoding = Some(e.clone())
    }
    d
}
pub(crate) fn parser_info() -> ParserInfo {
    ParserInfo::new("grist.xml")
        .with_implementation("quick-xml", "0.37.5")
        .with_specification_version("XML 1.0/1.1 + JATS 1.3 structural profile")
        .with_feature("xml")
}
fn envelope_from_decoded(d: &DecodedText, source: SourceInfo, o: &XmlOptions) -> XmlEnvelope {
    let (p, mut ds) = document_from_decoded(d, &source, o);
    let partial = d.report.makes_operation_partial() || ds.iter().any(|x| x.partial);
    let mut all = d.report.diagnostics.clone();
    all.append(&mut ds);
    let digest = options_digest(o).expect("XML options serialize");
    let mut e = if partial {
        Envelope::partial(
            OperationKind::Parse,
            ArtifactKind::Xml,
            source,
            parser_info(),
            digest,
            SchemaVersion::XML_V1,
            Some(p),
        )
    } else {
        Envelope::complete(
            OperationKind::Parse,
            ArtifactKind::Xml,
            source,
            parser_info(),
            digest,
            SchemaVersion::XML_V1,
            p,
        )
    };
    e.diagnostics = all;
    e.provenance
        .push(crate::text::decoding_provenance(&d.report));
    let mt = if e
        .payload
        .as_ref()
        .is_some_and(|p| p.dialect == XmlDialect::Jats)
    {
        "application/jats+xml"
    } else {
        "application/xml"
    };
    e.with_identity(
        ContentIdentity::for_raw_bytes(d.raw_bytes())
            .with_decoded(&d.text, d.report.encoding.label(), d.report.is_lossy())
            .with_format(FormatIdentity::new("xml", Some(mt))),
    )
    .with_canonical_payload_identity()
    .expect("XML canonicalization")
}
fn failed_decode(
    bytes: &[u8],
    source: SourceInfo,
    o: &XmlOptions,
    error: DecodeError,
) -> XmlEnvelope {
    Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::Xml,
        OperationStatus::Failed,
        source,
        parser_info(),
        options_digest(o).expect("XML options"),
        SchemaVersion::XML_V1,
    )
    .expect("terminal")
    .with_identity(
        ContentIdentity::for_raw_bytes(bytes)
            .with_format(FormatIdentity::new("xml", Some("application/xml"))),
    )
    .with_diagnostics(vec![error.diagnostic().with_parser("grist.xml")])
}
pub(crate) fn payload_node_count(d: &XmlDocument) -> usize {
    d.nodes.len()
        + d.nodes.iter().map(|n| n.attributes.len()).sum::<usize>()
        + d.entities.len()
        + d.scholarly_links.as_ref().map_or(0, |links| {
            links.targets.len() + links.relationships.len() + links.labels.len()
        })
        + 1
}

struct Frame {
    index: usize,
    name: String,
    path: String,
    ns: BTreeMap<String, String>,
    counts: HashMap<String, usize>,
    text: usize,
    comment: usize,
    pi: usize,
    entity: usize,
}
impl Frame {
    fn element_path(&mut self, n: &str) -> String {
        let c = self.counts.entry(n.into()).or_insert(0);
        *c += 1;
        format!("{}/{}[{}]", self.path, n, c)
    }
    fn content_path(&mut self, k: XmlNodeKind) -> String {
        let (l, c) = match k {
            XmlNodeKind::Text | XmlNodeKind::Cdata => ("text()", &mut self.text),
            XmlNodeKind::Comment => ("comment()", &mut self.comment),
            XmlNodeKind::ProcessingInstruction => ("processing-instruction()", &mut self.pi),
            XmlNodeKind::EntityReference => ("entity-reference()", &mut self.entity),
            _ => ("node()", &mut self.text),
        };
        *c += 1;
        format!("{}/{}[{}]", self.path, l, c)
    }
}
struct State<'a> {
    d: &'a DecodedText,
    lines: LineIndex,
    nodes: Vec<XmlNode>,
    roots: Vec<String>,
    stack: Vec<Frame>,
    root_counts: HashMap<String, usize>,
    doc_counts: [usize; 4],
    entities: Vec<XmlEntity>,
    diags: Vec<Diagnostic>,
    decl: Option<XmlDeclaration>,
    well: bool,
}
impl<'a> State<'a> {
    fn new(d: &'a DecodedText) -> Self {
        Self {
            d,
            lines: LineIndex::new(&d.text),
            nodes: vec![],
            roots: vec![],
            stack: vec![],
            root_counts: HashMap::new(),
            doc_counts: [0; 4],
            entities: vec![],
            diags: vec![],
            decl: None,
            well: true,
        }
    }
    fn range(&self, s: usize, e: usize) -> SourceRange {
        SourceRange::new(s, e, &self.lines)
    }
    fn locator(&self, r: SourceRange, p: &str) -> SourceLocator {
        SourceLocator::exact(r)
            .unwrap()
            .nested(LocationComponent::XmlPath { path: p.into() })
            .unwrap()
    }
    fn raw_range(&self, s: usize, e: usize) -> Option<RawByteRange> {
        self.d
            .raw_range_for_decoded(DecodedByteRange::from_usize(s, e))
    }
    fn parent(&self) -> Option<String> {
        self.stack.last().map(|f| self.nodes[f.index].id.clone())
    }
    fn push(&mut self, n: XmlNode) -> usize {
        let i = self.nodes.len();
        if let Some(f) = self.stack.last() {
            self.nodes[f.index].children.push(n.id.clone())
        }
        self.nodes.push(n);
        i
    }
    fn element_path(&mut self, n: &str) -> String {
        if let Some(f) = self.stack.last_mut() {
            f.element_path(n)
        } else {
            let c = self.root_counts.entry(n.into()).or_insert(0);
            *c += 1;
            format!("/{}[{}]", n, c)
        }
    }
    fn content_path(&mut self, k: XmlNodeKind) -> String {
        if let Some(f) = self.stack.last_mut() {
            return f.content_path(k);
        }
        let (i, l) = match k {
            XmlNodeKind::Text | XmlNodeKind::Cdata => (0, "text()"),
            XmlNodeKind::Comment => (1, "comment()"),
            XmlNodeKind::ProcessingInstruction => (2, "processing-instruction()"),
            XmlNodeKind::EntityReference => (3, "entity-reference()"),
            _ => (0, "node()"),
        };
        self.doc_counts[i] += 1;
        format!("/{}[{}]", l, self.doc_counts[i])
    }
    fn diagnostic(
        &self,
        code: &str,
        msg: impl Into<String>,
        s: usize,
        e: usize,
        path: &str,
        class: DiagnosticClass,
    ) -> Diagnostic {
        let r = self.range(s, e);
        let mut d = Diagnostic::warning("grist.xml", code, msg)
            .with_module("grist.xml")
            .with_range(r.clone())
            .with_locator(self.locator(r, path))
            .with_explanation_key(code)
            .partial();
        d.class = class;
        d
    }
    fn text_node(&mut self, k: XmlNodeKind, s: usize, e: usize, text: String) {
        if s == e {
            return;
        }
        let p = self.content_path(k);
        let r = self.range(s, e);
        let n = XmlNode {
            id: p.clone(),
            kind: k,
            range: r.clone(),
            locator: self.locator(r, &p),
            raw_range: self.raw_range(s, e),
            xml_path: p,
            depth: self.stack.len(),
            parent_id: self.parent(),
            children: vec![],
            qualified_name: None,
            local_name: None,
            prefix: None,
            namespace_uri: None,
            namespace_bindings: vec![],
            attributes: vec![],
            text: Some(text),
            raw: self.d.text[s..e].into(),
            self_closing: false,
            known_jats_element: false,
            recovered: false,
        };
        self.push(n);
    }
    fn raw_unknown(&mut self, s: usize, e: usize, msg: &str) {
        if s >= e {
            return;
        }
        let p = format!("/_recovery[{}]", self.nodes.len() + 1);
        let r = self.range(s, e);
        let n = XmlNode {
            id: p.clone(),
            kind: XmlNodeKind::RawUnknown,
            range: r.clone(),
            locator: self.locator(r, &p),
            raw_range: self.raw_range(s, e),
            xml_path: p.clone(),
            depth: self.stack.len(),
            parent_id: self.parent(),
            children: vec![],
            qualified_name: None,
            local_name: None,
            prefix: None,
            namespace_uri: None,
            namespace_bindings: vec![],
            attributes: vec![],
            text: None,
            raw: self.d.text[s..e].into(),
            self_closing: false,
            known_jats_element: false,
            recovered: true,
        };
        self.push(n);
        self.diags.push(self.diagnostic(
            "xml.parse.malformed",
            msg,
            s,
            e,
            &p,
            DiagnosticClass::MalformedInput,
        ));
        self.well = false
    }
}
impl State<'_> {
    fn start(&mut self, s: usize, e: usize, empty: bool) {
        let raw = &self.d.text[s..e];
        let Some(name) = markup_name(raw) else {
            self.raw_unknown(s, e, "start tag has no name");
            return;
        };
        let name = name.to_string();
        let path = self.element_path(&name);
        let mut ns = self.stack.last().map(|f| f.ns.clone()).unwrap_or_else(|| {
            BTreeMap::from([("xml".into(), "http://www.w3.org/XML/1998/namespace".into())])
        });
        let lex = lex_attributes(raw, s);
        for a in &lex.attrs {
            if a.name == "xmlns" {
                ns.insert(String::new(), a.value.clone());
            } else if let Some(p) = a.name.strip_prefix("xmlns:") {
                ns.insert(p.into(), a.value.clone());
            }
        }
        let (pref, local) = split_name(&name);
        let uri = ns.get(pref.as_deref().unwrap_or("")).cloned();
        if pref.is_some() && uri.is_none() {
            self.diags.push(self.diagnostic(
                "xml.namespace.unbound_prefix",
                format!("element <{name}> has an unbound prefix"),
                s,
                e,
                &path,
                DiagnosticClass::MalformedInput,
            ));
            self.well = false
        }
        let mut attrs = vec![];
        for a in lex.attrs {
            let decl = a.name == "xmlns" || a.name.starts_with("xmlns:");
            let (ap, al) = split_name(&a.name);
            let au = if decl {
                Some("http://www.w3.org/2000/xmlns/".into())
            } else {
                ap.as_ref().and_then(|p| ns.get(p)).cloned()
            };
            if ap.is_some() && au.is_none() && !decl {
                self.diags.push(self.diagnostic(
                    "xml.namespace.unbound_attribute_prefix",
                    format!("attribute {} has an unbound prefix", a.name),
                    a.start,
                    a.end,
                    &format!("{path}/@{}", a.name),
                    DiagnosticClass::MalformedInput,
                ));
                self.well = false
            }
            if let Some(value_start) = a.value_start {
                for reference in references(&a.value) {
                    let start = value_start + reference.start;
                    let end = value_start + reference.end;
                    let range = self.range(start, end);
                    let (value, kind, disposition) = entity_value(&reference.name);
                    let entity_path = format!(
                        "{path}/@{}/entity-reference()[{}]",
                        a.name,
                        self.entities.len() + 1
                    );
                    self.entities.push(XmlEntity {
                        kind,
                        name: reference.name.clone(),
                        raw: a.value[reference.start..reference.end].into(),
                        value,
                        disposition,
                        locator: self.locator(range, &entity_path),
                    });
                    if disposition == XmlEntityDisposition::Inert {
                        self.diags.push(self.diagnostic(
                            "xml.entity.unresolved",
                            format!(
                                "entity &{}; in attribute {} is preserved but not expanded",
                                reference.name, a.name
                            ),
                            start,
                            end,
                            &entity_path,
                            DiagnosticClass::UnsupportedContent,
                        ));
                    }
                }
            }
            let r = self.range(a.start, a.end);
            attrs.push(XmlAttribute {
                qualified_name: a.name.clone(),
                local_name: al,
                prefix: ap,
                namespace_uri: au,
                value: decode_entities(&a.value).0,
                raw_value: a.value.clone(),
                quote: a.quote,
                range: r.clone(),
                name_range: self.range(a.name_start, a.name_end),
                value_range: a
                    .value_start
                    .zip(a.value_end)
                    .map(|(x, y)| self.range(x, y)),
                locator: self.locator(r, &format!("{path}/@{}", a.name)),
                namespace_declaration: decl,
            });
        }
        let mut seen_attributes = std::collections::HashSet::new();
        for attribute in &attrs {
            if !seen_attributes.insert((
                attribute.namespace_uri.clone(),
                attribute.local_name.clone(),
            )) {
                self.diags.push(self.diagnostic(
                    "xml.attribute.duplicate",
                    format!("duplicate attribute {}", attribute.qualified_name),
                    attribute.range.byte_start,
                    attribute.range.byte_end,
                    &format!("{path}/@{}", attribute.qualified_name),
                    DiagnosticClass::MalformedInput,
                ));
                self.well = false;
            }
        }
        for x in lex.issues {
            self.diags.push(self.diagnostic(
                x.code,
                x.msg,
                x.start,
                x.end,
                &path,
                DiagnosticClass::MalformedInput,
            ));
            self.well = false
        }
        let bindings = ns
            .iter()
            .map(|(p, u)| XmlNamespaceBinding {
                prefix: (!p.is_empty()).then(|| p.clone()),
                uri: u.clone(),
                declared_here: attrs.iter().any(|a| {
                    (p.is_empty() && a.qualified_name == "xmlns")
                        || a.qualified_name == format!("xmlns:{p}")
                }),
            })
            .collect();
        let r = self.range(s, e);
        let id = path.clone();
        let i = self.push(XmlNode {
            id: id.clone(),
            kind: XmlNodeKind::Element,
            range: r.clone(),
            locator: self.locator(r, &path),
            raw_range: self.raw_range(s, e),
            xml_path: path.clone(),
            depth: self.stack.len(),
            parent_id: self.parent(),
            children: vec![],
            qualified_name: Some(name.clone()),
            local_name: Some(local.clone()),
            prefix: pref,
            namespace_uri: uri,
            namespace_bindings: bindings,
            attributes: attrs,
            text: None,
            raw: raw.into(),
            self_closing: empty,
            known_jats_element: known_jats(&local),
            recovered: false,
        });
        if self.stack.is_empty() {
            self.roots.push(id)
        }
        if !empty {
            self.stack.push(Frame {
                index: i,
                name,
                path,
                ns,
                counts: HashMap::new(),
                text: 0,
                comment: 0,
                pi: 0,
                entity: 0,
            })
        }
    }
    fn end(&mut self, s: usize, e: usize) {
        let found = markup_name(&self.d.text[s..e]).unwrap_or("").to_string();
        let Some(position) = self.stack.iter().rposition(|frame| frame.name == found) else {
            self.raw_unknown(s, e, "unexpected XML end tag");
            return;
        };
        while self.stack.len() > position + 1 {
            let frame = self.stack.pop().expect("frame above matching end tag");
            self.diags.push(self.diagnostic(
                "xml.element.unclosed",
                format!(
                    "element <{}> was implicitly closed before </{}>",
                    frame.name, found
                ),
                self.nodes[frame.index].range.byte_start,
                s,
                &frame.path,
                DiagnosticClass::MalformedInput,
            ));
            let start = self.nodes[frame.index].range.byte_start;
            let range = self.range(start, s);
            let locator = self.locator(range.clone(), &frame.path);
            let raw_range = self.raw_range(start, s);
            let raw = self.d.text[start..s].to_string();
            let node = &mut self.nodes[frame.index];
            node.range = range;
            node.locator = locator;
            node.raw_range = raw_range;
            node.raw = raw;
            node.recovered = true;
            self.well = false;
        }
        let frame = self.stack.pop().expect("matching end-tag frame");
        let start = self.nodes[frame.index].range.byte_start;
        let range = self.range(start, e);
        let locator = self.locator(range.clone(), &frame.path);
        let raw_range = self.raw_range(start, e);
        let raw = self.d.text[start..e].to_string();
        let node = &mut self.nodes[frame.index];
        node.range = range;
        node.locator = locator;
        node.raw_range = raw_range;
        node.raw = raw;
    }
    fn text_entities(&mut self, s: usize, e: usize) {
        let raw = self.d.text[s..e].to_string();
        let refs = references(&raw);
        let mut at = 0;
        for x in refs {
            if x.start > at {
                self.text_node(
                    XmlNodeKind::Text,
                    s + at,
                    s + x.start,
                    decode_entities(&raw[at..x.start]).0,
                )
            }
            let es = s + x.start;
            let ee = s + x.end;
            let p = self.content_path(XmlNodeKind::EntityReference);
            let r = self.range(es, ee);
            let (value, kind, disp) = entity_value(&x.name);
            let n = XmlNode {
                id: p.clone(),
                kind: XmlNodeKind::EntityReference,
                range: r.clone(),
                locator: self.locator(r.clone(), &p),
                raw_range: self.raw_range(es, ee),
                xml_path: p,
                depth: self.stack.len(),
                parent_id: self.parent(),
                children: vec![],
                qualified_name: Some(x.name.clone()),
                local_name: Some(x.name.clone()),
                prefix: None,
                namespace_uri: None,
                namespace_bindings: vec![],
                attributes: vec![],
                text: value.clone(),
                raw: raw[x.start..x.end].into(),
                self_closing: false,
                known_jats_element: false,
                recovered: false,
            };
            self.push(n);
            let ep = format!("/entities/entity[{}]", self.entities.len() + 1);
            self.entities.push(XmlEntity {
                kind,
                name: x.name.clone(),
                raw: raw[x.start..x.end].into(),
                value,
                disposition: disp,
                locator: self.locator(r, &ep),
            });
            if disp == XmlEntityDisposition::Inert {
                self.diags.push(self.diagnostic(
                    "xml.entity.unresolved",
                    format!("entity &{}; is preserved but not expanded", x.name),
                    es,
                    ee,
                    &ep,
                    DiagnosticClass::UnsupportedContent,
                ))
            }
            at = x.end
        }
        if at < raw.len() {
            self.text_node(XmlNodeKind::Text, s + at, e, decode_entities(&raw[at..]).0)
        }
    }
    fn doctype(&mut self, s: usize, e: usize) {
        let raw = self.d.text[s..e].to_string();
        let p = self.content_path(XmlNodeKind::Doctype);
        let r = self.range(s, e);
        let n = XmlNode {
            id: p.clone(),
            kind: XmlNodeKind::Doctype,
            range: r.clone(),
            locator: self.locator(r, &p),
            raw_range: self.raw_range(s, e),
            xml_path: p,
            depth: self.stack.len(),
            parent_id: self.parent(),
            children: vec![],
            qualified_name: None,
            local_name: None,
            prefix: None,
            namespace_uri: None,
            namespace_bindings: vec![],
            attributes: vec![],
            text: None,
            raw: raw.clone(),
            self_closing: false,
            known_jats_element: false,
            recovered: false,
        };
        self.push(n);
        for d in declarations(&raw) {
            let r = self.range(s + d.start, s + d.end);
            let p = format!("/entities/entity[{}]", self.entities.len() + 1);
            self.entities.push(XmlEntity {
                kind: if d.external {
                    XmlEntityKind::ExternalDeclaration
                } else {
                    XmlEntityKind::Declaration
                },
                name: d.name,
                raw: raw[d.start..d.end].into(),
                value: None,
                disposition: if d.external {
                    XmlEntityDisposition::RejectedExternalResolution
                } else {
                    XmlEntityDisposition::Inert
                },
                locator: self.locator(r, &p),
            })
        }
    }
    fn pi(&mut self, s: usize, e: usize) {
        let raw = self.d.text[s..e].to_string();
        self.text_node(
            XmlNodeKind::ProcessingInstruction,
            s,
            e,
            raw.trim_start_matches("<?").trim_end_matches("?>").into(),
        );
        let p = self
            .nodes
            .last()
            .map(|node| node.xml_path.clone())
            .unwrap_or_else(|| "/processing-instruction()[1]".into());
        if raw.starts_with("<?xml") && self.decl.is_none() {
            let r = self.range(s, e);
            self.decl = Some(XmlDeclaration {
                version: pseudo(&raw, "version"),
                encoding: pseudo(&raw, "encoding"),
                standalone: pseudo(&raw, "standalone"),
                raw,
                range: r.clone(),
                locator: self.locator(r, &p),
            })
        }
    }
    fn unclosed(&mut self) {
        let end = self.d.text.len();
        while let Some(f) = self.stack.pop() {
            let s = self.nodes[f.index].range.byte_start;
            self.diags.push(self.diagnostic(
                "xml.element.unclosed",
                format!("element <{}> is unclosed", f.name),
                s,
                end,
                &f.path,
                DiagnosticClass::MalformedInput,
            ));
            let r = self.range(s, end);
            let loc = self.locator(r.clone(), &f.path);
            let rr = self.raw_range(s, end);
            let raw = self.d.text[s..end].to_string();
            let n = &mut self.nodes[f.index];
            n.range = r;
            n.locator = loc;
            n.raw_range = rr;
            n.raw = raw;
            n.recovered = true;
            self.well = false
        }
    }
}
pub(crate) fn document_from_decoded(
    d: &DecodedText,
    _: &SourceInfo,
    o: &XmlOptions,
) -> (XmlDocument, Vec<Diagnostic>) {
    let mut s = State::new(d);
    let mut r = Reader::from_str(&d.text);
    r.config_mut().trim_text(false);
    r.config_mut().check_end_names = false;
    r.config_mut().expand_empty_elements = false;
    let mut prev = 0usize;
    loop {
        let ev = r.read_event();
        let end = usize::try_from(r.buffer_position())
            .unwrap_or(d.text.len())
            .min(d.text.len());
        match ev {
            Ok(Event::Start(_)) => s.start(prev, end, false),
            Ok(Event::Empty(_)) => s.start(prev, end, true),
            Ok(Event::End(_)) => s.end(prev, end),
            Ok(Event::Text(_)) => s.text_entities(prev, end),
            Ok(Event::CData(_)) => {
                let raw = &d.text[prev..end];
                let text = raw
                    .strip_prefix("<![CDATA[")
                    .and_then(|x| x.strip_suffix("]]>"))
                    .unwrap_or(raw)
                    .into();
                s.text_node(XmlNodeKind::Cdata, prev, end, text)
            }
            Ok(Event::Comment(_)) => {
                if o.retain_comments {
                    let raw = &d.text[prev..end];
                    let text = raw
                        .strip_prefix("<!--")
                        .and_then(|x| x.strip_suffix("-->"))
                        .unwrap_or(raw)
                        .into();
                    s.text_node(XmlNodeKind::Comment, prev, end, text)
                }
            }
            Ok(Event::Decl(_)) | Ok(Event::PI(_)) => s.pi(prev, end),
            Ok(Event::DocType(_)) => s.doctype(prev, end),
            Ok(Event::Eof) => break,
            Err(e) => {
                s.raw_unknown(
                    prev,
                    d.text.len(),
                    &format!("XML parser stopped at malformed input: {e}"),
                );
                break;
            }
        }
        prev = end
    }
    s.unclosed();
    let top_level_text = s
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.parent_id.is_none()
                && node.kind == XmlNodeKind::Text
                && node
                    .text
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty())
        })
        .map(|(index, node)| (index, node.range.clone(), node.xml_path.clone()))
        .collect::<Vec<_>>();
    for (index, range, path) in top_level_text {
        s.nodes[index].kind = XmlNodeKind::RawUnknown;
        s.nodes[index].recovered = true;
        s.diags.push(s.diagnostic(
            "xml.document.text_outside_root",
            "non-whitespace text outside the root element was retained as recovery content",
            range.byte_start,
            range.byte_end,
            &path,
            DiagnosticClass::MalformedInput,
        ));
        s.well = false;
    }
    if s.roots.len() != 1 {
        let rr = s.range(0, d.text.len());
        let mut x = Diagnostic::warning(
            "grist.xml",
            "xml.document.root_count",
            format!("expected one root element; found {}", s.roots.len()),
        )
        .with_range(rr.clone())
        .with_locator(s.locator(rr, "/"))
        .with_explanation_key("xml.document.root_count")
        .partial();
        x.class = DiagnosticClass::MalformedInput;
        s.diags.push(x);
        s.well = false
    }
    let findings = inspect_xml(d.text.as_bytes(), &Default::default());
    for f in &findings {
        let a = usize::try_from(f.byte_start).unwrap_or(0).min(d.text.len());
        let b = usize::try_from(f.byte_end)
            .unwrap_or(d.text.len())
            .min(d.text.len())
            .max(a);
        let path = match f.kind {
            XmlSecurityFindingKind::Doctype => "/doctype()",
            XmlSecurityFindingKind::EntityDeclaration => "/entities",
            XmlSecurityFindingKind::XInclude => "/xinclude",
            XmlSecurityFindingKind::RemoteSchemaLocation => "/schema-location",
        };
        s.diags.push(s.diagnostic(
            &f.code,
            &f.message,
            a,
            b,
            path,
            DiagnosticClass::SecurityRejection,
        ))
    }
    semantic_security(&mut s);
    let dialect = dialect(o.dialect, &s.nodes);
    let metadata = metadata(&s.nodes, dialect);
    let links = links(&s.nodes);
    let tables = tables(&s.nodes);
    let media = media(&s.nodes);
    let sections = sections(&s.nodes, dialect);
    let scholarly_links = if dialect == XmlDialect::Jats {
        let (links, diagnostics) = crate::jats::resolve_scholarly_links(&s.nodes);
        s.diags.extend(diagnostics);
        Some(links)
    } else {
        None
    };
    let dr = s.range(0, d.text.len());
    let loc = s.locator(dr.clone(), "/");
    let doc = XmlDocument {
        schema_version: SchemaVersion::XML_V1.into(),
        raw_bytes: d.raw_bytes().to_vec(),
        raw_range: RawByteRange::from_usize(0, d.raw_bytes().len()),
        decoded_text: d.text.clone(),
        decoded_range: dr,
        locator: loc,
        encoding: d.report.encoding.clone(),
        decoding: d.report.clone(),
        dialect,
        declaration: s.decl,
        nodes: s.nodes,
        root_element_ids: s.roots,
        metadata,
        links,
        tables,
        media,
        sections,
        scholarly_links,
        entities: s.entities,
        security_findings: findings,
        well_formed: s.well,
    };
    (doc, s.diags)
}
fn semantic_security(s: &mut State<'_>) {
    let nodes = s.nodes.clone();
    for n in nodes {
        if n.kind != XmlNodeKind::Element {
            continue;
        }
        if n.local_name.as_deref() == Some("include")
            && n.namespace_uri.as_deref() == Some("http://www.w3.org/2001/XInclude")
        {
            s.diags.push(s.diagnostic(
                "grist.security.xml.xinclude",
                "XInclude processing is disabled; the element remains inert",
                n.range.byte_start,
                n.range.byte_end,
                &n.xml_path,
                DiagnosticClass::SecurityRejection,
            ))
        }
        for a in &n.attributes {
            if matches!(
                a.local_name.as_str(),
                "schemaLocation" | "noNamespaceSchemaLocation"
            ) && a.value.contains("://")
            {
                s.diags.push(s.diagnostic(
                    "grist.security.xml.remote_schema",
                    "remote schema resolution is disabled; the URI remains metadata",
                    a.range.byte_start,
                    a.range.byte_end,
                    &format!("{}/@{}", n.xml_path, a.qualified_name),
                    DiagnosticClass::SecurityRejection,
                ))
            }
        }
    }
}

struct Lex {
    attrs: Vec<LexAttr>,
    issues: Vec<LexIssue>,
}
struct LexAttr {
    name: String,
    value: String,
    quote: Option<char>,
    start: usize,
    end: usize,
    name_start: usize,
    name_end: usize,
    value_start: Option<usize>,
    value_end: Option<usize>,
}
struct LexIssue {
    code: &'static str,
    msg: String,
    start: usize,
    end: usize,
}
fn lex_attributes(raw: &str, base: usize) -> Lex {
    let b = raw.as_bytes();
    let mut i = 1;
    if b.get(i) == Some(&b'/') {
        i += 1
    }
    while i < b.len() && !space(b[i]) && !matches!(b[i], b'>' | b'/') {
        i += 1
    }
    let mut attrs = vec![];
    let mut issues = vec![];
    while i < b.len() {
        while i < b.len() && space(b[i]) {
            i += 1
        }
        if i >= b.len() || matches!(b[i], b'>' | b'/') {
            break;
        }
        let start = i;
        let ns = i;
        while i < b.len() && !space(b[i]) && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1
        }
        let ne = i;
        let name = raw[ns..ne].to_string();
        while i < b.len() && space(b[i]) {
            i += 1
        }
        if b.get(i) != Some(&b'=') {
            issues.push(LexIssue {
                code: "xml.attribute.missing_value",
                msg: format!("attribute {name} has no value"),
                start: base + start,
                end: base + i.max(ne),
            });
            attrs.push(LexAttr {
                name,
                value: String::new(),
                quote: None,
                start: base + start,
                end: base + i.max(ne),
                name_start: base + ns,
                name_end: base + ne,
                value_start: None,
                value_end: None,
            });
            continue;
        }
        i += 1;
        while i < b.len() && space(b[i]) {
            i += 1
        }
        let q = b.get(i).copied().filter(|x| matches!(x, b'\'' | b'"'));
        let (vs, ve, end, quote) = if let Some(q) = q {
            i += 1;
            let vs = i;
            while i < b.len() && b[i] != q {
                i += 1
            }
            let ve = i;
            if i < b.len() {
                i += 1
            } else {
                issues.push(LexIssue {
                    code: "xml.attribute.unclosed_quote",
                    msg: format!("attribute {name} has an unclosed value"),
                    start: base + start,
                    end: base + i,
                })
            }
            (vs, ve, i, Some(char::from(q)))
        } else {
            let vs = i;
            while i < b.len() && !space(b[i]) && b[i] != b'>' {
                i += 1
            }
            issues.push(LexIssue {
                code: "xml.attribute.unquoted",
                msg: format!("attribute {name} has an unquoted value"),
                start: base + start,
                end: base + i,
            });
            (vs, i, i, None)
        };
        attrs.push(LexAttr {
            name,
            value: raw[vs..ve].into(),
            quote,
            start: base + start,
            end: base + end,
            name_start: base + ns,
            name_end: base + ne,
            value_start: Some(base + vs),
            value_end: Some(base + ve),
        })
    }
    Lex { attrs, issues }
}
fn markup_name(raw: &str) -> Option<&str> {
    let b = raw.as_bytes();
    let mut s = 1;
    if b.get(s) == Some(&b'/') {
        s += 1
    }
    while b.get(s).is_some_and(|x| space(*x)) {
        s += 1
    }
    let mut e = s;
    while e < b.len() && !space(b[e]) && !matches!(b[e], b'>' | b'/') {
        e += 1
    }
    (e > s).then(|| &raw[s..e])
}
fn split_name(n: &str) -> (Option<String>, String) {
    n.split_once(':')
        .map(|(p, l)| (Some(p.into()), l.into()))
        .unwrap_or((None, n.into()))
}
fn space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}
struct Ref {
    name: String,
    start: usize,
    end: usize,
}
fn references(raw: &str) -> Vec<Ref> {
    let mut v = vec![];
    let mut at = 0;
    while let Some(x) = raw[at..].find('&') {
        let s = at + x;
        let Some(z) = raw[s + 1..].find(';') else {
            break;
        };
        let e = s + z + 2;
        let name = &raw[s + 1..e - 1];
        if !name.is_empty() && !name.bytes().any(space) {
            v.push(Ref {
                name: name.into(),
                start: s,
                end: e,
            })
        }
        at = e
    }
    v
}
fn entity_value(n: &str) -> (Option<String>, XmlEntityKind, XmlEntityDisposition) {
    let p = match n {
        "amp" => Some("&"),
        "lt" => Some("<"),
        "gt" => Some(">"),
        "apos" => Some("'"),
        "quot" => Some("\""),
        _ => None,
    };
    if let Some(x) = p {
        return (
            Some(x.into()),
            XmlEntityKind::Predefined,
            XmlEntityDisposition::DecodedSafe,
        );
    }
    let number = if let Some(x) = n.strip_prefix("#x").or_else(|| n.strip_prefix("#X")) {
        u32::from_str_radix(x, 16).ok()
    } else {
        n.strip_prefix('#').and_then(|x| x.parse().ok())
    };
    if let Some(c) = number.and_then(char::from_u32) {
        return (
            Some(c.to_string()),
            XmlEntityKind::Numeric,
            XmlEntityDisposition::DecodedSafe,
        );
    }
    (None, XmlEntityKind::Named, XmlEntityDisposition::Inert)
}
fn decode_entities(raw: &str) -> (String, bool) {
    let refs = references(raw);
    let mut out = String::new();
    let mut at = 0;
    let mut unresolved = false;
    for r in refs {
        out.push_str(&raw[at..r.start]);
        if let (Some(v), _, _) = entity_value(&r.name) {
            out.push_str(&v)
        } else {
            out.push_str(&raw[r.start..r.end]);
            unresolved = true
        }
        at = r.end
    }
    out.push_str(&raw[at..]);
    (out, unresolved)
}
struct Declaration {
    name: String,
    start: usize,
    end: usize,
    external: bool,
}
fn declarations(raw: &str) -> Vec<Declaration> {
    let mut v = vec![];
    let mut at = 0;
    while let Some(x) = raw[at..].find("<!ENTITY") {
        let s = at + x;
        let e = raw[s..].find('>').map(|x| s + x + 1).unwrap_or(raw.len());
        let body = &raw[s + 8..e];
        let name = body
            .trim_start()
            .trim_start_matches('%')
            .split_whitespace()
            .next()
            .unwrap_or("unknown")
            .into();
        let upper = body.to_ascii_uppercase();
        v.push(Declaration {
            name,
            start: s,
            end: e,
            external: upper.contains("SYSTEM") || upper.contains("PUBLIC"),
        });
        at = e
    }
    v
}
fn pseudo(raw: &str, n: &str) -> Option<String> {
    let key = format!("{n}=");
    let s = raw.find(&key)? + key.len();
    let q = *raw.as_bytes().get(s)?;
    if !matches!(q, b'\'' | b'"') {
        return None;
    }
    let a = s + 1;
    let e = raw[a..].find(char::from(q))? + a;
    Some(raw[a..e].into())
}
fn dialect(want: XmlDialect, nodes: &[XmlNode]) -> XmlDialect {
    if want != XmlDialect::Auto {
        return want;
    }
    if nodes
        .iter()
        .find(|n| n.kind == XmlNodeKind::Element && n.parent_id.is_none())
        .is_some_and(|n| {
            n.local_name.as_deref() == Some("article")
                || n.namespace_uri.as_deref().is_some_and(|u| {
                    u.to_ascii_lowercase().contains("jats") || u.contains("nlm.nih.gov")
                })
        })
    {
        XmlDialect::Jats
    } else {
        XmlDialect::Xml
    }
}
fn index(nodes: &[XmlNode]) -> HashMap<&str, usize> {
    nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect()
}
fn text(nodes: &[XmlNode], root: &XmlNode) -> String {
    let p = root.xml_path.clone() + "/";
    nodes
        .iter()
        .filter(|n| n.xml_path.starts_with(&p))
        .filter_map(|n| n.text.as_deref())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn attr<'a>(n: &'a XmlNode, names: &[&str]) -> Option<&'a XmlAttribute> {
    n.attributes.iter().find(|a| {
        names
            .iter()
            .any(|x| a.qualified_name == *x || a.local_name == *x)
    })
}
fn metadata(nodes: &[XmlNode], d: XmlDialect) -> Vec<XmlMetadata> {
    let names = [
        "article-title",
        "subtitle",
        "journal-title",
        "publisher-name",
        "article-id",
        "subject",
        "kwd",
        "surname",
        "given-names",
        "email",
        "year",
        "month",
        "day",
        "volume",
        "issue",
        "fpage",
        "lpage",
        "abstract",
        "title",
    ];
    nodes
        .iter()
        .filter(|n| {
            d == XmlDialect::Jats && n.local_name.as_deref().is_some_and(|x| names.contains(&x))
        })
        .map(|n| XmlMetadata {
            node_id: n.id.clone(),
            kind: n.local_name.clone().unwrap_or_default(),
            name: attr(n, &["pub-id-type", "article-id-type", "contrib-type"])
                .map(|a| a.value.clone()),
            value: text(nodes, n),
            locator: n.locator.clone(),
        })
        .collect()
}
fn links(nodes: &[XmlNode]) -> Vec<XmlLink> {
    let mut v = vec![];
    for n in nodes {
        for a in &n.attributes {
            if matches!(a.local_name.as_str(), "href" | "rid") {
                let x = a.value.trim().to_ascii_lowercase();
                v.push(XmlLink {
                    node_id: n.id.clone(),
                    kind: attr(n, &["ref-type"])
                        .map(|x| x.value.clone())
                        .unwrap_or_else(|| n.local_name.clone().unwrap_or_else(|| "link".into())),
                    attribute: a.qualified_name.clone(),
                    destination: a.value.clone(),
                    remote: x.starts_with("http://")
                        || x.starts_with("https://")
                        || x.starts_with("//"),
                    locator: a.locator.clone(),
                })
            }
        }
    }
    v
}
fn ancestor<'a>(
    n: &'a XmlNode,
    nodes: &'a [XmlNode],
    ix: &HashMap<&str, usize>,
    names: &[&str],
) -> Option<&'a str> {
    let mut p = n.parent_id.as_deref();
    while let Some(id) = p {
        let x = ix.get(id).and_then(|i| nodes.get(*i))?;
        if x.local_name.as_deref().is_some_and(|n| names.contains(&n)) {
            return Some(x.id.as_str());
        }
        p = x.parent_id.as_deref()
    }
    None
}
fn span(x: Option<&str>) -> usize {
    x.and_then(|x| x.parse().ok())
        .filter(|x| *x > 0)
        .unwrap_or(1)
}
fn tables(nodes: &[XmlNode]) -> Vec<XmlTable> {
    let ix = index(nodes);
    nodes
        .iter()
        .filter(|n| n.local_name.as_deref() == Some("table"))
        .map(|t| {
            let rows = nodes
                .iter()
                .filter(|n| {
                    n.local_name.as_deref() == Some("tr")
                        && ancestor(n, nodes, &ix, &["table"]) == Some(t.id.as_str())
                })
                .map(|r| {
                    let cells = r
                        .children
                        .iter()
                        .filter_map(|id| ix.get(id.as_str()).and_then(|i| nodes.get(*i)))
                        .filter(|n| matches!(n.local_name.as_deref(), Some("td" | "th")))
                        .map(|c| XmlTableCell {
                            node_id: c.id.clone(),
                            text: text(nodes, c),
                            header: c.local_name.as_deref() == Some("th"),
                            row_span: span(attr(c, &["rowspan"]).map(|a| a.value.as_str())),
                            column_span: span(attr(c, &["colspan"]).map(|a| a.value.as_str())),
                            locator: c.locator.clone(),
                        })
                        .collect();
                    XmlTableRow {
                        node_id: r.id.clone(),
                        cells,
                        locator: r.locator.clone(),
                    }
                })
                .collect();
            XmlTable {
                node_id: t.id.clone(),
                rows,
                locator: t.locator.clone(),
            }
        })
        .collect()
}
fn media(nodes: &[XmlNode]) -> Vec<XmlMediaReference> {
    nodes
        .iter()
        .filter(|n| {
            matches!(
                n.local_name.as_deref(),
                Some(
                    "graphic" | "inline-graphic" | "media" | "supplementary-material" | "self-uri"
                )
            )
        })
        .map(|n| {
            let dest = attr(n, &["href"]).map(|a| a.value.clone());
            let remote = dest.as_deref().is_some_and(|x| {
                let x = x.to_ascii_lowercase();
                x.starts_with("http://") || x.starts_with("https://") || x.starts_with("//")
            });
            XmlMediaReference {
                node_id: n.id.clone(),
                kind: n.local_name.clone().unwrap_or_default(),
                destination: dest,
                media_type: attr(n, &["mime-type", "mimetype", "type"]).map(|a| a.value.clone()),
                description: nodes
                    .iter()
                    .find(|x| {
                        x.xml_path.starts_with(&(n.xml_path.clone() + "/"))
                            && matches!(x.local_name.as_deref(), Some("alt-text" | "caption"))
                    })
                    .map(|x| text(nodes, x)),
                remote,
                locator: n.locator.clone(),
            }
        })
        .collect()
}
fn sections(nodes: &[XmlNode], d: XmlDialect) -> Vec<XmlSection> {
    nodes
        .iter()
        .filter(|n| {
            n.local_name.as_deref() == Some("sec")
                || (d == XmlDialect::Jats
                    && matches!(
                        n.local_name.as_deref(),
                        Some("front" | "body" | "back" | "abstract" | "ref-list")
                    ))
        })
        .map(|n| XmlSection {
            node_id: n.id.clone(),
            kind: n.local_name.clone().unwrap_or_default(),
            title: nodes
                .iter()
                .find(|x| {
                    x.parent_id.as_deref() == Some(n.id.as_str())
                        && x.local_name.as_deref() == Some("title")
                })
                .map(|x| text(nodes, x)),
            label: attr(n, &["id", "sec-type"]).map(|a| a.value.clone()),
            locator: n.locator.clone(),
        })
        .collect()
}
fn known_jats(n: &str) -> bool {
    matches!(
        n,
        "article"
            | "front"
            | "journal-meta"
            | "article-meta"
            | "article-id"
            | "title-group"
            | "article-title"
            | "subtitle"
            | "contrib-group"
            | "contrib"
            | "name"
            | "surname"
            | "given-names"
            | "email"
            | "aff"
            | "corresp"
            | "author-notes"
            | "funding-group"
            | "award-group"
            | "pub-date"
            | "year"
            | "month"
            | "day"
            | "volume"
            | "issue"
            | "fpage"
            | "lpage"
            | "abstract"
            | "kwd-group"
            | "kwd"
            | "body"
            | "back"
            | "sec"
            | "title"
            | "p"
            | "list"
            | "list-item"
            | "xref"
            | "ext-link"
            | "ref-list"
            | "ref"
            | "mixed-citation"
            | "element-citation"
            | "table-wrap"
            | "table-wrap-group"
            | "table-wrap-foot"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "th"
            | "td"
            | "fig"
            | "fig-group"
            | "caption"
            | "graphic"
            | "inline-graphic"
            | "media"
            | "supplementary-material"
            | "label"
            | "bold"
            | "italic"
            | "sub"
            | "sup"
            | "disp-formula"
            | "inline-formula"
            | "fn-group"
            | "fn"
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jats_structure_is_preserved() {
        let x = r#"<?xml version="1.0"?><article xmlns="http://jats.nlm.nih.gov" xmlns:xlink="http://www.w3.org/1999/xlink"><front><article-meta><title-group><article-title>Safe &amp; exact</article-title></title-group></article-meta></front><body><sec id="s1"><title>Methods</title><table-wrap><table><tr><th>A</th><td rowspan="2">B</td></tr></table></table-wrap><fig><graphic xlink:href="figure.png"/></fig><custom:opaque xmlns:custom="urn:custom" custom:a="1">raw</custom:opaque></sec></body></article>"#;
        let e = parse_xml(x, SourceInfo::stdin("article.xml"), &Default::default());
        assert_eq!(e.status, OperationStatus::Complete, "{:#?}", e.diagnostics);
        let d = e.payload.unwrap();
        assert_eq!(d.dialect, XmlDialect::Jats);
        assert!(d.nodes.iter().all(|n| n.locator.validate().is_ok()));
        assert!(
            d.nodes
                .iter()
                .any(|n| n.qualified_name.as_deref() == Some("custom:opaque")
                    && !n.known_jats_element
                    && n.namespace_uri.as_deref() == Some("urn:custom")
                    && n.raw.contains("custom:a=\"1\""))
        );
        assert!(d.metadata.iter().any(|m| m.value == "Safe & exact"));
        assert_eq!(d.tables[0].rows[0].cells[1].row_span, 2);
        assert_eq!(d.media[0].destination.as_deref(), Some("figure.png"));
        assert!(
            d.sections
                .iter()
                .any(|s| s.title.as_deref() == Some("Methods"))
        )
    }
    #[test]
    fn hostile_and_malformed_are_inert() {
        let x = r#"<!DOCTYPE article [<!ENTITY xxe SYSTEM "file:///secret"><!ENTITY local "safe">]><article xmlns:xi="http://www.w3.org/2001/XInclude" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="urn:test https://network.invalid/s.xsd"><xi:include href="https://network.invalid/never"/><p>&xxe; &local;</article>tail"#;
        let e = parse_xml(x, SourceInfo::stdin("hostile.xml"), &Default::default());
        assert_eq!(e.status, OperationStatus::Partial);
        let d = e.payload.unwrap();
        assert!(
            d.entities
                .iter()
                .any(|x| x.name == "xxe" && x.disposition != XmlEntityDisposition::DecodedSafe)
        );
        assert!(
            e.diagnostics
                .iter()
                .any(|x| x.class == DiagnosticClass::SecurityRejection)
        );
    }
}
