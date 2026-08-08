//! Deterministic, evidence-retaining content and parser detection.

mod grammar;
mod packages;
mod signatures;
mod text;

use crate::core::{
    ContentIdentity, DetectionCandidate, DetectionEvidence, DetectionEvidenceKind, Diagnostic,
    FormatHint, FormatIdentity, Limits, ParserAvailability, SourceInfo,
};
use crate::registry::{ParserRegistry, ParserSelection, builtin_parser_registry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Source,
    Test,
    Manifest,
    Lockfile,
    Generated,
    Binary,
    Documentation,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Markdown,
    RestructuredText,
    AsciiDoc,
    Html,
    Xml,
    Rust,
    Python,
    JavaScript,
    #[serde(rename = "typescript")]
    TypeScript,
    Tsx,
    Jsx,
    Shell,
    Latex,
    Bibliography,
    Csv,
    Json,
    Jsonl,
    Yaml,
    Toml,
    Cbor,
    MessagePack,
    Protobuf,
    Arrow,
    Text,
    Pdf,
    Zip,
    Docx,
    Pptx,
    Xlsx,
    Xlsm,
    Epub,
    Odt,
    Odp,
    Ods,
    Rtf,
    Eml,
    Mbox,
    Msg,
    ICalendar,
    VCard,
    Sqlite,
    Png,
    Jpeg,
    Gif,
    Tiff,
    Webp,
    Bmp,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    SevenZip,
    Parquet,
    OleCompound,
    Binary,
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DetectionStatus {
    Selected,
    Ambiguous,
    Unsupported,
    #[default]
    Unknown,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityPolicy {
    #[default]
    ReturnAmbiguous,
    PreferAvailable,
    SelectHighestRanked,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectionOptions {
    pub minimum_confidence: f32,
    pub ambiguity_margin: f32,
    pub ambiguity_policy: AmbiguityPolicy,
    pub max_probe_bytes: usize,
}

impl Default for DetectionOptions {
    fn default() -> Self {
        Self {
            minimum_confidence: 0.40,
            ambiguity_margin: 0.08,
            ambiguity_policy: AmbiguityPolicy::ReturnAmbiguous,
            max_probe_bytes: 256 * 1024,
        }
    }
}

impl DetectionOptions {
    pub fn validate(&self) -> Result<(), DetectionOptionsError> {
        if !self.minimum_confidence.is_finite() || !(0.0..=1.0).contains(&self.minimum_confidence) {
            return Err(DetectionOptionsError::InvalidMinimumConfidence);
        }
        if !self.ambiguity_margin.is_finite() || !(0.0..=1.0).contains(&self.ambiguity_margin) {
            return Err(DetectionOptionsError::InvalidAmbiguityMargin);
        }
        if self.max_probe_bytes == 0 {
            return Err(DetectionOptionsError::ZeroProbeBudget);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DetectionOptionsError {
    #[error("minimum_confidence must be finite and between zero and one")]
    InvalidMinimumConfidence,
    #[error("ambiguity_margin must be finite and between zero and one")]
    InvalidAmbiguityMargin,
    #[error("max_probe_bytes must be greater than zero")]
    ZeroProbeBudget,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Detection {
    pub file_kind: FileKind,
    pub content_kind: ContentKind,
    pub language: Option<String>,
    pub confidence: f32,
    pub reasons: Vec<String>,
    #[serde(default)]
    pub candidates: Vec<DetectionCandidate>,
    #[serde(default)]
    pub status: DetectionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_parser: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Detection {
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            file_kind: FileKind::Unknown,
            content_kind: ContentKind::Unknown,
            language: None,
            confidence: 0.0,
            reasons: vec![reason.into()],
            candidates: Vec::new(),
            status: DetectionStatus::Unknown,
            selected_parser: None,
            diagnostics: Vec::new(),
        }
    }

    pub fn apply_to_identity(&self, identity: ContentIdentity) -> ContentIdentity {
        let identity = match self.selected_format_identity() {
            Some(format) => identity.with_format(format),
            None => identity,
        };
        identity.with_detection_candidates(self.candidates.clone())
    }

    pub fn selected_format_identity(&self) -> Option<FormatIdentity> {
        if self.status == DetectionStatus::Ambiguous {
            return None;
        }
        let format = format_for_kind(&self.content_kind)?;
        self.candidates
            .iter()
            .find(|candidate| candidate.identity.format == format)
            .map(|candidate| candidate.identity.clone())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Signal {
    pub identity: FormatIdentity,
    pub weight: f32,
    pub kind: DetectionEvidenceKind,
    pub description: String,
    pub decisive: bool,
}

impl Signal {
    pub fn new(
        format: &str,
        media_type: Option<&str>,
        weight: f32,
        kind: DetectionEvidenceKind,
        description: impl Into<String>,
    ) -> Self {
        Self {
            identity: FormatIdentity::new(format, media_type),
            weight,
            kind,
            description: description.into(),
            decisive: false,
        }
    }

    pub fn decisive(mut self) -> Self {
        self.decisive = true;
        self
    }
}

struct CandidateAccumulator {
    identity: FormatIdentity,
    residual: f32,
    evidence: BTreeSet<DetectionEvidence>,
    decisive: bool,
}

impl CandidateAccumulator {
    fn new(identity: FormatIdentity) -> Self {
        Self {
            identity,
            residual: 1.0,
            evidence: BTreeSet::new(),
            decisive: false,
        }
    }

    fn add(&mut self, signal: Signal) {
        if self.identity.media_type.is_none() {
            self.identity.media_type = signal.identity.media_type;
        }
        let evidence = DetectionEvidence::new(signal.kind, signal.description);
        if self.evidence.insert(evidence) {
            self.residual *= 1.0 - signal.weight.clamp(0.0, 1.0);
            self.decisive |= signal.decisive;
        }
    }

    fn confidence(&self) -> f32 {
        1.0 - self.residual
    }
}

pub fn detect_path(path: &Path, bytes: &[u8], limits: &Limits) -> Detection {
    let registry = builtin_parser_registry()
        .expect("built-in parser registry metadata must remain conflict-free");
    detect_with_registry(
        path,
        bytes,
        None,
        None,
        limits,
        &registry,
        &DetectionOptions::default(),
    )
    .expect("default detection options are valid")
}

pub fn detect_source(
    source: &SourceInfo,
    bytes: &[u8],
    format_hint: Option<&FormatHint>,
    limits: &Limits,
    registry: &ParserRegistry,
    options: &DetectionOptions,
) -> Result<Detection, DetectionOptionsError> {
    let path = source
        .path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&source.display_name));
    detect_with_registry(
        &path,
        bytes,
        source.declared_mime_type.as_deref(),
        format_hint,
        limits,
        registry,
        options,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn detect_with_registry(
    path: &Path,
    bytes: &[u8],
    declared_media_type: Option<&str>,
    format_hint: Option<&FormatHint>,
    limits: &Limits,
    registry: &ParserRegistry,
    options: &DetectionOptions,
) -> Result<Detection, DetectionOptionsError> {
    options.validate()?;
    let filename = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut diagnostics = Vec::new();
    if bytes.len() > limits.max_file_bytes {
        diagnostics.push(
            Diagnostic::error(
                "grist.detect",
                "limit.max_file_bytes",
                format!(
                    "file has {} bytes, exceeding max_file_bytes {}",
                    bytes.len(),
                    limits.max_file_bytes
                ),
            )
            .partial(),
        );
    }

    let mut signals = signatures::signals(bytes, &mut diagnostics);
    if signatures::is_zip(bytes) {
        signals.extend(packages::zip_signals(bytes, &mut diagnostics));
    }
    add_label_signals(
        &mut signals,
        &filename,
        &extension,
        declared_media_type,
        format_hint,
        registry,
    );
    signals.extend(text::signals(
        bytes,
        options.max_probe_bytes,
        &mut diagnostics,
    ));
    if let Some(sample) = text::sample(bytes, options.max_probe_bytes) {
        signals.extend(grammar::signals(&sample.text));
    }
    #[cfg(feature = "structured-binary")]
    #[cfg(feature = "columnar")]
    for format in crate::columnar::probe(bytes) {
        let (name, media, description) = match format {
            crate::columnar::ColumnarFormat::ArrowIpcFile => (
                "arrow",
                "application/vnd.apache.arrow.file",
                "valid Arrow IPC file framing",
            ),
            crate::columnar::ColumnarFormat::ArrowIpcStream => (
                "arrow",
                "application/vnd.apache.arrow.stream",
                "valid Arrow IPC stream framing",
            ),
            crate::columnar::ColumnarFormat::Parquet => (
                "parquet",
                "application/vnd.apache.parquet",
                "valid Parquet boundary framing",
            ),
        };
        signals.push(Signal::new(
            name,
            Some(media),
            0.94,
            DetectionEvidenceKind::Structure,
            description,
        ));
    }
    #[cfg(feature = "structured-binary")]
    for format in crate::structured_binary::probe_formats(bytes) {
        let (name, media, description, weight) = match format {
            crate::structured_binary::StructuredBinaryFormat::Cbor => (
                "cbor",
                "application/cbor",
                "complete CBOR structural probe",
                0.82,
            ),
            crate::structured_binary::StructuredBinaryFormat::MessagePack => (
                "messagepack",
                "application/msgpack",
                "complete MessagePack structural probe",
                0.82,
            ),
            crate::structured_binary::StructuredBinaryFormat::Protobuf => (
                "protobuf",
                "application/x-protobuf",
                "valid Protocol Buffers wire-structure probe; descriptor still required",
                0.82,
            ),
        };
        signals.push(Signal::new(
            name,
            Some(media),
            weight,
            DetectionEvidenceKind::Structure,
            description,
        ));
    }
    if signals.is_empty() {
        signals.push(Signal::new(
            "binary",
            Some("application/octet-stream"),
            0.45,
            DetectionEvidenceKind::Fallback,
            "no safe text or known signature probe matched",
        ));
    }

    let mut accumulators = BTreeMap::<String, CandidateAccumulator>::new();
    for signal in signals {
        accumulators
            .entry(signal.identity.format.clone())
            .or_insert_with(|| CandidateAccumulator::new(signal.identity.clone()))
            .add(signal);
    }
    let has_decisive = accumulators.values().any(|candidate| candidate.decisive);
    let mut candidates = accumulators
        .into_values()
        .map(|candidate| {
            let confidence = if has_decisive && !candidate.decisive {
                candidate.confidence().min(0.84)
            } else {
                candidate.confidence()
            };
            DetectionCandidate::new(
                0,
                candidate.identity,
                round_confidence(confidence),
                candidate.evidence.into_iter().collect(),
            )
        })
        .filter(|candidate| candidate.confidence >= 0.10)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| left.identity.cmp(&right.identity))
            .then_with(|| left.evidence.cmp(&right.evidence))
    });
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
        enrich_availability(candidate, registry);
    }

    add_contradiction_diagnostic(&candidates, &mut diagnostics);
    let (status, selected_index, selected_parser) = decide(&candidates, options, &mut diagnostics);
    let selected = selected_index.and_then(|index| candidates.get(index));
    let content_kind = selected
        .map(|candidate| content_kind_for_format(&candidate.identity.format))
        .unwrap_or(ContentKind::Unknown);
    let language = selected.and_then(|candidate| language_for_format(&candidate.identity.format));
    let confidence = candidates
        .first()
        .map_or(0.0, |candidate| candidate.confidence);
    let reasons = selected
        .or_else(|| candidates.first())
        .map(|candidate| {
            candidate
                .evidence
                .iter()
                .map(|evidence| evidence.description.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut file_kind = classify_file_kind(&filename, &extension, path);
    if is_binary_content(&content_kind) {
        file_kind = FileKind::Binary;
    }
    Ok(Detection {
        file_kind,
        content_kind,
        language,
        confidence,
        reasons,
        candidates,
        status,
        selected_parser,
        diagnostics,
    })
}

fn round_confidence(value: f32) -> f32 {
    (value * 10_000.0).round() / 10_000.0
}

fn enrich_availability(candidate: &mut DetectionCandidate, registry: &ParserRegistry) {
    match registry.select_format(&candidate.identity.format) {
        ParserSelection::Available(descriptor) => {
            candidate.parser_availability = ParserAvailability::Available;
            candidate.parser_id = Some(descriptor.id.clone());
            if candidate.identity.media_type.is_none() {
                candidate.identity.media_type = descriptor.format.media_types.first().cloned();
            }
        }
        ParserSelection::Unsupported { unavailable, .. } if !unavailable.is_empty() => {
            candidate.parser_availability = ParserAvailability::Unavailable;
            candidate.parser_id = unavailable.first().map(|item| item.descriptor.id.clone());
        }
        ParserSelection::Unsupported { .. } => {
            candidate.parser_availability = ParserAvailability::Unregistered;
        }
    }
}

fn decide(
    candidates: &[DetectionCandidate],
    options: &DetectionOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> (DetectionStatus, Option<usize>, Option<String>) {
    let Some(first) = candidates.first() else {
        return (DetectionStatus::Unknown, None, None);
    };
    if first.confidence < options.minimum_confidence {
        diagnostics.push(Diagnostic::warning(
            "grist.detect",
            "detect.confidence_below_minimum",
            format!(
                "highest confidence {:.4} is below configured minimum {:.4}",
                first.confidence, options.minimum_confidence
            ),
        ));
        return (DetectionStatus::Unknown, None, None);
    }
    let ambiguous_end = candidates
        .iter()
        .take_while(|candidate| {
            candidate.confidence >= options.minimum_confidence
                && first.confidence - candidate.confidence <= options.ambiguity_margin
        })
        .count();
    let selected_index = if ambiguous_end <= 1 {
        Some(0)
    } else {
        match options.ambiguity_policy {
            AmbiguityPolicy::ReturnAmbiguous => None,
            AmbiguityPolicy::SelectHighestRanked => Some(0),
            AmbiguityPolicy::PreferAvailable => {
                let available = candidates[..ambiguous_end]
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        candidate.parser_availability == ParserAvailability::Available
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                (available.len() == 1).then_some(available[0])
            }
        }
    };
    let Some(selected_index) = selected_index else {
        diagnostics.push(
            Diagnostic::warning(
                "grist.detect",
                "detect.ambiguous",
                format!(
                    "{} candidates fall within ambiguity margin {:.4}",
                    ambiguous_end, options.ambiguity_margin
                ),
            )
            .partial(),
        );
        return (DetectionStatus::Ambiguous, None, None);
    };
    let selected = &candidates[selected_index];
    match selected.parser_availability {
        ParserAvailability::Available => (
            DetectionStatus::Selected,
            Some(selected_index),
            selected.parser_id.clone(),
        ),
        ParserAvailability::Unavailable | ParserAvailability::Unregistered => {
            diagnostics.push(
                Diagnostic::unsupported(
                    "grist.detect",
                    format!(
                        "format {} is recognized but no parser is available",
                        selected.identity.format
                    ),
                )
                .partial(),
            );
            (DetectionStatus::Unsupported, Some(selected_index), None)
        }
    }
}

fn add_contradiction_diagnostic(
    candidates: &[DetectionCandidate],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(first) = candidates.first() else {
        return;
    };
    let first_decisive = first.evidence.iter().any(|evidence| {
        matches!(
            evidence.kind,
            DetectionEvidenceKind::MagicBytes | DetectionEvidenceKind::ContainerManifest
        )
    });
    let contradictory_label = candidates.iter().skip(1).any(|candidate| {
        candidate.evidence.iter().any(|evidence| {
            matches!(
                evidence.kind,
                DetectionEvidenceKind::Extension
                    | DetectionEvidenceKind::Filename
                    | DetectionEvidenceKind::DeclaredMediaType
            )
        })
    });
    if first_decisive && contradictory_label {
        diagnostics.push(Diagnostic::warning(
            "grist.detect",
            "detect.contradictory_evidence",
            format!(
                "content signature identifies {}; contradictory labels were retained but not selected",
                first.identity.format
            ),
        ));
    }
}

fn add_label_signals(
    signals: &mut Vec<Signal>,
    filename: &str,
    extension: &str,
    declared_media_type: Option<&str>,
    format_hint: Option<&FormatHint>,
    registry: &ParserRegistry,
) {
    add_special_filename_signal(signals, filename);
    add_extension_signal(signals, extension);
    let descriptors = registry_descriptors(registry);
    for descriptor in &descriptors {
        if !extension.is_empty()
            && extension_identity(extension).is_none()
            && descriptor
                .format
                .extensions
                .iter()
                .any(|value| normalize_extension(value) == extension)
        {
            signals.push(Signal::new(
                &descriptor.format.id,
                descriptor.format.media_types.first().map(String::as_str),
                0.55,
                DetectionEvidenceKind::Extension,
                format!(
                    ".{extension} extension registered for {}",
                    descriptor.format.id
                ),
            ));
        }
    }
    let declared =
        declared_media_type.or_else(|| format_hint.and_then(|hint| hint.media_type.as_deref()));
    if let Some(media_type) = declared {
        add_media_type_signal(signals, media_type);
        let normalized = normalize_media_type(media_type);
        for descriptor in &descriptors {
            if descriptor
                .format
                .media_types
                .iter()
                .filter(|_| media_type_identity(&normalized).is_none())
                .any(|value| normalize_media_type(value) == normalized)
            {
                signals.push(Signal::new(
                    &descriptor.format.id,
                    Some(&normalized),
                    0.72,
                    DetectionEvidenceKind::DeclaredMediaType,
                    format!("caller declared media type {normalized}"),
                ));
            }
        }
    }
    if let Some(hint) = format_hint {
        add_filename_hint(signals, hint.filename.as_deref());
        add_format_hint(signals, hint.format.as_deref(), registry);
    }
}

fn add_filename_hint(signals: &mut Vec<Signal>, filename: Option<&str>) {
    let Some(filename) = filename else {
        return;
    };
    let path = Path::new(filename);
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    add_special_filename_signal(signals, &name);
    add_extension_signal(signals, &extension);
}

fn add_format_hint(signals: &mut Vec<Signal>, format: Option<&str>, registry: &ParserRegistry) {
    let Some(format) = format else {
        return;
    };
    let (canonical, media) = match registry.select_format(format) {
        ParserSelection::Available(descriptor) => (
            descriptor.format.id.clone(),
            descriptor.format.media_types.first().cloned(),
        ),
        ParserSelection::Unsupported { unavailable, .. } if !unavailable.is_empty() => (
            unavailable[0].descriptor.format.id.clone(),
            unavailable[0]
                .descriptor
                .format
                .media_types
                .first()
                .cloned(),
        ),
        ParserSelection::Unsupported { .. } => (normalize_format(format), None),
    };
    signals.push(Signal::new(
        &canonical,
        media.as_deref(),
        0.78,
        DetectionEvidenceKind::Fallback,
        format!("caller supplied format hint {format}"),
    ));
}

fn registry_descriptors(registry: &ParserRegistry) -> Vec<crate::registry::ParserDescriptor> {
    let mut descriptors = registry.parsers();
    descriptors.extend(
        registry
            .unavailable_parsers()
            .into_iter()
            .map(|item| item.descriptor),
    );
    descriptors.sort_by(|left, right| left.id.cmp(&right.id));
    descriptors
}

fn add_special_filename_signal(signals: &mut Vec<Signal>, filename: &str) {
    let (format, media_type, label) = match filename {
        "cargo.toml" | "pyproject.toml" | "pipfile" => {
            ("toml", "application/toml", "package manifest")
        }
        "cargo.lock" | "poetry.lock" => ("toml", "application/toml", "package lockfile"),
        "package.json" | "tsconfig.json" | "composer.json" => {
            ("json", "application/json", "package manifest")
        }
        "package-lock.json" | "npm-shrinkwrap.json" | "pipfile.lock" => {
            ("json", "application/json", "package lockfile")
        }
        "pnpm-lock.yaml" | "pnpm-lock.yml" => ("yaml", "application/yaml", "package lockfile"),
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => {
            ("yaml", "application/yaml", "deployment manifest")
        }
        "pom.xml" => ("xml", "application/xml", "package manifest"),
        "dockerfile"
        | "containerfile"
        | "go.mod"
        | "go.sum"
        | "yarn.lock"
        | "build.gradle"
        | "build.gradle.kts"
        | "settings.gradle"
        | "settings.gradle.kts"
        | ".gitignore"
        | ".ignore"
        | ".gitattributes"
        | ".gitmodules" => ("text", "text/plain", "repository manifest or lockfile"),
        "readme" => ("markdown", "text/markdown", "README special filename"),
        "readme.md" | "readme.markdown" => {
            ("markdown", "text/markdown", "README markdown filename")
        }
        _ => return,
    };
    signals.push(Signal::new(
        format,
        Some(media_type),
        0.68,
        DetectionEvidenceKind::Filename,
        label,
    ));
}

fn add_extension_signal(signals: &mut Vec<Signal>, extension: &str) {
    let Some((format, media_type)) = extension_identity(extension) else {
        return;
    };
    signals.push(Signal::new(
        format,
        Some(media_type),
        0.55,
        DetectionEvidenceKind::Extension,
        format!(".{extension} extension"),
    ));
}

fn add_media_type_signal(signals: &mut Vec<Signal>, media_type: &str) {
    let normalized = normalize_media_type(media_type);
    let Some((format, canonical_media_type)) = media_type_identity(&normalized) else {
        return;
    };
    signals.push(Signal::new(
        format,
        Some(canonical_media_type),
        0.72,
        DetectionEvidenceKind::DeclaredMediaType,
        format!("caller declared media type {normalized}"),
    ));
    if let Some(charset) = declared_charset(media_type) {
        signals.push(Signal::new(
            format,
            Some(canonical_media_type),
            0.18,
            DetectionEvidenceKind::Charset,
            format!("caller declared charset {charset}"),
        ));
    }
}

fn declared_charset(value: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches(['\'', '"']).to_ascii_lowercase())
    })
}

fn normalize_media_type(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase()
}

fn normalize_extension(value: &str) -> String {
    value.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn normalize_format(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn extension_identity(extension: &str) -> Option<(&'static str, &'static str)> {
    Some(match extension {
        "rs" => ("rust", "text/x-rust"),
        "py" | "pyi" => ("python", "text/x-python"),
        "js" | "mjs" | "cjs" => ("javascript", "text/javascript"),
        "ts" | "mts" | "cts" => ("typescript", "text/typescript"),
        "tsx" => ("tsx", "text/tsx"),
        "jsx" => ("jsx", "text/jsx"),
        "sh" | "bash" => ("shell", "text/x-shellscript"),
        "md" | "markdown" => ("markdown", "text/markdown"),
        "rst" | "rest" => ("restructured-text", "text/x-rst"),
        "adoc" | "asciidoc" | "asc" => ("asciidoc", "text/asciidoc"),
        "tex" | "latex" => ("latex", "application/x-latex"),
        "bib" => ("bibtex", "application/x-bibtex"),
        "html" | "htm" => ("html", "text/html"),
        "xhtml" | "xml" | "jats" | "nxml" => ("xml", "application/xml"),
        "csv" => ("csv", "text/csv"),
        "tsv" => ("tsv", "text/tab-separated-values"),
        "json" => ("json", "application/json"),
        "jsonl" | "ndjson" => ("jsonl", "application/x-ndjson"),
        "yaml" | "yml" => ("yaml", "application/yaml"),
        "toml" => ("toml", "application/toml"),
        "cbor" => ("cbor", "application/cbor"),
        "msgpack" | "mpk" => ("messagepack", "application/msgpack"),
        "pb" | "protobuf" => ("protobuf", "application/x-protobuf"),
        "txt" => ("text", "text/plain"),
        "pdf" => ("pdf", "application/pdf"),
        "zip" => ("zip", "application/zip"),
        "docx" => (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "docm" => ("docm", "application/vnd.ms-word.document.macroEnabled.12"),
        "dotx" => (
            "dotx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
        ),
        "dotm" => ("dotm", "application/vnd.ms-word.template.macroEnabled.12"),
        "pptx" => (
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        "pptm" => (
            "pptm",
            "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
        ),
        "potx" => (
            "potx",
            "application/vnd.openxmlformats-officedocument.presentationml.template",
        ),
        "ppsx" => (
            "ppsx",
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
        ),
        "xlsx" | "xlsm" => (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        "epub" => ("epub", "application/epub+zip"),
        "odt" => ("odt", "application/vnd.oasis.opendocument.text"),
        "ott" => ("ott", "application/vnd.oasis.opendocument.text-template"),
        "odp" => ("odp", "application/vnd.oasis.opendocument.presentation"),
        "otp" => (
            "otp",
            "application/vnd.oasis.opendocument.presentation-template",
        ),
        "ods" => ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
        "ots" => (
            "ots",
            "application/vnd.oasis.opendocument.spreadsheet-template",
        ),
        "rtf" => ("rtf", "application/rtf"),
        "eml" => ("eml", "message/rfc822"),
        "mbox" => ("mbox", "application/mbox"),
        "ics" | "ifb" => ("icalendar", "text/calendar"),
        "vcf" | "vcard" => ("vcard", "text/vcard"),
        "sqlite" | "sqlite3" | "db" => ("sqlite", "application/vnd.sqlite3"),
        "png" => ("png", "image/png"),
        "jpg" | "jpeg" => ("jpeg", "image/jpeg"),
        "gif" => ("gif", "image/gif"),
        "tif" | "tiff" => ("tiff", "image/tiff"),
        "webp" => ("webp", "image/webp"),
        "bmp" => ("bmp", "image/bmp"),
        "gz" | "gzip" => ("gzip", "application/gzip"),
        "bz2" => ("bzip2", "application/x-bzip2"),
        "xz" => ("xz", "application/x-xz"),
        "zst" | "zstd" => ("zstd", "application/zstd"),
        "7z" => ("seven_zip", "application/x-7z-compressed"),
        "parquet" => ("parquet", "application/vnd.apache.parquet"),
        _ => return None,
    })
}

fn media_type_identity(media_type: &str) -> Option<(&'static str, &'static str)> {
    Some(match media_type {
        "text/plain" => ("text", "text/plain"),
        "text/markdown" => ("markdown", "text/markdown"),
        "text/x-rst" | "text/restructuredtext" => ("restructured-text", "text/x-rst"),
        "text/asciidoc" | "text/x-asciidoc" => ("asciidoc", "text/asciidoc"),
        "text/html" => ("html", "text/html"),
        "application/xhtml+xml" | "application/xml" | "text/xml" | "application/jats+xml" => {
            ("xml", "application/xml")
        }
        "text/csv" => ("csv", "text/csv"),
        "text/tab-separated-values" => ("tsv", "text/tab-separated-values"),
        "application/json" | "text/json" => ("json", "application/json"),
        "application/x-ndjson" | "application/jsonl" => ("jsonl", "application/x-ndjson"),
        "application/yaml" | "text/yaml" | "application/x-yaml" => ("yaml", "application/yaml"),
        "application/toml" => ("toml", "application/toml"),
        "application/cbor" => ("cbor", "application/cbor"),
        "application/msgpack" | "application/x-msgpack" => ("messagepack", "application/msgpack"),
        "application/x-protobuf" | "application/protobuf" => ("protobuf", "application/x-protobuf"),
        "text/x-rust" => ("rust", "text/x-rust"),
        "text/x-python" | "application/x-python-code" => ("python", "text/x-python"),
        "text/javascript" | "application/javascript" => ("javascript", "text/javascript"),
        "text/typescript" | "application/typescript" => ("typescript", "text/typescript"),
        "application/x-latex" | "text/x-tex" => ("latex", "application/x-latex"),
        "application/x-bibtex" | "text/x-bibtex" => ("bibtex", "application/x-bibtex"),
        "application/pdf" => ("pdf", "application/pdf"),
        "application/zip" => ("zip", "application/zip"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "application/vnd.ms-word.document.macroenabled.12" => {
            ("docm", "application/vnd.ms-word.document.macroEnabled.12")
        }
        "application/vnd.openxmlformats-officedocument.wordprocessingml.template" => (
            "dotx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
        ),
        "application/vnd.ms-word.template.macroenabled.12" => {
            ("dotm", "application/vnd.ms-word.template.macroEnabled.12")
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => (
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        "application/vnd.ms-powerpoint.presentation.macroenabled.12" => (
            "pptm",
            "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
        ),
        "application/vnd.openxmlformats-officedocument.presentationml.template" => (
            "potx",
            "application/vnd.openxmlformats-officedocument.presentationml.template",
        ),
        "application/vnd.openxmlformats-officedocument.presentationml.slideshow" => (
            "ppsx",
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
        ),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        "application/epub+zip" => ("epub", "application/epub+zip"),
        "application/vnd.oasis.opendocument.text" => {
            ("odt", "application/vnd.oasis.opendocument.text")
        }
        "application/vnd.oasis.opendocument.text-template" => {
            ("ott", "application/vnd.oasis.opendocument.text-template")
        }
        "application/vnd.oasis.opendocument.presentation" => {
            ("odp", "application/vnd.oasis.opendocument.presentation")
        }
        "application/vnd.oasis.opendocument.presentation-template" => (
            "otp",
            "application/vnd.oasis.opendocument.presentation-template",
        ),
        "application/vnd.oasis.opendocument.spreadsheet" => {
            ("ods", "application/vnd.oasis.opendocument.spreadsheet")
        }
        "application/vnd.oasis.opendocument.spreadsheet-template" => (
            "ots",
            "application/vnd.oasis.opendocument.spreadsheet-template",
        ),
        "application/rtf" | "text/rtf" => ("rtf", "application/rtf"),
        "message/rfc822" => ("eml", "message/rfc822"),
        "application/mbox" => ("mbox", "application/mbox"),
        "text/calendar" | "application/ics" => ("icalendar", "text/calendar"),
        "text/vcard" | "text/x-vcard" => ("vcard", "text/vcard"),
        "application/vnd.sqlite3" | "application/x-sqlite3" => {
            ("sqlite", "application/vnd.sqlite3")
        }
        "image/png" => ("png", "image/png"),
        "image/jpeg" => ("jpeg", "image/jpeg"),
        "image/gif" => ("gif", "image/gif"),
        "image/tiff" => ("tiff", "image/tiff"),
        "image/webp" => ("webp", "image/webp"),
        "image/bmp" => ("bmp", "image/bmp"),
        "application/gzip" => ("gzip", "application/gzip"),
        "application/x-bzip2" => ("bzip2", "application/x-bzip2"),
        "application/x-xz" => ("xz", "application/x-xz"),
        "application/zstd" => ("zstd", "application/zstd"),
        "application/x-7z-compressed" => ("seven_zip", "application/x-7z-compressed"),
        _ => return None,
    })
}

fn format_for_kind(kind: &ContentKind) -> Option<&'static str> {
    Some(match kind {
        ContentKind::Markdown => "markdown",
        ContentKind::RestructuredText => "restructured-text",
        ContentKind::AsciiDoc => "asciidoc",
        ContentKind::Html => "html",
        ContentKind::Xml => "xml",
        ContentKind::Rust => "rust",
        ContentKind::Python => "python",
        ContentKind::JavaScript => "javascript",
        ContentKind::TypeScript => "typescript",
        ContentKind::Tsx => "tsx",
        ContentKind::Jsx => "jsx",
        ContentKind::Shell => "shell",
        ContentKind::Latex => "latex",
        ContentKind::Bibliography => "bibtex",
        ContentKind::Csv => "csv",
        ContentKind::Json => "json",
        ContentKind::Jsonl => "jsonl",
        ContentKind::Yaml => "yaml",
        ContentKind::Toml => "toml",
        ContentKind::Cbor => "cbor",
        ContentKind::MessagePack => "messagepack",
        ContentKind::Protobuf => "protobuf",
        ContentKind::Arrow => "arrow",
        ContentKind::Text => "text",
        ContentKind::Pdf => "pdf",
        ContentKind::Zip => "zip",
        ContentKind::Docx => "docx",
        ContentKind::Pptx => "pptx",
        ContentKind::Xlsx => "xlsx",
        ContentKind::Xlsm => "xlsm",
        ContentKind::Epub => "epub",
        ContentKind::Odt => "odt",
        ContentKind::Odp => "odp",
        ContentKind::Ods => "ods",
        ContentKind::Rtf => "rtf",
        ContentKind::Eml => "eml",
        ContentKind::Mbox => "mbox",
        ContentKind::Msg => "msg",
        ContentKind::ICalendar => "icalendar",
        ContentKind::VCard => "vcard",
        ContentKind::Sqlite => "sqlite",
        ContentKind::Png => "png",
        ContentKind::Jpeg => "jpeg",
        ContentKind::Gif => "gif",
        ContentKind::Tiff => "tiff",
        ContentKind::Webp => "webp",
        ContentKind::Bmp => "bmp",
        ContentKind::Gzip => "gzip",
        ContentKind::Bzip2 => "bzip2",
        ContentKind::Xz => "xz",
        ContentKind::Zstd => "zstd",
        ContentKind::SevenZip => "seven_zip",
        ContentKind::Parquet => "parquet",
        ContentKind::OleCompound => "ole_compound",
        ContentKind::Binary => "binary",
        ContentKind::Unknown => return None,
    })
}

fn content_kind_for_format(format: &str) -> ContentKind {
    match normalize_format(format).as_str() {
        "markdown" | "md" => ContentKind::Markdown,
        "restructured_text" | "restructuredtext" | "rst" | "rest" => ContentKind::RestructuredText,
        "asciidoc" | "adoc" | "asc" => ContentKind::AsciiDoc,
        "html" => ContentKind::Html,
        "xml" => ContentKind::Xml,
        "rust" => ContentKind::Rust,
        "python" => ContentKind::Python,
        "javascript" | "js" => ContentKind::JavaScript,
        "typescript" | "ts" => ContentKind::TypeScript,
        "tsx" => ContentKind::Tsx,
        "jsx" => ContentKind::Jsx,
        "shell" | "bash" => ContentKind::Shell,
        "latex" | "tex" => ContentKind::Latex,
        "bibtex" | "biblatex" | "bibliography" | "bib" => ContentKind::Bibliography,
        "csv" | "tsv" => ContentKind::Csv,
        "json" => ContentKind::Json,
        "jsonl" | "ndjson" => ContentKind::Jsonl,
        "yaml" => ContentKind::Yaml,
        "toml" => ContentKind::Toml,
        "cbor" => ContentKind::Cbor,
        "messagepack" | "msgpack" | "message_pack" => ContentKind::MessagePack,
        "protobuf" | "protocol_buffers" | "proto_binary" => ContentKind::Protobuf,
        "arrow" | "arrow_ipc" | "feather" => ContentKind::Arrow,
        "text" | "plain_text" => ContentKind::Text,
        "pdf" => ContentKind::Pdf,
        "zip" => ContentKind::Zip,
        "docx" | "docm" | "dotx" | "dotm" => ContentKind::Docx,
        "pptx" | "pptm" | "potx" | "ppsx" => ContentKind::Pptx,
        "xlsx" => ContentKind::Xlsx,
        "xlsm" => ContentKind::Xlsm,
        "epub" => ContentKind::Epub,
        "odt" | "ott" => ContentKind::Odt,
        "odp" | "otp" => ContentKind::Odp,
        "ods" | "ots" => ContentKind::Ods,
        "rtf" => ContentKind::Rtf,
        "eml" | "email" | "rfc5322" | "message_rfc822" => ContentKind::Eml,
        "mbox" | "mailbox" | "application_mbox" => ContentKind::Mbox,
        "msg" | "outlook_msg" => ContentKind::Msg,
        "icalendar" | "ics" | "calendar" | "text_calendar" => ContentKind::ICalendar,
        "vcard" | "vcf" | "contact" | "text_vcard" => ContentKind::VCard,
        "sqlite" => ContentKind::Sqlite,
        "png" => ContentKind::Png,
        "jpeg" | "jpg" => ContentKind::Jpeg,
        "gif" => ContentKind::Gif,
        "tiff" => ContentKind::Tiff,
        "webp" => ContentKind::Webp,
        "bmp" => ContentKind::Bmp,
        "gzip" => ContentKind::Gzip,
        "bzip2" => ContentKind::Bzip2,
        "xz" => ContentKind::Xz,
        "zstd" => ContentKind::Zstd,
        "seven_zip" | "7z" => ContentKind::SevenZip,
        "parquet" => ContentKind::Parquet,
        "ole_compound" => ContentKind::OleCompound,
        "binary" => ContentKind::Binary,
        _ => ContentKind::Unknown,
    }
}

fn language_for_format(format: &str) -> Option<String> {
    Some(
        match normalize_format(format).as_str() {
            "rust" => "rust",
            "python" => "python",
            "javascript" => "javascript",
            "jsx" => "jsx",
            "typescript" | "tsx" => "typescript",
            "html" => "html",
            "xml" => "xml",
            "latex" => "latex",
            "shell" => "shell",
            _ => return None,
        }
        .to_string(),
    )
}

fn classify_file_kind(filename: &str, extension: &str, path: &Path) -> FileKind {
    let path_string = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if is_manifest_filename(filename)
        || path_string.contains("/.github/workflows/")
        || path_string.contains("/.gitlab/ci/")
    {
        return FileKind::Manifest;
    }
    if is_lockfile_filename(filename) {
        return FileKind::Lockfile;
    }
    if matches!(
        extension,
        "md" | "markdown" | "rst" | "rest" | "html" | "htm" | "tex" | "latex" | "pdf"
    ) {
        return FileKind::Documentation;
    }
    if path_string.contains("/target/")
        || path_string.contains("/dist/")
        || path_string.contains("/generated/")
        || filename.contains(".generated.")
    {
        return FileKind::Generated;
    }
    if path_string.contains("/tests/")
        || filename.starts_with("test_")
        || filename.ends_with("_test.rs")
        || filename.contains(".test.")
        || filename.contains(".spec.")
    {
        return FileKind::Test;
    }
    if matches!(
        extension,
        "rs" | "py"
            | "pyi"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "mts"
            | "cts"
            | "jsx"
            | "sh"
            | "bash"
    ) {
        return FileKind::Source;
    }
    FileKind::Unknown
}

fn is_manifest_filename(filename: &str) -> bool {
    matches!(
        filename,
        "cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "tsconfig.json"
            | "composer.json"
            | "pipfile"
            | "setup.py"
            | "setup.cfg"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "go.mod"
            | "dockerfile"
            | "containerfile"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yml"
            | "compose.yaml"
            | ".gitlab-ci.yml"
            | ".gitignore"
            | ".ignore"
            | ".gitattributes"
            | ".gitmodules"
    ) || filename.starts_with("requirements") && filename.ends_with(".txt")
}

fn is_lockfile_filename(filename: &str) -> bool {
    filename.ends_with(".lock")
        || matches!(
            filename,
            "cargo.lock"
                | "package-lock.json"
                | "npm-shrinkwrap.json"
                | "pnpm-lock.yaml"
                | "pnpm-lock.yml"
                | "yarn.lock"
                | "pipfile.lock"
                | "poetry.lock"
                | "go.sum"
        )
}

fn is_binary_content(kind: &ContentKind) -> bool {
    matches!(
        kind,
        ContentKind::Pdf
            | ContentKind::Zip
            | ContentKind::Docx
            | ContentKind::Pptx
            | ContentKind::Xlsx
            | ContentKind::Xlsm
            | ContentKind::Epub
            | ContentKind::Odt
            | ContentKind::Odp
            | ContentKind::Ods
            | ContentKind::Sqlite
            | ContentKind::Cbor
            | ContentKind::MessagePack
            | ContentKind::Protobuf
            | ContentKind::Png
            | ContentKind::Jpeg
            | ContentKind::Gif
            | ContentKind::Tiff
            | ContentKind::Webp
            | ContentKind::Bmp
            | ContentKind::Gzip
            | ContentKind::Bzip2
            | ContentKind::Xz
            | ContentKind::Zstd
            | ContentKind::SevenZip
            | ContentKind::Parquet
            | ContentKind::Msg
            | ContentKind::OleCompound
            | ContentKind::Binary
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_valid() {
        DetectionOptions::default().validate().unwrap();
    }

    #[test]
    fn detects_rust_source() {
        let detection = detect_path(
            Path::new("src/lib.rs"),
            b"pub fn x() {}",
            &Limits::default(),
        );
        assert_eq!(detection.content_kind, ContentKind::Rust);
        assert_eq!(detection.file_kind, FileKind::Source);
        #[cfg(feature = "rust")]
        assert_eq!(detection.status, DetectionStatus::Selected);
        #[cfg(not(feature = "rust"))]
        assert_eq!(detection.status, DetectionStatus::Unsupported);
    }
}
