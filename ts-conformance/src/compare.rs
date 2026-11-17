//! Comparing zelda's diagnostics against tsc's.

use std::collections::HashMap;

use crate::diagnostic::Diagnostic;

/// Diagnostics tsc reported that zelda didn't, and vice versa.
#[derive(Debug, Default, Clone)]
pub struct Diff {
    pub missing: Vec<Diagnostic>,
    pub extra: Vec<Diagnostic>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty()
    }
}

/// Multiset difference keyed by `(file, line, code)`: reporting the same error
/// twice on a line where tsc reports it once counts as one extra.
pub fn diff(expected: &[Diagnostic], actual: &[Diagnostic]) -> Diff {
    let mut remaining: HashMap<_, usize> = HashMap::new();
    for d in actual {
        *remaining.entry(d.key()).or_default() += 1;
    }

    let mut missing = Vec::new();
    for d in expected {
        match remaining.get_mut(&d.key()) {
            Some(n) if *n > 0 => *n -= 1,
            _ => missing.push(d.clone()),
        }
    }

    // Whatever is left over in `actual` is extra, preserving input order.
    let mut extra = Vec::new();
    for d in actual {
        if let Some(n) = remaining.get_mut(&d.key())
            && *n > 0
        {
            *n -= 1;
            extra.push(d.clone());
        }
    }
    Diff { missing, extra }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(line: u32, code: u32) -> Diagnostic {
        Diagnostic {
            file: "a.ts".into(),
            line,
            col: 1,
            code,
            message: String::new(),
        }
    }

    #[test]
    fn identical_sets_match() {
        assert!(diff(&[d(1, 2322), d(3, 2304)], &[d(3, 2304), d(1, 2322)]).is_empty());
    }

    #[test]
    fn columns_and_messages_are_ignored() {
        let mut ours = d(1, 2322);
        ours.col = 9;
        ours.message = "different words".into();
        assert!(diff(&[d(1, 2322)], &[ours]).is_empty());
    }

    #[test]
    fn reports_missing_and_extra() {
        let result = diff(&[d(1, 2322), d(2, 2322)], &[d(1, 2322), d(5, 2339)]);
        assert_eq!(result.missing, vec![d(2, 2322)]);
        assert_eq!(result.extra, vec![d(5, 2339)]);
    }

    #[test]
    fn counts_duplicates() {
        let result = diff(&[d(1, 2322)], &[d(1, 2322), d(1, 2322)]);
        assert!(result.missing.is_empty());
        assert_eq!(result.extra, vec![d(1, 2322)]);

        let result = diff(&[d(1, 2322), d(1, 2322)], &[d(1, 2322)]);
        assert_eq!(result.missing, vec![d(1, 2322)]);
    }
}
