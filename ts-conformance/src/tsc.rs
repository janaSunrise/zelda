//! The tsc oracle: expected diagnostics from TypeScript's reference baselines.
//!
//! For each test configuration it ran, tsc's harness writes baselines named
//! `<stem>.<kind>` or, for tests whose options vary (`// @strict: true, false`),
//! `<stem>(<key>=<value>,...).<kind>`. A config with errors has an
//! `.errors.txt`; one without errors simply has none.
//!
//! The absence of `.errors.txt` only means "no errors" if tsc actually ran
//! that config: the TS 7 harness silently skips configs using options it
//! dropped (`target: es5`, `module: amd`, ...). So we treat a config as
//! known only if *some* baseline exists for it (`.errors.txt`, `.js`, or
//! `.symbols`; together these cover every config the harness runs).

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::Diagnostic;
use crate::source::normalize_file_name;

/// Artifact suffixes that prove the harness ran a configuration.
const EVIDENCE_SUFFIXES: [&str; 3] = [".errors.txt", ".symbols", ".js"];

/// A test configuration the oracle has baselines for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Baseline name without suffix, e.g. `foo` or `foo(strict=true,target=es2015)`.
    pub name: String,
    /// The varied options, lowercase keys and values.
    pub options: BTreeMap<String, String>,
    pub has_errors: bool,
}

/// Index of the baselines in one suite directory.
#[derive(Debug, Default)]
struct SuiteIndex {
    /// stem → configs
    configs: HashMap<String, Vec<Config>>,
}

/// tsc's reference baselines, indexed by suite and test stem.
#[derive(Debug)]
pub struct Oracle {
    root: PathBuf,
    suites: HashMap<String, SuiteIndex>,
}

impl Oracle {
    /// Index `root/<suite>/` for each suite.
    pub fn load(root: &Path, suites: &[&str]) -> io::Result<Self> {
        let mut index = HashMap::new();
        for &suite in suites {
            let dir = root.join(suite);
            let mut names = Vec::new();
            for entry in std::fs::read_dir(&dir)? {
                names.push(entry?.file_name().to_string_lossy().into_owned());
            }
            index.insert(suite.to_string(), SuiteIndex::build(names));
        }
        Ok(Self {
            root: root.to_path_buf(),
            suites: index,
        })
    }

    /// All configurations tsc ran for a test.
    pub fn configs(&self, suite: &str, stem: &str) -> &[Config] {
        self.suites
            .get(suite)
            .and_then(|idx| idx.configs.get(stem))
            .map_or(&[], Vec::as_slice)
    }

    /// The diagnostics tsc reported for a configuration.
    pub fn expected(&self, suite: &str, config: &Config) -> io::Result<Vec<BaselineDiagnostic>> {
        if !config.has_errors {
            return Ok(Vec::new());
        }
        let path = self
            .root
            .join(suite)
            .join(format!("{}.errors.txt", config.name));
        Ok(parse_errors_baseline(&std::fs::read_to_string(path)?))
    }
}

impl SuiteIndex {
    fn build(file_names: impl IntoIterator<Item = String>) -> Self {
        // name → has_errors
        let mut seen: BTreeMap<String, bool> = BTreeMap::new();
        for file_name in file_names {
            let Some((name, suffix)) = EVIDENCE_SUFFIXES
                .iter()
                .find_map(|s| file_name.strip_suffix(s).map(|n| (n.to_string(), *s)))
            else {
                continue;
            };
            *seen.entry(name).or_default() |= suffix == ".errors.txt";
        }

        let mut configs: HashMap<String, Vec<Config>> = HashMap::new();
        for (name, has_errors) in seen {
            let (stem, options) = split_config_name(&name);
            configs.entry(stem.to_string()).or_default().push(Config {
                name,
                options,
                has_errors,
            });
        }
        Self { configs }
    }
}

/// `foo(strict=true,target=es2015)` → (`foo`, {strict: true, target: es2015}).
fn split_config_name(name: &str) -> (&str, BTreeMap<String, String>) {
    let parsed = name
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once('('))
        .and_then(|(stem, desc)| {
            let options = desc
                .split(',')
                .map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    Some((k.to_string(), v.to_string()))
                })
                .collect::<Option<BTreeMap<_, _>>>()?;
            Some((stem, options))
        });
    parsed.unwrap_or((name, BTreeMap::new()))
}

/// Choose which configuration of a multi-config test to compare against.
///
/// zelda has no compiler options, and behaves like tsc's defaults (`strict`,
/// latest target). So prefer configs closest to that: strict checks on, the
/// newest target and module system, and otherwise option defaults. Ties are
/// broken by name for determinism.
pub fn choose_config(configs: &[Config]) -> Option<&Config> {
    configs.iter().min_by(|a, b| {
        let rank =
            |c: &Config| -> Vec<u32> { c.options.iter().map(|(k, v)| option_rank(k, v)).collect() };
        rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name))
    })
}

/// Lower is preferred.
fn option_rank(key: &str, value: &str) -> u32 {
    const STRICT_FAMILY: [&str; 6] = [
        "strict",
        "strictnullchecks",
        "noimplicitany",
        "alwaysstrict",
        "useunknownincatchvariables",
        "strictbuiltiniteratorreturn",
    ];
    match (key, value) {
        (k, "true") if STRICT_FAMILY.contains(&k) => 0,
        (k, _) if STRICT_FAMILY.contains(&k) => 1,
        ("target", v) => target_rank(v),
        ("module", "esnext") => 0,
        ("module", "preserve") => 1,
        ("module", "nodenext") => 2,
        ("module", _) => 3,
        // Other booleans: prefer the default (off).
        (_, "false") => 0,
        (_, "true") => 1,
        _ => 2,
    }
}

/// `esnext` first, then newest `esYYYY` first.
fn target_rank(target: &str) -> u32 {
    match target {
        "esnext" => 0,
        "es6" => 10_000 - 2015,
        _ => target
            .strip_prefix("es")
            .and_then(|year| year.parse::<u32>().ok())
            .map_or(10_000, |year| 10_000u32.saturating_sub(year)),
    }
}

/// A diagnostic from an `.errors.txt` header. Global diagnostics (about
/// compiler options, not tied to a file) have no location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineDiagnostic {
    Located(Diagnostic),
    Global { code: u32, message: String },
}

static LOCATED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.+)\((\d+),(\d+)\): error TS(\d+): (.*)$").expect("valid regex")
});
static GLOBAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^error TS(\d+): (.*)$").expect("valid regex"));

/// Parse the summary at the top of an `.errors.txt` baseline: one line per
/// diagnostic (plus indented continuation lines for message chains), ended by
/// a blank line. The annotated source listing below it is ignored.
pub fn parse_errors_baseline(text: &str) -> Vec<BaselineDiagnostic> {
    let mut diagnostics = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            break;
        }
        if let Some(c) = LOCATED_RE.captures(line) {
            diagnostics.push(BaselineDiagnostic::Located(Diagnostic {
                file: normalize_file_name(&c[1]),
                line: c[2].parse().unwrap_or(0),
                col: c[3].parse().unwrap_or(0),
                code: c[4].parse().unwrap_or(0),
                message: c[5].to_string(),
            }));
        } else if let Some(c) = GLOBAL_RE.captures(line) {
            diagnostics.push(BaselineDiagnostic::Global {
                code: c[1].parse().unwrap_or(0),
                message: c[2].to_string(),
            });
        }
        // Otherwise: a continuation line of a multi-line message.
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(names: &[&str]) -> SuiteIndex {
        SuiteIndex::build(names.iter().map(ToString::to_string))
    }

    #[test]
    fn parses_header_and_stops_at_listing() {
        let text = "\
error TS5107: Option 'target=ES5' is deprecated.
/a.ts(3,7): error TS2322: Type 'string' is not assignable to type 'number'.
  Some elaboration line.
b.ts(1,1): error TS2304: Cannot find name 'x'.


==== /a.ts (1 errors) ====
    let x: number = 'oops';
!!! error TS2322: Type 'string' is not assignable to type 'number'.
";
        let diags = parse_errors_baseline(text);
        assert_eq!(diags.len(), 3);
        assert!(matches!(
            diags[0],
            BaselineDiagnostic::Global { code: 5107, .. }
        ));
        let BaselineDiagnostic::Located(d) = &diags[1] else {
            panic!()
        };
        assert_eq!(
            (d.file.as_str(), d.line, d.col, d.code),
            ("a.ts", 3, 7, 2322)
        );
        assert!(matches!(&diags[2], BaselineDiagnostic::Located(d) if d.file == "b.ts"));
    }

    #[test]
    fn indexes_configs_and_errors() {
        let idx = index(&[
            "plain.js",
            "plain.symbols",
            "plain.errors.txt",
            "clean.symbols",
            "multi(strict=false).js",
            "multi(strict=true).errors.txt",
            "multi(strict=true).js",
            "ignored.types",
        ]);
        let plain = &idx.configs["plain"];
        assert_eq!(plain.len(), 1);
        assert!(plain[0].has_errors);
        assert!(!idx.configs["clean"][0].has_errors);
        assert!(!idx.configs.contains_key("ignored"));

        let multi = &idx.configs["multi"];
        assert_eq!(multi.len(), 2);
        let chosen = choose_config(multi).unwrap();
        assert_eq!(chosen.name, "multi(strict=true)");
        assert!(chosen.has_errors);
    }

    #[test]
    fn prefers_newest_target() {
        let idx = index(&[
            "t(target=es2015).js",
            "t(target=es2022).js",
            "t(target=esnext).js",
        ]);
        assert_eq!(
            choose_config(&idx.configs["t"]).unwrap().name,
            "t(target=esnext)"
        );

        let idx = index(&["u(target=es2015).js", "u(target=es2022).js"]);
        assert_eq!(
            choose_config(&idx.configs["u"]).unwrap().name,
            "u(target=es2022)"
        );
    }

    #[test]
    fn splits_config_names() {
        let (stem, opts) = split_config_name("foo(module=esnext,target=es2015)");
        assert_eq!(stem, "foo");
        assert_eq!(opts["module"], "esnext");
        assert_eq!(opts["target"], "es2015");
        assert_eq!(split_config_name("bar").0, "bar");
    }
}
