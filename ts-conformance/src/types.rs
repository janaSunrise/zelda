//! Data types for test results.

use std::path::PathBuf;

use serde::Serialize;

use crate::compare::CompareResult;

#[derive(Debug, Clone)]
pub struct TestCase {
    pub path: PathBuf,
    pub category: Option<String>,
}

impl TestCase {
    pub fn from_path(path: PathBuf) -> Self {
        let category = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string());

        Self { path, category }
    }
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub test: TestCase,
    pub comparison: CompareResult,
}

impl TestResult {
    pub fn passed(&self) -> bool {
        self.comparison.passed()
    }

    pub fn category(&self) -> Option<&str> {
        self.test.category.as_deref()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub total_files: usize,
    pub passed: usize,
    pub failed: usize,
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

        let missing_errors: usize = results.iter().map(|r| r.comparison.missing).sum();
        let extra_errors: usize = results.iter().map(|r| r.comparison.extra).sum();

        let zelda_total_ms: u64 = results.iter().map(|r| r.comparison.zelda_ms).sum();
        let tsc_total_ms: u64 = results.iter().map(|r| r.comparison.tsc_ms).sum();

        let speedup = if zelda_total_ms > 0 {
            tsc_total_ms as f64 / zelda_total_ms as f64
        } else {
            0.0
        };

        Self {
            total_files,
            passed,
            failed,
            missing_errors,
            extra_errors,
            zelda_total_ms,
            tsc_total_ms,
            speedup,
        }
    }

    pub fn pass_rate(&self) -> f64 {
        if self.total_files == 0 {
            1.0
        } else {
            self.passed as f64 / self.total_files as f64
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryResult {
    pub name: String,
    pub total: usize,
    pub passed: usize,
    pub missing: usize,
    pub extra: usize,
    pub pass_rate: f64,
}

impl CategoryResult {
    pub fn from_results(name: String, results: &[&TestResult]) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed()).count();
        let missing: usize = results.iter().map(|r| r.comparison.missing).sum();
        let extra: usize = results.iter().map(|r| r.comparison.extra).sum();

        let pass_rate = if total == 0 {
            1.0
        } else {
            passed as f64 / total as f64
        };

        Self { name, total, passed, missing, extra, pass_rate }
    }
}

#[derive(Debug, Serialize)]
pub struct JsonReport {
    pub summary: Summary,
    pub categories: Vec<CategoryResult>,
    pub results: Vec<JsonTestResult>,
}

#[derive(Debug, Serialize)]
pub struct JsonTestResult {
    pub file: String,
    pub category: Option<String>,
    pub passed: bool,
    pub missing: usize,
    pub extra: usize,
    pub missing_codes: Vec<u32>,
    pub extra_codes: Vec<u32>,
    pub zelda_ms: u64,
    pub tsc_ms: u64,
}

impl From<&TestResult> for JsonTestResult {
    fn from(result: &TestResult) -> Self {
        let missing_codes: Vec<u32> = result
            .comparison
            .missing_diagnostics()
            .iter()
            .map(|d| d.code)
            .collect();

        let extra_codes: Vec<u32> = result
            .comparison
            .extra_diagnostics()
            .iter()
            .map(|d| d.code)
            .collect();

        Self {
            file: result.test.path.display().to_string(),
            category: result.test.category.clone(),
            passed: result.passed(),
            missing: result.comparison.missing,
            extra: result.comparison.extra,
            missing_codes,
            extra_codes,
            zelda_ms: result.comparison.zelda_ms,
            tsc_ms: result.comparison.tsc_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_extraction() {
        let test = TestCase::from_path(PathBuf::from("fixtures/generics/constraints.ts"));
        assert_eq!(test.category, Some("generics".to_string()));
    }
}
