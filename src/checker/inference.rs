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

            // Identifiers - look up in symbol table
            Expression::Identifier(ident) => self
                .symbols
                .lookup(ident.name.as_str())
                .map(|s| s.ty.clone())
                .unwrap_or(Type::Any),

            // Compound expressions
            Expression::ArrayExpression(arr) => self.infer_array_literal(arr),
            Expression::ObjectExpression(obj) => self.infer_object_literal(obj),
            Expression::ArrowFunctionExpression(arrow) => self.infer_arrow_function(arrow),
            Expression::FunctionExpression(func) => self.infer_function_expression(func),
            Expression::CallExpression(call) => self.infer_call_expression(call),

            // Member access
            Expression::StaticMemberExpression(member) => self.infer_member_expression(member),
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
            Expression::NewExpression(new_expr) => self.infer_new_expression(new_expr),

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
    fn infer_array_literal(&self, arr: &ArrayExpression) -> Type {
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
    fn infer_object_literal(&self, obj: &ObjectExpression) -> Type {
        let mut properties = Vec::new();

        for prop in &obj.properties {
            match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    let name = match &p.key {
                        PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                        _ => None,
                    };

                    if let Some(name) = name {
                        let ty = self.infer_expression(&p.value);
                        // Widen literal types in object properties
                        let ty = self.widen_type(ty);
                        properties.push(Property::new(name, ty));
                    }
                }
                ObjectPropertyKind::SpreadProperty(_) => {
                    // TODO: merge spread properties
                }
            }
        }

        Type::Object {
            properties,
            index_signature: None,
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
                    .map(|ann| self.resolve_type(&ann.type_annotation))
                    .unwrap_or(Type::Any);
                Param::new(name, ty)
            })
            .collect();

        let return_type = if let Some(ann) = &arrow.return_type {
            self.resolve_type(&ann.type_annotation)
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
                    .map(|ann| self.resolve_type(&ann.type_annotation))
                    .unwrap_or(Type::Any);
                Param::new(name, ty)
            })
            .collect();

        let return_type = if let Some(ann) = &func.return_type {
            self.resolve_type(&ann.type_annotation)
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

    /// Infer type of call expression.
    ///
    /// Returns the function's return type, or `any` if not callable.
    fn infer_call_expression(&self, call: &CallExpression) -> Type {
        let callee_type = self.infer_expression(&call.callee);

        if let Type::Function { return_type, .. } = callee_type {
            *return_type
        } else if matches!(callee_type, Type::Any) {
            Type::Any
        } else {
            // Calling non-function
            Type::Any
        }
    }

    /// Infer type of static member access (obj.prop).
    fn infer_member_expression(&self, member: &StaticMemberExpression) -> Type {
        let object_type = self.infer_expression(&member.object);
        let prop_name = member.property.name.as_str();

        self.get_property_type(&object_type, prop_name)
    }

    /// Infer type of computed member access (obj[expr]).
    fn infer_computed_member_expression(&self, member: &ComputedMemberExpression) -> Type {
        let object_type = self.infer_expression(&member.object);
        let index_type = self.infer_expression(&member.expression);

        // If indexing with string literal, treat as property access
        if let Type::StringLiteral(prop_name) = index_type {
            return self.get_property_type(&object_type, &prop_name);
        }

        // Array indexing
        if let Type::Array(element_type) = object_type {
            return *element_type;
        }

        // Tuple indexing with number literal
        if let Type::Tuple(types) = &object_type {
            if let Type::NumberLiteral(n) = index_type {
                let idx = n as usize;
                if idx < types.len() {
                    return types[idx].clone();
                }
            }
            // Unknown index - return union of all tuple types
            return self.unify_types(types.clone());
        }

        Type::Any
    }

    /// Get property type from an object type.
    pub(super) fn get_property_type(&self, object_type: &Type, prop_name: &str) -> Type {
        match object_type {
            Type::Object { properties, .. } => properties
                .iter()
                .find(|p| p.name == prop_name)
                .map(|p| p.ty.clone())
                .unwrap_or(Type::Any),
            Type::Any => Type::Any,
            Type::Unknown => Type::Any, // Property access on unknown is unsafe
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

    /// Infer type of new expression.
    fn infer_new_expression(&self, new_expr: &NewExpression) -> Type {
        // For now, return the callee as a type reference
        if let Expression::Identifier(ident) = &new_expr.callee {
            Type::TypeRef {
                name: ident.name.to_string(),
                type_args: vec![],
            }
        } else {
            Type::Any
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
                return type_args.into_iter().next().unwrap();
            }
            Type::TypeRef { name, type_args }
        } else {
            ty
        }
    }
}
