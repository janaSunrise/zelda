//! The binder walks the AST and populates the symbol table.

mod declarations;
mod expressions;
mod types;

use oxc_ast::ast::*;

use crate::symbols::{DuplicateSymbolError, ScopeKind, SymbolTable, UndefinedSymbolError};

#[derive(Debug, Clone)]
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

    pub fn bind_program(&mut self, program: &Program) {
        for stmt in &program.body {
            self.bind_statement(stmt);
        }
    }

    /// Dispatch to the appropriate binding method based on statement type.
    fn bind_statement(&mut self, stmt: &Statement) {
        match stmt {
            // Declarations
            Statement::VariableDeclaration(decl) => self.bind_variable_declaration(decl),
            Statement::FunctionDeclaration(decl) => self.bind_function_declaration(decl),
            Statement::ClassDeclaration(decl) => self.bind_class_declaration(decl),
            Statement::TSInterfaceDeclaration(decl) => self.bind_interface_declaration(decl),
            Statement::TSTypeAliasDeclaration(decl) => self.bind_type_alias_declaration(decl),

            // Control flow (creates scopes, checks expressions)
            Statement::BlockStatement(block) => self.bind_block_statement(block),
            Statement::IfStatement(if_stmt) => self.bind_if_statement(if_stmt),
            Statement::WhileStatement(while_stmt) => {
                self.bind_statement(&while_stmt.body);
            }
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
}
