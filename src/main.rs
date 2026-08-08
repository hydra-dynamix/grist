#[cfg(feature = "cli")]
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(feature = "cli")]
use grist::core::{Diagnostic, RequestId, SourceInfo};
#[cfg(feature = "cli")]
use grist::ingest::{FileIngestOptions, RepoIngestOptions};
#[cfg(feature = "cli")]
use std::io::Read;
#[cfg(feature = "cli")]
use std::path::PathBuf;

#[cfg(feature = "cli")]
#[derive(Parser)]
#[command(
    name = "grist",
    about = "Grist interpretation utility",
    long_about = "Grist parses documents/code/model outputs into typed JSON envelopes, emits public JSON Schemas, ingests files/repos, and transforms supported formats through DocumentGraph."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum Command {
    /// Detect one input and retain ranked ambiguity evidence.
    Detect {
        /// Input path or `-` for stdin.
        input: String,
        /// Filename hint, especially for stdin.
        #[arg(long)]
        filename: Option<String>,
        /// Declared MIME type hint.
        #[arg(long)]
        mime: Option<String>,
        /// Explicit format-kind hint.
        #[arg(long)]
        kind: Option<String>,
        /// Stable caller correlation ID.
        #[arg(long, default_value = "request-000000")]
        request_id: String,
    },
    /// Parse one input into a typed Grist JSON envelope.
    Parse {
        #[command(subcommand)]
        command: ParseCommand,
    },
    /// Detect and ingest one file or repository into Grist artifact reports.
    Ingest {
        #[command(subcommand)]
        command: IngestCommand,
    },
    /// Detect, parse, and return one universal inspection envelope.
    Inspect {
        /// Input path or `-` for stdin.
        input: String,
        /// Filename hint, especially for stdin.
        #[arg(long)]
        filename: Option<String>,
        /// Declared MIME type hint.
        #[arg(long)]
        mime: Option<String>,
        /// Explicit format-kind hint.
        #[arg(long)]
        kind: Option<String>,
        /// Stable caller correlation ID.
        #[arg(long, default_value = "request-000000")]
        request_id: String,
    },
    /// List or emit checked-in public JSON Schema contracts.
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
    /// Render parsed artifacts into stable inspection JSON such as rendered summaries.
    Render {
        #[command(subcommand)]
        command: RenderCommand,
    },
    /// Validate inputs against stable Grist-supported contracts.
    Validate {
        #[command(subcommand)]
        command: ValidateCommand,
    },
    /// Segment a source or DocumentGraph with a versioned options file.
    Segment {
        /// Source or graph path, or `-` for stdin.
        input: String,
        /// JSON, YAML, or TOML SegmentOptions file.
        #[arg(long)]
        config: PathBuf,
        /// Treat input as an already serialized DocumentGraph.
        #[arg(long)]
        graph: bool,
        /// Filename hint, especially for stdin.
        #[arg(long)]
        filename: Option<String>,
        /// Declared MIME type hint.
        #[arg(long)]
        mime: Option<String>,
        /// Explicit source format hint.
        #[arg(long)]
        kind: Option<String>,
        /// Stable caller correlation ID.
        #[arg(long, default_value = "request-000000")]
        request_id: String,
        /// Emit a documented NDJSON SegmentEvent stream.
        #[arg(long)]
        stream: bool,
    },
    /// Report the complete compiled feature and capability manifest.
    Capabilities,
    /// Convert supported inputs through DocumentGraph and render graph/Markdown/LaTeX.
    Transform {
        /// Positional paths: INPUT [OUTPUT], or OUTPUT when --file supplies INPUT. Input kind is inferred from extension: .md, .tex, .py, .rs, .ts, .tsx, .jsx.
        #[arg(value_name = "PATH", num_args = 0..=2)]
        paths: Vec<String>,
        /// Input path as a named flag, equivalent to the positional INPUT.
        #[arg(long, value_name = "INPUT")]
        file: Option<String>,
        /// Output path. If omitted, output is written to stdout.
        #[arg(short, long, value_name = "OUTPUT")]
        output: Option<PathBuf>,
        /// Write the normalized text fidelity/source-map manifest to this path.
        #[arg(long, value_name = "MANIFEST")]
        manifest: Option<PathBuf>,
        /// Explicit fidelity policy for normalized text targets.
        #[arg(long, value_enum, default_value_t = FidelityArg::Strict)]
        fidelity: FidelityArg,
        /// Stable correlation ID included in text-output manifests.
        #[arg(long, default_value = "request-000000")]
        request_id: String,
        /// Target representation to emit.
        #[arg(long, value_enum)]
        to: TransformTargetArg,
        /// Run deterministic conditional-obligation extraction before emitting the target.
        #[arg(long)]
        extract_obligations: bool,
    },
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum ParseCommand {
    /// Detect and parse through the built-in parser registry.
    Auto {
        /// Input path or `-` for stdin.
        input: String,
        /// Filename hint, especially for stdin.
        #[arg(long)]
        filename: Option<String>,
        /// Declared MIME type hint.
        #[arg(long)]
        mime: Option<String>,
        /// Explicit format-kind hint.
        #[arg(long)]
        kind: Option<String>,
        /// Stable caller correlation ID.
        #[arg(long, default_value = "request-000000")]
        request_id: String,
    },
    /// Parse plain text through the registry.
    Text {
        /// Input path or `-` for stdin.
        input: String,
    },
    /// Parse Markdown headings, paragraphs, links, fences, tables, and frontmatter.
    Markdown {
        /// Input path or `-` for stdin.
        input: String,
    },
    /// Parse LDGR Markdown Projection documents.
    #[command(name = "ldgr-projection")]
    LdgrProjection {
        /// Input path or `-` for stdin.
        input: String,
        /// Treat validation diagnostics as strict parse diagnostics.
        #[arg(long)]
        strict: bool,
    },
    /// Parse HTML fragments or documents.
    Html {
        /// Input path or `-` for stdin.
        input: String,
        /// HTML parse mode.
        #[arg(long, value_enum, default_value_t = HtmlModeArg::Auto)]
        mode: HtmlModeArg,
    },
    /// Parse XML or JATS as inert namespace-aware structure.
    Xml {
        /// Input path or `-` for stdin.
        input: String,
        /// XML dialect selection.
        #[arg(long, value_enum, default_value_t = XmlDialectArg::Auto)]
        dialect: XmlDialectArg,
    },
    /// Parse CSV data.
    #[command(alias = "tsv")]
    Csv {
        /// Input path or `-` for stdin.
        input: String,
        /// CSV delimiter.
        #[arg(long, value_enum, default_value_t = CsvDelimiterArg::Auto)]
        delimiter: CsvDelimiterArg,
        /// Treat the first row as data instead of headers.
        #[arg(long)]
        no_headers: bool,
    },
    /// Parse an OOXML workbook without calculating formulas.
    Xlsx { input: String },
    /// Parse a macro-enabled OOXML workbook and quarantine VBA projects.
    Xlsm { input: String },
    /// Parse an OpenDocument workbook without calculating formulas.
    Ods { input: String },
    /// Parse an OpenDocument spreadsheet template without calculating formulas.
    Ots { input: String },
    /// Parse Rust code with tree-sitter.
    Rust {
        /// Input path or `-` for stdin.
        input: String,
        /// Semantic/syntax detail level.
        #[arg(long, value_enum, default_value_t = RustDetailArg::Semantic)]
        detail: RustDetailArg,
    },
    /// Parse Python code with tree-sitter.
    Python {
        /// Input path or `-` for stdin.
        input: String,
        /// Semantic/syntax detail level.
        #[arg(long, value_enum, default_value_t = PythonDetailArg::Semantic)]
        detail: PythonDetailArg,
    },
    /// Parse JavaScript or JSX code with tree-sitter-javascript.
    #[command(name = "javascript", alias = "js")]
    JavaScript {
        /// Input path or `-` for stdin.
        input: String,
        /// JavaScript parser dialect.
        #[arg(long, value_enum, default_value_t = JavaScriptDialectArg::JavaScript)]
        dialect: JavaScriptDialectArg,
        /// Semantic/syntax detail level.
        #[arg(long, value_enum, default_value_t = JavaScriptDetailArg::Semantic)]
        detail: JavaScriptDetailArg,
    },
    /// Parse LaTeX documents, preserving unknown commands as raw nodes.
    Latex {
        /// Input path or `-` for stdin.
        input: String,
        /// Semantic/syntax detail level.
        #[arg(long, value_enum, default_value_t = LatexDetailArg::Semantic)]
        detail: LatexDetailArg,
        /// Canonical filesystem boundary for local input/include resolution.
        #[arg(long = "project-root")]
        project_roots: Vec<PathBuf>,
        /// Retain input/include commands as references without opening files.
        #[arg(long)]
        no_resolve_includes: bool,
    },
    /// Parse TypeScript, TSX, or JSX code with tree-sitter.
    #[command(name = "typescript", alias = "ts")]
    TypeScript {
        /// Input path or `-` for stdin.
        input: String,
        /// TypeScript parser dialect.
        #[arg(long, value_enum, default_value_t = TypeScriptDialectArg::TypeScript)]
        dialect: TypeScriptDialectArg,
        /// Semantic/syntax detail level.
        #[arg(long, value_enum, default_value_t = TypeScriptDetailArg::Semantic)]
        detail: TypeScriptDetailArg,
    },
    /// Parse JSON, JSONL, YAML, or TOML values with optional schema validation.
    Json {
        /// Input path or `-` for stdin.
        input: String,
        /// Serialization format.
        #[arg(long, value_enum, default_value_t = SerializationFormatArg::Json)]
        format: SerializationFormatArg,
        /// Optional JSON Schema file.
        #[arg(long)]
        schema: Option<PathBuf>,
    },
    /// Parse CBOR values or CBOR sequences with exact byte locators.
    Cbor {
        /// Input path or stdin marker.
        input: String,
    },
    /// Parse MessagePack objects or concatenated streams with extension retention.
    #[command(name = "messagepack", alias = "msgpack")]
    MessagePack {
        /// Input path or stdin marker.
        input: String,
    },
    /// Parse Protocol Buffers using a serialized FileDescriptorSet.
    Protobuf {
        /// Binary message input path or stdin marker.
        input: String,
        /// Serialized google.protobuf.FileDescriptorSet path.
        #[arg(long)]
        descriptor: PathBuf,
        /// Fully-qualified message name in the descriptor set.
        #[arg(long)]
        message: String,
    },
    /// Parse model-output text, repair candidate JSON/tool calls, and optionally validate.
    ModelOutput {
        /// Input path or `-` for stdin.
        input: String,
        /// Optional JSON Schema file.
        #[arg(long)]
        schema: Option<PathBuf>,
        /// Optional alias/repair rules file.
        #[arg(long)]
        rules: Option<PathBuf>,
        /// Remove <think>...</think> blocks before parsing.
        #[arg(long)]
        strip_think_blocks: bool,
        /// Enable Python-style command parsing.
        #[arg(long)]
        python_style: bool,
        /// Emit only the selected JSON value instead of the full envelope.
        #[arg(long)]
        json_value: bool,
    },
    /// Route any enabled registry format without adding CLI parser logic.
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum IngestCommand {
    /// Detect and ingest one file or stdin stream.
    File {
        /// Input path or `-` for stdin.
        input: String,
        /// Filename hint for stdin detection.
        #[arg(long)]
        filename: Option<PathBuf>,
        /// Declared MIME type hint.
        #[arg(long)]
        mime: Option<String>,
        /// Explicit kind hint.
        #[arg(long)]
        kind: Option<String>,
    },
    /// Walk a repository and ingest supported files.
    Repo {
        /// Repository root path.
        path: PathBuf,
        /// Include ignored files instead of honoring ignore rules.
        #[arg(long)]
        include_ignored: bool,
        /// Include glob; may be repeated.
        #[arg(long = "include")]
        include_globs: Vec<String>,
        /// Exclude glob; may be repeated.
        #[arg(long = "exclude")]
        exclude_globs: Vec<String>,
        /// Write artifacts to this directory instead of inlining them.
        #[arg(long)]
        external_artifact_dir: Option<PathBuf>,
    },
    /// Traverse an archive/container through the shared recursion controller.
    Archive {
        /// Input path or `-` for stdin.
        input: String,
        /// Filename hint, especially for stdin.
        #[arg(long)]
        filename: Option<String>,
        /// Declared MIME type hint.
        #[arg(long)]
        mime: Option<String>,
        /// Explicit container format hint.
        #[arg(long)]
        kind: Option<String>,
        /// Stable caller correlation ID.
        #[arg(long, default_value = "request-000000")]
        request_id: String,
        /// Inventory members without parsing leaf payloads.
        #[arg(long)]
        inventory_only: bool,
    },
    /// Ingest many inputs in stable caller order.
    Batch {
        /// Input paths. At most one may be `-`.
        #[arg(required = true)]
        inputs: Vec<String>,
        /// Explicit parser format or `auto`.
        #[arg(long, default_value = "auto")]
        format: String,
        /// Request IDs in input order. Defaults are deterministic by sequence.
        #[arg(long = "request-id")]
        request_ids: Vec<String>,
        /// Collect the stream into one JSON batch result instead of NDJSON.
        #[arg(long)]
        collect: bool,
    },
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum RenderCommand {
    /// Render a DocumentGraph with a fidelity report and complete source map.
    Graph {
        /// DocumentGraph JSON path or `-` for stdin.
        input: String,
        /// Normalized target format.
        #[arg(long, value_enum)]
        to: RenderTargetArg,
        /// Explicit fidelity policy.
        #[arg(long, value_enum, default_value_t = FidelityArg::Strict)]
        fidelity: FidelityArg,
        /// Write raw text here and emit only its manifest to stdout.
        #[arg(long)]
        text_output: Option<PathBuf>,
        /// Stable caller correlation ID for raw-output manifests.
        #[arg(long, default_value = "request-000000")]
        request_id: String,
    },
    /// Parse JSON and emit a structured rendered-summary JSON document.
    #[command(name = "json-summary")]
    JsonSummary {
        /// Input path or `-` for stdin.
        input: String,
        /// Optional JSON Schema file to validate before summarization.
        #[arg(long)]
        schema: Option<PathBuf>,
        /// Optional built-in summarization profile.
        #[arg(long, value_enum)]
        profile: Option<SummaryProfileArg>,
    },
    /// Parse JSON/JSONL/YAML/TOML and emit a structured rendered-summary JSON document.
    #[command(name = "serialization-summary")]
    SerializationSummary {
        /// Input path or `-` for stdin.
        input: String,
        /// Serialization format.
        #[arg(long, value_enum, default_value_t = SerializationFormatArg::Json)]
        format: SerializationFormatArg,
        /// Optional JSON Schema file to validate before summarization.
        #[arg(long)]
        schema: Option<PathBuf>,
        /// Optional built-in summarization profile.
        #[arg(long, value_enum)]
        profile: Option<SummaryProfileArg>,
    },
    /// Render a DocumentGraph using the normative `render <graph-path> --to` form.
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum ValidateCommand {
    /// Validate a JSON value against a named or filesystem JSON Schema.
    Input {
        /// Input path or `-` for stdin.
        input: String,
        /// Registered schema name or schema file path.
        #[arg(long)]
        schema: String,
    },
    /// Parse JSON and validate it against a JSON Schema file.
    Json {
        /// Input path or `-` for stdin.
        input: String,
        /// JSON Schema file.
        #[arg(long)]
        schema: PathBuf,
    },
    /// Validate using the normative `validate <path> --schema` form.
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum SerializationFormatArg {
    Json,
    Jsonl,
    Yaml,
    Toml,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum HtmlModeArg {
    Auto,
    Document,
    Fragment,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum XmlDialectArg {
    Auto,
    Xml,
    Jats,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum CsvDelimiterArg {
    Auto,
    Comma,
    Tab,
    Semicolon,
    Pipe,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum RustDetailArg {
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum PythonDetailArg {
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum JavaScriptDialectArg {
    JavaScript,
    Jsx,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum JavaScriptDetailArg {
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum TypeScriptDialectArg {
    TypeScript,
    JavaScript,
    Tsx,
    Jsx,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum LatexDetailArg {
    Semantic,
    SemanticWithSyntax,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum TypeScriptDetailArg {
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum SummaryProfileArg {
    DynamicEventDataset,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum TransformTargetArg {
    Graph,
    Markdown,
    Latex,
    Html,
    Text,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum RenderTargetArg {
    Markdown,
    Latex,
    Html,
    Text,
    Json,
}

#[cfg(feature = "cli")]
#[derive(Debug, Clone, Copy, ValueEnum)]
enum FidelityArg {
    Strict,
    RawFallback,
    Lossy,
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum SchemaCommand {
    /// List public schema names and schema versions.
    List,
    /// Emit one public JSON Schema by name.
    Emit {
        /// Schema name, for example `markdown-envelope`, `document-graph`, or `latex-envelope`.
        name: String,
    },
}

#[cfg(feature = "cli")]
fn main() {
    if let Err(err) = run() {
        let diagnostic = Diagnostic::error("grist.cli", "cli.error", err.to_string());
        println!("{}", serde_json::to_string_pretty(&diagnostic).unwrap());
        std::process::exit(1);
    }
}

#[cfg(feature = "cli")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Detect {
            input,
            filename,
            mime,
            kind,
            request_id,
        } => {
            let report = grist::cli::detect_input(
                &input,
                &input_hints(filename, mime, kind),
                parse_request_id(request_id)?,
            )?;
            print_json(&report)?;
        }
        Command::Parse { command } => match command {
            ParseCommand::Auto {
                input,
                filename,
                mime,
                kind,
                request_id,
            } => {
                let envelope = grist::cli::parse_input(
                    "auto",
                    &input,
                    &input_hints(filename, mime, kind),
                    parse_request_id(request_id)?,
                    None,
                )?;
                print_json(&envelope)?;
            }
            ParseCommand::Text { input } => {
                print_json(&parse_registry(&input, "text", None)?)?;
            }
            ParseCommand::Markdown { input } => {
                print_json(&parse_registry(&input, "markdown", None)?)?;
            }
            ParseCommand::LdgrProjection { input, strict } => {
                let options = grist::ldgr_projection::LdgrProjectionOptions {
                    strict,
                    ..Default::default()
                };
                print_json(&parse_registry(
                    &input,
                    "ldgr_projection",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::Html { input, mode } => {
                let options = grist::html::HtmlOptions {
                    mode: mode.into(),
                    ..Default::default()
                };
                print_json(&parse_registry(
                    &input,
                    "html",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::Xml { input, dialect } => {
                let options = grist::xml::XmlOptions {
                    dialect: dialect.into(),
                    ..Default::default()
                };
                print_json(&parse_registry(
                    &input,
                    "xml",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::Csv {
                input,
                delimiter,
                no_headers,
            } => {
                let options = grist::csv::CsvOptions {
                    delimiter: delimiter.into(),
                    has_headers: !no_headers,
                    ..Default::default()
                };
                print_json(&parse_registry(
                    &input,
                    "csv",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::Xlsx { input } => {
                print_json(&parse_registry(&input, "xlsx", None)?)?;
            }
            ParseCommand::Xlsm { input } => {
                print_json(&parse_registry(&input, "xlsm", None)?)?;
            }
            ParseCommand::Ods { input } => {
                print_json(&parse_registry(&input, "ods", None)?)?;
            }
            ParseCommand::Ots { input } => {
                print_json(&parse_registry(&input, "ots", None)?)?;
            }
            ParseCommand::Rust { input, detail } => {
                let options = grist::rust::RustIngestOptions {
                    detail: detail.into(),
                };
                print_json(&parse_registry(
                    &input,
                    "rust",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::Python { input, detail } => {
                let options = grist::python::PythonIngestOptions {
                    detail: detail.into(),
                };
                print_json(&parse_registry(
                    &input,
                    "python",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::JavaScript {
                input,
                dialect,
                detail,
            } => {
                let options = grist::javascript::JavaScriptIngestOptions {
                    dialect: dialect.into(),
                    detail: detail.into(),
                };
                let format = if options.dialect == grist::javascript::JavaScriptDialect::Jsx {
                    "jsx"
                } else {
                    "javascript"
                };
                print_json(&parse_registry(
                    &input,
                    format,
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::Latex {
                input,
                detail,
                project_roots,
                no_resolve_includes,
            } => {
                let options = grist::latex::LatexOptions {
                    detail: detail.into(),
                    allowed_roots: project_roots,
                    resolve_includes: !no_resolve_includes,
                    ..Default::default()
                };
                print_json(&parse_registry(
                    &input,
                    "latex",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::TypeScript {
                input,
                dialect,
                detail,
            } => {
                let options = grist::typescript::TypeScriptIngestOptions {
                    dialect: dialect.into(),
                    detail: detail.into(),
                };
                print_json(&parse_registry(
                    &input,
                    "typescript",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::Json {
                input,
                format,
                schema,
            } => {
                let parser_format = match format {
                    SerializationFormatArg::Json => "json",
                    SerializationFormatArg::Jsonl => "jsonl",
                    SerializationFormatArg::Yaml => "yaml",
                    SerializationFormatArg::Toml => "toml",
                };
                let options = grist::serialization::SerializationOptions {
                    schema: load_json_value_optional(schema.as_ref())?,
                    ..Default::default()
                };
                print_json(&parse_registry(
                    &input,
                    parser_format,
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::Cbor { input } => {
                print_json(&parse_registry(
                    &input,
                    "cbor",
                    Some(serde_json::to_value(
                        grist::structured_binary::StructuredBinaryOptions::default(),
                    )?),
                )?)?;
            }
            ParseCommand::MessagePack { input } => {
                print_json(&parse_registry(
                    &input,
                    "messagepack",
                    Some(serde_json::to_value(
                        grist::structured_binary::StructuredBinaryOptions::default(),
                    )?),
                )?)?;
            }
            ParseCommand::Protobuf {
                input,
                descriptor,
                message,
            } => {
                let options = grist::structured_binary::StructuredBinaryOptions {
                    protobuf: Some(grist::structured_binary::ProtobufDecodeOptions {
                        descriptor_set: std::fs::read(descriptor)?,
                        message_name: message,
                        preserve_unknown_fields: true,
                    }),
                    ..Default::default()
                };
                print_json(&parse_registry(
                    &input,
                    "protobuf",
                    Some(serde_json::to_value(options)?),
                )?)?;
            }
            ParseCommand::ModelOutput {
                input,
                schema,
                rules,
                strip_think_blocks,
                python_style,
                json_value,
            } => {
                let options = grist::model_output::ModelOutputOptions {
                    schema: load_json_value_optional(schema.as_ref())?,
                    strip_think_blocks,
                    parse_python_style_commands: python_style,
                    aliases: if let Some(rules_path) = rules.as_ref() {
                        load_alias_rules(rules_path)?
                    } else {
                        Default::default()
                    },
                    ..Default::default()
                };
                let report =
                    parse_registry(&input, "model_output", Some(serde_json::to_value(options)?))?;
                if json_value {
                    let payload = report
                        .payload
                        .as_ref()
                        .ok_or("model-output operation produced no payload")?;
                    print_json(selected_model_output_json_value(payload)?)?;
                } else {
                    print_json(&report)?;
                }
            }
            ParseCommand::External(args) => {
                let (format, input, hints, request_id, options) = parse_external_parse(args)?;
                print_json(&grist::cli::parse_input(
                    &format,
                    &input,
                    &hints,
                    parse_request_id(request_id)?,
                    options,
                )?)?;
            }
        },
        Command::Ingest { command } => match command {
            IngestCommand::File {
                input,
                filename,
                mime,
                kind,
            } => {
                if mime.is_some() || kind.is_some() {
                    let envelope = grist::cli::parse_input(
                        "auto",
                        &input,
                        &input_hints(
                            filename.map(|path| path.to_string_lossy().to_string()),
                            mime,
                            kind,
                        ),
                        RequestId::new("request-000000")?,
                        None,
                    )?
                    .with_operation(grist::core::OperationKind::Ingest);
                    print_json(&envelope)?;
                } else if input == "-" {
                    let mut bytes = Vec::new();
                    std::io::stdin().read_to_end(&mut bytes)?;
                    let source = SourceInfo::stdin(
                        filename
                            .as_ref()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|| "stdin".to_string()),
                    );
                    let options = FileIngestOptions {
                        filename_hint: filename,
                        ..Default::default()
                    };
                    print_json(&grist::ingest::ingest_bytes(&bytes, source, &options))?;
                } else {
                    print_json(&grist::ingest::ingest_path(
                        &PathBuf::from(input),
                        &FileIngestOptions::default(),
                    )?)?;
                }
            }
            IngestCommand::Repo {
                path,
                include_ignored,
                include_globs,
                exclude_globs,
                external_artifact_dir,
            } => {
                let options = RepoIngestOptions {
                    include_ignored,
                    include_globs,
                    exclude_globs,
                    inline_artifacts: external_artifact_dir.is_none(),
                    external_artifact_dir,
                    ..Default::default()
                };
                print_json(&grist::ingest::ingest_repo(&path, &options)?)?;
            }
            IngestCommand::Archive {
                input,
                filename,
                mime,
                kind,
                request_id,
                inventory_only,
            } => {
                let hints = input_hints(filename, mime, kind.clone());
                let (bytes, source) = grist::cli::read_input_bytes(&input, &hints)?;
                let container_format = kind
                    .or_else(|| {
                        infer_archive_format(
                            &source.display_name,
                            source.declared_mime_type.as_deref(),
                        )
                    })
                    .ok_or(
                        "archive ingestion requires --kind or a recognized filename extension",
                    )?;
                let ingestor = grist::ingest::Ingestor::builtin()?;
                let decoders = grist::archive::builtin_decoder_registry()?;
                let mode = if inventory_only {
                    grist::container::ContainerArtifactMode::InventoryOnly
                } else {
                    grist::container::ContainerArtifactMode::InlinePayload
                };
                let request = grist::container::ContainerParseRequest::new(
                    parse_request_id(request_id)?,
                    bytes,
                    source,
                    container_format,
                    grist::container::ContainerParseOptions::new(mode),
                    grist::cli::default_budget(),
                );
                let traversal = grist::container::ContainerRecursor::new(&ingestor, &decoders)
                    .parse(request, None)?;
                print_json(&traversal)?;
            }
            IngestCommand::Batch {
                inputs,
                format,
                request_ids,
                collect,
            } => {
                if inputs.iter().filter(|input| input.as_str() == "-").count() > 1 {
                    return Err("batch input may contain stdin at most once".into());
                }
                if !request_ids.is_empty() && request_ids.len() != inputs.len() {
                    return Err("--request-id count must match the number of inputs".into());
                }
                let mut requests = Vec::with_capacity(inputs.len());
                for (sequence, input) in inputs.into_iter().enumerate() {
                    let request_id = request_ids
                        .get(sequence)
                        .cloned()
                        .unwrap_or_else(|| format!("request-{sequence:06}"));
                    let (bytes, source) =
                        grist::cli::read_input_bytes(&input, &grist::cli::InputHints::default())?;
                    let mut request = grist::core::ParseRequest::new(
                        parse_request_id(request_id)?,
                        grist::core::Input::bytes(bytes),
                        source,
                        grist::cli::default_budget(),
                        grist::core::ProviderSet::none(),
                    );
                    if format != "auto" {
                        request = request
                            .with_format_hint(grist::core::FormatHint::exact(format.clone()));
                    }
                    requests.push(request);
                }
                let ingestor = grist::ingest::Ingestor::builtin()?;
                let cancellation = grist::core::CancellationToken::new();
                if collect {
                    print_json(&ingestor.batch(
                        requests,
                        grist::cli::default_budget(),
                        cancellation,
                    )?)?;
                } else {
                    for event in
                        ingestor.stream(requests, grist::cli::default_budget(), cancellation)?
                    {
                        print_json(&event)?;
                    }
                }
            }
        },
        Command::Inspect {
            input,
            filename,
            mime,
            kind,
            request_id,
        } => {
            let envelope = grist::cli::parse_input(
                "auto",
                &input,
                &input_hints(filename, mime, kind),
                parse_request_id(request_id)?,
                None,
            )?
            .with_operation(grist::core::OperationKind::Ingest);
            print_json(&envelope)?;
        }
        Command::Schema { command } => match command {
            SchemaCommand::List => print_json(&grist::schema::list_schemas())?,
            SchemaCommand::Emit { name } => {
                if let Some(schema) = grist::schema::schema_json(&name) {
                    print_json(&schema)?;
                } else {
                    return Err(
                        format!("unknown schema `{name}` or schemas feature disabled").into(),
                    );
                }
            }
        },
        Command::Render { command } => match command {
            RenderCommand::Graph {
                input,
                to,
                fidelity,
                text_output,
                request_id,
            } => render_graph_cli(input, to, fidelity, text_output, request_id)?,
            RenderCommand::JsonSummary {
                input,
                schema,
                profile,
            } => {
                let summary = render_serialization_summary(
                    &input,
                    SerializationFormatArg::Json,
                    schema.as_ref(),
                    profile,
                )?;
                print_json(&summary)?;
            }
            RenderCommand::SerializationSummary {
                input,
                format,
                schema,
                profile,
            } => {
                let summary =
                    render_serialization_summary(&input, format, schema.as_ref(), profile)?;
                print_json(&summary)?;
            }
            RenderCommand::External(args) => {
                let (input, to, fidelity, text_output, request_id) = parse_external_render(args)?;
                render_graph_cli(input, to, fidelity, text_output, request_id)?;
            }
        },
        Command::Validate { command } => match command {
            ValidateCommand::Input { input, schema } => validate_cli(&input, &schema)?,
            ValidateCommand::Json { input, schema } => {
                let (text, source) = read_text_input(&input, None)?;
                print_json(&grist::serialization::parse_serialization_with_options(
                    &text,
                    grist::serialization::SerializationFormat::Json,
                    source,
                    &grist::serialization::SerializationOptions {
                        schema: Some(load_json_value(&schema)?),
                        ..Default::default()
                    },
                ))?;
            }
            ValidateCommand::External(args) => {
                let (input, schema) = parse_external_validate(args)?;
                validate_cli(&input, &schema)?;
            }
        },
        Command::Segment {
            input,
            config,
            graph,
            filename,
            mime,
            kind,
            request_id,
            stream,
        } => {
            let request_id = parse_request_id(request_id)?;
            let hints = input_hints(filename, mime, kind);
            let (graph_value, source_identity, source) = if graph {
                grist::cli::graph_input(&input, &hints)?
            } else {
                parse_source_graph(&input, &hints, request_id.clone())?
            };
            let options: grist::segment::SegmentOptions = load_typed_file(&config)?;
            let document_identity = grist::core::ContentIdentity::default()
                .with_canonical_payload(graph_value.schema_version.as_str(), &graph_value)?;
            let collection = grist::segment::segment_document_graph(
                &graph_value,
                &source_identity,
                &document_identity,
                &options,
                None,
            )?;
            if stream {
                for (sequence, segment) in collection.segments.iter().cloned().enumerate() {
                    print_json(&grist::segment::SegmentEvent::Segment {
                        sequence: u64::try_from(sequence).unwrap_or(u64::MAX),
                        segment: Box::new(segment),
                    })?;
                }
                for diagnostic in &collection.diagnostics {
                    print_json(&grist::segment::SegmentEvent::Diagnostic {
                        diagnostic: Box::new(diagnostic.clone()),
                    })?;
                }
                print_json(&grist::segment::SegmentEvent::Terminal {
                    segment_count: u64::try_from(collection.segments.len()).unwrap_or(u64::MAX),
                    diagnostics: collection.diagnostics.clone(),
                })?;
            } else {
                let envelope = grist::core::Envelope::complete(
                    grist::core::OperationKind::Segment,
                    grist::core::ArtifactKind::SegmentCollection,
                    source,
                    grist::core::ParserInfo::new("grist.segment.structural"),
                    collection.options_digest.clone(),
                    grist::core::SchemaVersion::SEGMENT_COLLECTION_V1,
                    collection,
                )
                .with_identity(source_identity)
                .with_canonical_payload_identity()?;
                print_json(&envelope)?;
            }
        }
        Command::Capabilities => print_json(&grist::cli::capabilities()?)?,
        Command::Transform {
            paths,
            file,
            output,
            manifest,
            fidelity,
            request_id,
            to,
            extract_obligations,
        } => {
            let (input, output_path) = resolve_transform_paths(paths, file, output)?;
            let graph = parse_input_to_document_graph(&input)?;
            let operations = if extract_obligations {
                vec![grist::transform::NormalizedGraphOperation::ExtractConditionalObligations]
            } else {
                vec![grist::transform::NormalizedGraphOperation::Identity]
            };
            let transformed = grist::transform::transform_document_graph(
                &graph,
                &grist::transform::GraphTransformOptions { operations },
            )?;
            let transform_result = transformed
                .payload
                .as_ref()
                .ok_or("complete graph transform did not return a payload")?;
            match to {
                TransformTargetArg::Graph => {
                    if manifest.is_some() {
                        return Err("--manifest is only valid for text transform targets".into());
                    }
                    write_json_output(&transformed, output_path.as_ref())?;
                }
                target => {
                    let format = match target {
                        TransformTargetArg::Markdown => grist::render::RenderFormat::Markdown,
                        TransformTargetArg::Latex => grist::render::RenderFormat::Latex,
                        TransformTargetArg::Html => grist::render::RenderFormat::Html,
                        TransformTargetArg::Text => grist::render::RenderFormat::PlainText,
                        TransformTargetArg::Graph => unreachable!(),
                    };
                    let rendered = grist::render::render_document_graph(
                        &transform_result.graph,
                        format,
                        &grist::render::RenderOptions::new(fidelity.into()),
                    )?;
                    write_text_output(&rendered.content, output_path.as_ref())?;
                    if let Some(manifest_path) = manifest.as_ref() {
                        let destination = output_path.as_ref().map_or(
                            grist::cli::OutputDestination::Stdout,
                            |path| grist::cli::OutputDestination::Path {
                                path: grist::cli::path_label(path),
                            },
                        );
                        let output_manifest = grist::cli::output_manifest(
                            parse_request_id(request_id)?,
                            grist::core::OperationKind::Transform,
                            destination,
                            &rendered,
                            Some(&transform_result.source_map),
                        );
                        write_json_output(&output_manifest, Some(manifest_path))?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "cli")]
fn render_serialization_summary(
    input: &str,
    format: SerializationFormatArg,
    schema: Option<&PathBuf>,
    profile: Option<SummaryProfileArg>,
) -> Result<grist::summary::RenderedSummary, Box<dyn std::error::Error>> {
    let (text, source) = read_text_input(input, None)?;
    let envelope = grist::serialization::parse_serialization_with_options(
        &text,
        format.into(),
        source,
        &grist::serialization::SerializationOptions {
            schema: load_json_value_optional(schema)?,
            ..Default::default()
        },
    );
    let payload = envelope
        .payload
        .as_ref()
        .ok_or("serialization operation produced no payload")?;
    Ok(grist::summary::summarize_serialization_payload(
        payload,
        envelope.diagnostics,
        profile.map(Into::into),
    ))
}

#[cfg(feature = "cli")]
fn parse_input_to_document_graph(
    input: &str,
) -> Result<grist::document_graph::DocumentGraph, Box<dyn std::error::Error>> {
    let extension = if input == "-" {
        String::new()
    } else {
        PathBuf::from(input)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    let (format, options) = match extension.as_str() {
        "txt" | "text" => ("text", None),
        "md" | "markdown" => ("markdown", None),
        "tex" | "latex" => (
            "latex",
            Some(serde_json::to_value(grist::latex::LatexOptions::default())?),
        ),
        "bib" => (
            "bibtex",
            Some(serde_json::to_value(
                grist::bibliography::BibliographyOptions::default(),
            )?),
        ),
        "pdf" => (
            "pdf",
            Some(serde_json::to_value(grist::pdf::PdfOptions::default())?),
        ),
        "odt" | "ott" => (
            extension.as_str(),
            Some(serde_json::to_value(
                grist::odf_word::OdfWordOptions::default(),
            )?),
        ),
        "csv" | "tsv" => (
            "csv",
            Some(serde_json::to_value(grist::csv::CsvOptions {
                delimiter: if extension == "tsv" {
                    grist::csv::CsvDelimiter::Tab
                } else {
                    grist::csv::CsvDelimiter::Auto
                },
                ..Default::default()
            })?),
        ),
        "odp" | "otp" => (
            extension.as_str(),
            Some(serde_json::to_value(
                grist::presentation_odf::OdfPresentationOptions::default(),
            )?),
        ),
        "py" | "pyi" => (
            "python",
            Some(serde_json::to_value(
                grist::python::PythonIngestOptions::default(),
            )?),
        ),
        "xlsx" | "xlsm" => (
            extension.as_str(),
            Some(serde_json::to_value(
                grist::spreadsheet_ooxml::SpreadsheetOoxmlOptions::default(),
            )?),
        ),
        "ods" | "ots" => (
            extension.as_str(),
            Some(serde_json::to_value(
                grist::spreadsheet_odf::SpreadsheetOdfOptions::default(),
            )?),
        ),
        "rs" => (
            "rust",
            Some(serde_json::to_value(
                grist::rust::RustIngestOptions::default(),
            )?),
        ),
        "js" | "mjs" | "cjs" | "jsx" => (
            if extension == "jsx" { "jsx" } else { "javascript" },
            Some(serde_json::to_value(
                grist::javascript::JavaScriptIngestOptions {
                    dialect: if extension == "jsx" {
                        grist::javascript::JavaScriptDialect::Jsx
                    } else {
                        grist::javascript::JavaScriptDialect::JavaScript
                    },
                    ..Default::default()
                },
            )?),
        ),
        "ts" | "mts" | "cts" | "tsx" => (
            if extension == "tsx" { "tsx" } else { "typescript" },
            Some(serde_json::to_value(
                grist::typescript::TypeScriptIngestOptions {
                    dialect: match extension.as_str() {
                        "tsx" => grist::typescript::TypeScriptDialect::Tsx,
                        _ => grist::typescript::TypeScriptDialect::TypeScript,
                    },
                    ..Default::default()
                },
            )?),
        ),
        _ => return Err(format!(
            "cannot infer transform source kind for `{input}`; use a supported extension (.txt, .md, .csv, .tsv, .xlsx, .xlsm, .ods, .ots, .tex, .bib, .pdf, .odt, .ott, .odp, .otp, .py, .rs, .js, .ts, .tsx, .jsx)"
        )
        .into()),
    };
    let envelope = parse_registry(input, format, options)?;
    grist::cli::project_envelope_to_graph(&envelope, format!("graph:{input}"))
}

#[cfg(feature = "cli")]
fn parse_registry(
    input: &str,
    format: &str,
    options: Option<serde_json::Value>,
) -> Result<grist::core::Envelope<serde_json::Value>, Box<dyn std::error::Error>> {
    grist::cli::parse_input(
        format,
        input,
        &grist::cli::InputHints::default(),
        RequestId::new("request-000000")?,
        options,
    )
}

#[cfg(feature = "cli")]
fn input_hints(
    filename: Option<String>,
    media_type: Option<String>,
    kind: Option<String>,
) -> grist::cli::InputHints {
    grist::cli::InputHints {
        filename,
        media_type,
        kind,
    }
}

#[cfg(feature = "cli")]
fn parse_request_id(value: String) -> Result<RequestId, Box<dyn std::error::Error>> {
    Ok(RequestId::new(value)?)
}

#[cfg(feature = "cli")]
fn infer_archive_format(display_name: &str, media_type: Option<&str>) -> Option<String> {
    let from_media_type = match media_type.map(|value| value.split(';').next().unwrap_or(value)) {
        Some("application/zip") => Some("zip"),
        Some("application/x-tar") => Some("tar"),
        Some("application/gzip" | "application/x-gzip") => Some("gzip"),
        Some("application/x-bzip2") => Some("bzip2"),
        Some("application/x-xz") => Some("xz"),
        Some("application/zstd") => Some("zstd"),
        Some("application/x-7z-compressed") => Some("7z"),
        _ => None,
    };
    if let Some(format) = from_media_type {
        return Some(format.to_string());
    }
    let lower = display_name.to_ascii_lowercase();
    [
        (".tar.gz", "gzip"),
        (".tgz", "gzip"),
        (".tar.bz2", "bzip2"),
        (".tar.xz", "xz"),
        (".tar.zst", "zstd"),
        (".zip", "zip"),
        (".tar", "tar"),
        (".gz", "gzip"),
        (".bz2", "bzip2"),
        (".xz", "xz"),
        (".zst", "zstd"),
        (".7z", "7z"),
    ]
    .into_iter()
    .find_map(|(suffix, format)| lower.ends_with(suffix).then(|| format.to_string()))
}

#[cfg(feature = "cli")]
fn parse_source_graph(
    input: &str,
    hints: &grist::cli::InputHints,
    request_id: RequestId,
) -> Result<
    (
        grist::document_graph::DocumentGraph,
        grist::core::ContentIdentity,
        SourceInfo,
    ),
    Box<dyn std::error::Error>,
> {
    let envelope = grist::cli::parse_input("auto", input, hints, request_id, None)?;
    let identity = envelope
        .identity
        .clone()
        .ok_or("parse operation did not retain source identity")?;
    let source = envelope.source.clone();
    let graph = grist::cli::project_envelope_to_graph(&envelope, format!("graph:{input}"))?;
    Ok((graph, identity, source))
}

#[cfg(feature = "cli")]
fn load_typed_file<T: serde::de::DeserializeOwned>(
    path: &PathBuf,
) -> Result<T, Box<dyn std::error::Error>> {
    Ok(serde_json::from_value(load_json_value(path)?)?)
}

#[cfg(feature = "cli")]
fn render_graph_cli(
    input: String,
    to: RenderTargetArg,
    fidelity: FidelityArg,
    text_output: Option<PathBuf>,
    request_id: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let (graph, _, _) = grist::cli::graph_input(&input, &grist::cli::InputHints::default())?;
    let result = grist::render::render_document_graph(
        &graph,
        to.into(),
        &grist::render::RenderOptions::new(fidelity.into()),
    )?;
    if let Some(path) = text_output {
        std::fs::write(&path, &result.content)?;
        let manifest = grist::cli::output_manifest(
            parse_request_id(request_id)?,
            grist::core::OperationKind::Render,
            grist::cli::OutputDestination::Path {
                path: grist::cli::path_label(&path),
            },
            &result,
            None,
        );
        print_json(&manifest)?;
    } else {
        print_json(&result)?;
    }
    Ok(())
}

#[cfg(feature = "cli")]
type ExternalParseArgs = (
    String,
    String,
    grist::cli::InputHints,
    String,
    Option<serde_json::Value>,
);

#[cfg(feature = "cli")]
fn parse_external_parse(
    args: Vec<String>,
) -> Result<ExternalParseArgs, Box<dyn std::error::Error>> {
    let mut values = args.into_iter();
    let format = values.next().ok_or("parse requires a format")?;
    let input = values.next().ok_or("parse requires an input")?;
    let mut hints = grist::cli::InputHints::default();
    let mut request_id = "request-000000".to_string();
    let mut options = None;
    while let Some(argument) = values.next() {
        let value = values
            .next()
            .ok_or_else(|| format!("{argument} requires a value"))?;
        match argument.as_str() {
            "--filename" => hints.filename = Some(value),
            "--mime" => hints.media_type = Some(value),
            "--kind" => hints.kind = Some(value),
            "--request-id" => request_id = value,
            "--options" => options = Some(load_json_value(&PathBuf::from(value))?),
            _ => return Err(format!("unknown parse option `{argument}`").into()),
        }
    }
    Ok((format, input, hints, request_id, options))
}

#[cfg(feature = "cli")]
type ExternalRenderArgs = (
    String,
    RenderTargetArg,
    FidelityArg,
    Option<PathBuf>,
    String,
);

#[cfg(feature = "cli")]
fn parse_external_render(
    args: Vec<String>,
) -> Result<ExternalRenderArgs, Box<dyn std::error::Error>> {
    let mut values = args.into_iter();
    let input = values.next().ok_or("render requires a graph input")?;
    let mut to = None;
    let mut fidelity = FidelityArg::Strict;
    let mut text_output = None;
    let mut request_id = "request-000000".to_string();
    while let Some(argument) = values.next() {
        let value = values
            .next()
            .ok_or_else(|| format!("{argument} requires a value"))?;
        match argument.as_str() {
            "--to" => {
                to = Some(match value.as_str() {
                    "markdown" => RenderTargetArg::Markdown,
                    "latex" => RenderTargetArg::Latex,
                    "html" => RenderTargetArg::Html,
                    "text" => RenderTargetArg::Text,
                    "json" => RenderTargetArg::Json,
                    _ => return Err(format!("unknown render target `{value}`").into()),
                });
            }
            "--fidelity" => {
                fidelity = match value.as_str() {
                    "strict" => FidelityArg::Strict,
                    "raw-fallback" => FidelityArg::RawFallback,
                    "lossy" => FidelityArg::Lossy,
                    _ => return Err(format!("unknown fidelity mode `{value}`").into()),
                };
            }
            "--text-output" => text_output = Some(PathBuf::from(value)),
            "--request-id" => request_id = value,
            _ => return Err(format!("unknown render option `{argument}`").into()),
        }
    }
    Ok((
        input,
        to.ok_or("render requires --to")?,
        fidelity,
        text_output,
        request_id,
    ))
}

#[cfg(feature = "cli")]
fn validate_cli(input: &str, schema: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (bytes, source) = grist::cli::read_input_bytes(input, &grist::cli::InputHints::default())?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let schema_path = PathBuf::from(schema);
    let report = if schema_path.is_file() {
        let schema_value = load_json_value(&schema_path)?;
        let version = schema_value
            .get("$id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("external");
        grist::schema::validate_against_schema(schema, version, &value, &schema_value)?
    } else {
        grist::schema::validate_schema(schema, &value)?
    };
    let options_digest = grist::core::options_digest(&serde_json::json!({"schema": schema}))?;
    let envelope = grist::core::Envelope::complete(
        grist::core::OperationKind::Validate,
        grist::core::ArtifactKind::SchemaValidation,
        source,
        grist::core::ParserInfo::new("grist.schema.validate"),
        options_digest,
        grist::core::SchemaVersion::SCHEMA_VALIDATION_V1,
        report,
    )
    .with_identity(grist::core::ContentIdentity::for_raw_bytes(&bytes))
    .with_canonical_payload_identity()?;
    print_json(&envelope)?;
    Ok(())
}

#[cfg(feature = "cli")]
fn parse_external_validate(
    args: Vec<String>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let mut values = args.into_iter();
    let input = values.next().ok_or("validate requires an input")?;
    let flag = values.next().ok_or("validate requires --schema")?;
    if flag != "--schema" {
        return Err(format!("unknown validate option `{flag}`").into());
    }
    let schema = values.next().ok_or("--schema requires a value")?;
    if let Some(extra) = values.next() {
        return Err(format!("unexpected validate argument `{extra}`").into());
    }
    Ok((input, schema))
}

#[cfg(feature = "cli")]
fn read_text_input(
    input: &str,
    filename: Option<PathBuf>,
) -> Result<(String, SourceInfo), Box<dyn std::error::Error>> {
    if input == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        Ok((
            text,
            SourceInfo::stdin(
                filename
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "stdin".into()),
            ),
        ))
    } else {
        let path = PathBuf::from(input);
        Ok((
            std::fs::read_to_string(&path)?,
            SourceInfo::from_path(&path),
        ))
    }
}

#[cfg(feature = "cli")]
impl From<SummaryProfileArg> for grist::summary::SummaryProfile {
    fn from(value: SummaryProfileArg) -> Self {
        match value {
            SummaryProfileArg::DynamicEventDataset => Self::DynamicEventDataset,
        }
    }
}

#[cfg(feature = "cli")]
impl From<RenderTargetArg> for grist::render::RenderFormat {
    fn from(value: RenderTargetArg) -> Self {
        match value {
            RenderTargetArg::Markdown => Self::Markdown,
            RenderTargetArg::Latex => Self::Latex,
            RenderTargetArg::Html => Self::Html,
            RenderTargetArg::Text => Self::PlainText,
            RenderTargetArg::Json => Self::CanonicalJson,
        }
    }
}

#[cfg(feature = "cli")]
impl From<FidelityArg> for grist::render::FidelityMode {
    fn from(value: FidelityArg) -> Self {
        match value {
            FidelityArg::Strict => Self::Strict,
            FidelityArg::RawFallback => Self::RawFallback,
            FidelityArg::Lossy => Self::Lossy,
        }
    }
}

#[cfg(feature = "cli")]
impl From<SerializationFormatArg> for grist::serialization::SerializationFormat {
    fn from(value: SerializationFormatArg) -> Self {
        match value {
            SerializationFormatArg::Json => Self::Json,
            SerializationFormatArg::Jsonl => Self::Jsonl,
            SerializationFormatArg::Yaml => Self::Yaml,
            SerializationFormatArg::Toml => Self::Toml,
        }
    }
}

#[cfg(feature = "cli")]
impl From<XmlDialectArg> for grist::xml::XmlDialect {
    fn from(value: XmlDialectArg) -> Self {
        match value {
            XmlDialectArg::Auto => Self::Auto,
            XmlDialectArg::Xml => Self::Xml,
            XmlDialectArg::Jats => Self::Jats,
        }
    }
}

#[cfg(feature = "cli")]
impl From<HtmlModeArg> for grist::html::HtmlParseMode {
    fn from(value: HtmlModeArg) -> Self {
        match value {
            HtmlModeArg::Auto => Self::Auto,
            HtmlModeArg::Document => Self::Document,
            HtmlModeArg::Fragment => Self::Fragment,
        }
    }
}

#[cfg(feature = "cli")]
impl From<CsvDelimiterArg> for grist::csv::CsvDelimiter {
    fn from(value: CsvDelimiterArg) -> Self {
        match value {
            CsvDelimiterArg::Auto => Self::Auto,
            CsvDelimiterArg::Comma => Self::Comma,
            CsvDelimiterArg::Tab => Self::Tab,
            CsvDelimiterArg::Semicolon => Self::Semicolon,
            CsvDelimiterArg::Pipe => Self::Pipe,
        }
    }
}

#[cfg(feature = "cli")]
impl From<PythonDetailArg> for grist::python::PythonDetailMode {
    fn from(value: PythonDetailArg) -> Self {
        match value {
            PythonDetailArg::Semantic => Self::Semantic,
            PythonDetailArg::SemanticWithSyntax => Self::SemanticWithSyntax,
            PythonDetailArg::SyntaxDebug => Self::SyntaxDebug,
        }
    }
}

impl From<RustDetailArg> for grist::rust::RustDetailMode {
    fn from(value: RustDetailArg) -> Self {
        match value {
            RustDetailArg::Semantic => Self::Semantic,
            RustDetailArg::SemanticWithSyntax => Self::SemanticWithSyntax,
            RustDetailArg::SyntaxDebug => Self::SyntaxDebug,
        }
    }
}

#[cfg(feature = "cli")]
impl From<LatexDetailArg> for grist::latex::LatexDetailMode {
    fn from(value: LatexDetailArg) -> Self {
        match value {
            LatexDetailArg::Semantic => Self::Semantic,
            LatexDetailArg::SemanticWithSyntax => Self::SemanticWithSyntax,
        }
    }
}

#[cfg(feature = "cli")]
impl From<TypeScriptDialectArg> for grist::typescript::TypeScriptDialect {
    fn from(value: TypeScriptDialectArg) -> Self {
        match value {
            TypeScriptDialectArg::TypeScript => Self::TypeScript,
            TypeScriptDialectArg::JavaScript => Self::JavaScript,
            TypeScriptDialectArg::Tsx => Self::Tsx,
            TypeScriptDialectArg::Jsx => Self::Jsx,
        }
    }
}

#[cfg(feature = "cli")]
impl From<JavaScriptDialectArg> for grist::javascript::JavaScriptDialect {
    fn from(value: JavaScriptDialectArg) -> Self {
        match value {
            JavaScriptDialectArg::JavaScript => Self::JavaScript,
            JavaScriptDialectArg::Jsx => Self::Jsx,
        }
    }
}

#[cfg(feature = "cli")]
impl From<JavaScriptDetailArg> for grist::javascript::JavaScriptDetailMode {
    fn from(value: JavaScriptDetailArg) -> Self {
        match value {
            JavaScriptDetailArg::Semantic => Self::Semantic,
            JavaScriptDetailArg::SemanticWithSyntax => Self::SemanticWithSyntax,
            JavaScriptDetailArg::SyntaxDebug => Self::SyntaxDebug,
        }
    }
}

#[cfg(feature = "cli")]
impl From<TypeScriptDetailArg> for grist::typescript::TypeScriptDetailMode {
    fn from(value: TypeScriptDetailArg) -> Self {
        match value {
            TypeScriptDetailArg::Semantic => Self::Semantic,
            TypeScriptDetailArg::SemanticWithSyntax => Self::SemanticWithSyntax,
            TypeScriptDetailArg::SyntaxDebug => Self::SyntaxDebug,
        }
    }
}

#[cfg(feature = "cli")]
fn resolve_transform_paths(
    paths: Vec<String>,
    file: Option<String>,
    output: Option<PathBuf>,
) -> Result<(String, Option<PathBuf>), Box<dyn std::error::Error>> {
    let (input, positional_output) = if let Some(file) = file {
        if paths.len() > 1 {
            return Err("with --file, provide at most one positional output path".into());
        }
        (file, paths.into_iter().next())
    } else {
        let mut paths = paths.into_iter();
        let input = paths
            .next()
            .ok_or_else(|| "transform requires an input path".to_string())?;
        (input, paths.next())
    };

    if output.is_some() && positional_output.is_some() {
        return Err("provide output either as a positional path or with --output, not both".into());
    }

    Ok((
        input,
        output.or_else(|| positional_output.map(PathBuf::from)),
    ))
}

#[cfg(feature = "cli")]
fn write_text_output(
    text: &str,
    output: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(path) = output {
        std::fs::write(path, text)?;
    } else {
        print!("{text}");
    }
    Ok(())
}

#[cfg(feature = "cli")]
fn write_json_output<T: serde::Serialize>(
    value: &T,
    output: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string(value)?;
    if let Some(path) = output {
        std::fs::write(path, format!("{json}\n"))?;
    } else {
        println!("{json}");
    }
    Ok(())
}

#[cfg(feature = "cli")]
fn print_json<T: serde::Serialize>(value: &T) -> Result<(), serde_json::Error> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

#[cfg(feature = "cli")]
fn load_json_value(path: &PathBuf) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let value = match path.extension().and_then(|ext| ext.to_str()) {
        Some("yaml" | "yml") => {
            serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(&text)?)?
        }
        Some("toml") => serde_json::to_value(text.parse::<toml::Value>()?)?,
        _ => serde_json::from_str(&text)?,
    };
    Ok(value)
}

#[cfg(feature = "cli")]
fn load_json_value_optional(
    path: Option<&PathBuf>,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    path.map(load_json_value).transpose()
}

#[cfg(feature = "cli")]
fn selected_model_output_json_value(
    payload: &serde_json::Value,
) -> Result<&serde_json::Value, String> {
    let candidates = payload
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "model-output payload has no candidate list".to_string())?;
    let selected = match payload
        .get("selected_candidate_id")
        .and_then(serde_json::Value::as_str)
    {
        Some(selected) => selected,
        None if candidates.is_empty() => {
            return Err("no model-output candidate was detected".to_string());
        }
        None if payload.get("status").and_then(serde_json::Value::as_str) == Some("ambiguous") => {
            return Err(format!(
                "ambiguous model-output candidates: {} candidates were retained and none was selected",
                candidates.len()
            ));
        }
        None => return Err("no model-output candidate was selected".to_string()),
    };
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.get("id").and_then(serde_json::Value::as_str) == Some(selected))
        .ok_or_else(|| format!("selected model-output candidate {selected} was not retained"))?;
    let status = candidate
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if matches!(status, "incomplete" | "malformed") {
        let position = candidate.get("json_error");
        let suffix = position
            .map(|position| {
                format!(
                    " at byte {} (line {}, column {}): {}",
                    position
                        .get("byte_offset")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default(),
                    position
                        .get("line")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default(),
                    position
                        .get("column")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default(),
                    position
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("JSON parse failed")
                )
            })
            .unwrap_or_default();
        return Err(format!(
            "selected model-output candidate {selected} is {status}{suffix}"
        ));
    }
    candidate
        .get("value")
        .ok_or_else(|| format!("selected model-output candidate {selected} has no JSON value"))
}

#[cfg(feature = "cli")]
fn load_alias_rules(
    path: &PathBuf,
) -> Result<grist::model_output::AliasRules, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let rules = match path.extension().and_then(|ext| ext.to_str()) {
        Some("toml") => toml::from_str(&text)?,
        Some("yaml" | "yml") => serde_yaml::from_str(&text)?,
        _ => serde_json::from_str(&text)?,
    };
    Ok(rules)
}

#[cfg(not(feature = "cli"))]
fn main() {
    eprintln!("grist CLI requires the `cli` feature");
    std::process::exit(1);
}
