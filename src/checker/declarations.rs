//! Declaration type checking.
//!
//! For function declarations, this module:
//! - Collects all return statements from the function body
//! - Validates return types against the declared return type
//! - Detects missing returns in non-void functions

use oxc_ast::ast::*;
use oxc_span::Span;

use crate::errors;
use crate::symbols::{ScopeKind, SymbolKind};
use crate::types::{Type, TypeId};

use super::{Checker, TypeError};

impl<'a> Checker<'a> {
    /// Check a variable declaration for type compatibility.
    pub(super) fn check_variable_declaration(&mut self, decl: &VariableDeclaration) {
        let is_const = decl.kind == VariableDeclarationKind::Const;

        for declarator in &decl.declarations {
            // Track variable name for definite assignment analysis
            let var_name = match &declarator.id {
                BindingPattern::BindingIdentifier(ident) => Some(ident.name.to_string()),
                _ => None,
            };

            if let Some(init) = &declarator.init {
                // Variable is initialized - mark as assigned
                if let Some(name) = &var_name {
                    self.assigned_vars.insert(name.clone());
                }
                // Check sub-expressions first
                self.check_expression(init);

                // Infer the initializer type
                let init_type_id = self.infer_expression(init);

                // Widen literal types for `let` and `var`
                let init_type_id = if is_const {
                    init_type_id
                } else {
                    self.widen_type(init_type_id)
                };

                // If there's a type annotation, check compatibility
                if let Some(annotation) = &declarator.type_annotation {
                    let declared_type_id = self.resolve_ts_type(&annotation.type_annotation);
                    // Resolve computed types (keyof, indexed access, mapped types)
                    let declared_type_id = self.resolve_computed_type(declared_type_id);

                    // Check for object literal specific errors (missing/excess properties)
                    if let Expression::ObjectExpression(obj) = init {
                        self.check_object_literal_against_type(
                            obj,
                            declared_type_id,
                            declarator.span,
                        );
                    } else if let Expression::ArrayExpression(arr) = init {
                        // Contextual typing: check array literal against tuple type
                        self.check_array_literal_against_type(arr, declared_type_id, declarator.span);
                    } else if !self.is_assignable(init_type_id, declared_type_id) {
                        // Check if the error is due to missing properties
                        let missing = self.find_missing_properties(init_type_id, declared_type_id);
                        let init_str = self.fmt_type(init_type_id);
                        let declared_str = self.fmt_type(declared_type_id);
                        if missing.len() > 1 {
                            // Report TS2739 for multiple missing properties
                            self.errors.push(TypeError::missing_properties(
                                &missing,
                                &init_str,
                                &declared_str,
                                declarator.span,
                            ));
                        } else if missing.len() == 1 {
                            // Report TS2741 for single missing property
                            self.errors.push(TypeError::missing_property(
                                &missing[0],
                                &init_str,
                                &declared_str,
                                declarator.span,
                            ));
                        } else {
                            self.errors.push(TypeError::not_assignable(
                                &init_str,
                                &declared_str,
                                declarator.span,
                            ));
                        }
                    }
                }
            } else {
                // Variable declared without initializer - track as uninitialized
                if decl.kind == VariableDeclarationKind::Let
                    && let Some(name) = var_name
                {
                    self.uninitialized_vars.insert(name);
                }
            }
        }
    }

    /// Check an interface declaration.
    pub(super) fn check_interface_declaration(&mut self, decl: &TSInterfaceDeclaration) {
        for heritage in &decl.extends {
            let name = match &heritage.expression {
                Expression::Identifier(ident) => ident.name.to_string(),
                _ => continue,
            };
            if self.symbols.lookup_type(&name).is_none() {
                self.errors
                    .push(TypeError::undefined_type(&name, heritage.span));
            }
        }
    }

    /// Check a class declaration for type errors.
    pub(super) fn check_class_declaration(&mut self, class: &Class) {
        let class_name = class.id.as_ref().map(|id| id.name.as_str()).unwrap_or("");

        // Get the class's instance type
        let class_type_id = self.symbols.lookup_type(class_name).map(|s| s.ty);

        let class_type_id = match class_type_id {
            Some(ty_id) => ty_id,
            None => return,
        };

        // Check implements clause
        for heritage in &class.implements {
            let interface_name = match &heritage.expression {
                TSTypeName::IdentifierReference(ident) => ident.name.to_string(),
                TSTypeName::QualifiedName(qual) => qual.right.name.to_string(),
                TSTypeName::ThisExpression(_) => continue,
            };

            // Get type arguments from the implements clause
            let type_args: Vec<TypeId> = heritage
                .type_arguments
                .as_ref()
                .map(|args| {
                    args.params
                        .iter()
                        .map(|t| self.resolve_ts_type(t))
                        .collect()
                })
                .unwrap_or_default();

            // Resolve the interface type with type arguments
            let interface_type_id = self.resolve_type_ref_with_args(&interface_name, &type_args);

            if let Some(interface_id) = interface_type_id {
                let interface_type = self.get_type(interface_id).clone();
                if let Type::Object {
                    properties: interface_props,
                    ..
                } = interface_type
                {
                    let class_type = self.get_type(class_type_id).clone();
                    let class_props = if let Type::Object {
                        properties,
                        extends,
                        ..
                    } = class_type
                    {
                        self.resolve_object_properties(&properties, &extends)
                    } else {
                        vec![]
                    };

                    let class_prop_names: std::collections::HashSet<&str> =
                        class_props.iter().map(|p| p.name.as_str()).collect();

                    for interface_prop in &interface_props {
                        // Check if property exists in class
                        if !interface_prop.optional
                            && !class_prop_names.contains(interface_prop.name.as_str())
                        {
                            self.errors.push(TypeError::incorrectly_implements(
                                class_name,
                                &interface_name,
                                class.span,
                            ));
                            break;
                        }

                        // Check type compatibility if property exists
                        if let Some(class_prop) =
                            class_props.iter().find(|p| p.name == interface_prop.name)
                            && !self.is_assignable(class_prop.ty, interface_prop.ty)
                        {
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

        // Set class context for super call checking
        let prev_class = self.current_class.take();
        self.current_class = Some(class_name.to_string());

        // Check class body (methods, etc.)
        for element in &class.body.body {
            if let ClassElement::MethodDefinition(method) = element
                && let Some(body) = &method.value.body
            {
                for stmt in &body.statements {
                    self.check_statement(stmt);
                }
            }
        }

        // Restore previous context
        self.current_class = prev_class;
    }

    /// Check a function declaration for return type compatibility.
    pub(super) fn check_function_declaration(&mut self, func: &Function) {
        // Validate parameter order (even for functions without body)
        self.validate_function_parameters(&func.params);

        if let Some(body) = &func.body {
            // Push function scope to match binder's scope structure
            self.symbols.push_scope(ScopeKind::Function);

            // Bind type parameters to type namespace for generic functions
            if let Some(type_params) = &func.type_parameters {
                for param in &type_params.params {
                    let name = param.name.name.as_str();
                    let constraint_id = param
                        .constraint
                        .as_ref()
                        .map(|c| self.resolve_ts_type(c));
                    let default_id = param
                        .default
                        .as_ref()
                        .map(|d| self.resolve_ts_type(d));

                    let ty_id = self.intern(Type::TypeParameter {
                        name: name.to_string(),
                        constraint: constraint_id,
                        default: default_id,
                    });

                    let _ =
                        self.symbols
                            .define_type(name, ty_id, SymbolKind::TypeAlias, param.name.span);
                }
            }

            // Re-bind parameters in this scope for the checker
            for param in &func.params.items {
                if let BindingPattern::BindingIdentifier(ident) = &param.pattern {
                    let base_ty_id = param
                        .type_annotation
                        .as_ref()
                        .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(TypeId::ANY);

                    // Optional parameters have type T | undefined inside the function
                    let ty_id = if param.optional {
                        self.union_types(base_ty_id, TypeId::UNDEFINED)
                    } else {
                        base_ty_id
                    };

                    let _ = self.symbols.define(
                        ident.name.as_str(),
                        ty_id,
                        SymbolKind::Parameter,
                        ident.span,
                    );
                }
            }

            // Assertion functions return void, type guards return boolean.
            let declared_return_id = func.return_type.as_ref().map(|ann| {
                if let oxc_ast::ast::TSType::TSTypePredicate(pred) = &ann.type_annotation {
                    if pred.asserts {
                        TypeId::VOID
                    } else {
                        TypeId::BOOLEAN
                    }
                } else {
                    self.resolve_ts_type(&ann.type_annotation)
                }
            });

            // Collect return types from body
            let return_types = self.collect_return_types(body);

            // If there's a declared return type, check each return against it
            if let Some(declared_id) = declared_return_id {
                for (ret_type_id, span) in &return_types {
                    if !self.is_assignable(*ret_type_id, declared_id) {
                        let ret_str = self.fmt_type(*ret_type_id);
                        let declared_str = self.fmt_type(declared_id);
                        self.errors
                            .push(TypeError::not_assignable(&ret_str, &declared_str, *span));
                    }
                }

                // Check for missing return in non-void functions
                let declared = self.get_type(declared_id);
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

    /// Validate function parameter order.
    pub(super) fn validate_function_parameters(&mut self, params: &FormalParameters) {
        let mut seen_optional = false;

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
    }

    /// Collect all return statement types from a function body.
    pub(super) fn collect_return_types(&mut self, body: &FunctionBody) -> Vec<(TypeId, Span)> {
        let mut returns = Vec::new();
        self.collect_returns_from_statements(&body.statements, &mut returns);
        returns
    }

    fn collect_returns_from_statements(
        &mut self,
        stmts: &[Statement],
        returns: &mut Vec<(TypeId, Span)>,
    ) {
        for stmt in stmts {
            self.collect_returns_from_statement(stmt, returns);
        }
    }

    fn collect_returns_from_statement(
        &mut self,
        stmt: &Statement,
        returns: &mut Vec<(TypeId, Span)>,
    ) {
        match stmt {
            Statement::ReturnStatement(ret) => {
                let ty_id = ret
                    .argument
                    .as_ref()
                    .map(|arg| self.infer_expression(arg))
                    .unwrap_or(TypeId::UNDEFINED);
                returns.push((ty_id, ret.span));
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
