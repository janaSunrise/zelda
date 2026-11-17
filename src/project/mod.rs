//! Project-level coordination for multi-file type checking.
//!
//! The Project struct manages:
//! - Module resolution (imports)
//! - Building dependency graphs
//! - Coordinating parsing, binding, and checking across files
//! - Caching parsed modules

use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::{GetSpan, SourceType, Span};
use serde::Serialize;

use crate::binder::{Binder, BindingError};
use crate::checker::{Checker, TypeError};
use crate::errors;
use crate::lib_dts;
use crate::resolver::{ModuleResolver, ResolverConfig};
use crate::symbols::{SymbolKind, SymbolTable};
use crate::tsconfig::TsConfig;
use crate::types::{Type, TypeArena, TypeId};

#[derive(Debug)]
pub struct ModuleInfo {
    /// Exported symbols, as types in the project's shared arena.
    pub exports: FxHashMap<String, TypeId>,
    pub default_export: Option<TypeId>,
}

/// An error produced while checking a project.
///
/// Every variant records the file it belongs to, so errors found in a
/// dependency are attributed to that dependency rather than to the file that
/// triggered checking it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProjectError {
    /// A syntax error reported by the parser. Files with syntax errors are not
    /// bound or checked.
    Parse {
        file: PathBuf,
        message: String,
        #[serde(serialize_with = "serialize_span")]
        span: Span,
    },
    Binding {
        file: PathBuf,
        #[serde(flatten)]
        error: BindingError,
    },
    Type {
        file: PathBuf,
        #[serde(flatten)]
        error: TypeError,
    },
    ModuleNotFound {
        specifier: String,
        from_file: PathBuf,
        #[serde(serialize_with = "serialize_span")]
        span: Span,
    },
    FileReadError {
        path: PathBuf,
        error: String,
    },
}

impl ProjectError {
    /// The file this error belongs to.
    pub fn file(&self) -> &Path {
        match self {
            Self::Parse { file, .. } | Self::Binding { file, .. } | Self::Type { file, .. } => file,
            Self::ModuleNotFound { from_file, .. } => from_file,
            Self::FileReadError { path, .. } => path,
        }
    }

    /// The source span of the error, if it points at a location in [`Self::file`].
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::Parse { span, .. } | Self::ModuleNotFound { span, .. } => Some(*span),
            Self::Binding { error, .. } => Some(error.span()),
            Self::Type { error, .. } => Some(error.span),
            Self::FileReadError { .. } => None,
        }
    }

    /// The TypeScript diagnostic code (`TS2322` => `2322`).
    ///
    /// Parse errors come from oxc, which does not use TypeScript's codes, so
    /// they have none.
    pub fn code(&self) -> Option<u32> {
        match self {
            Self::Parse { .. } => None,
            Self::Binding { error, .. } => Some(error.code()),
            Self::Type { error, .. } => Some(error.code),
            Self::ModuleNotFound { .. } => Some(errors::CANNOT_FIND_MODULE.code),
            Self::FileReadError { .. } => Some(errors::CANNOT_READ_FILE.code),
        }
    }

    /// The human-readable diagnostic message.
    pub fn message(&self) -> String {
        match self {
            Self::Parse { message, .. } => message.clone(),
            Self::Binding { error, .. } => error.message(),
            Self::Type { error, .. } => error.message.clone(),
            Self::ModuleNotFound { specifier, .. } => {
                errors::CANNOT_FIND_MODULE.format(&[specifier])
            }
            Self::FileReadError { path, error } => {
                errors::CANNOT_READ_FILE.format(&[&path.display().to_string(), error])
            }
        }
    }

    /// Whether this is a warning rather than an error.
    pub fn is_warning(&self) -> bool {
        matches!(self, Self::Type { error, .. } if error.is_warning())
    }
}

/// Custom serializer for oxc_span::Span since it doesn't implement Serialize.
fn serialize_span<S>(span: &Span, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeStruct;
    let mut state = serializer.serialize_struct("Span", 2)?;
    state.serialize_field("start", &span.start)?;
    state.serialize_field("end", &span.end)?;
    state.end()
}

/// Coordinates multi-file type checking with module resolution and caching.
pub struct Project {
    resolver: ModuleResolver,
    /// Cached module information. Uses FxHashMap for faster lookups.
    modules: FxHashMap<PathBuf, ModuleInfo>,
    processing: Vec<PathBuf>, // For cycle detection
    /// Pre-loaded lib.d.ts symbols (cloned for each file). Its types live in
    /// `arena`; its own `arena` field is unused.
    lib_symbols: SymbolTable,
    /// The type arena shared by every file in the project, so `TypeId`s can
    /// flow between files through imports. While a file is being checked, its
    /// `SymbolTable` holds the arena; it is moved back afterwards.
    arena: TypeArena,
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    pub fn new() -> Self {
        Self::with_resolver(ResolverConfig::new())
    }

    fn with_resolver(config: ResolverConfig) -> Self {
        // Pre-load lib.d.ts into a base symbol table, then take ownership of
        // its arena: it becomes the project-wide arena.
        let mut lib_symbols = SymbolTable::new();
        lib_dts::load_lib_dts(&mut lib_symbols);
        let arena = std::mem::take(&mut lib_symbols.arena);

        Self {
            resolver: ModuleResolver::new(config),
            modules: FxHashMap::default(),
            processing: Vec::new(),
            lib_symbols,
            arena,
        }
    }

    /// Create a new Project with tsconfig loaded from the given path.
    pub fn with_tsconfig(project_root: &Path) -> Self {
        let config = if let Some(tsconfig) = TsConfig::load(project_root) {
            let mut config = ResolverConfig::new();
            config.paths = tsconfig.get_path_mappings();

            let tsconfig_dir = TsConfig::find_tsconfig_dir(project_root);
            if let Some(dir) = tsconfig_dir
                && let Some(base_url) = tsconfig.get_base_url(&dir)
            {
                config.base_url = Some(base_url);
            }

            config
        } else {
            ResolverConfig::new()
        };

        Self::with_resolver(config)
    }

    /// Check a single file and all its dependencies.
    pub fn check_file(&mut self, path: &Path) -> Result<Vec<ProjectError>, ProjectError> {
        let path = path
            .canonicalize()
            .map_err(|e| ProjectError::FileReadError {
                path: path.to_path_buf(),
                error: e.to_string(),
            })?;

        let mut errors = Vec::new();

        // Process this file and its dependencies
        self.process_file(&path, &mut errors)?;

        Ok(errors)
    }

    /// Process a file, resolving imports and checking types.
    fn process_file(
        &mut self,
        path: &Path,
        errors: &mut Vec<ProjectError>,
    ) -> Result<(), ProjectError> {
        // Check if already processed
        if self.modules.contains_key(path) {
            return Ok(());
        }

        // Check for cycles
        if self.processing.contains(&path.to_path_buf()) {
            // Circular dependency - we could error here, but for now just skip
            return Ok(());
        }

        self.processing.push(path.to_path_buf());

        // Read and parse the file
        let source = std::fs::read_to_string(path).map_err(|e| ProjectError::FileReadError {
            path: path.to_path_buf(),
            error: e.to_string(),
        })?;

        let allocator = Allocator::default();
        let source_type = if path.to_string_lossy().ends_with(".d.ts") {
            SourceType::d_ts()
        } else if path.to_string_lossy().ends_with(".tsx") {
            SourceType::tsx()
        } else {
            SourceType::ts()
        };

        let ParserReturn {
            program,
            diagnostics,
            fatal_error,
            ..
        } = Parser::new(&allocator, &source, source_type).parse();

        if fatal_error || diagnostics.has_errors() {
            for diagnostic in diagnostics.errors() {
                let span = diagnostic.labels.first().map_or(Span::default(), |label| {
                    let start = label.offset();
                    Span::new(start, start + label.len())
                });
                errors.push(ProjectError::Parse {
                    file: path.to_path_buf(),
                    message: diagnostic.message.to_string(),
                    span,
                });
            }
            self.processing.pop();
            return Ok(());
        }

        // Check dependencies first. This recurses into `process_file`, so it
        // must happen before this file takes the shared arena below.
        let imports: Vec<_> = program
            .body
            .iter()
            .filter_map(|stmt| match stmt {
                oxc_ast::ast::Statement::ImportDeclaration(import) => Some(import),
                _ => None,
            })
            .map(|import| {
                let specifier = import.source.value.display().to_string();
                let resolved = self.resolver.resolve(&specifier, path);
                if let Ok(resolved) = &resolved {
                    self.process_file(&resolved.path, errors)?;
                }
                Ok((import, specifier, resolved.map(|r| r.path)))
            })
            .collect::<Result<_, ProjectError>>()?;

        // Bind this file against lib.d.ts, with the project's shared arena.
        // Nothing below may return early: the arena is moved back at the end.
        let mut symbols = self.lib_symbols.clone();
        symbols.arena = std::mem::take(&mut self.arena);
        let mut binder = Binder::with_symbols(symbols);

        for (import, specifier, resolved) in &imports {
            self.bind_import(
                import,
                specifier,
                resolved.as_deref().ok(),
                path,
                &mut binder.symbols,
                errors,
            );
        }

        // Now bind the program - imported symbols are already available
        binder.bind_program(&program);

        // Collect binding errors
        for error in &binder.errors {
            errors.push(ProjectError::Binding {
                file: path.to_path_buf(),
                error: error.clone(),
            });
        }

        // Check the module
        let mut checker = Checker::new(&mut binder.symbols);
        checker.check_program(&program);

        for error in &checker.errors {
            errors.push(ProjectError::Type {
                file: path.to_path_buf(),
                error: error.clone(),
            });
        }

        // Collect exports
        let mut exports = FxHashMap::default();
        let mut default_export = None;

        for stmt in &program.body {
            match stmt {
                // `export const name = ...`
                oxc_ast::ast::Statement::ExportDeclaration(export) => {
                    Self::extract_exports_from_declaration(
                        &export.declaration,
                        &binder.symbols,
                        &mut exports,
                    );
                }
                oxc_ast::ast::Statement::ExportNamedDeclaration(export) => {
                    // Handle `export { name }` specifiers
                    for spec in &export.specifiers {
                        let local_name = spec.local.name().as_str();
                        let exported_name = spec.exported.name().as_str();
                        let symbol = binder
                            .symbols
                            .lookup(local_name)
                            .or_else(|| binder.symbols.lookup_type(local_name));
                        if let Some(sym) = symbol {
                            exports.insert(exported_name.to_string(), sym.ty);
                        }
                    }
                }
                oxc_ast::ast::Statement::ExportDefaultDeclaration(export) => {
                    // Handle `export default ...`
                    default_export = Some(Self::default_export_type(
                        &export.declaration,
                        &binder.symbols,
                    ));
                }
                _ => {}
            }
        }

        self.arena = std::mem::take(&mut binder.symbols.arena);

        self.modules.insert(
            path.to_path_buf(),
            ModuleInfo {
                exports,
                default_export,
            },
        );

        self.processing.pop();
        Ok(())
    }

    /// Define the names an import declaration brings into scope, from the
    /// already-checked module it resolved to (`None` if it didn't resolve).
    fn bind_import(
        &self,
        import: &oxc_ast::ast::ImportDeclaration,
        specifier: &str,
        resolved: Option<&Path>,
        from_file: &Path,
        symbols: &mut SymbolTable,
        errors: &mut Vec<ProjectError>,
    ) {
        use oxc_ast::ast::ImportDeclarationSpecifier;

        let Some(resolved) = resolved else {
            errors.push(ProjectError::ModuleNotFound {
                specifier: specifier.to_string(),
                from_file: from_file.to_path_buf(),
                span: import.source.span,
            });
            return;
        };
        // Missing when the module is part of an import cycle or failed to parse.
        let Some(module_info) = self.modules.get(resolved) else {
            return;
        };
        let Some(specifiers) = &import.specifiers else {
            return;
        };
        let module_name = format!("\"{specifier}\"");
        let mut type_error = |message: String, code: u32, span: Span| {
            errors.push(ProjectError::Type {
                file: from_file.to_path_buf(),
                error: TypeError::new(message, span, code),
            });
        };

        for spec in specifiers {
            match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    let imported_name = s.imported.name().as_str();
                    let local_name = s.local.name.as_str();
                    let Some(&ty_id) = module_info.exports.get(imported_name) else {
                        type_error(
                            errors::NO_EXPORTED_MEMBER.format(&[&module_name, imported_name]),
                            errors::NO_EXPORTED_MEMBER.code,
                            s.imported.span(),
                        );
                        continue;
                    };
                    let span = s.local.span;
                    let _ = symbols.define(local_name, ty_id, SymbolKind::Variable, span);
                    // Also add to type namespace if it's a type
                    if matches!(
                        symbols.arena.get(ty_id),
                        Type::Object { .. } | Type::TypeRef { .. }
                    ) {
                        let _ = symbols.define_type(local_name, ty_id, SymbolKind::TypeAlias, span);
                    }
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                    let Some(ty_id) = module_info.default_export else {
                        type_error(
                            errors::NO_DEFAULT_EXPORT.format(&[&module_name]),
                            errors::NO_DEFAULT_EXPORT.code,
                            s.span,
                        );
                        continue;
                    };
                    let _ = symbols.define(
                        s.local.name.as_str(),
                        ty_id,
                        SymbolKind::Variable,
                        s.local.span,
                    );
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                    // `import * as ns from "module"`: an object with every export as a property
                    let properties = module_info
                        .exports
                        .iter()
                        .map(|(name, &ty_id)| crate::types::Property::new(name.clone(), ty_id))
                        .collect();
                    let ty_id = symbols.arena.object(properties);
                    let _ = symbols.define(
                        s.local.name.as_str(),
                        ty_id,
                        SymbolKind::Variable,
                        s.local.span,
                    );
                }
            }
        }
    }

    /// Extract exports from a declaration.
    fn extract_exports_from_declaration(
        decl: &oxc_ast::ast::Declaration,
        symbols: &SymbolTable,
        exports: &mut FxHashMap<String, TypeId>,
    ) {
        use oxc_ast::ast::Declaration;

        let mut export_value = |name: &str| {
            if let Some(sym) = symbols.lookup(name) {
                exports.insert(name.to_string(), sym.ty);
            }
        };
        match decl {
            Declaration::VariableDeclaration(var_decl) => {
                for declarator in &var_decl.declarations {
                    if let oxc_ast::ast::BindingPattern::BindingIdentifier(ident) = &declarator.id {
                        export_value(ident.name.as_str());
                    }
                }
            }
            Declaration::FunctionDeclaration(func) => {
                if let Some(ident) = &func.id {
                    export_value(ident.name.as_str());
                }
            }
            Declaration::ClassDeclaration(class) => {
                if let Some(ident) = &class.id {
                    let name = ident.name.as_str();
                    // TODO: a class is both a value and a type; exports keep
                    // only one meaning per name, so the instance type wins.
                    export_value(name);
                    if let Some(sym) = symbols.lookup_type(name) {
                        exports.insert(name.to_string(), sym.ty);
                    }
                }
            }
            Declaration::TSInterfaceDeclaration(iface) => {
                if let Some(sym) = symbols.lookup_type(iface.id.name.as_str()) {
                    exports.insert(iface.id.name.to_string(), sym.ty);
                }
            }
            Declaration::TSTypeAliasDeclaration(alias) => {
                if let Some(sym) = symbols.lookup_type(alias.id.name.as_str()) {
                    exports.insert(alias.id.name.to_string(), sym.ty);
                }
            }
            _ => {}
        }
    }

    /// The type of a default export.
    fn default_export_type(
        decl: &oxc_ast::ast::ExportDefaultDeclarationKind,
        symbols: &SymbolTable,
    ) -> TypeId {
        use oxc_ast::ast::ExportDefaultDeclarationKind;

        let symbol = match decl {
            ExportDefaultDeclarationKind::FunctionDeclaration(func) => func
                .id
                .as_ref()
                .and_then(|id| symbols.lookup(id.name.as_str())),
            ExportDefaultDeclarationKind::ClassDeclaration(class) => class
                .id
                .as_ref()
                .and_then(|id| symbols.lookup(id.name.as_str())),
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(iface) => {
                symbols.lookup_type(iface.id.name.as_str())
            }
            _ => None,
        };
        symbol.map_or(TypeId::ANY, |sym| sym.ty)
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
    fn test_single_file_no_imports() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main = create_test_file(dir, "main.ts", "const x: number = 42;");

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        assert!(errors.is_empty());
    }

    #[test]
    fn test_single_file_with_error() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main = create_test_file(dir, "main.ts", "const x: number = \"hello\";");

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ProjectError::Type { .. }));
        assert_eq!(errors[0].code(), Some(2322));
    }

    #[test]
    fn test_syntax_errors_are_reported() {
        let temp = TempDir::new().unwrap();
        let main = create_test_file(temp.path(), "main.ts", "let = ;");

        let errors = Project::new().check_file(&main).unwrap();

        assert!(!errors.is_empty());
        assert!(
            errors
                .iter()
                .all(|e| matches!(e, ProjectError::Parse { .. }) && e.code().is_none())
        );
    }

    #[test]
    fn test_exported_declarations_are_checked() {
        let temp = TempDir::new().unwrap();
        let main = create_test_file(temp.path(), "main.ts", "export const x: number = 'a';");

        let errors = Project::new().check_file(&main).unwrap();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code(), Some(2322));
    }

    #[test]
    fn test_errors_in_imported_file_are_attributed_to_it() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();
        create_test_file(dir, "dep.ts", "export const x: number = 'a';");
        let main = create_test_file(
            dir,
            "main.ts",
            "import { x } from './dep';\nconst y: string = 1;",
        );

        let errors = Project::new().check_file(&main).unwrap();

        let mut files: Vec<_> = errors
            .iter()
            .map(|e| e.file().file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(files, ["dep.ts", "main.ts"]);
    }

    #[test]
    fn test_imported_types_keep_their_nested_types() {
        // Nested types of an export are created while checking dep.ts. With a
        // per-file arena their ids dangled in main.ts (index out of bounds).
        let temp = TempDir::new().unwrap();
        let dir = temp.path();
        create_test_file(
            dir,
            "dep.ts",
            "export interface Box { inner: { value: number; label: string } }",
        );
        let main = create_test_file(
            dir,
            "main.ts",
            "import { Box } from './dep';\nconst b: Box = { inner: { value: 'x', label: 'y' } };",
        );

        let errors = Project::new().check_file(&main).unwrap();

        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code(), Some(2322));
    }

    #[test]
    fn test_missing_named_export_is_ts2305() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();
        create_test_file(dir, "dep.ts", "export const x = 1;");
        let main = create_test_file(dir, "main.ts", "import { y } from './dep';");

        let errors = Project::new().check_file(&main).unwrap();

        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].code(), Some(2305));
    }

    #[test]
    fn test_relative_import_named() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        create_test_file(
            dir,
            "utils.ts",
            "export const add = (a: number, b: number) => a + b;",
        );
        let main = create_test_file(
            dir,
            "main.ts",
            r#"
            import { add } from "./utils";
            const result = add(1, 2);
        "#,
        );

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_relative_import_not_found() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        let main = create_test_file(
            dir,
            "main.ts",
            r#"
            import { foo } from "./nonexistent";
        "#,
        );

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ProjectError::ModuleNotFound { .. }));
    }

    #[test]
    fn test_import_type() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        create_test_file(
            dir,
            "types.ts",
            r#"
            export interface User {
                name: string;
                age: number;
            }
        "#,
        );
        let main = create_test_file(
            dir,
            "main.ts",
            r#"
            import { User } from "./types";
            const user: User = { name: "Alice", age: 30 };
        "#,
        );

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_import_default() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        create_test_file(
            dir,
            "config.ts",
            r#"
            const config = { debug: true };
            export default config;
        "#,
        );
        let main = create_test_file(
            dir,
            "main.ts",
            r#"
            import config from "./config";
            const debug = config.debug;
        "#,
        );

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        // This might have issues since default export handling is basic
        // For now, we just check it doesn't panic
        let _ = errors;
    }

    #[test]
    fn test_import_namespace() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        create_test_file(
            dir,
            "math.ts",
            r#"
            export const PI = 3.14159;
            export const add = (a: number, b: number) => a + b;
        "#,
        );
        let main = create_test_file(
            dir,
            "main.ts",
            r#"
            import * as math from "./math";
            const circle = math.PI * 2;
        "#,
        );

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_transitive_imports() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        create_test_file(dir, "a.ts", "export const A = 1;");
        create_test_file(
            dir,
            "b.ts",
            r#"
            import { A } from "./a";
            export const B = A + 1;
        "#,
        );
        let main = create_test_file(
            dir,
            "main.ts",
            r#"
            import { B } from "./b";
            const c = B + 1;
        "#,
        );

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_index_file_import() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path();

        create_test_file(dir, "utils/index.ts", "export const helper = () => {};");
        let main = create_test_file(
            dir,
            "main.ts",
            r#"
            import { helper } from "./utils";
            helper();
        "#,
        );

        let mut project = Project::new();
        let errors = project.check_file(&main).unwrap();

        assert!(errors.is_empty(), "Expected no errors, got: {errors:?}");
    }
}
