//! Type inference and checking.
//!
//! Type inference is bidirectional:
//! - Synthesis (bottom-up): compute type from expression
//! - Checking (top-down): verify expression matches expected type

mod calls;
mod declarations;
mod flow;
mod inference;
mod literals;
mod relater;
mod types;

#[cfg(test)]
mod tests;

use flow::{apply_guard, extract_type_guard, NarrowingContext};

use oxc_ast::ast::*;
use oxc_span::Span;

use crate::errors;
use crate::symbols::{ScopeKind, SymbolKind, SymbolTable};
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

    pub fn missing_property(prop: &str, source: &Type, target: &Type, span: Span) -> Self {
        Self::new(
            errors::PROPERTY_MISSING.format(&[prop, &source.to_string(), &target.to_string()]),
            span,
            errors::PROPERTY_MISSING.code,
        )
    }

    pub fn excess_property(prop: &str, target: &Type, span: Span) -> Self {
        Self::new(
            errors::EXCESS_PROPERTY.format(&[prop, &target.to_string()]),
            span,
            errors::EXCESS_PROPERTY.code,
        )
    }

    pub fn undefined_type(name: &str, span: Span) -> Self {
        Self::new(
            errors::CANNOT_FIND_NAME.format(&[name]),
            span,
            errors::CANNOT_FIND_NAME.code,
        )
    }

    pub fn constraint_violation(type_arg: &Type, constraint: &Type, span: Span) -> Self {
        Self::new(
            errors::CONSTRAINT_NOT_SATISFIED.format(&[&type_arg.to_string(), &constraint.to_string()]),
            span,
            errors::CONSTRAINT_NOT_SATISFIED.code,
        )
    }

    pub fn incorrectly_implements(class_name: &str, interface_name: &str, span: Span) -> Self {
        Self::new(
            errors::INCORRECTLY_IMPLEMENTS.format(&[class_name, interface_name]),
            span,
            errors::INCORRECTLY_IMPLEMENTS.code,
        )
    }
}

/// The type checker verifies type correctness of the program.
///
/// It uses:
/// - Bidirectional type checking (synthesis + checking)
/// - Structural type compatibility
/// - Type widening for mutable bindings
/// - Type narrowing for control flow analysis
pub struct Checker<'a> {
    pub symbols: &'a mut SymbolTable,
    pub errors: Vec<TypeError>,
    narrowing: NarrowingContext,
    /// Current class name for super call checking (Some when inside a class)
    current_class: Option<String>,
}

impl<'a> Checker<'a> {
    pub fn new(symbols: &'a mut SymbolTable) -> Self {
        Self {
            symbols,
            errors: Vec::new(),
            narrowing: NarrowingContext::new(),
            current_class: None,
        }
    }

    pub fn check_program(&mut self, program: &Program) {
        for stmt in &program.body {
            self.check_statement(stmt);
        }
    }

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

                // Extract and apply type guard for true branch
                let guard = extract_type_guard(&if_stmt.test);
                if let Some(extracted) = &guard {
                    if let Some(original_type) = self.lookup_variable_type(&extracted.variable) {
                        let narrowed = apply_guard(&original_type, &extracted.guard, extracted.negated);
                        self.narrowing.narrow(extracted.variable.clone(), narrowed);
                    }
                }

                self.check_statement(&if_stmt.consequent);
                self.narrowing.clear(); // Reset after true branch

                if let Some(alt) = &if_stmt.alternate {
                    // Apply negated guard for else branch
                    if let Some(extracted) = &guard {
                        if let Some(original_type) = self.lookup_variable_type(&extracted.variable) {
                            let narrowed = apply_guard(&original_type, &extracted.guard, !extracted.negated);
                            self.narrowing.narrow(extracted.variable.clone(), narrowed);
                        }
                    }
                    self.check_statement(alt);
                    self.narrowing.clear(); // Reset after else branch
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

            Statement::DoWhileStatement(do_while) => {
                self.check_statement(&do_while.body);
                self.check_expression(&do_while.test);
            }

            Statement::SwitchStatement(switch_stmt) => {
                self.check_expression(&switch_stmt.discriminant);
                for case in &switch_stmt.cases {
                    if let Some(test) = &case.test {
                        self.check_expression(test);
                    }
                    for stmt in &case.consequent {
                        self.check_statement(stmt);
                    }
                }
            }

            Statement::ForInStatement(for_in) => {
                self.check_expression(&for_in.right);
                self.check_statement(&for_in.body);
            }

            Statement::ForOfStatement(for_of) => {
                self.check_expression(&for_of.right);
                self.check_statement(&for_of.body);
            }

            Statement::TryStatement(try_stmt) => {
                for stmt in &try_stmt.block.body {
                    self.check_statement(stmt);
                }
                if let Some(handler) = &try_stmt.handler {
                    for stmt in &handler.body.body {
                        self.check_statement(stmt);
                    }
                }
                if let Some(finalizer) = &try_stmt.finalizer {
                    for stmt in &finalizer.body {
                        self.check_statement(stmt);
                    }
                }
            }

            Statement::ThrowStatement(throw_stmt) => {
                self.check_expression(&throw_stmt.argument);
            }

            Statement::TSInterfaceDeclaration(decl) => {
                self.check_interface_declaration(decl);
            }

            Statement::ClassDeclaration(class) => {
                self.check_class_declaration(class);
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
                self.check_new_expression(new_expr);
            }
            Expression::StaticMemberExpression(member) => {
                self.check_expression(&member.object);
                self.check_static_member_expression(member);
            }
            Expression::ComputedMemberExpression(member) => {
                self.check_expression(&member.object);
                self.check_expression(&member.expression);
                self.check_computed_member_expression(member);
            }

            // Literals and identifiers don't need checking
            _ => {}
        }
    }

    /// Check an assignment expression for type compatibility.
    fn check_assignment_expression(&mut self, assign: &AssignmentExpression) {
        self.check_expression(&assign.right);

        // Check property existence for member expression targets
        match &assign.left {
            AssignmentTarget::StaticMemberExpression(member) => {
                let object_type = self.infer_expression(&member.object);
                let prop_name = member.property.name.as_str();

                // Skip checking for `any` and `unknown` types
                if !matches!(object_type, Type::Any | Type::Unknown) {
                    if !self.has_property(&object_type, prop_name) {
                        self.errors.push(TypeError::property_not_found(
                            prop_name,
                            &object_type,
                            member.span,
                        ));
                    }
                }
            }
            _ => {}
        }

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

    /// Check a static member expression (obj.prop) for property existence.
    fn check_static_member_expression(&mut self, member: &StaticMemberExpression) {
        let object_type = self.infer_expression(&member.object);
        let prop_name = member.property.name.as_str();

        // Skip checking for `any` and `unknown` types
        if matches!(object_type, Type::Any | Type::Unknown) {
            return;
        }

        // Check if property exists on the object type
        if !self.has_property(&object_type, prop_name) {
            self.errors.push(TypeError::property_not_found(
                prop_name,
                &object_type,
                member.span,
            ));
        }
    }

    /// Check a computed member expression (obj["prop"] or obj[expr]) for property existence.
    fn check_computed_member_expression(&mut self, member: &ComputedMemberExpression) {
        let object_type = self.infer_expression(&member.object);
        let index_type = self.infer_expression(&member.expression);

        // Skip checking for `any` and `unknown` types
        if matches!(object_type, Type::Any | Type::Unknown) {
            return;
        }

        // If indexing with a string literal, check property existence
        if let Type::StringLiteral(prop_name) = &index_type {
            if !self.has_property(&object_type, prop_name) {
                self.errors.push(TypeError::property_not_found(
                    prop_name,
                    &object_type,
                    member.span,
                ));
            }
        }

        // Array/tuple indexing with number is always valid
        // Index signatures are checked separately
    }

    /// Check if a type has a property with the given name.
    fn has_property(&self, ty: &Type, prop_name: &str) -> bool {
        // Resolve TypeRef first
        let resolved = if let Type::TypeRef { name, .. } = ty {
            self.symbols.lookup_type(name).map(|s| s.ty.clone())
        } else {
            None
        };
        let ty = resolved.as_ref().unwrap_or(ty);

        // For TypeParameter with a constraint, use the constraint
        let resolved_constraint = if let Type::TypeParameter { constraint: Some(constraint), .. } = ty {
            // If the constraint is a TypeRef, resolve it
            let resolved_constraint = if let Type::TypeRef { name, .. } = constraint.as_ref() {
                self.symbols.lookup_type(name).map(|s| s.ty.clone()).unwrap_or_else(|| (**constraint).clone())
            } else {
                (**constraint).clone()
            };
            Some(resolved_constraint)
        } else {
            None
        };
        let ty = resolved_constraint.as_ref().unwrap_or(ty);

        match ty {
            Type::Object { properties, index_signature, extends, .. } => {
                let all_props = self.resolve_object_properties(properties, extends);
                if all_props.iter().any(|p| p.name == prop_name) {
                    return true;
                }
                if index_signature.is_some() {
                    return true;
                }
                false
            }
            Type::Array(_) => {
                // Arrays have built-in properties
                matches!(prop_name, "length" | "push" | "pop" | "shift" | "unshift"
                    | "slice" | "splice" | "concat" | "join" | "map" | "filter"
                    | "reduce" | "forEach" | "find" | "findIndex" | "includes"
                    | "indexOf" | "every" | "some" | "sort" | "reverse" | "fill"
                    | "flat" | "flatMap" | "at" | "entries" | "keys" | "values")
            }
            Type::String | Type::StringLiteral(_) => {
                // Strings have built-in properties
                matches!(prop_name, "length" | "charAt" | "charCodeAt" | "concat"
                    | "includes" | "endsWith" | "startsWith" | "indexOf" | "lastIndexOf"
                    | "match" | "replace" | "search" | "slice" | "split" | "substring"
                    | "toLowerCase" | "toUpperCase" | "trim" | "trimStart" | "trimEnd"
                    | "padStart" | "padEnd" | "repeat" | "at")
            }
            Type::Union(types) => {
                // Property must exist on all union members
                types.iter().all(|t| self.has_property(t, prop_name))
            }
            Type::Intersection(types) => {
                // Property must exist on at least one intersection member
                types.iter().any(|t| self.has_property(t, prop_name))
            }
            Type::ClassConstructor { static_members, .. } => {
                // Class constructor has static members accessible via ClassName.member
                static_members.iter().any(|p| p.name == prop_name)
            }
            Type::Any | Type::Unknown => true,
            _ => false,
        }
    }

    /// Look up a variable's type from the symbol table.
    fn lookup_variable_type(&self, name: &str) -> Option<Type> {
        self.symbols.lookup(name).map(|s| s.ty.clone())
    }
}
