//! Governed parser fixtures, deterministic builders, and golden-output policy.
//!
//! The checked corpus is described by `fixtures/corpus.v1.json`. Consumers such
//! as the universal parser-promotion harness load it through [`load_corpus`]
//! rather than inferring meaning from directory names.

mod golden;
mod manifest;
mod validation;

pub use golden::{
    ExpectedOutputError, canonical_expected_bytes, canonical_expected_sha256,
    normalize_expected_value,
};

pub use manifest::{
    BuilderKind, CORPUS_MANIFEST_SCHEMA_VERSION, CorpusPolicy, EXPECTED_OUTPUT_POLICY_VERSION,
    ExpectedNormalization, ExpectedOutput, FixtureBuilder, FixtureCase, FixtureClass,
    FixtureCorpusManifest, FixtureFile, FixtureFormat, FixtureHandling, FixtureLicense,
    FixtureOrigin, FixtureProvenance, FixtureStorage, REQUIRED_FIXTURE_CLASSES, Redistribution,
};
pub use validation::{
    CORPUS_VALIDATION_REPORT_SCHEMA_VERSION, CorpusLoadError, CorpusValidationReport,
    CorpusViolation, load_corpus, validate_corpus_at,
};
