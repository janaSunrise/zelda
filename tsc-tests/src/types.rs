use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticError {
    pub code: u32,
    pub message: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug)]
pub struct TestResult {
    pub file: PathBuf,
    pub zelda_errors: Vec<DiagnosticError>,
    pub tsc_errors: Vec<DiagnosticError>,
    pub matched: Vec<u32>,    // Error codes found by both
    pub missing: Vec<u32>,    // tsc found, zelda didn't
    pub extra: Vec<u32>,      // zelda found, tsc didn't
    pub zelda_ms: u64,
    pub tsc_ms: u64,
}

impl TestResult {
    pub fn passed(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub total_files: usize,
    pub passed: usize,
    pub failed: usize,
    pub matched_errors: usize,
    pub missing_errors: usize,
    pub extra_errors: usize,
    pub zelda_total_ms: u64,
    pub tsc_total_ms: u64,
    pub speedup: f64,
}

impl Summary {
    pub fn from_results(results: &[TestResult]) -> Self {
        let total_files = results.len();
        let passed = results.iter().filter(|r| r.passed()).count();
        let failed = total_files - passed;

        let matched_errors: usize = results.iter().map(|r| r.matched.len()).sum();
        let missing_errors: usize = results.iter().map(|r| r.missing.len()).sum();
        let extra_errors: usize = results.iter().map(|r| r.extra.len()).sum();

        let zelda_total_ms: u64 = results.iter().map(|r| r.zelda_ms).sum();
        let tsc_total_ms: u64 = results.iter().map(|r| r.tsc_ms).sum();

        let speedup = if zelda_total_ms > 0 {
            tsc_total_ms as f64 / zelda_total_ms as f64
        } else {
            0.0
        };

        Self {
            total_files,
            passed,
            failed,
            matched_errors,
            missing_errors,
            extra_errors,
            zelda_total_ms,
            tsc_total_ms,
            speedup,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JsonReport {
    pub summary: Summary,
    pub results: Vec<JsonTestResult>,
}

#[derive(Debug, Serialize)]
pub struct JsonTestResult {
    pub file: String,
    pub passed: bool,
    pub matched: Vec<u32>,
    pub missing: Vec<u32>,
    pub extra: Vec<u32>,
    pub zelda_ms: u64,
    pub tsc_ms: u64,
}

impl From<&TestResult> for JsonTestResult {
    fn from(result: &TestResult) -> Self {
        Self {
            file: result.file.display().to_string(),
            passed: result.passed(),
            matched: result.matched.clone(),
            missing: result.missing.clone(),
            extra: result.extra.clone(),
            zelda_ms: result.zelda_ms,
            tsc_ms: result.tsc_ms,
        }
    }
}
