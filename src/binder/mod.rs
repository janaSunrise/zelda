//! The binder walks the AST and populates the symbol table.

mod declarations;
mod expressions;
mod types;

use oxc_ast::ast::*;
use serde::Serialize;

use crate::symbols::{
    DuplicateSymbolError, ScopeKind, SymbolKind, SymbolTable, UndefinedSymbolError,
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BindingError {
    DuplicateSymbol(DuplicateSymbolError),
    UndefinedSymbol(UndefinedSymbolError),
}

impl From<DuplicateSymbolError> for BindingError {
    fn from(err: DuplicateSymbolError) -> Self {
        BindingError::DuplicateSymbol(err)
    }
}

impl From<UndefinedSymbolError> for BindingError {
    fn from(err: UndefinedSymbolError) -> Self {
        BindingError::UndefinedSymbol(err)
    }
}

/// Binder performs two main tasks:
/// 1. Register declarations (variables, functions, types) in the symbol table
/// 2. Check for undefined references in expressions
pub struct Binder {
    pub symbols: SymbolTable,
    pub errors: Vec<BindingError>,
}

impl Default for Binder {
    fn default() -> Self {
        Self::new()
    }
}

impl Binder {
    pub fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            errors: Vec::new(),
        }
    }

    /// Create a binder with a pre-populated symbol table (e.g., from lib.d.ts).
    pub fn with_symbols(symbols: SymbolTable) -> Self {
        Self {
            symbols,
            errors: Vec::new(),
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
            Statement::TSModuleDeclaration(decl) => self.bind_ts_module_declaration(decl),

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
            Statement::ExportNamedDeclaration(export) => {
                if let Some(decl) = &export.declaration {
                    self.bind_declaration(decl);
                }
                // Re-export specifiers (export { x from "./other" }) are handled at project level
            }
            Statement::ExportDefaultDeclaration(export) => {
                self.bind_export_default_declaration(export);
            }

            // Import declarations are handled at project level
            Statement::ImportDeclaration(_) => {}

            _ => {}
        }
    }

    fn bind_block_statement(&mut self, block: &BlockStatement) {
        self.symbols.push_scope(ScopeKind::Block);
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
        self.symbols.push_scope(ScopeKind::Block);
        self.bind_expression(&while_stmt.test);
        self.bind_statement(&while_stmt.body);
        self.symbols.pop_scope();
    }

    fn bind_do_while_statement(&mut self, do_while: &DoWhileStatement) {
        self.symbols.push_scope(ScopeKind::Block);
        self.bind_statement(&do_while.body);
        self.bind_expression(&do_while.test);
        self.symbols.pop_scope();
    }

    /// For loops create a block scope for their initializer.
    /// `for (let i = 0; ...)` - the `i` is scoped to the loop.
    fn bind_for_statement(&mut self, for_stmt: &ForStatement) {
        self.symbols.push_scope(ScopeKind::Block);

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
            self.symbols.push_scope(ScopeKind::Block);
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
        self.symbols.push_scope(ScopeKind::Block);
        match &for_in.left {
            ForStatementLeft::VariableDeclaration(decl) => {
                self.bind_variable_declaration(decl);
            }
            ForStatementLeft::AssignmentTargetIdentifier(ident) => {
                if self.symbols.lookup(ident.name.as_str()).is_none()
                    && !is_builtin_global(ident.name.as_str())
                {
                    self.errors
                        .push(BindingError::UndefinedSymbol(UndefinedSymbolError {
                            name: ident.name.to_string(),
                            span: ident.span,
                            is_type: false,
                        }));
                }
            }
            _ => {}
        }
        self.bind_expression(&for_in.right);
        self.bind_statement(&for_in.body);
        self.symbols.pop_scope();
    }

    fn bind_for_of_statement(&mut self, for_of: &ForOfStatement) {
        self.symbols.push_scope(ScopeKind::Block);
        match &for_of.left {
            ForStatementLeft::VariableDeclaration(decl) => {
                self.bind_variable_declaration(decl);
            }
            ForStatementLeft::AssignmentTargetIdentifier(ident) => {
                if self.symbols.lookup(ident.name.as_str()).is_none()
                    && !is_builtin_global(ident.name.as_str())
                {
                    self.errors
                        .push(BindingError::UndefinedSymbol(UndefinedSymbolError {
                            name: ident.name.to_string(),
                            span: ident.span,
                            is_type: false,
                        }));
                }
            }
            _ => {}
        }
        self.bind_expression(&for_of.right);
        self.bind_statement(&for_of.body);
        self.symbols.pop_scope();
    }

    fn bind_try_statement(&mut self, try_stmt: &TryStatement) {
        // Try block
        self.symbols.push_scope(ScopeKind::Block);
        for stmt in &try_stmt.block.body {
            self.bind_statement(stmt);
        }
        self.symbols.pop_scope();

        // Catch handler
        if let Some(handler) = &try_stmt.handler {
            self.symbols.push_scope(ScopeKind::Block);
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
            self.symbols.push_scope(ScopeKind::Block);
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
            Declaration::TSModuleDeclaration(module) => self.bind_ts_module_declaration(module),
            _ => {}
        }
    }

    /// Bind a `declare module "name"` or `namespace Name { }` declaration.
    /// Ambient modules provide types for external packages without implementation.
    fn bind_ts_module_declaration(&mut self, decl: &oxc_ast::ast::TSModuleDeclaration) {
        // Get the module name
        let name = match &decl.id {
            oxc_ast::ast::TSModuleDeclarationName::Identifier(ident) => ident.name.to_string(),
            oxc_ast::ast::TSModuleDeclarationName::StringLiteral(lit) => lit.value.to_string(),
        };

        // For now, we create an empty module type. In a full implementation,
        // we'd process the body and collect exports.
        if let Some(body) = &decl.body {
            match body {
                oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block) => {
                    // Process statements in the module block
                    self.symbols.push_scope(ScopeKind::Block);
                    for stmt in &block.body {
                        self.bind_statement(stmt);
                    }
                    self.symbols.pop_scope();
                }
                oxc_ast::ast::TSModuleDeclarationBody::TSModuleDeclaration(nested) => {
                    // Nested module: `module A.B { }`
                    self.bind_ts_module_declaration(nested);
                }
            }
        }

        // Register the module name in the type namespace
        let span = match &decl.id {
            oxc_ast::ast::TSModuleDeclarationName::Identifier(ident) => ident.span,
            oxc_ast::ast::TSModuleDeclarationName::StringLiteral(lit) => lit.span,
        };

        // Modules are registered as empty objects for now
        let _ = self.symbols.define_type(
            name,
            crate::types::Type::Object {
                properties: vec![],
                index_signature: None,
                extends: vec![],
                type_params: vec![],
            },
            SymbolKind::TypeAlias,
            span,
        );
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
pub(crate) fn is_builtin_global(name: &str) -> bool {
    matches!(
        name,
        "console"
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

    fn parse_and_bind(source: &str) -> Binder {
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let result = Parser::new(&allocator, source, source_type).parse();
        assert!(!result.panicked);

        let mut binder = Binder::new();
        binder.bind_program(&result.program);
        binder
    }

    #[test]
    fn test_bind_variable() {
        let binder = parse_and_bind("const x: number = 42;");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup("x").is_some());
    }

    #[test]
    fn test_bind_function() {
        let binder = parse_and_bind("function add(a: number, b: number): number { return a + b; }");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup("add").is_some());
    }

    #[test]
    fn test_bind_interface() {
        let binder = parse_and_bind("interface User { name: string; age: number; }");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup_type("User").is_some());
    }

    #[test]
    fn test_duplicate_variable() {
        let binder = parse_and_bind("const x = 1; const x = 2;");
        assert_eq!(binder.errors.len(), 1);
        match &binder.errors[0] {
            BindingError::DuplicateSymbol(err) => assert_eq!(err.name, "x"),
            _ => panic!("Expected DuplicateSymbol error"),
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
        assert!(binder.symbols.lookup("Person").is_some());
        assert!(binder.symbols.lookup_type("Person").is_some());
    }

    #[test]
    fn test_arrow_function() {
        let binder = parse_and_bind("const add = (a: number, b: number) => a + b;");
        assert!(binder.errors.is_empty());
        assert!(binder.symbols.lookup("add").is_some());
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
}
