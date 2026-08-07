use grist::core::{
    BudgetProfile, BudgetSelection, CompoundMemberInput, FormatOptions, Input, InputError,
    InputOrigin, NetworkAccess, ParseRequest, Provider, ProviderKind, ProviderSet, RequestId,
    SecretString, SourceInfo,
};
use std::fs;
use std::io::{Cursor, Seek, SeekFrom};
use std::sync::Arc;

fn trusted_budget() -> BudgetSelection {
    BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1)
}

#[test]
fn resolves_raw_bytes_without_text_coercion() {
    let resolved = Input::bytes(vec![0, 0xff, b'a'])
        .resolve(&trusted_budget())
        .unwrap();
    assert_eq!(resolved.raw_bytes(), &[0, 0xff, b'a']);
    assert!(!resolved.declared_utf8());
    assert_eq!(resolved.utf8_text(), None);
    assert_eq!(resolved.origin(), &InputOrigin::Bytes);
}

#[test]
fn resolves_utf8_text_to_its_exact_bytes() {
    let resolved = Input::utf8("naive: é").resolve(&trusted_budget()).unwrap();
    assert_eq!(resolved.raw_bytes(), "naive: é".as_bytes());
    assert!(resolved.declared_utf8());
    assert_eq!(resolved.utf8_text(), Some("naive: é"));
    assert_eq!(resolved.origin(), &InputOrigin::Utf8Text);
}

#[test]
fn resolves_seekable_reader_from_the_start() {
    let mut reader = Cursor::new(b"seekable".to_vec());
    reader.seek(SeekFrom::Start(4)).unwrap();
    let resolved = Input::seekable(reader).resolve(&trusted_budget()).unwrap();
    assert_eq!(resolved.raw_bytes(), b"seekable");
    assert_eq!(resolved.origin(), &InputOrigin::SeekableReader);
}

#[test]
fn resolves_one_shot_stream() {
    let resolved = Input::stream(Cursor::new(b"stream".to_vec()))
        .resolve(&trusted_budget())
        .unwrap();
    assert_eq!(resolved.raw_bytes(), b"stream");
    assert_eq!(resolved.origin(), &InputOrigin::Stream);
}

#[test]
fn resolves_filesystem_path() {
    let path = std::env::temp_dir().join(format!("grist-core-input-{}.bin", std::process::id()));
    fs::write(&path, b"path bytes").unwrap();
    let resolved = Input::path(&path).resolve(&trusted_budget()).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(resolved.raw_bytes(), b"path bytes");
    assert_eq!(resolved.origin(), &InputOrigin::Path(path));
}

#[test]
fn resolves_virtual_compound_member_with_parent_identity() {
    let parent = SourceInfo::from_uri("urn:archive:42", "bundle.zip");
    let member = CompoundMemberInput::new(
        parent.clone(),
        "docs/readme.md",
        Some(3),
        Input::bytes(b"member".to_vec()),
    );
    let resolved = Input::compound_member(member)
        .resolve(&trusted_budget())
        .unwrap();
    assert_eq!(resolved.raw_bytes(), b"member");
    assert_eq!(
        resolved.origin(),
        &InputOrigin::CompoundMember {
            parent_source: parent,
            member_path: "docs/readme.md".into(),
            member_index: Some(3),
            backing: Box::new(InputOrigin::Bytes),
        }
    );
}

#[test]
fn input_budget_failure_is_not_end_of_input() {
    let mut budget = BudgetProfile::UntrustedServiceV1.budget();
    budget.max_input_bytes = Some(3);
    let error = Input::stream(Cursor::new(b"four".to_vec()))
        .resolve(&BudgetSelection::custom(budget))
        .unwrap_err();
    assert!(matches!(
        error,
        InputError::ByteLimitExceeded {
            limit: 3,
            observed_at_least: 4
        }
    ));
}

#[derive(Debug)]
struct DemoOptions {
    mode: &'static str,
}

impl FormatOptions for DemoOptions {
    const FORMAT: &'static str = "demo";
}

#[test]
fn request_keeps_typed_format_options_and_batch_id() {
    let request_id = RequestId::new("batch-7/item-42").unwrap();
    let request = ParseRequest::new(
        request_id.clone(),
        Input::utf8("payload"),
        SourceInfo::stdin("item.demo"),
        trusted_budget(),
        ProviderSet::none(),
    )
    .with_format_options(DemoOptions { mode: "semantic" });
    assert_eq!(request.request_id, request_id);
    assert_eq!(request.format_options.mode, "semantic");
    assert_eq!(
        request
            .format_hint
            .as_ref()
            .and_then(|hint| hint.format.as_deref()),
        Some("demo")
    );
    let resolved = request.resolve_input().unwrap();
    assert_eq!(resolved.input.raw_bytes(), b"payload");
}

#[test]
fn request_ids_are_stable_serializable_correlation_keys() {
    let id = RequestId::new("batch-a/0001").unwrap();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(
        serde_json::from_str::<String>(&json).unwrap(),
        "batch-a/0001"
    );
    assert_eq!(serde_json::from_str::<RequestId>(&json).unwrap(), id);
    assert!(RequestId::new("").is_err());
    assert!(serde_json::from_value::<RequestId>(serde_json::Value::String(String::new())).is_err());
    assert!(RequestId::new(String::from_utf8(vec![b'a', 10, b'b']).unwrap()).is_err());
}

#[test]
fn built_in_parser_options_implement_the_typed_contract() {
    fn format_of<O: FormatOptions>() -> &'static str {
        O::FORMAT
    }

    assert_eq!(format_of::<grist::text::TextOptions>(), "text");
    #[cfg(feature = "csv")]
    assert_eq!(format_of::<grist::csv::CsvOptions>(), "csv");
    #[cfg(feature = "html")]
    assert_eq!(format_of::<grist::html::HtmlOptions>(), "html");
    #[cfg(feature = "latex")]
    assert_eq!(format_of::<grist::latex::LatexOptions>(), "latex");
    #[cfg(feature = "markdown")]
    assert_eq!(format_of::<grist::markdown::MarkdownOptions>(), "markdown");
    #[cfg(feature = "ldgr-projection")]
    assert_eq!(
        format_of::<grist::ldgr_projection::LdgrProjectionOptions>(),
        "ldgr_projection"
    );
    #[cfg(feature = "rust")]
    assert_eq!(format_of::<grist::rust::RustIngestOptions>(), "rust");
    #[cfg(feature = "python")]
    assert_eq!(format_of::<grist::python::PythonIngestOptions>(), "python");
    #[cfg(feature = "typescript")]
    assert_eq!(
        format_of::<grist::typescript::TypeScriptIngestOptions>(),
        "typescript"
    );
    #[cfg(feature = "serialization")]
    assert_eq!(
        format_of::<grist::serialization::SerializationOptions>(),
        "structured_text"
    );
    #[cfg(feature = "model-output")]
    assert_eq!(
        format_of::<grist::model_output::ModelOutputOptions>(),
        "model_output"
    );
}

#[test]
fn source_info_serializes_all_caller_supplied_identity_labels() {
    let parent = SourceInfo::from_uri("urn:mail:message-7", "message.eml");
    let source = SourceInfo::new("report.csv")
        .with_path("inbox/report.csv")
        .with_declared_mime_type("text/csv")
        .with_uri("urn:attachment:report")
        .with_repository_relative_path("fixtures/report.csv")
        .with_ingestion_timestamp("2026-08-06T10:30:00Z")
        .with_parent(parent);
    let value = serde_json::to_value(&source).unwrap();
    assert_eq!(value["display_name"], "report.csv");
    assert_eq!(value["declared_mime_type"], "text/csv");
    assert_eq!(value["uri"], "urn:attachment:report");
    assert_eq!(value["repository_relative_path"], "fixtures/report.csv");
    assert_eq!(value["parent"]["display_name"], "message.eml");
}

struct SecretProvider {
    credential: SecretString,
}

impl Provider for SecretProvider {
    fn name(&self) -> &str {
        "private-ocr"
    }
}

#[test]
fn provider_selection_is_explicit_and_debug_output_redacts_secrets() {
    let provider = Arc::new(SecretProvider {
        credential: SecretString::new("do-not-leak"),
    });
    assert_eq!(provider.credential.expose(), "do-not-leak");
    assert!(!format!("{:?}", provider.credential).contains("do-not-leak"));
    let mut providers = ProviderSet::none();
    providers.select(ProviderKind::Ocr, provider, NetworkAccess::Denied);
    let binding = providers.selected(ProviderKind::Ocr).unwrap();
    assert_eq!(binding.provider().name(), "private-ocr");
    assert_eq!(binding.network_access(), NetworkAccess::Denied);
    assert!(!format!("{providers:?}").contains("do-not-leak"));
}
