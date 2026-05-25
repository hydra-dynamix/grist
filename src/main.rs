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
    Markdown { input: String },
    Rust { input: String },
    Json { input: String },
    ModelOutput { input: String },
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
            ParseCommand::Json { input } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "serialization")]
                print_json(&grist::serialization::parse_serialization(
                    &text,
                    grist::serialization::SerializationFormat::Json,
                    source,
                ))?;
                #[cfg(not(feature = "serialization"))]
                print_json(&Diagnostic::error(
                    "grist.cli",
                    "feature.disabled",
                    "serialization feature is disabled",
                ))?;
            }
            ParseCommand::ModelOutput { input } => {
                let (text, source) = read_text_input(&input, None)?;
                #[cfg(feature = "model-output")]
                print_json(&grist::model_output::parse_model_output(
                    &text,
                    source,
                    &grist::model_output::ModelOutputOptions::default(),
                ))?;
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

#[cfg(not(feature = "cli"))]
fn main() {
    eprintln!("grist CLI requires the `cli` feature");
    std::process::exit(1);
}
