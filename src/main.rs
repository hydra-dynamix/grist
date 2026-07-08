#[cfg(feature = "cli")]
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(feature = "cli")]
use grist::core::{Diagnostic, SourceInfo};
#[cfg(feature = "cli")]
use grist::document_graph::ToDocumentGraph;
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
    /// Parse CSV data.
    Csv {
        /// Input path or `-` for stdin.
        input: String,
        /// CSV delimiter.
        #[arg(long, value_enum, default_value_t = CsvDelimiterArg::Comma)]
        delimiter: CsvDelimiterArg,
        /// Treat the first row as data instead of headers.
        #[arg(long)]
        no_headers: bool,
    },
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
    /// Parse LaTeX documents, preserving unknown commands as raw nodes.
    Latex {
        /// Input path or `-` for stdin.
        input: String,
        /// Semantic/syntax detail level.
        #[arg(long, value_enum, default_value_t = LatexDetailArg::Semantic)]
        detail: LatexDetailArg,
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
        /// Reserved explicit kind override.
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
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum RenderCommand {
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
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum ValidateCommand {
    /// Parse JSON and validate it against a JSON Schema file.
    Json {
        /// Input path or `-` for stdin.
        input: String,
        /// JSON Schema file.
        #[arg(long)]
        schema: PathBuf,
    },
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
enum CsvDelimiterArg {
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
enum TypeScriptDialectArg {
    TypeScript,
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
        Command::Parse { command } => match command {
            ParseCommand::Markdown { input } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "markdown")]
                print_json(&grist::markdown::parse_markdown(&text, source))?;
                #[cfg(not(feature = "markdown"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "markdown feature is disabled",
                ))?;
            }
            ParseCommand::LdgrProjection { input, strict } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "ldgr-projection")]
                print_json(&grist::ldgr_projection::parse_ldgr_projection(
                    &text,
                    source,
                    grist::ldgr_projection::LdgrProjectionOptions {
                        strict,
                        ..Default::default()
                    },
                ))?;
                #[cfg(not(feature = "ldgr-projection"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "ldgr-projection feature is disabled",
                ))?;
            }
            ParseCommand::Html { input, mode } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "html")]
                print_json(&grist::html::parse_html(
                    &text,
                    source,
                    &grist::html::HtmlOptions { mode: mode.into() },
                ))?;
                #[cfg(not(feature = "html"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "html feature is disabled",
                ))?;
            }
            ParseCommand::Csv {
                input,
                delimiter,
                no_headers,
            } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "csv")]
                print_json(&grist::csv::parse_csv(
                    &text,
                    source,
                    &grist::csv::CsvOptions {
                        delimiter: delimiter.into(),
                        has_headers: !no_headers,
                    },
                ))?;
                #[cfg(not(feature = "csv"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "csv feature is disabled",
                ))?;
            }
            ParseCommand::Rust { input, detail } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "rust")]
                print_json(&grist::rust::parse_rust(
                    &text,
                    source,
                    &grist::rust::RustIngestOptions {
                        detail: detail.into(),
                    },
                ))?;
                #[cfg(not(feature = "rust"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "rust feature is disabled",
                ))?;
            }
            ParseCommand::Python { input, detail } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "python")]
                print_json(&grist::python::parse_python(
                    &text,
                    source,
                    &grist::python::PythonIngestOptions {
                        detail: detail.into(),
                    },
                ))?;
                #[cfg(not(feature = "python"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "python feature is disabled",
                ))?;
            }
            ParseCommand::Latex { input, detail } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "latex")]
                print_json(&grist::latex::parse_latex(
                    &text,
                    source,
                    &grist::latex::LatexOptions {
                        detail: detail.into(),
                    },
                ))?;
                #[cfg(not(feature = "latex"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "latex feature is disabled",
                ))?;
            }
            ParseCommand::TypeScript {
                input,
                dialect,
                detail,
            } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "typescript")]
                print_json(&grist::typescript::parse_typescript(
                    &text,
                    source,
                    &grist::typescript::TypeScriptIngestOptions {
                        dialect: dialect.into(),
                        detail: detail.into(),
                    },
                ))?;
                #[cfg(not(feature = "typescript"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "typescript feature is disabled",
                ))?;
            }
            ParseCommand::Json {
                input,
                format,
                schema,
            } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "serialization")]
                print_json(&grist::serialization::parse_serialization_with_options(
                    &text,
                    format.into(),
                    source,
                    &grist::serialization::SerializationOptions {
                        schema: load_json_value_optional(schema.as_ref())?,
                    },
                ))?;
                #[cfg(not(feature = "serialization"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "serialization feature is disabled",
                ))?;
            }
            ParseCommand::ModelOutput {
                input,
                schema,
                rules,
                strip_think_blocks,
                python_style,
                json_value,
            } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "model-output")]
                {
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
                    let report = grist::model_output::parse_model_output(&text, source, &options);
                    if json_value {
                        let value = report
                            .payload
                            .selected_candidate_id
                            .as_ref()
                            .and_then(|selected_id| {
                                report
                                    .payload
                                    .candidates
                                    .iter()
                                    .find(|candidate| candidate.id == *selected_id)
                            })
                            .and_then(|candidate| candidate.value.as_ref())
                            .ok_or_else(|| "no selected model-output JSON value".to_string())?;
                        print_json(value)?;
                    } else {
                        print_json(&report)?;
                    }
                }
                #[cfg(not(feature = "model-output"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "model-output feature is disabled",
                ))?;
            }
        },
        Command::Ingest { command } => match command {
            IngestCommand::File {
                input,
                filename,
                kind: _,
            } => {
                if input == "-" {
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
        },
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
        },
        Command::Validate { command } => match command {
            ValidateCommand::Json { input, schema } => {
                let (text, source) = read_text_input(&input, None)?;
                print_json(&grist::serialization::parse_serialization_with_options(
                    &text,
                    grist::serialization::SerializationFormat::Json,
                    source,
                    &grist::serialization::SerializationOptions {
                        schema: Some(load_json_value(&schema)?),
                    },
                ))?;
            }
        },
        Command::Transform {
            paths,
            file,
            output,
            to,
            extract_obligations,
        } => {
            let (input, output_path) = resolve_transform_paths(paths, file, output)?;
            let mut graph = parse_input_to_document_graph(&input)?;
            if extract_obligations {
                grist::document_graph::extract_conditional_obligations(&mut graph);
            }
            match to {
                TransformTargetArg::Graph => write_json_output(&graph, output_path.as_ref())?,
                TransformTargetArg::Markdown => {
                    let rendered = grist::document_graph::render_markdown(
                        &graph,
                        grist::document_graph::TransformOptions::default(),
                    )?;
                    write_text_output(&rendered, output_path.as_ref())?;
                }
                TransformTargetArg::Latex => {
                    let rendered = grist::document_graph::render_latex(
                        &graph,
                        grist::document_graph::TransformOptions::default(),
                    )?;
                    write_text_output(&rendered, output_path.as_ref())?;
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
        },
    );
    Ok(grist::summary::summarize_serialization_payload(
        &envelope.payload,
        envelope.diagnostics,
        profile.map(Into::into),
    ))
}

#[cfg(feature = "cli")]
fn parse_input_to_document_graph(
    input: &str,
) -> Result<grist::document_graph::DocumentGraph, Box<dyn std::error::Error>> {
    let (text, source) = read_text_input(input, None)?;
    let extension = if input == "-" {
        String::new()
    } else {
        PathBuf::from(input)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    let context = grist::document_graph::DocumentGraphContext::new(format!("graph:{input}"));
    match extension.as_str() {
        "md" | "markdown" => Ok(grist::markdown::parse_markdown(&text, source)
            .payload
            .to_document_graph(context)?),
        "tex" | "latex" => Ok(grist::latex::parse_latex(
            &text,
            source,
            &grist::latex::LatexOptions::default(),
        )
        .payload
        .to_document_graph(context)?),
        "py" | "pyi" => Ok(grist::python::parse_python(
            &text,
            source,
            &grist::python::PythonIngestOptions::default(),
        )
        .payload
        .to_document_graph(context)?),
        "rs" => Ok(grist::rust::parse_rust(
            &text,
            source,
            &grist::rust::RustIngestOptions::default(),
        )
        .payload
        .to_document_graph(context)?),
        "ts" | "mts" | "cts" | "tsx" | "jsx" => Ok(grist::typescript::parse_typescript(
            &text,
            source,
            &grist::typescript::TypeScriptIngestOptions {
                dialect: match extension.as_str() {
                    "tsx" => grist::typescript::TypeScriptDialect::Tsx,
                    "jsx" => grist::typescript::TypeScriptDialect::Jsx,
                    _ => grist::typescript::TypeScriptDialect::TypeScript,
                },
                ..Default::default()
            },
        )
        .payload
        .to_document_graph(context)?),
        _ => Err(format!(
            "cannot infer transform source kind for `{input}`; use a supported extension (.md, .tex, .py, .rs, .ts, .tsx, .jsx)"
        )
        .into()),
    }
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
            TypeScriptDialectArg::Tsx => Self::Tsx,
            TypeScriptDialectArg::Jsx => Self::Jsx,
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
