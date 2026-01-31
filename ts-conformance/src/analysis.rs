//! Error code analysis.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::compare::MatchKind;
use crate::diagnostic::error_description;
use crate::types::TestResult;

#[derive(Debug, Clone)]
pub struct ErrorCodeStats {
    pub code: u32,
    pub description: String,
    pub missing_count: usize,
    pub extra_count: usize,
    pub example_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct IssuePattern {
    pub description: String,
    pub failure_count: usize,
    pub category: String,
}

#[derive(Debug, Clone)]
pub struct AnalysisReport {
    pub missing_codes: Vec<ErrorCodeStats>,
    pub extra_codes: Vec<ErrorCodeStats>,
    pub top_issues: Vec<IssuePattern>,
}

impl AnalysisReport {
    pub fn from_results(results: &[TestResult]) -> Self {
        let mut missing_map: HashMap<u32, ErrorCodeStats> = HashMap::new();
        let mut extra_map: HashMap<u32, ErrorCodeStats> = HashMap::new();
        let mut category_failures: HashMap<String, Vec<PathBuf>> = HashMap::new();

        for result in results {
            let file = result.test.path.clone();
            let category = result.test.category.clone().unwrap_or_else(|| "uncategorized".to_string());

            if !result.passed() {
                category_failures.entry(category.clone()).or_default().push(file.clone());
            }

            for m in &result.comparison.matches {
                match m.kind {
                    MatchKind::Missing => {
                        if let Some(ref diag) = m.theirs {
                            let entry = missing_map.entry(diag.code).or_insert_with(|| ErrorCodeStats {
                                code: diag.code,
                                description: error_description(diag.code).to_string(),
                                missing_count: 0,
                                extra_count: 0,
                                example_files: Vec::new(),
                            });
                            entry.missing_count += 1;
                            if entry.example_files.len() < 3 {
                                entry.example_files.push(file.clone());
                            }
                        }
                    }
                    MatchKind::Extra => {
                        if let Some(ref diag) = m.ours {
                            let entry = extra_map.entry(diag.code).or_insert_with(|| ErrorCodeStats {
                                code: diag.code,
                                description: error_description(diag.code).to_string(),
                                missing_count: 0,
                                extra_count: 0,
                                example_files: Vec::new(),
                            });
                            entry.extra_count += 1;
                            if entry.example_files.len() < 3 {
                                entry.example_files.push(file.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut missing_codes: Vec<_> = missing_map.into_values().collect();
        missing_codes.sort_by(|a, b| b.missing_count.cmp(&a.missing_count));

        let mut extra_codes: Vec<_> = extra_map.into_values().collect();
        extra_codes.sort_by(|a, b| b.extra_count.cmp(&a.extra_count));

        let mut top_issues: Vec<IssuePattern> = category_failures
            .into_iter()
            .filter(|(_, files)| files.len() >= 2)
            .map(|(category, files)| IssuePattern {
                description: format!("Failures in {}", category),
                failure_count: files.len(),
                category,
            })
            .collect();
        top_issues.sort_by(|a, b| b.failure_count.cmp(&a.failure_count));
        top_issues.truncate(5);

        Self { missing_codes, extra_codes, top_issues }
    }

    pub fn top_missing(&self, n: usize) -> &[ErrorCodeStats] {
        &self.missing_codes[..n.min(self.missing_codes.len())]
    }

    pub fn top_extra(&self, n: usize) -> &[ErrorCodeStats] {
        &self.extra_codes[..n.min(self.extra_codes.len())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::{CompareResult, Match};
    use crate::diagnostic::Diagnostic;
    use crate::types::TestCase;

    fn make_result(path: &str, missing_codes: &[u32], extra_codes: &[u32]) -> TestResult {
        let test = TestCase::from_path(PathBuf::from(path));
        let mut matches = Vec::new();

        for &code in missing_codes {
            matches.push(Match {
                ours: None,
                theirs: Some(Diagnostic { code, message: "Missing".to_string(), line: 1, col: 1 }),
                kind: MatchKind::Missing,
            });
        }
        for &code in extra_codes {
            matches.push(Match {
                ours: Some(Diagnostic { code, message: "Extra".to_string(), line: 1, col: 1 }),
                theirs: None,
                kind: MatchKind::Extra,
            });
        }

        let comparison = CompareResult {
            matches,
            missing: missing_codes.len(),
            extra: extra_codes.len(),
            zelda_ms: 10,
            tsc_ms: 100,
        };
        TestResult { test, comparison }
    }

    #[test]
    fn test_analysis_counts() {
        let results = vec![
            make_result("tests/unions/foo.ts", &[2322, 2339], &[]),
            make_result("tests/unions/bar.ts", &[2322], &[2304]),
        ];
        let report = AnalysisReport::from_results(&results);
        assert_eq!(report.missing_codes[0].code, 2322);
        assert_eq!(report.missing_codes[0].missing_count, 2);
    }

    #[test]
    fn test_category_patterns() {
        let results = vec![
            make_result("tests/unions/a.ts", &[2322], &[]),
            make_result("tests/unions/b.ts", &[2322], &[]),
        ];
        let report = AnalysisReport::from_results(&results);
        assert_eq!(report.top_issues[0].category, "unions");
    }
}
