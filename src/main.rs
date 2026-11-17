use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand, ValueEnum};
use miette::{Diagnostic, NamedSource, SourceSpan};
use owo_colors::OwoColorize;
use serde::Serialize;
use thiserror::Error;

use zelda::{Project, ProjectError};

#[derive(Parser)]
#[command(name = "zelda", version)]
#[command(about = "Fast TypeScript type checker", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Clone, Copy, Default, Debug)]
enum OutputFormat {
    #[default]
    Pretty,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    /// Type check the given files and their imports.
    Check {
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Output format
        #[arg(long, value_enum, default_value = "pretty")]
        format: OutputFormat,
    },
}

#[derive(Error, Debug, Diagnostic)]
#[error("{message}")]
struct PrettyDiagnostic {
    message: String,
    #[source_code]
    src: NamedSource<Arc<String>>,
    #[label]
    span: Option<SourceSpan>,
}

#[derive(Debug, Serialize)]
struct JsonFileResult {
    file: String,
    errors: Vec<ProjectError>,
    error_count: usize,
    warning_count: usize,
    duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct JsonOutput {
    files: Vec<JsonFileResult>,
    total_errors: usize,
    total_warnings: usize,
    total_duration_ms: u64,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Check { files, format } => {
            if check_files(&files, format) > 0 {
                std::process::exit(1);
            }
        }
    }
}

/// Check each file (and its imports) and report diagnostics. Returns the error count.
fn check_files(files: &[PathBuf], format: OutputFormat) -> usize {
    let total_start = Instant::now();

    // Use the first file's directory to find tsconfig.json
    let project_root = files
        .first()
        .and_then(|f| f.parent())
        .unwrap_or(Path::new("."));
    let mut project = Project::with_tsconfig(project_root);
    let mut sources = SourceCache::default();
    let mut json_results = Vec::new();
    let mut total_errors = 0;
    let mut total_warnings = 0;

    for file in files {
        let file_start = Instant::now();
        let errors = match project.check_file(file) {
            Ok(errors) => errors,
            Err(err) => vec![err],
        };

        let warnings = errors.iter().filter(|e| e.is_warning()).count();
        let error_count = errors.len() - warnings;
        total_errors += error_count;
        total_warnings += warnings;

        match format {
            OutputFormat::Pretty => {
                for err in &errors {
                    report_pretty(err, &mut sources);
                }
                print_file_summary(file, error_count, warnings);
            }
            OutputFormat::Json => json_results.push(JsonFileResult {
                file: file.display().to_string(),
                errors,
                error_count,
                warning_count: warnings,
                duration_ms: file_start.elapsed().as_millis() as u64,
            }),
        }
    }

    if let OutputFormat::Json = format {
        let output = JsonOutput {
            files: json_results,
            total_errors,
            total_warnings,
            total_duration_ms: total_start.elapsed().as_millis() as u64,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&output).expect("JSON output is serializable")
        );
    }

    total_errors
}

/// Lazily loaded source text, keyed by path, for rendering code frames.
#[derive(Default)]
struct SourceCache(HashMap<PathBuf, Arc<String>>);

impl SourceCache {
    fn get(&mut self, path: &Path) -> Arc<String> {
        self.0
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(std::fs::read_to_string(path).unwrap_or_default()))
            .clone()
    }
}

fn report_pretty(err: &ProjectError, sources: &mut SourceCache) {
    let message = match err.code() {
        Some(code) => format!("TS{code}: {}", err.message()),
        None => err.message(),
    };

    if err.is_warning() {
        eprintln!("{}: {message}", "warning".yellow());
        if let Some(span) = err.span() {
            eprintln!("  --> {}:{}", err.file().display(), span.start);
        }
        return;
    }

    let diagnostic = PrettyDiagnostic {
        message,
        src: NamedSource::new(err.file().display().to_string(), sources.get(err.file())),
        span: err
            .span()
            .map(|span| (span.start as usize, span.size() as usize).into()),
    };
    eprintln!("{:?}", miette::Report::new(diagnostic));
}

fn print_file_summary(file: &Path, errors: usize, warnings: usize) {
    let name = file.display();
    match (errors, warnings) {
        (0, 0) => println!("{} {name}: No errors", "✓".green()),
        (0, w) => println!("{} {name}: {w} warning(s)", "⚠".yellow()),
        (e, 0) => eprintln!("\n{} Found {e} error(s) in {name}", "✗".red()),
        (e, w) => eprintln!(
            "\n{} Found {e} error(s) and {w} warning(s) in {name}",
            "✗".red()
        ),
    }
}
