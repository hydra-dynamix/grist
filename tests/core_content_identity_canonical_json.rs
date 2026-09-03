use grist::core::{
    AggregateContentIdentity, AggregateMemberIdentity, ArtifactKind, CanonicalJsonVersion,
    ContentIdentity, Diagnostic, Envelope, FormatIdentity, OperationKind, ParserInfo, SourceInfo,
    canonical_json_bytes, canonical_json_sha256, empty_options_digest, options_digest,
};
use grist::detect::detect_path;
use serde_json::{Map, json};
use std::path::Path;

fn member(path: &str, index: u64, bytes: &[u8]) -> AggregateMemberIdentity {
    AggregateMemberIdentity::new(path, Some(index), &ContentIdentity::for_raw_bytes(bytes))
}

#[test]
fn sha256_identities_match_golden_vectors() {
    let empty = ContentIdentity::for_raw_bytes(b"");
    assert_eq!(
        empty.raw.unwrap().sha256,
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );

    let abc = ContentIdentity::for_raw_bytes(b"abc");
    assert_eq!(
        abc.raw.unwrap().sha256,
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );

    assert_eq!(
        canonical_json_sha256(&json!({"b": 2, "a": 1})).unwrap(),
        "sha256:43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777"
    );
}

#[test]
fn canonical_json_is_byte_stable_across_map_insertion_order() {
    let mut first = Map::new();
    first.insert("b".into(), json!(2));
    first.insert("a".into(), json!(1));

    let mut second = Map::new();
    second.insert("a".into(), json!(1));
    second.insert("b".into(), json!(2));

    let expected = br#"{"a":1,"b":2}"#;
    assert_eq!(canonical_json_bytes(&first).unwrap(), expected);
    assert_eq!(canonical_json_bytes(&second).unwrap(), expected);
    assert_eq!(
        options_digest(&first).unwrap(),
        options_digest(&second).unwrap()
    );
}

#[test]
fn lossy_decoding_and_its_diagnostic_do_not_change_raw_identity() {
    let bytes = [0x66, 0x80, 0x6f];
    let raw = ContentIdentity::for_raw_bytes(&bytes);
    let raw_digest = raw.raw.as_ref().unwrap().sha256.clone();

    let decoded = raw.with_decoded("f\u{fffd}o", "windows-1252", true);
    let _diagnostic = Diagnostic::warning(
        "grist.decode",
        "decode.replacement",
        "one undecodable byte was replaced",
    )
    .partial();

    assert_eq!(
        raw_digest,
        "sha256:edb3d848684a3437ea1944dd1361b87aa07f7dbefbf5498b83d85875f50f0444"
    );
    assert_eq!(decoded.raw.as_ref().unwrap().sha256, raw_digest);
    assert!(decoded.decoded.as_ref().unwrap().lossy);
    assert_ne!(
        decoded.decoded.as_ref().unwrap().sha256,
        decoded.raw.as_ref().unwrap().sha256
    );
}

#[test]
fn aggregate_identity_is_stable_across_parallel_completion_order() {
    let forward = vec![member("a.txt", 1, b"alpha"), member("b.txt", 2, b"beta")];
    let reverse = vec![member("b.txt", 2, b"beta"), member("a.txt", 1, b"alpha")];

    let forward = AggregateContentIdentity::new(forward).unwrap();
    let reverse = AggregateContentIdentity::new(reverse).unwrap();

    assert_eq!(forward, reverse);
    assert_eq!(forward.manifest_byte_length, 393);
    assert_eq!(
        forward.sha256,
        "sha256:688a61274a420dc6df6406e6395630b76494db41ed416f16fcb34cee63f6f2d5"
    );
    assert_eq!(forward.total_byte_length, 9);
    assert_eq!(forward.members[0].member_path, "a.txt");
}

#[test]
fn detection_retains_ranked_evidence_and_format_media_identity() {
    let bytes = b"{\"answer\":42}";
    let detection = detect_path(Path::new("answer.json"), bytes, &Default::default());

    assert_eq!(detection.candidates.len(), 1);
    assert_eq!(detection.candidates[0].rank, 1);
    assert_eq!(detection.candidates[0].identity.format, "json");
    assert_eq!(
        detection.candidates[0].identity.media_type.as_deref(),
        Some("application/json")
    );
    assert!(!detection.candidates[0].evidence.is_empty());

    let identity = detection.apply_to_identity(ContentIdentity::for_raw_bytes(bytes).with_decoded(
        std::str::from_utf8(bytes).unwrap(),
        "utf-8",
        false,
    ));
    assert_eq!(
        identity.format,
        Some(FormatIdentity::new("json", Some("application/json")))
    );
    assert_eq!(identity.detection_candidates, detection.candidates);
}

#[test]
fn canonicalization_versions_are_explicit_compatibility_gates() {
    assert_eq!(
        serde_json::to_value(CanonicalJsonVersion::CURRENT).unwrap(),
        "grist/canonical-json/v1"
    );
    assert!(
        serde_json::from_value::<CanonicalJsonVersion>(json!("grist/canonical-json/v2")).is_err()
    );
}

#[test]
fn envelope_json_exposes_raw_and_canonical_payload_identities() {
    let payload = json!({"b": 2, "a": 1});
    let envelope = Envelope::complete(
        OperationKind::Parse,
        ArtifactKind::Serialization,
        SourceInfo::stdin("value.json"),
        ParserInfo::new("test"),
        empty_options_digest(),
        "example/value/v1",
        payload,
    )
    .with_identity(ContentIdentity::for_raw_bytes(br#"{"b":2,"a":1}"#));

    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(
        value["identity"]["raw"]["sha256"],
        "sha256:3fb75453225c732a76b7899ea2096dda1455189c89817239732182f73fe5a09f"
    );
    assert_eq!(
        value["identity"]["canonical_payload"]["sha256"],
        "sha256:43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777"
    );
    assert_eq!(
        value["identity"]["canonical_payload"]["payload_schema_version"],
        "example/value/v1"
    );
    assert_eq!(
        value["identity"]["canonical_payload"]["canonicalization"],
        "grist/canonical-json/v1"
    );
}

#[test]
fn compound_runtime_input_exposes_an_aggregate_member_identity() {
    use grist::core::{BudgetProfile, BudgetSelection, CompoundMemberInput, Input};

    let input = Input::compound_member(CompoundMemberInput::new(
        SourceInfo::stdin("archive.zip"),
        "nested/data.json",
        Some(7),
        Input::bytes(b"{}"),
    ));
    let resolved = input
        .resolve(&BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1))
        .unwrap();
    let member = resolved.aggregate_member_identity().unwrap();

    assert_eq!(member.member_path, "nested/data.json");
    assert_eq!(member.member_index, Some(7));
    assert_eq!(member.byte_length, 2);
}
