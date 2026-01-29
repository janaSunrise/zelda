//! Module resolution for imports.
//!
//! Handles resolving import specifiers to file paths:
//! - Relative imports: `./foo`, `../bar`
//! - Node modules: `lodash`, `@types/node`
//! - Path mapping: `@/components` -> `src/components`

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModule {
    /// The resolved file path.
    pub path: PathBuf,
    /// Whether this is a declaration file (.d.ts).
    pub is_declaration: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolveError {
    /// Module not found at any of the tried paths.
    ModuleNotFound {
        specifier: String,
        tried_paths: Vec<PathBuf>,
    },
    /// Circular dependency detected.
    CircularDependency {
        specifier: String,
        cycle: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ResolverConfig {
    /// Base URL for non-relative imports.
    pub base_url: Option<PathBuf>,
    /// Path mappings (e.g., "@/*" -> ["src/*"]).
    pub paths: Vec<(String, Vec<String>)>,
    /// Extensions to try when resolving (default: [".ts", ".tsx", ".d.ts"]).
    pub extensions: Vec<String>,
}

impl ResolverConfig {
    pub fn new() -> Self {
        Self {
            base_url: None,
            paths: vec![],
            extensions: vec![
                ".ts".to_string(),
                ".tsx".to_string(),
                ".d.ts".to_string(),
                "/index.ts".to_string(),
                "/index.tsx".to_string(),
                "/index.d.ts".to_string(),
            ],
        }
    }

    /// Create a config with path mappings from tsconfig.
    pub fn with_paths(mut self, paths: Vec<(String, Vec<String>)>) -> Self {
        self.paths = paths;
        self
    }

    /// Set the base URL for non-relative imports.
    pub fn with_base_url(mut self, base_url: PathBuf) -> Self {
        self.base_url = Some(base_url);
        self
    }
}

pub struct ModuleResolver {
    config: ResolverConfig,
}

impl ModuleResolver {
    pub fn new(config: ResolverConfig) -> Self {
        Self { config }
    }

    /// Resolve an import specifier to a file path.
    /// Tries: relative -> path mappings -> baseUrl -> node_modules
    pub fn resolve(
        &self,
        specifier: &str,
        from_file: &Path,
    ) -> Result<ResolvedModule, ResolveError> {
        if specifier.starts_with("./") || specifier.starts_with("../") {
            return self.resolve_relative(specifier, from_file);
        }

        if let Some(result) = self.try_path_mappings(specifier, from_file) {
            return Ok(result);
        }

        if let Some(result) = self.try_base_url(specifier) {
            return Ok(result);
        }

        self.resolve_node_module(specifier, from_file)
    }

    /// Try path mappings: `@/*` matches `@/utils`, `@components/*` matches `@components/Button`
    fn try_path_mappings(&self, specifier: &str, from_file: &Path) -> Option<ResolvedModule> {
        if self.config.paths.is_empty() {
            return None;
        }

        let base_dir = match &self.config.base_url {
            Some(base) => base.clone(),
            None => from_file.parent()?.to_path_buf(),
        };

        for (pattern, substitutions) in &self.config.paths {
            if let Some(matched) = self.match_path_pattern(pattern, specifier) {
                for substitution in substitutions {
                    let resolved_path = if substitution.contains('*') {
                        substitution.replace('*', &matched)
                    } else {
                        substitution.clone()
                    };

                    let full_path = base_dir.join(&resolved_path);
                    if let Ok(module) = self.try_resolve_path(&full_path) {
                        return Some(module);
                    }
                }
            }
        }

        None
    }

    /// Match pattern `@/*` against specifier `@/utils` -> returns `Some("utils")`
    fn match_path_pattern(&self, pattern: &str, specifier: &str) -> Option<String> {
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            if specifier.starts_with(prefix) {
                return Some(specifier[prefix.len()..].to_string());
            }
        } else if pattern == specifier {
            return Some(String::new());
        }
        None
    }

    fn try_base_url(&self, specifier: &str) -> Option<ResolvedModule> {
        let base_url = self.config.base_url.as_ref()?;
        let full_path = base_url.join(specifier);
        self.try_resolve_path(&full_path).ok()
    }

    /// Try resolving a path with various TypeScript extensions.
    fn try_resolve_path(&self, base_path: &Path) -> Result<ResolvedModule, ResolveError> {
        let mut tried_paths = Vec::new();

        if let Some(ext) = base_path.extension() {
            let ext_str = ext.to_string_lossy().to_string();
            if ext_str == "ts" || ext_str == "tsx" || ext_str == "js" || ext_str == "jsx" {
                if base_path.exists() {
                    return Ok(ResolvedModule {
                        path: base_path
                            .canonicalize()
                            .unwrap_or_else(|_| base_path.to_path_buf()),
                        is_declaration: ext_str == "d.ts",
                    });
                }
                tried_paths.push(base_path.to_path_buf());
            }
        }

        for ext in &self.config.extensions {
            let try_path = if ext.starts_with('/') {
                base_path.join(&ext[1..])
            } else {
                PathBuf::from(format!("{}{}", base_path.display(), ext))
            };

            if try_path.exists() {
                let is_declaration = try_path.to_string_lossy().ends_with(".d.ts");
                return Ok(ResolvedModule {
                    path: try_path.canonicalize().unwrap_or(try_path),
                    is_declaration,
                });
            }
            tried_paths.push(try_path);
        }

        Err(ResolveError::ModuleNotFound {
            specifier: base_path.display().to_string(),
            tried_paths,
        })
    }

    /// Resolve relative imports (./foo or ../bar).
    fn resolve_relative(
        &self,
        specifier: &str,
        from_file: &Path,
    ) -> Result<ResolvedModule, ResolveError> {
        let base_dir = from_file.parent().unwrap_or(Path::new("."));
        let relative_path = Path::new(specifier);
        let base_path = base_dir.join(relative_path);

        let mut tried_paths = Vec::new();

        // If specifier already has an extension, try it directly
        if let Some(ext) = base_path.extension() {
            let ext_str = ext.to_string_lossy().to_string();
            if ext_str == "ts" || ext_str == "tsx" || ext_str == "js" || ext_str == "jsx" {
                if base_path.exists() {
                    let is_decl = ext_str == "d.ts";
                    return Ok(ResolvedModule {
                        path: base_path
                            .canonicalize()
                            .unwrap_or_else(|_| base_path.clone()),
                        is_declaration: is_decl,
                    });
                }
                tried_paths.push(base_path.clone());
            }
        }

        for ext in &self.config.extensions {
            let try_path = if ext.starts_with('/') {
                // Directory index: ./foo -> ./foo/index.ts
                base_path.join(&ext[1..])
            } else {
                // File extension: ./foo -> ./foo.ts
                PathBuf::from(format!("{}{}", base_path.display(), ext))
            };

            if try_path.exists() {
                let is_declaration = try_path.to_string_lossy().ends_with(".d.ts");
                return Ok(ResolvedModule {
                    path: try_path.canonicalize().unwrap_or(try_path),
                    is_declaration,
                });
            }
            tried_paths.push(try_path);
        }

        Err(ResolveError::ModuleNotFound {
            specifier: specifier.to_string(),
            tried_paths,
        })
    }

    /// Resolve a node module import (lodash, @types/node).
    fn resolve_node_module(
        &self,
        specifier: &str,
        from_file: &Path,
    ) -> Result<ResolvedModule, ResolveError> {
        let mut current_dir = from_file.parent();
        let mut tried_paths = Vec::new();

        // Walk up the directory tree looking for node_modules
        while let Some(dir) = current_dir {
            let node_modules = dir.join("node_modules").join(specifier);

            // Try package.json types/typings field
            let package_json = node_modules.join("package.json");
            if package_json.exists() {
                if let Ok(content) = std::fs::read_to_string(&package_json) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        // Try "types" field first, then "typings"
                        for field in &["types", "typings"] {
                            if let Some(types_path) = json.get(field).and_then(|v| v.as_str()) {
                                let resolved = node_modules.join(types_path);
                                if resolved.exists() {
                                    return Ok(ResolvedModule {
                                        path: resolved.canonicalize().unwrap_or(resolved),
                                        is_declaration: true,
                                    });
                                }
                                tried_paths.push(resolved);
                            }
                        }
                    }
                }
            }

            // Try index.d.ts
            let index_dts = node_modules.join("index.d.ts");
            if index_dts.exists() {
                return Ok(ResolvedModule {
                    path: index_dts.canonicalize().unwrap_or(index_dts),
                    is_declaration: true,
                });
            }
            tried_paths.push(index_dts);

            // Try @types/package
            let at_types = dir.join("node_modules").join("@types").join(specifier);
            let at_types_index = at_types.join("index.d.ts");
            if at_types_index.exists() {
                return Ok(ResolvedModule {
                    path: at_types_index.canonicalize().unwrap_or(at_types_index),
                    is_declaration: true,
                });
            }
            tried_paths.push(at_types_index);

            current_dir = dir.parent();
        }

        Err(ResolveError::ModuleNotFound {
            specifier: specifier.to_string(),
            tried_paths,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_resolve_relative_with_extension() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        // Create test files
        let main_file = create_test_file(dir, "main.ts", "import { x } from './foo';");
        create_test_file(dir, "foo.ts", "export const x = 1;");

        let resolver = ModuleResolver::new(ResolverConfig::new());
        let result = resolver.resolve("./foo", &main_file);

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.path.ends_with("foo.ts"));
        assert!(!resolved.is_declaration);
    }

    #[test]
    fn test_resolve_relative_tsx_extension() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main_file = create_test_file(dir, "main.ts", "import { App } from './App';");
        create_test_file(dir, "App.tsx", "export const App = () => {};");

        let resolver = ModuleResolver::new(ResolverConfig::new());
        let result = resolver.resolve("./App", &main_file);

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.path.ends_with("App.tsx"));
    }

    #[test]
    fn test_resolve_relative_index_file() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main_file = create_test_file(dir, "main.ts", "import { x } from './utils';");
        create_test_file(dir, "utils/index.ts", "export const x = 1;");

        let resolver = ModuleResolver::new(ResolverConfig::new());
        let result = resolver.resolve("./utils", &main_file);

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(
            resolved.path.ends_with("utils/index.ts") || resolved.path.ends_with("utils\\index.ts")
        );
    }

    #[test]
    fn test_resolve_relative_parent_directory() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main_file = create_test_file(
            dir,
            "src/components/Button.ts",
            "import { theme } from '../theme';",
        );
        create_test_file(dir, "src/theme.ts", "export const theme = {};");

        let resolver = ModuleResolver::new(ResolverConfig::new());
        let result = resolver.resolve("../theme", &main_file);

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.path.ends_with("theme.ts"));
    }

    #[test]
    fn test_resolve_relative_not_found() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main_file = create_test_file(dir, "main.ts", "import { x } from './nonexistent';");

        let resolver = ModuleResolver::new(ResolverConfig::new());
        let result = resolver.resolve("./nonexistent", &main_file);

        assert!(result.is_err());
        if let Err(ResolveError::ModuleNotFound {
            specifier,
            tried_paths,
        }) = result
        {
            assert_eq!(specifier, "./nonexistent");
            assert!(!tried_paths.is_empty());
        } else {
            panic!("Expected ModuleNotFound error");
        }
    }

    #[test]
    fn test_resolve_declaration_file() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main_file = create_test_file(dir, "main.ts", "import { x } from './types';");
        create_test_file(dir, "types.d.ts", "export declare const x: number;");

        let resolver = ModuleResolver::new(ResolverConfig::new());
        let result = resolver.resolve("./types", &main_file);

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.path.ends_with("types.d.ts"));
        assert!(resolved.is_declaration);
    }

    #[test]
    fn test_resolve_with_explicit_extension() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main_file = create_test_file(dir, "main.ts", "import { x } from './foo.ts';");
        create_test_file(dir, "foo.ts", "export const x = 1;");

        let resolver = ModuleResolver::new(ResolverConfig::new());
        let result = resolver.resolve("./foo.ts", &main_file);

        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.path.ends_with("foo.ts"));
    }

    #[test]
    fn test_resolve_path_mapping() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        // Create src/utils.ts
        create_test_file(dir, "src/utils.ts", "export const x = 1;");
        let main_file = create_test_file(dir, "main.ts", "import { x } from '@/utils';");

        // Configure path mapping: @/* -> src/*
        let config = ResolverConfig::new()
            .with_paths(vec![("@/*".to_string(), vec!["src/*".to_string()])])
            .with_base_url(dir.to_path_buf());

        let resolver = ModuleResolver::new(config);
        let result = resolver.resolve("@/utils", &main_file);

        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let resolved = result.unwrap();
        assert!(
            resolved.path.ends_with("src/utils.ts") || resolved.path.ends_with("src\\utils.ts")
        );
    }

    #[test]
    fn test_resolve_path_mapping_nested() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        // Create src/components/Button.ts
        create_test_file(
            dir,
            "src/components/Button.ts",
            "export const Button = () => {};",
        );
        let main_file = create_test_file(
            dir,
            "main.ts",
            "import { Button } from '@/components/Button';",
        );

        let config = ResolverConfig::new()
            .with_paths(vec![("@/*".to_string(), vec!["src/*".to_string()])])
            .with_base_url(dir.to_path_buf());

        let resolver = ModuleResolver::new(config);
        let result = resolver.resolve("@/components/Button", &main_file);

        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    }
}
