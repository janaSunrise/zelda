//! Aggregate statistics: which error codes zelda most often misses or
//! over-reports. This is the most direct guide to what to implement next.

use std::collections::HashMap;

use crate::diagnostic::Diagnostic;
use crate::types::TestResult;

/// How often one error code is missing (or extra) across the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeStats {
    pub code: u32,
    /// Total diagnostics with this code.
    pub occurrences: usize,
    /// Distinct tests they appear in.
    pub tests: usize,
    /// Tests that would pass if only this code were fixed.
    pub sole_blocker: usize,
    /// One message, for context.
    pub example: String,
}

/// Rank codes in `missing` (or `extra`) across failing results, most
/// impactful first.
pub fn rank_codes<'a>(
    results: &'a [TestResult],
    select: impl Fn(&'a TestResult) -> &'a [Diagnostic],
) -> Vec<CodeStats> {
    let mut stats: HashMap<u32, CodeStats> = HashMap::new();
    for r in results {
        let diags = select(r);
        let mut codes: Vec<u32> = diags.iter().map(|d| d.code).collect();
        codes.sort_unstable();
        codes.dedup();
        // The test fails only because of this code: nothing else differs.
        let sole = codes.len() == 1 && r.missing.len() + r.extra.len() == diags.len();

        for d in diags {
            let entry = stats.entry(d.code).or_insert_with(|| CodeStats {
                code: d.code,
                occurrences: 0,
                tests: 0,
                sole_blocker: 0,
                example: d.message.clone(),
            });
            entry.occurrences += 1;
        }
        for code in codes {
            let entry = stats.get_mut(&code).expect("inserted above");
            entry.tests += 1;
            entry.sole_blocker += usize::from(sole);
        }
    }

    let mut ranked: Vec<_> = stats.into_values().collect();
    ranked.sort_by(|a, b| {
        (b.sole_blocker, b.tests, b.occurrences, a.code).cmp(&(
            a.sole_blocker,
            a.tests,
            a.occurrences,
            b.code,
        ))
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Outcome;

    fn diag(code: u32) -> Diagnostic {
        Diagnostic {
            file: "a.ts".into(),
            line: 1,
            col: 1,
            code,
            message: format!("msg {code}"),
        }
    }

    #[test]
    fn ranks_by_unblocking_potential() {
        let mut only_2322 = TestResult::new("t1", Outcome::Fail);
        only_2322.missing = vec![diag(2322), diag(2322)];
        let mut mixed = TestResult::new("t2", Outcome::Fail);
        mixed.missing = vec![diag(2304), diag(2322)];
        let mut only_2304 = TestResult::new("t3", Outcome::Fail);
        only_2304.missing = vec![diag(2304)];
        only_2304.extra = vec![diag(1)];

        let ranked = rank_codes(&[only_2322, mixed, only_2304], |r| &r.missing);
        assert_eq!(ranked[0].code, 2322);
        assert_eq!(
            (
                ranked[0].occurrences,
                ranked[0].tests,
                ranked[0].sole_blocker
            ),
            (3, 2, 1)
        );
        // 2304 is in two tests but never the only problem.
        assert_eq!((ranked[1].code, ranked[1].sole_blocker), (2304, 0));
    }
}
