#[cfg(any(feature = "pdf", all(feature = "python", feature = "typescript")))]
use grist::core::ParserAvailability;
#[cfg(all(feature = "python", feature = "typescript"))]
use grist::core::{ArtifactKind, FormatHint, ParserInfo};
use grist::core::{DetectionEvidenceKind, Limits};
#[cfg(all(feature = "python", feature = "typescript"))]
use grist::detect::AmbiguityPolicy;
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::registry::builtin_parser_registry;
#[cfg(all(feature = "python", feature = "typescript"))]
use grist::registry::{
    FormatMetadata, OptionsMetadata, Parser, ParserContext, ParserDescriptor, ParserError,
    ParserOutput, ParserRegistry, SchemaMetadata,
};
#[cfg(all(feature = "python", feature = "typescript"))]
use serde_json::json;
use std::path::Path;
#[cfg(all(feature = "python", feature = "typescript"))]
use std::sync::Arc;

fn detect(
    path: &str,
    bytes: &[u8],
    mime: Option<&str>,
    options: &DetectionOptions,
) -> grist::detect::Detection {
    detect_with_registry(
        Path::new(path),
        bytes,
        mime,
        None,
        &Limits::default(),
        &builtin_parser_registry().unwrap(),
        options,
    )
    .unwrap()
}

#[cfg(feature = "pdf")]
#[test]
fn contradictory_extension_and_mime_cannot_override_magic() {
    let result = detect(
        "mislabeled.txt",
        b"%PDF-1.7\n1 0 obj",
        Some("text/plain; charset=utf-8"),
        &DetectionOptions::default(),
    );

    assert_eq!(result.status, DetectionStatus::Selected);
    assert_eq!(result.content_kind, ContentKind::Pdf);
    assert_eq!(result.candidates[0].identity.format, "pdf");
    assert_eq!(
        result.candidates[0].parser_availability,
        ParserAvailability::Available
    );
    assert!(
        result.candidates[0]
            .evidence
            .iter()
            .any(|evidence| { evidence.kind == DetectionEvidenceKind::MagicBytes })
    );
    assert!(
        result
            .candidates
            .iter()
            .any(|candidate| { candidate.identity.format == "text" && candidate.rank > 1 })
    );
    assert!(result.candidates.iter().any(|candidate| {
        candidate.identity.format == "text"
            && candidate.evidence.iter().any(|evidence| {
                evidence.kind == DetectionEvidenceKind::Charset
                    && evidence.description.contains("utf-8")
            })
    }));
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "detect.contradictory_evidence" })
    );
}

#[test]
fn windows_1252_compatible_text_is_charset_evidence_not_binary() {
    let result = detect(
        "note.txt",
        &[b'c', b'a', b'f', 0xe9],
        None,
        &DetectionOptions::default(),
    );

    assert_eq!(result.status, DetectionStatus::Selected);
    assert_eq!(result.content_kind, ContentKind::Text);
    assert!(result.candidates[0].evidence.iter().any(|evidence| {
        evidence.kind == DetectionEvidenceKind::Charset
            && evidence.description.contains("windows-1252")
    }));
}

#[test]
fn unknown_high_entropy_bytes_do_not_become_windows_text() {
    let result = detect(
        "blob",
        &[0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88],
        None,
        &DetectionOptions::default(),
    );

    assert_eq!(result.content_kind, ContentKind::Binary);
    assert_eq!(result.status, DetectionStatus::Unsupported);
}

#[test]
#[cfg(feature = "serialization")]
fn extensionless_structure_and_utf16_bom_are_retained() {
    let json = "{\"answer\":42}";
    let mut utf16 = vec![0xff, 0xfe];
    for unit in json.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let result = detect("payload", &utf16, None, &DetectionOptions::default());

    assert_eq!(result.status, DetectionStatus::Selected);
    assert_eq!(result.content_kind, ContentKind::Json);
    assert!(result.candidates[0].evidence.iter().any(|evidence| {
        evidence.kind == DetectionEvidenceKind::Charset
            && evidence.description.contains("utf-16le BOM")
    }));
    assert!(
        result.candidates[0]
            .evidence
            .iter()
            .any(|evidence| { evidence.kind == DetectionEvidenceKind::Structure })
    );
}

#[test]
#[cfg(feature = "serialization")]
fn malformed_json_keeps_ranked_grammar_evidence() {
    let result = detect(
        "broken.json",
        br#"{"answer":"#,
        None,
        &DetectionOptions::default(),
    );

    assert_eq!(result.status, DetectionStatus::Selected);
    assert_eq!(result.content_kind, ContentKind::Json);
    assert!(result.candidates[0].evidence.iter().any(|evidence| {
        evidence.kind == DetectionEvidenceKind::GrammarProbe
            && evidence.description.contains("malformed")
    }));
}

#[test]
fn extensionless_shebang_and_special_manifest_names_are_evidence() {
    let script = detect(
        "tool",
        b"#!/usr/bin/env python3\nprint('ok')\n",
        None,
        &DetectionOptions::default(),
    );
    assert_eq!(script.content_kind, ContentKind::Python);
    assert!(
        script.candidates[0]
            .evidence
            .iter()
            .any(|evidence| { evidence.kind == DetectionEvidenceKind::Shebang })
    );

    let manifest = detect(
        "Cargo.toml",
        b"[package]\nname = \"demo\"\n",
        None,
        &DetectionOptions::default(),
    );
    assert_eq!(manifest.content_kind, ContentKind::Manifest);
    assert!(
        manifest.candidates[0]
            .evidence
            .iter()
            .any(|evidence| { evidence.kind == DetectionEvidenceKind::Filename })
    );
}

#[test]
fn package_manifest_refines_extensionless_zip_to_docx() {
    let bytes = stored_zip(&[("word/document.xml", b"<w:document/>")]);
    let result = detect("upload", &bytes, None, &DetectionOptions::default());

    assert_eq!(
        result.status,
        if cfg!(feature = "word-ooxml") {
            DetectionStatus::Selected
        } else {
            DetectionStatus::Unsupported
        }
    );
    assert_eq!(result.content_kind, ContentKind::Docx);
    assert_eq!(result.candidates[0].identity.format, "docx");
    assert!(
        result.candidates[0]
            .evidence
            .iter()
            .any(|evidence| { evidence.kind == DetectionEvidenceKind::ContainerManifest })
    );
    assert!(
        result
            .candidates
            .iter()
            .any(|candidate| { candidate.identity.format == "zip" && candidate.rank > 1 })
    );
}

#[test]
fn ooxml_content_types_are_decisive_package_evidence() {
    let content_types = br#"<Types><Override ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/></Types>"#;
    let bytes = stored_zip(&[("[Content_Types].xml", content_types)]);
    let result = detect("upload", &bytes, None, &DetectionOptions::default());

    assert_eq!(result.content_kind, ContentKind::Pptx);
    assert!(result.candidates[0].evidence.iter().any(|evidence| {
        evidence.kind == DetectionEvidenceKind::ContainerManifest
            && evidence.description.contains("[Content_Types].xml")
    }));
}

#[test]
#[cfg(all(feature = "python", feature = "typescript"))]
fn ambiguity_policy_is_explicit_and_deterministic() {
    let bytes = b"x = 1\n";
    let first = detect("snippet", bytes, None, &DetectionOptions::default());
    let second = detect("snippet", bytes, None, &DetectionOptions::default());
    assert_eq!(first, second);
    assert_eq!(first.status, DetectionStatus::Ambiguous);
    assert_eq!(first.content_kind, ContentKind::Unknown);
    assert_eq!(
        first.candidates[0].confidence,
        first.candidates[1].confidence
    );
    assert_eq!(first.candidates[0].identity.format, "python");
    assert_eq!(first.candidates[1].identity.format, "typescript");

    let options = DetectionOptions {
        ambiguity_policy: AmbiguityPolicy::SelectHighestRanked,
        ..DetectionOptions::default()
    };
    let selected = detect("snippet", bytes, None, &options);
    assert_eq!(selected.status, DetectionStatus::Selected);
    assert_eq!(selected.content_kind, ContentKind::Python);
}

#[cfg(all(feature = "python", feature = "typescript"))]
struct NoopParser;

#[cfg(all(feature = "python", feature = "typescript"))]
impl Parser for NoopParser {
    fn parse(&self, _context: &mut ParserContext<'_>) -> Result<ParserOutput, ParserError> {
        Ok(ParserOutput::complete(json!({})))
    }
}

#[test]
#[cfg(all(feature = "python", feature = "typescript"))]
fn prefer_available_uses_the_active_registry() {
    let mut registry = ParserRegistry::empty();
    registry
        .register_caller(
            ParserDescriptor::caller(
                "caller.python",
                FormatMetadata::new("python", ArtifactKind::PythonCode)
                    .with_media_types(["text/x-python"])
                    .with_extensions(["py"]),
                ParserInfo::new("caller.python"),
                SchemaMetadata::new("python", "fixture/python/v1"),
                OptionsMetadata::empty("python"),
            ),
            Arc::new(NoopParser),
        )
        .unwrap();
    let options = DetectionOptions {
        ambiguity_policy: AmbiguityPolicy::PreferAvailable,
        ..DetectionOptions::default()
    };
    let result = detect_with_registry(
        Path::new("snippet"),
        b"x = 1\n",
        None,
        Some(&FormatHint::default()),
        &Limits::default(),
        &registry,
        &options,
    )
    .unwrap();

    assert_eq!(result.status, DetectionStatus::Selected);
    assert_eq!(result.content_kind, ContentKind::Python);
    assert_eq!(result.selected_parser.as_deref(), Some("caller.python"));
    assert_eq!(
        result.candidates[0].parser_availability,
        ParserAvailability::Available
    );
}

#[test]
fn invalid_policy_values_fail_explicitly() {
    let options = DetectionOptions {
        ambiguity_margin: f32::NAN,
        ..DetectionOptions::default()
    };
    assert!(
        detect_with_registry(
            Path::new("payload"),
            b"text",
            None,
            None,
            &Limits::default(),
            &builtin_parser_registry().unwrap(),
            &options,
        )
        .is_err()
    );
}

fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in entries {
        let local_offset = u32::try_from(output.len()).unwrap();
        output.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        push_u16(&mut output, 20);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u16(&mut output, 0);
        push_u32(&mut output, 0);
        push_u32(&mut output, u32::try_from(data.len()).unwrap());
        push_u32(&mut output, u32::try_from(data.len()).unwrap());
        push_u16(&mut output, u16::try_from(name.len()).unwrap());
        push_u16(&mut output, 0);
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(data);

        directory.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        push_u16(&mut directory, 20);
        push_u16(&mut directory, 20);
        for _ in 0..4 {
            push_u16(&mut directory, 0);
        }
        push_u32(&mut directory, 0);
        push_u32(&mut directory, u32::try_from(data.len()).unwrap());
        push_u32(&mut directory, u32::try_from(data.len()).unwrap());
        push_u16(&mut directory, u16::try_from(name.len()).unwrap());
        for _ in 0..4 {
            push_u16(&mut directory, 0);
        }
        push_u32(&mut directory, 0);
        push_u32(&mut directory, local_offset);
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_offset = u32::try_from(output.len()).unwrap();
    let directory_size = u32::try_from(directory.len()).unwrap();
    output.extend_from_slice(&directory);
    output.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    push_u16(&mut output, 0);
    push_u16(&mut output, 0);
    push_u16(&mut output, u16::try_from(entries.len()).unwrap());
    push_u16(&mut output, u16::try_from(entries.len()).unwrap());
    push_u32(&mut output, directory_size);
    push_u32(&mut output, directory_offset);
    push_u16(&mut output, 0);
    output
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}
