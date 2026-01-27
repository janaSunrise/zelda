//! Type inference and checking.
//!
//! Type inference is bidirectional:
//! - Synthesis (bottom-up): compute type from expression
//! - Checking (top-down): verify expression matches expected type

mod assignability;
mod inference;
mod narrowing;
mod types;

use narrowing::{apply_guard, extract_type_guard, NarrowingContext};

use oxc_ast::ast::*;
use oxc_span::{GetSpan, Span};

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

    /// Check a function call for argument count and type errors.
    fn check_call_expression(&mut self, call: &CallExpression) {
        // First check sub-expressions (but skip Super since it's not a regular expression)
        if !matches!(call.callee, Expression::Super(_)) {
            self.check_expression(&call.callee);
        }
        for arg in &call.arguments {
            if let Some(expr) = arg.as_expression() {
                self.check_expression(expr);
            }
        }

        // Handle super() call specially
        if let Expression::Super(_) = &call.callee {
            self.check_super_call(call);
            return;
        }

        // Get the callee type
        let callee_type = self.infer_expression(&call.callee);

        // Only check if it's a function type
        if let Type::Function { params, type_params, .. } = callee_type {
            // Collect argument types for type inference
            let arg_types: Vec<Type> = call
                .arguments
                .iter()
                .filter_map(|arg| arg.as_expression())
                .map(|expr| self.infer_expression(expr))
                .collect();

            // Get explicit type arguments from the call if present
            let explicit_type_args: Vec<Type> = call
                .type_arguments
                .as_ref()
                .map(|args| {
                    args.params
                        .iter()
                        .map(|t| self.resolve_ts_type(t))
                        .collect()
                })
                .unwrap_or_default();

            // Build substitution map for generic functions
            let substitutions = if !type_params.is_empty() {
                if !explicit_type_args.is_empty() {
                    // Use explicit type arguments
                    self.build_substitution_map(&type_params, &explicit_type_args)
                } else {
                    // Infer type arguments from argument types
                    self.infer_type_args_from_call(&type_params, &params, &arg_types)
                }
            } else {
                std::collections::HashMap::new()
            };

            // Check constraints for each type argument
            for tp in &type_params {
                if let Some(constraint) = &tp.constraint {
                    if let Some(type_arg) = substitutions.get(&tp.name) {
                        if !self.satisfies_constraint(type_arg, constraint) {
                            self.errors.push(TypeError::constraint_violation(
                                type_arg,
                                constraint,
                                call.span,
                            ));
                        }
                    }
                }
            }

            // Substitute type parameters in parameter types for checking
            let instantiated_params: Vec<crate::types::Param> = params
                .iter()
                .map(|p| crate::types::Param {
                    name: p.name.clone(),
                    ty: if substitutions.is_empty() {
                        p.ty.clone()
                    } else {
                        self.substitute_type_params(&p.ty, &substitutions)
                    },
                    optional: p.optional,
                    rest: p.rest,
                })
                .collect();

            let arg_count = call.arguments.len();
            // Rest params don't count as required - they accept 0 or more args
            let required_params = instantiated_params
                .iter()
                .filter(|p| !p.optional && !p.rest)
                .count();
            let has_rest = instantiated_params.last().map(|p| p.rest).unwrap_or(false);

            // Check argument count
            if arg_count < required_params {
                self.errors.push(TypeError::wrong_argument_count(
                    required_params,
                    arg_count,
                    call.span,
                ));
            } else if !has_rest && arg_count > instantiated_params.len() {
                // Too many arguments (and no rest param)
                self.errors.push(TypeError::wrong_argument_count(
                    instantiated_params.len(),
                    arg_count,
                    call.span,
                ));
            }

            // Check argument types against instantiated parameter types
            // Special handling for rest parameters: arguments beyond normal params
            // are checked against the element type of the rest param array
            let non_rest_param_count = if has_rest {
                instantiated_params.len() - 1
            } else {
                instantiated_params.len()
            };

            for (i, arg) in call.arguments.iter().enumerate() {
                if let Some(expr) = arg.as_expression() {
                    let arg_type = self.infer_expression(expr);

                    let param_type = if i < non_rest_param_count {
                        // Regular parameter
                        &instantiated_params[i].ty
                    } else if has_rest {
                        // Rest parameter - check against element type of the array
                        let rest_param = instantiated_params.last().unwrap();
                        if let Type::Array(elem_type) = &rest_param.ty {
                            elem_type.as_ref()
                        } else {
                            // Rest param should always be array type
                            &rest_param.ty
                        }
                    } else {
                        // No more params and no rest - already handled by arg count check
                        break;
                    };

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

    /// Check a super() call in a derived class constructor.
    fn check_super_call(&mut self, call: &CallExpression) {
        // Get current class and its parent
        let class_name = match &self.current_class {
            Some(name) => name.clone(),
            None => return, // super() outside class - ignore (other error)
        };

        // Look up the class type and its extends clause
        let class_type = self.symbols.lookup_type(&class_name).map(|s| s.ty.clone());
        let parent_class_name = match class_type {
            Some(Type::Object { extends, .. }) if !extends.is_empty() => {
                if let Type::TypeRef { name, .. } = &extends[0] {
                    name.clone()
                } else {
                    return;
                }
            }
            _ => return, // No parent class
        };

        // Get parent's constructor type
        let parent_constructor = self.symbols.lookup(&parent_class_name).map(|s| s.ty.clone());

        let params = match parent_constructor {
            Some(Type::ClassConstructor { params, .. }) => params,
            Some(Type::Function { params, .. }) => params, // For backward compatibility
            _ => return,
        };

        // Check argument types
        let arg_count = call.arguments.len();
        let required_params = params.iter().filter(|p| !p.optional && !p.rest).count();
        let has_rest = params.last().map(|p| p.rest).unwrap_or(false);

        // Check argument count
        if arg_count < required_params {
            self.errors.push(TypeError::wrong_argument_count(
                required_params,
                arg_count,
                call.span,
            ));
        } else if !has_rest && arg_count > params.len() {
            self.errors.push(TypeError::wrong_argument_count(
                params.len(),
                arg_count,
                call.span,
            ));
        }

        // Check argument types
        let non_rest_param_count = if has_rest { params.len() - 1 } else { params.len() };

        for (i, arg) in call.arguments.iter().enumerate() {
            if let Some(expr) = arg.as_expression() {
                let arg_type = self.infer_expression(expr);

                let param_type = if i < non_rest_param_count {
                    &params[i].ty
                } else if has_rest {
                    let rest_param = params.last().unwrap();
                    if let Type::Array(elem_type) = &rest_param.ty {
                        elem_type.as_ref()
                    } else {
                        &rest_param.ty
                    }
                } else {
                    break;
                };

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

    /// Check a new expression for constructor argument count and type errors.
    fn check_new_expression(&mut self, new_expr: &NewExpression) {
        // First check sub-expressions
        self.check_expression(&new_expr.callee);
        for arg in &new_expr.arguments {
            if let Some(expr) = arg.as_expression() {
                self.check_expression(expr);
            }
        }

        // Get the constructor type from the value namespace
        let constructor_type = if let Expression::Identifier(ident) = &new_expr.callee {
            self.symbols.lookup(ident.name.as_str()).map(|s| s.ty.clone())
        } else {
            None
        };

        // Extract params and type_params from either Function or ClassConstructor
        let (params, type_params) = match constructor_type {
            Some(Type::Function { params, type_params, .. }) => (params, type_params),
            Some(Type::ClassConstructor { params, type_params, .. }) => (params, type_params),
            _ => return,
        };

        // Collect argument types
        let arg_types: Vec<Type> = new_expr
            .arguments
            .iter()
            .filter_map(|arg| arg.as_expression())
            .map(|expr| self.infer_expression(expr))
            .collect();

        // Get explicit type arguments if present
        let explicit_type_args: Vec<Type> = new_expr
            .type_arguments
            .as_ref()
            .map(|args| {
                args.params
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect()
            })
            .unwrap_or_default();

        // Build substitution map for generic constructors
        let substitutions = if !type_params.is_empty() {
            if !explicit_type_args.is_empty() {
                self.build_substitution_map(&type_params, &explicit_type_args)
            } else {
                self.infer_type_args_from_call(&type_params, &params, &arg_types)
            }
        } else {
            std::collections::HashMap::new()
        };

        // Check type parameter constraints
        for tp in &type_params {
            if let Some(constraint) = &tp.constraint {
                if let Some(type_arg) = substitutions.get(&tp.name) {
                    if !self.satisfies_constraint(type_arg, constraint) {
                        self.errors.push(TypeError::constraint_violation(
                            type_arg,
                            constraint,
                            new_expr.span,
                        ));
                    }
                }
            }
        }

        // Instantiate parameter types
        let instantiated_params: Vec<crate::types::Param> = params
            .iter()
            .map(|p| crate::types::Param {
                name: p.name.clone(),
                ty: if substitutions.is_empty() {
                    p.ty.clone()
                } else {
                    self.substitute_type_params(&p.ty, &substitutions)
                },
                optional: p.optional,
                rest: p.rest,
            })
            .collect();

        let arg_count = new_expr.arguments.len();
        let required_params = instantiated_params
            .iter()
            .filter(|p| !p.optional && !p.rest)
            .count();
        let has_rest = instantiated_params.last().map(|p| p.rest).unwrap_or(false);

        // Check argument count
        if arg_count < required_params {
            self.errors.push(TypeError::wrong_argument_count(
                required_params,
                arg_count,
                new_expr.span,
            ));
        } else if !has_rest && arg_count > instantiated_params.len() {
            self.errors.push(TypeError::wrong_argument_count(
                instantiated_params.len(),
                arg_count,
                new_expr.span,
            ));
        }

        // Check argument types
        let non_rest_param_count = if has_rest {
            instantiated_params.len() - 1
        } else {
            instantiated_params.len()
        };

        for (i, arg) in new_expr.arguments.iter().enumerate() {
            if let Some(expr) = arg.as_expression() {
                let arg_type = self.infer_expression(expr);

                let param_type = if i < non_rest_param_count {
                    &instantiated_params[i].ty
                } else if has_rest {
                    let rest_param = instantiated_params.last().unwrap();
                    if let Type::Array(elem_type) = &rest_param.ty {
                        elem_type.as_ref()
                    } else {
                        &rest_param.ty
                    }
                } else {
                    break;
                };

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
                    let declared_type = self.resolve_ts_type(&annotation.type_annotation);

                    // Check for object literal specific errors (missing/excess properties)
                    if let Expression::ObjectExpression(obj) = init {
                        self.check_object_literal_against_type(obj, &declared_type, declarator.span);
                    } else if let Expression::ArrayExpression(arr) = init {
                        // Contextual typing: check array literal against tuple type
                        self.check_array_literal_against_type(arr, &declared_type, declarator.span);
                    } else if !self.is_assignable(&init_type, &declared_type) {
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

    /// Check an array literal against an expected type.
    ///
    /// Handles contextual typing for tuples: [1, "hello"] against [number, string]
    fn check_array_literal_against_type(
        &mut self,
        arr: &ArrayExpression,
        expected: &Type,
        span: Span,
    ) {
        // If expected type is a tuple, check element-by-element
        if let Type::Tuple(expected_types) = expected {
            // Check length
            if arr.elements.len() != expected_types.len() {
                let inferred = self.infer_array_literal(arr);
                self.errors
                    .push(TypeError::not_assignable(&inferred, expected, span));
                return;
            }

            // Check each element against expected type
            for (elem, expected_type) in arr.elements.iter().zip(expected_types.iter()) {
                if let Some(expr) = elem.as_expression() {
                    let elem_type = self.infer_expression(expr);
                    if !self.is_assignable(&elem_type, expected_type) {
                        self.errors.push(TypeError::not_assignable(
                            &elem_type,
                            expected_type,
                            span,
                        ));
                    }
                }
            }
            return;
        }

        // If expected type is an array, check all elements against element type
        if let Type::Array(expected_elem) = expected {
            for elem in &arr.elements {
                if let Some(expr) = elem.as_expression() {
                    let elem_type = self.infer_expression(expr);
                    if !self.is_assignable(&elem_type, expected_elem) {
                        self.errors.push(TypeError::not_assignable(
                            &elem_type,
                            expected_elem,
                            span,
                        ));
                    }
                }
            }
            return;
        }

        // Fall back to normal assignability
        let inferred = self.infer_array_literal(arr);
        if !self.is_assignable(&inferred, expected) {
            self.errors
                .push(TypeError::not_assignable(&inferred, expected, span));
        }
    }

    /// Check an interface declaration.
    /// Validates that all extended interfaces exist in the type namespace.
    fn check_interface_declaration(&mut self, decl: &TSInterfaceDeclaration) {
        for heritage in &decl.extends {
            let name = match &heritage.expression {
                Expression::Identifier(ident) => ident.name.to_string(),
                _ => continue,
            };
            if self.symbols.lookup_type(&name).is_none() {
                self.errors.push(TypeError::undefined_type(&name, heritage.span));
            }
        }
    }

    /// Check a class declaration for type errors.
    ///
    /// Validates:
    /// 1. Class correctly implements all interfaces in its implements clause
    fn check_class_declaration(&mut self, class: &Class) {
        let class_name = class.id.as_ref().map(|id| id.name.as_str()).unwrap_or("");

        // Get the class's instance type
        let class_type = self.symbols.lookup_type(class_name).map(|s| s.ty.clone());

        let class_type = match class_type {
            Some(ty) => ty,
            None => return,
        };

        // Check implements clause
        for heritage in &class.implements {
            let interface_name = match &heritage.expression {
                TSTypeName::IdentifierReference(ident) => ident.name.to_string(),
                TSTypeName::QualifiedName(qual) => qual.right.name.to_string(),
                TSTypeName::ThisExpression(_) => continue,
            };

            // Get the interface type
            let interface_type = self.symbols.lookup_type(&interface_name).map(|s| s.ty.clone());

            if let Some(interface_type) = interface_type {
                // Check that class implements all required members
                if let Type::Object { properties: interface_props, .. } = &interface_type {
                    let class_props = if let Type::Object { properties, extends, .. } = &class_type {
                        self.resolve_object_properties(properties, extends)
                    } else {
                        vec![]
                    };

                    let class_prop_names: std::collections::HashSet<&str> =
                        class_props.iter().map(|p| p.name.as_str()).collect();

                    for interface_prop in interface_props {
                        // Check if property exists in class
                        if !interface_prop.optional && !class_prop_names.contains(interface_prop.name.as_str()) {
                            self.errors.push(TypeError::incorrectly_implements(
                                class_name,
                                &interface_name,
                                class.span,
                            ));
                            break; // One error per interface is enough
                        }

                        // Check type compatibility if property exists
                        if let Some(class_prop) = class_props.iter().find(|p| p.name == interface_prop.name) {
                            if !self.is_assignable(&class_prop.ty, &interface_prop.ty) {
                                self.errors.push(TypeError::incorrectly_implements(
                                    class_name,
                                    &interface_name,
                                    class.span,
                                ));
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Set class context for super call checking
        let prev_class = self.current_class.take();
        self.current_class = Some(class_name.to_string());

        // Check class body (methods, etc.)
        for element in &class.body.body {
            if let ClassElement::MethodDefinition(method) = element {
                if let Some(body) = &method.value.body {
                    for stmt in &body.statements {
                        self.check_statement(stmt);
                    }
                }
            }
        }

        // Restore previous context
        self.current_class = prev_class;
    }

    /// Check an object literal against an expected type.
    ///
    /// This performs:
    /// 1. Missing property check - all required properties must be present
    /// 2. Excess property check - no extra properties allowed (fresh literal only)
    /// 3. Property type compatibility check
    fn check_object_literal_against_type(
        &mut self,
        obj: &ObjectExpression,
        expected: &Type,
        span: Span,
    ) {
        // Resolve TypeRef to actual type, instantiating generic types
        let resolved = if let Type::TypeRef { name, type_args } = expected {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or_else(|| expected.clone())
        } else {
            expected.clone()
        };

        let Type::Object {
            properties: expected_props,
            index_signature,
            extends,
            ..
        } = &resolved
        else {
            let source = self.infer_object_literal(obj);
            if !self.is_assignable(&source, expected) {
                self.errors
                    .push(TypeError::not_assignable(&source, expected, span));
            }
            return;
        };

        let expected_props = self.resolve_object_properties(expected_props, extends);

        // Collect properties from the object literal
        let mut literal_props: Vec<(String, Span)> = Vec::new();
        for prop in &obj.properties {
            if let ObjectPropertyKind::ObjectProperty(p) = prop {
                if let Some(name) = match &p.key {
                    PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                    PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                    PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                    _ => None,
                } {
                    literal_props.push((name.clone(), p.span));

                    // Check property type compatibility
                    if let Some(expected_prop) = expected_props.iter().find(|ep| ep.name == name) {
                        let value_type = self.infer_expression(&p.value);
                        if !self.is_assignable(&value_type, &expected_prop.ty) {
                            self.errors.push(TypeError::not_assignable(
                                &value_type,
                                &expected_prop.ty,
                                p.span,
                            ));
                        }
                    }
                }
            }
        }

        let literal_prop_names: std::collections::HashSet<&str> =
            literal_props.iter().map(|(n, _)| n.as_str()).collect();

        // Check for missing required properties
        let source_type = self.infer_object_literal(obj);
        for expected_prop in &expected_props {
            if !expected_prop.optional && !literal_prop_names.contains(expected_prop.name.as_str()) {
                self.errors.push(TypeError::missing_property(
                    &expected_prop.name,
                    &source_type,
                    expected,
                    span,
                ));
            }
        }

        // Check for excess properties (only if no index signature)
        // If index signature exists, verify value types match
        let expected_prop_names: std::collections::HashSet<&str> =
            expected_props.iter().map(|p| p.name.as_str()).collect();

        for prop in &obj.properties {
            if let ObjectPropertyKind::ObjectProperty(p) = prop {
                if let Some(name) = match &p.key {
                    PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                    PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                    PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                    _ => None,
                } {
                    // Property not in expected_props - either excess or needs index sig check
                    if !expected_prop_names.contains(name.as_str()) {
                        if let Some(idx_sig) = index_signature {
                            // Index signature exists - check value type compatibility
                            let value_type = self.infer_expression(&p.value);
                            if !self.is_assignable(&value_type, &idx_sig.value_type) {
                                self.errors.push(TypeError::not_assignable(
                                    &value_type,
                                    &idx_sig.value_type,
                                    p.span,
                                ));
                            }
                        } else {
                            // No index signature - excess property error
                            self.errors.push(TypeError::excess_property(
                                &name,
                                expected,
                                p.span,
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Look up a variable's type from the symbol table.
    fn lookup_variable_type(&self, name: &str) -> Option<Type> {
        self.symbols.lookup(name).map(|s| s.ty.clone())
    }

    /// Check a function declaration for return type compatibility.
    fn check_function_declaration(&mut self, func: &Function) {
        // Validate parameter order (even for functions without body)
        self.validate_function_parameters(&func.params);

        if let Some(body) = &func.body {
            // Push function scope to match binder's scope structure
            self.symbols.push_scope(ScopeKind::Function);

            // Bind type parameters to type namespace for generic functions
            if let Some(type_params) = &func.type_parameters {
                for param in &type_params.params {
                    let name = param.name.name.as_str();
                    let constraint = param.constraint.as_ref().map(|c| crate::type_resolution::resolve_ts_type(c));
                    let default = param.default.as_ref().map(|d| crate::type_resolution::resolve_ts_type(d));

                    let ty = Type::TypeParameter {
                        name: name.to_string(),
                        constraint: constraint.map(Box::new),
                        default: default.map(Box::new),
                    };

                    let _ = self.symbols.define_type(name, ty, SymbolKind::TypeAlias, param.name.span);
                }
            }

            // Re-bind parameters in this scope for the checker
            // For optional parameters, the type inside the function is T | undefined
            for param in &func.params.items {
                if let BindingPattern::BindingIdentifier(ident) = &param.pattern {
                    let base_ty = param
                        .type_annotation
                        .as_ref()
                        .map(|ann| crate::type_resolution::resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(Type::Any);

                    // Optional parameters have type T | undefined inside the function
                    let ty = if param.optional {
                        Type::union(vec![base_ty, Type::Undefined])
                    } else {
                        base_ty
                    };

                    let _ = self.symbols.define(
                        ident.name.as_str(),
                        ty,
                        SymbolKind::Parameter,
                        ident.span,
                    );
                }
            }

            // Get declared return type
            let declared_return = func
                .return_type
                .as_ref()
                .map(|ann| self.resolve_ts_type(&ann.type_annotation));

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
                let requires_return = !matches!(
                    declared,
                    Type::Void | Type::Undefined | Type::Any | Type::Never
                );
                if requires_return && return_types.is_empty() {
                    self.errors.push(TypeError::missing_return(func.span));
                }
            }

            // Check statements in body
            for stmt in &body.statements {
                self.check_statement(stmt);
            }

            // Pop function scope
            self.symbols.pop_scope();
        }
    }

    /// Validate function parameter order:
    /// - Required parameters cannot follow optional parameters (TS1016)
    /// - Rest parameter must be last (TS1014)
    fn validate_function_parameters(&mut self, params: &FormalParameters) {
        let mut seen_optional = false;

        // In oxc, rest parameter is stored separately in params.rest, not in items
        // So we only need to check that required params don't follow optional ones
        for param in &params.items {
            if param.optional {
                seen_optional = true;
            } else if seen_optional {
                // Required parameter after optional - TS1016
                self.errors.push(TypeError::new(
                    errors::REQUIRED_AFTER_OPTIONAL.format(&[]),
                    param.span,
                    errors::REQUIRED_AFTER_OPTIONAL.code,
                ));
            }
        }
        // Note: Rest parameter validation (must be last) is handled by the parser
        // If items appear after rest, it's a parser error, not a checker error
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
            Statement::DoWhileStatement(do_while) => {
                self.collect_returns_from_statement(&do_while.body, returns);
            }
            Statement::ForStatement(for_stmt) => {
                self.collect_returns_from_statement(&for_stmt.body, returns);
            }
            Statement::ForInStatement(for_in) => {
                self.collect_returns_from_statement(&for_in.body, returns);
            }
            Statement::ForOfStatement(for_of) => {
                self.collect_returns_from_statement(&for_of.body, returns);
            }
            Statement::SwitchStatement(switch_stmt) => {
                for case in &switch_stmt.cases {
                    for stmt in &case.consequent {
                        self.collect_returns_from_statement(stmt, returns);
                    }
                }
            }
            Statement::TryStatement(try_stmt) => {
                self.collect_returns_from_statements(&try_stmt.block.body, returns);
                if let Some(handler) = &try_stmt.handler {
                    self.collect_returns_from_statements(&handler.body.body, returns);
                }
                if let Some(finalizer) = &try_stmt.finalizer {
                    self.collect_returns_from_statements(&finalizer.body, returns);
                }
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

        let mut checker = Checker::new(&mut binder.symbols);
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

    #[test]
    fn test_object_missing_property() {
        let errors = check("const x: { a: number } = {};");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2741);
    }

    #[test]
    fn test_object_missing_multiple_properties() {
        let errors = check("const x: { a: number; b: string } = {};");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_object_has_required_property() {
        let errors = check("const x: { a: number } = { a: 1 };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_object_property_type_mismatch() {
        let errors = check("const x: { a: number } = { a: \"hello\" };");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_object_excess_property() {
        let errors = check("const x: { a: number } = { a: 1, b: 2 };");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2353);
    }

    #[test]
    fn test_object_multiple_excess_properties() {
        let errors = check("const x: { a: number } = { a: 1, b: 2, c: 3 };");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_object_optional_property_missing_ok() {
        let errors = check("const x: { a: number; b?: string } = { a: 1 };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_object_optional_property_present_ok() {
        let errors = check("const x: { a: number; b?: string } = { a: 1, b: \"hi\" };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_object_optional_property_wrong_type() {
        let errors = check("const x: { a: number; b?: string } = { a: 1, b: 42 };");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_object_all_optional_empty_ok() {
        let errors = check("const x: { a?: number; b?: string } = {};");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_property_access_exists() {
        let errors = check("const obj = { x: 1 }; const y = obj.x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_property_access_not_exists() {
        let errors = check("const obj = { x: 1 }; const y = obj.z;");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339);
    }

    #[test]
    fn test_property_access_on_typed_object() {
        let errors = check("const obj: { x: number } = { x: 1 }; const y = obj.x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_property_access_not_exists_on_typed() {
        let errors = check("const obj: { x: number } = { x: 1 }; const y = obj.z;");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339);
    }

    #[test]
    fn test_computed_property_access_string_literal() {
        let errors = check("const obj = { x: 1 }; const y = obj[\"x\"];");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_computed_property_access_not_exists() {
        let errors = check("const obj = { x: 1 }; const y = obj[\"z\"];");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339);
    }

    #[test]
    fn test_property_access_on_any() {
        // Accessing property on `any` should not error
        let errors = check("const obj: any = {}; const y = obj.anything;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_array_length_property() {
        let errors = check("const arr = [1, 2, 3]; const len = arr.length;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_array_method_property() {
        let errors = check("const arr = [1, 2, 3]; const mapped = arr.map;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_string_length_property() {
        let errors = check("const s = \"hello\"; const len = s.length;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_nested_property_access() {
        let errors = check("const obj = { inner: { x: 1 } }; const y = obj.inner.x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_nested_property_not_exists() {
        let errors = check("const obj = { inner: { x: 1 } }; const y = obj.inner.z;");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339);
    }

    #[test]
    fn test_union_source_all_branches_must_match() {
        // string | number is NOT assignable to string
        let errors = check("const x: string | number = 1; const y: string = x;");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_union_target_any_branch_works() {
        // string is assignable to string | number
        let errors = check("const x: string | number = \"hello\";");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_with_null() {
        let errors = check("const x: string | null = null;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_with_undefined() {
        let errors = check("const x: number | undefined = undefined;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_nested_union() {
        let errors = check("const x: (string | number) | boolean = true;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_intersection_must_satisfy_all() {
        // Object must have both a and b
        let errors = check("const x: { a: number } & { b: string } = { a: 1 };");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_intersection_satisfies_all() {
        let errors = check("const x: { a: number } & { b: string } = { a: 1, b: \"hi\" };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_array_element_type_mismatch() {
        let errors = check("const x: number[] = [1, 2, \"three\"];");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_array_covariance() {
        // string[] should not be assignable to (string | number)[] in strict mode
        // but we're in non-strict, so this works
        let errors = check("const x: string[] = [\"a\"]; const y: (string | number)[] = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_tuple_exact_length() {
        let errors = check("const x: [number, string] = [1];");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_tuple_type_mismatch() {
        let errors = check("const x: [number, string] = [1, 2];");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_tuple_correct() {
        let errors = check("const x: [number, string] = [1, \"hello\"];");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_tuple_to_array() {
        // Tuple [number, number] should be assignable to number[]
        let errors = check("const x: [number, number] = [1, 2]; const y: number[] = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_return_covariance() {
        // () => string is assignable to () => string | number
        let errors = check(r#"
            const f: () => string = () => "hello";
            const g: () => string | number = f;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_param_contravariance() {
        // (x: string | number) => void is assignable to (x: string) => void
        let errors = check(r#"
            const f: (x: string | number) => void = (x) => {};
            const g: (x: string) => void = f;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_fewer_params_ok() {
        // () => void is assignable to (x: number) => void (callback compatibility)
        let errors = check(r#"
            const f: () => void = () => {};
            const g: (x: number) => void = f;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_more_params_error() {
        // (x: number, y: number) => void is NOT assignable to (x: number) => void
        let errors = check(r#"
            const f: (x: number, y: number) => void = (x, y) => {};
            const g: (x: number) => void = f;
        "#);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_any_accepts_anything() {
        let errors = check("const x: any = { foo: 1, bar: \"test\" };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_any_assignable_to_anything() {
        let errors = check("const x: any = 1; const y: string = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_unknown_accepts_anything() {
        let errors = check("const x: unknown = { foo: 1 };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_never_assignable_to_anything() {
        let errors = check(r#"
            function fail(): never { throw new Error(); }
            const x: string = fail();
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_void_function() {
        let errors = check("function f(): void { return; }");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_ternary_inference() {
        let errors = check("const x = true ? 1 : \"hello\"; const y: string | number = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_logical_and_inference() {
        // a && b returns b's type if a is truthy
        let errors = check("const x = true && 42; const y: number = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_logical_or_inference() {
        let errors = check("const x = false || \"fallback\"; const y: boolean | string = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_nullish_coalescing_inference() {
        let errors = check("const x = null ?? \"default\"; const y: null | string = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arithmetic_inference() {
        let errors = check("const x = 1 + 2 * 3 - 4 / 2; const y: number = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_string_concat_inference() {
        let errors = check("const x = \"hello\" + \" \" + \"world\"; const y: string = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_comparison_inference() {
        let errors = check("const x = 1 < 2; const y: boolean = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_typeof_inference() {
        let errors = check("const x = typeof 42; const y: string = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_unary_not_inference() {
        let errors = check("const x = !true; const y: boolean = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_unary_minus_inference() {
        let errors = check("const x = -42; const y: number = x;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_nested_object_inference() {
        let errors = check(r#"
            const obj = {
                user: {
                    name: "alice",
                    age: 30
                },
                active: true
            };
            const name: string = obj.user.name;
            const age: number = obj.user.age;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_array_of_objects_inference() {
        let errors = check(r#"
            const users = [
                { name: "alice", age: 30 },
                { name: "bob", age: 25 }
            ];
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_returning_object() {
        let errors = check(r#"
            function createUser(name: string, age: number) {
                return { name, age };
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_call_with_literal_args() {
        let errors = check(r#"
            function greet(name: string, age: number): string {
                return name;
            }
            const result = greet("alice", 30);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_call_with_variable_args() {
        let errors = check(r#"
            function add(a: number, b: number): number {
                return a + b;
            }
            const x = 1;
            const y = 2;
            const sum = add(x, y);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_call_with_wrong_literal_type() {
        let errors = check(r#"
            function square(n: number): number {
                return n * n;
            }
            const result = square("hello");
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2345);
    }

    #[test]
    fn test_callback_function() {
        let errors = check(r#"
            function map(arr: number[], fn: (x: number) => number): number[] {
                return arr;
            }
            const doubled = map([1, 2, 3], (x: number) => x * 2);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_as_type() {
        let errors = check(r#"
            interface User {
                name: string;
                age: number;
            }
            const user: User = { name: "alice", age: 30 };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_missing_property() {
        let errors = check(r#"
            interface User {
                name: string;
                age: number;
            }
            const user: User = { name: "alice" };
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2741);
    }

    #[test]
    fn test_interface_optional_property() {
        let errors = check(r#"
            interface User {
                name: string;
                age?: number;
            }
            const user: User = { name: "alice" };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_type_alias() {
        let errors = check(r#"
            type StringOrNumber = string | number;
            const x: StringOrNumber = 42;
            const y: StringOrNumber = "hello";
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_empty_object_literal() {
        let errors = check("const x: {} = {};");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_object_with_only_optional_properties() {
        let errors = check("const x: { a?: number; b?: string } = {};");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_deeply_nested_property_access() {
        let errors = check(r#"
            const obj = { a: { b: { c: { d: 42 } } } };
            const val: number = obj.a.b.c.d;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_array_index_access() {
        let errors = check(r#"
            const arr = [1, 2, 3];
            const first = arr[0];
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_spread_operator_in_array() {
        let errors = check(r#"
            const a = [1, 2];
            const b = [...a, 3, 4];
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_template_literal() {
        let errors = check(r#"
            const name = "world";
            const greeting: string = `Hello, ${name}!`;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_function_expression_body() {
        let errors = check("const double = (x: number) => x * 2;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_function_block_body() {
        let errors = check(r#"
            const double = (x: number) => {
                return x * 2;
            };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_iife() {
        let errors = check("const x = ((n: number) => n * 2)(5);");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_recursive_function() {
        let errors = check(r#"
            function factorial(n: number): number {
                if (n <= 1) return 1;
                return n * factorial(n - 1);
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_mutually_recursive_functions() {
        let errors = check(r#"
            function isEven(n: number): boolean {
                if (n === 0) return true;
                return isOdd(n - 1);
            }
            function isOdd(n: number): boolean {
                if (n === 0) return false;
                return isEven(n - 1);
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_const_assertion_behavior() {
        // const gets literal type, let gets widened
        let errors = check(r#"
            const x = "hello";
            let y = "hello";
            const a: "hello" = x;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_multiple_statements_multiple_errors() {
        let errors = check(r#"
            const x: number = "wrong";
            const y: string = 123;
            const z: boolean = "also wrong";
        "#);
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn test_void_vs_undefined() {
        let errors = check(r#"
            function f(): void {}
            function g(): undefined { return undefined; }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_bigint_literal() {
        let errors = check("const x = 9007199254740991n;");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_sequence_expression() {
        let errors = check("const x = (1, 2, 3);");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_new_expression() {
        let errors = check("const date = new Date();");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_for_loop_scope() {
        let errors = check(r#"
            for (let i = 0; i < 10; i++) {
                const x = i;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_while_loop() {
        let errors = check(r#"
            let x = 0;
            while (x < 10) {
                x = x + 1;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_if_else() {
        let errors = check(r#"
            const x = 5;
            if (x > 0) {
                const positive = true;
            } else {
                const negative = true;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_typeof_narrowing_string() {
        let errors = check(r#"
            function f(x: string | number) {
                if (typeof x === "string") {
                    const y: string = x;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_typeof_narrowing_number() {
        let errors = check(r#"
            function f(x: string | number) {
                if (typeof x === "number") {
                    const y: number = x;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_null_check_narrowing() {
        let errors = check(r#"
            function f(x: string | null) {
                if (x !== null) {
                    const y: string = x;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_undefined_check_narrowing() {
        let errors = check(r#"
            function f(x: string | undefined) {
                if (x !== undefined) {
                    const y: string = x;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_narrowing_not_applied_outside_if() {
        let errors = check(r#"
            function f(x: string | number) {
                if (typeof x === "string") {
                    const y: string = x;
                }
                const z: string = x;
            }
        "#);
        // z assignment should fail because x is still string | number outside if
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_switch_statement() {
        let errors = check(r#"
            function f(x: number): string {
                switch (x) {
                    case 1:
                        return "one";
                    case 2:
                        return "two";
                    default:
                        return "other";
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_try_catch() {
        let errors = check(r#"
            function f(): number {
                try {
                    return 1;
                } catch (e) {
                    return 0;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_for_in_loop() {
        let errors = check(r#"
            function f(obj: { a: number; b: number }): void {
                for (const key in obj) {
                    const x: string = key;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_for_of_loop() {
        let errors = check(r#"
            function f(arr: number[]): number {
                let sum = 0;
                for (const item of arr) {
                    sum = sum + item;
                }
                return sum;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_do_while_loop() {
        let errors = check(r#"
            function f(): number {
                let x = 0;
                do {
                    x = x + 1;
                } while (x < 10);
                return x;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_basic() {
        let errors = check("const obj: { [key: string]: number } = { a: 1, b: 2 };");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_value_type_mismatch() {
        let errors = check("const obj: { [key: string]: number } = { a: \"wrong\" };");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_index_signature_property_access() {
        let errors = check(r#"
            const obj: { [key: string]: number } = { a: 1 };
            const x: number = obj.anyProp;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_property_access_wrong_type() {
        let errors = check(r#"
            const obj: { [key: string]: number } = { a: 1 };
            const x: string = obj.anyProp;
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_index_signature_with_explicit_property() {
        let errors = check(r#"
            const obj: { name: string; [key: string]: string } = { name: "test", extra: "ok" };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_explicit_property_mismatch() {
        let errors = check(r#"
            const obj: { name: string; [key: string]: string } = { name: 42 };
        "#);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_index_signature_computed_access() {
        let errors = check(r#"
            const obj: { [key: string]: number } = { a: 1 };
            const key: string = "test";
            const val = obj[key];
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_number_key() {
        let errors = check(r#"
            const arr: { [index: number]: string } = { 0: "first", 1: "second" };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_empty_object() {
        let errors = check("const obj: { [key: string]: number } = {};");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_with_index_signature() {
        let errors = check(r#"
            interface StringMap {
                [key: string]: string;
            }
            const map: StringMap = { hello: "world", foo: "bar" };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_index_signature_property_access() {
        let errors = check(r#"
            interface NumberDict {
                [key: string]: number;
            }
            function f(d: NumberDict): number {
                return d.anyKey;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_mixed_explicit_and_index() {
        let errors = check(r#"
            interface Config {
                name: string;
                [key: string]: string;
            }
            const cfg: Config = { name: "app", version: "1.0" };
            const n: string = cfg.name;
            const v: string = cfg.version;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_excess_property_with_index() {
        let errors = check(r#"
            const obj: { known: number; [key: string]: number } = {
                known: 1,
                extra: 2,
                another: 3
            };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_nested_object() {
        let errors = check(r#"
            interface UserMap {
                [id: string]: { name: string; age: number };
            }
            const users: UserMap = {
                user1: { name: "Alice", age: 30 },
                user2: { name: "Bob", age: 25 }
            };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_index_signature_function_value() {
        let errors = check(r#"
            interface Handlers {
                [event: string]: () => void;
            }
            const h: Handlers = {
                click: () => {},
                hover: () => {}
            };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_property_access_common() {
        let errors = check(r#"
            interface A { x: number; y: string; }
            interface B { x: number; z: boolean; }
            function f(val: A | B): number {
                return val.x;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_property_access_not_common() {
        let errors = check(r#"
            interface A { x: number; y: string; }
            interface B { x: number; z: boolean; }
            function f(val: A | B): string {
                return val.y;
            }
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339);
    }

    #[test]
    fn test_intersection_property_access() {
        let errors = check(r#"
            interface A { x: number; }
            interface B { y: string; }
            function f(val: A & B) {
                const a: number = val.x;
                const b: string = val.y;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_typeof_narrowing_in_if() {
        let errors = check(r#"
            function f(x: string | number) {
                if (typeof x === "string") {
                    const s: string = x;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_typeof_narrowing_to_number() {
        let errors = check(r#"
            function f(x: string | number) {
                if (typeof x === "number") {
                    const n: number = x;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_null_narrowing() {
        let errors = check(r#"
            function f(x: string | null) {
                if (x !== null) {
                    const s: string = x;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_undefined_narrowing() {
        let errors = check(r#"
            function f(x: string | undefined) {
                if (x !== undefined) {
                    const s: string = x;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_assignment_valid() {
        let errors = check(r#"
            let x: string | number = "hello";
            x = 42;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_assignment_invalid() {
        let errors = check("const x: string | number = true;");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_intersection_object_literal() {
        let errors = check(r#"
            type Named = { name: string };
            type Aged = { age: number };
            const person: Named & Aged = { name: "Alice", age: 30 };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_of_literals() {
        let errors = check(r#"
            type Direction = "left" | "right" | "up" | "down";
            const dir: Direction = "left";
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_union_of_literals_invalid() {
        let errors = check(r#"
            type Direction = "left" | "right" | "up" | "down";
            const dir: Direction = "diagonal";
        "#);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_excess_property_fresh_literal_errors() {
        // Direct object literal: excess property check SHOULD apply
        let errors = check("const x: { a: number } = { a: 1, b: 2 };");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2353); // Excess property error
    }

    #[test]
    fn test_excess_property_variable_bypasses() {
        // Assigning through variable: excess property check should NOT apply
        // TypeScript's "freshness" rule - object literals lose freshness when assigned to a variable
        let errors = check(r#"
            const obj = { a: 1, b: 2 };
            const x: { a: number } = obj;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_excess_property_variable_still_checks_types() {
        // Even without excess checks, property types must still match
        let errors = check(r#"
            const obj = { a: "wrong" };
            const x: { a: number } = obj;
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_excess_property_variable_missing_property() {
        let errors = check(r#"
            const obj = { b: 2 };
            const x: { a: number } = obj;
        "#);
        assert_eq!(errors.len(), 1);
        // Note: Currently reports as type mismatch (2322) rather than missing property (2741)
        // because the variable assignment goes through is_assignable() not check_object_literal_against_type()
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_excess_property_function_param_variable() {
        let errors = check(r#"
            function f(x: { a: number }) {}
            const obj = { a: 1, b: 2 };
            f(obj);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_extends_basic() {
        // interface B extends A should inherit A's properties
        let errors = check(r#"
            interface Animal {
                name: string;
            }
            interface Dog extends Animal {
                breed: string;
            }
            const dog: Dog = { name: "Rex", breed: "German Shepherd" };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_extends_missing_base_property() {
        // Must have properties from base interface
        let errors = check(r#"
            interface Animal {
                name: string;
            }
            interface Dog extends Animal {
                breed: string;
            }
            const dog: Dog = { breed: "Labrador" };
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2741); // Missing property 'name'
    }

    #[test]
    fn test_interface_extends_missing_derived_property() {
        // Must have properties from derived interface too
        let errors = check(r#"
            interface Animal {
                name: string;
            }
            interface Dog extends Animal {
                breed: string;
            }
            const dog: Dog = { name: "Rex" };
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2741); // Missing property 'breed'
    }

    #[test]
    fn test_interface_extends_multiple() {
        // interface C extends A, B gets properties from both
        let errors = check(r#"
            interface Named {
                name: string;
            }
            interface Aged {
                age: number;
            }
            interface Person extends Named, Aged {
                email: string;
            }
            const person: Person = { name: "Alice", age: 30, email: "alice@example.com" };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_extends_multiple_missing() {
        // Must have properties from all extended interfaces
        let errors = check(r#"
            interface Named {
                name: string;
            }
            interface Aged {
                age: number;
            }
            interface Person extends Named, Aged {
                email: string;
            }
            const person: Person = { name: "Alice", email: "alice@example.com" };
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2741); // Missing 'age'
    }

    #[test]
    fn test_interface_extends_chain() {
        // A extends B extends C - should get all properties
        let errors = check(r#"
            interface A {
                a: number;
            }
            interface B extends A {
                b: string;
            }
            interface C extends B {
                c: boolean;
            }
            const obj: C = { a: 1, b: "hello", c: true };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_extends_nonexistent() {
        // Extending a non-existent interface should error
        let errors = check(r#"
            interface Dog extends NonExistent {
                breed: string;
            }
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2304); // Cannot find name 'NonExistent'
    }

    #[test]
    fn test_interface_extends_with_optional() {
        // Optional properties in base should remain optional
        let errors = check(r#"
            interface Base {
                required: string;
                optional?: number;
            }
            interface Derived extends Base {
                extra: boolean;
            }
            const obj: Derived = { required: "hello", extra: true };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_recursive_interface() {
        // Interface referencing itself (linked list)
        let errors = check(r#"
            interface ListNode {
                value: number;
                next: ListNode | null;
            }
            const node: ListNode = { value: 1, next: null };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_recursive_interface_nested() {
        // Nested recursive structure
        let errors = check(r#"
            interface TreeNode {
                value: number;
                left: TreeNode | null;
                right: TreeNode | null;
            }
            const tree: TreeNode = {
                value: 1,
                left: { value: 2, left: null, right: null },
                right: null
            };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_recursive_type_alias() {
        // Type alias referencing itself
        let errors = check(r#"
            type JsonValue = string | number | boolean | null | JsonArray | JsonObject;
            type JsonArray = JsonValue[];
            type JsonObject = { [key: string]: JsonValue };
            const data: JsonValue = { name: "test", values: [1, 2, 3] };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_method_signature() {
        let errors = check(r#"
            interface Calculator {
                add(a: number, b: number): number;
                subtract(a: number, b: number): number;
            }
            const calc: Calculator = {
                add: (a: number, b: number) => a + b,
                subtract: (a: number, b: number) => a - b
            };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_interface_readonly_property() {
        // Readonly properties should be accepted in object literals
        let errors = check(r#"
            interface Point {
                readonly x: number;
                readonly y: number;
            }
            const p: Point = { x: 10, y: 20 };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_type_alias_union() {
        let errors = check(r#"
            type StringOrNumber = string | number;
            const a: StringOrNumber = "hello";
            const b: StringOrNumber = 42;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_type_alias_intersection() {
        let errors = check(r#"
            type Named = { name: string };
            type Aged = { age: number };
            type Person = Named & Aged;
            const p: Person = { name: "Alice", age: 30 };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_type_alias_to_interface() {
        let errors = check(r#"
            interface User {
                name: string;
            }
            type UserAlias = User;
            const u: UserAlias = { name: "Bob" };
        "#);
        assert!(errors.is_empty());
    }

    // ========================================================================
    // Milestone 9: Generics
    // ========================================================================

    // M9.1: Parse Generic Type Parameters
    // ------------------------------------

    #[test]
    fn test_generic_function_declaration_basic() {
        // Basic generic function should parse without errors
        let errors = check(r#"
            function identity<T>(x: T): T {
                return x;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_function_with_multiple_type_params() {
        // Test that multiple type parameters are parsed and the function body
        // can reference them. The actual tuple inference is tested separately.
        let errors = check(r#"
            function makePair<A, B>(a: A, b: B): { first: A; second: B } {
                return { first: a, second: b };
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_interface_basic() {
        let errors = check(r#"
            interface Box<T> {
                value: T;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_type_alias_basic() {
        let errors = check(r#"
            type Pair<A, B> = { first: A; second: B };
        "#);
        assert!(errors.is_empty());
    }

    // M9.2: Instantiate Generic Types
    // --------------------------------

    #[test]
    fn test_generic_function_explicit_type_arg() {
        // Calling generic function with explicit type argument
        let errors = check(r#"
            function identity<T>(x: T): T {
                return x;
            }
            const result: number = identity<number>(42);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_function_explicit_type_arg_mismatch() {
        // Type argument says string, but passing number - should error
        let errors = check(r#"
            function identity<T>(x: T): T {
                return x;
            }
            const result = identity<string>(42);
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2345); // Argument of type 'X' is not assignable to parameter of type 'Y'
    }

    #[test]
    fn test_generic_interface_instantiation() {
        let errors = check(r#"
            interface Box<T> {
                value: T;
            }
            const numBox: Box<number> = { value: 42 };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_interface_instantiation_error() {
        let errors = check(r#"
            interface Box<T> {
                value: T;
            }
            const numBox: Box<number> = { value: "hello" };
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322); // Type 'X' is not assignable to type 'Y'
    }

    // M9.3: Infer Type Arguments
    // ---------------------------

    #[test]
    fn test_generic_function_type_inference_simple() {
        // identity(42) should infer T = number
        let errors = check(r#"
            function identity<T>(x: T): T {
                return x;
            }
            const result: number = identity(42);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_function_type_inference_string() {
        let errors = check(r#"
            function identity<T>(x: T): T {
                return x;
            }
            const result: string = identity("hello");
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_function_type_inference_mismatch() {
        // identity(42) infers T = number, but assigning to string should error
        let errors = check(r#"
            function identity<T>(x: T): T {
                return x;
            }
            const result: string = identity(42);
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2322);
    }

    #[test]
    fn test_generic_function_type_inference_multiple_args() {
        // Infer from multiple arguments
        let errors = check(r#"
            function first<T>(a: T, b: T): T {
                return a;
            }
            const result: number = first(1, 2);
        "#);
        assert!(errors.is_empty());
    }

    // M9.4: Generic Constraints
    // --------------------------

    #[test]
    fn test_generic_constraint_basic() {
        let errors = check(r#"
            function getLength<T extends { length: number }>(x: T): number {
                return x.length;
            }
            const len = getLength("hello");
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_constraint_violation() {
        let errors = check(r#"
            function getLength<T extends { length: number }>(x: T): number {
                return x.length;
            }
            const len = getLength(42);
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2344); // Type does not satisfy constraint
    }

    #[test]
    fn test_generic_constraint_with_explicit_type_arg() {
        // Explicit type arg that doesn't satisfy constraint
        let errors = check(r#"
            interface HasLength {
                length: number;
            }
            function getLength<T extends HasLength>(x: T): number {
                return x.length;
            }
            const len = getLength<number>(42);
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2344);
    }

    // M9.5: Generic Defaults
    // -----------------------

    #[test]
    fn test_generic_default_type() {
        let errors = check(r#"
            interface Container<T = string> {
                value: T;
            }
            const c: Container = { value: "hello" };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_default_type_override() {
        let errors = check(r#"
            interface Container<T = string> {
                value: T;
            }
            const c: Container<number> = { value: 42 };
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_function_default() {
        let errors = check(r#"
            function wrap<T = string>(x: T): { value: T } {
                return { value: x };
            }
            const result = wrap("hello");
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_optional_param_call_without_arg() {
        let errors = check("function f(x?: number) {} f();");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_optional_param_call_with_arg() {
        let errors = check("function f(x?: number) {} f(42);");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_optional_param_call_with_undefined() {
        let errors = check("function f(x?: number) {} f(undefined);");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_optional_param_wrong_type() {
        let errors = check(r#"function f(x?: number) {} f("hello");"#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2345);
    }

    #[test]
    fn test_required_param_after_optional_error() {
        let errors = check("function f(x?: number, y: string) {}");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 1016);
    }

    #[test]
    fn test_multiple_optional_params() {
        let errors = check("function f(x?: number, y?: string) {} f(); f(1); f(1, 'a');");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_optional_param_type_is_union_with_undefined() {
        let errors = check(r#"
            function f(x?: number) {
                let y: number | undefined = x;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_optional_param_strict_mode_deferred() {
        let errors = check(r#"
            function f(x?: number): number {
                return x;
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_rest_param_basic() {
        let errors = check("function f(...args: number[]) {} f(1, 2, 3);");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_rest_param_empty() {
        let errors = check("function f(...args: number[]) {} f();");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_rest_param_wrong_type() {
        let errors = check(r#"function f(...args: number[]) {} f(1, "hello", 3);"#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2345);
    }

    #[test]
    fn test_rest_param_must_be_last() {
        // Parser enforces rest param must be last - syntax error, not type error
    }

    #[test]
    fn test_rest_param_with_regular_params() {
        let errors = check("function f(x: number, ...rest: string[]) {} f(1, 'a', 'b');");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_rest_param_spread_call() {
        let errors = check(r#"
            function f(...args: number[]) {}
            const arr = [1, 2, 3];
            f(...arr);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_contextual_typing() {
        let errors = check(r#"
            const f: (x: number) => number = x => x + 1;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_explicit_types() {
        let errors = check(r#"
            const f = (x: number): number => x + 1;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_expression_body_call() {
        let errors = check(r#"
            const f = (x: number) => x * 2;
            const result: number = f(5);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_block_body_call() {
        let errors = check(r#"
            const f = (x: number): number => {
                return x * 2;
            };
            const result: number = f(5);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_method_shorthand_in_object() {
        let errors = check(r#"
            const obj = {
                greet(name: string): string {
                    return "Hello " + name;
                }
            };
            const result: string = obj.greet("World");
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_method_shorthand_equivalent() {
        let errors = check(r#"
            const obj1 = { foo(): number { return 1; } };
            const obj2 = { foo: function(): number { return 1; } };
            const x: number = obj1.foo();
            const y: number = obj2.foo();
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_with_rest_param() {
        let errors = check(r#"
            const sum = (...nums: number[]): number => {
                let total = 0;
                return total;
            };
            sum(1, 2, 3);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_function_expression_with_rest_param() {
        let errors = check(r#"
            const sum = function(...nums: number[]): number {
                return 0;
            };
            sum(1, 2, 3);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_arrow_with_optional_param() {
        let errors = check(r#"
            const greet = (name?: string): string => {
                return "Hello";
            };
            greet();
            greet("World");
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_combined_optional_and_rest_params() {
        let errors = check("function f(required: string, optional?: number, ...rest: boolean[]) {} f(\"hello\"); f(\"hello\", 42); f(\"hello\", 42, true, false);");
        assert!(errors.is_empty());
    }

    // ======================================================================
    // Milestone 11: Classes
    // ======================================================================

    #[test]
    fn test_class_basic_instantiation() {
        // Basic class with property and instantiation with `new`
        let errors = check(r#"
            class Point {
                x: number;
                y: number;
            }
            const p = new Point();
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_instance_property_access() {
        // Access properties on class instance
        let errors = check(r#"
            class Point {
                x: number;
                y: number;
            }
            const p = new Point();
            const x: number = p.x;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_instance_property_not_found() {
        // Error when accessing non-existent property
        let errors = check(r#"
            class Point {
                x: number;
                y: number;
            }
            const p = new Point();
            const z = p.z;
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339); // Property 'z' does not exist
    }

    #[test]
    fn test_class_constructor_with_params() {
        // Class with constructor that takes parameters
        let errors = check(r#"
            class Point {
                x: number;
                y: number;
                constructor(x: number, y: number) {
                    this.x = x;
                    this.y = y;
                }
            }
            const p = new Point(1, 2);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_constructor_wrong_arg_count() {
        // Error when passing wrong number of args to constructor
        let errors = check(r#"
            class Point {
                x: number;
                y: number;
                constructor(x: number, y: number) {
                    this.x = x;
                    this.y = y;
                }
            }
            const p = new Point(1);
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2554); // Expected N arguments, but got M
    }

    #[test]
    fn test_class_constructor_wrong_arg_type() {
        // Error when passing wrong type to constructor
        let errors = check(r#"
            class Point {
                x: number;
                y: number;
                constructor(x: number, y: number) {
                    this.x = x;
                    this.y = y;
                }
            }
            const p = new Point("hello", 2);
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2345); // Argument of type 'X' not assignable to 'Y'
    }

    #[test]
    fn test_class_method_call() {
        // Call method on class instance
        let errors = check(r#"
            class Calculator {
                add(a: number, b: number): number {
                    return a + b;
                }
            }
            const calc = new Calculator();
            const result: number = calc.add(1, 2);
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_method_wrong_arg_type() {
        // Error when calling method with wrong argument type
        let errors = check(r#"
            class Calculator {
                add(a: number, b: number): number {
                    return a + b;
                }
            }
            const calc = new Calculator();
            const result = calc.add("hello", 2);
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2345);
    }

    #[test]
    fn test_class_as_type_annotation() {
        // Use class name as type annotation
        let errors = check(r#"
            class User {
                name: string;
            }
            const user: User = new User();
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_type_mismatch() {
        // Error when assigning wrong type to class-typed variable
        let errors = check(r#"
            class User {
                name: string;
            }
            const user: User = { name: "alice" };
        "#);
        // Object literal should be assignable to class type (structural typing)
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_extends_basic() {
        // Derived class inherits properties from base
        let errors = check(r#"
            class Animal {
                name: string;
            }
            class Dog extends Animal {
                breed: string;
            }
            const dog = new Dog();
            const name: string = dog.name;
            const breed: string = dog.breed;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_extends_method() {
        // Derived class inherits methods from base
        let errors = check(r#"
            class Animal {
                speak(): string {
                    return "...";
                }
            }
            class Dog extends Animal {
                bark(): string {
                    return "woof";
                }
            }
            const dog = new Dog();
            const sound1: string = dog.speak();
            const sound2: string = dog.bark();
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_extends_property_not_on_base() {
        // Error when accessing derived property on base type
        let errors = check(r#"
            class Animal {
                name: string;
            }
            class Dog extends Animal {
                breed: string;
            }
            const animal = new Animal();
            const breed = animal.breed;
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339); // Property does not exist
    }

    #[test]
    fn test_class_assignable_to_base() {
        // Derived class is assignable to base class type
        let errors = check(r#"
            class Animal {
                name: string;
            }
            class Dog extends Animal {
                breed: string;
            }
            const dog = new Dog();
            const animal: Animal = dog;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_class_basic() {
        // Generic class with type parameter
        let errors = check(r#"
            class Box<T> {
                value: T;
            }
            const box = new Box<number>();
            const val: number = box.value;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_class_constraint_satisfied() {
        // Generic class with constraint that is satisfied
        let errors = check(r#"
            class Container<T extends { length: number }> {
                item: T;
            }
            const c = new Container<string>();
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_generic_class_constraint_violated() {
        // Error when constraint is not satisfied
        let errors = check(r#"
            class Container<T extends { length: number }> {
                item: T;
            }
            const c = new Container<number>();
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2344); // Type does not satisfy constraint
    }

    // ======================================================================
    // M11: Static Members
    // ======================================================================

    #[test]
    fn test_static_property_access() {
        // Access static property via ClassName.prop
        let errors = check(r#"
            class Counter {
                static count: number;
            }
            const c: number = Counter.count;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_static_method_call() {
        // Call static method via ClassName.method()
        let errors = check(r#"
            class Factory {
                static create(): string {
                    return "instance";
                }
            }
            const s: string = Factory.create();
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_static_property_not_on_instance() {
        // Error when accessing static property on instance
        let errors = check(r#"
            class Counter {
                static count: number;
            }
            const c = new Counter();
            const x = c.count;
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339); // Property does not exist
    }

    #[test]
    fn test_static_property_not_found() {
        // Error when accessing non-existent static property
        let errors = check(r#"
            class Counter {
                static count: number;
            }
            const x = Counter.nonexistent;
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339);
    }

    // ======================================================================
    // M11: Class Implements Interface
    // ======================================================================

    #[test]
    fn test_class_implements_interface() {
        // Class correctly implements interface
        let errors = check(r#"
            interface Printable {
                print(): void;
            }
            class Document implements Printable {
                print(): void {}
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_class_implements_missing_method() {
        // Error when class is missing interface method
        let errors = check(r#"
            interface Printable {
                print(): void;
            }
            class Document implements Printable {
            }
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2420); // Class incorrectly implements interface
    }

    #[test]
    fn test_class_implements_missing_property() {
        // Error when class is missing interface property
        let errors = check(r#"
            interface Named {
                name: string;
            }
            class Person implements Named {
            }
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2420);
    }

    #[test]
    fn test_class_implements_multiple_interfaces() {
        // Class implements multiple interfaces
        let errors = check(r#"
            interface Named {
                name: string;
            }
            interface Aged {
                age: number;
            }
            class Person implements Named, Aged {
                name: string;
                age: number;
            }
        "#);
        assert!(errors.is_empty());
    }

    // ======================================================================
    // M11: Parameter Properties
    // ======================================================================

    #[test]
    fn test_parameter_property_public() {
        // Parameter property with public modifier creates instance property
        let errors = check(r#"
            class Point {
                constructor(public x: number, public y: number) {}
            }
            const p = new Point(1, 2);
            const x: number = p.x;
            const y: number = p.y;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_parameter_property_readonly() {
        // Parameter property with readonly modifier
        let errors = check(r#"
            class Point {
                constructor(readonly x: number) {}
            }
            const p = new Point(1);
            const x: number = p.x;
        "#);
        assert!(errors.is_empty());
    }

    // ======================================================================
    // M11: this Type in Methods
    // ======================================================================

    #[test]
    fn test_this_type_in_method() {
        // 'this' in method refers to instance type
        let errors = check(r#"
            class Counter {
                count: number;
                increment(): void {
                    this.count = this.count + 1;
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_this_property_not_found() {
        // Error when accessing non-existent property via this
        let errors = check(r#"
            class Counter {
                count: number;
                increment(): void {
                    this.nonexistent = 1;
                }
            }
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2339);
    }

    // ======================================================================
    // M11: super() Calls
    // ======================================================================

    #[test]
    fn test_super_call_in_derived_constructor() {
        // super() call in derived class constructor
        let errors = check(r#"
            class Animal {
                constructor(public name: string) {}
            }
            class Dog extends Animal {
                constructor(name: string, public breed: string) {
                    super(name);
                }
            }
            const d = new Dog("Rex", "German Shepherd");
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_super_call_wrong_args() {
        // Error when super() called with wrong argument types
        let errors = check(r#"
            class Animal {
                constructor(public name: string) {}
            }
            class Dog extends Animal {
                constructor() {
                    super(42);
                }
            }
        "#);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, 2345); // Argument not assignable
    }

    // ======================================================================
    // M11: super Property Access
    // ======================================================================

    #[test]
    fn test_super_property_access() {
        // Access base class method via super
        let errors = check(r#"
            class Animal {
                speak(): string {
                    return "...";
                }
            }
            class Dog extends Animal {
                speak(): string {
                    return super.speak() + " woof";
                }
            }
        "#);
        assert!(errors.is_empty());
    }

    // ======================================================================
    // M11: Class Expressions
    // ======================================================================

    #[test]
    fn test_class_expression() {
        // Anonymous class expression
        let errors = check(r#"
            const MyClass = class {
                value: number;
            };
            const obj = new MyClass();
            const v: number = obj.value;
        "#);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_named_class_expression() {
        // Named class expression
        let errors = check(r#"
            const MyClass = class InnerName {
                value: number;
            };
            const obj = new MyClass();
            const v: number = obj.value;
        "#);
        assert!(errors.is_empty());
    }
}
