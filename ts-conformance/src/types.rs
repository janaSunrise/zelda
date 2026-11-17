//! Per-test results.

use std::fmt;

use serde::Serialize;

use crate::diagnostic::Diagnostic;

/// What happened when running one test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum Outcome {
    /// zelda reported exactly tsc's diagnostics.
    Pass,
    /// zelda's diagnostics differ from tsc's.
    Fail,
    /// oxc failed to parse a file that tsc accepts (or reports differently).
    ParseError,
    /// The checker panicked, overflowed its stack, or otherwise died.
    Crash(String),
    Timeout,
    /// Not compared; the reason says why.
    Skipped(String),
}

impl Outcome {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::ParseError => "parse error",
            Self::Crash(_) => "crash",
            Self::Timeout => "timeout",
            Self::Skipped(_) => "skipped",
        }
    }

    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped(_))
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Crash(detail) | Self::Skipped(detail) => write!(f, "{} ({detail})", self.label()),
            _ => f.write_str(self.label()),
        }
    }
}

/// The result of running one test.
#[derive(Debug, Clone, Serialize)]
pub struct TestResult {
    /// Path relative to `tests/cases`, e.g. `compiler/foo.ts`.
    pub id: String,
    /// The baseline configuration compared against, e.g. `foo(strict=true)`.
    pub config: Option<String>,
    pub outcome: Outcome,
    /// tsc's diagnostics.
    pub expected: Vec<Diagnostic>,
    /// zelda's diagnostics.
    pub actual: Vec<Diagnostic>,
    /// Syntax errors from the parser, if any.
    pub parse_errors: Vec<String>,
    /// In `expected` but not `actual`.
    pub missing: Vec<Diagnostic>,
    /// In `actual` but not `expected`.
    pub extra: Vec<Diagnostic>,
    pub duration_ms: u64,
}

impl TestResult {
    pub fn new(id: impl Into<String>, outcome: Outcome) -> Self {
        Self {
            id: id.into(),
            config: None,
            outcome,
            expected: Vec::new(),
            actual: Vec::new(),
            parse_errors: Vec::new(),
            missing: Vec::new(),
            extra: Vec::new(),
            duration_ms: 0,
        }
    }

    /// Grouping for the report: `compiler`, or `conformance/<area>`.
    pub fn category(&self) -> &str {
        let mut parts = self.id.splitn(3, '/');
        let suite = parts.next().unwrap_or_default();
        match (suite, parts.next(), parts.next()) {
            ("conformance", Some(area), Some(_)) => &self.id[..suite.len() + 1 + area.len()],
            _ => suite,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        let r = |id: &str| TestResult::new(id, Outcome::Pass);
        assert_eq!(r("compiler/foo.ts").category(), "compiler");
        assert_eq!(
            r("conformance/types/union/u.ts").category(),
            "conformance/types"
        );
        assert_eq!(r("conformance/top.ts").category(), "conformance");
    }
}
