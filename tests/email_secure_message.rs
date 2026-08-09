#![cfg(feature = "email-message")]

use grist::core::{
    BudgetSelection, ContentIdentity, Input, NetworkAccess, OperationStatus, ParseRequest,
    ProviderKind, ProviderSet, RequestId, ResourceBudget, SourceInfo,
};
use grist::document_graph::{DocumentGraphContext, ToDocumentGraph};
use grist::email::{EmailDocument, EmailOptions, SmimeKind, parse_email};
use grist::provider::{
    DecryptionProvider, DecryptionProviderAdapter, DecryptionRequest, DecryptionResult,
    ProviderDeterminism, ProviderError, ProviderMetadata,
};
use grist::registry::builtin_parser_registry;
use grist::segment::{SegmentOptions, segment_document_graph};
use std::sync::Arc;

#[test]
fn tnef_attributes_are_inert_located_and_checksum_verified() {
    let mut tnef = vec![0x78, 0x9f, 0x3e, 0x22, 0x01, 0x00];
    tnef.extend_from_slice(&[0x01, 0x0c, 0x80, 0x02, 0x00]);
    tnef.extend_from_slice(&3u32.to_le_bytes());
    tnef.extend_from_slice(b"abc");
    tnef.extend_from_slice(&294u16.to_le_bytes());
    let mut message = concat!(
        "Content-Type: application/ms-tnef; name=winmail.dat\r\n",
        "Content-Disposition: attachment; filename=winmail.dat\r\n",
        "Content-Transfer-Encoding: binary\r\n\r\n"
    )
    .as_bytes()
    .to_vec();
    message.extend_from_slice(&tnef);

    let envelope = parse_email(
        &message,
        SourceInfo::new("tnef.eml"),
        &EmailOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document = envelope.payload().unwrap();
    let parsed = document.mime.tnef.as_ref().unwrap();
    assert!(parsed.signature_valid && parsed.complete);
    assert_eq!(parsed.identity.byte_length, tnef.len() as u64);
    assert_eq!(parsed.attributes.len(), 1);
    assert_eq!(parsed.attributes[0].known_name.as_deref(), Some("body"));
    assert!(parsed.attributes[0].checksum_valid);
    assert_eq!(parsed.attributes[0].locator.components().len(), 3);
}

#[test]
fn malformed_tnef_is_retained_as_partial_without_overread() {
    let mut message =
        b"Content-Type: application/ms-tnef\r\nContent-Transfer-Encoding: binary\r\n\r\n".to_vec();
    message.extend_from_slice(&[0x78, 0x9f, 0x3e, 0x22, 0x01, 0x00, 0x01, 0x0c]);
    let envelope = parse_email(
        &message,
        SourceInfo::new("truncated-tnef.eml"),
        &EmailOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let tnef = envelope.payload().unwrap().mime.tnef.as_ref().unwrap();
    assert!(tnef.signature_valid);
    assert!(!tnef.complete);
    assert_eq!(tnef.trailing_bytes, 2);
}

#[test]
fn signed_smime_preserves_native_signature_and_segments_only_signed_text() {
    let message = concat!(
        "Content-Type: multipart/signed; boundary=s; protocol=application/pkcs7-signature; micalg=sha-256\r\n\r\n",
        "--s\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nsigned body\r\n",
        "--s\r\nContent-Type: application/pkcs7-signature\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\nc2lnbmF0dXJlLWJ5dGVz\r\n",
        "--s--\r\n"
    );
    let envelope = parse_email(
        message.as_bytes(),
        SourceInfo::new("signed.eml"),
        &EmailOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document = envelope.payload().unwrap();
    let signed = document.mime.smime.as_ref().unwrap();
    assert_eq!(signed.kind, SmimeKind::MultipartSigned);
    assert_eq!(signed.signed_content_path.as_deref(), Some(&[1][..]));
    assert_eq!(signed.signature_path.as_deref(), Some(&[2][..]));
    let signature = document.mime.children[1].smime.as_ref().unwrap();
    assert_eq!(signature.kind, SmimeKind::DetachedSignature);
    assert_eq!(signature.native.identity.byte_length, 20);

    let graph = document
        .to_document_graph(DocumentGraphContext::new("graph:signed"))
        .unwrap();
    assert!(graph.nodes.iter().any(|node| {
        node.attrs
            .get("smime")
            .is_some_and(serde_json::Value::is_object)
    }));
    let source_identity = envelope.identity.as_ref().unwrap();
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
    let text = segments
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("signed body"));
    assert!(!text.contains("signature-bytes"));
}

struct FixtureDecryptor {
    fail: bool,
    credential: &'static str,
}

impl DecryptionProvider for FixtureDecryptor {
    fn decrypt(&self, _request: &DecryptionRequest<'_>) -> Result<DecryptionResult, ProviderError> {
        let _credential_is_present = !self.credential.is_empty();
        if self.fail {
            Err(ProviderError::failure(
                "fixture-smime",
                "fixture decryption failed",
            ))
        } else {
            Ok(DecryptionResult::new(
                b"Content-Type: text/plain; charset=utf-8\r\n\r\ndecrypted body".to_vec(),
                Some("message/rfc822"),
            ))
        }
    }
}

fn dispatch_encrypted(fail: bool) -> grist::core::Envelope<serde_json::Value> {
    dispatch_encrypted_with_budget(fail, ResourceBudget::trusted_unbounded())
}

fn dispatch_encrypted_with_budget(
    fail: bool,
    budget: ResourceBudget,
) -> grist::core::Envelope<serde_json::Value> {
    let provider = DecryptionProviderAdapter::new(
        ProviderMetadata::new(
            "fixture-smime",
            "fixture-decryptor",
            "1",
            ProviderDeterminism::Guaranteed,
        )
        .unwrap(),
        FixtureDecryptor {
            fail,
            credential: "fixture-private-key-material",
        },
    )
    .unwrap();
    let mut providers = ProviderSet::none();
    providers.select(
        ProviderKind::Decryption,
        Arc::new(provider),
        NetworkAccess::Denied,
    );
    let request = ParseRequest::new(
        RequestId::new(if fail { "smime-fail" } else { "smime-ok" }).unwrap(),
        Input::bytes(
            b"Content-Type: application/pkcs7-mime; smime-type=enveloped-data\r\nContent-Transfer-Encoding: base64\r\n\r\nb3BhcXVl"
                .to_vec(),
        ),
        SourceInfo::new("encrypted.eml"),
        BudgetSelection::custom(budget),
        providers,
    );
    builtin_parser_registry()
        .unwrap()
        .dispatch("eml", request, None)
        .unwrap()
}

#[test]
fn decrypted_provider_output_obeys_the_shared_memory_budget() {
    let mut budget = ResourceBudget::trusted_unbounded();
    budget.max_memory_bytes = Some(120);
    let envelope = dispatch_encrypted_with_budget(false, budget);
    assert_eq!(envelope.status, OperationStatus::Partial);
    let document: EmailDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    let attempt = document
        .mime
        .smime
        .as_ref()
        .unwrap()
        .decryption
        .as_ref()
        .unwrap();
    assert_eq!(attempt.status, OperationStatus::Partial);
    assert!(attempt.content_identity.is_some());
    assert!(attempt.parsed.is_none());
    assert!(
        attempt
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "grist.budget.memory_bytes.exhausted")
    );
}

#[test]
fn explicit_provider_adds_separate_decrypted_representation_and_provenance() {
    let envelope = dispatch_encrypted(false);
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert_eq!(envelope.providers.len(), 1);
    assert_eq!(envelope.provenance.len(), 2);
    let document: EmailDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    assert!(document.mime.encrypted);
    assert_eq!(
        document
            .mime
            .smime
            .as_ref()
            .unwrap()
            .native
            .identity
            .byte_length,
        8
    );
    let decryption = document
        .mime
        .smime
        .as_ref()
        .unwrap()
        .decryption
        .as_ref()
        .unwrap();
    assert_eq!(decryption.status, OperationStatus::Complete);
    assert_eq!(decryption.provider, "fixture-smime");
    assert_eq!(
        decryption.content_identity.as_ref().unwrap().byte_length,
        57
    );
    assert_eq!(
        decryption.parsed.as_ref().unwrap().envelope["payload"]["mime"]["text"]["text"],
        "decrypted body"
    );
    let serialized = serde_json::to_string(&document).unwrap();
    assert!(!serialized.contains("opaque"));
    assert!(!serialized.contains("fixture-private-key-material"));
}

#[test]
fn provider_failure_preserves_ciphertext_evidence_and_partial_status() {
    let envelope = dispatch_encrypted(true);
    assert_eq!(envelope.status, OperationStatus::Partial);
    assert_eq!(envelope.providers.len(), 1);
    let document: EmailDocument = serde_json::from_value(envelope.payload.unwrap()).unwrap();
    let smime = document.mime.smime.as_ref().unwrap();
    assert_eq!(smime.native.identity.byte_length, 8);
    let decryption = smime.decryption.as_ref().unwrap();
    assert_eq!(decryption.status, OperationStatus::Failed);
    assert!(decryption.parsed.is_none());
    assert!(document.mime.encrypted);
}

#[cfg(feature = "cli")]
#[test]
fn cli_auto_parse_exposes_inert_smime_without_a_provider() {
    use std::fs;
    use std::process::Command;

    let path = std::env::temp_dir().join(format!(
        "grist-secure-message-contract-{}.eml",
        std::process::id()
    ));
    fs::write(
        &path,
        b"Content-Type: application/pkcs7-mime; smime-type=enveloped-data\r\n\r\nopaque",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_grist"))
        .args(["parse", "auto", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "partial");
    assert_eq!(value["payload"]["mime"]["smime"]["kind"], "enveloped_data");
    assert_eq!(value["providers"], serde_json::json!([]));
}
