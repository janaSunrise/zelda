use std::path::Path;
use std::process::Command;
use std::time::Instant;

use regex::Regex;

use crate::types::DiagnosticError;

pub struct TscResult {
    pub errors: Vec<DiagnosticError>,
    pub duration_ms: u64,
}

pub fn run_tsc(file: &Path) -> Result<TscResult, String> {
    let start = Instant::now();

    // Try npx tsc first (for local installations), fall back to global tsc
    // Note: npx will find TypeScript if installed in node_modules
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
        .map_err(|e| format!("Failed to run tsc (is TypeScript installed?): {}", e))?;

    let duration_ms = start.elapsed().as_millis() as u64;

    // tsc outputs errors to stdout
    let stdout = String::from_utf8_lossy(&output.stdout);
    let errors = parse_tsc_output(&stdout);

    Ok(TscResult { errors, duration_ms })
}

/// Parse tsc output and extract error information.
///
/// TSC output format:
/// `file.ts(line,col): error TS{code}: message`
pub fn parse_tsc_output(output: &str) -> Vec<DiagnosticError> {
    // Pattern matches: filename(line,column): error TScode: message
    let re = Regex::new(r"[^(]+\((\d+),(\d+)\): error TS(\d+): (.+)").unwrap();

    output
        .lines()
        .filter_map(|line| {
            re.captures(line).map(|cap| DiagnosticError {
                line: cap[1].parse().unwrap_or(1),
                column: cap[2].parse().unwrap_or(1),
                code: cap[3].parse().unwrap_or(0),
                message: cap[4].to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tsc_output() {
        let output = r#"test.ts(5,7): error TS2322: Type 'string' is not assignable to type 'number'.
test.ts(10,5): error TS2339: Property 'foo' does not exist on type 'object'."#;

        let errors = parse_tsc_output(output);

        assert_eq!(errors.len(), 2);

        assert_eq!(errors[0].line, 5);
        assert_eq!(errors[0].column, 7);
        assert_eq!(errors[0].code, 2322);
        assert!(errors[0].message.contains("not assignable"));

        assert_eq!(errors[1].line, 10);
        assert_eq!(errors[1].column, 5);
        assert_eq!(errors[1].code, 2339);
        assert!(errors[1].message.contains("does not exist"));
    }

    #[test]
    fn test_parse_empty_output() {
        let errors = parse_tsc_output("");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_parse_no_errors() {
        let output = "Some other output that doesn't match the error pattern";
        let errors = parse_tsc_output(output);
        assert!(errors.is_empty());
    }
}
