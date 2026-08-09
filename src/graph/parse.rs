use super::{
    GraphDocument, GraphEdge, GraphInputEncoding, GraphNode, GraphOptions, GraphParseRequest,
    GraphParseResult, GraphSourceMap, diagnostic_codes,
};
use crate::core::{
    ArtifactKind, BudgetProfile, BudgetSelection, CancellationToken, ContentIdentity, Diagnostic,
    InputError, LineIndex, LocationComponent, OperationControl, OperationControlError,
    OperationKind, OperationStatus, ParserInfo, SourceInfo, SourceLocator, SourceRange,
    empty_options_digest, options_digest,
};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

const PARSER: &str = "grist.graph";

/// Metadata for the built-in JSON/YAML graph syntax parser.
pub fn parser_info() -> ParserInfo {
    ParserInfo::new(PARSER)
        .with_feature("graph")
        .with_grammar_version(GraphDocument::SCHEMA_VERSION)
}

/// Parse a graph, selecting JSON or YAML deterministically from options and source hints.
pub fn parse_graph(bytes: &[u8], source: SourceInfo, options: &GraphOptions) -> GraphParseResult {
    let encoding = select_encoding(bytes, &source, options);
    parse_with_fresh_control(bytes, source, options, encoding)
}

/// Parse the JSON encoding of `grist/graph-document/v1`.
pub fn parse_graph_json(
    bytes: &[u8],
    source: SourceInfo,
    options: &GraphOptions,
) -> GraphParseResult {
    parse_with_fresh_control(bytes, source, options, GraphInputEncoding::Json)
}

/// Parse the safe YAML encoding of `grist/graph-document/v1`.
pub fn parse_graph_yaml(
    bytes: &[u8],
    source: SourceInfo,
    options: &GraphOptions,
) -> GraphParseResult {
    parse_with_fresh_control(bytes, source, options, GraphInputEncoding::Yaml)
}

/// Parse with caller-owned cancellation and resource accounting.
pub fn parse_graph_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &GraphOptions,
    control: &OperationControl,
) -> GraphParseResult {
    let encoding = select_encoding(bytes, &source, options);
    parse_controlled(bytes, source, options, encoding, control, false, false)
}

/// Registry bridge for input that the shared dispatcher already resolved and
/// charged to the operation budget.
pub(crate) fn parse_graph_resolved_with_operation_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &GraphOptions,
    control: &OperationControl,
) -> GraphParseResult {
    let encoding = select_encoding(bytes, &source, options);
    parse_controlled(bytes, source, options, encoding, control, true, false)
}

/// Resolve and parse the shared graph request without charging resolved input twice.
pub fn parse_graph_request(request: GraphParseRequest) -> GraphParseResult {
    let input = request.input;
    let source = request.source;
    let format_options = request.format_options;
    let budget = request.budget;
    let cancellation = request.cancellation;
    let digest = graph_options_digest(&format_options);
    let control = match OperationControl::new(&budget, cancellation) {
        Ok(control) => control,
        Err(error) => {
            return terminal_result(
                source,
                digest,
                None,
                OperationStatus::Failed,
                Diagnostic::error(PARSER, "grist.budget.invalid", error.to_string()),
                GraphSourceMap::default(),
            );
        }
    };
    let resolved = match input.resolve_with_control(&control) {
        Ok(input) => input,
        Err(error) => {
            let (status, diagnostic) = input_error_diagnostic(error);
            return terminal_result(
                source,
                digest,
                None,
                status,
                diagnostic,
                GraphSourceMap::default(),
            );
        }
    };
    let bytes = resolved.raw_bytes();
    let encoding = select_encoding(bytes, &source, &format_options);
    let identity = resolved.content_identity();
    parse_controlled_with_identity(
        bytes,
        source,
        &format_options,
        encoding,
        &control,
        true,
        resolved.declared_utf8(),
        Some(identity),
    )
}

fn parse_with_fresh_control(
    bytes: &[u8],
    source: SourceInfo,
    options: &GraphOptions,
    encoding: GraphInputEncoding,
) -> GraphParseResult {
    let control = OperationControl::new(
        &BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        CancellationToken::new(),
    )
    .expect("trusted graph parser budget is valid");
    parse_controlled(bytes, source, options, encoding, &control, false, false)
}

fn parse_controlled(
    bytes: &[u8],
    source: SourceInfo,
    options: &GraphOptions,
    encoding: GraphInputEncoding,
    control: &OperationControl,
    input_accounted: bool,
    decoded_accounted: bool,
) -> GraphParseResult {
    parse_controlled_with_identity(
        bytes,
        source,
        options,
        encoding,
        control,
        input_accounted,
        decoded_accounted,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn parse_controlled_with_identity(
    bytes: &[u8],
    source: SourceInfo,
    options: &GraphOptions,
    encoding: GraphInputEncoding,
    control: &OperationControl,
    input_accounted: bool,
    decoded_accounted: bool,
    supplied_identity: Option<ContentIdentity>,
) -> GraphParseResult {
    let digest = graph_options_digest(options);
    let mut identity = supplied_identity.unwrap_or_else(|| ContentIdentity::for_raw_bytes(bytes));
    if let Err(error) = control.checkpoint() {
        return control_terminal(
            source,
            digest,
            Some(identity),
            error,
            GraphSourceMap::default(),
            0,
        );
    }
    if !input_accounted
        && let Err(error) = control.budget().consume_input_bytes(u64_len(bytes.len()))
    {
        return control_terminal(
            source,
            digest,
            Some(identity),
            error.into(),
            GraphSourceMap::default(),
            0,
        );
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let diagnostic = malformed(format!("graph input is not UTF-8: {error}"), &source, None);
            return terminal_result(
                source,
                digest,
                Some(identity),
                OperationStatus::Failed,
                diagnostic,
                GraphSourceMap::default(),
            );
        }
    };
    identity = identity.with_decoded(text, "utf-8", false);
    if !decoded_accounted
        && let Err(error) = control
            .budget()
            .consume_decoded_characters(u64_len(text.chars().count()))
    {
        return control_terminal(
            source,
            digest,
            Some(identity),
            error.into(),
            GraphSourceMap::default(),
            0,
        );
    }
    if let Err(error) = control
        .budget()
        .observe_memory_bytes(u64_len(bytes.len().saturating_mul(2)))
    {
        return control_terminal(
            source,
            digest,
            Some(identity),
            error.into(),
            GraphSourceMap::default(),
            0,
        );
    }

    let locators = declaration_locators(text, encoding);
    let value = match decode_value(text, encoding) {
        Ok(value) => value,
        Err(error) => {
            let diagnostic = decode_diagnostic(error, encoding, &source, locators.graph.clone());
            return terminal_result(
                source,
                digest,
                Some(identity),
                OperationStatus::Failed,
                diagnostic,
                locators,
            );
        }
    };
    if let Err(error) = control.budget().observe_nesting_depth(value_depth(&value)) {
        return control_terminal(source, digest, Some(identity), error.into(), locators, 0);
    }
    if let Err(error) = control
        .budget()
        .observe_memory_bytes(estimated_value_bytes(&value))
    {
        return control_terminal(source, digest, Some(identity), error.into(), locators, 0);
    }

    let wire: GraphWire = match serde_json::from_value(value) {
        Ok(wire) => wire,
        Err(error) => {
            let message = error.to_string();
            let code = if message.contains("unknown field") {
                diagnostic_codes::FIELD_UNKNOWN
            } else {
                "grist.input.malformed"
            };
            let diagnostic = Diagnostic::error(PARSER, code, message)
                .with_source(source.display_name.clone())
                .with_locator(locators.graph.clone().expect("graph locator exists"));
            return terminal_result(
                source,
                digest,
                Some(identity),
                OperationStatus::Failed,
                diagnostic,
                locators,
            );
        }
    };
    if wire.schema_version != GraphDocument::SCHEMA_VERSION {
        let diagnostic = Diagnostic::error(
            PARSER,
            diagnostic_codes::SCHEMA_VERSION_UNSUPPORTED,
            format!(
                "unsupported graph schema version {:?}; expected {:?}",
                wire.schema_version,
                GraphDocument::SCHEMA_VERSION
            ),
        )
        .with_source(source.display_name.clone())
        .with_locator(field_locator(text, encoding, "/schema_version"));
        return terminal_result(
            source,
            digest,
            Some(identity),
            OperationStatus::Unsupported,
            diagnostic,
            locators,
        );
    }
    if let Some(message) = validate_lexical_fields(&wire) {
        let diagnostic = malformed(message, &source, locators.graph.clone());
        return terminal_result(
            source,
            digest,
            Some(identity),
            OperationStatus::Failed,
            diagnostic,
            locators,
        );
    }

    let mut document = GraphDocument {
        schema_version: wire.schema_version,
        id: wire.id,
        directed: wire.directed,
        nodes: Vec::new(),
        edges: Vec::new(),
        attrs: wire.attrs,
    };
    let mut retained_map = GraphSourceMap {
        graph: locators.graph.clone(),
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    if let Err(error) = control.budget().consume_nodes(1) {
        return control_terminal(
            source,
            digest,
            Some(identity),
            error.into(),
            retained_map,
            0,
        );
    }
    for (index, node) in wire.nodes.into_iter().enumerate() {
        if let Err(error) = control
            .checkpoint()
            .and_then(|_| control.budget().consume_nodes(1).map_err(Into::into))
        {
            return partial_or_terminal(source, digest, identity, error, document, retained_map);
        }
        document.nodes.push(node.into());
        retained_map.nodes.push(
            locators
                .nodes
                .get(index)
                .cloned()
                .unwrap_or_else(|| field_locator(text, encoding, &format!("/nodes/{index}"))),
        );
    }
    for (index, edge) in wire.edges.into_iter().enumerate() {
        if let Err(error) = control
            .checkpoint()
            .and_then(|_| control.budget().consume_records(1).map_err(Into::into))
        {
            return partial_or_terminal(source, digest, identity, error, document, retained_map);
        }
        document.edges.push(edge.into_graph_edge(document.directed));
        retained_map.edges.push(
            locators
                .edges
                .get(index)
                .cloned()
                .unwrap_or_else(|| field_locator(text, encoding, &format!("/edges/{index}"))),
        );
    }
    let output_bytes = serde_json::to_vec(&document)
        .map(|bytes| u64_len(bytes.len()))
        .unwrap_or(u64::MAX);
    if let Err(error) = control.budget().consume_output_bytes(output_bytes) {
        return partial_or_terminal(
            source,
            digest,
            identity,
            error.into(),
            document,
            retained_map,
        );
    }
    if let Err(error) = control.checkpoint() {
        return partial_or_terminal(source, digest, identity, error, document, retained_map);
    }

    let envelope = crate::core::Envelope::complete(
        OperationKind::Parse,
        ArtifactKind::GraphDocument,
        source,
        parser_info(),
        digest,
        GraphDocument::SCHEMA_VERSION,
        document,
    )
    .with_identity(identity)
    .with_canonical_payload_identity()
    .expect("GraphDocument canonical serialization is infallible");
    GraphParseResult {
        envelope,
        source_map: retained_map,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphWire {
    schema_version: String,
    #[serde(default)]
    id: Option<String>,
    directed: bool,
    nodes: Vec<NodeWire>,
    edges: Vec<EdgeWire>,
    #[serde(default)]
    attrs: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeWire {
    id: String,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    attrs: BTreeMap<String, Value>,
}

impl From<NodeWire> for GraphNode {
    fn from(node: NodeWire) -> Self {
        Self {
            id: node.id,
            labels: node.labels,
            attrs: node.attrs,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeWire {
    id: String,
    source: String,
    target: String,
    #[serde(default)]
    directed: Option<bool>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    attrs: BTreeMap<String, Value>,
}

impl EdgeWire {
    fn into_graph_edge(self, document_directed: bool) -> GraphEdge {
        GraphEdge {
            id: self.id,
            source: self.source,
            target: self.target,
            directed: self.directed.unwrap_or(document_directed),
            label: self.label,
            attrs: self.attrs,
        }
    }
}

fn validate_lexical_fields(wire: &GraphWire) -> Option<String> {
    if wire.id.as_deref() == Some("") {
        return Some("graph id must not be empty".into());
    }
    for (index, node) in wire.nodes.iter().enumerate() {
        if node.id.is_empty() {
            return Some(format!("node {index} id must not be empty"));
        }
        if node.labels.iter().any(String::is_empty) {
            return Some(format!(
                "node {index} labels must not contain an empty string"
            ));
        }
    }
    for (index, edge) in wire.edges.iter().enumerate() {
        if edge.id.is_empty() || edge.source.is_empty() || edge.target.is_empty() {
            return Some(format!(
                "edge {index} id, source, and target must not be empty"
            ));
        }
        if edge.label.as_deref() == Some("") {
            return Some(format!("edge {index} label must not be empty"));
        }
    }
    None
}

#[derive(Debug)]
enum DecodeError {
    Malformed(String),
    UnsupportedYaml(String),
}

fn decode_value(text: &str, encoding: GraphInputEncoding) -> Result<Value, DecodeError> {
    match encoding {
        GraphInputEncoding::Json => serde_json::from_str::<StrictValue>(text)
            .map(|value| value.0)
            .map_err(|error| DecodeError::Malformed(error.to_string())),
        GraphInputEncoding::Yaml => {
            scan_yaml_safety(text)?;
            let mut documents = serde_yaml::Deserializer::from_str(text);
            let Some(document) = documents.next() else {
                return Err(DecodeError::Malformed("YAML document is empty".into()));
            };
            let value = StrictValue::deserialize(document)
                .map(|value| value.0)
                .map_err(|error| DecodeError::Malformed(error.to_string()))?;
            if documents.next().is_some() {
                return Err(DecodeError::UnsupportedYaml(
                    "multiple YAML documents are not supported".into(),
                ));
            }
            Ok(value)
        }
    }
}

/// A JSON-compatible value decoder that rejects duplicate keys before maps are built.
struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON-compatible scalar, sequence, or mapping")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Null))
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if !value.is_finite() {
            return Err(E::custom("non-finite numbers are not supported"));
        }
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(StrictValue)
            .ok_or_else(|| E::custom("number cannot be represented as JSON"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value.into())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(StrictValue(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        let mut values = serde_json::Map::new();
        while let Some(key) = mapping.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate mapping key {key:?}")));
            }
            let StrictValue(value) = mapping.next_value()?;
            values.insert(key, value);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

fn scan_yaml_safety(text: &str) -> Result<(), DecodeError> {
    let mut documents = 0_u32;
    for line in text.lines() {
        let content = strip_yaml_comment(line).trim();
        if content == "---" {
            documents += 1;
            if documents > 1 {
                return Err(DecodeError::UnsupportedYaml(
                    "multiple YAML documents are not supported".into(),
                ));
            }
            continue;
        }
        if content == "..." {
            continue;
        }
        if content.starts_with("<<:") || content.contains(" <<:") {
            return Err(DecodeError::UnsupportedYaml(
                "YAML merge keys are not supported".into(),
            ));
        }
        if let Some(indicator) = forbidden_yaml_indicator(content) {
            return Err(DecodeError::UnsupportedYaml(format!(
                "YAML {indicator} are not supported"
            )));
        }
        let scalar = content
            .rsplit_once(':')
            .map_or(content, |(_, value)| value)
            .trim()
            .trim_start_matches('-')
            .trim();
        if matches!(
            scalar.to_ascii_lowercase().as_str(),
            ".nan" | ".inf" | "+.inf" | "-.inf"
        ) || contains_non_finite_yaml_scalar(content)
        {
            return Err(DecodeError::UnsupportedYaml(
                "non-finite YAML numbers are not supported".into(),
            ));
        }
    }
    Ok(())
}

fn contains_non_finite_yaml_scalar(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    [".nan", ".inf", "+.inf", "-.inf"].iter().any(|needle| {
        lower.match_indices(needle).any(|(start, value)| {
            let before = lower[..start].chars().next_back();
            let after = lower[start + value.len()..].chars().next();
            before.is_none_or(|character| {
                character.is_whitespace() || matches!(character, ':' | ',' | '[' | '{' | '-')
            }) && after.is_none_or(|character| {
                character.is_whitespace() || matches!(character, ',' | ']' | '}')
            })
        })
    })
}

fn strip_yaml_comment(line: &str) -> &str {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if double && character == '\\' && !escaped {
            escaped = true;
            continue;
        }
        if character == '"' && !single && !escaped {
            double = !double;
        } else if character == '\'' && !double {
            single = !single;
        } else if character == '#' && !single && !double {
            return &line[..index];
        }
        escaped = false;
    }
    line
}

fn forbidden_yaml_indicator(content: &str) -> Option<&'static str> {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut boundary = true;
    for character in content.chars() {
        if double && character == '\\' && !escaped {
            escaped = true;
            continue;
        }
        if character == '"' && !single && !escaped {
            double = !double;
        } else if character == '\'' && !double {
            single = !single;
        } else if !single && !double && boundary {
            match character {
                '&' => return Some("anchors"),
                '*' => return Some("aliases"),
                '!' => return Some("explicit tags"),
                _ => {}
            }
        }
        boundary = character.is_whitespace() || matches!(character, ':' | ',' | '[' | '{' | '-');
        escaped = false;
    }
    None
}

fn decode_diagnostic(
    error: DecodeError,
    encoding: GraphInputEncoding,
    source: &SourceInfo,
    locator: Option<SourceLocator>,
) -> Diagnostic {
    match error {
        DecodeError::Malformed(message) => malformed(
            format!(
                "malformed {} graph input: {message}",
                encoding_name(encoding)
            ),
            source,
            locator,
        ),
        DecodeError::UnsupportedYaml(message) => {
            let mut diagnostic =
                Diagnostic::error(PARSER, diagnostic_codes::YAML_FEATURE_UNSUPPORTED, message)
                    .with_source(source.display_name.clone());
            if let Some(locator) = locator {
                diagnostic = diagnostic.with_locator(locator);
            }
            diagnostic
        }
    }
}

fn malformed(
    message: impl Into<String>,
    source: &SourceInfo,
    locator: Option<SourceLocator>,
) -> Diagnostic {
    let mut diagnostic =
        Diagnostic::malformed(PARSER, message).with_source(source.display_name.clone());
    if let Some(locator) = locator {
        diagnostic = diagnostic.with_locator(locator);
    }
    diagnostic
}

fn select_encoding(
    bytes: &[u8],
    source: &SourceInfo,
    options: &GraphOptions,
) -> GraphInputEncoding {
    if let Some(encoding) = options.encoding {
        return encoding;
    }
    let mime = source.declared_mime_type.as_deref().unwrap_or_default();
    if mime.eq_ignore_ascii_case("application/json") || mime.ends_with("+json") {
        return GraphInputEncoding::Json;
    }
    if matches!(
        mime.to_ascii_lowercase().as_str(),
        "application/yaml" | "application/x-yaml" | "text/yaml" | "text/x-yaml"
    ) {
        return GraphInputEncoding::Yaml;
    }
    let path = source.path.as_deref().unwrap_or(&source.display_name);
    if path
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("json"))
    {
        return GraphInputEncoding::Json;
    }
    let first = bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace());
    if matches!(first, Some(b'{') | Some(b'[')) {
        GraphInputEncoding::Json
    } else {
        GraphInputEncoding::Yaml
    }
}

fn encoding_name(encoding: GraphInputEncoding) -> &'static str {
    match encoding {
        GraphInputEncoding::Json => "JSON",
        GraphInputEncoding::Yaml => "YAML",
    }
}

fn declaration_locators(text: &str, encoding: GraphInputEncoding) -> GraphSourceMap {
    match encoding {
        GraphInputEncoding::Json => GraphSourceMap {
            graph: Some(json_pointer("")),
            nodes: json_array_len(text, "nodes")
                .map(|length| {
                    (0..length)
                        .map(|index| json_pointer(&format!("/nodes/{index}")))
                        .collect()
                })
                .unwrap_or_default(),
            edges: json_array_len(text, "edges")
                .map(|length| {
                    (0..length)
                        .map(|index| json_pointer(&format!("/edges/{index}")))
                        .collect()
                })
                .unwrap_or_default(),
        },
        GraphInputEncoding::Yaml => {
            let index = LineIndex::new(text);
            GraphSourceMap {
                graph: Some(text_locator(0, text.len(), &index)),
                nodes: yaml_sequence_ranges(text, "nodes")
                    .into_iter()
                    .map(|(start, end)| text_locator(start, end, &index))
                    .collect(),
                edges: yaml_sequence_ranges(text, "edges")
                    .into_iter()
                    .map(|(start, end)| text_locator(start, end, &index))
                    .collect(),
            }
        }
    }
}

fn json_array_len(text: &str, field: &str) -> Option<usize> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get(field)?
        .as_array()
        .map(Vec::len)
}

fn yaml_sequence_ranges(text: &str, field: &str) -> Vec<(usize, usize)> {
    let lines = line_ranges(text);
    let Some((section_index, _, _, section_indent)) =
        lines.iter().enumerate().find_map(|(index, (start, end))| {
            let line = &text[*start..*end];
            let trimmed = strip_yaml_comment(line).trim_end();
            let indent = trimmed.len().saturating_sub(trimmed.trim_start().len());
            (indent == 0 && trimmed.trim_start() == format!("{field}:"))
                .then_some((index, *start, *end, indent))
        })
    else {
        return Vec::new();
    };
    let mut starts = Vec::new();
    let mut item_indent = None;
    let mut section_end = text.len();
    for (start, end) in lines.iter().skip(section_index + 1).copied() {
        let raw = &text[start..end];
        let content = strip_yaml_comment(raw).trim_end_matches(['\r', '\n']);
        if content.trim().is_empty() {
            continue;
        }
        let indent = content.len().saturating_sub(content.trim_start().len());
        if indent <= section_indent {
            section_end = start;
            break;
        }
        let trimmed = content.trim_start();
        if trimmed == "-" || trimmed.starts_with("- ") {
            let expected = *item_indent.get_or_insert(indent);
            if indent == expected {
                starts.push(start + indent);
            }
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(section_end);
            (*start, trim_range_end(text, *start, end))
        })
        .collect()
}

fn line_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        ranges.push((start, end));
        start = end;
    }
    if start < text.len() || text.is_empty() {
        ranges.push((start, text.len()));
    }
    ranges
}

fn trim_range_end(text: &str, start: usize, mut end: usize) -> usize {
    while end > start && text.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end
}

fn field_locator(text: &str, encoding: GraphInputEncoding, pointer: &str) -> SourceLocator {
    match encoding {
        GraphInputEncoding::Json => json_pointer(pointer),
        GraphInputEncoding::Yaml => {
            let index = LineIndex::new(text);
            text_locator(0, text.len(), &index)
        }
    }
}

fn json_pointer(pointer: &str) -> SourceLocator {
    SourceLocator::exact(LocationComponent::JsonPointer {
        pointer: pointer.into(),
    })
    .expect("JSON Pointer locator is valid")
}

fn text_locator(start: usize, end: usize, index: &LineIndex) -> SourceLocator {
    SourceLocator::exact(SourceRange::new(start, end, index))
        .expect("decoded text range locator is valid")
}

fn value_depth(value: &Value) -> u64 {
    fn depth(value: &Value, current: u64) -> u64 {
        match value {
            Value::Array(values) => values
                .iter()
                .map(|value| depth(value, current.saturating_add(1)))
                .max()
                .unwrap_or(current.saturating_add(1)),
            Value::Object(values) => values
                .values()
                .map(|value| depth(value, current.saturating_add(1)))
                .max()
                .unwrap_or(current.saturating_add(1)),
            _ => current,
        }
    }
    depth(value, 1)
}

fn estimated_value_bytes(value: &Value) -> u64 {
    serde_json::to_vec(value)
        .map(|bytes| u64_len(bytes.len().saturating_mul(2)))
        .unwrap_or(u64::MAX)
}

fn partial_or_terminal(
    source: SourceInfo,
    digest: String,
    identity: ContentIdentity,
    error: OperationControlError,
    document: GraphDocument,
    source_map: GraphSourceMap,
) -> GraphParseResult {
    if matches!(error, OperationControlError::Cancelled(_)) {
        return control_terminal(
            source,
            digest,
            Some(identity),
            error,
            GraphSourceMap::default(),
            0,
        );
    }
    let retained = u64_len(
        document
            .nodes
            .len()
            .saturating_add(document.edges.len())
            .saturating_add(1),
    );
    let diagnostic = error.diagnostic(PARSER);
    let envelope = crate::core::Envelope::partial(
        OperationKind::Parse,
        ArtifactKind::GraphDocument,
        source,
        parser_info(),
        digest,
        GraphDocument::SCHEMA_VERSION,
        (retained > 0).then_some(document),
    )
    .with_identity(identity)
    .with_diagnostics(vec![diagnostic])
    .with_canonical_payload_identity()
    .expect("partial GraphDocument canonical serialization is infallible");
    GraphParseResult {
        envelope,
        source_map,
    }
}

fn control_terminal(
    source: SourceInfo,
    digest: String,
    identity: Option<ContentIdentity>,
    error: OperationControlError,
    source_map: GraphSourceMap,
    emitted: u64,
) -> GraphParseResult {
    let status = error.operation_status(emitted);
    if status == OperationStatus::Partial {
        unreachable!("partial graph control errors require a retained GraphDocument")
    }
    terminal_result(
        source,
        digest,
        identity,
        status,
        error.diagnostic(PARSER),
        source_map,
    )
}

fn terminal_result(
    source: SourceInfo,
    digest: String,
    identity: Option<ContentIdentity>,
    status: OperationStatus,
    diagnostic: Diagnostic,
    source_map: GraphSourceMap,
) -> GraphParseResult {
    let mut envelope = crate::core::Envelope::without_payload(
        OperationKind::Parse,
        ArtifactKind::GraphDocument,
        status,
        source,
        parser_info(),
        digest,
        GraphDocument::SCHEMA_VERSION,
    )
    .expect("graph terminal status must not carry a payload")
    .with_diagnostics(vec![diagnostic]);
    if let Some(identity) = identity {
        envelope = envelope.with_identity(identity);
    }
    GraphParseResult {
        envelope,
        source_map,
    }
}

fn input_error_diagnostic(error: InputError) -> (OperationStatus, Diagnostic) {
    let message = error.to_string();
    match error {
        InputError::Cancelled => (
            OperationStatus::Cancelled,
            Diagnostic::info(PARSER, "grist.operation.cancelled", message),
        ),
        InputError::ByteLimitExceeded { .. } | InputError::BudgetExceeded(_) => (
            OperationStatus::Failed,
            Diagnostic::budget_exhausted(PARSER, message),
        ),
        InputError::InvalidBudget(_) => (
            OperationStatus::Failed,
            Diagnostic::error(PARSER, "grist.budget.invalid", message),
        ),
        InputError::Io(_) => (
            OperationStatus::Failed,
            Diagnostic::malformed(PARSER, message),
        ),
    }
}

fn graph_options_digest(options: &GraphOptions) -> String {
    options_digest(options).unwrap_or_else(|_| empty_options_digest())
}

fn u64_len(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
