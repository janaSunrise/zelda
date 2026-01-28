//! Error comparison logic between zelda and tsc.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::types::{DiagnosticError, TestResult};

/// Compare errors from zelda and tsc, returning a TestResult.
///
/// Comparison is based on error codes only, not line numbers or messages,
/// since the exact wording and positions may differ slightly.
pub fn compare_errors(
    file: PathBuf,
    zelda_errors: Vec<DiagnosticError>,
    tsc_errors: Vec<DiagnosticError>,
    zelda_ms: u64,
    tsc_ms: u64,
) -> TestResult {
    // Collect unique error codes from each
    let zelda_codes: HashSet<u32> = zelda_errors.iter().map(|e| e.code).collect();
    let tsc_codes: HashSet<u32> = tsc_errors.iter().map(|e| e.code).collect();

    // Find matched, missing, and extra error codes
    let matched: Vec<u32> = zelda_codes.intersection(&tsc_codes).copied().collect();
    let missing: Vec<u32> = tsc_codes.difference(&zelda_codes).copied().collect();
    let extra: Vec<u32> = zelda_codes.difference(&tsc_codes).copied().collect();

    TestResult {
        file,
        zelda_errors,
        tsc_errors,
        matched,
        missing,
        extra,
        zelda_ms,
        tsc_ms,
    }
}

/// Compare with tolerance for error count differences.
///
/// This is useful when zelda might report multiple instances of the same error
/// while tsc consolidates them.
#[allow(dead_code)]
pub fn compare_errors_with_counts(
    file: PathBuf,
    zelda_errors: Vec<DiagnosticError>,
    tsc_errors: Vec<DiagnosticError>,
    zelda_ms: u64,
    tsc_ms: u64,
) -> TestResult {
    use std::collections::HashMap;

    // Count occurrences of each error code
    let zelda_counts: HashMap<u32, usize> = zelda_errors.iter().fold(HashMap::new(), |mut acc, e| {
        *acc.entry(e.code).or_insert(0) += 1;
        acc
    });

    let tsc_counts: HashMap<u32, usize> = tsc_errors.iter().fold(HashMap::new(), |mut acc, e| {
        *acc.entry(e.code).or_insert(0) += 1;
        acc
    });

    let all_codes: HashSet<u32> = zelda_counts.keys().chain(tsc_counts.keys()).copied().collect();

    let mut matched = Vec::new();
    let mut missing = Vec::new();
    let mut extra = Vec::new();

    for code in all_codes {
        let zelda_count = *zelda_counts.get(&code).unwrap_or(&0);
        let tsc_count = *tsc_counts.get(&code).unwrap_or(&0);

        if zelda_count > 0 && tsc_count > 0 {
            matched.push(code);
        } else if tsc_count > 0 {
            missing.push(code);
        } else {
            extra.push(code);
        }
    }

    TestResult {
        file,
        zelda_errors,
        tsc_errors,
        matched,
        missing,
        extra,
        zelda_ms,
        tsc_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_error(code: u32) -> DiagnosticError {
        DiagnosticError {
            code,
            message: format!("Error {}", code),
            line: 1,
            column: 1,
        }
    }

    #[test]
    fn test_exact_match() {
        let zelda = vec![make_error(2322), make_error(2339)];
        let tsc = vec![make_error(2322), make_error(2339)];

        let result = compare_errors("test.ts".into(), zelda, tsc, 10, 100);

        assert!(result.passed());
        assert_eq!(result.matched.len(), 2);
        assert!(result.missing.is_empty());
        assert!(result.extra.is_empty());
    }

    #[test]
    fn test_missing_errors() {
        let zelda = vec![make_error(2322)];
        let tsc = vec![make_error(2322), make_error(2339)];

        let result = compare_errors("test.ts".into(), zelda, tsc, 10, 100);

        assert!(!result.passed());
        assert_eq!(result.matched, vec![2322]);
        assert_eq!(result.missing, vec![2339]);
        assert!(result.extra.is_empty());
    }

    #[test]
    fn test_extra_errors() {
        let zelda = vec![make_error(2322), make_error(2339)];
        let tsc = vec![make_error(2322)];

        let result = compare_errors("test.ts".into(), zelda, tsc, 10, 100);

        assert!(!result.passed());
        assert_eq!(result.matched, vec![2322]);
        assert!(result.missing.is_empty());
        assert_eq!(result.extra, vec![2339]);
    }

    #[test]
    fn test_no_errors() {
        let zelda: Vec<DiagnosticError> = vec![];
        let tsc: Vec<DiagnosticError> = vec![];

        let result = compare_errors("test.ts".into(), zelda, tsc, 10, 100);

        assert!(result.passed());
        assert!(result.matched.is_empty());
    }
}
