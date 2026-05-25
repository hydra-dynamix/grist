#[cfg(feature = "cli")]
use clap::{Parser, Subcommand};
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
    Rust {
        input: String,
    },
    Json {
        input: String,
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
    },
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
            ParseCommand::Rust { input } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "rust")]
                print_json(&grist::rust::parse_rust(
                    &text,
                    source,
                    &grist::rust::RustIngestOptions::default(),
                ))?;
                #[cfg(not(feature = "rust"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "rust feature is disabled",
                ))?;
            }
            ParseCommand::Json { input, schema } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "serialization")]
                print_json(&grist::serialization::parse_serialization_with_options(
                    &text,
                    grist::serialization::SerializationFormat::Json,
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
                    let mut options = grist::model_output::ModelOutputOptions::default();
                    options.schema = load_json_value_optional(schema.as_ref())?;
                    options.strip_think_blocks = strip_think_blocks;
                    if let Some(rules_path) = rules.as_ref() {
                        options.aliases = load_alias_rules(rules_path)?;
                    }
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
            } => {
                let options = RepoIngestOptions {
                    include_ignored,
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
