//! Test execution.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use crate::compare::compare_codes;
use crate::tsc::run_tsc;
use crate::types::{TestCase, TestResult};
use crate::zelda::run_zelda;

pub fn run_single_test(file: &Path, zelda_binary: &Path, iterations: usize) -> TestResult {
    let test = TestCase::from_path(file.to_path_buf());

    let mut best_zelda_ms = u64::MAX;
    let mut zelda_errors = Vec::new();

    for i in 0..iterations {
        match run_zelda(file, zelda_binary) {
            Ok(result) => {
                if result.duration_ms < best_zelda_ms {
                    best_zelda_ms = result.duration_ms;
                }
                if i == 0 {
                    zelda_errors = result.errors;
                }
            }
            Err(e) => {
                eprintln!("Warning: zelda failed on {}: {}", file.display(), e);
                break;
            }
        }
    }

    let mut best_tsc_ms = u64::MAX;
    let mut tsc_errors = Vec::new();

    for i in 0..iterations {
        match run_tsc(file) {
            Ok(result) => {
                if result.duration_ms < best_tsc_ms {
                    best_tsc_ms = result.duration_ms;
                }
                if i == 0 {
                    tsc_errors = result.errors;
                }
            }
            Err(e) => {
                eprintln!("Warning: tsc failed on {}: {}", file.display(), e);
                break;
            }
        }
    }

    if best_zelda_ms == u64::MAX {
        best_zelda_ms = 0;
    }
    if best_tsc_ms == u64::MAX {
        best_tsc_ms = 0;
    }

    let comparison = compare_codes(
        file.to_path_buf(),
        zelda_errors,
        tsc_errors,
        best_zelda_ms,
        best_tsc_ms,
    );

    TestResult { test, comparison }
}

pub fn run_all_tests(
    fixtures: &[PathBuf],
    zelda_binary: &Path,
    iterations: usize,
    show_progress: bool,
) -> Vec<TestResult> {
    if !show_progress || fixtures.len() < 10 {
        return fixtures
            .par_iter()
            .map(|file| run_single_test(file, zelda_binary, iterations))
            .collect();
    }

    let pb = ProgressBar::new(fixtures.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
            .unwrap()
            .progress_chars("█░░"),
    );

    let passed = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);

    let results: Vec<TestResult> = fixtures
        .par_iter()
        .map(|file| {
            let result = run_single_test(file, zelda_binary, iterations);
            if result.passed() {
                passed.fetch_add(1, Ordering::Relaxed);
            } else {
                failed.fetch_add(1, Ordering::Relaxed);
            }
            let p = passed.load(Ordering::Relaxed);
            let f = failed.load(Ordering::Relaxed);
            pb.set_message(format!("{} passed, {} failed", p, f));
            pb.inc(1);
            result
        })
        .collect();

    pb.finish_and_clear();
    results
}
