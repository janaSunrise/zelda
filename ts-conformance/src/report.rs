//! Human-readable and JSON reports.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::Serialize;

use crate::analysis::{CodeStats, rank_codes};
use crate::types::{Outcome, TestResult};

/// Counts of each outcome.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Summary {
    pub total: usize,
    pub skipped: usize,
    pub ran: usize,
    pub pass: usize,
    /// Passing tests where tsc reports at least one error (the rest are
    /// tests where both report nothing).
    pub pass_with_errors: usize,
    pub fail: usize,
    pub parse_error: usize,
    pub crash: usize,
    pub timeout: usize,
}

impl Summary {
    pub fn from_results<'a>(results: impl IntoIterator<Item = &'a TestResult>) -> Self {
        let mut s = Self::default();
        for r in results {
            s.total += 1;
            match r.outcome {
                Outcome::Skipped(_) => s.skipped += 1,
                Outcome::Pass => {
                    s.pass += 1;
                    s.pass_with_errors += usize::from(!r.expected.is_empty());
                }
                Outcome::Fail => s.fail += 1,
                Outcome::ParseError => s.parse_error += 1,
                Outcome::Crash(_) => s.crash += 1,
                Outcome::Timeout => s.timeout += 1,
            }
        }
        s.ran = s.total - s.skipped;
        s
    }

    pub fn pass_rate(&self) -> f64 {
        if self.ran == 0 {
            0.0
        } else {
            100.0 * self.pass as f64 / self.ran as f64
        }
    }
}

/// What to include in the text report.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReportOptions {
    /// Print each non-passing test with its diagnostic diff.
    pub details: bool,
    /// Print every skipped test with its reason.
    pub list_skipped: bool,
    /// How many rows of error-code rankings to show.
    pub top_codes: usize,
}

pub fn print_report(results: &[TestResult], opts: ReportOptions) {
    let summary = Summary::from_results(results);

    if opts.details {
        for r in results
            .iter()
            .filter(|r| !r.outcome.is_pass() && !r.outcome.is_skipped())
        {
            print_details(r);
        }
    }
    if opts.list_skipped {
        for r in results {
            if let Outcome::Skipped(reason) = &r.outcome {
                println!("skipped  {}  ({reason})", r.id);
            }
        }
        println!();
    }

    println!("Summary");
    println!("  tests         {:>6}", summary.total);
    println!("  skipped       {:>6}", summary.skipped);
    println!("  ran           {:>6}", summary.ran);
    println!(
        "    pass        {:>6}  {:.1}%  ({} matching tsc's errors, {} error-free)",
        summary.pass,
        summary.pass_rate(),
        summary.pass_with_errors,
        summary.pass - summary.pass_with_errors
    );
    println!("    fail        {:>6}", summary.fail);
    println!("    parse error {:>6}", summary.parse_error);
    println!("    crash       {:>6}", summary.crash);
    println!("    timeout     {:>6}", summary.timeout);

    print_categories(results);
    print_skip_reasons(results);

    if opts.top_codes > 0 {
        let failing: Vec<TestResult> = results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Fail))
            .cloned()
            .collect();
        print_codes(
            "Most-missed error codes (tsc reports them, zelda doesn't)",
            &rank_codes(&failing, |r| &r.missing),
            opts.top_codes,
        );
        print_codes(
            "Most over-reported error codes (zelda reports them, tsc doesn't)",
            &rank_codes(&failing, |r| &r.extra),
            opts.top_codes,
        );
    }

    let crashes: Vec<_> = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Crash(_) | Outcome::Timeout))
        .collect();
    if !crashes.is_empty() {
        println!("\nCrashes and timeouts");
        for r in crashes.iter().take(20) {
            println!("  {}  {}", r.id, r.outcome);
        }
        if crashes.len() > 20 {
            println!("  ... and {} more", crashes.len() - 20);
        }
    }
}

fn print_details(r: &TestResult) {
    println!("── {} [{}]", r.id, r.outcome);
    if let Some(config) = &r.config {
        println!("   baseline: {config}");
    }
    for e in &r.parse_errors {
        println!("   syntax   {e}");
    }
    for d in &r.missing {
        println!("   missing  {d}");
    }
    for d in &r.extra {
        println!("   extra    {d}");
    }
    println!();
}

fn print_categories(results: &[TestResult]) {
    let mut by_category: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for r in results.iter().filter(|r| !r.outcome.is_skipped()) {
        let entry = by_category.entry(r.category()).or_default();
        entry.0 += 1;
        entry.1 += usize::from(r.outcome.is_pass());
    }
    if by_category.len() < 2 {
        return;
    }
    let width = by_category.keys().map(|k| k.len()).max().unwrap_or(0);
    println!(
        "\n{:<w$}  {:>6}  {:>6}  {:>6}",
        "By category",
        "ran",
        "pass",
        "rate",
        w = width + 2
    );
    for (category, (ran, pass)) in by_category {
        println!(
            "  {category:<width$}  {ran:>6}  {pass:>6}  {:>5.1}%",
            100.0 * pass as f64 / ran as f64
        );
    }
}

fn print_skip_reasons(results: &[TestResult]) {
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    for r in results {
        if let Outcome::Skipped(reason) = &r.outcome {
            *reasons.entry(reason).or_default() += 1;
        }
    }
    if reasons.is_empty() {
        return;
    }
    let mut reasons: Vec<_> = reasons.into_iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!("\nSkip reasons");
    for (reason, count) in reasons {
        println!("  {count:>6}  {reason}");
    }
}

fn print_codes(title: &str, stats: &[CodeStats], limit: usize) {
    if stats.is_empty() {
        return;
    }
    println!("\n{title}");
    println!("  code     tests   only  total  example");
    for s in stats.iter().take(limit) {
        let mut example = s.example.clone();
        if example.chars().count() > 70 {
            example = example.chars().take(67).collect::<String>() + "...";
        }
        println!(
            "  TS{:<5} {:>6} {:>6} {:>6}  {example}",
            s.code, s.tests, s.sole_blocker, s.occurrences
        );
    }
    println!("  (only = tests where this is the single remaining difference)");
}

#[derive(Serialize)]
struct JsonReport<'a> {
    summary: Summary,
    results: &'a [TestResult],
}

/// Write the summary and every result as JSON, for tooling and diffing runs.
pub fn write_json(results: &[TestResult], path: &Path) -> io::Result<()> {
    let report = JsonReport {
        summary: Summary::from_results(results),
        results,
    };
    let file = io::BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer_pretty(file, &report).map_err(io::Error::other)
}
