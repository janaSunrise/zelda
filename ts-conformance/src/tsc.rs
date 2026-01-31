//! Run TypeScript's tsc and parse output.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use regex::Regex;

use crate::diagnostic::Diagnostic;

pub struct TscResult {
    pub errors: Vec<Diagnostic>,
    pub duration_ms: u64,
}

pub fn run_tsc(file: &Path) -> Result<TscResult, String> {
    let start = Instant::now();

    let output = Command::new("npx")
        .args(["--yes", "tsc", "--noEmit", "--strict"])
        .arg(file)
        .output()
        .or_else(|_| {
            Command::new("tsc")
                .args(["--noEmit", "--strict"])
                .arg(file)
                .output()
        })
        .map_err(|e| format!("Failed to run tsc: {}", e))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let errors = parse_tsc_output(&stdout);

    Ok(TscResult { errors, duration_ms })
}

pub fn parse_tsc_output(output: &str) -> Vec<Diagnostic> {
    let re = Regex::new(r"[^(]+\((\d+),(\d+)\): error TS(\d+): (.+)").unwrap();

    output
        .lines()
        .filter_map(|line| {
            re.captures(line).map(|cap| Diagnostic {
                code: cap[3].parse().unwrap_or(0),
                message: cap[4].to_string(),
                line: cap[1].parse().unwrap_or(1),
                col: cap[2].parse().unwrap_or(1),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tsc_output() {
        let output = "test.ts(5,7): error TS2322: Type 'string' is not assignable to type 'number'.
test.ts(10,5): error TS2339: Property 'foo' does not exist on type 'object'.";

        let errors = parse_tsc_output(output);

        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].line, 5);
        assert_eq!(errors[0].code, 2322);
        assert_eq!(errors[1].line, 10);
        assert_eq!(errors[1].code, 2339);
    }

    #[test]
    fn test_parse_empty_output() {
        assert!(parse_tsc_output("").is_empty());
    }
}
