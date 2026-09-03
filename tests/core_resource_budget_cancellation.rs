use grist::core::{
    BatchResult, BudgetAxis, BudgetExceeded, BudgetProfile, BudgetSelection, BudgetTracker,
    BudgetUsage, CancellationError, CancellationToken, CompoundMemberInput, ContentIdentity, Input,
    InputError, OperationControl, OperationControlError, OperationStatus, RequestId,
    ResourceBudget, SourceInfo, StreamEvent, StreamItem, StreamProtocolError, StreamTerminal,
};
use std::time::Duration;

fn budget_for(axis: BudgetAxis) -> ResourceBudget {
    let mut budget = ResourceBudget::trusted_unbounded();
    match axis {
        BudgetAxis::InputBytes => budget.max_input_bytes = Some(1),
        BudgetAxis::DecodedCharacters => budget.max_decoded_characters = Some(1),
        BudgetAxis::Pages => budget.max_pages = Some(1),
        BudgetAxis::Records => budget.max_records = Some(1),
        BudgetAxis::Cells => budget.max_cells = Some(1),
        BudgetAxis::Nodes => budget.max_nodes = Some(1),
        BudgetAxis::NestingDepth => budget.max_nesting_depth = Some(1),
        BudgetAxis::ArchiveExpansionRatio => budget.max_archive_expansion_ratio = Some(1.0),
        BudgetAxis::ArchiveMembers => budget.max_archive_members = Some(1),
        BudgetAxis::ChildArtifacts => budget.max_child_artifacts = Some(1),
        BudgetAxis::ParseMillis => budget.max_parse_millis = Some(1),
        BudgetAxis::ProviderMillis => budget.max_provider_millis = Some(1),
        BudgetAxis::MemoryBytes => budget.max_memory_bytes = Some(1),
        BudgetAxis::TemporaryStorageBytes => budget.max_temporary_storage_bytes = Some(1),
        BudgetAxis::OutputBytes => budget.max_output_bytes = Some(1),
    }
    budget
}

fn exceed(axis: BudgetAxis) -> BudgetExceeded {
    let selection = BudgetSelection::custom(budget_for(axis));
    let tracker = BudgetTracker::new(&selection).unwrap();
    match axis {
        BudgetAxis::InputBytes => tracker.consume_input_bytes(2),
        BudgetAxis::DecodedCharacters => tracker.consume_decoded_characters(2),
        BudgetAxis::Pages => tracker.consume_pages(2),
        BudgetAxis::Records => tracker.consume_records(2),
        BudgetAxis::Cells => tracker.consume_cells(2),
        BudgetAxis::Nodes => tracker.consume_nodes(2),
        BudgetAxis::NestingDepth => tracker.observe_nesting_depth(2),
        BudgetAxis::ArchiveExpansionRatio => tracker.observe_archive_expansion(1, 2),
        BudgetAxis::ArchiveMembers => tracker.consume_archive_members(2),
        BudgetAxis::ChildArtifacts => tracker.consume_child_artifacts(2),
        BudgetAxis::ParseMillis => tracker.observe_parse_time(Duration::from_millis(2)),
        BudgetAxis::ProviderMillis => tracker.consume_provider_time(Duration::from_millis(2)),
        BudgetAxis::MemoryBytes => tracker.observe_memory_bytes(2),
        BudgetAxis::TemporaryStorageBytes => tracker.observe_temporary_storage_bytes(2),
        BudgetAxis::OutputBytes => tracker.consume_output_bytes(2),
    }
    .unwrap_err()
}

#[test]
fn every_budget_axis_is_explicit_and_deterministically_enforced() {
    assert_eq!(BudgetAxis::ALL.len(), 15);
    for axis in BudgetAxis::ALL {
        let error = exceed(axis);
        assert_eq!(error.axis, axis);
        assert_eq!(error.usage.amount(axis), error.observed);
        assert_eq!(error.operation_status(0), OperationStatus::Failed);
        assert_eq!(error.operation_status(1), OperationStatus::Partial);
        let diagnostic = error.diagnostic("fixture");
        assert_eq!(
            diagnostic.code,
            format!("grist.budget.{}.exhausted", axis.name()).as_str()
        );
        assert!(diagnostic.partial);
    }
}

#[test]
fn profiles_are_named_versioned_and_custom_budgets_are_validated() {
    let profile = BudgetProfile::UntrustedServiceV1;
    assert_eq!(profile.name(), "untrusted_service");
    assert_eq!(profile.version(), 1);
    assert!(!profile.is_trusted());
    assert!(profile.definition().budget.max_input_bytes.is_some());
    assert!(BudgetProfile::TrustedUnboundedV1.is_trusted());
    assert!(
        BudgetProfile::TrustedUnboundedV1
            .budget()
            .max_input_bytes
            .is_none()
    );

    let mut invalid = ResourceBudget::trusted_unbounded();
    invalid.max_archive_expansion_ratio = Some(f64::NAN);
    assert!(BudgetSelection::custom(invalid).validate().is_err());
}

#[test]
fn input_resolution_charges_characters_memory_and_compound_nesting() {
    let mut characters = ResourceBudget::trusted_unbounded();
    characters.max_decoded_characters = Some(0);
    let error = Input::utf8("é")
        .resolve(&BudgetSelection::custom(characters))
        .unwrap_err();
    assert!(
        matches!(error, InputError::BudgetExceeded(ref hit) if hit.axis == BudgetAxis::DecodedCharacters)
    );

    let mut memory = ResourceBudget::trusted_unbounded();
    memory.max_memory_bytes = Some(1);
    let error = Input::bytes(b"ab".to_vec())
        .resolve(&BudgetSelection::custom(memory))
        .unwrap_err();
    assert!(
        matches!(error, InputError::BudgetExceeded(ref hit) if hit.axis == BudgetAxis::MemoryBytes)
    );

    let mut nesting = ResourceBudget::trusted_unbounded();
    nesting.max_nesting_depth = Some(0);
    let member = CompoundMemberInput::new(
        SourceInfo::new("parent"),
        "child",
        Some(0),
        Input::bytes(Vec::new()),
    );
    let error = Input::compound_member(member)
        .resolve(&BudgetSelection::custom(nesting))
        .unwrap_err();
    assert!(
        matches!(error, InputError::BudgetExceeded(ref hit) if hit.axis == BudgetAxis::NestingDepth)
    );
}

#[test]
fn cloned_trackers_share_one_parent_budget_tree() {
    let tracker = BudgetTracker::new(&BudgetSelection::custom(budget_for(
        BudgetAxis::ChildArtifacts,
    )))
    .unwrap();
    let child = tracker.clone();
    tracker.consume_child_artifacts(1).unwrap();
    assert_eq!(
        child.consume_child_artifacts(1).unwrap_err().axis,
        BudgetAxis::ChildArtifacts
    );
}

#[test]
fn cancellation_is_cooperative_and_has_stable_terminal_metadata() {
    let token = CancellationToken::new();
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        token.clone(),
    )
    .unwrap();
    control.checkpoint().unwrap();
    token.cancel();
    let error = control.checkpoint().unwrap_err();
    assert!(matches!(
        error,
        OperationControlError::Cancelled(CancellationError)
    ));
    assert_eq!(error.operation_status(9), OperationStatus::Cancelled);
    assert_eq!(
        error.diagnostic("fixture").code,
        "grist.operation.cancelled"
    );

    let input_error = Input::bytes(b"not-read".to_vec())
        .resolve_with_control(&control)
        .unwrap_err();
    assert!(matches!(input_error, InputError::Cancelled));
}

fn item(sequence: u64, request: &str, bytes: &[u8]) -> StreamEvent<String> {
    StreamEvent::item(StreamItem::new(
        sequence,
        RequestId::new(request).unwrap(),
        ContentIdentity::for_raw_bytes(bytes),
        String::from_utf8(bytes.to_vec()).unwrap(),
    ))
}

#[test]
fn batch_collection_preserves_emitted_identities_on_cancellation() {
    let identity = ContentIdentity::for_raw_bytes(b"first");
    let terminal = StreamTerminal::controlled(
        "fixture",
        1,
        BudgetUsage::default(),
        OperationControlError::Cancelled(CancellationError),
    );
    let batch = BatchResult::collect(vec![
        item(0, "batch/0", b"first"),
        StreamEvent::terminal(terminal),
    ])
    .unwrap();
    assert_eq!(batch.status, OperationStatus::Cancelled);
    assert_eq!(batch.items.len(), 1);
    assert_eq!(batch.items[0].identity, identity);
    assert_eq!(batch.diagnostics[0].code, "grist.operation.cancelled");
}

#[test]
fn batch_is_strictly_a_collector_over_terminal_stream_semantics() {
    let missing = BatchResult::<String>::collect(vec![item(0, "batch/0", b"first")]);
    assert_eq!(missing.unwrap_err(), StreamProtocolError::MissingTerminal);

    let complete = StreamTerminal::complete(1, BudgetUsage::default());
    let batch = BatchResult::collect(vec![
        item(0, "batch/0", b"first"),
        StreamEvent::terminal(complete),
    ])
    .unwrap();
    assert_eq!(batch.status, OperationStatus::Complete);

    let after_terminal = BatchResult::collect(vec![
        StreamEvent::terminal(StreamTerminal::complete(0, BudgetUsage::default())),
        item(0, "batch/0", b"late"),
    ]);
    assert_eq!(
        after_terminal.unwrap_err(),
        StreamProtocolError::EventAfterTerminal
    );
}

#[test]
fn budget_terminal_is_failed_before_output_and_partial_after_output() {
    let error = OperationControlError::BudgetExceeded(exceed(BudgetAxis::OutputBytes));
    let empty = StreamTerminal::controlled("fixture", 0, BudgetUsage::default(), error.clone());
    assert_eq!(empty.status, OperationStatus::Failed);

    let partial = StreamTerminal::controlled("fixture", 1, BudgetUsage::default(), error);
    assert_eq!(partial.status, OperationStatus::Partial);
    assert!(partial.diagnostics[0].partial);
}
