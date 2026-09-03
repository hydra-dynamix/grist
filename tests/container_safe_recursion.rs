use grist::container::{
    ArtifactContent, ArtifactDisposition, ArtifactRelationship, ArtifactStoreError,
    ContainerArtifactMode, ContainerChild, ContainerChildStatus, ContainerClass,
    ContainerDecodeContext, ContainerDecodeFailure, ContainerDecoder, ContainerDecoderRegistry,
    ContainerMember, ContainerParseOptions, ContainerParseRequest, ContainerRecursor,
    ContainerUnavailable, ContainerUnavailableKind, ContentAddressedArtifactReference,
    ContentAddressedArtifactResolver, ContentAddressedArtifactSink, ParentRelativeLocator,
};
use grist::core::{
    BudgetAxis, BudgetProfile, BudgetSelection, FormatHint, IndexPosition, LocationComponent,
    OperationStatus, RequestId, ResourceBudget, SourceInfo,
};
use grist::ingest::Ingestor;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MemoryStore(Mutex<BTreeMap<String, Vec<u8>>>);

impl ContentAddressedArtifactSink for MemoryStore {
    fn store(
        &self,
        reference: &ContentAddressedArtifactReference,
        bytes: &[u8],
    ) -> Result<(), ArtifactStoreError> {
        reference
            .validate_bytes(bytes)
            .map_err(|error| ArtifactStoreError::new(error.to_string()))?;
        self.0
            .lock()
            .unwrap()
            .insert(reference.digest.clone(), bytes.to_vec());
        Ok(())
    }
}

impl ContentAddressedArtifactResolver for MemoryStore {
    fn resolve(
        &self,
        reference: &ContentAddressedArtifactReference,
    ) -> Result<Vec<u8>, ArtifactStoreError> {
        self.0
            .lock()
            .unwrap()
            .get(&reference.digest)
            .cloned()
            .ok_or_else(|| ArtifactStoreError::new("missing fixture digest"))
    }
}

struct ArchiveDecoder;

impl ContainerDecoder for ArchiveDecoder {
    fn format(&self) -> &str {
        "fixture_archive"
    }

    fn class(&self) -> ContainerClass {
        ContainerClass::Archive
    }

    fn decode(
        &self,
        context: &ContainerDecodeContext<'_>,
    ) -> Result<Vec<ContainerMember>, ContainerDecodeFailure> {
        context.checkpoint()?;
        Ok(vec![
            available_archive_member(2, "note.md", b"# nested\n".to_vec())
                .with_media_type("text/markdown")
                .with_format_hint(FormatHint::exact("markdown")),
            unavailable_archive_member(6, "failed.bin", ContainerUnavailableKind::Failed),
            available_archive_member(0, "mail.eml", b"mail".to_vec())
                .with_nested_container_format("fixture_email"),
            unavailable_archive_member(4, "unsupported.bin", ContainerUnavailableKind::Unsupported),
            unavailable_archive_member(1, "secret.bin", ContainerUnavailableKind::Encrypted),
            unavailable_archive_member(5, "rejected.bin", ContainerUnavailableKind::Rejected),
            unavailable_archive_member(3, "skipped.bin", ContainerUnavailableKind::Skipped),
        ])
    }
}

struct EmailDecoder;

impl ContainerDecoder for EmailDecoder {
    fn format(&self) -> &str {
        "fixture_email"
    }

    fn class(&self) -> ContainerClass {
        ContainerClass::Email
    }

    fn decode(
        &self,
        context: &ContainerDecodeContext<'_>,
    ) -> Result<Vec<ContainerMember>, ContainerDecodeFailure> {
        context.check_member_capacity(16)?;
        let locator = ParentRelativeLocator::single(LocationComponent::EmailPart {
            message_id: Some("message@example.test".into()),
            mime_path: vec![IndexPosition::one_based(2).unwrap()],
            header: None,
        })
        .unwrap();
        Ok(vec![
            ContainerMember::available(0, locator, b"attachment text".to_vec())
                .with_declared_filename("attached.txt")
                .with_media_type("text/plain")
                .with_relationship(ArtifactRelationship::AttachmentOf)
                .with_disposition(ArtifactDisposition::Attachment)
                .with_format_hint(FormatHint::exact("text")),
        ])
    }
}

fn available_archive_member(order: u64, path: &str, bytes: Vec<u8>) -> ContainerMember {
    let locator = ParentRelativeLocator::single(LocationComponent::ArchiveMember {
        member_path: path.into(),
        member_index: IndexPosition::zero_based(order),
    })
    .unwrap();
    ContainerMember::available(order, locator, bytes).with_declared_filename(path)
}

fn unavailable_archive_member(
    order: u64,
    path: &str,
    kind: ContainerUnavailableKind,
) -> ContainerMember {
    let locator = ParentRelativeLocator::single(LocationComponent::ArchiveMember {
        member_path: path.into(),
        member_index: IndexPosition::zero_based(order),
    })
    .unwrap();
    ContainerMember::unavailable(
        order,
        locator,
        ContainerUnavailable::new(kind, format!("fixture.{kind:?}"))
            .with_message(format!("fixture {kind:?}")),
    )
    .with_declared_filename(path)
}

fn registry() -> ContainerDecoderRegistry {
    let mut registry = ContainerDecoderRegistry::new();
    registry.register(Arc::new(ArchiveDecoder)).unwrap();
    registry.register(Arc::new(EmailDecoder)).unwrap();
    registry
}

fn parse(
    mode: ContainerArtifactMode,
    budget: BudgetSelection,
    store: Option<&MemoryStore>,
) -> grist::container::ContainerTraversal {
    let ingestor = Ingestor::builtin().unwrap();
    let registry = registry();
    ContainerRecursor::new(&ingestor, &registry)
        .parse(
            ContainerParseRequest::new(
                RequestId::new("fixture/root").unwrap(),
                b"root archive".to_vec(),
                SourceInfo::new("root.arc"),
                "fixture_archive",
                ContainerParseOptions::new(mode),
                budget,
            ),
            store.map(|value| value as &dyn ContentAddressedArtifactSink),
        )
        .unwrap()
}

#[test]
fn nested_locators_statuses_and_budget_allocations_are_complete() {
    let result = parse(
        ContainerArtifactMode::InlinePayload,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(result.status, OperationStatus::Partial);
    assert_eq!(result.children.len(), 7);
    assert_eq!(result.children[0].source_order, 0);
    assert_eq!(result.children[0].status, ContainerChildStatus::Parsed);
    assert_eq!(result.children[0].children.len(), 1);
    let attachment = &result.children[0].children[0];
    assert_eq!(attachment.status, ContainerChildStatus::Parsed);
    assert_eq!(attachment.depth, 2);
    assert_eq!(attachment.artifact.locator.components().len(), 2);
    assert!(matches!(
        attachment.artifact.locator.components(),
        [
            LocationComponent::ArchiveMember { .. },
            LocationComponent::EmailPart { .. }
        ]
    ));
    assert_eq!(
        attachment
            .parsed
            .as_ref()
            .expect("leaf parser envelope")
            .status,
        OperationStatus::Complete
    );

    let statuses = result
        .children
        .iter()
        .map(|child| child.status)
        .collect::<Vec<_>>();
    assert!(statuses.contains(&ContainerChildStatus::Skipped));
    assert!(statuses.contains(&ContainerChildStatus::Encrypted));
    assert!(statuses.contains(&ContainerChildStatus::Unsupported));
    assert!(statuses.contains(&ContainerChildStatus::Rejected));
    assert!(statuses.contains(&ContainerChildStatus::Failed));

    let root = &result.budget_tree.root;
    assert_eq!(root.children.len(), result.children.len());
    assert_eq!(root.children[0].children.len(), 1);
    assert_eq!(
        root.children[0].children[0].allocation_id,
        attachment.allocation_id
    );
    assert_eq!(result.budget_tree.final_usage.child_artifacts, 8);
    assert_eq!(result.budget_tree.final_usage.archive_members, 7);
}

#[test]
fn all_storage_modes_keep_the_same_recursive_child_identities() {
    let inventory = parse(
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    let inline = parse(
        ContainerArtifactMode::InlinePayload,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    let store = MemoryStore::default();
    let addressed = parse(
        ContainerArtifactMode::ContentAddressed,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Some(&store),
    );

    assert_eq!(
        artifact_ids(&inventory.children),
        artifact_ids(&inline.children)
    );
    assert_eq!(
        artifact_ids(&inventory.children),
        artifact_ids(&addressed.children)
    );
    assert!(inventory.children[0].artifact.content.is_none());
    assert!(matches!(
        inline.children[0].artifact.content,
        Some(ArtifactContent::Inline(_))
    ));
    let reference = addressed.children[0]
        .artifact
        .content_reference()
        .expect("content-addressed reference");
    assert_eq!(
        store.resolve(reference).unwrap(),
        addressed.children[0]
            .artifact
            .identity
            .content
            .raw
            .as_ref()
            .map(|_| b"mail".to_vec())
            .unwrap()
    );
    assert_eq!(
        inventory.children[0].artifact.extraction.status,
        grist::container::ArtifactExtractionStatus::InventoryOnly
    );
}

#[test]
fn recursion_and_expansion_limits_are_explicit_in_the_same_tree() {
    let mut depth_budget = ResourceBudget::trusted_unbounded();
    depth_budget.max_nesting_depth = Some(1);
    let depth = parse(
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::custom(depth_budget),
        None,
    );
    let limited = &depth.children[0].children[0];
    assert_eq!(limited.status, ContainerChildStatus::BudgetLimited);
    assert_eq!(
        depth.budget_tree.root.children[0].children[0].limit_hit,
        Some(BudgetAxis::NestingDepth)
    );
    assert!(
        limited.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "grist.budget.nesting_depth.exhausted"
        })
    );

    let mut expansion_budget = ResourceBudget::trusted_unbounded();
    expansion_budget.max_archive_expansion_ratio = Some(0.1);
    let expansion = parse(
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::custom(expansion_budget),
        None,
    );
    assert_eq!(
        expansion.children[0].status,
        ContainerChildStatus::BudgetLimited
    );
    assert_eq!(
        expansion.budget_tree.root.children[0].limit_hit,
        Some(BudgetAxis::ArchiveExpansionRatio)
    );
}

#[test]
fn allocation_ids_and_source_order_are_deterministic() {
    let first = parse(
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    let second = parse(
        ContainerArtifactMode::InventoryOnly,
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        None,
    );
    assert_eq!(
        allocation_ids(&first.children),
        allocation_ids(&second.children)
    );
    assert_eq!(
        first
            .children
            .iter()
            .map(|child| child.source_order)
            .collect::<Vec<_>>(),
        (0..7).collect::<Vec<_>>()
    );
}

fn artifact_ids(children: &[ContainerChild]) -> Vec<String> {
    children
        .iter()
        .flat_map(|child| {
            std::iter::once(child.artifact.identity.artifact_id.clone())
                .chain(artifact_ids(&child.children))
        })
        .collect()
}

fn allocation_ids(children: &[ContainerChild]) -> Vec<String> {
    children
        .iter()
        .flat_map(|child| {
            std::iter::once(child.allocation_id.clone()).chain(allocation_ids(&child.children))
        })
        .collect()
}
