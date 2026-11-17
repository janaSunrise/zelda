//! Which tests are in scope for zelda.
//!
//! Some tests exercise things zelda deliberately does not model: JavaScript
//! files, custom lib configurations, non-strict checking, module resolution
//! settings. Comparing those would only add noise, so they are skipped with
//! a reason (shown in the report). Pass `--all` to run them anyway.

use std::collections::BTreeMap;

use crate::source::TestFile;

/// File extensions zelda checks.
const TS_EXTENSIONS: [&str; 4] = [".ts", ".tsx", ".mts", ".cts"];

/// Options that change semantics zelda doesn't model, and the values that
/// make a test out of scope (`None` = any value).
const UNSUPPORTED_OPTIONS: &[(&str, Option<&str>, &str)] = &[
    ("allowjs", Some("true"), "JavaScript files (allowJs)"),
    ("checkjs", Some("true"), "JavaScript files (checkJs)"),
    ("strict", Some("false"), "non-strict mode"),
    ("strictnullchecks", Some("false"), "non-strict mode"),
    ("noimplicitany", Some("false"), "non-strict mode"),
    ("nolib", Some("true"), "custom lib (noLib)"),
    ("lib", None, "custom lib (@lib)"),
    ("libfiles", None, "custom lib (@libFiles)"),
    ("moduleresolution", None, "module resolution config"),
    ("paths", None, "module resolution config"),
    ("baseurl", None, "module resolution config"),
    ("typeroots", None, "module resolution config"),
    ("types", None, "module resolution config"),
    ("rootdirs", None, "module resolution config"),
    ("currentdirectory", None, "module resolution config"),
    ("pretty", Some("true"), "pretty diagnostics output"),
];

/// Why a test can't be meaningfully compared, or `None` if it can.
///
/// `options` are the effective settings: the test's directives with the
/// chosen baseline configuration applied on top.
pub fn unsupported_reason(test: &TestFile, options: &BTreeMap<String, String>) -> Option<String> {
    if test.has_symlinks {
        return Some("symlinks".into());
    }
    for unit in &test.units {
        let name = unit.name.to_ascii_lowercase();
        if name.contains("node_modules/") || name.ends_with("package.json") {
            return Some("node_modules / package.json resolution".into());
        }
        if name.ends_with("tsconfig.json") {
            return Some("tsconfig.json".into());
        }
        if !TS_EXTENSIONS.iter().any(|ext| name.ends_with(ext)) {
            return Some("non-TypeScript files".into());
        }
        if name.contains('\\') || name.split('/').any(|part| part == "..") {
            return Some("unusual file path".into());
        }
    }
    for (key, value, reason) in UNSUPPORTED_OPTIONS {
        if let Some(actual) = options.get(*key) {
            let actual = actual.to_ascii_lowercase();
            if value.is_none_or(|v| actual == v) {
                return Some((*reason).into());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(text: &str) -> Option<String> {
        let test = TestFile::parse("compiler/t.ts", text);
        unsupported_reason(&test, &test.settings)
    }

    #[test]
    fn plain_test_is_supported() {
        assert_eq!(reason("// @target: esnext\nlet x = 1;"), None);
    }

    #[test]
    fn skips_js_and_config_files() {
        assert_eq!(
            reason("// @filename: a.js\nvar x;").as_deref(),
            Some("non-TypeScript files")
        );
        assert_eq!(
            reason("// @filename: tsconfig.json\n{}").as_deref(),
            Some("tsconfig.json")
        );
    }

    #[test]
    fn skips_non_strict_and_custom_lib() {
        assert_eq!(
            reason("// @strict: false\nlet x;").as_deref(),
            Some("non-strict mode")
        );
        assert_eq!(
            reason("// @Lib: es5\nlet x;").as_deref(),
            Some("custom lib (@lib)")
        );
        assert_eq!(reason("// @strict: true\nlet x;"), None);
    }
}
