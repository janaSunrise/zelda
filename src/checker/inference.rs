//! Expression type inference (synthesis / bottom-up).
//!
//! Given an expression, compute its type by examining its structure.
//! This is the "synthesis" direction of bidirectional type checking.

use oxc_ast::ast::*;

use crate::types::{Param, Property, Type};

use super::Checker;

impl<'a> Checker<'a> {
    /// Infer the type of an expression (synthesis / bottom-up).
    pub fn infer_expression(&self, expr: &Expression) -> Type {
        match expr {
            // Literals
            Expression::StringLiteral(s) => Type::StringLiteral(s.value.to_string()),
            Expression::NumericLiteral(n) => Type::NumberLiteral(n.value),
            Expression::BooleanLiteral(b) => Type::BooleanLiteral(b.value),
            Expression::NullLiteral(_) => Type::Null,
            Expression::BigIntLiteral(_) => Type::Number,

            // Identifiers: check narrowing context first, then symbol table
            Expression::Identifier(ident) => {
                // Check narrowing context first for narrowed types
                if let Some(narrowed) = self.narrowing.get_narrowed(ident.name.as_str()) {
                    return narrowed.clone();
                }
                // Fall back to symbol table
                self.symbols
                    .lookup(ident.name.as_str())
                    .map(|s| s.ty.clone())
                    .unwrap_or(Type::Any)
            }

            // Compound expressions
            Expression::ArrayExpression(arr) => self.infer_array_literal(arr),
            Expression::ObjectExpression(obj) => self.infer_object_literal(obj),
            Expression::ArrowFunctionExpression(arrow) => self.infer_arrow_function(arrow),
            Expression::FunctionExpression(func) => self.infer_function_expression(func),
            Expression::CallExpression(call) => {
                self.infer_call_expression(call)
            }

            // Member access
            Expression::StaticMemberExpression(member) => {
                let object_type = self.infer_expression(&member.object);
                self.get_property_type(&object_type, member.property.name.as_str())
            }
            Expression::ComputedMemberExpression(member) => {
                self.infer_computed_member_expression(member)
            }

            // Operators
            Expression::BinaryExpression(binary) => self.infer_binary_expression(binary),
            Expression::UnaryExpression(unary) => self.infer_unary_expression(unary),

            // Conditional (ternary)
            Expression::ConditionalExpression(cond) => {
                let consequent = self.infer_expression(&cond.consequent);
                let alternate = self.infer_expression(&cond.alternate);
                self.union_types(consequent, alternate)
            }

            // Logical operators
            Expression::LogicalExpression(logical) => {
                let left = self.infer_expression(&logical.left);
                let right = self.infer_expression(&logical.right);
                match logical.operator {
                    LogicalOperator::And => right, // a && b returns b if a is truthy
                    LogicalOperator::Or => self.union_types(left, right),
                    LogicalOperator::Coalesce => self.union_types(left, right),
                }
            }

            // Assignment returns the assigned value's type
            Expression::AssignmentExpression(assign) => self.infer_expression(&assign.right),

            // Template literals
            Expression::TemplateLiteral(_) => Type::String,
            Expression::TaggedTemplateExpression(_) => Type::Any, // Depends on tag function

            // New expression
            Expression::NewExpression(new_expr) => {
                if let Expression::Identifier(ident) = &new_expr.callee {
                    Type::TypeRef {
                        name: ident.name.to_string(),
                        type_args: vec![],
                    }
                } else {
                    Type::Any
                }
            }

            // Await unwraps Promise
            Expression::AwaitExpression(await_expr) => {
                let inner = self.infer_expression(&await_expr.argument);
                self.unwrap_promise(inner)
            }

            // Other
            Expression::YieldExpression(_) => Type::Any,
            Expression::ThisExpression(_) => Type::Any, // TODO: proper this typing
            Expression::ParenthesizedExpression(paren) => self.infer_expression(&paren.expression),

            // Sequence expression returns last
            Expression::SequenceExpression(seq) => seq
                .expressions
                .last()
                .map(|e| self.infer_expression(e))
                .unwrap_or(Type::Undefined),

            _ => Type::Any,
        }
    }

    /// Infer type of array literal.
    ///
    /// Empty arrays get `never[]`, otherwise union of all element types.
    pub(super) fn infer_array_literal(&self, arr: &ArrayExpression) -> Type {
        if arr.elements.is_empty() {
            return Type::Array(Box::new(Type::Never));
        }

        let mut element_types: Vec<Type> = Vec::new();

        for elem in &arr.elements {
            match elem {
                ArrayExpressionElement::SpreadElement(spread) => {
                    let spread_type = self.infer_expression(&spread.argument);
                    if let Type::Array(inner) = spread_type {
                        element_types.push(*inner);
                    } else {
                        element_types.push(spread_type);
                    }
                }
                ArrayExpressionElement::Elision(_) => {
                    element_types.push(Type::Undefined);
                }
                _ => {
                    if let Some(expr) = elem.as_expression() {
                        let ty = self.infer_expression(expr);
                        // Widen literal types in arrays
                        element_types.push(self.widen_type(ty));
                    }
                }
            }
        }

        // Deduplicate and create union if multiple types
        let unified = self.unify_types(element_types);
        Type::Array(Box::new(unified))
    }

    /// Infer type of object literal.
    ///
    /// Handles:
    /// - Regular properties: `{ x: 1 }`
    /// - Shorthand properties: `{ x }` (equivalent to `{ x: x }`)
    /// - Computed properties: `{ [expr]: value }`
    /// - Method shorthand: `{ foo() {} }`
    pub(super) fn infer_object_literal(&self, obj: &ObjectExpression) -> Type {
        let mut properties = Vec::new();

        for prop in &obj.properties {
            match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    // Handle method shorthand: { foo() {} }
                    if p.method {
                        if let PropertyKey::StaticIdentifier(ident) = &p.key {
                            let ty = self.infer_expression(&p.value);
                            properties.push(Property::new(ident.name.to_string(), ty));
                        }
                        continue;
                    }

                    let name = match &p.key {
                        PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                        PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                        // Computed property: { [expr]: value }
                        _ if p.computed => {
                            // For computed properties with string literal keys, use the value
                            if let PropertyKey::StringLiteral(s) = &p.key {
                                Some(s.value.to_string())
                            } else {
                                // Dynamic computed key would need index signature
                                None
                            }
                        }
                        _ => None,
                    };

                    if let Some(name) = name {
                        // Shorthand property: { x } is equivalent to { x: x }
                        // p.shorthand is true and p.value is the same identifier
                        let ty = self.infer_expression(&p.value);
                        // Widen literal types in object properties
                        let ty = self.widen_type(ty);
                        properties.push(Property::new(name, ty));
                    }
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    // Merge properties from spread object
                    let spread_type = self.infer_expression(&spread.argument);
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

        Type::Object {
            properties,
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        }
    }

    /// Infer type of arrow function.
    fn infer_arrow_function(&self, arrow: &ArrowFunctionExpression) -> Type {
        let params: Vec<Param> = arrow
            .params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern {
                    BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty = p
                    .type_annotation
                    .as_ref()
                    .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                    .unwrap_or(Type::Any);
                Param::new(name, ty)
            })
            .collect();

        let return_type = if let Some(ann) = &arrow.return_type {
            self.resolve_ts_type(&ann.type_annotation)
        } else {
            // Infer from body
            if arrow.expression {
                // Expression body: () => expr
                if let Some(Statement::ExpressionStatement(expr)) = arrow.body.statements.first() {
                    self.infer_expression(&expr.expression)
                } else {
                    Type::Undefined
                }
            } else {
                // Block body: infer from returns
                let returns = self.collect_return_types(&arrow.body);
                if returns.is_empty() {
                    Type::Void
                } else {
                    let types: Vec<Type> = returns.into_iter().map(|(t, _)| t).collect();
                    self.unify_types(types)
                }
            }
        };

        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params: vec![],
        }
    }

    /// Infer type of function expression.
    fn infer_function_expression(&self, func: &Function) -> Type {
        let params: Vec<Param> = func
            .params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern {
                    BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty = p
                    .type_annotation
                    .as_ref()
                    .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                    .unwrap_or(Type::Any);
                Param::new(name, ty)
            })
            .collect();

        let return_type = if let Some(ann) = &func.return_type {
            self.resolve_ts_type(&ann.type_annotation)
        } else if let Some(body) = &func.body {
            let returns = self.collect_return_types(body);
            if returns.is_empty() {
                Type::Void
            } else {
                let types: Vec<Type> = returns.into_iter().map(|(t, _)| t).collect();
                self.unify_types(types)
            }
        } else {
            Type::Void
        };

        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params: vec![],
        }
    }

    /// Infer type of computed member access (obj[expr]).
    ///
    /// Handles:
    /// - String literal key: `obj["prop"]` - property access
    /// - Array indexing: `arr[0]` - returns element type
    /// - Tuple indexing: `tuple[0]` - returns specific element or union
    /// - Index signature: `obj[key]` where key matches index signature key type
    fn infer_computed_member_expression(&self, member: &ComputedMemberExpression) -> Type {
        let object_type = self.infer_expression(&member.object);
        let index_type = self.infer_expression(&member.expression);

        // Resolve TypeRef first
        let resolved_type = if let Type::TypeRef { name, .. } = &object_type {
            self.symbols
                .lookup_type(name)
                .map(|s| s.ty.clone())
                .unwrap_or_else(|| object_type.clone())
        } else {
            object_type.clone()
        };

        // If indexing with string literal, treat as property access
        if let Type::StringLiteral(prop_name) = &index_type {
            return self.get_property_type(&resolved_type, prop_name);
        }

        // Array indexing
        if let Type::Array(element_type) = &resolved_type {
            return (**element_type).clone();
        }

        // Tuple indexing with number literal
        if let Type::Tuple(types) = &resolved_type {
            if let Type::NumberLiteral(n) = index_type {
                let idx = n as usize;
                if idx < types.len() {
                    return types[idx].clone();
                }
            }
            // Unknown index - return union of all tuple types
            return self.unify_types(types.clone());
        }

        // Object with index signature
        if let Type::Object { index_signature, .. } = &resolved_type {
            if let Some(idx_sig) = index_signature {
                // Check if index type is compatible with index signature key type
                let key_matches = match (&index_type, &*idx_sig.key_type) {
                    // String index - accepts string and string literal
                    (Type::String, Type::String) => true,
                    (Type::StringLiteral(_), Type::String) => true,
                    // Number index - accepts number and number literal
                    (Type::Number, Type::Number) => true,
                    (Type::NumberLiteral(_), Type::Number) => true,
                    // Number can also access string index (numbers coerce to strings)
                    (Type::Number, Type::String) => true,
                    (Type::NumberLiteral(_), Type::String) => true,
                    _ => false,
                };
                if key_matches {
                    return (*idx_sig.value_type).clone();
                }
            }
        }

        Type::Any
    }

    /// Get property type from an object type.
    ///
    /// Resolution order:
    /// 1. Resolve TypeRef to its underlying type
    /// 2. For TypeParameter with constraint, use the constraint type
    /// 3. Look for an explicit property with the given name
    /// 4. If not found and there's a string index signature, return its value type
    /// 5. Fall back to Any
    pub(super) fn get_property_type(&self, object_type: &Type, prop_name: &str) -> Type {
        // Resolve TypeRef first
        let resolved_type = if let Type::TypeRef { name, .. } = object_type {
            self.symbols
                .lookup_type(name)
                .map(|s| s.ty.clone())
                .unwrap_or_else(|| object_type.clone())
        } else {
            object_type.clone()
        };

        // For TypeParameter with a constraint, use the constraint for property lookup
        let resolved_type = if let Type::TypeParameter { constraint: Some(constraint), .. } = &resolved_type {
            (**constraint).clone()
        } else {
            resolved_type
        };

        match &resolved_type {
            Type::Object {
                properties,
                index_signature,
                ..
            } => {
                if let Some(prop) = properties.iter().find(|p| p.name == prop_name) {
                    return prop.ty.clone();
                }
                if let Some(idx_sig) = index_signature {
                    if matches!(*idx_sig.key_type, Type::String) {
                        return (*idx_sig.value_type).clone();
                    }
                }
                Type::Any
            }
            Type::Any | Type::Unknown => Type::Any,
            _ => Type::Any,
        }
    }

    /// Infer type of binary expression.
    fn infer_binary_expression(&self, binary: &BinaryExpression) -> Type {
        let left = self.infer_expression(&binary.left);
        let right = self.infer_expression(&binary.right);

        match binary.operator {
            // Arithmetic (except +)
            BinaryOperator::Subtraction
            | BinaryOperator::Multiplication
            | BinaryOperator::Division
            | BinaryOperator::Remainder
            | BinaryOperator::Exponential => Type::Number,

            // Addition - number + number = number, string + any = string
            BinaryOperator::Addition => {
                if self.is_string_like(&left) || self.is_string_like(&right) {
                    Type::String
                } else {
                    Type::Number
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
            | BinaryOperator::StrictInequality => Type::Boolean,

            // Bitwise
            BinaryOperator::BitwiseAnd
            | BinaryOperator::BitwiseOR
            | BinaryOperator::BitwiseXOR
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight
            | BinaryOperator::ShiftRightZeroFill => Type::Number,

            // instanceof / in
            BinaryOperator::Instanceof | BinaryOperator::In => Type::Boolean,
        }
    }

    /// Infer type of unary expression.
    fn infer_unary_expression(&self, unary: &UnaryExpression) -> Type {
        match unary.operator {
            UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus | UnaryOperator::BitwiseNot => {
                Type::Number
            }
            UnaryOperator::LogicalNot => Type::Boolean,
            UnaryOperator::Typeof => Type::String,
            UnaryOperator::Void => Type::Undefined,
            UnaryOperator::Delete => Type::Boolean,
        }
    }

    /// Check if a type is string-like.
    fn is_string_like(&self, ty: &Type) -> bool {
        matches!(ty, Type::String | Type::StringLiteral(_))
    }

    /// Unwrap Promise<T> to T.
    fn unwrap_promise(&self, ty: Type) -> Type {
        if let Type::TypeRef { name, type_args } = ty {
            if name == "Promise" && !type_args.is_empty() {
                return type_args.into_iter().next().expect("checked !is_empty()");
            }
            Type::TypeRef { name, type_args }
        } else {
            ty
        }
    }

    /// Infer the return type of a function call, handling generic type inference.
    fn infer_call_expression(&self, call: &CallExpression) -> Type {
        let callee_type = self.infer_expression(&call.callee);

        if let Type::Function { params, return_type, type_params } = callee_type {
            // If no type parameters, just return the return type
            if type_params.is_empty() {
                return *return_type;
            }

            // Collect argument types
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

            // Build substitution map
            let substitutions = if !explicit_type_args.is_empty() {
                // Use explicit type arguments
                self.build_substitution_map(&type_params, &explicit_type_args)
            } else {
                // Infer type arguments from argument types
                self.infer_type_args_from_call(&type_params, &params, &arg_types)
            };

            // Substitute type parameters in the return type
            if substitutions.is_empty() {
                *return_type
            } else {
                self.substitute_type_params(&return_type, &substitutions)
            }
        } else {
            Type::Any
        }
    }
}
