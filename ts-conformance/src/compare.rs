//! Error comparison between zelda and tsc.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::diagnostic::Diagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Matched,
    Missing,
    Extra,
}

#[derive(Debug, Clone)]
pub struct Match {
    pub ours: Option<Diagnostic>,
    pub theirs: Option<Diagnostic>,
    pub kind: MatchKind,
}

#[derive(Debug, Clone)]
pub struct CompareResult {
    pub matches: Vec<Match>,
    pub missing: usize,
    pub extra: usize,
    pub zelda_ms: u64,
    pub tsc_ms: u64,
}

impl CompareResult {
    pub fn passed(&self) -> bool {
        self.missing == 0 && self.extra == 0
    }

    pub fn missing_diagnostics(&self) -> Vec<&Diagnostic> {
        self.matches
            .iter()
            .filter(|m| m.kind == MatchKind::Missing)
            .filter_map(|m| m.theirs.as_ref())
            .collect()
    }

    pub fn extra_diagnostics(&self) -> Vec<&Diagnostic> {
        self.matches
            .iter()
            .filter(|m| m.kind == MatchKind::Extra)
            .filter_map(|m| m.ours.as_ref())
            .collect()
    }
}

pub fn compare_codes(
    _file: PathBuf,
    zelda_errors: Vec<Diagnostic>,
    tsc_errors: Vec<Diagnostic>,
    zelda_ms: u64,
    tsc_ms: u64,
) -> CompareResult {
    let zelda_codes: HashSet<u32> = zelda_errors.iter().map(|e| e.code).collect();
    let tsc_codes: HashSet<u32> = tsc_errors.iter().map(|e| e.code).collect();

    let matched_codes: Vec<u32> = zelda_codes.intersection(&tsc_codes).copied().collect();
    let missing_codes: Vec<u32> = tsc_codes.difference(&zelda_codes).copied().collect();
    let extra_codes: Vec<u32> = zelda_codes.difference(&tsc_codes).copied().collect();

    let mut matches = Vec::new();

    for code in &matched_codes {
        let ours = zelda_errors.iter().find(|e| e.code == *code).cloned();
        let theirs = tsc_errors.iter().find(|e| e.code == *code).cloned();
        matches.push(Match { ours, theirs, kind: MatchKind::Matched });
    }

    for code in &missing_codes {
        let theirs = tsc_errors.iter().find(|e| e.code == *code).cloned();
        matches.push(Match { ours: None, theirs, kind: MatchKind::Missing });
    }

    for code in &extra_codes {
        let ours = zelda_errors.iter().find(|e| e.code == *code).cloned();
        matches.push(Match { ours, theirs: None, kind: MatchKind::Extra });
    }

    CompareResult {
        matches,
        missing: missing_codes.len(),
        extra: extra_codes.len(),
        zelda_ms,
        tsc_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(code: u32) -> Diagnostic {
        Diagnostic { code, message: format!("Error {}", code), line: 1, col: 1 }
    }

    #[test]
    fn test_all_match() {
        let zelda = vec![diag(2322), diag(2339)];
        let tsc = vec![diag(2322), diag(2339)];
        let result = compare_codes("test.ts".into(), zelda, tsc, 10, 100);
        assert!(result.passed());
        assert_eq!(result.missing, 0);
        assert_eq!(result.extra, 0);
    }

    #[test]
    fn test_missing() {
        let zelda = vec![];
        let tsc = vec![diag(2322)];
        let result = compare_codes("test.ts".into(), zelda, tsc, 10, 100);
        assert!(!result.passed());
        assert_eq!(result.missing, 1);
    }

    #[test]
    fn test_extra() {
        let zelda = vec![diag(2339)];
        let tsc = vec![];
        let result = compare_codes("test.ts".into(), zelda, tsc, 10, 100);
        assert!(!result.passed());
        assert_eq!(result.extra, 1);
    }

    #[test]
    fn test_both_empty() {
        let result = compare_codes("test.ts".into(), vec![], vec![], 10, 100);
        assert!(result.passed());
    }
}
