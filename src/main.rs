mod types;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::{IntoDiagnostic, Result};
use oxc_allocator::Allocator;
use oxc_parser::{Parser as OxcParser, ParserReturn};
use oxc_span::SourceType;

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

    println!("AST for {}", path.display());
    println!("{:#?}", program);

    Ok(())
}
