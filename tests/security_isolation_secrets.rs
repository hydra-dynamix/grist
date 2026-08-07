use grist::container::{
    ArchiveEntryKind, ContainerArtifactMode, ContainerChildStatus, ContainerClass,
    ContainerDecodeContext, ContainerDecodeFailure, ContainerDecoder, ContainerDecoderRegistry,
    ContainerMember, ContainerParseOptions, ContainerParseRequest, ContainerRecursor,
    ParentRelativeLocator,
};
use grist::core::{
    BudgetProfile, BudgetSelection, IndexPosition, LocationComponent, NetworkAccess, ProviderSet,
    RequestId, SecretString, SourceInfo,
};
use grist::ingest::Ingestor;
use grist::provider::{
    BackendPermission, IsolatedBackendOptions, IsolatedBackendRequest, IsolationPolicy, OcrOptions,
    OcrProvider, OcrProviderAdapter, OcrRequest, OcrResult, ProviderDeterminism, ProviderError,
    ProviderMetadata, ProviderOutcome, ProviderRequest, ProviderRequestContext,
};
use grist::security::{
    ArchiveMemberDescriptor, ArchiveSecurityPolicy, PrivateTemporaryStorage, SecurityPolicy,
    TemporaryRetentionPolicy, XmlSecurityFindingKind, inspect_xml,
};
use serde_json::json;
use std::fs;
use std::sync::Arc;

#[test]
fn strict_policy_has_no_execution_or_implicit_network_mode() {
    let policy = SecurityPolicy::default();
    let value = serde_json::to_value(policy).unwrap();
    assert_eq!(value["execution"], "preserve_inert");
    assert_eq!(value["network"], "explicit_providers_only");
    assert_eq!(value["active_rendering"], "escape");
    assert_eq!(value["input_metadata"], "caller_directed_only");
}

#[test]
fn xml_guard_inventories_every_active_reference_without_resolving_it() {
    let xml = br#"<!DOCTYPE article [<!ENTITY xxe SYSTEM 'file:///never-read'>]>
        <article xmlns:xi='http://www.w3.org/2001/XInclude'
          xsi:schemaLocation='urn:test https://example.invalid/schema.xsd'>
          <xi:include href='https://example.invalid/secret'/>&xxe;
        </article>"#;
    let findings = inspect_xml(xml, &Default::default());
    assert!(
        findings
            .iter()
            .any(|item| item.kind == XmlSecurityFindingKind::Doctype)
    );
    assert!(
        findings
            .iter()
            .any(|item| item.kind == XmlSecurityFindingKind::EntityDeclaration)
    );
    assert!(
        findings
            .iter()
            .any(|item| item.kind == XmlSecurityFindingKind::XInclude)
    );
    assert!(
        findings
            .iter()
            .any(|item| item.kind == XmlSecurityFindingKind::RemoteSchemaLocation)
    );
    let serialized = serde_json::to_string(&findings).unwrap();
    assert!(!serialized.contains("example.invalid"));
    assert!(!serialized.contains("never-read"));
}

#[test]
fn archive_policy_rejects_portable_escapes_links_devices_and_collisions() {
    let policy = ArchiveSecurityPolicy::default();
    for path in [
        "../escape",
        "C:\\escape",
        "/absolute",
        "safe/../../escape",
        "NUL.txt",
        "safe.txt:stream",
    ] {
        assert!(
            policy
                .validate_member(&ArchiveMemberDescriptor {
                    path,
                    kind: ArchiveEntryKind::RegularFile,
                    link_target: None,
                })
                .is_err()
        );
    }
    for kind in [
        ArchiveEntryKind::SymbolicLink,
        ArchiveEntryKind::HardLink,
        ArchiveEntryKind::BlockDevice,
        ArchiveEntryKind::CharacterDevice,
        ArchiveEntryKind::Fifo,
        ArchiveEntryKind::Socket,
    ] {
        assert!(
            policy
                .validate_member(&ArchiveMemberDescriptor {
                    path: "member",
                    kind,
                    link_target: Some("../outside"),
                })
                .is_err()
        );
    }
    let first = policy
        .validate_member(&ArchiveMemberDescriptor {
            path: "Folder/Report.txt",
            kind: ArchiveEntryKind::RegularFile,
            link_target: None,
        })
        .unwrap();
    let second = policy
        .validate_member(&ArchiveMemberDescriptor {
            path: "folder\\report.TXT. ",
            kind: ArchiveEntryKind::RegularFile,
            link_target: None,
        })
        .unwrap();
    assert_eq!(first, second);
}

struct HostileArchive;

impl ContainerDecoder for HostileArchive {
    fn format(&self) -> &str {
        "hostile_archive"
    }
    fn class(&self) -> ContainerClass {
        ContainerClass::Archive
    }
    fn decode(
        &self,
        _context: &ContainerDecodeContext<'_>,
    ) -> Result<Vec<ContainerMember>, ContainerDecodeFailure> {
        Ok(vec![
            member(0, "../escape", b"escape"),
            member(1, "A.txt", b"first"),
            member(2, "a.TXT", b"second"),
            member(3, "link", b"target")
                .with_entry_kind(ArchiveEntryKind::SymbolicLink)
                .with_link_target("../outside"),
            member(4, "safe.txt", b"safe"),
        ])
    }
}

fn member(order: u64, path: &str, bytes: &[u8]) -> ContainerMember {
    let locator = ParentRelativeLocator::single(LocationComponent::ArchiveMember {
        member_path: path.into(),
        member_index: IndexPosition::zero_based(order),
    })
    .unwrap();
    ContainerMember::available(order, locator, bytes.to_vec()).with_declared_filename(path)
}

#[test]
fn recursive_container_rejections_remain_inventory_records_with_diagnostics() {
    let ingestor = Ingestor::builtin().unwrap();
    let mut registry = ContainerDecoderRegistry::new();
    registry.register(Arc::new(HostileArchive)).unwrap();
    let result = ContainerRecursor::new(&ingestor, &registry)
        .parse(
            ContainerParseRequest::new(
                RequestId::new("hostile/archive").unwrap(),
                b"archive".to_vec(),
                SourceInfo::new("hostile.arc"),
                "hostile_archive",
                ContainerParseOptions::new(ContainerArtifactMode::InlinePayload)
                    .without_leaf_payloads(),
                BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
            ),
            None,
        )
        .unwrap();
    let rejected = result
        .children
        .iter()
        .filter(|child| child.status == ContainerChildStatus::Rejected)
        .collect::<Vec<_>>();
    assert_eq!(rejected.len(), 4);
    for child in rejected {
        assert!(child.artifact.identity.content.raw.is_some());
        assert!(child.artifact.content.is_none());
        assert!(child.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .code
                .as_str()
                .starts_with("grist.security.archive.")
        }));
    }
    assert_eq!(
        result
            .children
            .iter()
            .filter(|child| child.status != ContainerChildStatus::Rejected)
            .count(),
        1
    );
}

fn test_base(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("grist-security-{name}-{}", std::process::id()))
}

#[test]
fn private_temporary_storage_deletes_on_drop_and_blocks_path_escape() {
    let base = test_base("delete-on-drop");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir(&base).unwrap();
    let private_path;
    {
        let storage =
            PrivateTemporaryStorage::create_in(&base, TemporaryRetentionPolicy::DeleteOnDrop)
                .unwrap();
        private_path = storage.path().to_path_buf();
        assert!(private_path.starts_with(base.canonicalize().unwrap()));
        assert!(
            storage
                .write_generated("input.bin", b"private", None)
                .unwrap()
                .is_file()
        );
        assert!(
            storage
                .write_generated("../escape", b"blocked", None)
                .is_err()
        );
        assert!(!base.parent().unwrap().join("escape").exists());
    }
    assert!(!private_path.exists());
    fs::remove_dir(&base).unwrap();
}

#[test]
fn native_backend_policy_is_bounded_and_its_private_storage_enforces_the_limit() {
    let policy = IsolationPolicy::strict(1_000, 32 * 1024 * 1024, 4).unwrap();
    assert!(policy.private_temporary_filesystem);
    assert!(policy.read_only_input);
    assert_eq!(policy.max_processes, 1);
    assert!(!policy.active_content_execution);
    assert!(!policy.inherit_environment);
    assert!(!policy.host_filesystem_access);
    let context = ProviderRequestContext::new(
        b"legacy",
        NetworkAccess::Denied,
        &json!({"backend": "fixture"}),
    )
    .unwrap();
    let request = IsolatedBackendRequest::new(
        BackendPermission::ExplicitlyAllowed,
        context,
        IsolatedBackendOptions {
            format: "legacy-word".into(),
            output_schema_version: "fixture/legacy-word/v1".into(),
            isolation: policy.clone(),
            backend_options: json!({}),
        },
    )
    .unwrap();
    let mut storage = request.create_private_temporary_storage().unwrap();
    assert!(storage.write_generated("within.bin", b"four", None).is_ok());
    assert!(storage.write_generated("over.bin", b"x", None).is_err());
    storage.cleanup().unwrap();

    let mut unsafe_policy = policy;
    unsafe_policy.active_content_execution = true;
    assert!(unsafe_policy.validate().is_err());
}

struct FailingLeakProvider {
    secret: String,
}
impl OcrProvider for FailingLeakProvider {
    fn recognize(&self, _request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError> {
        Err(ProviderError::failure(
            "leaker",
            format!("failed with {}", self.secret),
        ))
    }
}

struct ResultLeakProvider {
    secret: String,
}
impl OcrProvider for ResultLeakProvider {
    fn recognize(&self, _request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError> {
        Ok(OcrResult {
            text: self.secret.clone(),
            ..Default::default()
        })
    }
}

struct PanicProvider;
impl OcrProvider for PanicProvider {
    fn recognize(&self, _request: &OcrRequest<'_>) -> Result<OcrResult, ProviderError> {
        panic!("hostile provider panic")
    }
}

fn invoke_provider(
    implementation: impl OcrProvider,
    secret: &SecretString,
) -> grist::provider::ProviderResponse {
    let metadata =
        ProviderMetadata::new("leaker", "fixture", "1", ProviderDeterminism::Guaranteed).unwrap();
    let provider = Arc::new(OcrProviderAdapter::new(metadata, implementation).unwrap());
    let mut providers = ProviderSet::none();
    providers.select(
        grist::core::ProviderKind::Ocr,
        provider,
        NetworkAccess::Denied,
    );
    let context = ProviderRequestContext::new(b"image", NetworkAccess::Denied, &json!({}))
        .unwrap()
        .with_text_secret("password", secret)
        .unwrap();
    providers
        .invoke(&ProviderRequest::Ocr(OcrRequest::new(
            context,
            OcrOptions::default(),
        )))
        .unwrap()
}

#[test]
fn provider_errors_results_and_panics_cannot_cross_the_boundary_with_secrets() {
    let literal = "credential-must-never-serialize";
    let secret = SecretString::new(literal);
    for response in [
        invoke_provider(
            FailingLeakProvider {
                secret: literal.into(),
            },
            &secret,
        ),
        invoke_provider(
            ResultLeakProvider {
                secret: literal.into(),
            },
            &secret,
        ),
        invoke_provider(PanicProvider, &secret),
    ] {
        assert!(matches!(response.outcome, ProviderOutcome::Failed));
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains(literal));
        assert!(!format!("{response:?}").contains(literal));
        assert!(!response.metadata.diagnostics.is_empty());
    }
}

#[cfg(feature = "markdown")]
#[test]
fn markdown_renderer_escapes_html_uris_and_raw_fallbacks() {
    use grist::document_graph::{
        DocumentGraph, DocumentKind, DocumentNode, DocumentNodeKind, TransformOptions,
        render_markdown,
    };
    let mut graph = DocumentGraph::new("security-render", DocumentKind::Document);
    graph.add_node(DocumentNode::new("root", DocumentNodeKind::Document));
    graph.add_node(
        DocumentNode::new("paragraph", DocumentNodeKind::Paragraph)
            .with_text("<script>globalThis.pwned=true</script>"),
    );
    graph.add_node(
        DocumentNode::new("link", DocumentNodeKind::Link)
            .with_text("click")
            .with_attr("destination", "java\nscript:alert(1)"),
    );
    graph.add_node(
        DocumentNode::new("raw", DocumentNodeKind::RawBlock)
            .with_text("</script>```<img src=x onerror=alert(1)>")
            .with_ordinal(3),
    );
    let rendered = render_markdown(&graph, TransformOptions::default()).unwrap();
    assert!(!rendered.contains("<script>"));
    assert!(!rendered.to_ascii_lowercase().contains("javascript:"));
    assert!(rendered.contains("#grist-blocked-active-uri"));
    assert!(rendered.contains("````text"));
}

#[cfg(feature = "latex")]
#[test]
fn latex_renderer_never_emits_untrusted_commands() {
    use grist::document_graph::{
        DocumentGraph, DocumentKind, DocumentNode, DocumentNodeKind, TransformOptions, render_latex,
    };
    let mut graph = DocumentGraph::new("security-latex", DocumentKind::Document);
    graph.add_node(
        DocumentNode::new("raw", DocumentNodeKind::RawBlock)
            .with_text("\\immediate\\write18{touch owned}"),
    );
    let rendered = render_latex(&graph, TransformOptions::default()).unwrap();
    assert!(!rendered.contains("\\immediate\\write18"));
    assert!(rendered.contains("\\textbackslash{}immediate"));
}
