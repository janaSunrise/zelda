//! Binding declarations: variables, functions, classes, interfaces, type aliases.
//!
//! Each declaration type registers symbols in the appropriate namespace:
//! - Variables, functions: value namespace
//! - Interfaces, type aliases: type namespace
//! - Classes: both namespaces (constructor + instance type)

use oxc_ast::ast::*;

use crate::symbols::{ScopeKind, SymbolKind};
use crate::types::resolution;
use crate::types::{Type, TypeParam};

use super::Binder;

impl Binder {
    /// Bind a variable declaration (let, const, var).
    pub(super) fn bind_variable_declaration(&mut self, decl: &VariableDeclaration) {
        let is_const = decl.kind == VariableDeclarationKind::Const;
        for declarator in &decl.declarations {
            self.bind_variable_declarator(declarator, is_const);
        }
    }

    /// Bind a single variable declarator.
    ///
    /// 1. Extract the name from the binding pattern
    /// 2. Get type from annotation or infer from initializer
    /// 3. Widen literal types for let/var (not const)
    /// 4. Add symbol to current scope
    /// 5. Check initializer for undefined references
    fn bind_variable_declarator(&mut self, declarator: &VariableDeclarator, is_const: bool) {
        if let BindingPattern::BindingIdentifier(ident) = &declarator.id {
            let name = ident.name.as_str();
            let span = ident.span;

            let ty = self.resolve_binding_type(declarator);
            // Widen literal types for let/var
            let ty = if is_const { ty } else { self.widen_type(ty) };

            if let Err(err) = self.symbols.define(name, ty, SymbolKind::Variable, span) {
                self.errors.push(err.into());
            }

            // For class expressions, also register instance type in type namespace
            if let Some(Expression::ClassExpression(class)) = &declarator.init {
                let (instance_type, _, _) = resolution::build_class_type(class);
                if let Err(err) = self.symbols.define_type(name, instance_type, SymbolKind::Class, span) {
                    self.errors.push(err.into());
                }
            }
        }

        if let Some(init) = &declarator.init {
            self.bind_expression(init);
        }
    }

    /// Bind a function declaration.
    ///
    /// 1. Add function to current scope (before body, for recursion)
    /// 2. Push new function scope
    /// 3. Add type parameters to type namespace (for generics)
    /// 4. Add parameters to function scope
    /// 5. Bind body statements
    /// 6. Pop back to parent scope
    pub(super) fn bind_function_declaration(&mut self, decl: &Function) {
        if let Some(ident) = &decl.id {
            let name = ident.name.as_str();
            let span = ident.span;
            let ty = self.build_function_type(decl);

            if let Err(err) = self.symbols.define(name, ty, SymbolKind::Function, span) {
                self.errors.push(err.into());
            }
        }

        self.symbols.push_scope(ScopeKind::Function);

        // Bind type parameters to type namespace within function scope
        self.bind_type_parameters(&decl.type_parameters);

        for param in &decl.params.items {
            self.bind_formal_parameter(param);
        }

        if let Some(body) = &decl.body {
            for stmt in &body.statements {
                self.bind_statement(stmt);
            }
        }

        self.symbols.pop_scope();
    }

    /// Bind type parameters to the type namespace.
    ///
    /// For a function like `function id<T>(x: T): T`, this registers `T` as a
    /// type in the current scope so that parameter types like `T` can be resolved.
    fn bind_type_parameters(&mut self, type_params: &Option<oxc_allocator::Box<TSTypeParameterDeclaration>>) {
        if let Some(params) = type_params {
            for param in &params.params {
                let name = param.name.name.as_str();
                let span = param.name.span;

                // Build the TypeParameter type
                let constraint = param.constraint.as_ref().map(|c| resolution::resolve_ts_type(c));
                let default = param.default.as_ref().map(|d| resolution::resolve_ts_type(d));

                let ty = Type::TypeParameter {
                    name: name.to_string(),
                    constraint: constraint.map(Box::new),
                    default: default.map(Box::new),
                };

                // Register in type namespace
                if let Err(err) = self.symbols.define_type(name, ty, SymbolKind::TypeAlias, span) {
                    self.errors.push(err.into());
                }
            }
        }
    }

    fn bind_formal_parameter(&mut self, param: &FormalParameter) {
        if let BindingPattern::BindingIdentifier(ident) = &param.pattern {
            let name = ident.name.as_str();
            let span = ident.span;
            let ty = self.resolve_type_annotation_oxc(&param.type_annotation);

            if let Err(err) = self.symbols.define(name, ty, SymbolKind::Parameter, span) {
                self.errors.push(err.into());
            }
        }
    }

    /// Bind a catch clause parameter.
    ///
    /// Catch parameters: `catch (e)` or `catch (e: Error)`.
    pub(super) fn bind_catch_parameter(&mut self, param: &CatchParameter) {
        if let BindingPattern::BindingIdentifier(ident) = &param.pattern {
            let name = ident.name.as_str();
            let span = ident.span;
            // Catch parameters are typically `unknown` or `any` in TypeScript
            let ty = param
                .type_annotation
                .as_ref()
                .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                .unwrap_or(Type::Unknown);

            if let Err(err) = self.symbols.define(name, ty, SymbolKind::Parameter, span) {
                self.errors.push(err.into());
            }
        }
    }

    /// Bind a class declaration.
    ///
    /// Classes exist in both value and type namespaces:
    /// - Value: ClassConstructor type (constructor params + static members)
    /// - Type: the instance type (Object with properties and methods)
    pub(super) fn bind_class_declaration(&mut self, decl: &Class) {
        if let Some(ident) = &decl.id {
            let name = ident.name.as_str();
            let span = ident.span;

            // Build the class type (instance type, constructor type, and static type)
            let (instance_type, constructor_type, static_type) = resolution::build_class_type(decl);

            // Extract type params from instance_type
            let class_type_params = if let Type::Object { type_params, .. } = &instance_type {
                type_params.clone()
            } else {
                vec![]
            };

            // Extract static members from static_type
            let static_members = if let Type::Object { properties, .. } = static_type {
                properties
            } else {
                vec![]
            };

            // Value namespace: ClassConstructor with constructor params and static members
            let value_type = if let Some(Type::Function { params, type_params, .. }) = constructor_type {
                Type::ClassConstructor {
                    params,
                    type_params,
                    static_members,
                }
            } else {
                // No explicit constructor - default constructor with no parameters
                Type::ClassConstructor {
                    params: vec![],
                    type_params: class_type_params,
                    static_members,
                }
            };

            if let Err(err) = self.symbols.define(name, value_type, SymbolKind::Class, span) {
                self.errors.push(err.into());
            }
            // Type namespace: the instance type (Object with properties)
            if let Err(err) = self.symbols.define_type(name, instance_type, SymbolKind::Class, span) {
                self.errors.push(err.into());
            }
        }

        self.symbols.push_scope(ScopeKind::Class);

        for element in &decl.body.body {
            self.bind_class_element(element);
        }

        self.symbols.pop_scope();
    }

    fn bind_class_element(&mut self, element: &ClassElement) {
        match element {
            ClassElement::MethodDefinition(method) => {
                let func = &method.value;
                self.symbols.push_scope(ScopeKind::Function);

                for param in &func.params.items {
                    self.bind_formal_parameter(param);
                }

                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.bind_statement(stmt);
                    }
                }

                self.symbols.pop_scope();
            }
            ClassElement::PropertyDefinition(prop) => {
                if let Some(value) = &prop.value {
                    self.bind_expression(value);
                }
            }
            _ => {}
        }
    }

    /// Bind an interface declaration.
    ///
    /// Interfaces only exist in the type namespace.
    pub(super) fn bind_interface_declaration(&mut self, decl: &TSInterfaceDeclaration) {
        let name = decl.id.name.as_str();
        let span = decl.id.span;
        let ty = self.build_interface_type(decl);

        if let Err(err) = self.symbols.define_type(name, ty, SymbolKind::Interface, span) {
            self.errors.push(err.into());
        }
    }

    /// Bind a type alias declaration.
    ///
    /// Type aliases only exist in the type namespace.
    /// For generic type aliases like `type Foo<T> = ...`, we wrap the body in a
    /// GenericTypeAlias structure to preserve type parameters for later instantiation.
    pub(super) fn bind_type_alias_declaration(&mut self, decl: &TSTypeAliasDeclaration) {
        let name = decl.id.name.as_str();
        let span = decl.id.span;
        let body_type = self.resolve_ts_type(&decl.type_annotation);

        // Extract type parameters for generic type aliases
        let type_params: Vec<TypeParam> = decl
            .type_parameters
            .as_ref()
            .map(|params| {
                params
                    .params
                    .iter()
                    .map(|p| {
                        let mut tp = TypeParam::new(p.name.name.to_string());
                        if let Some(constraint) = &p.constraint {
                            tp = tp.with_constraint(self.resolve_ts_type(constraint));
                        }
                        if let Some(default) = &p.default {
                            tp = tp.with_default(self.resolve_ts_type(default));
                        }
                        tp
                    })
                    .collect()
            })
            .unwrap_or_default();

        // For generic type aliases, wrap in a Function type to preserve type params
        // This is a bit of a hack but allows reusing existing infrastructure
        // The return_type is the actual alias body, params are empty
        let ty = if type_params.is_empty() {
            body_type
        } else {
            // Store as a generic "alias function" - the return type is the alias body
            // When resolved with type args, we substitute and return the return type
            Type::Function {
                params: vec![],
                return_type: Box::new(body_type),
                type_params,
                type_predicate: None,
            }
        };

        if let Err(err) = self.symbols.define_type(name, ty, SymbolKind::TypeAlias, span) {
            self.errors.push(err.into());
        }
    }

    /// Bind an arrow function expression.
    ///
    /// Arrow functions: `() => expr` or `() => { stmts }`.
    /// In oxc, the body is always a FunctionBody struct. When `arrow.expression` is true,
    /// it contains a single ExpressionStatement wrapping the expression.
    pub(super) fn bind_arrow_function(&mut self, arrow: &ArrowFunctionExpression) {
        self.symbols.push_scope(ScopeKind::Function);

        // Bind type parameters for generic arrow functions
        self.bind_type_parameters(&arrow.type_parameters);

        for param in &arrow.params.items {
            self.bind_formal_parameter(param);
        }

        for stmt in &arrow.body.statements {
            self.bind_statement(stmt);
        }

        self.symbols.pop_scope();
    }

    /// Bind a named function expression.
    ///
    /// Named function expressions bind their name inside their own scope.
    /// `const f = function foo() { foo(); }` - `foo` is only visible inside.
    pub(super) fn bind_function_expression(&mut self, func: &Function) {
        self.symbols.push_scope(ScopeKind::Function);

        // Bind type parameters for generic function expressions
        self.bind_type_parameters(&func.type_parameters);

        if let Some(ident) = &func.id {
            let name = ident.name.as_str();
            let span = ident.span;
            let ty = self.build_function_type(func);
            if let Err(err) = self.symbols.define(name, ty, SymbolKind::Function, span) {
                self.errors.push(err.into());
            }
        }

        for param in &func.params.items {
            self.bind_formal_parameter(param);
        }

        if let Some(body) = &func.body {
            for stmt in &body.statements {
                self.bind_statement(stmt);
            }
        }

        self.symbols.pop_scope();
    }
}
