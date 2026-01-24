mod binder;
mod symbols;
mod types;
mod utils;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::{IntoDiagnostic, Result};
use oxc_allocator::Allocator;
use oxc_parser::{Parser as OxcParser, ParserReturn};
use oxc_span::SourceType;

use crate::binder::{Binder, BindingError};
use crate::utils::offset_to_line_col;

#[derive(Parser)]
#[command(name = "zelda")]
#[command(about = "Fast TypeScript type checker", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Check {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Check { files } => {
            for file in files {
                check_file(&file)?;
            }
        }
    }

    Ok(())
}

fn check_file(path: &PathBuf) -> Result<()> {
    let source_text = std::fs::read_to_string(path).into_diagnostic()?;

    let allocator = Allocator::default();
    let source_type = SourceType::ts();

    let ParserReturn {
        program,
        errors,
        panicked,
        ..
    } = OxcParser::new(&allocator, &source_text, source_type).parse();

    if panicked {
        eprintln!("Parser panicked on {}", path.display());
        return Ok(());
    }

    if !errors.is_empty() {
        eprintln!("Parse errors in {}:", path.display());
        for error in errors {
            eprintln!("  {}", error);
        }
        return Ok(());
    }

    let mut binder = Binder::new();
    binder.bind_program(&program);

    let mut has_errors = false;
    for error in &binder.errors {
        has_errors = true;
        match error {
            BindingError::DuplicateSymbol(err) => {
                let (line, col) = offset_to_line_col(&source_text, err.duplicate.start);
                eprintln!(
                    "{}({}:{}): error: Duplicate identifier '{}'",
                    path.display(),
                    line,
                    col,
                    err.name
                );
            }
            BindingError::UndefinedSymbol(err) => {
                let (line, col) = offset_to_line_col(&source_text, err.span.start);
                eprintln!(
                    "{}({}:{}): error: Cannot find name '{}'",
                    path.display(),
                    line,
                    col,
                    err.name
                );
            }
        }
    }

    if !has_errors {
        println!("{}: No errors found ({} symbols bound)", path.display(), binder.symbols.symbols.len());
    }

    Ok(())
}
