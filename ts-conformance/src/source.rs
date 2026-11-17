//! The TypeScript test corpus: discovering test files and splitting them into
//! units the way tsc's test harness does.
//!
//! Matching the harness exactly matters: baseline line numbers refer to the
//! *unit* content, after `// @option` lines have been removed and leading blank
//! lines dropped. See `makeUnitsFromTest` in TypeScript's `harnessIO.ts` and
//! `ParseTestFilesAndSymlinks` in the Go port's `test_case_parser.go`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// Test suites under `tests/cases` that have compiler baselines.
pub const SUITES: [&str; 2] = ["compiler", "conformance"];

/// `// @name: value`. `\w` is ASCII-only in JavaScript, so it is here too.
static OPTION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^//\s*@([A-Za-z0-9_]+)\s*:\s*([^\r\n]*)").expect("valid regex"));

/// `// @link: target -> link`
static LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^//\s*@link\s*:\s*([^\r\n]*)\s*->\s*([^\r\n]*)").expect("valid regex")
});

/// A single TypeScript conformance test file.
#[derive(Debug, Clone)]
pub struct TestFile {
    /// Path relative to `tests/cases`, with `/` separators, e.g. `compiler/foo.ts`.
    pub id: String,
    /// `compiler` or `conformance`.
    pub suite: String,
    /// File name without its `.ts`/`.tsx` extension; baselines are named after it.
    pub stem: String,
    /// Compiler settings from `// @option: value` lines, keyed by lowercase name.
    pub settings: BTreeMap<String, String>,
    /// The files that make up the test, in declaration order.
    pub units: Vec<Unit>,
    /// Whether the test declares symlinks (`@link` / `@symlink`).
    pub has_symlinks: bool,
}

/// One file within a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// Normalized file name (no leading `/`), as used in baseline diagnostics.
    pub name: String,
    pub content: String,
}

/// Find all test files under `cases_dir/{compiler,conformance}`, sorted by id.
pub fn discover(cases_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut tests = Vec::new();
    for suite in SUITES {
        walk(&cases_dir.join(suite), &mut |path| {
            let is_test = path
                .extension()
                .is_some_and(|ext| ext == "ts" || ext == "tsx");
            if is_test && let Ok(rel) = path.strip_prefix(cases_dir) {
                tests.push((to_id(rel), path.to_path_buf()));
            }
        });
    }
    tests.sort();
    tests
}

fn walk(dir: &Path, visit: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}

fn to_id(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Normalize a unit or baseline file name: tsc's harness roots files at `/`,
/// and baselines print them either with or without the leading slash.
pub fn normalize_file_name(name: &str) -> String {
    name.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

impl TestFile {
    /// Parse a test file's content. `id` is its path relative to `tests/cases`.
    pub fn parse(id: &str, text: &str) -> Self {
        let file_name = id.rsplit('/').next().unwrap_or(id);
        let suite = id.split('/').next().unwrap_or_default().to_string();
        let stem = file_name
            .strip_suffix(".tsx")
            .or_else(|| file_name.strip_suffix(".ts"))
            .unwrap_or(file_name)
            .to_string();

        let mut settings = BTreeMap::new();
        let mut units = Vec::new();
        let mut has_symlinks = false;
        let mut current_name: Option<String> = None;
        let mut content = String::new();

        for line in split_lines(text) {
            if LINK_RE.is_match(line) {
                has_symlinks = true;
                continue;
            }
            let Some(caps) = OPTION_RE.captures(line) else {
                // A content line. Leading blank lines are dropped because a
                // newline is only inserted once the content is non-empty.
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(line);
                continue;
            };

            let key = caps[1].to_ascii_lowercase();
            let value = caps[2].trim().to_string();
            if key == "symlink" {
                has_symlinks = true;
            }
            if key != "filename" {
                settings.insert(key, value);
                continue;
            }

            // `// @filename:` starts a new unit. Content before the first one
            // can only be comments, and is discarded.
            if let Some(name) = current_name.take() {
                units.push(Unit {
                    name: normalize_file_name(&name),
                    content: std::mem::take(&mut content),
                });
            } else {
                content.clear();
            }
            current_name = Some(value);
        }

        let name = current_name.unwrap_or_else(|| file_name.to_string());
        units.push(Unit {
            name: normalize_file_name(&name),
            content,
        });

        Self {
            id: id.to_string(),
            suite,
            stem,
            settings,
            units,
            has_symlinks,
        }
    }

    pub fn unit(&self, name: &str) -> Option<&Unit> {
        self.units.iter().find(|u| u.name == name)
    }
}

/// Split on `\r?\n`, like the harness's `lineDelimiter`.
fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_file_strips_options_and_leading_blank_lines() {
        let text = "// @target: es2015\n// @Strict: true\n\n\nlet x = 1;\n\nlet y = 2;\n";
        let test = TestFile::parse("compiler/foo.ts", text);

        assert_eq!(test.suite, "compiler");
        assert_eq!(test.stem, "foo");
        assert_eq!(test.settings["target"], "es2015");
        assert_eq!(test.settings["strict"], "true");
        assert_eq!(
            test.units,
            vec![Unit {
                name: "foo.ts".into(),
                content: "let x = 1;\n\nlet y = 2;\n".into(),
            }]
        );
    }

    #[test]
    fn multi_file_splits_on_filename() {
        let text = "// @module: commonjs\r\n// comment before first file\r\n// @Filename: /a.ts\r\nexport const a = 1;\r\n// @filename: b.ts\r\n\r\nimport { a } from './a';";
        let test = TestFile::parse("conformance/es6/modules/bar.ts", text);

        assert_eq!(test.suite, "conformance");
        assert_eq!(
            test.settings.get("module").map(String::as_str),
            Some("commonjs")
        );
        assert_eq!(test.units.len(), 2);
        assert_eq!(test.units[0].name, "a.ts");
        assert_eq!(test.units[0].content, "export const a = 1;");
        assert_eq!(test.units[1].name, "b.ts");
        assert_eq!(test.units[1].content, "import { a } from './a';");
    }

    #[test]
    fn detects_symlinks() {
        let test = TestFile::parse("compiler/l.ts", "// @link: /a -> /b\nlet x;");
        assert!(test.has_symlinks);
        assert_eq!(test.units[0].content, "let x;");
    }

    #[test]
    fn tsx_stem() {
        assert_eq!(TestFile::parse("compiler/jsx.tsx", "").stem, "jsx");
    }
}
