//! The committed list of passing tests, used as a regression ratchet.
//!
//! `passing.txt` lists every test id that currently passes, one per line.
//! A run fails if any of them no longer pass. When more tests start passing,
//! `--update` adds them, so progress is locked in and visible in review.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use crate::types::TestResult;

const HEADER: &str = "\
# Conformance tests that zelda passes. Regenerate with:
#   cargo conformance --update
# A run fails if any test listed here stops passing.
";

#[derive(Debug, Default)]
pub struct Expectations {
    pub passing: BTreeSet<String>,
}

/// How a run compares to the committed expectations.
#[derive(Debug, Default)]
pub struct Delta {
    /// Expected to pass, but didn't.
    pub regressions: Vec<String>,
    /// Passed, but not listed yet.
    pub new_passes: Vec<String>,
}

impl Expectations {
    /// Load the list; a missing file is an empty list.
    pub fn load(path: &Path) -> io::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let passing = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
        Ok(Self { passing })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut text = String::from(HEADER);
        for id in &self.passing {
            text.push_str(id);
            text.push('\n');
        }
        std::fs::write(path, text)
    }

    /// Compare against the results of a (possibly filtered) run. Tests that
    /// weren't run are left alone.
    pub fn delta(&self, results: &[TestResult]) -> Delta {
        let mut delta = Delta::default();
        for r in results {
            match (self.passing.contains(&r.id), r.outcome.is_pass()) {
                (true, false) => delta.regressions.push(r.id.clone()),
                (false, true) => delta.new_passes.push(r.id.clone()),
                _ => {}
            }
        }
        delta
    }

    /// Update the list from a run: tests that ran are set to their current
    /// status; tests that didn't run keep their entry.
    pub fn update(&mut self, results: &[TestResult]) {
        for r in results {
            if r.outcome.is_pass() {
                self.passing.insert(r.id.clone());
            } else {
                self.passing.remove(&r.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Outcome;

    fn result(id: &str, outcome: Outcome) -> TestResult {
        TestResult::new(id, outcome)
    }

    #[test]
    fn delta_and_update() {
        let mut exp = Expectations {
            passing: ["a", "b", "untouched"].map(String::from).into(),
        };
        let results = vec![
            result("a", Outcome::Pass),
            result("b", Outcome::Fail),
            result("c", Outcome::Pass),
            result("d", Outcome::Crash("boom".into())),
        ];

        let delta = exp.delta(&results);
        assert_eq!(delta.regressions, vec!["b"]);
        assert_eq!(delta.new_passes, vec!["c"]);

        exp.update(&results);
        let ids: Vec<_> = exp.passing.iter().map(String::as_str).collect();
        assert_eq!(ids, vec!["a", "c", "untouched"]);
    }

    #[test]
    fn round_trips_through_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("passing.txt");
        assert!(Expectations::load(&path).unwrap().passing.is_empty());

        let exp = Expectations {
            passing: ["compiler/x.ts", "compiler/a.ts"].map(String::from).into(),
        };
        exp.save(&path).unwrap();
        assert_eq!(Expectations::load(&path).unwrap().passing, exp.passing);
    }
}
