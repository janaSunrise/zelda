//! Test orchestration - runs zelda and tsc on all fixtures.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::compare::compare_errors;
use crate::tsc::run_tsc;
use crate::types::TestResult;
use crate::zelda::run_zelda;

/// Discover all TypeScript test fixtures in a directory.
pub fn discover_fixtures(fixtures_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    discover_recursive(fixtures_dir, &mut files);
    files.sort();
    files
}

fn discover_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                discover_recursive(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "ts") {
                files.push(path);
            }
        }
    }
}

/// Run a single test file through both zelda and tsc.
pub fn run_single_test(file: &Path, zelda_binary: &Path, iterations: usize) -> TestResult {
    // Run multiple iterations and take the minimum time for more stable benchmarks
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

    // Handle case where neither succeeded
    if best_zelda_ms == u64::MAX {
        best_zelda_ms = 0;
    }
    if best_tsc_ms == u64::MAX {
        best_tsc_ms = 0;
    }

    compare_errors(
        file.to_path_buf(),
        zelda_errors,
        tsc_errors,
        best_zelda_ms,
        best_tsc_ms,
    )
}

/// Run all tests in parallel.
pub fn run_all_tests(
    fixtures: &[PathBuf],
    zelda_binary: &Path,
    iterations: usize,
) -> Vec<TestResult> {
    fixtures
        .par_iter()
        .map(|file| run_single_test(file, zelda_binary, iterations))
        .collect()
}

/// Run tests sequentially (useful for debugging).
#[allow(dead_code)]
pub fn run_all_tests_sequential(
    fixtures: &[PathBuf],
    zelda_binary: &Path,
    iterations: usize,
) -> Vec<TestResult> {
    fixtures
        .iter()
        .map(|file| run_single_test(file, zelda_binary, iterations))
        .collect()
}
