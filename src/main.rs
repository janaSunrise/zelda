mod binder;  // binder/ directory
mod checker; // checker/ directory
mod errors;
mod symbols;
mod types;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use miette::{Diagnostic, IntoDiagnostic, NamedSource, Result, SourceSpan};
use owo_colors::OwoColorize;
use oxc_allocator::Allocator;
use oxc_parser::{Parser as OxcParser, ParserReturn};
use oxc_span::SourceType;
use thiserror::Error;

use crate::binder::{Binder, BindingError};
use crate::checker::Checker;

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

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Check { files } => {
            let mut total_errors = 0;
            for file in files {
                total_errors += check_file(&file)?;
            }
            if total_errors > 0 {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn check_file(path: &PathBuf) -> Result<usize> {
    let source_text = std::fs::read_to_string(path).into_diagnostic()?;
    let source = Arc::new(source_text.clone());

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
        return Ok(1);
    }

    if !errors.is_empty() {
        let count = errors.len();
        for error in errors {
            eprintln!("{}", error);
        }
        return Ok(count);
    }

    let mut binder = Binder::new();
    binder.bind_program(&program);

    let mut checker = Checker::new(&binder.symbols);
    checker.check_program(&program);

    let mut error_count = 0;
    let filename = path.display().to_string();

    // Report binding errors
    for error in &binder.errors {
        error_count += 1;
        let diag = match error {
            BindingError::DuplicateSymbol(err) => ZeldaError {
                message: errors::DUPLICATE_IDENTIFIER.format_full(&[&err.name]),
                src: NamedSource::new(&filename, source.clone()),
                span: (err.duplicate.start as usize, err.duplicate.size() as usize).into(),
                label: format!("'{}' is already declared", err.name),
                help: Some("Consider using a different name".into()),
            },
            BindingError::UndefinedSymbol(err) => ZeldaError {
                message: errors::CANNOT_FIND_NAME.format_full(&[&err.name]),
                src: NamedSource::new(&filename, source.clone()),
                span: (err.span.start as usize, err.span.size() as usize).into(),
                label: "not found in this scope".into(),
                help: Some("Check if the variable is declared before use".into()),
            },
        };
        eprintln!("{:?}", miette::Report::new(diag));
    }

    // Report type errors
    for error in &checker.errors {
        error_count += 1;
        let diag = ZeldaError {
            message: format!("TS{}: {}", error.code, error.message),
            src: NamedSource::new(&filename, source.clone()),
            span: (error.span.start as usize, error.span.size() as usize).into(),
            label: "type mismatch here".into(),
            help: None,
        };
        eprintln!("{:?}", miette::Report::new(diag));
    }

    if error_count == 0 {
        println!("{} {}: No errors", "✓".green(), path.display());
    } else {
        eprintln!(
            "\n{} Found {} error(s) in {}",
            "✗".red(),
            error_count,
            path.display()
        );
    }

    Ok(error_count)
}
