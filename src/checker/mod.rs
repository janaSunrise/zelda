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

use flow::{NarrowingContext, apply_guard_with_resolver, extract_type_guard};

use oxc_ast::ast::*;
use oxc_span::Span;
use serde::Serialize;

use crate::errors;
use crate::symbols::SymbolTable;
use crate::types::Type;

/// Maximum Levenshtein distance for "Did you mean?" suggestions.
const MAX_SUGGESTION_DISTANCE: usize = 3;

/// Threshold for "and X more" format (TSC uses 5+).
const MANY_MISSING_PROPS_THRESHOLD: usize = 5;

/// Properties to show before "and X more".
const SHOWN_MISSING_PROPS_COUNT: usize = 4;

/// Compute Levenshtein (edit) distance between two strings.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    // Optimization: if one string is empty, distance is the length of the other
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    // Use two rows instead of a full matrix to save memory
    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for (i, a_char) in a.chars().enumerate() {
        curr_row[0] = i + 1;

        for (j, b_char) in b.chars().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1) // deletion
                .min(curr_row[j] + 1) // insertion
                .min(prev_row[j] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Find most similar name from candidates for "Did you mean?" suggestions.
fn find_similar_name<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .filter_map(|c| {
            let dist = levenshtein_distance(name, c);
            (dist > 0 && dist <= MAX_SUGGESTION_DISTANCE).then_some((c, dist))
        })
        .min_by_key(|(_, dist)| *dist)
        .map(|(c, _)| c)
}

/// Severity level of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Errors prevent successful compilation.
    Error,
    /// Warnings indicate potential issues but don't block compilation.
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct TypeError {
    pub message: String,
    #[serde(serialize_with = "serialize_span")]
    pub span: Span,
    pub code: u32,
    pub severity: Severity,
    /// Related spans that provide additional context.
    pub related: Vec<RelatedSpan>,
}

/// A related span provides additional context for an error.
#[derive(Debug, Clone, Serialize)]
pub struct RelatedSpan {
    pub message: String,
    #[serde(serialize_with = "serialize_span")]
    pub span: Span,
}

/// Custom serializer for oxc_span::Span since it doesn't implement Serialize.
fn serialize_span<S>(span: &Span, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeStruct;
    let mut state = serializer.serialize_struct("Span", 2)?;
    state.serialize_field("start", &span.start)?;
    state.serialize_field("end", &span.end)?;
    state.end()
}

impl TypeError {
    pub fn new(message: impl Into<String>, span: Span, code: u32) -> Self {
        Self {
            message: message.into(),
            span,
            code,
            severity: Severity::Error,
            related: Vec::new(),
        }
    }

    /// Create a warning instead of an error.
    pub fn warning(message: impl Into<String>, span: Span, code: u32) -> Self {
        Self {
            message: message.into(),
            span,
            code,
            severity: Severity::Warning,
            related: Vec::new(),
        }
    }

    /// Add a related span to this error for additional context.
    pub fn with_related(mut self, message: impl Into<String>, span: Span) -> Self {
        self.related.push(RelatedSpan {
            message: message.into(),
            span,
        });
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn is_warning(&self) -> bool {
        self.severity == Severity::Warning
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

    /// Create a property-not-found error with a "Did you mean?" suggestion.
    ///
    /// If a similar property name is found among the available properties,
    /// uses TS2551 with a suggestion. Otherwise falls back to TS2339.
    pub fn property_not_found_with_suggestion(
        prop: &str,
        ty: &Type,
        available_props: &[String],
        span: Span,
    ) -> Self {
        if let Some(suggestion) =
            find_similar_name(prop, available_props.iter().map(|s| s.as_str()))
        {
            Self::new(
                errors::PROPERTY_NOT_EXIST_SUGGESTION.format(&[prop, &ty.to_string(), suggestion]),
                span,
                errors::PROPERTY_NOT_EXIST_SUGGESTION.code,
            )
        } else {
            Self::property_not_found(prop, ty, span)
        }
    }

    pub fn static_member_suggestion(prop: &str, ty: &Type, class_name: &str, span: Span) -> Self {
        Self::new(
            errors::STATIC_MEMBER_SUGGESTION.format(&[prop, &ty.to_string(), class_name]),
            span,
            errors::STATIC_MEMBER_SUGGESTION.code,
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

    pub fn missing_properties(
        missing_props: &[String],
        source: &Type,
        target: &Type,
        span: Span,
    ) -> Self {
        // TSC uses TS2740 when there are 5+ missing properties (shows first 4 + "and X more")
        // TSC uses TS2739 when there are 2-4 missing properties (shows all)
        if missing_props.len() >= MANY_MISSING_PROPS_THRESHOLD {
            // TS2740: Show first N properties + "and X more"
            let shown_props = missing_props[..SHOWN_MISSING_PROPS_COUNT].join(", ");
            let remaining = missing_props.len() - SHOWN_MISSING_PROPS_COUNT;
            Self::new(
                errors::MANY_PROPERTIES_MISSING.format(&[
                    &source.to_string(),
                    &target.to_string(),
                    &shown_props,
                    &remaining.to_string(),
                ]),
                span,
                errors::MANY_PROPERTIES_MISSING.code,
            )
        } else {
            // TS2739: Type 'X' is missing the following properties from type 'Y': a, b, c
            let props_list = missing_props.join(", ");
            Self::new(
                errors::MULTIPLE_PROPERTIES_MISSING.format(&[
                    &source.to_string(),
                    &target.to_string(),
                    &props_list,
                ]),
                span,
                errors::MULTIPLE_PROPERTIES_MISSING.code,
            )
        }
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
            errors::CONSTRAINT_NOT_SATISFIED
                .format(&[&type_arg.to_string(), &constraint.to_string()]),
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
    /// Variables that have been definitely assigned (initialized). Uses FxHashSet for faster lookups.
    assigned_vars: rustc_hash::FxHashSet<String>,
    /// Variables declared without initializer that need assignment checking. Uses FxHashSet for faster lookups.
    uninitialized_vars: rustc_hash::FxHashSet<String>,
}

impl<'a> Checker<'a> {
    pub fn new(symbols: &'a mut SymbolTable) -> Self {
        Self {
            symbols,
            errors: Vec::new(),
            narrowing: NarrowingContext::new(),
            current_class: None,
            assigned_vars: rustc_hash::FxHashSet::default(),
            uninitialized_vars: rustc_hash::FxHashSet::default(),
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

                // Check for assertion function calls and apply narrowing
                if let Expression::CallExpression(call) = &expr.expression {
                    self.apply_assertion_narrowing(call);
                }
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
                if let Some(extracted) = &guard
                    && let Some(original_type) = self.lookup_variable_type(&extracted.variable) {
                        // Create resolver closure that can look up TypeRefs
                        let resolver =
                            |name: &str| self.symbols.lookup_type(name).map(|s| s.ty.clone());
                        let narrowed = apply_guard_with_resolver(
                            &original_type,
                            &extracted.guard,
                            extracted.negated,
                            resolver,
                        );
                        self.narrowing.narrow(extracted.variable.clone(), narrowed);
                    }

                self.check_statement(&if_stmt.consequent);
                self.narrowing.clear(); // Reset after true branch

                if let Some(alt) = &if_stmt.alternate {
                    // Apply negated guard for else branch
                    if let Some(extracted) = &guard
                        && let Some(original_type) = self.lookup_variable_type(&extracted.variable)
                        {
                            let resolver =
                                |name: &str| self.symbols.lookup_type(name).map(|s| s.ty.clone());
                            let narrowed = apply_guard_with_resolver(
                                &original_type,
                                &extracted.guard,
                                !extracted.negated,
                                resolver,
                            );
                            self.narrowing.narrow(extracted.variable.clone(), narrowed);
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
                if let Some(init) = &for_stmt.init
                    && let ForStatementInit::VariableDeclaration(decl) = init {
                        self.check_variable_declaration(decl);
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

    /// After calling an assertion function like `assertIsString(x)`, narrow x to string.
    fn apply_assertion_narrowing(&mut self, call: &CallExpression) {
        let callee_type = self.infer_expression(&call.callee);

        if let Type::Function {
            type_predicate: Some(predicate),
            params,
            ..
        } = &callee_type
        {
            if !predicate.asserts {
                return;
            }

            // Match the predicate's parameter name to find which argument gets narrowed.
            // For example, if the predicate says "asserts val is string" and val is the
            // first parameter, we narrow the first argument passed to the call.
            let param_index = params
                .iter()
                .position(|p| p.name == predicate.parameter_name);

            if let Some(idx) = param_index
                && let Some(arg) = call.arguments.get(idx)
                    && let Some(Expression::Identifier(ident)) = arg.as_expression()
                        && let Some(narrowed_type) = &predicate.type_annotation {
                            self.narrowing
                                .narrow(ident.name.to_string(), (**narrowed_type).clone());
                        }
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

            // Check identifiers for used-before-assigned
            Expression::Identifier(ident) => {
                let name = ident.name.as_str();
                // If the variable was declared without an initializer and hasn't been assigned yet
                if self.uninitialized_vars.contains(name) && !self.assigned_vars.contains(name) {
                    self.errors.push(TypeError::new(
                        errors::USED_BEFORE_ASSIGNED.format(&[name]),
                        ident.span,
                        errors::USED_BEFORE_ASSIGNED.code,
                    ));
                }
            }

            // Literals don't need checking
            _ => {}
        }
    }

    /// Check an assignment expression for type compatibility.
    fn check_assignment_expression(&mut self, assign: &AssignmentExpression) {
        self.check_expression(&assign.right);

        // Mark identifier targets as assigned (for definite assignment analysis)
        if let AssignmentTarget::AssignmentTargetIdentifier(ident) = &assign.left {
            self.assigned_vars.insert(ident.name.to_string());
        }

        // Check property existence for member expression targets
        if let AssignmentTarget::StaticMemberExpression(member) = &assign.left {
            let object_type = self.infer_expression(&member.object);
            let prop_name = member.property.name.as_str();

            // Skip checking for `any` and `unknown` types
            if !matches!(object_type, Type::Any | Type::Unknown)
                && !self.has_property(&object_type, prop_name) {
                    let available_props = self.get_available_properties(&object_type);
                    self.errors
                        .push(TypeError::property_not_found_with_suggestion(
                            prop_name,
                            &object_type,
                            &available_props,
                            member.span,
                        ));
                }
        }

        // Get the target type
        let target_type = match &assign.left {
            AssignmentTarget::AssignmentTargetIdentifier(ident) => self
                .symbols
                .lookup(ident.name.as_str())
                .map(|s| s.ty.clone()),
            AssignmentTarget::StaticMemberExpression(member) => {
                // Get property type from the object type
                let object_type = self.infer_expression(&member.object);
                let prop_name = member.property.name.as_str();
                let prop_type = self.get_property_type(&object_type, prop_name);
                // Only return Some if it's not Any (means we found a real type)
                if matches!(prop_type, Type::Any) {
                    None
                } else {
                    Some(prop_type)
                }
            }
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
            // Check if this might be a static member accessed on an instance (TS2576)
            // If the object is a TypeRef (class instance), check if the class has this as a static member
            if let Type::TypeRef {
                name: class_name, ..
            } = &object_type
            {
                // Look up the class constructor in the value namespace
                if let Some(symbol) = self.symbols.lookup(class_name)
                    && let Type::ClassConstructor { static_members, .. } = &symbol.ty {
                        // Check if the property exists as a static member
                        if static_members.iter().any(|p| p.name == prop_name) {
                            self.errors.push(TypeError::static_member_suggestion(
                                prop_name,
                                &object_type,
                                class_name,
                                member.span,
                            ));
                            return;
                        }
                    }
            }

            // Collect available properties for "Did you mean?" suggestions
            let available_props = self.get_available_properties(&object_type);
            self.errors
                .push(TypeError::property_not_found_with_suggestion(
                    prop_name,
                    &object_type,
                    &available_props,
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
        if let Type::StringLiteral(prop_name) = &index_type
            && !self.has_property(&object_type, prop_name) {
                let available_props = self.get_available_properties(&object_type);
                self.errors
                    .push(TypeError::property_not_found_with_suggestion(
                        prop_name,
                        &object_type,
                        &available_props,
                        member.span,
                    ));
            }

        // Array/tuple indexing with number is always valid
        // Index signatures are checked separately
    }

    /// Collect all available property names from a type.
    ///
    /// Used for "Did you mean?" suggestions when a property is not found.
    fn get_available_properties(&self, ty: &Type) -> Vec<String> {
        // Convert primitives to their apparent types
        let apparent_type = self.get_apparent_type(ty);

        // Resolve TypeRef to its underlying type
        let resolved = if let Type::TypeRef { name, type_args } = &apparent_type {
            self.resolve_type_ref_with_args(name, type_args)
        } else {
            None
        };
        let ty = resolved.as_ref().unwrap_or(&apparent_type);

        // For TypeParameter with a constraint, use the constraint
        let resolved_constraint = if let Type::TypeParameter {
            constraint: Some(constraint),
            ..
        } = ty
        {
            let resolved_constraint = if let Type::TypeRef { name, type_args } = constraint.as_ref()
            {
                self.resolve_type_ref_with_args(name, type_args)
                    .unwrap_or_else(|| (**constraint).clone())
            } else {
                (**constraint).clone()
            };
            Some(resolved_constraint)
        } else {
            None
        };
        let ty = resolved_constraint.as_ref().unwrap_or(ty);

        match ty {
            Type::Object {
                properties,
                extends,
                ..
            } => {
                let all_props = self.resolve_object_properties(properties, extends);
                all_props.iter().map(|p| p.name.clone()).collect()
            }
            Type::Union(types) => {
                // For unions, collect properties that exist on ALL members
                if types.is_empty() {
                    return vec![];
                }
                let first_props: rustc_hash::FxHashSet<String> = self
                    .get_available_properties(&types[0])
                    .into_iter()
                    .collect();
                types[1..]
                    .iter()
                    .fold(first_props, |acc, t| {
                        let props: rustc_hash::FxHashSet<String> =
                            self.get_available_properties(t).into_iter().collect();
                        acc.intersection(&props).cloned().collect()
                    })
                    .into_iter()
                    .collect()
            }
            Type::Intersection(types) => {
                // For intersections, collect properties from ALL members
                types
                    .iter()
                    .flat_map(|t| self.get_available_properties(t))
                    .collect::<rustc_hash::FxHashSet<_>>()
                    .into_iter()
                    .collect()
            }
            Type::ClassConstructor { static_members, .. } => {
                static_members.iter().map(|p| p.name.clone()).collect()
            }
            _ => vec![],
        }
    }

    /// Check if a type has a property with the given name.
    ///
    /// Uses the apparent type pattern to resolve primitive properties from lib.d.ts interfaces.
    fn has_property(&self, ty: &Type, prop_name: &str) -> bool {
        // Convert primitives to their apparent types (e.g., string -> String interface)
        let apparent_type = self.get_apparent_type(ty);

        // Resolve TypeRef to its underlying type
        let resolved = if let Type::TypeRef { name, type_args } = &apparent_type {
            self.resolve_type_ref_with_args(name, type_args)
        } else {
            None
        };
        let ty = resolved.as_ref().unwrap_or(&apparent_type);

        // For TypeParameter with a constraint, use the constraint
        let resolved_constraint = if let Type::TypeParameter {
            constraint: Some(constraint),
            ..
        } = ty
        {
            // If the constraint is a TypeRef, resolve it
            let resolved_constraint = if let Type::TypeRef { name, type_args } = constraint.as_ref()
            {
                self.resolve_type_ref_with_args(name, type_args)
                    .unwrap_or_else(|| (**constraint).clone())
            } else {
                (**constraint).clone()
            };
            Some(resolved_constraint)
        } else {
            None
        };
        let ty = resolved_constraint.as_ref().unwrap_or(ty);

        match ty {
            Type::Object {
                properties,
                index_signature,
                extends,
                ..
            } => {
                let all_props = self.resolve_object_properties(properties, extends);
                if all_props.iter().any(|p| p.name == prop_name) {
                    return true;
                }
                if index_signature.is_some() {
                    return true;
                }
                false
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
