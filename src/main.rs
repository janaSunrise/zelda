mod binder;
mod checker;
mod errors;
mod lib_dts;
mod project;
mod resolver;
mod symbols;
mod tsconfig;
mod types;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand, ValueEnum};
use miette::{Diagnostic, NamedSource, Result, SourceSpan};
use owo_colors::OwoColorize;
use serde::Serialize;
use thiserror::Error;

use crate::binder::BindingError;
use crate::project::{Project, ProjectError};

#[derive(Parser)]
#[command(name = "zelda")]
#[command(about = "Fast TypeScript type checker", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Clone, Default, Debug)]
enum OutputFormat {
    #[default]
    Pretty,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    Check {
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Output format (pretty or json)
        #[arg(long, value_enum, default_value = "pretty")]
        format: OutputFormat,
    },
}

#[derive(Error, Debug, Diagnostic)]
#[error("{message}")]
struct ZeldaError {
    message: String,
    #[source_code]
    src: NamedSource<Arc<String>>,
    #[label("{label}")]
    span: SourceSpan,
    label: String,
    #[help]
    help: Option<String>,
}

/// JSON output structure for a single file check result.
#[derive(Debug, Serialize)]
struct JsonFileResult {
    file: String,
    errors: Vec<ProjectError>,
    error_count: usize,
    warning_count: usize,
    duration_ms: u64,
}

/// JSON output structure for the complete check result.
#[derive(Debug, Serialize)]
struct JsonOutput {
    files: Vec<JsonFileResult>,
    total_errors: usize,
    total_warnings: usize,
    total_duration_ms: u64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Check { files, format } => {
            let total_errors = check_files(&files, &format)?;
            if total_errors > 0 {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

/// Check multiple files using the Project infrastructure for multi-file support.
fn check_files(files: &[PathBuf], format: &OutputFormat) -> Result<usize> {
    let total_start = Instant::now();

    // Use the first file's directory to find tsconfig.json
    let project_root = files.first().and_then(|f| f.parent()).unwrap_or(std::path::Path::new("."));
    let mut project = Project::with_tsconfig(project_root);
    let mut sources: HashMap<PathBuf, Arc<String>> = HashMap::new();
    let mut total_errors = 0;
    let mut total_warnings = 0;
    let mut json_results: Vec<JsonFileResult> = Vec::new();

    for file in files {
        let file_start = Instant::now();

        // Check the file and its dependencies
        let errors = match project.check_file(file) {
            Ok(errs) => errs,
            Err(ProjectError::FileReadError { path, error }) => {
                match format {
                    OutputFormat::Pretty => {
                        eprintln!("{}: Cannot read file: {}", path.display(), error);
                    }
                    OutputFormat::Json => {
                        json_results.push(JsonFileResult {
                            file: path.display().to_string(),
                            errors: vec![ProjectError::FileReadError { path: path.clone(), error }],
                            error_count: 1,
                            warning_count: 0,
                            duration_ms: file_start.elapsed().as_millis() as u64,
                        });
                    }
                }
                total_errors += 1;
                continue;
            }
            Err(e) => {
                match format {
                    OutputFormat::Pretty => {
                        eprintln!("Error checking {}: {:?}", file.display(), e);
                    }
                    OutputFormat::Json => {
                        json_results.push(JsonFileResult {
                            file: file.display().to_string(),
                            errors: vec![e],
                            error_count: 1,
                            warning_count: 0,
                            duration_ms: file_start.elapsed().as_millis() as u64,
                        });
                    }
                }
                total_errors += 1;
                continue;
            }
        };

        match format {
            OutputFormat::Pretty => {
                // Load source texts for error reporting (lazily)
                for err in &errors {
                    let path = match err {
                        ProjectError::Binding(BindingError::DuplicateSymbol(_)) |
                        ProjectError::Binding(BindingError::UndefinedSymbol(_)) => file.clone(),
                        ProjectError::Type(_) => file.clone(),
                        ProjectError::ModuleNotFound { from_file, .. } => from_file.clone(),
                        ProjectError::FileReadError { path, .. } => path.clone(),
                    };
                    if !sources.contains_key(&path) {
                        if let Ok(text) = std::fs::read_to_string(&path) {
                            sources.insert(path.clone(), Arc::new(text));
                        }
                    }
                }

                // Report errors
                let (file_errors, file_warnings) = report_errors(file, &errors, &sources);
                total_errors += file_errors;
                total_warnings += file_warnings;
            }
            OutputFormat::Json => {
                // Count errors and warnings
                let mut file_errors = 0;
                let mut file_warnings = 0;
                for err in &errors {
                    match err {
                        ProjectError::Type(type_err) if type_err.is_warning() => {
                            file_warnings += 1;
                        }
                        _ => {
                            file_errors += 1;
                        }
                    }
                }

                json_results.push(JsonFileResult {
                    file: file.display().to_string(),
                    errors,
                    error_count: file_errors,
                    warning_count: file_warnings,
                    duration_ms: file_start.elapsed().as_millis() as u64,
                });

                total_errors += file_errors;
                total_warnings += file_warnings;
            }
        }
    }

    match format {
        OutputFormat::Pretty => {
            // Summary
            if total_errors == 0 && total_warnings == 0 {
                for file in files {
                    println!("{} {}: No errors", "✓".green(), file.display());
                }
            }
        }
        OutputFormat::Json => {
            let output = JsonOutput {
                files: json_results,
                total_errors,
                total_warnings,
                total_duration_ms: total_start.elapsed().as_millis() as u64,
            };
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
    }

    Ok(total_errors)
}

/// Report errors for a single file and return (error_count, warning_count).
fn report_errors(
    file: &PathBuf,
    errors: &[ProjectError],
    sources: &HashMap<PathBuf, Arc<String>>,
) -> (usize, usize) {
    let mut error_count = 0;
    let mut warning_count = 0;

    let filename = file.display().to_string();
    let source = sources.get(file).cloned().unwrap_or_else(|| Arc::new(String::new()));

    for err in errors {
        match err {
            ProjectError::Binding(BindingError::DuplicateSymbol(e)) => {
                error_count += 1;
                let diag = ZeldaError {
                    message: errors::DUPLICATE_IDENTIFIER.format_full(&[&e.name]),
                    src: NamedSource::new(&filename, source.clone()),
                    span: (e.duplicate.start as usize, e.duplicate.size() as usize).into(),
                    label: format!("'{}' is already declared", e.name),
                    help: Some("Consider using a different name".into()),
                };
                eprintln!("{:?}", miette::Report::new(diag));
            }
            ProjectError::Binding(BindingError::UndefinedSymbol(e)) => {
                error_count += 1;
                let diag = ZeldaError {
                    message: errors::CANNOT_FIND_NAME.format_full(&[&e.name]),
                    src: NamedSource::new(&filename, source.clone()),
                    span: (e.span.start as usize, e.span.size() as usize).into(),
                    label: "not found in this scope".into(),
                    help: Some("Check if the variable is declared before use".into()),
                };
                eprintln!("{:?}", miette::Report::new(diag));
            }
            ProjectError::Type(type_err) => {
                if type_err.is_warning() {
                    warning_count += 1;
                    let prefix = "warning".yellow();
                    eprintln!(
                        "{}: TS{}: {}",
                        prefix,
                        type_err.code,
                        type_err.message
                    );
                    eprintln!(
                        "  --> {}:{}",
                        filename,
                        type_err.span.start
                    );
                } else {
                    error_count += 1;
                    let diag = ZeldaError {
                        message: format!("TS{}: {}", type_err.code, type_err.message),
                        src: NamedSource::new(&filename, source.clone()),
                        span: (type_err.span.start as usize, type_err.span.size() as usize).into(),
                        label: "type mismatch here".into(),
                        help: None,
                    };
                    eprintln!("{:?}", miette::Report::new(diag));
                }
            }
            ProjectError::ModuleNotFound { specifier, from_file, span } => {
                error_count += 1;
                let from_filename = from_file.display().to_string();
                let from_source = sources.get(from_file).cloned().unwrap_or_else(|| Arc::new(String::new()));
                let diag = ZeldaError {
                    message: errors::CANNOT_FIND_MODULE.format_full(&[specifier]),
                    src: NamedSource::new(&from_filename, from_source),
                    span: (span.start as usize, span.size() as usize).into(),
                    label: "module not found".into(),
                    help: Some("Check the module path and ensure the file exists".into()),
                };
                eprintln!("{:?}", miette::Report::new(diag));
            }
            ProjectError::FileReadError { path, error } => {
                error_count += 1;
                eprintln!(
                    "{}: TS5083: Cannot read file '{}': {}",
                    "error".red(),
                    path.display(),
                    error
                );
            }
        }
    }

    if error_count > 0 || warning_count > 0 {
        if error_count == 0 {
            println!(
                "{} {}: {} warning(s)",
                "⚠".yellow(),
                filename,
                warning_count
            );
        } else {
            eprintln!(
                "\n{} Found {} error(s){} in {}",
                "✗".red(),
                error_count,
                if warning_count > 0 {
                    format!(" and {} warning(s)", warning_count)
                } else {
                    String::new()
                },
                filename
            );
        }
    }

    (error_count, warning_count)
}

