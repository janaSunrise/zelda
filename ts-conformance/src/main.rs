//! `ts-conformance`: run zelda against TypeScript's test suite.
//!
//! Usage: `cargo conformance [FILTER]...`. See the crate README.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use ts_conformance::expectations::Expectations;
use ts_conformance::report::{self, ReportOptions};
use ts_conformance::runner::{self, RunConfig};
use ts_conformance::source::{self, SUITES};
use ts_conformance::tsc::Oracle;
use ts_conformance::zelda;

/// Workspace root, so the tool works from any directory.
const WORKSPACE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// Where the TypeScript repo keeps its test inputs and TS 7 baselines.
const CASES_DIR: &str = "tests/cases";
const BASELINES_DIR: &str = "tsc/testdata/baselines/reference/submodule";

#[derive(Parser)]
#[command(name = "ts-conformance", args_conflicts_with_subcommands = true)]
#[command(
    about = "Check zelda against TypeScript's conformance suite, using tsc's baselines as the oracle"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Only run tests whose id (e.g. `compiler/foo.ts`) contains this text (case-insensitive),
    /// or matches it as a glob if it contains `*` or `?`.
    filters: Vec<String>,

    /// Rewrite the passing-tests list to match this run.
    #[arg(long, conflicts_with = "all")]
    update: bool,

    /// Fail if tests pass that aren't in the passing-tests list (for CI).
    #[arg(long)]
    locked: bool,

    /// Also run tests that use features zelda doesn't support.
    #[arg(long)]
    all: bool,

    /// Show the diagnostic diff for each non-passing test. On by default when
    /// 20 or fewer tests run.
    #[arg(short, long)]
    verbose: bool,

    /// List each skipped test with its reason.
    #[arg(long)]
    skipped: bool,

    /// Write every result to this JSON file.
    #[arg(long, value_name = "PATH")]
    json: Option<PathBuf>,

    /// Per-test timeout in seconds.
    #[arg(long, default_value_t = 10)]
    timeout: u64,

    /// Number of tests to run in parallel (default: number of CPUs).
    #[arg(short, long)]
    jobs: Option<usize>,

    /// Rows to show in the error-code rankings.
    #[arg(long, default_value_t = 15)]
    top: usize,

    /// Path to the TypeScript repo checkout.
    #[arg(long, value_name = "DIR", default_value_os_t = Path::new(WORKSPACE).join("_submodules/typescript"))]
    typescript: PathBuf,

    /// Path to the passing-tests list.
    #[arg(long, value_name = "PATH", default_value_os_t = Path::new(WORKSPACE).join("ts-conformance/passing.txt"))]
    expectations: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Internal: check files in-process and print diagnostics as JSON.
    #[command(hide = true)]
    Worker {
        #[arg(long)]
        root: PathBuf,
        files: Vec<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Some(Command::Worker { root, files }) = &cli.command {
        let diagnostics = zelda::check(root, files);
        println!(
            "{}",
            serde_json::to_string(&diagnostics).expect("diagnostics are serializable")
        );
        return ExitCode::SUCCESS;
    }

    match run(&cli) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, String> {
    let cases_dir = cli.typescript.join(CASES_DIR);
    let baselines_dir = cli.typescript.join(BASELINES_DIR);
    if !cases_dir.is_dir() || !baselines_dir.is_dir() {
        return Err(format!(
            "TypeScript test suite not found under {}.\n\
             Set it up with: ./scripts/setup-ts-tests.sh",
            cli.typescript.display()
        ));
    }

    if let Some(jobs) = cli.jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build_global()
            .map_err(|e| e.to_string())?;
    }

    let tests: Vec<_> = source::discover(&cases_dir)
        .into_iter()
        .filter(|(id, _)| {
            cli.filters.is_empty() || cli.filters.iter().any(|f| matches_filter(f, id))
        })
        .collect();
    if tests.is_empty() {
        return Err("no tests matched".into());
    }

    let config = RunConfig {
        oracle: Oracle::load(&baselines_dir, &SUITES)
            .map_err(|e| format!("reading baselines in {}: {e}", baselines_dir.display()))?,
        worker: std::env::current_exe().map_err(|e| e.to_string())?,
        timeout: Duration::from_secs(cli.timeout),
        include_unsupported: cli.all,
    };

    eprintln!("Running {} tests...", tests.len());
    let results = runner::run_all(&config, &tests);
    let ran = results.iter().filter(|r| !r.outcome.is_skipped()).count();

    report::print_report(
        &results,
        ReportOptions {
            details: cli.verbose || ran <= 20,
            list_skipped: cli.skipped,
            top_codes: cli.top,
        },
    );
    if let Some(path) = &cli.json {
        report::write_json(&results, path)
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
        println!("\nWrote {}", path.display());
    }

    let mut expectations = Expectations::load(&cli.expectations)
        .map_err(|e| format!("reading {}: {e}", cli.expectations.display()))?;
    let delta = expectations.delta(&results);

    if cli.update {
        expectations.update(&results);
        expectations
            .save(&cli.expectations)
            .map_err(|e| format!("writing {}: {e}", cli.expectations.display()))?;
        println!(
            "\nUpdated {}: +{} -{} ({} passing)",
            cli.expectations.display(),
            delta.new_passes.len(),
            delta.regressions.len(),
            expectations.passing.len()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let mut ok = true;
    if !delta.regressions.is_empty() {
        ok = false;
        println!(
            "\n{} regression(s): expected to pass, but didn't:",
            delta.regressions.len()
        );
        for id in &delta.regressions {
            let outcome = results
                .iter()
                .find(|r| &r.id == id)
                .map(|r| r.outcome.to_string());
            println!("  {id}  ({})", outcome.unwrap_or_default());
        }
        println!("Run `cargo conformance <test> -v` to see the diff.");
    }
    if !delta.new_passes.is_empty() {
        ok &= !cli.locked;
        println!(
            "\n{} test(s) newly passing. Lock them in with `cargo conformance --update`.",
            delta.new_passes.len()
        );
        for id in delta.new_passes.iter().take(20) {
            println!("  {id}");
        }
        if delta.new_passes.len() > 20 {
            println!("  ... and {} more", delta.new_passes.len() - 20);
        }
    }
    if ok && delta.regressions.is_empty() && delta.new_passes.is_empty() {
        println!("\nNo regressions.");
    }

    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Case-insensitive substring match, or glob match if the filter contains `*` or `?`.
fn matches_filter(filter: &str, id: &str) -> bool {
    let (filter, id) = (filter.to_lowercase(), id.to_lowercase());
    if filter.contains(['*', '?']) {
        glob_match(filter.as_bytes(), id.as_bytes())
    } else {
        id.contains(&filter)
    }
}

/// Minimal glob: `*` matches any run of characters (including `/`), `?` any one.
fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut backtrack = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            backtrack = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = backtrack {
            p = star_p + 1;
            t = star_t + 1;
            backtrack = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters() {
        assert!(matches_filter("enum", "compiler/constEnum1.ts"));
        assert!(matches_filter("compiler/enum*", "compiler/enumBasics.ts"));
        assert!(!matches_filter(
            "compiler/enum*",
            "conformance/enums/enumBasics.ts"
        ));
        assert!(matches_filter(
            "*/union*.ts",
            "conformance/types/union/unionTypes1.ts"
        ));
        assert!(matches_filter("a?c", "abc"));
        assert!(!matches_filter("a?c", "ac"));
    }
}
