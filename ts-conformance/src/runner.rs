//! Running tests: prepare inputs, check them in an isolated worker process,
//! and compare against the oracle.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use wait_timeout::ChildExt;

use crate::compare;
use crate::diagnostic::{Diagnostic, LineIndex};
use crate::source::TestFile;
use crate::support::unsupported_reason;
use crate::tsc::{BaselineDiagnostic, Oracle, choose_config};
use crate::types::{Outcome, TestResult};
use crate::zelda::RawDiagnostic;

/// Settings shared by every test in a run.
pub struct RunConfig {
    pub oracle: Oracle,
    /// Executable that implements the `worker` subcommand (normally ourselves).
    pub worker: PathBuf,
    pub timeout: Duration,
    /// Run tests even if they use features zelda doesn't support.
    pub include_unsupported: bool,
}

/// Run all tests in parallel, with a progress bar on stderr.
pub fn run_all(config: &RunConfig, tests: &[(String, PathBuf)]) -> Vec<TestResult> {
    let progress = ProgressBar::new(tests.len() as u64);
    progress.set_style(
        ProgressStyle::with_template("{spinner} [{bar:40}] {pos}/{len} {elapsed_precise} {msg}")
            .expect("valid template")
            .progress_chars("=> "),
    );

    let results = tests
        .par_iter()
        .map(|(id, path)| {
            let result = run_test(config, id, path);
            progress.inc(1);
            result
        })
        .collect();
    progress.finish_and_clear();
    results
}

/// Run a single test end to end.
pub fn run_test(config: &RunConfig, id: &str, path: &Path) -> TestResult {
    let start = Instant::now();
    let mut result = run_test_inner(config, id, path);
    result.duration_ms = start.elapsed().as_millis() as u64;
    result
}

fn run_test_inner(config: &RunConfig, id: &str, path: &Path) -> TestResult {
    let skipped = |reason: &str| TestResult::new(id, Outcome::Skipped(reason.into()));

    let Ok(bytes) = std::fs::read(path) else {
        return skipped("unreadable test file");
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return skipped("non-UTF-8 test file");
    };
    let test = TestFile::parse(id, &text);

    let Some(chosen) = choose_config(config.oracle.configs(&test.suite, &test.stem)) else {
        return skipped("no tsc baseline (not run by the TypeScript harness)");
    };

    let mut options: BTreeMap<String, String> = test.settings.clone();
    options.extend(chosen.options.clone());
    if !config.include_unsupported
        && let Some(reason) = unsupported_reason(&test, &options)
    {
        return skipped(&reason);
    }

    let expected = match config.oracle.expected(&test.suite, chosen) {
        Ok(diags) => diags,
        Err(err) => return skipped(&format!("unreadable baseline: {err}")),
    };
    // Option-level diagnostics and diagnostics in files outside the test
    // (e.g. lib files) are not something zelda can reproduce.
    let expected: Vec<Diagnostic> = expected
        .into_iter()
        .filter_map(|d| match d {
            BaselineDiagnostic::Located(d) if test.unit(&d.file).is_some() => Some(d),
            _ => None,
        })
        .collect();

    let mut result = TestResult::new(id, Outcome::Pass);
    result.config = Some(chosen.name.clone());
    result.expected = expected;

    let raw = match check_in_worker(config, &test) {
        Ok(raw) => raw,
        Err(outcome) => {
            result.outcome = outcome;
            return result;
        }
    };

    let line_indexes: HashMap<&str, LineIndex> = test
        .units
        .iter()
        .map(|u| (u.name.as_str(), LineIndex::new(&u.content)))
        .collect();
    for d in raw {
        let Some(code) = d.code else {
            result
                .parse_errors
                .push(format!("{}: {}", d.file, d.message));
            continue;
        };
        let (line, col) = line_indexes
            .get(d.file.as_str())
            .map_or((0, 0), |idx| idx.line_col(d.start as usize));
        result.actual.push(Diagnostic {
            file: d.file,
            line,
            col,
            code,
            message: d.message,
        });
    }

    let diff = compare::diff(&result.expected, &result.actual);
    result.outcome = if !result.parse_errors.is_empty() {
        Outcome::ParseError
    } else if diff.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail
    };
    result.missing = diff.missing;
    result.extra = diff.extra;
    result
}

/// Write the test's units to a temp dir and check them in a worker process.
fn check_in_worker(config: &RunConfig, test: &TestFile) -> Result<Vec<RawDiagnostic>, Outcome> {
    let crash = |msg: String| Outcome::Crash(msg);

    let dir = tempfile::tempdir().map_err(|e| crash(format!("tempdir: {e}")))?;
    for unit in &test.units {
        let path = dir.path().join(&unit.name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| crash(format!("mkdir: {e}")))?;
        }
        std::fs::write(&path, &unit.content).map_err(|e| crash(format!("write: {e}")))?;
    }

    // Output goes to files rather than pipes so a chatty worker can never
    // block on a full pipe while we wait on it.
    let out_path = dir.path().join(".zelda-stdout.json");
    let err_path = dir.path().join(".zelda-stderr.txt");
    let create = |path: &Path| {
        std::fs::File::create(path).map_err(|e| crash(format!("create {}: {e}", path.display())))
    };
    let (out_file, err_file) = (create(&out_path)?, create(&err_path)?);

    let mut child = Command::new(&config.worker)
        .arg("worker")
        .arg("--root")
        .arg(dir.path())
        .args(test.units.iter().map(|u| &u.name))
        .stdin(Stdio::null())
        .stdout(out_file)
        .stderr(err_file)
        .spawn()
        .map_err(|e| crash(format!("spawn worker: {e}")))?;

    let status = match child.wait_timeout(config.timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Outcome::Timeout);
        }
        Err(e) => return Err(crash(format!("wait: {e}"))),
    };

    if !status.success() {
        let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
        return Err(crash(summarize_crash(&status.to_string(), &stderr)));
    }

    let output = std::fs::read_to_string(&out_path).map_err(|e| crash(format!("read: {e}")))?;
    serde_json::from_str(&output).map_err(|e| crash(format!("bad worker output: {e}")))
}

/// A one-line crash summary: the panic message if there is one.
fn summarize_crash(status: &str, stderr: &str) -> String {
    let panic_line = stderr
        .lines()
        .skip_while(|l| !l.contains("panicked at"))
        .nth(1)
        .map(str::trim);
    let overflow = stderr.contains("has overflowed its stack");
    match (panic_line, overflow) {
        (_, true) => "stack overflow".into(),
        (Some(msg), _) => format!("panic: {msg}"),
        (None, _) => status.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_summaries() {
        let stderr = "\nthread 'main' panicked at src/checker/mod.rs:10:5:\nindex out of bounds\nnote: run with `RUST_BACKTRACE=1`";
        assert_eq!(
            summarize_crash("exit status: 101", stderr),
            "panic: index out of bounds"
        );
        let stderr =
            "\nthread 'main' has overflowed its stack\nfatal runtime error: stack overflow";
        assert_eq!(summarize_crash("signal: 6", stderr), "stack overflow");
        assert_eq!(summarize_crash("signal: 9", ""), "signal: 9");
    }
}
