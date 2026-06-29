use crate::core::{
    ArtifactKind, Diagnostic, Envelope, Hashes, ParserInfo, SchemaVersion, SourceInfo, SourceRange,
};
use crate::markdown::{MarkdownNodeKind, parse_markdown};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

pub const LDGR_PROJECTION_SCHEMA_VERSION: &str = "grist.ldgr_projection.v1";
const PARSER: &str = "grist.ldgr_projection";

pub type LdgrProjectionEnvelope = Envelope<LdgrProjectionDocument>;

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LdgrProjectionDocument {
    pub schema_version: String,
    pub metadata: LdgrProjectionMetadata,
    pub machine_blocks: Vec<LdgrMachineBlock>,
    pub typed: LdgrDocument,
    pub markdown: Option<MarkdownProjectionTrace>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LdgrProjectionOptions {
    pub strict: bool,
    pub include_markdown_trace: bool,
}

impl Default for LdgrProjectionOptions {
    fn default() -> Self {
        Self {
            strict: false,
            include_markdown_trace: true,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LdgrProjectionValidationOptions {
    pub strict: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LdgrProjectionRenderOptions {
    pub include_title: bool,
    pub contextual_prose: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum LdgrProjectionRenderError {
    #[error("failed to render YAML: {0}")]
    Yaml(String),
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LdgrProjectionMetadata {
    pub ldgr_doc: u64,
    pub kind: LdgrDocumentKind,
    pub id: String,
    pub schema: String,
    pub status: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub parent: Option<LdgrRef>,
    pub depends_on: Vec<LdgrRef>,
    pub produces: Vec<LdgrRef>,
    pub tags: Vec<String>,
    pub extra: Map<String, Value>,
}

impl Default for LdgrProjectionMetadata {
    fn default() -> Self {
        Self {
            ldgr_doc: 0,
            kind: LdgrDocumentKind::Ticket,
            id: String::new(),
            schema: String::new(),
            status: None,
            created: None,
            updated: None,
            parent: None,
            depends_on: Vec::new(),
            produces: Vec::new(),
            tags: Vec::new(),
            extra: Map::new(),
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LdgrDocumentKind {
    Spec,
    Epoch,
    Ticket,
    WorkItem,
    Artifact,
    Decision,
    Claim,
    Validation,
    RunReport,
    Graph,
    TicketIndex,
    BatchState,
}

impl LdgrDocumentKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::Epoch => "epoch",
            Self::Ticket => "ticket",
            Self::WorkItem => "work_item",
            Self::Artifact => "artifact",
            Self::Decision => "decision",
            Self::Claim => "claim",
            Self::Validation => "validation",
            Self::RunReport => "run_report",
            Self::Graph => "graph",
            Self::TicketIndex => "ticket_index",
            Self::BatchState => "batch_state",
        }
    }

    fn required_block_kind(&self) -> Option<&'static str> {
        match self {
            Self::Ticket => Some("contract"),
            Self::TicketIndex => Some("ticket-index"),
            Self::Graph => Some("graph"),
            Self::BatchState => Some("batch-state"),
            Self::Validation => Some("validation"),
            Self::RunReport => Some("run-report"),
            _ => None,
        }
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct LdgrRef {
    pub kind: String,
    pub value: String,
}

impl<'de> Deserialize<'de> for LdgrRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::String(raw) => Self::parse(&raw).map_err(D::Error::custom),
            Value::Object(mut map) => {
                let kind = map
                    .remove("kind")
                    .and_then(|value| value.as_str().map(str::to_string))
                    .ok_or_else(|| D::Error::custom("reference object requires string kind"))?;
                let value = map
                    .remove("value")
                    .and_then(|value| value.as_str().map(str::to_string))
                    .ok_or_else(|| D::Error::custom("reference object requires string value"))?;
                if kind.is_empty() || value.is_empty() {
                    return Err(D::Error::custom(
                        "reference kind and value must be non-empty",
                    ));
                }
                Ok(Self { kind, value })
            }
            _ => Err(D::Error::custom("reference must be a string or object")),
        }
    }
}

impl LdgrRef {
    pub fn parse(input: &str) -> Result<Self, String> {
        let Some((kind, value)) = input.split_once(':') else {
            return Err("reference must contain ':'".into());
        };
        if kind.is_empty() || value.is_empty() {
            return Err("reference kind and value must be non-empty".into());
        }
        Ok(Self {
            kind: kind.to_string(),
            value: value.to_string(),
        })
    }

    fn as_string(&self) -> String {
        format!("{}:{}", self.kind, self.value)
    }
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LdgrMachineBlock {
    pub kind: String,
    pub format: String,
    pub attributes: Vec<String>,
    pub value: Value,
    pub raw: String,
    pub range: Option<SourceRange>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownProjectionTrace {
    pub frontmatter_range: Option<SourceRange>,
    pub machine_block_ranges: Vec<SourceRange>,
    pub original_markdown_sha256: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "document", rename_all = "snake_case")]
pub enum LdgrDocument {
    Spec(Value),
    Epoch(Value),
    Ticket(LdgrTicketDocument),
    WorkItem(Value),
    Artifact(Value),
    Decision(Value),
    Claim(Value),
    Validation(LdgrValidationDocument),
    RunReport(LdgrRunReportDocument),
    Graph(LdgrGraphDocument),
    TicketIndex(LdgrTicketIndexDocument),
    BatchState(LdgrBatchStateDocument),
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct LdgrTicketDocument {
    pub title: String,
    pub description: String,
    pub requirements: Vec<LdgrRequirement>,
    pub constraints: Vec<LdgrConstraint>,
    pub tests: Vec<LdgrTest>,
    pub validation_instructions: Vec<String>,
    pub expected_artifacts: Vec<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrRequirement {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub evidence_required: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrConstraint {
    pub id: String,
    pub text: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrTest {
    pub id: String,
    pub command: Option<String>,
    pub scenario: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct LdgrTicketIndexDocument {
    pub tickets: Vec<LdgrTicketIndexEntry>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LdgrTicketIndexEntry {
    pub id: String,
    pub artifact: LdgrRef,
    pub title: String,
    pub work_item: Option<LdgrRef>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct LdgrGraphDocument {
    pub nodes: Vec<LdgrGraphNode>,
    pub edges: Vec<LdgrGraphEdge>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LdgrGraphNode {
    pub id: String,
    pub artifact: Option<LdgrRef>,
    pub work_item: Option<LdgrRef>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LdgrGraphEdge {
    pub dependency: String,
    pub dependent: String,
    pub kind: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct LdgrBatchStateDocument {
    pub batch_id: String,
    pub graph_artifact_id: LdgrRef,
    pub ticket_index_artifact_id: LdgrRef,
    pub status: String,
    pub current_wave: Option<String>,
    pub waves: Vec<LdgrBatchWave>,
    pub workers: Vec<LdgrBatchWorker>,
    pub blocked: Vec<LdgrBlockedTicket>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrBatchWave {
    pub wave_id: String,
    pub node_ids: Vec<String>,
    pub worker_ids: Vec<String>,
    pub status: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LdgrBatchWorker {
    pub worker_id: String,
    pub ticket_id: String,
    pub work_item_id: LdgrRef,
    pub worktree_path: LdgrRef,
    pub worker_db_path: LdgrRef,
    pub worker_artifact_root: Option<LdgrRef>,
    pub status: String,
    pub process: Option<LdgrBatchWorkerProcess>,
    pub summary: Option<LdgrBatchWorkerSummary>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrBatchWorkerProcess {
    pub launch_id: String,
    pub pid: Option<u32>,
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub stdout_path: Option<LdgrRef>,
    pub stderr_path: Option<LdgrRef>,
    pub exit_status_path: Option<LdgrRef>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrBatchWorkerSummary {
    pub run_status: Option<String>,
    pub validation_status: Option<String>,
    pub conflict_status: Option<String>,
    pub changed_files: Vec<String>,
    pub observations: Vec<String>,
    pub artifact_refs: Vec<String>,
    pub blocking_reason: Option<String>,
    pub recommended_action: Option<String>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrBlockedTicket {
    pub ticket_id: String,
    pub reason: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct LdgrValidationDocument {
    pub validator: String,
    pub status: String,
    pub targets: Vec<Map<String, Value>>,
    pub evidence: Vec<LdgrRef>,
    pub findings: Vec<LdgrFinding>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrFinding {
    pub id: String,
    pub status: String,
    pub text: String,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct LdgrRunReportDocument {
    pub status: String,
    pub summary: Option<String>,
    pub links: Map<String, Value>,
    pub outcomes: Vec<LdgrRunOutcome>,
}

#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LdgrRunOutcome {
    pub id: String,
    pub status: String,
    pub evidence: Vec<LdgrRef>,
}

pub fn parse_ldgr_projection(
    text: &str,
    source: SourceInfo,
    options: LdgrProjectionOptions,
) -> LdgrProjectionEnvelope {
    let markdown = parse_markdown(text, source.clone());
    let mut diagnostics = markdown.diagnostics.clone();
    let metadata = parse_metadata(
        markdown.payload.frontmatter.as_ref().map(|fm| &fm.value),
        &mut diagnostics,
    );
    let mut machine_blocks = parse_machine_blocks(&markdown, &options, &mut diagnostics);
    validate_machine_blocks(&metadata, &machine_blocks, &options, &mut diagnostics);
    let typed = build_typed_document(&metadata, &machine_blocks, &mut diagnostics);
    let trace = options
        .include_markdown_trace
        .then(|| MarkdownProjectionTrace {
            frontmatter_range: markdown
                .payload
                .frontmatter
                .as_ref()
                .map(|fm| fm.range.clone()),
            machine_block_ranges: machine_blocks
                .iter()
                .filter_map(|block| block.range.clone())
                .collect(),
            original_markdown_sha256: markdown
                .hashes
                .as_ref()
                .and_then(|hashes| hashes.text_sha256.clone()),
        });

    // Keep deterministic ordering for consumers that diff projections.
    machine_blocks.sort_by(|a, b| {
        a.kind.cmp(&b.kind).then(
            a.range
                .as_ref()
                .map(|r| r.byte_start)
                .cmp(&b.range.as_ref().map(|r| r.byte_start)),
        )
    });

    let document = LdgrProjectionDocument {
        schema_version: LDGR_PROJECTION_SCHEMA_VERSION.to_string(),
        metadata,
        machine_blocks,
        typed,
        markdown: trace,
    };

    Envelope::new(
        ArtifactKind::LdgrProjection,
        source,
        ParserInfo::new(PARSER),
        SchemaVersion::LDGR_PROJECTION_V1,
        document,
    )
    .with_hashes(Hashes::for_bytes(text.as_bytes(), Some(text)))
    .with_diagnostics(diagnostics)
}

pub fn validate_ldgr_projection(
    document: &LdgrProjectionDocument,
    options: LdgrProjectionValidationOptions,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    validate_machine_blocks(
        &document.metadata,
        &document.machine_blocks,
        &LdgrProjectionOptions {
            strict: options.strict,
            include_markdown_trace: false,
        },
        &mut diagnostics,
    );
    validate_typed_document(&document.metadata, &document.typed, &mut diagnostics);
    diagnostics
}

pub fn render_ldgr_projection(
    document: &LdgrProjectionDocument,
    options: LdgrProjectionRenderOptions,
) -> Result<String, LdgrProjectionRenderError> {
    let frontmatter = metadata_to_value(&document.metadata);
    let mut output = String::new();
    output.push_str("---\n");
    output.push_str(
        &serde_yaml::to_string(&frontmatter)
            .map_err(|err| LdgrProjectionRenderError::Yaml(err.to_string()))?,
    );
    output.push_str("---\n");
    if options.include_title {
        if let Some(title) = typed_title(&document.typed) {
            output.push_str("\n# ");
            output.push_str(title);
            output.push('\n');
        }
    }
    if let Some(prose) = options.contextual_prose {
        output.push('\n');
        output.push_str(prose.trim_end());
        output.push('\n');
    }
    for block in blocks_for_render(document) {
        output.push_str("\n```");
        output.push_str("ldgr-");
        output.push_str(&block.kind);
        output.push(' ');
        output.push_str(&block.format);
        for attr in &block.attributes {
            output.push(' ');
            output.push_str(attr);
        }
        output.push('\n');
        if !block.raw.trim().is_empty()
            && block.value == yaml_to_json(&block.raw).unwrap_or(Value::Null)
        {
            output.push_str(block.raw.trim_end());
            output.push('\n');
        } else {
            output.push_str(
                &serde_yaml::to_string(&block.value)
                    .map_err(|err| LdgrProjectionRenderError::Yaml(err.to_string()))?,
            );
        }
        output.push_str("```\n");
    }
    Ok(output)
}

fn parse_metadata(
    value: Option<&Option<Value>>,
    diagnostics: &mut Vec<Diagnostic>,
) -> LdgrProjectionMetadata {
    let Some(Some(Value::Object(map))) = value else {
        diagnostics.push(error(
            "frontmatter.missing",
            "LDGR projection requires YAML frontmatter",
        ));
        return LdgrProjectionMetadata::default();
    };

    let ldgr_doc = get_u64(map, "ldgr_doc", diagnostics).unwrap_or(0);
    if ldgr_doc != 1 {
        diagnostics.push(error(
            "frontmatter.ldgr_doc",
            "ldgr_doc must equal integer 1",
        ));
    }
    let kind =
        match get_string(map, "kind", diagnostics).and_then(|raw| parse_kind(&raw, diagnostics)) {
            Some(kind) => kind,
            None => LdgrDocumentKind::Ticket,
        };
    let id = get_string(map, "id", diagnostics).unwrap_or_default();
    if id.is_empty() {
        diagnostics.push(error("frontmatter.id", "id must be non-empty"));
    }
    let schema = get_string(map, "schema", diagnostics).unwrap_or_default();
    if schema.is_empty() {
        diagnostics.push(error("frontmatter.schema", "schema must be non-empty"));
    } else if !schema.contains(kind.as_str()) {
        diagnostics.push(error(
            "frontmatter.schema_mismatch",
            "schema should match document kind",
        ));
    }

    let mut metadata = LdgrProjectionMetadata {
        ldgr_doc,
        kind,
        id,
        schema,
        status: optional_string(map, "status"),
        created: optional_string(map, "created"),
        updated: optional_string(map, "updated"),
        parent: optional_ref(map, "parent", diagnostics),
        depends_on: ref_list(map, "depends_on", diagnostics),
        produces: ref_list(map, "produces", diagnostics),
        tags: string_list(map, "tags", diagnostics),
        extra: Map::new(),
    };

    let known: HashSet<&str> = [
        "ldgr_doc",
        "kind",
        "id",
        "schema",
        "status",
        "created",
        "updated",
        "parent",
        "depends_on",
        "produces",
        "tags",
    ]
    .into_iter()
    .collect();
    for (key, value) in map {
        if !known.contains(key.as_str()) {
            metadata.extra.insert(key.clone(), value.clone());
        }
    }
    metadata
}

fn parse_machine_blocks(
    markdown: &crate::markdown::MarkdownEnvelope,
    options: &LdgrProjectionOptions,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<LdgrMachineBlock> {
    let mut blocks = Vec::new();
    for node in &markdown.payload.nodes {
        if node.kind != MarkdownNodeKind::CodeFence {
            continue;
        }
        let info = node.info.as_deref().unwrap_or("");
        let mut parts = info.split_whitespace();
        let Some(first) = parts.next() else {
            continue;
        };
        let Some(kind) = first.strip_prefix("ldgr-") else {
            continue;
        };
        let format = parts.next().unwrap_or("yaml").to_string();
        let attributes = parts.map(str::to_string).collect::<Vec<_>>();
        let raw = node.text.clone().unwrap_or_default();
        let mut value = Value::Null;
        if format == "yaml" {
            match yaml_to_json(&raw) {
                Ok(parsed) => value = parsed,
                Err(err) => diagnostics.push(with_range(
                    error(
                        "machine_block.yaml",
                        format!("invalid LDGR machine block YAML: {err}"),
                    ),
                    node.range.clone(),
                )),
            }
        } else {
            diagnostics.push(with_range(
                error(
                    "machine_block.format",
                    format!("unsupported LDGR machine block format `{format}`"),
                ),
                node.range.clone(),
            ));
        }
        if !known_block_kind(kind) {
            let diagnostic = if options.strict {
                error(
                    "machine_block.unknown",
                    format!("unknown LDGR machine block kind `{kind}`"),
                )
            } else {
                Diagnostic::warning(
                    PARSER,
                    "machine_block.unknown",
                    format!("unknown LDGR machine block kind `{kind}`"),
                )
            };
            diagnostics.push(with_range(diagnostic, node.range.clone()));
        }
        blocks.push(LdgrMachineBlock {
            kind: kind.to_string(),
            format,
            attributes,
            value,
            raw,
            range: node.range.clone(),
        });
    }
    blocks
}

fn validate_machine_blocks(
    metadata: &LdgrProjectionMetadata,
    blocks: &[LdgrMachineBlock],
    _options: &LdgrProjectionOptions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(required) = metadata.kind.required_block_kind() {
        let matches = blocks.iter().filter(|block| block.kind == required).count();
        if matches == 0 {
            diagnostics.push(error(
                "machine_block.required_missing",
                format!("required ldgr-{required} block is missing"),
            ));
        } else if matches > 1 {
            diagnostics.push(error(
                "machine_block.duplicate",
                format!("duplicate singleton ldgr-{required} blocks"),
            ));
        }
    }
}

fn build_typed_document(
    metadata: &LdgrProjectionMetadata,
    blocks: &[LdgrMachineBlock],
    diagnostics: &mut Vec<Diagnostic>,
) -> LdgrDocument {
    match metadata.kind {
        LdgrDocumentKind::Ticket => {
            let doc: LdgrTicketDocument =
                typed_from_block(blocks, "contract", diagnostics).unwrap_or_default();
            validate_ticket(&doc, diagnostics);
            LdgrDocument::Ticket(doc)
        }
        LdgrDocumentKind::TicketIndex => {
            let doc: LdgrTicketIndexDocument =
                typed_from_block(blocks, "ticket-index", diagnostics).unwrap_or_default();
            validate_ticket_index(&doc, diagnostics);
            LdgrDocument::TicketIndex(doc)
        }
        LdgrDocumentKind::Graph => {
            let doc: LdgrGraphDocument =
                typed_from_block(blocks, "graph", diagnostics).unwrap_or_default();
            validate_graph(&doc, diagnostics);
            LdgrDocument::Graph(doc)
        }
        LdgrDocumentKind::BatchState => {
            let doc: LdgrBatchStateDocument =
                typed_from_block(blocks, "batch-state", diagnostics).unwrap_or_default();
            validate_batch_state(metadata, &doc, diagnostics);
            LdgrDocument::BatchState(doc)
        }
        LdgrDocumentKind::Validation => {
            let doc: LdgrValidationDocument =
                typed_from_block(blocks, "validation", diagnostics).unwrap_or_default();
            LdgrDocument::Validation(doc)
        }
        LdgrDocumentKind::RunReport => {
            let doc: LdgrRunReportDocument =
                typed_from_block(blocks, "run-report", diagnostics).unwrap_or_default();
            LdgrDocument::RunReport(doc)
        }
        LdgrDocumentKind::Spec => LdgrDocument::Spec(Value::Null),
        LdgrDocumentKind::Epoch => LdgrDocument::Epoch(Value::Null),
        LdgrDocumentKind::WorkItem => LdgrDocument::WorkItem(Value::Null),
        LdgrDocumentKind::Artifact => LdgrDocument::Artifact(Value::Null),
        LdgrDocumentKind::Decision => LdgrDocument::Decision(Value::Null),
        LdgrDocumentKind::Claim => LdgrDocument::Claim(Value::Null),
    }
}

fn validate_typed_document(
    metadata: &LdgrProjectionMetadata,
    typed: &LdgrDocument,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match (metadata.kind.clone(), typed) {
        (LdgrDocumentKind::Ticket, LdgrDocument::Ticket(doc)) => validate_ticket(doc, diagnostics),
        (LdgrDocumentKind::TicketIndex, LdgrDocument::TicketIndex(doc)) => {
            validate_ticket_index(doc, diagnostics)
        }
        (LdgrDocumentKind::Graph, LdgrDocument::Graph(doc)) => validate_graph(doc, diagnostics),
        (LdgrDocumentKind::BatchState, LdgrDocument::BatchState(doc)) => {
            validate_batch_state(metadata, doc, diagnostics)
        }
        _ => {}
    }
}

fn typed_from_block<T: for<'de> Deserialize<'de>>(
    blocks: &[LdgrMachineBlock],
    kind: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<T> {
    let block = blocks.iter().find(|block| block.kind == kind)?;
    match serde_json::from_value(block.value.clone()) {
        Ok(value) => Some(value),
        Err(err) => {
            diagnostics.push(with_range(
                error(
                    "typed.decode",
                    format!("failed to decode ldgr-{kind} block: {err}"),
                ),
                block.range.clone(),
            ));
            None
        }
    }
}

fn validate_ticket(doc: &LdgrTicketDocument, diagnostics: &mut Vec<Diagnostic>) {
    if doc.title.is_empty() {
        diagnostics.push(error("ticket.title", "ticket title is required"));
    }
    if doc.description.is_empty() {
        diagnostics.push(error(
            "ticket.description",
            "ticket description is required",
        ));
    }
    if doc.requirements.is_empty() {
        diagnostics.push(error(
            "ticket.requirements",
            "at least one requirement is required",
        ));
    }
    check_unique(
        doc.requirements.iter().map(|item| item.id.as_str()),
        "ticket.requirement_id.duplicate",
        diagnostics,
    );
    check_unique(
        doc.constraints.iter().map(|item| item.id.as_str()),
        "ticket.constraint_id.duplicate",
        diagnostics,
    );
    check_unique(
        doc.tests.iter().map(|item| item.id.as_str()),
        "ticket.test_id.duplicate",
        diagnostics,
    );
    if doc.tests.is_empty() && doc.validation_instructions.is_empty() {
        diagnostics.push(error(
            "ticket.validation",
            "empty tests require validation_instructions",
        ));
    }
}

fn validate_ticket_index(doc: &LdgrTicketIndexDocument, diagnostics: &mut Vec<Diagnostic>) {
    check_unique(
        doc.tickets.iter().map(|item| item.id.as_str()),
        "ticket_index.ticket_id.duplicate",
        diagnostics,
    );
    for entry in &doc.tickets {
        require_ref_kind(
            &entry.artifact,
            "artifact",
            "ticket_index.artifact_ref",
            diagnostics,
        );
        if let Some(work_item) = &entry.work_item {
            require_ref_kind(work_item, "work", "ticket_index.work_item_ref", diagnostics);
        }
    }
}

fn validate_graph(doc: &LdgrGraphDocument, diagnostics: &mut Vec<Diagnostic>) {
    check_unique(
        doc.nodes.iter().map(|node| node.id.as_str()),
        "graph.node_id.duplicate",
        diagnostics,
    );
    let node_ids: HashSet<&str> = doc.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &doc.edges {
        if edge.dependency == edge.dependent {
            diagnostics.push(error("graph.edge.self", "graph self-edges are not allowed"));
        }
        if !node_ids.contains(edge.dependency.as_str()) {
            diagnostics.push(error(
                "graph.edge.dependency_missing",
                format!("edge dependency `{}` is not a node", edge.dependency),
            ));
        }
        if !node_ids.contains(edge.dependent.as_str()) {
            diagnostics.push(error(
                "graph.edge.dependent_missing",
                format!("edge dependent `{}` is not a node", edge.dependent),
            ));
        }
        adjacency
            .entry(edge.dependency.as_str())
            .or_default()
            .push(edge.dependent.as_str());
    }
    if has_cycle(&adjacency) {
        diagnostics.push(error("graph.cycle", "graph contains a dependency cycle"));
    }
}

fn validate_batch_state(
    metadata: &LdgrProjectionMetadata,
    doc: &LdgrBatchStateDocument,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if doc.batch_id != metadata.id {
        diagnostics.push(error(
            "batch_state.batch_id",
            "batch_id must match frontmatter id",
        ));
    }
    require_ref_kind(
        &doc.graph_artifact_id,
        "artifact",
        "batch_state.graph_artifact_id",
        diagnostics,
    );
    require_ref_kind(
        &doc.ticket_index_artifact_id,
        "artifact",
        "batch_state.ticket_index_artifact_id",
        diagnostics,
    );
    if let Some(current_wave) = &doc.current_wave {
        if !doc.waves.iter().any(|wave| wave.wave_id == *current_wave) {
            diagnostics.push(error(
                "batch_state.current_wave",
                "current_wave must refer to an existing wave",
            ));
        }
    }
    check_unique(
        doc.workers.iter().map(|worker| worker.worker_id.as_str()),
        "batch_state.worker_id.duplicate",
        diagnostics,
    );
    for worker in &doc.workers {
        require_ref_kind(
            &worker.work_item_id,
            "work",
            "batch_state.work_item_id",
            diagnostics,
        );
        require_ref_kind(
            &worker.worktree_path,
            "path",
            "batch_state.worktree_path",
            diagnostics,
        );
        require_ref_kind(
            &worker.worker_db_path,
            "db",
            "batch_state.worker_db_path",
            diagnostics,
        );
    }
}

fn yaml_to_json(raw: &str) -> Result<Value, serde_yaml::Error> {
    serde_yaml::from_str::<serde_yaml::Value>(raw)
        .and_then(|value| serde_json::to_value(value).map_err(serde_yaml::Error::custom))
}

fn metadata_to_value(metadata: &LdgrProjectionMetadata) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("ldgr_doc".into(), Value::from(metadata.ldgr_doc));
    map.insert("kind".into(), Value::String(metadata.kind.as_str().into()));
    map.insert("id".into(), Value::String(metadata.id.clone()));
    map.insert("schema".into(), Value::String(metadata.schema.clone()));
    insert_option(&mut map, "status", metadata.status.clone());
    insert_option(&mut map, "created", metadata.created.clone());
    insert_option(&mut map, "updated", metadata.updated.clone());
    if let Some(parent) = &metadata.parent {
        map.insert("parent".into(), Value::String(parent.as_string()));
    }
    if !metadata.depends_on.is_empty() {
        map.insert(
            "depends_on".into(),
            Value::Array(
                metadata
                    .depends_on
                    .iter()
                    .map(|r| Value::String(r.as_string()))
                    .collect(),
            ),
        );
    }
    if !metadata.produces.is_empty() {
        map.insert(
            "produces".into(),
            Value::Array(
                metadata
                    .produces
                    .iter()
                    .map(|r| Value::String(r.as_string()))
                    .collect(),
            ),
        );
    }
    if !metadata.tags.is_empty() {
        map.insert(
            "tags".into(),
            Value::Array(metadata.tags.iter().cloned().map(Value::String).collect()),
        );
    }
    for (key, value) in &metadata.extra {
        map.insert(key.clone(), value.clone());
    }
    map
}

fn insert_option(map: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        map.insert(key.into(), Value::String(value));
    }
}

fn blocks_for_render(document: &LdgrProjectionDocument) -> Vec<LdgrMachineBlock> {
    let kind = document.metadata.kind.required_block_kind();
    let rendered_value = match &document.typed {
        LdgrDocument::Ticket(doc) => serde_json::to_value(doc).ok(),
        LdgrDocument::TicketIndex(doc) => serde_json::to_value(doc).ok(),
        LdgrDocument::Graph(doc) => serde_json::to_value(doc).ok(),
        LdgrDocument::BatchState(doc) => serde_json::to_value(doc).ok(),
        LdgrDocument::Validation(doc) => serde_json::to_value(doc).ok(),
        LdgrDocument::RunReport(doc) => serde_json::to_value(doc).ok(),
        _ => None,
    };
    if let (Some(kind), Some(value)) = (kind, rendered_value) {
        vec![LdgrMachineBlock {
            kind: kind.into(),
            format: "yaml".into(),
            attributes: Vec::new(),
            value,
            raw: String::new(),
            range: None,
        }]
    } else {
        document.machine_blocks.clone()
    }
}

fn typed_title(typed: &LdgrDocument) -> Option<&str> {
    match typed {
        LdgrDocument::Ticket(doc) if !doc.title.is_empty() => Some(&doc.title),
        _ => None,
    }
}

fn get_u64(map: &Map<String, Value>, key: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<u64> {
    match map.get(key) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(_) => {
            diagnostics.push(error(
                format!("frontmatter.{key}"),
                format!("{key} must be an integer"),
            ));
            None
        }
        None => {
            diagnostics.push(error(
                format!("frontmatter.{key}.missing"),
                format!("{key} is required"),
            ));
            None
        }
    }
}

fn get_string(
    map: &Map<String, Value>,
    key: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<String> {
    match map.get(key) {
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            diagnostics.push(error(
                format!("frontmatter.{key}"),
                format!("{key} must be a string"),
            ));
            None
        }
        None => {
            diagnostics.push(error(
                format!("frontmatter.{key}.missing"),
                format!("{key} is required"),
            ));
            None
        }
    }
}

fn optional_string(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_string)
}

fn optional_ref(
    map: &Map<String, Value>,
    key: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<LdgrRef> {
    map.get(key)
        .and_then(Value::as_str)
        .and_then(|raw| parse_ref_for_field(raw, key, diagnostics))
}

fn ref_list(
    map: &Map<String, Value>,
    key: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<LdgrRef> {
    match map.get(key) {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .and_then(|raw| parse_ref_for_field(raw, key, diagnostics))
            })
            .collect(),
        Some(Value::String(raw)) => parse_ref_for_field(raw, key, diagnostics)
            .into_iter()
            .collect(),
        Some(_) => {
            diagnostics.push(error(
                format!("frontmatter.{key}"),
                format!("{key} must be a string or list of strings"),
            ));
            Vec::new()
        }
    }
}

fn string_list(
    map: &Map<String, Value>,
    key: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<String> {
    match map.get(key) {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(raw)) => vec![raw.clone()],
        Some(_) => {
            diagnostics.push(error(
                format!("frontmatter.{key}"),
                format!("{key} must be a string or list of strings"),
            ));
            Vec::new()
        }
    }
}

fn parse_ref_for_field(
    raw: &str,
    field: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<LdgrRef> {
    match LdgrRef::parse(raw) {
        Ok(reference) => Some(reference),
        Err(err) => {
            diagnostics.push(error(
                format!("ref.{field}"),
                format!("invalid reference `{raw}`: {err}"),
            ));
            None
        }
    }
}

fn parse_kind(raw: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<LdgrDocumentKind> {
    let kind = match raw {
        "spec" => LdgrDocumentKind::Spec,
        "epoch" => LdgrDocumentKind::Epoch,
        "ticket" => LdgrDocumentKind::Ticket,
        "work_item" => LdgrDocumentKind::WorkItem,
        "artifact" => LdgrDocumentKind::Artifact,
        "decision" => LdgrDocumentKind::Decision,
        "claim" => LdgrDocumentKind::Claim,
        "validation" => LdgrDocumentKind::Validation,
        "run_report" => LdgrDocumentKind::RunReport,
        "graph" => LdgrDocumentKind::Graph,
        "ticket_index" => LdgrDocumentKind::TicketIndex,
        "batch_state" => LdgrDocumentKind::BatchState,
        _ => {
            diagnostics.push(error(
                "frontmatter.kind",
                format!("unsupported LDGR document kind `{raw}`"),
            ));
            return None;
        }
    };
    Some(kind)
}

fn known_block_kind(kind: &str) -> bool {
    matches!(
        kind,
        "contract" | "graph" | "ticket-index" | "batch-state" | "validation" | "run-report"
    )
}

fn require_ref_kind(
    reference: &LdgrRef,
    expected: &str,
    code: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if reference.kind != expected {
        diagnostics.push(error(
            code,
            format!(
                "reference `{}` must use `{expected}:`",
                reference.as_string()
            ),
        ));
    }
}

fn check_unique<'a>(
    values: impl Iterator<Item = &'a str>,
    code: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = HashSet::new();
    for value in values {
        if value.is_empty() {
            diagnostics.push(error(
                code.replace("duplicate", "empty"),
                "id must be non-empty",
            ));
        } else if !seen.insert(value) {
            diagnostics.push(error(code, format!("duplicate id `{value}`")));
        }
    }
}

fn has_cycle<'a>(adjacency: &HashMap<&'a str, Vec<&'a str>>) -> bool {
    fn visit<'a>(
        node: &'a str,
        adjacency: &HashMap<&'a str, Vec<&'a str>>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> bool {
        if visited.contains(node) {
            return false;
        }
        if !visiting.insert(node) {
            return true;
        }
        if let Some(nexts) = adjacency.get(node) {
            for next in nexts {
                if visit(next, adjacency, visiting, visited) {
                    return true;
                }
            }
        }
        visiting.remove(node);
        visited.insert(node);
        false
    }
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    adjacency
        .keys()
        .any(|node| visit(node, adjacency, &mut visiting, &mut visited))
}

fn error(code: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(PARSER, code, message)
}

fn with_range(mut diagnostic: Diagnostic, range: Option<SourceRange>) -> Diagnostic {
    diagnostic.range = range;
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> LdgrProjectionEnvelope {
        parse_ldgr_projection(
            text,
            SourceInfo::stdin("doc.md"),
            LdgrProjectionOptions::default(),
        )
    }

    #[test]
    fn parses_ticket_and_ignores_prose() {
        let report = parse(
            "---\nldgr_doc: 1\nkind: ticket\nid: ticket.a\nschema: ldgr.ticket.v1\nextra: kept\n---\n# Not machine\n- ignore me\n\n```ldgr-contract yaml\ntitle: Ready\ndescription: Do work.\nrequirements:\n  - id: req.a\n    text: A requirement\nvalidation_instructions:\n  - inspect output\n```\n",
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|d| d.severity != crate::core::Severity::Error)
        );
        assert_eq!(report.payload.metadata.extra["extra"], "kept");
        match &report.payload.typed {
            LdgrDocument::Ticket(ticket) => assert_eq!(ticket.title, "Ready"),
            other => panic!("unexpected typed doc: {other:?}"),
        }
    }

    #[test]
    fn detects_graph_cycle() {
        let report = parse(
            "---\nldgr_doc: 1\nkind: graph\nid: graph.a\nschema: ldgr.graph.v1\n---\n```ldgr-graph yaml\nnodes:\n  - id: a\n  - id: b\nedges:\n  - dependency: a\n    dependent: b\n  - dependency: b\n    dependent: a\n```\n",
        );
        assert!(report.diagnostics.iter().any(|d| d.code == "graph.cycle"));
    }

    #[test]
    fn parses_graph_dependency_direction_and_ticket_index() {
        let graph = parse(
            "---\nldgr_doc: 1\nkind: graph\nid: graph.a\nschema: ldgr.graph.v1\n---\n```ldgr-graph yaml\nnodes:\n  - id: first\n    artifact: artifact:1\n    work_item: work:first\n  - id: second\nedges:\n  - dependency: first\n    dependent: second\n    kind: blocks\n```\n",
        );
        match &graph.payload.typed {
            LdgrDocument::Graph(doc) => {
                assert_eq!(doc.edges[0].dependency, "first");
                assert_eq!(doc.edges[0].dependent, "second");
            }
            other => panic!("unexpected typed doc: {other:?}"),
        }

        let index = parse(
            "---\nldgr_doc: 1\nkind: ticket_index\nid: ticket_index.a\nschema: ldgr.ticket_index.v1\n---\n```ldgr-ticket-index yaml\ntickets:\n  - id: ticket.a\n    artifact: artifact:21\n    title: A\n    work_item: work:a\n```\n",
        );
        assert!(
            index
                .diagnostics
                .iter()
                .all(|d| d.severity != crate::core::Severity::Error)
        );
        match &index.payload.typed {
            LdgrDocument::TicketIndex(doc) => assert_eq!(doc.tickets[0].artifact.kind, "artifact"),
            other => panic!("unexpected typed doc: {other:?}"),
        }
    }

    #[test]
    fn parses_batch_state_and_validation_documents() {
        let batch = parse(
            "---\nldgr_doc: 1\nkind: batch_state\nid: batch.1\nschema: ldgr.batch_state.v1\n---\n```ldgr-batch-state yaml\nbatch_id: batch.1\ngraph_artifact_id: artifact:31\nticket_index_artifact_id: artifact:30\nstatus: running\ncurrent_wave: wave-1\nwaves:\n  - wave_id: wave-1\n    node_ids: [ticket.a]\n    worker_ids: [worker-1]\n    status: complete\nworkers:\n  - worker_id: worker-1\n    ticket_id: ticket.a\n    work_item_id: work:a\n    worktree_path: path:.worktrees/a\n    worker_db_path: db:workers/a/ldgr.db\n    status: success\nblocked: []\n```\n",
        );
        assert!(
            batch
                .diagnostics
                .iter()
                .all(|d| d.severity != crate::core::Severity::Error)
        );
        match &batch.payload.typed {
            LdgrDocument::BatchState(doc) => {
                assert_eq!(doc.current_wave.as_deref(), Some("wave-1"))
            }
            other => panic!("unexpected typed doc: {other:?}"),
        }

        let validation = parse(
            "---\nldgr_doc: 1\nkind: validation\nid: validation.1\nschema: ldgr.validation.v1\n---\n```ldgr-validation yaml\nvalidator: conduct.final-validator\nstatus: accepted\ntargets:\n  - graph: graph.a\nevidence:\n  - artifact:44\nfindings:\n  - id: finding.covered\n    status: passed\n    text: Covered\n```\n",
        );
        match &validation.payload.typed {
            LdgrDocument::Validation(doc) => assert_eq!(doc.evidence[0].kind, "artifact"),
            other => panic!("unexpected typed doc: {other:?}"),
        }
    }

    #[test]
    fn reports_unknown_blocks_and_invalid_machine_yaml() {
        let report = parse(
            "---\nldgr_doc: 1\nkind: run_report\nid: run.1\nschema: ldgr.run_report.v1\n---\n```ldgr-extra yaml\nvalue: true\n```\n```ldgr-run-report yaml\nstatus: [\n```\n",
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "machine_block.unknown"
                    && d.severity == crate::core::Severity::Warning)
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "machine_block.yaml" && d.range.is_some())
        );

        let strict = parse_ldgr_projection(
            "---\nldgr_doc: 1\nkind: run_report\nid: run.1\nschema: ldgr.run_report.v1\n---\n```ldgr-extra yaml\nvalue: true\n```\n",
            SourceInfo::stdin("doc.md"),
            LdgrProjectionOptions {
                strict: true,
                ..Default::default()
            },
        );
        assert!(strict.diagnostics.iter().any(
            |d| d.code == "machine_block.unknown" && d.severity == crate::core::Severity::Error
        ));
    }

    #[test]
    fn render_ticket_round_trips() {
        let report = parse(
            "---\nldgr_doc: 1\nkind: ticket\nid: ticket.a\nschema: ldgr.ticket.v1\n---\n```ldgr-contract yaml\ntitle: Ready\ndescription: Do work.\nrequirements:\n  - id: req.a\n    text: A requirement\nvalidation_instructions:\n  - inspect output\n```\n",
        );
        let rendered = render_ldgr_projection(
            &report.payload,
            LdgrProjectionRenderOptions {
                include_title: true,
                contextual_prose: None,
            },
        )
        .unwrap();
        let reparsed = parse(&rendered);
        assert_eq!(report.payload.metadata.id, reparsed.payload.metadata.id);
        assert_eq!(report.payload.typed, reparsed.payload.typed);
    }
}
