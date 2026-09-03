use grist::core::{
    CanonicalPayloadIdentity, LineIndex, OperationKind, OperationStatus, SourceRange, sha256_hex,
};
use grist::document_graph::{DocumentGraph, DocumentKind, DocumentNode, DocumentNodeKind};
use grist::render::{ReconstructionClaim, RenderFormat, RenderOptions, render_document_graph};
use grist::transform::{
    FormatReconstructionClaim, FormatReconstructor, GraphTransformOptions,
    NormalizedGraphOperation, PackageByteRange, ReconstructionFidelity,
    ReconstructionFixtureEvidence, ReconstructionOptions, ReconstructionProduct,
    ReconstructionSourceMapEntry, TransformMapStatus, reconstruct_package,
    transform_document_graph,
};

fn graph() -> DocumentGraph {
    let text = "If a package is rebuilt, it must retain its content.";
    let index = LineIndex::new(text);
    let mut graph = DocumentGraph::new("transform:test", DocumentKind::Document);
    graph.add_node(
        DocumentNode::new("root", DocumentNodeKind::Document).with_range(SourceRange::new(
            0,
            text.len(),
            &index,
        )),
    );
    graph.add_node(
        DocumentNode::new("rule", DocumentNodeKind::Paragraph)
            .with_text(text)
            .with_range(SourceRange::new(0, text.len(), &index))
            .with_ordinal(0),
    );
    graph.add_contains("root", "rule");
    graph
}

#[test]
fn normalized_operations_return_envelopes_and_complete_source_maps() {
    let input = graph();
    let envelope = transform_document_graph(
        &input,
        &GraphTransformOptions {
            operations: vec![NormalizedGraphOperation::ExtractConditionalObligations],
        },
    )
    .unwrap();
    assert_eq!(envelope.operation, OperationKind::Transform);
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert_eq!(envelope.provenance.len(), 1);
    assert!(envelope.provenance[0].input_identity.is_some());
    assert!(envelope.provenance[0].output_identity.is_some());
    let result = envelope.payload.unwrap();
    assert!(result.fidelity.lossless);
    assert!(result.fidelity.losses.is_empty());
    assert_eq!(result.source_map.entries.len(), result.graph.nodes.len());
    for original in &input.nodes {
        let entry = result
            .source_map
            .entries
            .iter()
            .find(|entry| entry.output_node_id == original.id)
            .unwrap();
        assert_eq!(entry.status, TransformMapStatus::Preserved);
        assert_eq!(
            entry.input_node_ids.as_slice(),
            std::slice::from_ref(&original.id)
        );
    }
    let derived = result
        .source_map
        .entries
        .iter()
        .filter(|entry| entry.status == TransformMapStatus::Derived)
        .collect::<Vec<_>>();
    assert_eq!(derived.len(), 3);
    assert!(
        derived.iter().all(|entry| {
            entry.input_node_ids == ["rule"] && !entry.derivation_steps.is_empty()
        })
    );
}

#[test]
fn invalid_graph_transform_pipelines_fail_closed() {
    let graph = graph();
    assert!(
        transform_document_graph(
            &graph,
            &GraphTransformOptions {
                operations: Vec::new(),
            },
        )
        .is_err()
    );
    assert!(
        transform_document_graph(
            &graph,
            &GraphTransformOptions {
                operations: vec![
                    NormalizedGraphOperation::Identity,
                    NormalizedGraphOperation::Identity,
                ],
            },
        )
        .is_err()
    );
}

struct FixtureReconstructor {
    claim: FormatReconstructionClaim,
    bytes: Vec<u8>,
}

impl FormatReconstructor for FixtureReconstructor {
    type Error = std::io::Error;

    fn claim(&self) -> &FormatReconstructionClaim {
        &self.claim
    }

    fn reconstruct(
        &self,
        _graph: &DocumentGraph,
        _options: &ReconstructionOptions,
    ) -> Result<ReconstructionProduct, Self::Error> {
        Ok(ReconstructionProduct {
            package_bytes: self.bytes.clone(),
            achieved_fidelity: ReconstructionFidelity::ByteIdentical,
            differences: Vec::new(),
            source_map: vec![ReconstructionSourceMapEntry {
                package_part: "/".to_string(),
                generated: Some(PackageByteRange {
                    byte_start: 0,
                    byte_end: self.bytes.len(),
                }),
                source_node_ids: vec!["root".to_string()],
            }],
        })
    }
}

fn fixture_reconstructor(graph: &DocumentGraph) -> FixtureReconstructor {
    let bytes = include_bytes!("../fixtures/generated/reconstruction/minimal.gristpkg").to_vec();
    let input_sha256 = CanonicalPayloadIdentity::new(graph.schema_version.as_str(), graph)
        .unwrap()
        .sha256;
    FixtureReconstructor {
        claim: FormatReconstructionClaim {
            format: "grist_fixture_package".to_string(),
            media_type: "application/x-grist-fixture-package".to_string(),
            package_profile: "minimal-v1".to_string(),
            implementation: "grist.test.fixture-reconstructor".to_string(),
            implementation_version: "1".to_string(),
            maximum_fidelity: ReconstructionFidelity::ByteIdentical,
            fixture_evidence: vec![ReconstructionFixtureEvidence {
                fixture_id: "fixtures/generated/reconstruction/minimal.gristpkg".to_string(),
                input_sha256,
                expected_package_sha256: sha256_hex(&bytes),
                verified_fidelity: ReconstructionFidelity::ByteIdentical,
            }],
        },
        bytes,
    }
}

#[test]
fn format_reconstruction_is_fixture_scoped_and_always_reports_fidelity() {
    let graph = graph();
    let reconstructor = fixture_reconstructor(&graph);
    let expected = sha256_hex(&reconstructor.bytes);
    let envelope = reconstruct_package(
        &reconstructor,
        &graph,
        &ReconstructionOptions {
            required_fidelity: ReconstructionFidelity::ByteIdentical,
            expected_package_sha256: Some(expected.clone()),
        },
    )
    .unwrap();
    assert_eq!(envelope.operation, OperationKind::Transform);
    assert_eq!(envelope.status, OperationStatus::Complete);
    let result = envelope.payload.unwrap();
    assert_eq!(result.package_sha256, expected);
    assert_eq!(
        result.fidelity_report.achieved_fidelity,
        ReconstructionFidelity::ByteIdentical
    );
    assert_eq!(result.fidelity_report.format, "grist_fixture_package");
    assert_eq!(result.fidelity_report.fixture_evidence.len(), 1);
    assert!(result.fidelity_report.differences.is_empty());
}

#[test]
fn reconstruction_rejects_unbacked_or_false_exact_claims() {
    let graph = graph();
    let mut reconstructor = fixture_reconstructor(&graph);
    reconstructor.claim.fixture_evidence.clear();
    assert!(
        reconstruct_package(&reconstructor, &graph, &ReconstructionOptions::default()).is_err()
    );

    let reconstructor = fixture_reconstructor(&graph);
    let wrong = format!("sha256:{}", "0".repeat(64));
    assert!(
        reconstruct_package(
            &reconstructor,
            &graph,
            &ReconstructionOptions {
                required_fidelity: ReconstructionFidelity::ByteIdentical,
                expected_package_sha256: Some(wrong),
            },
        )
        .is_err()
    );
}

#[test]
fn normalized_rendering_never_claims_package_reconstruction() {
    let rendered = render_document_graph(
        &graph(),
        RenderFormat::CanonicalJson,
        &RenderOptions::default(),
    )
    .unwrap();
    assert_eq!(
        rendered.fidelity.reconstruction_claim,
        ReconstructionClaim::NormalizedNotByteRoundTrip
    );
}
