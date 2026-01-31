//! Run zelda and parse JSON output.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde::Deserialize;

use crate::diagnostic::Diagnostic;

#[derive(Debug, Deserialize)]
struct ZeldaFileResult {
    #[allow(dead_code)]
    file: String,
    errors: Vec<ZeldaError>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ZeldaError {
    Type(ZeldaTypeError),
    Binding(ZeldaBindingError),
    ModuleNotFound { specifier: String },
    FileReadError { path: String, error: String },
}

#[derive(Debug, Deserialize)]
struct ZeldaTypeError {
    message: String,
    code: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ZeldaBindingError {
    DuplicateSymbol { name: String },
    UndefinedSymbol { name: String },
}

#[derive(Debug, Deserialize)]
struct ZeldaOutput {
    files: Vec<ZeldaFileResult>,
}

pub struct ZeldaResult {
    pub errors: Vec<Diagnostic>,
    pub duration_ms: u64,
}

pub fn run_zelda(file: &Path, zelda_binary: &Path) -> Result<ZeldaResult, String> {
    let start = Instant::now();

    let output = Command::new(zelda_binary)
        .args(["check", "--format", "json"])
        .arg(file)
        .output()
        .map_err(|e| format!("Failed to run zelda: {}", e))?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.trim().is_empty() {
        return Err(format!(
            "Zelda produced no output. stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let zelda_output: ZeldaOutput =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse zelda output: {}", e))?;

    let mut errors = Vec::new();
    for file_result in zelda_output.files {
        for error in file_result.errors {
            if let Some(diag) = convert_zelda_error(&error) {
                errors.push(diag);
            }
        }
    }

    Ok(ZeldaResult { errors, duration_ms })
}

fn convert_zelda_error(error: &ZeldaError) -> Option<Diagnostic> {
    match error {
        ZeldaError::Type(e) => Some(Diagnostic {
            code: e.code,
            message: e.message.clone(),
            line: 1,
            col: 1,
        }),
        ZeldaError::Binding(ZeldaBindingError::DuplicateSymbol { name }) => Some(Diagnostic {
            code: 2300,
            message: format!("Duplicate identifier '{}'.", name),
            line: 1,
            col: 1,
        }),
        ZeldaError::Binding(ZeldaBindingError::UndefinedSymbol { name }) => Some(Diagnostic {
            code: 2304,
            message: format!("Cannot find name '{}'.", name),
            line: 1,
            col: 1,
        }),
        ZeldaError::ModuleNotFound { specifier } => Some(Diagnostic {
            code: 2307,
            message: format!("Cannot find module '{}'.", specifier),
            line: 1,
            col: 1,
        }),
        ZeldaError::FileReadError { path, error } => Some(Diagnostic {
            code: 5083,
            message: format!("Cannot read file '{}': {}", path, error),
            line: 1,
            col: 1,
        }),
    }
}
