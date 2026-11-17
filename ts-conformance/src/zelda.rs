//! The worker side: run zelda's checker in-process on a test's files.
//!
//! The runner invokes this in a child process (`ts-conformance worker`) so
//! that a crash or hang in the checker is isolated to one test.

use std::path::Path;

use serde::{Deserialize, Serialize};
use zelda::Project;

/// A diagnostic as zelda reports it: a byte span within a test file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawDiagnostic {
    /// Path relative to the test root, with `/` separators.
    pub file: String,
    pub start: u32,
    /// TypeScript error code, or `None` for syntax errors from the parser.
    pub code: Option<u32>,
    pub message: String,
}

/// Check `files` (relative to `root`) and return all errors found, including
/// errors in files they import.
pub fn check(root: &Path, files: &[String]) -> Vec<RawDiagnostic> {
    // Project paths are canonical (on macOS, /var → /private/var), so the
    // root must be too for `strip_prefix` to work.
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut project = Project::new();
    let mut diagnostics = Vec::new();

    for file in files {
        let errors = match project.check_file(&root.join(file)) {
            Ok(errors) => errors,
            Err(err) => vec![err],
        };
        for err in errors.iter().filter(|e| !e.is_warning()) {
            let path = err.file();
            let rel = path.strip_prefix(&root).unwrap_or(path);
            diagnostics.push(RawDiagnostic {
                file: rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
                start: err.span().map_or(0, |s| s.start),
                code: err.code(),
                message: err.message(),
            });
        }
    }
    diagnostics
}
