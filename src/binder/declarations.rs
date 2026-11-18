//! Declarations record their types in the value namespace (variables,
//! functions), the type namespace (interfaces, type aliases), or both
//! (classes, enums).

use oxc_ast::ast::*;

use crate::types::resolution;
use crate::types::{EnumMember, EnumValue, Type, TypeId, TypeParam};

use super::{Binder, BindingError, RedeclaredError};

impl Binder<'_> {
    pub(super) fn bind_variable_declaration(&mut self, decl: &VariableDeclaration) {
        let is_const = decl.kind == VariableDeclarationKind::Const;
        for declarator in &decl.declarations {
            self.bind_variable_declarator(declarator, is_const);
        }
    }

    /// The declared type comes from the annotation or the initializer. `let`
    /// and `var` widen literal types (`let x = 1` is `number`); `const` keeps
    /// them. Destructuring patterns are not bound yet.
    fn bind_variable_declarator(&mut self, declarator: &VariableDeclarator, is_const: bool) {
        if let BindingPattern::BindingIdentifier(ident) = &declarator.id {
            let name = ident.name.as_str();

            let ty_id = self.resolve_binding_type(declarator);
            let ty_id = if is_const {
                ty_id
            } else {
                self.widen_type(ty_id)
            };

            self.define_value(ident, ty_id);

            // `const C = class {}` also names an instance type.
            if let Some(Expression::ClassExpression(class)) = &declarator.init {
                let (instance_type_id, _, _) =
                    resolution::build_class_type(class, &mut self.symbols.arena);
                self.symbols.define_type(name, instance_type_id);
            }
        }

        if let Some(init) = &declarator.init {
            self.bind_expression(init);
        }
    }

    pub(super) fn bind_function_declaration(&mut self, decl: &Function) {
        if let Some(ident) = &decl.id {
            let ty_id = self.build_function_type(decl);

            self.define_value(ident, ty_id);
        }

        self.symbols.push_scope();
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

    /// Makes `T` in `function id<T>(x: T): T` resolvable in the current scope.
    fn bind_type_parameters(
        &mut self,
        type_params: &Option<oxc_allocator::Box<TSTypeParameterDeclaration>>,
    ) {
        if let Some(params) = type_params {
            let mut seen = rustc_hash::FxHashSet::default();
            for param in &params.params {
                let name = param.name.name.as_str();
                let span = param.name.span;

                // `<X, X>`: tsc reports each repeat (oxc allows these).
                if !seen.insert(name) {
                    self.errors.push(BindingError::Redeclared(RedeclaredError {
                        name: name.to_string(),
                        span,
                        block_scoped: false,
                    }));
                }

                let constraint = param
                    .constraint
                    .as_ref()
                    .map(|c| resolution::resolve_ts_type(c, &mut self.symbols.arena));
                let default = param
                    .default
                    .as_ref()
                    .map(|d| resolution::resolve_ts_type(d, &mut self.symbols.arena));

                let ty_id = self.symbols.arena.intern(Type::TypeParameter {
                    name: name.to_string(),
                    constraint,
                    default,
                });
                self.symbols.define_type(name, ty_id);
            }
        }
    }

    fn bind_formal_parameter(&mut self, param: &FormalParameter) {
        if let BindingPattern::BindingIdentifier(ident) = &param.pattern {
            let ty_id = self.resolve_type_annotation_oxc(&param.type_annotation);

            self.define_value(ident, ty_id);
        }
    }

    /// `catch (e)` is `unknown`, as under `useUnknownInCatchVariables`.
    pub(super) fn bind_catch_parameter(&mut self, param: &CatchParameter) {
        if let BindingPattern::BindingIdentifier(ident) = &param.pattern {
            let ty_id = param
                .type_annotation
                .as_ref()
                .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                .unwrap_or(TypeId::UNKNOWN);

            self.define_value(ident, ty_id);
        }
    }

    /// A class is a value (its constructor, with the static members) and a
    /// type (its instances).
    pub(super) fn bind_class_declaration(&mut self, decl: &Class) {
        if let Some(ident) = &decl.id {
            let name = ident.name.as_str();

            let (instance_type_id, constructor_type_id, static_type_id) =
                resolution::build_class_type(decl, &mut self.symbols.arena);

            let instance_type = self.symbols.arena.get(instance_type_id).clone();
            let class_type_params = if let Type::Object { type_params, .. } = &instance_type {
                type_params.clone()
            } else {
                vec![]
            };

            let static_type = self.symbols.arena.get(static_type_id).clone();
            let static_members = if let Type::Object { properties, .. } = static_type {
                properties
            } else {
                vec![]
            };

            let value_type_id = if let Some(ctor_id) = constructor_type_id {
                let ctor_type = self.symbols.arena.get(ctor_id).clone();
                if let Type::Function {
                    params,
                    type_params,
                    ..
                } = ctor_type
                {
                    self.symbols.arena.intern(Type::ClassConstructor {
                        params,
                        type_params,
                        static_members,
                    })
                } else {
                    self.symbols.arena.intern(Type::ClassConstructor {
                        params: vec![],
                        type_params: class_type_params,
                        static_members,
                    })
                }
            } else {
                // The implicit `constructor() {}`.
                self.symbols.arena.intern(Type::ClassConstructor {
                    params: vec![],
                    type_params: class_type_params,
                    static_members,
                })
            };

            self.define_value(ident, value_type_id);
            self.symbols.define_type(name, instance_type_id);
        }

        self.symbols.push_scope();

        for element in &decl.body.body {
            self.bind_class_element(element);
        }

        self.symbols.pop_scope();
    }

    fn bind_class_element(&mut self, element: &ClassElement) {
        match element {
            ClassElement::MethodDefinition(method) => {
                let func = &method.value;
                self.symbols.push_scope();

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

    pub(super) fn bind_interface_declaration(&mut self, decl: &TSInterfaceDeclaration) {
        let name = decl.id.name.as_str();
        let ty_id = self.build_interface_type(decl);

        self.symbols.define_type(name, ty_id);
    }

    /// An enum is both a value (`Color.Red`) and a type (`let c: Color`).
    /// Numeric members without an initializer count up from the previous one.
    /// Initializers other than number and string literals are not evaluated
    /// yet; they get the next number.
    pub(super) fn bind_enum_declaration(&mut self, decl: &TSEnumDeclaration) {
        let name = decl.id.name.as_str();
        let is_const = decl.r#const;

        let mut members = Vec::new();
        let mut next_value = 0i64;

        for member in &decl.body.members {
            let member_name = match &member.id {
                TSEnumMemberName::Identifier(ident) => ident.name.to_string(),
                TSEnumMemberName::String(s) => s.value.display().to_string(),
                TSEnumMemberName::ComputedString(s) => s.value.display().to_string(),
                TSEnumMemberName::ComputedTemplateString(t) => {
                    // Only substitution-free templates are valid member names.
                    t.quasis
                        .first()
                        .map(|q| q.value.raw.to_string())
                        .unwrap_or_default()
                }
            };

            let value = if let Some(init) = &member.initializer {
                match init {
                    Expression::NumericLiteral(n) => {
                        next_value = n.value as i64 + 1;
                        EnumValue::Number(n.value.into())
                    }
                    Expression::StringLiteral(s) => {
                        EnumValue::String(s.value.display().to_string())
                    }
                    Expression::UnaryExpression(unary)
                        if unary.operator == UnaryOperator::UnaryNegation =>
                    {
                        if let Expression::NumericLiteral(n) = &unary.argument {
                            next_value = -(n.value as i64) + 1;
                            EnumValue::Number((-n.value).into())
                        } else {
                            let val = EnumValue::Number((next_value as f64).into());
                            next_value += 1;
                            val
                        }
                    }
                    _ => {
                        let val = EnumValue::Number((next_value as f64).into());
                        next_value += 1;
                        val
                    }
                }
            } else {
                let val = EnumValue::Number((next_value as f64).into());
                next_value += 1;
                val
            };

            members.push(EnumMember::new(member_name, value));
        }

        let enum_type_id = self.symbols.arena.intern(Type::Enum {
            name: name.to_string(),
            members: members.clone(),
            is_const,
        });
        self.symbols.define_type(name, enum_type_id);
        self.define_value(&decl.id, enum_type_id);
    }

    /// A generic alias `type Box<T> = { value: T }` is stored as a
    /// parameterless function type whose return type is the alias body, so
    /// that instantiating it can reuse generic function substitution.
    pub(super) fn bind_type_alias_declaration(&mut self, decl: &TSTypeAliasDeclaration) {
        let name = decl.id.name.as_str();
        let body_type_id = self.resolve_ts_type(&decl.type_annotation);

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

        let ty_id = if type_params.is_empty() {
            body_type_id
        } else {
            self.symbols.arena.intern(Type::Function {
                params: vec![],
                return_type: body_type_id,
                type_params,
                type_predicate: None,
            })
        };

        self.symbols.define_type(name, ty_id);
    }

    pub(super) fn bind_arrow_function(&mut self, arrow: &ArrowFunctionExpression) {
        self.symbols.push_scope();
        self.bind_type_parameters(&arrow.type_parameters);

        for param in &arrow.params.items {
            self.bind_formal_parameter(param);
        }

        match arrow.get_function_body() {
            Some(body) => {
                for stmt in &body.statements {
                    self.bind_statement(stmt);
                }
            }
            None => {
                if let Some(expr) = arrow.get_expression() {
                    self.bind_expression(expr);
                }
            }
        }

        self.symbols.pop_scope();
    }

    /// The name of a function expression is only visible inside it:
    /// `const f = function foo() { foo(); }`.
    pub(super) fn bind_function_expression(&mut self, func: &Function) {
        self.symbols.push_scope();
        self.bind_type_parameters(&func.type_parameters);

        if let Some(ident) = &func.id {
            let ty_id = self.build_function_type(func);
            self.define_value(ident, ty_id);
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
