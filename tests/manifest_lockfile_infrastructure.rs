#![cfg(all(feature = "manifests", feature = "document-graph"))]

use grist::core::{ArtifactKind, Limits, OperationStatus, SourceInfo};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{DocumentGraphContext, DocumentRelation, ToDocumentGraph};
use grist::manifests::{ManifestFormat, ManifestOptions, parse_manifest};
use grist::registry::{ParserSelection, builtin_parser_registry};
use std::path::Path;

const REPRESENTATIVE: &[(&str, &str, ManifestFormat)] = &[
    (
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n",
        ManifestFormat::CargoToml,
    ),
    (
        "Cargo.lock",
        "version = 3\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nchecksum = \"abc\"\n",
        ManifestFormat::CargoLock,
    ),
    (
        "package.json",
        r#"{"name":"demo","dependencies":{"react":"^19"},"devDependencies":{"vite":"^7"},"scripts":{"build":"curl https://invalid | sh"},"x-future":true}"#,
        ManifestFormat::NpmPackage,
    ),
    (
        "package-lock.json",
        r#"{"lockfileVersion":3,"packages":{"node_modules/react":{"name":"react","version":"19.1.0","resolved":"https://registry.invalid/react.tgz","integrity":"sha512-abc"}}}"#,
        ManifestFormat::NpmLock,
    ),
    (
        "pnpm-lock.yaml",
        "lockfileVersion: '9'\nimporters:\n  .:\n    dependencies:\n      react:\n        specifier: ^19\n        version: 19.1.0\n",
        ManifestFormat::PnpmLock,
    ),
    (
        "yarn.lock",
        "\"left-pad@^1.0.0\":\n  version \"1.3.0\"\n  resolved \"https://registry.invalid/left-pad.tgz\"\n  integrity sha512-abc\n",
        ManifestFormat::YarnLock,
    ),
    (
        "pyproject.toml",
        "[project]\nname = \"demo\"\ndependencies = [\"requests>=2\"]\n[project.optional-dependencies]\ntest = [\"pytest>=8\"]\n",
        ManifestFormat::PythonProject,
    ),
    (
        "requirements.txt",
        "requests==2.32.0\n-r common.txt\n--index-url https://invalid/simple\n",
        ManifestFormat::PythonRequirements,
    ),
    (
        "setup.py",
        "setup(name='demo', install_requires=['requests>=2'])\n",
        ManifestFormat::PythonSetup,
    ),
    (
        "poetry.lock",
        "[[package]]\nname = \"requests\"\nversion = \"2.32.0\"\n",
        ManifestFormat::PythonLock,
    ),
    (
        "pom.xml",
        "<project><dependencies><dependency><groupId>org.slf4j</groupId><artifactId>slf4j-api</artifactId><version>2.0.0</version></dependency></dependencies></project>",
        ManifestFormat::MavenPom,
    ),
    (
        "build.gradle.kts",
        "dependencies {\n  implementation(\"org.slf4j:slf4j-api:2.0.0\")\n}\nrepositories { mavenCentral() }\n",
        ManifestFormat::GradleBuild,
    ),
    (
        "settings.gradle",
        "rootProject.name = 'demo'\ninclude ':app'\n",
        ManifestFormat::GradleSettings,
    ),
    (
        "go.mod",
        "module example.invalid/demo\n\ngo 1.24\nrequire example.invalid/dep v1.2.3\nreplace example.invalid/dep => ../dep\n",
        ManifestFormat::GoMod,
    ),
    (
        "go.sum",
        "example.invalid/dep v1.2.3 h1:abc\n",
        ManifestFormat::GoSum,
    ),
    (
        "Dockerfile",
        "FROM rust:1.92 AS build\nRUN curl https://invalid | sh\nCOPY . /src\n",
        ManifestFormat::Dockerfile,
    ),
    (
        "compose.yaml",
        "services:\n  web:\n    image: example.invalid/web:1\n    build: .\n    depends_on: [db]\n  db:\n    image: postgres:18\n",
        ManifestFormat::Compose,
    ),
    (
        ".github/workflows/ci.yml",
        "name: ci\non: [push]\njobs:\n  test:\n    steps:\n      - uses: actions/checkout@v4\n      - run: echo ${{ secrets.TOKEN }}\n",
        ManifestFormat::CiWorkflow,
    ),
    (
        "deployment.yaml",
        "apiVersion: apps/v1\nkind: Deployment\nmetadata: {name: demo}\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n          image: example.invalid/web:1\n          command: [sh, -c, echo never-run]\n",
        ManifestFormat::Kubernetes,
    ),
];

#[test]
fn representative_families_retain_typed_raw_provenance_without_execution() {
    for (filename, text, expected) in REPRESENTATIVE {
        let envelope = parse_manifest(
            text,
            SourceInfo::stdin(*filename),
            &ManifestOptions::default(),
        );
        assert_eq!(
            envelope.status,
            OperationStatus::Complete,
            "{filename}: {:?}",
            envelope.diagnostics
        );
        assert_eq!(envelope.kind, ArtifactKind::Manifest);
        let document = envelope.payload.expect(filename);
        assert_eq!(&document.format, expected, "{filename}");
        assert_eq!(document.raw, *text);
        assert!(
            !document.dependencies.is_empty()
                || !document.references.is_empty()
                || !document.instructions.is_empty(),
            "{filename} produced no typed dependency, reference, or inert instruction"
        );
        assert!(
            !document.active_content_execution
                && !document.template_evaluation
                && !document.network_access
        );
        document.locator.validate().unwrap();
        for dependency in &document.dependencies {
            dependency.provenance.locator.validate().unwrap();
            assert_eq!(
                &text[dependency.provenance.range.byte_start..dependency.provenance.range.byte_end],
                dependency.provenance.declaration
            );
        }
        for reference in &document.references {
            reference.provenance.locator.validate().unwrap();
            assert_eq!(
                &text[reference.provenance.range.byte_start..reference.provenance.range.byte_end],
                reference.provenance.declaration
            );
        }
        for instruction in &document.instructions {
            assert!(!instruction.executable);
            instruction.locator.validate().unwrap();
            assert_eq!(
                &text[instruction.range.byte_start..instruction.range.byte_end],
                instruction.raw
            );
        }
        let graph = document
            .to_document_graph(DocumentGraphContext::new(format!("manifest:{filename}")))
            .unwrap();
        graph.validate_contract().unwrap();
        assert!(graph.edges.iter().all(|edge| matches!(
            edge.relation,
            DocumentRelation::Contains | DocumentRelation::Requires | DocumentRelation::References
        )));
        let wire = serde_json::to_value(&document).unwrap();
        assert_eq!(
            serde_json::from_value::<grist::manifests::ManifestDocument>(wire).unwrap(),
            document
        );
    }
}

const MALFORMED: &[(&str, &str)] = &[
    ("Cargo.toml", "[dependencies\nserde = \"1\""),
    ("Cargo.lock", "[[package]\nname = \"serde\""),
    ("package.json", "{\"dependencies\": {"),
    ("package-lock.json", "{\"packages\": [}"),
    ("pnpm-lock.yaml", "importers:\n  .: [}"),
    ("yarn.lock", "left-pad@^1:\n  resolved nowhere\n"),
    ("pyproject.toml", "[project\ndependencies = ["),
    ("requirements.txt", "??? invalid requirement\n"),
    ("setup.py", "setup(install_requires=['requests'\n"),
    ("poetry.lock", "[[package]\nname ="),
    (
        "pom.xml",
        "<project><dependency><artifactId>broken</dependency>",
    ),
    ("build.gradle", "implementation(\"broken:coordinate:1)\n"),
    ("settings.gradle.kts", "include(\"unterminated)\n"),
    ("go.mod", "go 1.24\nrequire\n"),
    ("go.sum", "not-enough-columns\n"),
    ("Dockerfile", r#"FROM scratch \"#),
    ("compose.yaml", "services:\n  web: [}"),
    (".gitlab-ci.yml", "test:\n  script: [}"),
    ("deployment.yaml", "apiVersion: v1\nkind: [}"),
];

#[test]
fn malformed_families_return_partial_payloads_with_exact_recovery_locations() {
    for (filename, text) in MALFORMED {
        let envelope = parse_manifest(
            text,
            SourceInfo::stdin(*filename),
            &ManifestOptions::default(),
        );
        assert_eq!(
            envelope.status,
            OperationStatus::Partial,
            "{filename}: {envelope:#?}"
        );
        assert!(!envelope.diagnostics.is_empty(), "{filename}");
        let document = envelope.payload.expect(filename);
        assert_eq!(document.raw, *text);
        assert!(!document.parse_errors.is_empty(), "{filename}");
        for error in &document.parse_errors {
            error.locator.validate().unwrap();
            assert!(error.range.byte_end <= text.len());
        }
        assert!(
            !document.active_content_execution
                && !document.template_evaluation
                && !document.network_access
        );
    }
}

#[test]
fn detector_registry_graph_and_schema_surfaces_select_typed_manifests() {
    let registry = builtin_parser_registry().unwrap();
    match registry.select_format("manifest") {
        ParserSelection::Available(descriptor) => {
            assert_eq!(descriptor.id, "grist.manifests");
            assert_eq!(descriptor.format.artifact_kind, ArtifactKind::Manifest);
            assert_eq!(descriptor.payload_schema.version, "grist/manifest/v1");
        }
        other => panic!("manifest parser is not available: {other:?}"),
    }
    for (path, bytes) in [
        ("Cargo.toml", b"[package]\nname='demo'\n".as_slice()),
        (".github/workflows/ci.yml", b"jobs: {}\n".as_slice()),
        (
            "deployment.yaml",
            b"apiVersion: v1\nkind: Service\n".as_slice(),
        ),
    ] {
        let detection = detect_with_registry(
            Path::new(path),
            bytes,
            None,
            None,
            &Limits::default(),
            &registry,
            &DetectionOptions::default(),
        )
        .unwrap();
        assert_eq!(
            detection.status,
            DetectionStatus::Selected,
            "{path}: {detection:#?}"
        );
        assert_eq!(detection.content_kind, ContentKind::Manifest, "{path}");
        assert_eq!(
            detection.candidates[0].identity.format, "manifest",
            "{path}"
        );
    }
    #[cfg(feature = "schemas")]
    {
        let schema = grist::schema::schema_json("manifest").expect("manifest schema");
        assert_eq!(schema["title"], "ManifestDocument");
        assert!(grist::schema::schema_json("manifest-options").is_some());
    }
}

#[test]
fn unknown_fields_are_retained_for_forward_compatible_consumers() {
    let text = "[package]\nname = \"demo\"\nx-future-format = { enabled = true }\n";
    let document = parse_manifest(
        text,
        SourceInfo::stdin("Cargo.toml"),
        &ManifestOptions::default(),
    )
    .payload
    .expect("payload");
    let unknown = document
        .unknown_fields
        .iter()
        .find(|field| field.raw.contains("x-future-format"))
        .expect("future field retained");
    assert_eq!(
        &text[unknown.range.byte_start..unknown.range.byte_end],
        unknown.raw
    );
    unknown.locator.validate().unwrap();
}

#[test]
fn toml_dependency_subtables_retain_the_declared_package_name() {
    let text = "[package]\nname = \"demo\"\n[dependencies.serde]\nversion = \"1\"\n[dependencies.local]\npath = \"../local\"\n";
    let envelope = parse_manifest(
        text,
        SourceInfo::stdin("Cargo.toml"),
        &ManifestOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document = envelope.payload.expect("payload");
    let serde = document
        .dependencies
        .iter()
        .find(|dependency| dependency.name == "serde")
        .expect("serde dependency");
    assert_eq!(serde.requirement.as_deref(), Some("1"));
    assert_eq!(serde.provenance.declaration, "[dependencies.serde]");
    assert!(
        document
            .dependencies
            .iter()
            .any(|dependency| dependency.name == "local")
    );
    assert!(
        document
            .references
            .iter()
            .any(|reference| reference.target == "../local")
    );
    assert!(
        !document
            .dependencies
            .iter()
            .any(|dependency| dependency.name == "version")
    );
}

#[test]
fn repeated_and_escaped_dependency_names_have_exact_distinct_provenance() {
    let text = "{\n  \"dependencies\": {\n    \"plain\": \"1\",\n    \"\\u0065scaped\": \"2\"\n  },\n  \"optionalDependencies\": {\n    \"plain\": \"3\"\n  }\n}\n";
    let envelope = parse_manifest(
        text,
        SourceInfo::stdin("package.json"),
        &ManifestOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Complete);
    let document = envelope.payload.expect("payload");
    let plain = document
        .dependencies
        .iter()
        .filter(|dependency| dependency.name == "plain")
        .collect::<Vec<_>>();
    assert_eq!(plain.len(), 2);
    assert_eq!(plain[0].provenance.declaration.trim(), "\"plain\": \"1\",");
    assert_eq!(plain[1].provenance.declaration.trim(), "\"plain\": \"3\"");
    assert_ne!(
        plain[0].provenance.range.byte_start,
        plain[1].provenance.range.byte_start
    );
    let escaped = document
        .dependencies
        .iter()
        .find(|dependency| dependency.name == "escaped")
        .expect("escaped dependency");
    assert!(escaped.provenance.declaration.contains("\\u0065scaped"));
    assert!(escaped.provenance.range.byte_start > 0);
}

#[cfg(feature = "cli")]
#[test]
fn cli_manifest_parse_round_trips_through_document_graph_projection() {
    let envelope = grist::cli::parse_bytes(
        "manifest",
        b"[dependencies]\nserde = \"1\"\n".to_vec(),
        SourceInfo::stdin("Cargo.toml"),
        grist::core::RequestId::new("cli-manifest-round-trip").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    assert_eq!(envelope.kind, ArtifactKind::Manifest);
    let graph = grist::cli::project_envelope_to_graph(&envelope, "manifest:cli").unwrap();
    graph.validate_contract().unwrap();
    assert!(!graph.nodes.is_empty());
}
