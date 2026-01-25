//! Type inference and checking.
//!
//! Type inference is bidirectional:
//! - Synthesis (bottom-up): compute type from expression
//! - Checking (top-down): verify expression matches expected type

mod assignability;
mod inference;
mod types;

use oxc_ast::ast::*;
use oxc_span::{GetSpan, Span};

use crate::errors;
use crate::symbols::SymbolTable;
use crate::types::Type;

#[derive(Debug, Clone)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
    pub code: u32,
}

impl TypeError {
    pub fn new(message: impl Into<String>, span: Span, code: u32) -> Self {
        Self {
            message: message.into(),
            span,
            code,
        }
    }

    pub fn not_assignable(source: &Type, target: &Type, span: Span) -> Self {
        Self::new(
            errors::NOT_ASSIGNABLE.format(&[&source.to_string(), &target.to_string()]),
            span,
            errors::NOT_ASSIGNABLE.code,
        )
    }

    pub fn argument_not_assignable(arg_type: &Type, param_type: &Type, span: Span) -> Self {
        Self::new(
            errors::ARG_NOT_ASSIGNABLE.format(&[&arg_type.to_string(), &param_type.to_string()]),
            span,
            errors::ARG_NOT_ASSIGNABLE.code,
        )
    }

    pub fn wrong_argument_count(expected: usize, got: usize, span: Span) -> Self {
        Self::new(
            errors::WRONG_ARG_COUNT.format(&[&expected.to_string(), &got.to_string()]),
            span,
            errors::WRONG_ARG_COUNT.code,
        )
    }

    pub fn property_not_found(prop: &str, ty: &Type, span: Span) -> Self {
        Self::new(
            errors::PROPERTY_NOT_EXIST.format(&[prop, &ty.to_string()]),
            span,
            errors::PROPERTY_NOT_EXIST.code,
        )
    }

    pub fn missing_return(span: Span) -> Self {
        Self::new(
            errors::MISSING_RETURN.format(&[]),
            span,
            errors::MISSING_RETURN.code,
        )
    }
}

/// The type checker verifies type correctness of the program.
///
/// It uses:
/// - Bidirectional type checking (synthesis + checking)
/// - Structural type compatibility
/// - Type widening for mutable bindings
pub struct Checker<'a> {
    pub symbols: &'a SymbolTable,
    pub errors: Vec<TypeError>,
}

impl<'a> Checker<'a> {
    pub fn new(symbols: &'a SymbolTable) -> Self {
        Self {
            symbols,
            errors: Vec::new(),
        }
    }

    pub fn check_program(&mut self, program: &Program) {
        for stmt in &program.body {
            self.check_statement(stmt);
        }
    }

    /// Check a statement for type errors.
    fn check_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::VariableDeclaration(decl) => self.check_variable_declaration(decl),
            Statement::FunctionDeclaration(func) => self.check_function_declaration(func),
            Statement::ExpressionStatement(expr) => {
                self.check_expression(&expr.expression);
            }
            Statement::ReturnStatement(ret) => {
                if let Some(arg) = &ret.argument {
                    self.check_expression(arg);
                }
            }
            Statement::BlockStatement(block) => {
                for stmt in &block.body {
                    self.check_statement(stmt);
                }
            }
            Statement::IfStatement(if_stmt) => {
                self.check_expression(&if_stmt.test);
                self.check_statement(&if_stmt.consequent);
                if let Some(alt) = &if_stmt.alternate {
                    self.check_statement(alt);
                }
            }
            Statement::WhileStatement(while_stmt) => {
                self.check_expression(&while_stmt.test);
                self.check_statement(&while_stmt.body);
            }
            Statement::ForStatement(for_stmt) => {
                if let Some(init) = &for_stmt.init {
                    if let ForStatementInit::VariableDeclaration(decl) = init {
                        self.check_variable_declaration(decl);
                    }
                }
                if let Some(test) = &for_stmt.test {
                    self.check_expression(test);
                }
                if let Some(update) = &for_stmt.update {
                    self.check_expression(update);
                }
                self.check_statement(&for_stmt.body);
            }
            _ => {}
        }
    }

    /// Check an expression for type errors.
    ///
    /// This walks the expression tree and checks for:
    /// - Function call argument count and types
    /// - Assignment type compatibility
    /// - Other expression-level errors
    fn check_expression(&mut self, expr: &Expression) {
        match expr {
            Expression::CallExpression(call) => {
                self.check_call_expression(call);
            }

            Expression::AssignmentExpression(assign) => {
                self.check_assignment_expression(assign);
            }

            // Recursively check sub-expressions
            Expression::BinaryExpression(binary) => {
                self.check_expression(&binary.left);
                self.check_expression(&binary.right);
            }
            Expression::UnaryExpression(unary) => {
                self.check_expression(&unary.argument);
            }
            Expression::ConditionalExpression(cond) => {
                self.check_expression(&cond.test);
                self.check_expression(&cond.consequent);
                self.check_expression(&cond.alternate);
            }
            Expression::LogicalExpression(logical) => {
                self.check_expression(&logical.left);
                self.check_expression(&logical.right);
            }
            Expression::ArrayExpression(arr) => {
                for elem in &arr.elements {
                    if let Some(expr) = elem.as_expression() {
                        self.check_expression(expr);
                    }
                }
            }
            Expression::ObjectExpression(obj) => {
                for prop in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(p) = prop {
                        self.check_expression(&p.value);
                    }
                }
            }
            Expression::ArrowFunctionExpression(arrow) => {
                for stmt in &arrow.body.statements {
                    self.check_statement(stmt);
                }
            }
            Expression::ParenthesizedExpression(paren) => {
                self.check_expression(&paren.expression);
            }
            Expression::SequenceExpression(seq) => {
                for expr in &seq.expressions {
                    self.check_expression(expr);
                }
            }
            Expression::AwaitExpression(await_expr) => {
                self.check_expression(&await_expr.argument);
            }
            Expression::NewExpression(new_expr) => {
                // Check constructor arguments similar to call expressions
                self.check_expression(&new_expr.callee);
                for arg in &new_expr.arguments {
                    if let Some(expr) = arg.as_expression() {
                        self.check_expression(expr);
                    }
                }
            }
            Expression::StaticMemberExpression(member) => {
                self.check_expression(&member.object);
            }
            Expression::ComputedMemberExpression(member) => {
                self.check_expression(&member.object);
                self.check_expression(&member.expression);
            }

            // Literals and identifiers don't need checking
            _ => {}
        }
    }

    /// Check a function call for argument count and type errors.
    fn check_call_expression(&mut self, call: &CallExpression) {
        // First check sub-expressions
        self.check_expression(&call.callee);
        for arg in &call.arguments {
            if let Some(expr) = arg.as_expression() {
                self.check_expression(expr);
            }
        }

        // Get the callee type
        let callee_type = self.infer_expression(&call.callee);

        // Only check if it's a function type
        if let Type::Function { params, .. } = callee_type {
            let arg_count = call.arguments.len();
            let required_params = params.iter().filter(|p| !p.optional).count();
            let total_params = params.len();

            // Check argument count
            if arg_count < required_params {
                self.errors.push(TypeError::wrong_argument_count(
                    required_params,
                    arg_count,
                    call.span,
                ));
            } else if arg_count > total_params {
                // Too many arguments (and no rest param)
                let has_rest = params.last().map(|p| p.rest).unwrap_or(false);
                if !has_rest {
                    self.errors.push(TypeError::wrong_argument_count(
                        total_params,
                        arg_count,
                        call.span,
                    ));
                }
            }

            // Check argument types
            for (i, arg) in call.arguments.iter().enumerate() {
                if i >= params.len() {
                    break; // Rest params or extra args handled above
                }
                if let Some(expr) = arg.as_expression() {
                    let arg_type = self.infer_expression(expr);
                    let param_type = &params[i].ty;
                    if !self.is_assignable(&arg_type, param_type) {
                        self.errors.push(TypeError::argument_not_assignable(
                            &arg_type,
                            param_type,
                            expr.span(),
                        ));
                    }
                }
            }
        }
    }

    /// Check an assignment expression for type compatibility.
    fn check_assignment_expression(&mut self, assign: &AssignmentExpression) {
        self.check_expression(&assign.right);

        // Get the target type
        let target_type = match &assign.left {
            AssignmentTarget::AssignmentTargetIdentifier(ident) => self
                .symbols
                .lookup(ident.name.as_str())
                .map(|s| s.ty.clone()),
            _ => None,
        };

        if let Some(target_type) = target_type {
            let value_type = self.infer_expression(&assign.right);
            if !self.is_assignable(&value_type, &target_type) {
                self.errors.push(TypeError::not_assignable(
                    &value_type,
                    &target_type,
                    assign.span,
                ));
            }
        }
    }

    /// Check a variable declaration for type compatibility.
    fn check_variable_declaration(&mut self, decl: &VariableDeclaration) {
        let is_const = decl.kind == VariableDeclarationKind::Const;

        for declarator in &decl.declarations {
            if let Some(init) = &declarator.init {
                // Check sub-expressions first
                self.check_expression(init);

                // Infer the initializer type
                let init_type = self.infer_expression(init);

                // Widen literal types for `let` and `var`
                let init_type = if is_const {
                    init_type
                } else {
                    self.widen_type(init_type)
                };

                // If there's a type annotation, check compatibility
                if let Some(annotation) = &declarator.type_annotation {
                    let declared_type = self.resolve_type(&annotation.type_annotation);
                    if !self.is_assignable(&init_type, &declared_type) {
                        self.errors.push(TypeError::not_assignable(
                            &init_type,
                            &declared_type,
                            declarator.span,
                        ));
                    }
                }
            }
        }
    }

    /// Check a function declaration for return type compatibility.
    fn check_function_declaration(&mut self, func: &Function) {
        if let Some(body) = &func.body {
            // Get declared return type
            let declared_return = func
                .return_type
                .as_ref()
                .map(|ann| self.resolve_type(&ann.type_annotation));

            // Collect return types from body
            let return_types = self.collect_return_types(body);

            // If there's a declared return type, check each return against it
            if let Some(declared) = &declared_return {
                for (ret_type, span) in &return_types {
                    if !self.is_assignable(ret_type, declared) {
                        self.errors
                            .push(TypeError::not_assignable(ret_type, declared, *span));
                    }
                }

                // Check for missing return in non-void functions
                // A function needs a return if its return type is not void/undefined/any/never
                if self.requires_return(declared) && return_types.is_empty() {
                    self.errors.push(TypeError::missing_return(func.span));
                }
            }

            // Check statements in body
            for stmt in &body.statements {
                self.check_statement(stmt);
            }
        }
    }

    /// Check if a return type requires an explicit return statement.
    fn requires_return(&self, return_type: &Type) -> bool {
        !matches!(
            return_type,
            Type::Void | Type::Undefined | Type::Any | Type::Never
        )
    }

    /// Collect all return statement types from a function body.
    fn collect_return_types(&self, body: &FunctionBody) -> Vec<(Type, Span)> {
        let mut returns = Vec::new();
        self.collect_returns_from_statements(&body.statements, &mut returns);
        returns
    }

    fn collect_returns_from_statements(
        &self,
        stmts: &[Statement],
        returns: &mut Vec<(Type, Span)>,
    ) {
        for stmt in stmts {
            self.collect_returns_from_statement(stmt, returns);
        }
    }

    fn collect_returns_from_statement(&self, stmt: &Statement, returns: &mut Vec<(Type, Span)>) {
        match stmt {
            Statement::ReturnStatement(ret) => {
                let ty = ret
                    .argument
                    .as_ref()
                    .map(|arg| self.infer_expression(arg))
                    .unwrap_or(Type::Undefined);
                returns.push((ty, ret.span));
            }
            Statement::BlockStatement(block) => {
                self.collect_returns_from_statements(&block.body, returns);
            }
            Statement::IfStatement(if_stmt) => {
                self.collect_returns_from_statement(&if_stmt.consequent, returns);
                if let Some(alt) = &if_stmt.alternate {
                    self.collect_returns_from_statement(alt, returns);
                }
            }
            Statement::WhileStatement(while_stmt) => {
                self.collect_returns_from_statement(&while_stmt.body, returns);
            }
            Statement::ForStatement(for_stmt) => {
                self.collect_returns_from_statement(&for_stmt.body, returns);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binder::Binder;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    /// Check source and return type errors
    fn check(source: &str) -> Vec<TypeError> {
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let result = Parser::new(&allocator, source, source_type).parse();
        assert!(!result.panicked);

        let mut binder = Binder::new();
        binder.bind_program(&result.program);

        let mut checker = Checker::new(&binder.symbols);
        checker.check_program(&result.program);

        checker.errors
    }

    #[test]
    fn test_infer_string_literal() {
        let errors = check("const x = \"hello\";");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_infer_number_literal() {
        let errors = check("const x = 42;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_const_vs_let_widening() {
        let errors = check("const x = \"hello\"; let y = \"world\";");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_type_mismatch() {
        let errors = check("const x: number = \"hello\";");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_array_inference() {
        let errors = check("const arr = [1, 2, 3];");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_mixed_array_inference() {
        let errors = check("const arr = [1, \"hello\"];");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_object_inference() {
        let errors = check("const obj = { x: 1, y: \"hello\" };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_return_check() {
        let errors = check("function f(): number { return \"hello\"; }");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_function_return_inference() {
        let errors = check("function f() { return 42; }");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_function_inference() {
        let errors = check("const f = (x: number) => x + 1;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_binary_operations() {
        let errors = check("const x = 1 + 2; const y = \"a\" + \"b\";");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_assignable_literal_to_base() {
        let errors = check("const x: string = \"hello\";");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_assignable() {
        let errors = check("const x: string | number = 42;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_not_assignable() {
        let errors = check("const x: string | number = true;");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_shorthand_property() {
        // { x } is equivalent to { x: x }
        let errors = check("const x = 1; const obj = { x };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_method_shorthand() {
        // { foo() {} } is equivalent to { foo: function() {} }
        let errors = check("const obj = { foo() { return 1; } };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_spread_in_object() {
        let errors = check("const a = { x: 1 }; const b = { ...a, y: 2 };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_empty_array() {
        let errors = check("const arr: number[] = [];");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_empty_array_inferred() {
        // Empty array infers never[] but we allow it
        let errors = check("const arr = [];");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_call_wrong_arg_count() {
        let errors = check("function f(x: number) {} f();");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2554); // Expected N arguments, but got M
    }

    #[test]
    fn test_function_call_too_many_args() {
        let errors = check("function f(x: number) {} f(1, 2);");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2554);
    }

    #[test]
    fn test_function_call_wrong_arg_type() {
        let errors = check("function f(x: number) {} f(\"hello\");");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2345); // Argument not assignable
    }

    #[test]
    fn test_function_call_correct() {
        let errors = check("function f(x: number, y: string) {} f(1, \"hello\");");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_call_optional_param() {
        let errors = check("function f(x: number, y?: string) {} f(1);");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_reassignment_type_mismatch() {
        let errors = check("let x: number = 1; x = \"hello\";");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_reassignment_correct() {
        let errors = check("let x: number = 1; x = 2;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_missing_return_in_non_void() {
        let errors = check("function f(): number {}");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2366); // Missing return
    }

    #[test]
    fn test_void_function_no_return_ok() {
        let errors = check("function f(): void {}");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_undefined_return_no_return_ok() {
        let errors = check("function f(): undefined {}");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_null_assignable_to_number() {
        // In non-strict mode, null is assignable to anything
        let errors = check("const x: number = null;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_undefined_assignable_to_string() {
        // In non-strict mode, undefined is assignable to anything
        let errors = check("const x: string = undefined;");
        assert!(errors.is_empty());
    }
}
