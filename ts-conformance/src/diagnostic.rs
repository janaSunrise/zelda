//! Diagnostic data structures.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: u32,
    pub message: String,
    pub line: u32,
    pub col: u32,
}

pub fn error_description(code: u32) -> &'static str {
    match code {
        2300 => "Duplicate identifier",
        2304 => "Cannot find name",
        2307 => "Cannot find module",
        2322 => "Type not assignable",
        2339 => "Property does not exist on type",
        2345 => "Argument not assignable to parameter",
        2532 => "Object is possibly 'undefined'",
        2551 => "Property does not exist (did you mean?)",
        2769 => "No overload matches this call",
        5083 => "Cannot read file",
        _ => "Unknown error",
    }
}
