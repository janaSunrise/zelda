//! Declaration type checking.
//!
//! For function declarations, this module:
//! - Collects all return statements from the function body
//! - Validates return types against the declared return type
//! - Detects missing returns in non-void functions

use oxc_ast::ast::*;
use oxc_span::{GetSpan, Span};

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
                        self.check_array_literal_against_type(
                            arr,
                            declared_type_id,
                            declarator.span,
                        );
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
        let name = decl.id.name.as_str();
        if self.is_own_base_type(name) {
            let params = decl.type_parameters.as_ref().map_or(String::new(), |tp| {
                let names: Vec<&str> = tp.params.iter().map(|p| p.name.name.as_str()).collect();
                format!("<{}>", names.join(", "))
            });
            let display = format!("{name}{params}");
            self.errors.push(TypeError::new(
                errors::CIRCULAR_BASE_TYPE.format(&[&display]),
                decl.id.span,
                errors::CIRCULAR_BASE_TYPE.code,
            ));
        }
    }

    /// Check a class declaration for type errors.
    pub(super) fn check_class_declaration(&mut self, class: &Class) {
        let class_name = class.id.as_ref().map(|id| id.name.as_str()).unwrap_or("");

        self.check_property_initialization(class);

        if let Some(id) = &class.id
            && self.is_own_base_type(class_name)
        {
            self.errors.push(TypeError::new(
                errors::CIRCULAR_BASE_EXPRESSION.format(&[class_name]),
                id.span,
                errors::CIRCULAR_BASE_EXPRESSION.code,
            ));
        }

        // Get the class's instance type
        let class_type_id = self.symbols.lookup_type(class_name).map(|s| s.ty);

        let Some(class_type_id) = class_type_id else {
            return;
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
                    let constraint_id = param.constraint.as_ref().map(|c| self.resolve_ts_type(c));
                    let default_id = param.default.as_ref().map(|d| self.resolve_ts_type(d));

                    let ty_id = self.intern(Type::TypeParameter {
                        name: name.to_string(),
                        constraint: constraint_id,
                        default: default_id,
                    });

                    let _ = self.symbols.define_type(
                        name,
                        ty_id,
                        SymbolKind::TypeAlias,
                        param.name.span,
                    );
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

                    if let Some(symbol) = ident.symbol_id.get() {
                        self.symbols.define_value(self.scoping, symbol, ty_id);
                    }
                }
            }

            // Assertion functions return void, type guards return boolean.
            let declared_return_id = func.return_type.as_ref().map(|ann| {
                if let TSType::TSTypePredicate(pred) = &ann.type_annotation {
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

                // A return is only required if the end of the body is reachable
                // and the (awaited) return type doesn't admit `undefined`.
                if !func.generator && can_complete_normally(&body.statements) {
                    let required_id = if func.r#async {
                        self.awaited_type(declared_id)
                    } else {
                        declared_id
                    };
                    let requires_return = !self.admits_undefined(required_id)
                        && !matches!(self.get_type(required_id), Type::Never);
                    if requires_return {
                        // TS2355 if the function never returns a value, TS2366 if
                        // only some paths do. tsc reports on the return type.
                        let diagnostic = if return_types.is_empty() {
                            errors::MUST_RETURN_VALUE
                        } else {
                            errors::MISSING_RETURN
                        };
                        let span = func
                            .return_type
                            .as_ref()
                            .map_or(func.span, |ann| ann.type_annotation.span());
                        self.errors.push(TypeError::new(
                            diagnostic.format(&[]),
                            span,
                            diagnostic.code,
                        ));
                    }
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

    /// Whether the named class or interface appears in its own chain of base
    /// types (`class A extends B {}; class B extends A {}`).
    fn is_own_base_type(&self, name: &str) -> bool {
        let base_names = |type_name: &str| -> Vec<String> {
            let Some(symbol) = self.symbols.lookup_type(type_name) else {
                return Vec::new();
            };
            let Type::Object { extends, .. } = self.get_type(symbol.ty) else {
                return Vec::new();
            };
            extends
                .iter()
                .filter_map(|&base| match self.get_type(base) {
                    Type::TypeRef { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect()
        };

        let mut visited = rustc_hash::FxHashSet::default();
        let mut pending = base_names(name);
        while let Some(base) = pending.pop() {
            if base == name {
                return true;
            }
            if visited.insert(base.clone()) {
                pending.extend(base_names(&base));
            }
        }
        false
    }

    /// TS2564 (`strictPropertyInitialization`): an instance property whose
    /// type excludes `undefined` must be initialized inline or assigned on
    /// every path through the constructor.
    fn check_property_initialization(&mut self, class: &Class) {
        if class.declare || self.in_declaration_file {
            return;
        }
        let constructor_body = class.body.body.iter().find_map(|element| match element {
            ClassElement::MethodDefinition(method)
                if method.kind == MethodDefinitionKind::Constructor =>
            {
                method.value.body.as_deref()
            }
            _ => None,
        });

        for element in &class.body.body {
            let (key, annotation, exempt) = match element {
                ClassElement::PropertyDefinition(p) => (
                    &p.key,
                    &p.type_annotation,
                    p.value.is_some()
                        || p.r#static
                        || p.declare
                        || p.optional
                        || p.definite
                        || p.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition,
                ),
                ClassElement::AccessorProperty(p) => (
                    &p.key,
                    &p.type_annotation,
                    p.value.is_some()
                        || p.r#static
                        || p.definite
                        || p.r#type == AccessorPropertyType::TSAbstractAccessorProperty,
                ),
                _ => continue,
            };
            // Without an annotation the type is `any`, which allows undefined.
            let Some(annotation) = annotation else {
                continue;
            };
            // tsc only checks identifier and private names.
            let name = match key {
                PropertyKey::StaticIdentifier(id) => id.name.to_string(),
                PropertyKey::PrivateIdentifier(id) => format!("#{}", id.name),
                _ => continue,
            };
            if exempt {
                continue;
            }
            let ty = self.resolve_ts_type(&annotation.type_annotation);
            if self.admits_undefined(ty) {
                continue;
            }
            if constructor_body.is_some_and(|body| definitely_assigns(&body.statements, &name)) {
                continue;
            }
            self.errors.push(TypeError::new(
                errors::PROPERTY_NOT_INITIALIZED.format(&[&name]),
                key.span(),
                errors::PROPERTY_NOT_INITIALIZED.code,
            ));
        }
    }

    /// The type a `Promise<T>` resolves to (`T`), or the type itself.
    fn awaited_type(&mut self, ty: TypeId) -> TypeId {
        match self.get_type(ty) {
            Type::TypeRef { name, type_args } if name == "Promise" && type_args.len() == 1 => {
                type_args[0]
            }
            _ => ty,
        }
    }

    /// Whether `undefined` is assignable to the type without an error:
    /// `any`, `unknown`, `void`, `undefined`, or a union containing one.
    fn admits_undefined(&mut self, ty: TypeId) -> bool {
        let resolved = self.resolve_computed_type(ty);
        match self.get_type(resolved).clone() {
            Type::Any | Type::Unknown | Type::Void | Type::Undefined => true,
            Type::Union(members) => members.into_iter().any(|m| self.admits_undefined(m)),
            _ => false,
        }
    }
}

/// How a run of statements can complete, for one property.
enum Completion {
    /// Falls through; whether the property is assigned by then.
    Normal(bool),
    /// Every path returned (with the property assigned) or threw.
    Exits,
}

/// Whether `this.<name>` is assigned on every path through a constructor body.
fn definitely_assigns(statements: &[Statement], name: &str) -> bool {
    match completion(statements, false, name) {
        None => false,
        Some(Completion::Exits) => true,
        Some(Completion::Normal(assigned)) => assigned,
    }
}

/// `None` if some path returns before the property is assigned. Assignments in
/// loops, `try`, and `switch` aren't counted (they may not run), and returns in
/// them aren't tracked, so this is conservative in both directions only there.
fn completion(statements: &[Statement], mut assigned: bool, name: &str) -> Option<Completion> {
    for statement in statements {
        match statement {
            Statement::ExpressionStatement(stmt) => {
                assigned |= assigns_this_property(&stmt.expression, name);
            }
            Statement::ReturnStatement(ret) => {
                if let Some(argument) = &ret.argument {
                    assigned |= assigns_this_property(argument, name);
                }
                return assigned.then_some(Completion::Exits);
            }
            Statement::ThrowStatement(_) => return Some(Completion::Exits),
            Statement::BlockStatement(block) => match completion(&block.body, assigned, name)? {
                Completion::Exits => return Some(Completion::Exits),
                Completion::Normal(a) => assigned = a,
            },
            Statement::IfStatement(stmt) => {
                let then = completion(std::slice::from_ref(&stmt.consequent), assigned, name)?;
                let otherwise = match &stmt.alternate {
                    Some(alt) => completion(std::slice::from_ref(alt), assigned, name)?,
                    None => Completion::Normal(assigned),
                };
                match (then, otherwise) {
                    (Completion::Exits, Completion::Exits) => return Some(Completion::Exits),
                    (Completion::Exits, Completion::Normal(a))
                    | (Completion::Normal(a), Completion::Exits) => assigned = a,
                    (Completion::Normal(a), Completion::Normal(b)) => assigned = a && b,
                }
            }
            _ => {}
        }
    }
    Some(Completion::Normal(assigned))
}

/// Whether evaluating `expr` assigns `this.<name>`: `this.x = ...`, chains
/// (`this.a = this.b = v`), sequences, and destructuring (`({ k: this.x } = o)`).
fn assigns_this_property(expr: &Expression, name: &str) -> bool {
    match expr.without_parentheses() {
        Expression::AssignmentExpression(assign) => {
            target_assigns(&assign.left, name) || assigns_this_property(&assign.right, name)
        }
        Expression::SequenceExpression(seq) => seq
            .expressions
            .iter()
            .any(|e| assigns_this_property(e, name)),
        _ => false,
    }
}

fn target_assigns(target: &AssignmentTarget, name: &str) -> bool {
    match target {
        AssignmentTarget::StaticMemberExpression(member) => {
            matches!(member.object, Expression::ThisExpression(_)) && member.property.name == name
        }
        AssignmentTarget::PrivateFieldExpression(field) => {
            matches!(field.object, Expression::ThisExpression(_))
                && name.strip_prefix('#') == Some(field.field.name.as_str())
        }
        AssignmentTarget::ObjectAssignmentTarget(object) => {
            object.properties.iter().any(|property| match property {
                AssignmentTargetProperty::AssignmentTargetPropertyProperty(p) => {
                    maybe_default_assigns(&p.binding, name)
                }
                AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(_) => false,
            }) || object
                .rest
                .as_ref()
                .is_some_and(|rest| target_assigns(&rest.target, name))
        }
        AssignmentTarget::ArrayAssignmentTarget(array) => {
            array
                .elements
                .iter()
                .flatten()
                .any(|element| maybe_default_assigns(element, name))
                || array
                    .rest
                    .as_ref()
                    .is_some_and(|rest| target_assigns(&rest.target, name))
        }
        _ => false,
    }
}

fn maybe_default_assigns(target: &AssignmentTargetMaybeDefault, name: &str) -> bool {
    match target {
        AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(with_default) => {
            target_assigns(&with_default.binding, name)
        }
        other => other
            .as_assignment_target()
            .is_some_and(|t| target_assigns(t, name)),
    }
}

/// Whether control can reach the end of a statement list (tsc's "end point
/// reachable"). Approximate: conditions are only constant when they are the
/// literal `true`.
fn can_complete_normally(statements: &[Statement]) -> bool {
    statements.iter().all(statement_completes)
}

fn statement_completes(statement: &Statement) -> bool {
    match statement {
        Statement::ReturnStatement(_) | Statement::ThrowStatement(_) => false,
        Statement::BlockStatement(block) => can_complete_normally(&block.body),
        Statement::IfStatement(stmt) => match &stmt.alternate {
            Some(alternate) => {
                statement_completes(&stmt.consequent) || statement_completes(alternate)
            }
            None => true,
        },
        Statement::WhileStatement(stmt) => !is_true(&stmt.test) || breaks_out(&stmt.body),
        Statement::DoWhileStatement(stmt) => !is_true(&stmt.test) || breaks_out(&stmt.body),
        Statement::ForStatement(stmt) => {
            stmt.test.as_ref().is_some_and(|test| !is_true(test)) || breaks_out(&stmt.body)
        }
        Statement::TryStatement(stmt) => {
            if let Some(finalizer) = &stmt.finalizer
                && !can_complete_normally(&finalizer.body)
            {
                return false;
            }
            can_complete_normally(&stmt.block.body)
                || stmt
                    .handler
                    .as_ref()
                    .is_some_and(|handler| can_complete_normally(&handler.body.body))
        }
        Statement::SwitchStatement(stmt) => {
            // Exits only if there's a default, no case breaks out, and the last
            // case doesn't fall off the end.
            let has_default = stmt.cases.iter().any(|case| case.test.is_none());
            let breaks = stmt
                .cases
                .iter()
                .any(|case| case.consequent.iter().any(breaks_out));
            let last_completes = stmt
                .cases
                .last()
                .is_none_or(|case| can_complete_normally(&case.consequent));
            !has_default || breaks || last_completes
        }
        Statement::LabeledStatement(stmt) => statement_completes(&stmt.body),
        _ => true,
    }
}

fn is_true(expr: &Expression) -> bool {
    matches!(expr.without_parentheses(), Expression::BooleanLiteral(lit) if lit.value)
}

/// Whether a `break` in `statement` exits the enclosing loop or switch: any
/// labeled break, or an unlabeled one not inside a nested loop or switch.
fn breaks_out(statement: &Statement) -> bool {
    match statement {
        Statement::BreakStatement(_) => true,
        Statement::BlockStatement(block) => block.body.iter().any(breaks_out),
        Statement::IfStatement(stmt) => {
            breaks_out(&stmt.consequent) || stmt.alternate.as_ref().is_some_and(breaks_out)
        }
        Statement::TryStatement(stmt) => {
            stmt.block.body.iter().any(breaks_out)
                || stmt
                    .handler
                    .as_ref()
                    .is_some_and(|h| h.body.body.iter().any(breaks_out))
                || stmt
                    .finalizer
                    .as_ref()
                    .is_some_and(|f| f.body.iter().any(breaks_out))
        }
        Statement::LabeledStatement(stmt) => breaks_out(&stmt.body),
        // Unlabeled breaks inside these target them, not the outer loop.
        Statement::WhileStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::ForStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_)
        | Statement::SwitchStatement(_) => contains_labeled_break(statement),
        _ => false,
    }
}

/// Conservatively treat any labeled break in a nested loop as leaving the outer one.
fn contains_labeled_break(statement: &Statement) -> bool {
    let body = match statement {
        Statement::WhileStatement(s) => &s.body,
        Statement::DoWhileStatement(s) => &s.body,
        Statement::ForStatement(s) => &s.body,
        Statement::ForInStatement(s) => &s.body,
        Statement::ForOfStatement(s) => &s.body,
        Statement::BreakStatement(s) => return s.label.is_some(),
        Statement::BlockStatement(b) => return b.body.iter().any(contains_labeled_break),
        Statement::IfStatement(s) => {
            return contains_labeled_break(&s.consequent)
                || s.alternate.as_ref().is_some_and(contains_labeled_break);
        }
        Statement::SwitchStatement(s) => {
            return s
                .cases
                .iter()
                .any(|case| case.consequent.iter().any(contains_labeled_break));
        }
        _ => return false,
    };
    contains_labeled_break(body)
}
