//! Expression type inference (synthesis / bottom-up).
//!
//! Given an expression, compute its type by examining its structure.
//! This is the "synthesis" direction of bidirectional type checking.

use oxc_ast::ast::*;

use rustc_hash::FxHashMap;

use crate::types::{Param, Property, Type, TypeId, TypeParam};

use super::Checker;

impl<'a> Checker<'a> {
    /// Infer the type of an expression (synthesis / bottom-up).
    pub fn infer_expression(&mut self, expr: &Expression) -> TypeId {
        match expr {
            // Literals
            Expression::StringLiteral(s) => {
                self.intern(Type::StringLiteral(s.value.to_string()))
            }
            Expression::NumericLiteral(n) => {
                self.intern(Type::NumberLiteral(n.value))
            }
            Expression::BooleanLiteral(b) => {
                self.intern(Type::BooleanLiteral(b.value))
            }
            Expression::NullLiteral(_) => TypeId::NULL,
            Expression::BigIntLiteral(_) => TypeId::NUMBER,

            // Identifiers: check narrowing context first, then symbol table
            Expression::Identifier(ident) => {
                // Check narrowing context first for narrowed types
                if let Some(narrowed_id) = self.narrowing.get_narrowed_id(ident.name.as_str()) {
                    return narrowed_id;
                }
                // Fall back to symbol table
                self.symbols
                    .lookup(ident.name.as_str())
                    .map(|s| s.ty)
                    .unwrap_or(TypeId::ANY)
            }

            // Compound expressions
            Expression::ArrayExpression(arr) => self.infer_array_literal(arr),
            Expression::ObjectExpression(obj) => self.infer_object_literal(obj),
            Expression::ArrowFunctionExpression(arrow) => self.infer_arrow_function(arrow),
            Expression::FunctionExpression(func) => self.infer_function_expression(func),
            Expression::CallExpression(call) => self.infer_call_expression(call),

            // Member access
            Expression::StaticMemberExpression(member) => {
                let object_type_id = self.infer_expression(&member.object);
                self.get_property_type(object_type_id, member.property.name.as_str())
            }
            Expression::ComputedMemberExpression(member) => {
                self.infer_computed_member_expression(member)
            }

            // Operators
            Expression::BinaryExpression(binary) => self.infer_binary_expression(binary),
            Expression::UnaryExpression(unary) => self.infer_unary_expression(unary),

            // Conditional (ternary)
            Expression::ConditionalExpression(cond) => {
                let consequent_id = self.infer_expression(&cond.consequent);
                let alternate_id = self.infer_expression(&cond.alternate);
                self.union_types(consequent_id, alternate_id)
            }

            // Logical operators
            Expression::LogicalExpression(logical) => {
                let left_id = self.infer_expression(&logical.left);
                let right_id = self.infer_expression(&logical.right);
                match logical.operator {
                    LogicalOperator::And => right_id, // a && b returns b if a is truthy
                    LogicalOperator::Or => self.union_types(left_id, right_id),
                    LogicalOperator::Coalesce => self.union_types(left_id, right_id),
                }
            }

            // Assignment returns the assigned value's type
            Expression::AssignmentExpression(assign) => self.infer_expression(&assign.right),

            // Template literals
            Expression::TemplateLiteral(_) => TypeId::STRING,
            Expression::TaggedTemplateExpression(_) => TypeId::ANY, // Depends on tag function

            // New expression returns the class instance type with type arguments
            Expression::NewExpression(new_expr) => {
                if let Expression::Identifier(ident) = &new_expr.callee {
                    // Capture explicit type arguments if provided
                    let type_args: Vec<TypeId> = new_expr
                        .type_arguments
                        .as_ref()
                        .map(|args| {
                            args.params
                                .iter()
                                .map(|t| self.resolve_ts_type(t))
                                .collect()
                        })
                        .unwrap_or_default();
                    self.intern(Type::TypeRef {
                        name: ident.name.to_string(),
                        type_args,
                    })
                } else {
                    TypeId::ANY
                }
            }

            // Await unwraps Promise
            Expression::AwaitExpression(await_expr) => {
                let inner_id = self.infer_expression(&await_expr.argument);
                self.unwrap_promise(inner_id)
            }

            // Other
            Expression::YieldExpression(_) => TypeId::ANY,
            Expression::ThisExpression(_) => {
                // Return the current class's instance type
                if let Some(class_name) = &self.current_class {
                    self.symbols
                        .lookup_type(class_name)
                        .map(|s| s.ty)
                        .unwrap_or(TypeId::ANY)
                } else {
                    TypeId::ANY
                }
            }
            Expression::ParenthesizedExpression(paren) => self.infer_expression(&paren.expression),

            // Sequence expression returns last
            Expression::SequenceExpression(seq) => seq
                .expressions
                .last()
                .map(|e| self.infer_expression(e))
                .unwrap_or(TypeId::UNDEFINED),

            _ => TypeId::ANY,
        }
    }

    /// Infer type of array literal.
    ///
    /// Empty arrays get `never[]`, otherwise union of all element types.
    pub(super) fn infer_array_literal(&mut self, arr: &ArrayExpression) -> TypeId {
        if arr.elements.is_empty() {
            return self.arena_mut().array(TypeId::NEVER);
        }

        let mut element_type_ids: Vec<TypeId> = Vec::new();

        for elem in &arr.elements {
            match elem {
                ArrayExpressionElement::SpreadElement(spread) => {
                    let spread_type_id = self.infer_expression(&spread.argument);
                    let spread_type = self.get_type(spread_type_id).clone();
                    if let Type::Array(inner_id) = spread_type {
                        element_type_ids.push(inner_id);
                    } else {
                        element_type_ids.push(spread_type_id);
                    }
                }
                ArrayExpressionElement::Elision(_) => {
                    element_type_ids.push(TypeId::UNDEFINED);
                }
                _ => {
                    if let Some(expr) = elem.as_expression() {
                        let ty_id = self.infer_expression(expr);
                        // Widen literal types in arrays
                        element_type_ids.push(self.widen_type(ty_id));
                    }
                }
            }
        }

        // Deduplicate and create union if multiple types
        let unified_id = self.unify_types(element_type_ids);
        self.arena_mut().array(unified_id)
    }

    /// Infer type of object literal.
    pub(super) fn infer_object_literal(&mut self, obj: &ObjectExpression) -> TypeId {
        let mut properties = Vec::new();

        for prop in &obj.properties {
            match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    // Handle method shorthand: { foo() {} }
                    if p.method {
                        if let PropertyKey::StaticIdentifier(ident) = &p.key {
                            let ty_id = self.infer_expression(&p.value);
                            properties.push(Property::new(ident.name.to_string(), ty_id));
                        }
                        continue;
                    }

                    let name = match &p.key {
                        PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                        PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                        _ if p.computed => {
                            if let PropertyKey::StringLiteral(s) = &p.key {
                                Some(s.value.to_string())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };

                    if let Some(name) = name {
                        let ty_id = self.infer_expression(&p.value);
                        // Widen literal types in object properties
                        let widened_id = self.widen_type(ty_id);
                        properties.push(Property::new(name, widened_id));
                    }
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    let spread_type_id = self.infer_expression(&spread.argument);
                    let spread_type = self.get_type(spread_type_id).clone();
                    if let Type::Object {
                        properties: spread_props,
                        ..
                    } = spread_type
                    {
                        properties.extend(spread_props);
                    }
                }
            }
        }

        self.arena_mut().object(properties)
    }

    /// Infer type of arrow function.
    fn infer_arrow_function(&mut self, arrow: &ArrowFunctionExpression) -> TypeId {
        let mut params: Vec<Param> = arrow
            .params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern {
                    BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty_id = p
                    .type_annotation
                    .as_ref()
                    .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                    .unwrap_or(TypeId::ANY);
                let mut param = Param::new(name, ty_id);
                if p.optional {
                    param = param.optional();
                }
                param
            })
            .collect();

        // Handle rest parameter
        if let Some(rest_param) = &arrow.params.rest {
            let name = match &rest_param.rest.argument {
                BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                _ => "args".to_string(),
            };
            let ty_id = rest_param
                .type_annotation
                .as_ref()
                .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                .unwrap_or_else(|| self.arena_mut().array(TypeId::ANY));
            params.push(Param::new(name, ty_id).rest());
        }

        let return_type_id = if let Some(ann) = &arrow.return_type {
            self.resolve_ts_type(&ann.type_annotation)
        } else {
            // Infer from body
            if arrow.expression {
                // Expression body: () => expr
                if let Some(Statement::ExpressionStatement(expr)) = arrow.body.statements.first() {
                    self.infer_expression(&expr.expression)
                } else {
                    TypeId::UNDEFINED
                }
            } else {
                // Block body: infer from returns
                let returns = self.collect_return_types(&arrow.body);
                if returns.is_empty() {
                    TypeId::VOID
                } else {
                    let type_ids: Vec<TypeId> = returns.into_iter().map(|(t, _)| t).collect();
                    self.unify_types(type_ids)
                }
            }
        };

        self.arena_mut().function(params, return_type_id)
    }

    /// Infer type of function expression.
    fn infer_function_expression(&mut self, func: &Function) -> TypeId {
        let mut params: Vec<Param> = func
            .params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern {
                    BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty_id = p
                    .type_annotation
                    .as_ref()
                    .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                    .unwrap_or(TypeId::ANY);
                let mut param = Param::new(name, ty_id);
                if p.optional {
                    param = param.optional();
                }
                param
            })
            .collect();

        // Handle rest parameter
        if let Some(rest_param) = &func.params.rest {
            let name = match &rest_param.rest.argument {
                BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                _ => "args".to_string(),
            };
            let ty_id = rest_param
                .type_annotation
                .as_ref()
                .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                .unwrap_or_else(|| self.arena_mut().array(TypeId::ANY));
            params.push(Param::new(name, ty_id).rest());
        }

        let return_type_id = if let Some(ann) = &func.return_type {
            self.resolve_ts_type(&ann.type_annotation)
        } else if let Some(body) = &func.body {
            let returns = self.collect_return_types(body);
            if returns.is_empty() {
                TypeId::VOID
            } else {
                let type_ids: Vec<TypeId> = returns.into_iter().map(|(t, _)| t).collect();
                self.unify_types(type_ids)
            }
        } else {
            TypeId::VOID
        };

        self.arena_mut().function(params, return_type_id)
    }

    /// Infer type of computed member access (obj[expr]).
    fn infer_computed_member_expression(&mut self, member: &ComputedMemberExpression) -> TypeId {
        let object_type_id = self.infer_expression(&member.object);
        let index_type_id = self.infer_expression(&member.expression);

        // Resolve TypeRef first
        let object_type = self.get_type(object_type_id).clone();
        let resolved_type_id = if let Type::TypeRef { name, .. } = &object_type {
            self.symbols
                .lookup_type(name)
                .map(|s| s.ty)
                .unwrap_or(object_type_id)
        } else {
            object_type_id
        };

        let resolved_type = self.get_type(resolved_type_id).clone();
        let index_type = self.get_type(index_type_id).clone();

        // If indexing with string literal, treat as property access
        if let Type::StringLiteral(prop_name) = &index_type {
            return self.get_property_type(resolved_type_id, prop_name);
        }

        // Array indexing
        if let Type::Array(element_type_id) = resolved_type {
            return element_type_id;
        }

        // Tuple indexing with number literal
        if let Type::Tuple(type_ids) = resolved_type {
            if let Type::NumberLiteral(n) = index_type {
                let idx = n as usize;
                if idx < type_ids.len() {
                    return type_ids[idx];
                }
            }
            // Unknown index - return union of all tuple types
            return self.unify_types(type_ids);
        }

        // Object with index signature
        if let Type::Object {
            index_signature, ..
        } = resolved_type
        {
            if let Some(idx_sig) = index_signature {
                let key_ty = self.get_type(idx_sig.key_type);
                let key_matches = match (&index_type, key_ty) {
                    (Type::String, Type::String) => true,
                    (Type::StringLiteral(_), Type::String) => true,
                    (Type::Number, Type::Number) => true,
                    (Type::NumberLiteral(_), Type::Number) => true,
                    (Type::Number, Type::String) => true,
                    (Type::NumberLiteral(_), Type::String) => true,
                    _ => false,
                };
                if key_matches {
                    return idx_sig.value_type;
                }
            }
        }

        TypeId::ANY
    }

    /// Get property type from an object type.
    pub(super) fn get_property_type(&mut self, object_type_id: TypeId, prop_name: &str) -> TypeId {
        // First check for primitive types and look up their corresponding interfaces
        let object_type = self.get_type(object_type_id);
        let interface_name = match object_type {
            Type::String | Type::StringLiteral(_) => Some("String"),
            Type::Number | Type::NumberLiteral(_) => Some("Number"),
            Type::Boolean | Type::BooleanLiteral(_) => Some("Boolean"),
            Type::Array(_) => Some("Array"),
            _ => None,
        };

        // If this is a primitive, look up its interface type
        if let Some(iface_name) = interface_name {
            if let Some(symbol) = self.symbols.lookup_type(iface_name) {
                // Recursively get property type from the interface
                return self.get_property_type(symbol.ty, prop_name);
            }
        }

        // Convert primitives to their apparent types
        let apparent_type_id = self.get_apparent_type(object_type_id);

        // Resolve TypeRef to its underlying type, applying type argument substitution
        let apparent_type = self.get_type(apparent_type_id).clone();
        let (resolved_type_id, type_args) = if let Type::TypeRef { name, type_args } = &apparent_type {
            let base_id = self.symbols
                .lookup_type(name)
                .map(|s| s.ty)
                .unwrap_or(apparent_type_id);
            (base_id, type_args.clone())
        } else {
            (apparent_type_id, vec![])
        };

        // For TypeParameter with a constraint, use the constraint for property lookup
        let resolved_type = self.get_type(resolved_type_id).clone();
        let after_param_type_id = if let Type::TypeParameter {
            constraint: Some(constraint_id),
            ..
        } = resolved_type
        {
            constraint_id
        } else {
            resolved_type_id
        };

        // If the type is still a TypeRef (e.g., constraint is HasLength interface), resolve it
        let after_param_type = self.get_type(after_param_type_id).clone();
        let final_type_id = if let Type::TypeRef { name, .. } = &after_param_type {
            self.symbols
                .lookup_type(name)
                .map(|s| s.ty)
                .unwrap_or(after_param_type_id)
        } else {
            after_param_type_id
        };

        let final_type = self.get_type(final_type_id).clone();
        match final_type {
            Type::Object {
                properties,
                index_signature,
                extends,
                type_params,
            } => {
                // Resolve all properties including those from extended interfaces
                let all_props = self.resolve_object_properties(&properties, &extends);
                if let Some(prop) = all_props.iter().find(|p| p.name == prop_name) {
                    // Apply type argument substitution if we have type params and args
                    let prop_ty = if !type_params.is_empty() && !type_args.is_empty() {
                        self.substitute_type(prop.ty, &type_params, &type_args)
                    } else {
                        prop.ty
                    };
                    return prop_ty;
                }
                if let Some(idx_sig) = index_signature {
                    let key_ty = self.get_type(idx_sig.key_type);
                    if matches!(key_ty, Type::String) {
                        return idx_sig.value_type;
                    }
                }
                TypeId::ANY
            }
            Type::ClassConstructor { static_members, .. } => {
                if let Some(prop) = static_members.iter().find(|p| p.name == prop_name) {
                    return prop.ty;
                }
                TypeId::ANY
            }
            Type::Union(type_ids) => {
                // For unions, the property must exist in all members
                // The result is the union of all property types
                let mut prop_types: Vec<TypeId> = Vec::new();
                for member_id in type_ids {
                    let prop_ty = self.get_property_type(member_id, prop_name);
                    if prop_ty == TypeId::ANY {
                        // Property not found in this member - property doesn't exist in union
                        return TypeId::ANY;
                    }
                    prop_types.push(prop_ty);
                }
                if prop_types.is_empty() {
                    return TypeId::ANY;
                }
                if prop_types.len() == 1 {
                    return prop_types[0];
                }
                // Create union of all property types
                self.symbols.arena.union(prop_types)
            }
            Type::Intersection(type_ids) => {
                // For intersections, check each member for the property
                // Return the first property type found
                for member_id in type_ids {
                    let prop_ty = self.get_property_type(member_id, prop_name);
                    if prop_ty != TypeId::ANY {
                        return prop_ty;
                    }
                }
                TypeId::ANY
            }
            Type::Any | Type::Unknown => TypeId::ANY,
            _ => TypeId::ANY,
        }
    }

    /// Infer type of binary expression.
    fn infer_binary_expression(&mut self, binary: &BinaryExpression) -> TypeId {
        let left_id = self.infer_expression(&binary.left);
        let right_id = self.infer_expression(&binary.right);

        match binary.operator {
            // Arithmetic (except +)
            BinaryOperator::Subtraction
            | BinaryOperator::Multiplication
            | BinaryOperator::Division
            | BinaryOperator::Remainder
            | BinaryOperator::Exponential => TypeId::NUMBER,

            // Addition - number + number = number, string + any = string
            BinaryOperator::Addition => {
                if self.is_string_like(left_id) || self.is_string_like(right_id) {
                    TypeId::STRING
                } else {
                    TypeId::NUMBER
                }
            }

            // Comparison
            BinaryOperator::LessThan
            | BinaryOperator::LessEqualThan
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterEqualThan
            | BinaryOperator::Equality
            | BinaryOperator::Inequality
            | BinaryOperator::StrictEquality
            | BinaryOperator::StrictInequality => TypeId::BOOLEAN,

            // Bitwise
            BinaryOperator::BitwiseAnd
            | BinaryOperator::BitwiseOR
            | BinaryOperator::BitwiseXOR
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight
            | BinaryOperator::ShiftRightZeroFill => TypeId::NUMBER,

            // instanceof / in
            BinaryOperator::Instanceof | BinaryOperator::In => TypeId::BOOLEAN,
        }
    }

    /// Infer type of unary expression.
    fn infer_unary_expression(&mut self, unary: &UnaryExpression) -> TypeId {
        match unary.operator {
            UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus | UnaryOperator::BitwiseNot => {
                TypeId::NUMBER
            }
            UnaryOperator::LogicalNot => TypeId::BOOLEAN,
            UnaryOperator::Typeof => TypeId::STRING,
            UnaryOperator::Void => TypeId::UNDEFINED,
            UnaryOperator::Delete => TypeId::BOOLEAN,
        }
    }

    /// Check if a type is string-like.
    fn is_string_like(&self, ty_id: TypeId) -> bool {
        let ty = self.get_type(ty_id);
        matches!(ty, Type::String | Type::StringLiteral(_))
    }

    /// Convert primitive types to their interface equivalents for method resolution.
    pub(super) fn get_apparent_type(&self, ty_id: TypeId) -> TypeId {
        let ty = self.get_type(ty_id);
        match ty {
            Type::String | Type::StringLiteral(_) => {
                // Return TypeRef to String interface - but we can't intern here (&self)
                // For now, return the type_id itself - property lookup will handle this
                ty_id
            }
            Type::Number | Type::NumberLiteral(_) => ty_id,
            Type::Boolean | Type::BooleanLiteral(_) => ty_id,
            Type::Array(_) => ty_id,
            _ => ty_id,
        }
    }

    /// Unwrap Promise<T> to T.
    fn unwrap_promise(&self, ty_id: TypeId) -> TypeId {
        let ty = self.get_type(ty_id);
        if let Type::TypeRef { name, type_args } = ty {
            if name == "Promise" && !type_args.is_empty() {
                return type_args[0];
            }
        }
        ty_id
    }

    /// Infer the return type of a function call, handling generic type inference.
    fn infer_call_expression(&mut self, call: &CallExpression) -> TypeId {
        let callee_type_id = self.infer_expression(&call.callee);
        let callee_type = self.get_type(callee_type_id).clone();

        if let Type::Function {
            params,
            return_type,
            type_params,
            ..
        } = callee_type
        {
            // If no type parameters, just return the return type
            if type_params.is_empty() {
                return return_type;
            }

            // Collect argument types
            let arg_type_ids: Vec<TypeId> = call
                .arguments
                .iter()
                .filter_map(|arg| arg.as_expression())
                .map(|expr| self.infer_expression(expr))
                .collect();

            // Get explicit type arguments from the call if present
            let explicit_type_args: Vec<TypeId> = call
                .type_arguments
                .as_ref()
                .map(|args| {
                    args.params
                        .iter()
                        .map(|t| self.resolve_ts_type(t))
                        .collect()
                })
                .unwrap_or_default();

            // Build substitution map
            let substitutions = if !explicit_type_args.is_empty() {
                // Use explicit type arguments
                self.build_substitution_map(&type_params, &explicit_type_args)
            } else {
                // Infer type arguments from argument types
                self.infer_type_args_from_call(&type_params, &params, &arg_type_ids)
            };

            // Substitute type parameters in the return type
            if substitutions.is_empty() {
                return_type
            } else {
                self.substitute_type_params(return_type, &substitutions)
            }
        } else {
            TypeId::ANY
        }
    }

    /// Check if a type has a specific property.
    pub(super) fn has_property(&mut self, ty_id: TypeId, prop_name: &str) -> bool {
        let prop_type_id = self.get_property_type(ty_id, prop_name);
        prop_type_id != TypeId::ANY
    }

    /// Substitute type parameters with type arguments.
    /// E.g., T with type_params=[T] and type_args=[number] => number
    fn substitute_type(
        &mut self,
        ty_id: TypeId,
        type_params: &[TypeParam],
        type_args: &[TypeId],
    ) -> TypeId {
        // Build substitution map: param name -> type argument
        let substitutions: FxHashMap<String, TypeId> = type_params
            .iter()
            .zip(type_args.iter())
            .map(|(param, &arg)| (param.name.clone(), arg))
            .collect();

        if substitutions.is_empty() {
            return ty_id;
        }

        self.substitute_type_params(ty_id, &substitutions)
    }
}
