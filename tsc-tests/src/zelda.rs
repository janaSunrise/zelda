//! Run zelda and parse its JSON output.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde::Deserialize;

use crate::types::DiagnosticError;

/// Zelda's JSON output structure for a single file.
#[derive(Debug, Deserialize)]
struct ZeldaFileResult {
    file: String,
    errors: Vec<ZeldaError>,
    #[allow(dead_code)]
    error_count: usize,
    #[allow(dead_code)]
    warning_count: usize,
    #[allow(dead_code)]
    duration_ms: u64,
}

/// Zelda's error format.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ZeldaError {
    Type(ZeldaTypeError),
    Binding(ZeldaBindingError),
    ModuleNotFound {
        specifier: String,
        #[allow(dead_code)]
        from_file: String,
        span: ZeldaSpan,
    },
    FileReadError {
        path: String,
        error: String,
    },
}

#[derive(Debug, Deserialize)]
struct ZeldaTypeError {
    message: String,
    span: ZeldaSpan,
    code: u32,
    #[allow(dead_code)]
    severity: String,
    #[allow(dead_code)]
    related: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ZeldaBindingError {
    DuplicateSymbol(ZeldaDuplicateSymbol),
    UndefinedSymbol(ZeldaUndefinedSymbol),
}

#[derive(Debug, Deserialize)]
struct ZeldaDuplicateSymbol {
    name: String,
    #[allow(dead_code)]
    existing: ZeldaSpan,
    duplicate: ZeldaSpan,
}

#[derive(Debug, Deserialize)]
struct ZeldaUndefinedSymbol {
    name: String,
    span: ZeldaSpan,
    #[allow(dead_code)]
    is_type: bool,
}

#[derive(Debug, Deserialize)]
struct ZeldaSpan {
    start: u32,
    #[allow(dead_code)]
    end: u32,
}

/// Zelda's complete JSON output.
#[derive(Debug, Deserialize)]
struct ZeldaOutput {
    files: Vec<ZeldaFileResult>,
    #[allow(dead_code)]
    total_errors: usize,
    #[allow(dead_code)]
    total_warnings: usize,
    #[allow(dead_code)]
    total_duration_ms: u64,
}

/// Result of running zelda on a file.
pub struct ZeldaResult {
    pub errors: Vec<DiagnosticError>,
    pub duration_ms: u64,
}

/// Run zelda on the given file and return parsed errors.
pub fn run_zelda(file: &Path, zelda_binary: &Path) -> Result<ZeldaResult, String> {
    let start = Instant::now();

    let output = Command::new(zelda_binary)
        .args(["check", "--format", "json"])
        .arg(file)
        .output()
        .map_err(|e| format!("Failed to run zelda: {}", e))?;

    let duration_ms = start.elapsed().as_millis() as u64;

    // Zelda exits with code 1 if there are errors, but still outputs JSON
    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.trim().is_empty() {
        return Err(format!(
            "Zelda produced no output. stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let zelda_output: ZeldaOutput =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse zelda output: {}", e))?;

    // Convert zelda errors to our common format
    let mut errors = Vec::new();

    for file_result in zelda_output.files {
        for error in file_result.errors {
            if let Some(diag) = convert_zelda_error(&error, &stdout) {
                errors.push(diag);
            }
        }
    }

    Ok(ZeldaResult { errors, duration_ms })
}

/// Convert a zelda error to our common DiagnosticError format.
fn convert_zelda_error(error: &ZeldaError, source: &str) -> Option<DiagnosticError> {
    match error {
        ZeldaError::Type(e) => {
            let (line, column) = offset_to_line_col(source, e.span.start);
            Some(DiagnosticError {
                code: e.code,
                message: e.message.clone(),
                line,
                column,
            })
        }
        ZeldaError::Binding(ZeldaBindingError::DuplicateSymbol(e)) => {
            let (line, column) = offset_to_line_col(source, e.duplicate.start);
            Some(DiagnosticError {
                code: 2300, // TS2300: Duplicate identifier
                message: format!("Duplicate identifier '{}'.", e.name),
                line,
                column,
            })
        }
        ZeldaError::Binding(ZeldaBindingError::UndefinedSymbol(e)) => {
            let (line, column) = offset_to_line_col(source, e.span.start);
            Some(DiagnosticError {
                code: 2304, // TS2304: Cannot find name
                message: format!("Cannot find name '{}'.", e.name),
                line,
                column,
            })
        }
        ZeldaError::ModuleNotFound { specifier, span, .. } => {
            let (line, column) = offset_to_line_col(source, span.start);
            Some(DiagnosticError {
                code: 2307, // TS2307: Cannot find module
                message: format!("Cannot find module '{}'.", specifier),
                line,
                column,
            })
        }
        ZeldaError::FileReadError { path, error } => {
            Some(DiagnosticError {
                code: 5083, // TS5083: Cannot read file
                message: format!("Cannot read file '{}': {}", path, error),
                line: 1,
                column: 1,
            })
        }
    }
}

/// Convert a byte offset to line and column numbers (1-indexed).
fn offset_to_line_col(_source: &str, _offset: u32) -> (u32, u32) {
    // For simplicity, we'll use placeholder values since we're mainly comparing error codes
    // In a full implementation, we'd read the original file and compute line/col
    (1, 1)
}
