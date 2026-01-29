//! Call expression type checking.
//!
//! This module handles type checking for all call-related expressions:
//! - Regular function calls: `foo(args)`
//! - Method calls: `obj.method(args)`
//! - Constructor calls: `new Class(args)`
//! - Super calls: `super(args)`
//!
//! # Type Argument Handling
//!
//! For generic functions and classes, this module handles:
//! - Explicit type arguments: `identity<string>(x)`
//! - Type argument inference: `identity(x)` where T is inferred from x
//! - Constraint checking: ensuring type arguments satisfy extends clauses

use oxc_ast::ast::*;
use oxc_span::{GetSpan, Span};

use crate::types::{Param, Type};

use super::{Checker, TypeError};

impl<'a> Checker<'a> {
    /// Check a function call for argument count and type errors.
    pub(super) fn check_call_expression(&mut self, call: &CallExpression) {
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
        if let Type::Function {
            params,
            type_params,
            ..
        } = callee_type
        {
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
                rustc_hash::FxHashMap::default()
            };

            // Check constraints for each type argument
            // Use TS2344 for explicit type args, TS2345 for inferred type args
            let has_explicit_type_args = !explicit_type_args.is_empty();
            for tp in &type_params {
                if let Some(constraint) = &tp.constraint
                    && let Some(type_arg) = substitutions.get(&tp.name)
                        && !self.satisfies_constraint(type_arg, constraint) {
                            if has_explicit_type_args {
                                // Explicit type args: "Type 'X' does not satisfy constraint 'Y'"
                                self.errors.push(TypeError::constraint_violation(
                                    type_arg, constraint, call.span,
                                ));
                            } else {
                                // Inferred type args: "Argument of type 'X' is not assignable to parameter of type 'Y'"
                                self.errors.push(TypeError::argument_not_assignable(
                                    type_arg, constraint, call.span,
                                ));
                            }
                        }
            }

            // Substitute type parameters in parameter types for checking
            let instantiated_params: Vec<Param> = params
                .iter()
                .map(|p| Param {
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

            // Check arguments against instantiated parameters
            self.check_arguments(&instantiated_params, &call.arguments, call.span);
        }
    }

    /// Check a super() call in a derived class constructor.
    pub(super) fn check_super_call(&mut self, call: &CallExpression) {
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
        let parent_constructor = self
            .symbols
            .lookup(&parent_class_name)
            .map(|s| s.ty.clone());

        let params = match parent_constructor {
            Some(Type::ClassConstructor { params, .. }) => params,
            Some(Type::Function { params, .. }) => params, // For backward compatibility
            _ => return,
        };

        // Check arguments against parent constructor parameters
        self.check_arguments(&params, &call.arguments, call.span);
    }

    /// Check a new expression for constructor argument count and type errors.
    pub(super) fn check_new_expression(&mut self, new_expr: &NewExpression) {
        // First check sub-expressions
        self.check_expression(&new_expr.callee);
        for arg in &new_expr.arguments {
            if let Some(expr) = arg.as_expression() {
                self.check_expression(expr);
            }
        }

        // Get the constructor type from the value namespace
        let constructor_type = if let Expression::Identifier(ident) = &new_expr.callee {
            self.symbols
                .lookup(ident.name.as_str())
                .map(|s| s.ty.clone())
        } else {
            None
        };

        // Extract params and type_params from either Function or ClassConstructor
        let (params, type_params) = match constructor_type {
            Some(Type::Function {
                params,
                type_params,
                ..
            }) => (params, type_params),
            Some(Type::ClassConstructor {
                params,
                type_params,
                ..
            }) => (params, type_params),
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
            rustc_hash::FxHashMap::default()
        };

        // Check type parameter constraints
        // Use TS2344 for explicit type args, TS2345 for inferred type args
        let has_explicit_type_args = !explicit_type_args.is_empty();
        for tp in &type_params {
            if let Some(constraint) = &tp.constraint
                && let Some(type_arg) = substitutions.get(&tp.name)
                    && !self.satisfies_constraint(type_arg, constraint) {
                        if has_explicit_type_args {
                            self.errors.push(TypeError::constraint_violation(
                                type_arg,
                                constraint,
                                new_expr.span,
                            ));
                        } else {
                            self.errors.push(TypeError::argument_not_assignable(
                                type_arg,
                                constraint,
                                new_expr.span,
                            ));
                        }
                    }
        }

        // Instantiate parameter types
        let instantiated_params: Vec<Param> = params
            .iter()
            .map(|p| Param {
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

        // Check arguments against instantiated parameters
        self.check_arguments(&instantiated_params, &new_expr.arguments, new_expr.span);
    }

    /// Check that arguments match parameter types.
    ///
    /// This is the unified argument checking logic used by:
    /// - `check_call_expression` for regular function calls
    /// - `check_super_call` for super() constructor calls
    /// - `check_new_expression` for new ClassName() calls
    ///
    /// Handles:
    /// - Argument count validation (too few / too many)
    /// - Rest parameter handling (variadic functions)
    /// - Type compatibility checking for each argument
    fn check_arguments(
        &mut self,
        params: &[Param],
        args: &oxc_allocator::Vec<'_, Argument<'_>>,
        call_span: Span,
    ) {
        let arg_count = args.len();
        let required_params = params.iter().filter(|p| !p.optional && !p.rest).count();
        let has_rest = params.last().map(|p| p.rest).unwrap_or(false);

        // Check argument count
        if arg_count < required_params {
            self.errors.push(TypeError::wrong_argument_count(
                required_params,
                arg_count,
                call_span,
            ));
        } else if !has_rest && arg_count > params.len() {
            // Too many arguments (and no rest param)
            self.errors.push(TypeError::wrong_argument_count(
                params.len(),
                arg_count,
                call_span,
            ));
        }

        // Check argument types
        let non_rest_param_count = if has_rest {
            params.len() - 1
        } else {
            params.len()
        };

        for (i, arg) in args.iter().enumerate() {
            if let Some(expr) = arg.as_expression() {
                let arg_type = self.infer_expression(expr);

                let param_type = if i < non_rest_param_count {
                    // Regular parameter
                    &params[i].ty
                } else if has_rest {
                    // Rest parameter - check against element type of the array
                    let rest_param = params.last().unwrap();
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
