//! Console output and reporting.

use std::collections::HashMap;

use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table};
use console::style;

use crate::analysis::AnalysisReport;
use crate::compare::MatchKind;
use crate::types::{CategoryResult, JsonReport, JsonTestResult, Summary, TestResult};

pub fn print_pretty(results: &[&TestResult], summary: &Summary, verbose: bool, analyze: bool) {
    print_header();
    print_category_table(results);

    if !results.is_empty() && results.iter().any(|r| !r.passed()) {
        println!();
        print_failures(results, verbose);
    }

    if analyze {
        println!();
        let all_results: Vec<TestResult> = results.iter().map(|r| (*r).clone()).collect();
        let report = AnalysisReport::from_results(&all_results);
        print_analysis(&report);
    }

    println!();
    print_summary(summary);
}

fn print_header() {
    let title = style("TypeScript Conformance Report").bold().cyan();
    println!("\n{}", "═".repeat(60));
    println!("  {}", title);
    println!("{}\n", "═".repeat(60));
}

fn print_category_table(results: &[&TestResult]) {
    let mut categories: HashMap<String, Vec<&TestResult>> = HashMap::new();
    for result in results {
        let cat = result.category().unwrap_or("uncategorized").to_string();
        categories.entry(cat).or_default().push(result);
    }

    if categories.is_empty() {
        return;
    }

    let mut cat_results: Vec<CategoryResult> = categories
        .iter()
        .map(|(name, results)| CategoryResult::from_results(name.clone(), results))
        .collect();
    cat_results.sort_by(|a, b| b.pass_rate.partial_cmp(&a.pass_rate).unwrap());

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Category").add_attribute(Attribute::Bold),
            Cell::new("Progress").add_attribute(Attribute::Bold),
            Cell::new("Missing").add_attribute(Attribute::Bold),
            Cell::new("Extra").add_attribute(Attribute::Bold),
        ]);

    for cat in &cat_results {
        let progress_bar = make_progress_bar(cat.pass_rate);
        let pct = format!("{:.0}%", cat.pass_rate * 100.0);

        let missing_cell = if cat.missing > 0 {
            Cell::new(cat.missing).fg(Color::Red)
        } else {
            Cell::new(cat.missing)
        };

        let extra_cell = if cat.extra > 0 {
            Cell::new(cat.extra).fg(Color::Yellow)
        } else {
            Cell::new(cat.extra)
        };

        table.add_row(vec![
            Cell::new(&cat.name),
            Cell::new(format!("{} {}", progress_bar, pct)),
            missing_cell,
            extra_cell,
        ]);
    }

    println!("{table}");
}

fn make_progress_bar(progress: f64) -> String {
    let width = 10;
    let filled = (progress * width as f64).round() as usize;
    let empty = width - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

fn print_failures(results: &[&TestResult], verbose: bool) {
    let failures: Vec<_> = results.iter().filter(|r| !r.passed()).collect();
    if failures.is_empty() {
        return;
    }

    println!("{}", style("Failures:").bold().red());
    println!("{}", "─".repeat(60));

    for result in failures.iter().take(10) {
        println!();
        println!(
            "{} {}",
            style("FAIL:").red().bold(),
            style(result.test.path.display()).white().bold()
        );

        let missing: Vec<_> = result
            .comparison
            .matches
            .iter()
            .filter(|m| m.kind == MatchKind::Missing)
            .collect();

        if !missing.is_empty() {
            println!(
                "\n  {} errors TSC found but Zelda missed:",
                style("Missing").red()
            );
            for m in missing.iter().take(5) {
                if let Some(ref diag) = m.theirs {
                    println!(
                        "    L{}:{}  {}  {}",
                        diag.line,
                        diag.col,
                        style(format!("TS{}", diag.code)).yellow(),
                        truncate(&diag.message, 50)
                    );
                }
            }
            if missing.len() > 5 {
                println!("    ... and {} more", missing.len() - 5);
            }
        }

        let extra: Vec<_> = result
            .comparison
            .matches
            .iter()
            .filter(|m| m.kind == MatchKind::Extra)
            .collect();

        if !extra.is_empty() {
            println!(
                "\n  {} errors Zelda found but TSC didn't:",
                style("Extra").yellow()
            );
            for m in extra.iter().take(5) {
                if let Some(ref diag) = m.ours {
                    println!(
                        "    L{}:{}  {}  {}",
                        diag.line,
                        diag.col,
                        style(format!("TS{}", diag.code)).yellow(),
                        truncate(&diag.message, 50)
                    );
                }
            }
            if extra.len() > 5 {
                println!("    ... and {} more", extra.len() - 5);
            }
        }

        if verbose {
            println!("\n  {}", style("(use -v to show source context)").dim());
        }

        println!("\n{}", "─".repeat(60));
    }

    if failures.len() > 10 {
        println!(
            "\n{} more failures not shown",
            style(format!("... and {}", failures.len() - 10)).dim()
        );
    }
}

fn print_analysis(report: &AnalysisReport) {
    println!("{}", style("Error Code Analysis").bold().cyan());
    println!("{}", "─".repeat(60));

    if !report.missing_codes.is_empty() {
        println!(
            "\n{} (TSC found, Zelda didn't):",
            style("Missing Error Codes").red()
        );

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                Cell::new("Code").add_attribute(Attribute::Bold),
                Cell::new("Description").add_attribute(Attribute::Bold),
                Cell::new("Count").add_attribute(Attribute::Bold),
                Cell::new("Example Files").add_attribute(Attribute::Bold),
            ]);

        for stat in report.top_missing(5) {
            let examples = stat
                .example_files
                .iter()
                .filter_map(|p| p.file_name())
                .filter_map(|n| n.to_str())
                .collect::<Vec<_>>()
                .join(", ");

            table.add_row(vec![
                Cell::new(format!("TS{}", stat.code)).fg(Color::Yellow),
                Cell::new(truncate(&stat.description, 40)),
                Cell::new(stat.missing_count).fg(Color::Red),
                Cell::new(truncate(&examples, 30)),
            ]);
        }

        println!("{table}");
    }

    if !report.extra_codes.is_empty() {
        println!(
            "\n{} (Zelda found, TSC didn't):",
            style("Extra Error Codes").yellow()
        );

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec![
                Cell::new("Code").add_attribute(Attribute::Bold),
                Cell::new("Description").add_attribute(Attribute::Bold),
                Cell::new("Count").add_attribute(Attribute::Bold),
                Cell::new("Example Files").add_attribute(Attribute::Bold),
            ]);

        for stat in report.top_extra(5) {
            let examples = stat
                .example_files
                .iter()
                .filter_map(|p| p.file_name())
                .filter_map(|n| n.to_str())
                .collect::<Vec<_>>()
                .join(", ");

            table.add_row(vec![
                Cell::new(format!("TS{}", stat.code)).fg(Color::Yellow),
                Cell::new(truncate(&stat.description, 40)),
                Cell::new(stat.extra_count).fg(Color::Yellow),
                Cell::new(truncate(&examples, 30)),
            ]);
        }

        println!("{table}");
    }

    if !report.top_issues.is_empty() {
        println!("\n{}:", style("Top Issues by Pattern").magenta());
        for (i, issue) in report.top_issues.iter().enumerate() {
            println!(
                "  {}. {} ({} failures in {})",
                i + 1,
                style(&issue.description).white(),
                style(issue.failure_count).red(),
                style(&issue.category).cyan()
            );
        }
    }
}

fn print_summary(summary: &Summary) {
    let pass_rate = summary.pass_rate() * 100.0;
    let status = if summary.failed == 0 {
        style("PASS").green().bold()
    } else {
        style("FAIL").red().bold()
    };

    let passed_str = if summary.failed == 0 {
        style(summary.passed.to_string()).green()
    } else {
        style(summary.passed.to_string()).white()
    };

    println!(
        "Overall: {}  {}/{} passing ({:.1}%)",
        status, passed_str, summary.total_files, pass_rate
    );

    println!(
        "Errors:  {} missing, {} extra",
        if summary.missing_errors > 0 {
            style(summary.missing_errors.to_string()).red()
        } else {
            style(summary.missing_errors.to_string()).white()
        },
        if summary.extra_errors > 0 {
            style(summary.extra_errors.to_string()).yellow()
        } else {
            style(summary.extra_errors.to_string()).white()
        }
    );

    println!(
        "Time:    Zelda {}ms vs TSC {}ms ({:.1}x faster)",
        summary.zelda_total_ms, summary.tsc_total_ms, summary.speedup
    );
}

pub fn print_json(results: &[TestResult], summary: &Summary) {
    let mut categories: HashMap<String, Vec<&TestResult>> = HashMap::new();
    for result in results {
        let cat = result.category().unwrap_or("uncategorized").to_string();
        categories.entry(cat).or_default().push(result);
    }

    let cat_results: Vec<CategoryResult> = categories
        .iter()
        .map(|(name, results)| CategoryResult::from_results(name.clone(), results))
        .collect();

    let json_results: Vec<JsonTestResult> = results.iter().map(JsonTestResult::from).collect();

    let report = JsonReport {
        summary: summary.clone(),
        categories: cat_results,
        results: json_results,
    };

    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

pub fn print_compact(summary: &Summary) {
    let status = if summary.failed == 0 {
        style("PASS").green().bold()
    } else {
        style("FAIL").red().bold()
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

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_bar() {
        assert_eq!(make_progress_bar(0.0), "[░░░░░░░░░░]");
        assert_eq!(make_progress_bar(0.5), "[█████░░░░░]");
        assert_eq!(make_progress_bar(1.0), "[██████████]");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("this is a long string", 10), "this is...");
    }
}
