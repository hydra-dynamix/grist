#[cfg(feature = "cli")]
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(feature = "cli")]
use grist::core::{Diagnostic, SourceInfo};
#[cfg(feature = "cli")]
use grist::ingest::{FileIngestOptions, RepoIngestOptions};
#[cfg(feature = "cli")]
use std::io::Read;
#[cfg(feature = "cli")]
use std::path::PathBuf;

#[cfg(feature = "cli")]
#[derive(Parser)]
#[command(name = "grist", about = "Grist interpretation utility")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum Command {
    Parse {
        #[command(subcommand)]
        command: ParseCommand,
    },
    Ingest {
        #[command(subcommand)]
        command: IngestCommand,
    },
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum ParseCommand {
    Markdown {
        input: String,
    },
    Html {
        input: String,
        #[arg(long, value_enum, default_value_t = HtmlModeArg::Auto)]
        mode: HtmlModeArg,
    },
    Csv {
        input: String,
        #[arg(long, value_enum, default_value_t = CsvDelimiterArg::Comma)]
        delimiter: CsvDelimiterArg,
        #[arg(long)]
        no_headers: bool,
    },
    Rust {
        input: String,
        #[arg(long, value_enum, default_value_t = RustDetailArg::Semantic)]
        detail: RustDetailArg,
    },
    Python {
        input: String,
        #[arg(long, value_enum, default_value_t = PythonDetailArg::Semantic)]
        detail: PythonDetailArg,
    },
    #[command(name = "typescript", alias = "ts")]
    TypeScript {
        input: String,
        #[arg(long, value_enum, default_value_t = TypeScriptDialectArg::TypeScript)]
        dialect: TypeScriptDialectArg,
        #[arg(long, value_enum, default_value_t = TypeScriptDetailArg::Semantic)]
        detail: TypeScriptDetailArg,
    },
    Json {
        input: String,
        #[arg(long, value_enum, default_value_t = SerializationFormatArg::Json)]
        format: SerializationFormatArg,
        #[arg(long)]
        schema: Option<PathBuf>,
    },
    ModelOutput {
        input: String,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        rules: Option<PathBuf>,
        #[arg(long)]
        strip_think_blocks: bool,
    },
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum IngestCommand {
    File {
        input: String,
        #[arg(long)]
        filename: Option<PathBuf>,
        #[arg(long)]
        kind: Option<String>,
    },
    Repo {
        path: PathBuf,
        #[arg(long)]
        include_ignored: bool,
        #[arg(long = "include")]
        include_globs: Vec<String>,
        #[arg(long = "exclude")]
        exclude_globs: Vec<String>,
        #[arg(long)]
        external_artifact_dir: Option<PathBuf>,
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
enum TypeScriptDetailArg {
    Semantic,
    SemanticWithSyntax,
    SyntaxDebug,
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
enum SchemaCommand {
    List,
    Emit { name: String },
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
            } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "model-output")]
                {
                    let options = grist::model_output::ModelOutputOptions {
                        schema: load_json_value_optional(schema.as_ref())?,
                        strip_think_blocks,
                        aliases: if let Some(rules_path) = rules.as_ref() {
                            load_alias_rules(rules_path)?
                        } else {
                            Default::default()
                        },
                        ..Default::default()
                    };
                    print_json(&grist::model_output::parse_model_output(
                        &text, source, &options,
                    ))?;
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
    }
    Ok(())
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
fn print_json<T: serde::Serialize>(value: &T) -> Result<(), serde_json::Error> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

#[cfg(feature = "cli")]
fn load_json_value_optional(
    path: Option<&PathBuf>,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    path.map(|path| {
        let text = std::fs::read_to_string(path)?;
        let value = match path.extension().and_then(|ext| ext.to_str()) {
            Some("yaml" | "yml") => {
                serde_json::to_value(serde_yaml::from_str::<serde_yaml::Value>(&text)?)?
            }
            Some("toml") => serde_json::to_value(text.parse::<toml::Value>()?)?,
            _ => serde_json::from_str(&text)?,
        };
        Ok(value)
    })
    .transpose()
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
