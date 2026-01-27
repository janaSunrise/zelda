use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Root tsconfig.json structure.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TsConfig {
    #[serde(default)]
    pub compiler_options: CompilerOptions,
}

/// Compiler options we care about.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompilerOptions {
    /// Base URL for non-relative module imports.
    pub base_url: Option<String>,
    /// Path mappings: { "@/*": ["src/*"] }
    pub paths: Option<HashMap<String, Vec<String>>>,
    /// Library files to include (e.g., ["es2020", "dom"])
    pub lib: Option<Vec<String>>,
}

impl TsConfig {
    /// Load tsconfig.json by walking up from `project_root`.
    pub fn load(project_root: &Path) -> Option<Self> {
        let tsconfig_path = Self::find_tsconfig(project_root)?;
        Self::parse(&tsconfig_path)
    }

    /// Find the directory containing tsconfig.json.
    pub fn find_tsconfig_dir(start: &Path) -> Option<PathBuf> {
        Self::find_tsconfig(start).and_then(|p| p.parent().map(|p| p.to_path_buf()))
    }

    fn find_tsconfig(start: &Path) -> Option<PathBuf> {
        let mut current = if start.is_file() {
            start.parent()?
        } else {
            start
        };

        loop {
            let candidate = current.join("tsconfig.json");
            if candidate.exists() {
                return Some(candidate);
            }
            current = current.parent()?;
        }
    }

    fn parse(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Convert path mappings to resolver format: `{ "@/*": ["src/*"] }` → `[("@/*", vec!["src/*"])]`
    pub fn get_path_mappings(&self) -> Vec<(String, Vec<String>)> {
        self.compiler_options
            .paths
            .as_ref()
            .map(|paths| paths.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    /// Get base URL resolved relative to tsconfig location.
    pub fn get_base_url(&self, tsconfig_dir: &Path) -> Option<PathBuf> {
        self.compiler_options
            .base_url
            .as_ref()
            .map(|base| tsconfig_dir.join(base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_tsconfig(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("tsconfig.json");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_parse_minimal_tsconfig() {
        let temp = TempDir::new().unwrap();
        create_tsconfig(temp.path(), "{}");

        let config = TsConfig::load(temp.path()).unwrap();
        assert!(config.compiler_options.base_url.is_none());
        assert!(config.compiler_options.paths.is_none());
    }

    #[test]
    fn test_parse_base_url() {
        let temp = TempDir::new().unwrap();
        create_tsconfig(
            temp.path(),
            r#"{
                "compilerOptions": {
                    "baseUrl": "./src"
                }
            }"#,
        );

        let config = TsConfig::load(temp.path()).unwrap();
        assert_eq!(config.compiler_options.base_url, Some("./src".to_string()));
    }

    #[test]
    fn test_parse_path_mappings() {
        let temp = TempDir::new().unwrap();
        create_tsconfig(
            temp.path(),
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "@/*": ["src/*"],
                        "@components/*": ["src/components/*"]
                    }
                }
            }"#,
        );

        let config = TsConfig::load(temp.path()).unwrap();
        let paths = config.compiler_options.paths.unwrap();
        assert_eq!(paths.get("@/*"), Some(&vec!["src/*".to_string()]));
        assert_eq!(
            paths.get("@components/*"),
            Some(&vec!["src/components/*".to_string()])
        );
    }

    #[test]
    fn test_parse_lib() {
        let temp = TempDir::new().unwrap();
        create_tsconfig(
            temp.path(),
            r#"{
                "compilerOptions": {
                    "lib": ["es2020", "dom"]
                }
            }"#,
        );

        let config = TsConfig::load(temp.path()).unwrap();
        let lib = config.compiler_options.lib.unwrap();
        assert_eq!(lib, vec!["es2020", "dom"]);
    }

    #[test]
    fn test_get_path_mappings() {
        let temp = TempDir::new().unwrap();
        create_tsconfig(
            temp.path(),
            r#"{
                "compilerOptions": {
                    "paths": {
                        "@/*": ["src/*"]
                    }
                }
            }"#,
        );

        let config = TsConfig::load(temp.path()).unwrap();
        let mappings = config.get_path_mappings();
        assert_eq!(mappings.len(), 1);
        assert!(mappings.iter().any(|(k, v)| k == "@/*" && v == &vec!["src/*".to_string()]));
    }

    #[test]
    fn test_get_base_url() {
        let temp = TempDir::new().unwrap();
        create_tsconfig(
            temp.path(),
            r#"{
                "compilerOptions": {
                    "baseUrl": "./src"
                }
            }"#,
        );

        let config = TsConfig::load(temp.path()).unwrap();
        let base_url = config.get_base_url(temp.path()).unwrap();
        assert_eq!(base_url, temp.path().join("src"));
    }

    #[test]
    fn test_find_tsconfig_walks_up() {
        let temp = TempDir::new().unwrap();
        create_tsconfig(temp.path(), r#"{"compilerOptions": {}}"#);

        // Create nested directory
        let nested = temp.path().join("src").join("components");
        fs::create_dir_all(&nested).unwrap();

        // Should find tsconfig in parent
        let config = TsConfig::load(&nested);
        assert!(config.is_some());
    }

    #[test]
    fn test_no_tsconfig_returns_none() {
        let temp = TempDir::new().unwrap();
        let config = TsConfig::load(temp.path());
        assert!(config.is_none());
    }
}
