//! Byte-level XML active-reference guard used before parser selection.

use serde::{Deserialize, Serialize};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum XmlSecurityPolicy {
    #[default]
    RejectActiveReferences,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum XmlSecurityFindingKind {
    Doctype,
    EntityDeclaration,
    XInclude,
    RemoteSchemaLocation,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XmlSecurityFinding {
    pub code: String,
    pub kind: XmlSecurityFindingKind,
    pub byte_start: u64,
    pub byte_end: u64,
    pub message: String,
}

/// Inspect without expanding, resolving, fetching, or decoding entity content.
pub fn inspect_xml(bytes: &[u8], policy: &XmlSecurityPolicy) -> Vec<XmlSecurityFinding> {
    let XmlSecurityPolicy::RejectActiveReferences = policy;
    let folded = bytes.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>();
    let mut findings = Vec::new();
    add_all(
        &folded,
        b"<!doctype",
        XmlSecurityFindingKind::Doctype,
        "grist.security.xml.doctype",
        "XML document type declarations are disabled",
        &mut findings,
    );
    add_all(
        &folded,
        b"<!entity",
        XmlSecurityFindingKind::EntityDeclaration,
        "grist.security.xml.entity_declaration",
        "XML entity declarations are disabled",
        &mut findings,
    );
    for needle in [b"<xi:include".as_slice(), b"<xinclude".as_slice()] {
        add_all(
            &folded,
            needle,
            XmlSecurityFindingKind::XInclude,
            "grist.security.xml.xinclude",
            "XML XInclude processing is disabled",
            &mut findings,
        );
    }
    for needle in [
        b"schemalocation".as_slice(),
        b"nonamespaceschemalocation".as_slice(),
    ] {
        for start in find_all(&folded, needle) {
            let window_end = folded.len().min(start.saturating_add(2_048));
            if find_all(&folded[start..window_end], b"://")
                .next()
                .is_some()
            {
                findings.push(finding(
                    XmlSecurityFindingKind::RemoteSchemaLocation,
                    "grist.security.xml.remote_schema",
                    start,
                    start + needle.len(),
                    "remote XML schema resolution is disabled",
                ));
            }
        }
    }
    findings.sort_by_key(|finding| (finding.byte_start, finding.byte_end, finding.code.clone()));
    findings.dedup_by(|left, right| {
        left.kind == right.kind
            && left.byte_start == right.byte_start
            && left.byte_end == right.byte_end
    });
    findings
}

fn add_all(
    haystack: &[u8],
    needle: &[u8],
    kind: XmlSecurityFindingKind,
    code: &str,
    message: &str,
    findings: &mut Vec<XmlSecurityFinding>,
) {
    findings.extend(
        find_all(haystack, needle)
            .map(|start| finding(kind, code, start, start + needle.len(), message)),
    );
}

fn find_all<'a>(haystack: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(move |(index, window)| (window == needle).then_some(index))
}

fn finding(
    kind: XmlSecurityFindingKind,
    code: &str,
    start: usize,
    end: usize,
    message: &str,
) -> XmlSecurityFinding {
    XmlSecurityFinding {
        code: code.into(),
        kind,
        byte_start: u64::try_from(start).unwrap_or(u64::MAX),
        byte_end: u64::try_from(end).unwrap_or(u64::MAX),
        message: message.into(),
    }
}
