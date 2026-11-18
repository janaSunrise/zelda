//! The binder walks the AST and populates the symbol table.

mod declarations;
mod expressions;
mod types;

use oxc_ast::ast::*;
use oxc_diagnostics::OxcDiagnostic;
use oxc_semantic::{Scoping, Semantic, SymbolFlags};
use oxc_span::Span;
use serde::Serialize;

use crate::errors;

use crate::symbols::{SymbolTable, UndefinedSymbolError};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BindingError {
    /// A declaration conflicts with another of the same name in its scope.
    Redeclared(RedeclaredError),
    UndefinedSymbol(UndefinedSymbolError),
}

/// One declaration of a name that is declared more than once in a scope, in
/// a way TypeScript doesn't allow to merge. tsc reports every such declaration.
#[derive(Debug, Clone, Serialize)]
pub struct RedeclaredError {
    pub name: String,
    #[serde(serialize_with = "serialize_span")]
    pub span: Span,
    /// `let`/`const` are involved (TS2451 rather than TS2300).
    pub block_scoped: bool,
}

impl BindingError {
    pub fn span(&self) -> Span {
        match self {
            Self::Redeclared(e) => e.span,
            Self::UndefinedSymbol(e) => e.span,
        }
    }

    pub fn code(&self) -> u32 {
        match self {
            Self::Redeclared(e) if e.block_scoped => errors::CANNOT_REDECLARE_BLOCK_SCOPED.code,
            Self::Redeclared(_) => errors::DUPLICATE_IDENTIFIER.code,
            Self::UndefinedSymbol(_) => errors::CANNOT_FIND_NAME.code,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Redeclared(e) if e.block_scoped => {
                errors::CANNOT_REDECLARE_BLOCK_SCOPED.format(&[&e.name])
            }
            Self::Redeclared(e) => errors::DUPLICATE_IDENTIFIER.format(&[&e.name]),
            Self::UndefinedSymbol(e) => errors::CANNOT_FIND_NAME.format(&[&e.name]),
        }
    }
}

fn serialize_span<S: serde::Serializer>(span: &Span, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    let mut state = serializer.serialize_struct("Span", 2)?;
    state.serialize_field("start", &span.start)?;
    state.serialize_field("end", &span.end)?;
    state.end()
}

/// Errors that come from oxc's semantic analysis of a file: names that are
/// never declared (TS2304) and conflicting redeclarations (TS2300 / TS2451).
///
/// `symbols` supplies the globals (lib.d.ts) that unresolved references may
/// refer to.
pub fn semantic_errors(
    semantic: &Semantic,
    diagnostics: &[OxcDiagnostic],
    symbols: &SymbolTable,
) -> Vec<BindingError> {
    let scoping = semantic.scoping();
    let mut errors = Vec::new();

    for (name, reference_ids) in scoping.root_unresolved_references() {
        let name = name.as_str();
        let known = is_builtin_global(name)
            || symbols.global_value(name).is_some()
            || symbols.lookup_type(name).is_some();
        if known {
            continue;
        }
        for &reference_id in reference_ids {
            let flags = scoping.get_reference(reference_id).flags();
            // Only report names used as values (or in `typeof`). The bundled
            // lib.d.ts is a subset of TypeScript's, so a missing type or
            // namespace (`JSX.Element`) is more likely a gap there than an
            // error in the program.
            if !flags.is_value() && !flags.is_value_as_type() {
                continue;
            }
            let reference = scoping.get_reference(reference_id);
            errors.push(BindingError::UndefinedSymbol(UndefinedSymbolError {
                name: name.to_string(),
                span: semantic.reference_span(reference),
                is_type: false,
            }));
        }
    }

    // oxc reports a redeclaration exactly when TypeScript's declaration
    // merging rules forbid it (its SymbolFlags excludes mirror tsc's).
    let mut reported = rustc_hash::FxHashSet::default();
    for diagnostic in diagnostics {
        let Some(name) = diagnostic
            .message
            .strip_prefix("Identifier `")
            .and_then(|rest| rest.strip_suffix("` has already been declared"))
        else {
            continue;
        };
        let spans: Vec<Span> = diagnostic
            .labels
            .iter()
            .map(|label| Span::new(label.offset(), label.offset() + label.len()))
            .collect();
        let block_scoped = spans.first().is_some_and(|&first| {
            scoping.symbol_ids().any(|symbol| {
                scoping.symbol_span(symbol) == first
                    && scoping
                        .symbol_flags(symbol)
                        .intersects(SymbolFlags::BlockScopedVariable)
            })
        });
        for span in spans {
            if reported.insert(span) {
                errors.push(BindingError::Redeclared(RedeclaredError {
                    name: name.to_string(),
                    span,
                    block_scoped,
                }));
            }
        }
    }

    errors.sort_by_key(|e| e.span().start);
    errors
}

/// Binder performs two main tasks:
/// 1. Register declarations (variables, functions, types) in the symbol table
/// 2. Check for undefined references in expressions
///
/// `scoping` is oxc's semantic model of the same program: scopes, symbols,
/// and resolved references.
pub struct Binder<'s> {
    pub symbols: SymbolTable,
    pub errors: Vec<BindingError>,
    scoping: &'s Scoping,
}

impl<'s> Binder<'s> {
    /// Create a binder over a pre-populated symbol table (e.g., from lib.d.ts).
    pub fn new(symbols: SymbolTable, scoping: &'s Scoping) -> Self {
        Self {
            symbols,
            errors: Vec::new(),
            scoping,
        }
    }

    /// Record the value type of a declaration's binding.
    pub(super) fn define_value(&mut self, ident: &BindingIdentifier, ty: crate::types::TypeId) {
        if let Some(symbol) = ident.symbol_id.get() {
            self.symbols.define_value(self.scoping, symbol, ty);
        }
    }

    /// Consume the binder and return the symbol table.
    pub fn into_symbols(self) -> SymbolTable {
        self.symbols
    }

    pub fn bind_program(&mut self, program: &Program) {
        for stmt in &program.body {
            self.bind_statement(stmt);
        }
    }

    fn bind_statement(&mut self, stmt: &Statement) {
        match stmt {
            // Declarations
            Statement::VariableDeclaration(decl) => self.bind_variable_declaration(decl),
            Statement::FunctionDeclaration(decl) => self.bind_function_declaration(decl),
            Statement::ClassDeclaration(decl) => self.bind_class_declaration(decl),
            Statement::TSInterfaceDeclaration(decl) => self.bind_interface_declaration(decl),
            Statement::TSTypeAliasDeclaration(decl) => self.bind_type_alias_declaration(decl),
            Statement::TSNamespaceDeclaration(decl) => self.bind_namespace_declaration(decl),
            Statement::TSExternalModuleDeclaration(decl) => {
                self.bind_external_module_declaration(decl);
            }
            Statement::TSGlobalDeclaration(decl) => self.bind_module_block(&decl.body),
            Statement::TSEnumDeclaration(decl) => self.bind_enum_declaration(decl),

            // Control flow (creates scopes, checks expressions)
            Statement::BlockStatement(block) => self.bind_block_statement(block),
            Statement::IfStatement(if_stmt) => self.bind_if_statement(if_stmt),
            Statement::WhileStatement(while_stmt) => self.bind_while_statement(while_stmt),
            Statement::DoWhileStatement(do_while) => self.bind_do_while_statement(do_while),

            Statement::SwitchStatement(switch_stmt) => self.bind_switch_statement(switch_stmt),
            Statement::ForInStatement(for_in) => self.bind_for_in_statement(for_in),
            Statement::ForOfStatement(for_of) => self.bind_for_of_statement(for_of),
            Statement::TryStatement(try_stmt) => self.bind_try_statement(try_stmt),
            Statement::ForStatement(for_stmt) => self.bind_for_statement(for_stmt),

            // Expressions
            Statement::ReturnStatement(ret) => {
                if let Some(arg) = &ret.argument {
                    self.bind_expression(arg);
                }
            }
            Statement::ExpressionStatement(expr_stmt) => {
                self.bind_expression(&expr_stmt.expression);
            }

            // Export declarations - bind the declaration inside
            Statement::ExportDeclaration(export) => self.bind_declaration(&export.declaration),
            // Export specifiers (`export { x }`, `export { x } from "./other"`)
            // are handled at project level.
            Statement::ExportDefaultDeclaration(export) => {
                self.bind_export_default_declaration(export);
            }

            // Import declarations are handled at project level
            Statement::ImportDeclaration(_) => {}

            _ => {}
        }
    }

    fn bind_block_statement(&mut self, block: &BlockStatement) {
        self.symbols.push_scope();
        for stmt in &block.body {
            self.bind_statement(stmt);
        }
        self.symbols.pop_scope();
    }

    fn bind_if_statement(&mut self, if_stmt: &IfStatement) {
        self.bind_expression(&if_stmt.test);
        self.bind_statement(&if_stmt.consequent);
        if let Some(alt) = &if_stmt.alternate {
            self.bind_statement(alt);
        }
    }

    fn bind_while_statement(&mut self, while_stmt: &WhileStatement) {
        self.symbols.push_scope();
        self.bind_expression(&while_stmt.test);
        self.bind_statement(&while_stmt.body);
        self.symbols.pop_scope();
    }

    fn bind_do_while_statement(&mut self, do_while: &DoWhileStatement) {
        self.symbols.push_scope();
        self.bind_statement(&do_while.body);
        self.bind_expression(&do_while.test);
        self.symbols.pop_scope();
    }

    /// For loops create a block scope for their initializer.
    /// `for (let i = 0; ...)` - the `i` is scoped to the loop.
    fn bind_for_statement(&mut self, for_stmt: &ForStatement) {
        self.symbols.push_scope();

        if let Some(init) = &for_stmt.init {
            match init {
                ForStatementInit::VariableDeclaration(decl) => {
                    self.bind_variable_declaration(decl);
                }
                _ => {
                    if let Some(expr) = init.as_expression() {
                        self.bind_expression(expr);
                    }
                }
            }
        }

        if let Some(test) = &for_stmt.test {
            self.bind_expression(test);
        }

        if let Some(update) = &for_stmt.update {
            self.bind_expression(update);
        }

        self.bind_statement(&for_stmt.body);

        self.symbols.pop_scope();
    }

    fn bind_switch_statement(&mut self, switch_stmt: &SwitchStatement) {
        self.bind_expression(&switch_stmt.discriminant);
        for case in &switch_stmt.cases {
            self.symbols.push_scope();
            if let Some(test) = &case.test {
                self.bind_expression(test);
            }
            for stmt in &case.consequent {
                self.bind_statement(stmt);
            }
            self.symbols.pop_scope();
        }
    }

    fn bind_for_in_statement(&mut self, for_in: &ForInStatement) {
        self.symbols.push_scope();
        if let ForStatementLeft::VariableDeclaration(decl) = &for_in.left {
            self.bind_variable_declaration(decl);
        }
        self.bind_expression(&for_in.right);
        self.bind_statement(&for_in.body);
        self.symbols.pop_scope();
    }

    fn bind_for_of_statement(&mut self, for_of: &ForOfStatement) {
        self.symbols.push_scope();
        if let ForStatementLeft::VariableDeclaration(decl) = &for_of.left {
            self.bind_variable_declaration(decl);
        }
        self.bind_expression(&for_of.right);
        self.bind_statement(&for_of.body);
        self.symbols.pop_scope();
    }

    fn bind_try_statement(&mut self, try_stmt: &TryStatement) {
        // Try block
        self.symbols.push_scope();
        for stmt in &try_stmt.block.body {
            self.bind_statement(stmt);
        }
        self.symbols.pop_scope();

        // Catch handler
        if let Some(handler) = &try_stmt.handler {
            self.symbols.push_scope();
            if let Some(param) = &handler.param {
                self.bind_catch_parameter(param);
            }
            for stmt in &handler.body.body {
                self.bind_statement(stmt);
            }
            self.symbols.pop_scope();
        }

        // Finally block
        if let Some(finalizer) = &try_stmt.finalizer {
            self.symbols.push_scope();
            for stmt in &finalizer.body {
                self.bind_statement(stmt);
            }
            self.symbols.pop_scope();
        }
    }

    /// Bind a declaration (used for export declarations).
    fn bind_declaration(&mut self, decl: &Declaration) {
        match decl {
            Declaration::VariableDeclaration(var_decl) => self.bind_variable_declaration(var_decl),
            Declaration::FunctionDeclaration(func) => self.bind_function_declaration(func),
            Declaration::ClassDeclaration(class) => self.bind_class_declaration(class),
            Declaration::TSInterfaceDeclaration(iface) => self.bind_interface_declaration(iface),
            Declaration::TSTypeAliasDeclaration(alias) => self.bind_type_alias_declaration(alias),
            Declaration::TSNamespaceDeclaration(ns) => self.bind_namespace_declaration(ns),
            Declaration::TSExternalModuleDeclaration(module) => {
                self.bind_external_module_declaration(module);
            }
            Declaration::TSGlobalDeclaration(global) => self.bind_module_block(&global.body),
            Declaration::TSEnumDeclaration(decl) => self.bind_enum_declaration(decl),
            _ => {}
        }
    }

    /// Bind a `namespace Name { }` (or legacy `module Name { }`) declaration.
    ///
    /// For now the namespace is registered as an empty object type; its body
    /// is bound in a nested scope so that references inside are still checked.
    fn bind_namespace_declaration(&mut self, decl: &TSNamespaceDeclaration) {
        match &decl.body {
            TSNamespaceDeclarationBody::TSModuleBlock(block) => self.bind_module_block(block),
            // Dotted namespace: `namespace A.B { }`
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(nested) => {
                self.bind_namespace_declaration(nested);
            }
        }
        self.define_empty_module(decl.id.name.to_string());
    }

    /// Bind an ambient `declare module "name" { }` declaration.
    ///
    /// Ambient modules provide types for external packages without implementation.
    fn bind_external_module_declaration(&mut self, decl: &TSExternalModuleDeclaration) {
        if let Some(body) = &decl.body {
            self.bind_module_block(body);
        }
        self.define_empty_module(decl.id.value.display().to_string());
    }

    fn bind_module_block(&mut self, block: &TSModuleBlock) {
        self.symbols.push_scope();
        for stmt in &block.body {
            self.bind_statement(stmt);
        }
        self.symbols.pop_scope();
    }

    fn define_empty_module(&mut self, name: String) {
        let empty_object_id = self.symbols.arena.object(vec![]);
        self.symbols.define_type(name, empty_object_id);
    }

    /// Bind an export default declaration.
    fn bind_export_default_declaration(&mut self, export: &ExportDefaultDeclaration) {
        match &export.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                self.bind_function_declaration(func);
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                self.bind_class_declaration(class);
            }
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(iface) => {
                self.bind_interface_declaration(iface);
            }
            _ => {
                // Expression exports (export default expr) - just check the expression
                if let Some(expr) = export.declaration.as_expression() {
                    self.bind_expression(expr);
                }
            }
        }
    }
}

/// Built-in globals that we skip for now (no lib.d.ts yet).
/// Once we load lib.d.ts, these will be properly defined.
/// Globals that TypeScript's standard lib declares but the bundled lib.d.ts
/// may not: ECMAScript built-ins, common DOM/Node globals, and `arguments`.
pub(crate) fn is_builtin_global(name: &str) -> bool {
    matches!(
        name,
        "arguments"
            | "AggregateError"
            | "ArrayBuffer"
            | "Atomics"
            | "BigInt"
            | "BigInt64Array"
            | "BigUint64Array"
            | "DataView"
            | "EvalError"
            | "FinalizationRegistry"
            | "Float32Array"
            | "Float64Array"
            | "Function"
            | "Int8Array"
            | "Int16Array"
            | "Int32Array"
            | "Intl"
            | "Proxy"
            | "Reflect"
            | "SharedArrayBuffer"
            | "URIError"
            | "Uint8Array"
            | "Uint8ClampedArray"
            | "Uint16Array"
            | "Uint32Array"
            | "WeakRef"
            | "decodeURI"
            | "decodeURIComponent"
            | "encodeURI"
            | "encodeURIComponent"
            | "escape"
            | "eval"
            | "unescape"
            | "navigator"
            | "location"
            | "fetch"
            | "queueMicrotask"
            | "structuredClone"
            | "console"
            | "window"
            | "document"
            | "global"
            | "globalThis"
            | "process"
            | "require"
            | "module"
            | "exports"
            | "__dirname"
            | "__filename"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "clearInterval"
            | "Promise"
            | "Array"
            | "Object"
            | "String"
            | "Number"
            | "Boolean"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "Symbol"
            | "Error"
            | "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "ReferenceError"
            | "JSON"
            | "Math"
            | "Date"
            | "RegExp"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "isFinite"
            | "undefined"
            | "NaN"
            | "Infinity"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    struct Bound {
        errors: Vec<BindingError>,
        symbols: SymbolTable,
    }

    fn parse_and_bind(source: &str) -> Bound {
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let result = Parser::new(&allocator, source, source_type).parse();
        assert!(!result.fatal_error);
        let oxc_semantic::SemanticBuilderReturn {
            semantic,
            diagnostics,
        } = oxc_semantic::SemanticBuilder::new()
            .with_build_nodes(true)
            .build(&result.program);

        let mut binder = Binder::new(SymbolTable::new(), semantic.scoping());
        binder.bind_program(&result.program);
        let mut errors = binder.errors;
        errors.extend(semantic_errors(&semantic, &diagnostics, &binder.symbols));
        Bound {
            errors,
            symbols: binder.symbols,
        }
    }

    #[test]
    fn test_bind_variable() {
        let binder = parse_and_bind("const x: number = 42;");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.global_value("x").is_some());
    }

    #[test]
    fn test_bind_function() {
        let binder = parse_and_bind("function add(a: number, b: number): number { return a + b; }");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.global_value("add").is_some());
    }

    #[test]
    fn test_bind_interface() {
        let binder = parse_and_bind("interface User { name: string; age: number; }");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup_type("User").is_some());
    }

    #[test]
    fn test_duplicate_variable() {
        // tsc reports TS2451 on every declaration of a redeclared `const`.
        let binder = parse_and_bind("const x = 1; const x = 2;");
        assert_eq!(binder.errors.len(), 2);
        for error in &binder.errors {
            match error {
                BindingError::Redeclared(err) => {
                    assert_eq!(err.name, "x");
                    assert!(err.block_scoped);
                }
                _ => panic!("Expected Redeclared error, got {error:?}"),
            }
            assert_eq!(error.code(), 2451);
        }
    }

    #[test]
    fn test_undefined_reference() {
        let binder = parse_and_bind("const x = y;");
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "y"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_function_scope() {
        let binder = parse_and_bind(
            r#"
            const x = 1;
            function foo() {
                const y = 2;
                return x + y;
            }
            "#,
        );
        assert!(binder.errors.is_empty());
    }

    #[test]
    fn test_shadowing_allowed() {
        let binder = parse_and_bind(
            r#"
            const x = 1;
            function foo() {
                const x = 2;
                return x;
            }
            "#,
        );
        assert!(binder.errors.is_empty());
    }

    #[test]
    fn test_type_alias() {
        let binder = parse_and_bind("type StringOrNumber = string | number;");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup_type("StringOrNumber").is_some());
    }

    #[test]
    fn test_class_declaration() {
        let binder = parse_and_bind(
            r#"
            class Person {
                name: string;
                constructor(name: string) {
                    this.name = name;
                }
            }
            "#,
        );
        assert!(binder.errors.is_empty());
        // Class is in both namespaces
        assert!(binder.symbols.global_value("Person").is_some());
        assert!(binder.symbols.lookup_type("Person").is_some());
    }

    #[test]
    fn test_arrow_function() {
        let binder = parse_and_bind("const add = (a: number, b: number) => a + b;");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.global_value("add").is_some());
    }

    #[test]
    fn test_block_scope() {
        let binder = parse_and_bind(
            r#"
            {
                const x = 1;
            }
            const y = x;
            "#,
        );
        // x should not be visible outside the block
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_while_loop_scope() {
        let binder = parse_and_bind(
            r#"
            while (true) {
                let x = 1;
            }
            const y = x;
            "#,
        );
        // x should not be visible outside the while loop
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_do_while_scope() {
        let binder = parse_and_bind(
            r#"
            do {
                let x = 1;
            } while (true);
            const y = x;
            "#,
        );
        // x should not be visible outside the do-while loop
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_switch_scope() {
        let binder = parse_and_bind(
            r#"
            switch (1) {
                case 1:
                    let x = 1;
                    break;
            }
            const y = x;
            "#,
        );
        // x should not be visible outside the switch
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_for_in_scope() {
        let binder = parse_and_bind(
            r#"
            for (let k in {}) {
                let x = 1;
            }
            const y = x;
            "#,
        );
        // x should not be visible outside the for-in loop
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_for_of_scope() {
        let binder = parse_and_bind(
            r#"
            for (let item of [1, 2, 3]) {
                let x = item;
            }
            const y = x;
            "#,
        );
        // x should not be visible outside the for-of loop
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_try_catch_scope() {
        let binder = parse_and_bind(
            r#"
            try {
                let x = 1;
            } catch (e) {
                let y = 2;
            }
            const z = x;
            "#,
        );
        // x should not be visible outside the try block
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_catch_parameter_visible_in_catch() {
        let binder = parse_and_bind(
            r#"
            try {
                throw new Error();
            } catch (e) {
                const x = e;
            }
            "#,
        );
        // Catch parameter should be visible in catch block
        assert!(binder.errors.is_empty());
    }

    #[test]
    fn test_finally_scope() {
        let binder = parse_and_bind(
            r#"
            try {
                let x = 1;
            } finally {
                let y = 2;
            }
            const z = y;
            "#,
        );
        // y should not be visible outside the finally block
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::UndefinedSymbol(err) => assert_eq!(err.name, "y"),
            _ => panic!("Expected UndefinedSymbol error"),
        }
    }

    #[test]
    fn test_hoisted_function_is_found() {
        let binder = parse_and_bind("foo(); function foo() {}");
        assert!(binder.errors.is_empty(), "{:?}", binder.errors);
    }

    #[test]
    fn test_declaration_merging_rules() {
        // Mergeable declarations are fine.
        let binder = parse_and_bind("interface A { a: 1 } interface A { b: 2 } var v; var v;");
        assert!(binder.errors.is_empty(), "{:?}", binder.errors);

        // Conflicting ones are reported on every declaration.
        let binder = parse_and_bind("let x; let x; class C {} class C {}");
        let codes: Vec<u32> = binder.errors.iter().map(BindingError::code).collect();
        assert_eq!(codes, [2451, 2451, 2300, 2300]);
    }

    #[test]
    fn test_duplicate_type_parameter() {
        let binder = parse_and_bind("function f<X, X>() {}");
        assert_eq!(binder.errors.len(), 1, "{:?}", binder.errors);
        assert_eq!(binder.errors[0].code(), 2300);
    }

    #[test]
    fn test_unknown_value_is_ts2304_but_unknown_type_is_not_yet() {
        let binder = parse_and_bind("let a: Missing = nope;");
        let names: Vec<&str> = binder
            .errors
            .iter()
            .filter_map(|e| match e {
                BindingError::UndefinedSymbol(e) => Some(e.name.as_str()),
                BindingError::Redeclared(_) => None,
            })
            .collect();
        assert_eq!(names, ["nope"]);
    }
}
