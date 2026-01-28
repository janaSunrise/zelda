//! TSC Compatibility Testing CLI for Zelda
//!
//! This tool runs zelda and TypeScript's tsc on a set of test fixtures,
//! compares their error output, and reports on compatibility and performance.

mod compare;
mod report;
mod runner;
mod tsc;
mod types;
mod zelda;

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use report::{print_compact, print_json, print_table};
use runner::{discover_fixtures, run_all_tests};
use types::Summary;

#[derive(Parser)]
#[command(name = "tsc-tests")]
#[command(about = "TSC compatibility testing and benchmarking for zelda")]
struct Cli {
    /// Directory containing test fixtures
    #[arg(short, long, default_value = "fixtures")]
    fixtures: PathBuf,

    /// Path to zelda binary
    #[arg(short = 'z', long, default_value = "../target/release/zelda")]
    zelda: PathBuf,

    /// Output format
    #[arg(short, long, value_enum, default_value = "table")]
    output: OutputFormat,

    /// Number of benchmark iterations per file
    #[arg(short, long, default_value = "3")]
    iterations: usize,

    /// Only show summary (no individual results)
    #[arg(short, long)]
    summary_only: bool,

    /// Run specific fixture file(s) instead of the whole directory
    #[arg(short = 'r', long = "run")]
    run_files: Vec<PathBuf>,
}

#[derive(ValueEnum, Clone, Default)]
enum OutputFormat {
    #[default]
    Table,
    Json,
    Compact,
}

fn main() {
    let cli = Cli::parse();

    // Verify zelda binary exists
    if !cli.zelda.exists() {
        eprintln!(
            "Error: zelda binary not found at '{}'",
            cli.zelda.display()
        );
        eprintln!("Please build zelda first: cargo build --release");
        std::process::exit(1);
    }

    // Determine which files to test
    let fixtures = if !cli.run_files.is_empty() {
        cli.run_files.clone()
    } else {
        if !cli.fixtures.exists() {
            eprintln!(
                "Error: fixtures directory not found at '{}'",
                cli.fixtures.display()
            );
            std::process::exit(1);
        }
        discover_fixtures(&cli.fixtures)
    };

    if fixtures.is_empty() {
        eprintln!("No test fixtures found");
        std::process::exit(1);
    }

    println!(
        "Running {} test(s) with {} iteration(s)...",
        fixtures.len(),
        cli.iterations
    );

    // Run all tests
    let results = run_all_tests(&fixtures, &cli.zelda, cli.iterations);

    // Generate summary
    let summary = Summary::from_results(&results);

    // Output results
    let failed = summary.failed;
    match cli.output {
        OutputFormat::Table => {
            if cli.summary_only {
                print_compact(&summary);
            } else {
                print_table(&results, &summary);
            }
        }
        OutputFormat::Json => {
            print_json(&results, summary);
        }
        OutputFormat::Compact => {
            print_compact(&summary);
        }
    }

    // Exit with failure if any tests failed
    if failed > 0 {
        std::process::exit(1);
    }
}
