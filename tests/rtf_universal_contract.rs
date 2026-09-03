#![cfg(all(feature = "rtf", feature = "document-graph", feature = "schemas"))]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, Limits, LocationComponent, OperationStatus,
    ParseRequest, ProviderSet, RequestId, ResourceBudget, SchemaVersion, SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{
    DocumentGraphContext, DocumentNodeKind, DocumentRelation, ToDocumentGraph,
};
use grist::registry::{Capability, builtin_parser_registry};
use grist::render::{FidelityMode, RenderFormat, RenderOptions, render_document_graph};
use grist::rtf::{RtfDocument, RtfElement, RtfOptions, RtfRevisionKind};
use grist::segment::{SegmentOptions, segment_document_graph};
use std::path::Path;

const COMPLEX: &[u8] = br#"{\rtf1\ansi\ansicpg1252\deff0
{\fonttbl{\f0\fswiss\fcharset0 Arial;}{\f1\froman Times New Roman;}}
{\colortbl;\red255\green0\blue0;\red0\green0\blue255;}
{\stylesheet{\s0 Normal;}{\s1\sbasedon0\snext0 Heading 1;}{\cs2\additive Emphasis;}}
{\*\listtable{\list\listtemplateid10{\listlevel\levelnfc23\levelstartat1{\leveltext\'01\u8226 ?;}{\levelnumbers;}}\listid5}}
{\*\listoverridetable{\listoverride\listid5\listoverridecount0\ls1}}
{\info{\title Complete RTF}{\author Grist}}{\*\generator Grist fixture;}
\pard\s0 Plain {\b bold} \u945? and \u-10179?\u-8704? emoji.\par
\pard\ls1\ilvl0 List item\par\pard
{\field\fldlock{\*\fldinst HYPERLINK "https://example.invalid/rtf"}{\fldrslt Example link}}\par
\trowd\intbl\cellx1000\cellx2000 First\cell Second\cell\row\pard
Normal {\revised\revauth0 inserted}{\deleted\revauth1 old}.\par
{\*\annotation{\atnauthor Reviewer}\atnid7 Note body}
{\pict\pngblip\picw1\pich1 \'89 504E47}
{\object\objemb{\*\objclass Package}{\*\objdata\bin4 MZ00}{\result Object result}}
\mystery42 retained unknown\par}"#;

fn dispatch(bytes: Vec<u8>, budget: ResourceBudget) -> grist::core::Envelope<serde_json::Value> {
    let request = ParseRequest::new(
        RequestId::new("rtf-test").unwrap(),
        Input::bytes(bytes),
        SourceInfo::new("fixture.rtf"),
        BudgetSelection::custom(budget),
        ProviderSet::none(),
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch(
            "rtf",
            request,
            Some(serde_json::to_value(RtfOptions::default()).unwrap()),
        )
        .unwrap()
}

#[test]
fn rich_rtf_preserves_structure_semantics_artifacts_and_views() {
    let first = dispatch(COMPLEX.to_vec(), ResourceBudget::trusted_unbounded());
    let second = dispatch(COMPLEX.to_vec(), ResourceBudget::trusted_unbounded());
    assert_eq!(
        first.status,
        OperationStatus::Complete,
        "{:#?}",
        first.diagnostics
    );
    assert_eq!(first.payload, second.payload);
    let document: RtfDocument = serde_json::from_value(first.payload.clone().unwrap()).unwrap();
    assert_eq!(document.rtf_version, 1);
    assert_eq!(document.ansi_code_page, 1252);
    assert_eq!(document.fonts.len(), 2);
    assert_eq!(document.fonts[0].name, "Arial");
    assert_eq!(document.colors.len(), 3);
    assert_eq!(document.styles.len(), 3);
    assert!(
        document
            .styles
            .iter()
            .any(|style| style.number == 0 && style.name == "Normal")
    );
    assert_eq!(document.lists.len(), 1);
    assert_eq!(document.list_overrides.len(), 1);
    assert_eq!(document.list_items.len(), 1);
    assert_eq!(document.tables[0].rows[0].cells.len(), 2);
    assert_eq!(document.fields[0].field_type, "HYPERLINK");
    assert_eq!(
        document.fields[0].target.as_deref(),
        Some("https://example.invalid/rtf")
    );
    assert_eq!(document.images[0].binary_length, 4);
    assert_eq!(document.objects[0].binary_length, 4);
    assert_eq!(document.embedded_artifacts.len(), 2);
    assert_eq!(
        document.embedded_artifacts[0]
            .identity
            .content
            .raw
            .as_ref()
            .unwrap()
            .byte_length,
        4
    );
    assert_eq!(document.comments[0].author.as_deref(), Some("Reviewer"));
    assert!(document.views.accepted.contains("α"));
    assert!(document.views.accepted.contains("😀"));
    assert!(document.views.accepted.contains("inserted"));
    assert!(!document.views.accepted.contains(" old"));
    assert!(!document.views.original.contains("inserted"));
    assert!(document.views.original.contains(" old"));
    assert_eq!(document.views.original, document.views.rejected);
    assert!(
        document
            .revisions
            .iter()
            .any(|revision| revision.kind == RtfRevisionKind::Inserted)
    );
    let unknown = document
        .unknown_controls
        .iter()
        .find(|control| control.name == "mystery")
        .unwrap();
    assert_eq!(unknown.parameter, Some(42));
    let range = match unknown.locator.components() {
        [
            LocationComponent::TextRange {
                byte_start,
                byte_end,
                ..
            },
        ] => *byte_start..*byte_end,
        other => panic!("unexpected locator: {other:?}"),
    };
    assert_eq!(&COMPLEX[range], br"\mystery42 ");
    assert_eq!(document.raw_source_bytes.as_deref(), Some(COMPLEX));
    assert!(matches!(
        document.root.contents[0],
        RtfElement::Control { .. }
    ));

    let graph = document
        .to_document_graph(DocumentGraphContext::new("rtf-graph"))
        .unwrap();
    for kind in [
        DocumentNodeKind::Paragraph,
        DocumentNodeKind::ListItem,
        DocumentNodeKind::Table,
        DocumentNodeKind::FormField,
        DocumentNodeKind::Image,
        DocumentNodeKind::Attachment,
        DocumentNodeKind::Revision,
        DocumentNodeKind::Comment,
        DocumentNodeKind::Unknown,
    ] {
        assert!(
            graph.nodes.iter().any(|node| node.kind == kind),
            "missing {kind:?}"
        );
    }
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::LinksTo)
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.relation == DocumentRelation::RevisionOf)
    );
    let rendered = render_document_graph(
        &graph,
        RenderFormat::PlainText,
        &RenderOptions::new(FidelityMode::RawFallback),
    )
    .unwrap();
    assert!(rendered.content.contains("Plain bold"));
    let source_identity = first.identity.as_ref().unwrap();
    let document_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        source_identity,
        &document_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(!segments.segments.is_empty());
    assert!(
        segments
            .segments
            .iter()
            .all(|segment| !segment.locators.is_empty())
    );
    let schema = grist::schema::schema_json("rtf").unwrap();
    let report = grist::schema::validate_against_schema(
        "rtf",
        SchemaVersion::RTF_V1,
        first.payload.as_ref().unwrap(),
        &schema,
    )
    .unwrap();
    assert!(report.valid, "{:#?}", report.issues);
}

#[test]
fn detection_registry_and_capability_surfaces_select_rtf() {
    let registry = builtin_parser_registry().unwrap();
    let descriptor = registry
        .snapshot()
        .parsers
        .into_iter()
        .find(|parser| parser.format.id == "rtf")
        .unwrap();
    assert!(
        descriptor
            .capabilities
            .contains(&Capability::EmbeddedArtifacts)
    );
    assert!(
        descriptor
            .capabilities
            .contains(&Capability::DocumentGraphProjection)
    );
    let detection = detect_with_registry(
        Path::new("extensionless"),
        COMPLEX,
        None,
        None,
        &Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detection.status, DetectionStatus::Selected);
    assert_eq!(detection.content_kind, ContentKind::Rtf);
    assert_eq!(detection.candidates[0].identity.format, "rtf");
}

#[test]
fn malformed_groups_and_escapes_recover_without_silent_success() {
    let malformed = br"{\rtf1\ansi before {\b bold\'z".to_vec();
    let envelope = dispatch(malformed, ResourceBudget::trusted_unbounded());
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert!(envelope.payload.is_some());
    assert!(
        envelope
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.locator.is_some())
    );
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unclosed group"))
    );
    let document: RtfDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert!(!document.root.closed);
    assert!(document.views.visible.contains("before"));

    let unmatched = dispatch(
        br"{\rtf1 valid}}".to_vec(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(unmatched.status, OperationStatus::Partial);
    assert!(unmatched.payload.is_some());
}

#[test]
fn deep_groups_obey_the_shared_nesting_budget() {
    let mut bytes = br"{\rtf1 ".to_vec();
    bytes.extend(std::iter::repeat_n(b'{', 32));
    bytes.extend_from_slice(b"deep");
    bytes.extend(std::iter::repeat_n(b'}', 33));
    let complete = dispatch(bytes.clone(), ResourceBudget::trusted_unbounded());
    assert_eq!(complete.status, OperationStatus::Complete);
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_nesting_depth = Some(8);
    let limited = dispatch(bytes, budget);
    assert_eq!(limited.status, OperationStatus::Failed);
    assert!(limited.payload.is_none());
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str().contains("budget"))
    );
}

#[cfg(feature = "cli")]
#[test]
fn cli_routes_rtf_to_the_same_public_payload() {
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("grist-rtf-{nonce}.rtf"));
    fs::write(&path, COMPLEX).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "rtf", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["status"], "complete");
    assert_eq!(envelope["kind"], "rtf");
    assert_eq!(envelope["payload"]["schema_version"], SchemaVersion::RTF_V1);
    assert_eq!(
        envelope["payload"]["views"]["visible"],
        envelope["payload"]["views"]["accepted"]
    );
}
