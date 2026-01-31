//! TypeScript Conformance Testing CLI

mod analysis;
mod compare;
mod diagnostic;
mod report;
mod runner;
mod source;
mod tsc;
mod types;
mod zelda;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use report::{print_compact, print_json, print_pretty};
use runner::run_all_tests;
use source::TestSource;
use types::Summary;

#[derive(Parser)]
#[command(name = "ts-conformance")]
#[command(about = "TypeScript conformance testing for zelda")]
struct Cli {
    #[arg(short, long)]
    source: Option<PathBuf>,

    #[arg(short, long)]
    filter: Option<String>,

    #[arg(long)]
    failures: bool,

    #[arg(short, long)]
    verbose: bool,

    #[arg(long)]
    analyze: bool,

    #[arg(short = 'o', long, value_enum, default_value = "pretty")]
    format: OutputFormat,

    #[arg(short = 'z', long, default_value = "target/release/zelda")]
    zelda: PathBuf,

    #[arg(short, long, default_value = "3")]
    iterations: usize,

    #[arg(long)]
    summary_only: bool,

    #[arg(short = 'r', long = "run")]
    run_files: Vec<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Bench {
        #[arg(long)]
        codebases: Option<String>,
    },
}

#[derive(ValueEnum, Clone, Default, Debug)]
enum OutputFormat {
    #[default]
    Pretty,
    Json,
    Compact,
}

fn main() {
    let cli = Cli::parse();

    if let Some(Command::Bench { codebases }) = cli.command {
        eprintln!("Benchmarking not yet implemented");
        if let Some(cb) = codebases {
            eprintln!("Would benchmark: {}", cb);
        }
        std::process::exit(0);
    }

    if !cli.zelda.exists() {
        eprintln!("Error: zelda binary not found at '{}'", cli.zelda.display());
        eprintln!("Build zelda first: cargo build --release");
        std::process::exit(1);
    }

    let test_source = match cli.source {
        Some(path) => TestSource::custom(path),
        None => TestSource::typescript(),
    };

    let tests = if !cli.run_files.is_empty() {
        cli.run_files.clone()
    } else {
        test_source.discover(cli.filter.as_deref())
    };

    if tests.is_empty() {
        eprintln!("No test files found");
        std::process::exit(1);
    }

    let show_progress = !matches!(cli.format, OutputFormat::Json);
    let results = run_all_tests(&tests, &cli.zelda, cli.iterations, show_progress);
    let summary = Summary::from_results(&results);
    let failed = summary.failed;

    let results_to_show: Vec<_> = if cli.failures {
        results.iter().filter(|r| !r.passed()).collect()
    } else {
        results.iter().collect()
    };

    match cli.format {
        OutputFormat::Pretty => {
            if cli.summary_only {
                print_compact(&summary);
            } else {
                print_pretty(&results_to_show, &summary, cli.verbose, cli.analyze);
            }
        }
        OutputFormat::Json => {
            print_json(&results, &summary);
        }
        OutputFormat::Compact => {
            print_compact(&summary);
        }
    }

    if failed > 0 {
        std::process::exit(1);
    }
}
