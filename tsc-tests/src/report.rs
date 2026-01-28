//! Results formatting for table and JSON output.

use colored::Colorize;

use crate::types::{JsonReport, JsonTestResult, Summary, TestResult};

/// Print results as a formatted table.
pub fn print_table(results: &[TestResult], summary: &Summary) {
    println!();
    println!("{}", "TSC Compatibility Report".bold());
    println!("{}", "========================".bold());
    println!();

    // Column widths
    const FILE_W: usize = 28;
    const NUM_W: usize = 9;
    const TIME_W: usize = 11;
    const SPEED_W: usize = 10;

    // Print header
    println!(
        "{:FILE_W$} {:>NUM_W$} {:>NUM_W$} {:>NUM_W$} {:>TIME_W$} {:>TIME_W$} {:>SPEED_W$}",
        "File", "Matched", "Missing", "Extra", "Zelda(ms)", "TSC(ms)", "Speedup"
    );
    println!("{}", "─".repeat(FILE_W + NUM_W * 3 + TIME_W * 2 + SPEED_W + 6));

    // Print each result
    for result in results {
        let file_name = result
            .file
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| result.file.display().to_string());

        // Truncate long filenames
        let file_display = if file_name.len() > FILE_W {
            format!("{}…", &file_name[..FILE_W - 1])
        } else {
            file_name.clone()
        };

        let matched = result.matched.len();
        let missing = result.missing.len();
        let extra = result.extra.len();

        let speedup = if result.zelda_ms > 0 {
            result.tsc_ms as f64 / result.zelda_ms as f64
        } else {
            f64::INFINITY
        };

        // Print with proper padding first, then apply colors
        let file_col = format!("{:FILE_W$}", file_display);
        let matched_col = format!("{:>NUM_W$}", matched);
        let missing_col = format!("{:>NUM_W$}", missing);
        let extra_col = format!("{:>NUM_W$}", extra);
        let zelda_col = format!("{:>TIME_W$}", result.zelda_ms);
        let tsc_col = format!("{:>TIME_W$}", result.tsc_ms);
        let speedup_col = format!("{:>SPEED_W$.1}x", speedup);

        // Apply colors after formatting
        let file_colored = if result.passed() {
            file_col.green()
        } else {
            file_col.red()
        };

        let missing_colored = if missing > 0 {
            missing_col.red()
        } else {
            missing_col.normal()
        };

        let extra_colored = if extra > 0 {
            extra_col.yellow()
        } else {
            extra_col.normal()
        };

        let speedup_colored = speedup_col.cyan();

        println!(
            "{} {} {} {} {} {} {}",
            file_colored,
            matched_col,
            missing_colored,
            extra_colored,
            zelda_col,
            tsc_col,
            speedup_colored
        );

        // Show details for failed tests
        if !result.passed() {
            if !result.missing.is_empty() {
                let codes: Vec<String> = result.missing.iter().map(|c| format!("TS{}", c)).collect();
                println!("    {} {}", "Missing:".red(), codes.join(", "));
            }
            if !result.extra.is_empty() {
                let codes: Vec<String> = result.extra.iter().map(|c| format!("TS{}", c)).collect();
                println!("    {} {}", "Extra:".yellow(), codes.join(", "));
            }
        }
    }

    // Print summary
    println!();
    println!("{}", "─".repeat(FILE_W + NUM_W * 3 + TIME_W * 2 + SPEED_W + 6));
    println!();

    let pass_rate = if summary.total_files > 0 {
        100.0 * summary.passed as f64 / summary.total_files as f64
    } else {
        0.0
    };

    if summary.failed == 0 {
        println!(
            "{}  {}/{} passed ({:.0}%)",
            "✓".green(),
            summary.passed.to_string().green(),
            summary.total_files,
            pass_rate
        );
    } else {
        println!(
            "{}  {}/{} passed ({:.0}%)",
            "✗".red(),
            summary.passed,
            summary.total_files,
            pass_rate
        );
    }
    println!();
    println!(
        "  Errors:  {} matched, {} missing, {} extra",
        summary.matched_errors.to_string().green(),
        if summary.missing_errors > 0 {
            summary.missing_errors.to_string().red()
        } else {
            summary.missing_errors.to_string().normal()
        },
        if summary.extra_errors > 0 {
            summary.extra_errors.to_string().yellow()
        } else {
            summary.extra_errors.to_string().normal()
        }
    );
    println!(
        "  Time:    Zelda {}ms vs TSC {}ms",
        summary.zelda_total_ms, summary.tsc_total_ms
    );
    println!(
        "  Speedup: {}",
        format!("{:.1}x faster", summary.speedup).cyan().bold()
    );
    println!();
}

/// Print results as JSON.
pub fn print_json(results: &[TestResult], summary: Summary) {
    let json_results: Vec<JsonTestResult> = results.iter().map(JsonTestResult::from).collect();

    let report = JsonReport {
        summary,
        results: json_results,
    };

    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

/// Print a compact one-line summary (useful for CI).
pub fn print_compact(summary: &Summary) {
    let status = if summary.failed == 0 {
        "PASS".green().bold()
    } else {
        "FAIL".red().bold()
    };

    println!(
        "{} {}/{} tests, {:.1}x speedup ({}ms vs {}ms)",
        status,
        summary.passed,
        summary.total_files,
        summary.speedup,
        summary.zelda_total_ms,
        summary.tsc_total_ms
    );
}
