//! Convert AST type nodes to our Type representation.

use oxc_allocator::Box as OxcBox;
use oxc_ast::ast::*;

use crate::types::{Param, Property, Type, TypeParam};

use super::Binder;

impl Binder {
    /// Get type from annotation if present, otherwise infer from initializer.
    /// Falls back to `any` if neither exists.
    pub(super) fn resolve_binding_type(&self, declarator: &VariableDeclarator) -> Type {
        if let Some(annotation) = &declarator.type_annotation {
            return self.resolve_ts_type(&annotation.type_annotation);
        }

        if let Some(init) = &declarator.init {
            return self.infer_expression_type(init);
        }

        Type::Any
    }

    /// Helper for oxc's arena-allocated Box<TSTypeAnnotation>.
    pub(super) fn resolve_type_annotation_oxc(
        &self,
        annotation: &Option<OxcBox<TSTypeAnnotation>>,
    ) -> Type {
        match annotation {
            Some(ann) => self.resolve_ts_type(&ann.type_annotation),
            None => Type::Any,
        }
    }

    /// Convert oxc's TSType AST node to our Type representation.
    pub(super) fn resolve_ts_type(&self, ts_type: &TSType) -> Type {
        match ts_type {
            // Primitive keywords
            TSType::TSStringKeyword(_) => Type::String,
            TSType::TSNumberKeyword(_) => Type::Number,
            TSType::TSBooleanKeyword(_) => Type::Boolean,
            TSType::TSNullKeyword(_) => Type::Null,
            TSType::TSUndefinedKeyword(_) => Type::Undefined,
            TSType::TSVoidKeyword(_) => Type::Void,
            TSType::TSAnyKeyword(_) => Type::Any,
            TSType::TSUnknownKeyword(_) => Type::Unknown,
            TSType::TSNeverKeyword(_) => Type::Never,

            // Literal types
            TSType::TSLiteralType(lit) => match &lit.literal {
                TSLiteral::StringLiteral(s) => Type::StringLiteral(s.value.to_string()),
                TSLiteral::NumericLiteral(n) => Type::NumberLiteral(n.value),
                TSLiteral::BooleanLiteral(b) => Type::BooleanLiteral(b.value),
                _ => Type::Any,
            },

            // Array types
            TSType::TSArrayType(arr) => {
                Type::Array(Box::new(self.resolve_ts_type(&arr.element_type)))
            }

            // Tuple types
            TSType::TSTupleType(tuple) => {
                let types: Vec<Type> = tuple
                    .element_types
                    .iter()
                    .map(|elem| self.resolve_tuple_element(elem))
                    .collect();
                Type::Tuple(types)
            }

            // Union types
            TSType::TSUnionType(union) => {
                let types: Vec<Type> = union
                    .types
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect();
                Type::Union(types)
            }

            // Intersection types
            TSType::TSIntersectionType(inter) => {
                let types: Vec<Type> = inter
                    .types
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect();
                Type::Intersection(types)
            }

            // Type references (e.g., Array<T>, Promise<T>, custom types)
            TSType::TSTypeReference(type_ref) => {
                let name = match &type_ref.type_name {
                    TSTypeName::IdentifierReference(ident) => ident.name.to_string(),
                    TSTypeName::QualifiedName(qual) => qual.right.name.to_string(),
                    TSTypeName::ThisExpression(_) => "this".to_string(),
                };

                let type_args: Vec<Type> = type_ref
                    .type_arguments
                    .as_ref()
                    .map(|params| {
                        params
                            .params
                            .iter()
                            .map(|t| self.resolve_ts_type(t))
                            .collect()
                    })
                    .unwrap_or_default();

                Type::TypeRef { name, type_args }
            }

            // Function types
            TSType::TSFunctionType(func) => {
                let params: Vec<Param> = func
                    .params
                    .items
                    .iter()
                    .map(|p| {
                        let name = match &p.pattern {
                            BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                            _ => "_".to_string(),
                        };
                        let ty = self.resolve_type_annotation_oxc(&p.type_annotation);
                        Param::new(name, ty)
                    })
                    .collect();

                let return_type = self.resolve_ts_type(&func.return_type.type_annotation);

                Type::Function {
                    params,
                    return_type: Box::new(return_type),
                    type_params: vec![],
                }
            }

            // Type literals (inline object types)
            TSType::TSTypeLiteral(lit) => {
                let properties: Vec<Property> = lit
                    .members
                    .iter()
                    .filter_map(|member| {
                        if let TSSignature::TSPropertySignature(prop) = member {
                            let name = match &prop.key {
                                PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                                PropertyKey::StringLiteral(s) => s.value.to_string(),
                                _ => return None,
                            };
                            let ty = prop
                                .type_annotation
                                .as_ref()
                                .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                                .unwrap_or(Type::Any);
                            let mut property = Property::new(name, ty);
                            if prop.optional {
                                property = property.optional();
                            }
                            if prop.readonly {
                                property = property.readonly();
                            }
                            Some(property)
                        } else {
                            None
                        }
                    })
                    .collect();

                Type::Object {
                    properties,
                    index_signature: None,
                }
            }

            // Parenthesized types
            TSType::TSParenthesizedType(paren) => self.resolve_ts_type(&paren.type_annotation),

            _ => Type::Any,
        }
    }

    /// Tuple elements forms:
    /// - Regular: `[string, number]`
    /// - Optional: `[string, number?]`
    /// - Rest: `[string, ...number[]]`
    /// - Named: `[name: string, age: number]`
    fn resolve_tuple_element(&self, elem: &TSTupleElement) -> Type {
        match elem {
            TSTupleElement::TSOptionalType(opt) => self.resolve_ts_type(&opt.type_annotation),
            TSTupleElement::TSRestType(rest) => self.resolve_ts_type(&rest.type_annotation),
            TSTupleElement::TSNamedTupleMember(named) => {
                self.resolve_tuple_element(&named.element_type)
            }
            _ => {
                if let Some(ty) = elem.as_ts_type() {
                    self.resolve_ts_type(ty)
                } else {
                    Type::Any
                }
            }
        }
    }

    pub(super) fn build_function_type(&self, func: &Function) -> Type {
        let params: Vec<Param> = func
            .params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern {
                    BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty = self.resolve_type_annotation_oxc(&p.type_annotation);
                let mut param = Param::new(name, ty);
                if p.optional {
                    param = param.optional();
                }
                param
            })
            .collect();

        let return_type = func
            .return_type
            .as_ref()
            .map(|ann| self.resolve_ts_type(&ann.type_annotation))
            .unwrap_or(Type::Void);

        let type_params: Vec<TypeParam> = func
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
                        tp
                    })
                    .collect()
            })
            .unwrap_or_default();

        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params,
        }
    }

    /// Build an object type from an interface declaration.
    pub(super) fn build_interface_type(&self, decl: &TSInterfaceDeclaration) -> Type {
        let properties: Vec<Property> = decl
            .body
            .body
            .iter()
            .filter_map(|member| {
                if let TSSignature::TSPropertySignature(prop) = member {
                    let name = match &prop.key {
                        PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                        PropertyKey::StringLiteral(s) => s.value.to_string(),
                        _ => return None,
                    };
                    let ty = prop
                        .type_annotation
                        .as_ref()
                        .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(Type::Any);
                    let mut property = Property::new(name, ty);
                    if prop.optional {
                        property = property.optional();
                    }
                    if prop.readonly {
                        property = property.readonly();
                    }
                    Some(property)
                } else {
                    None
                }
            })
            .collect();

        Type::Object {
            properties,
            index_signature: None,
        }
    }

    /// Infer expression type for initializers (quick inference during binding).
    pub(super) fn infer_expression_type(&self, expr: &Expression) -> Type {
        match expr {
            Expression::StringLiteral(s) => Type::StringLiteral(s.value.to_string()),
            Expression::NumericLiteral(n) => Type::NumberLiteral(n.value),
            Expression::BooleanLiteral(b) => Type::BooleanLiteral(b.value),
            Expression::NullLiteral(_) => Type::Null,
            Expression::ArrayExpression(arr) => {
                let elem_type = self.infer_array_element_type(arr);
                Type::Array(Box::new(elem_type))
            }
            Expression::ObjectExpression(obj) => self.infer_object_type(obj),
            Expression::ArrowFunctionExpression(arrow) => self.infer_arrow_function_type(arrow),
            Expression::FunctionExpression(func) => self.infer_function_expression_type(func),
            Expression::Identifier(ident) => {
                self.symbols
                    .lookup(ident.name.as_str())
                    .map(|s| s.ty.clone())
                    .unwrap_or(Type::Any)
            }
            Expression::BinaryExpression(binary) => self.infer_binary_type(binary),
            Expression::UnaryExpression(unary) => self.infer_unary_type(unary),
            Expression::CallExpression(call) => self.infer_call_type(call),
            Expression::ConditionalExpression(cond) => {
                let consequent = self.infer_expression_type(&cond.consequent);
                let alternate = self.infer_expression_type(&cond.alternate);
                if consequent == alternate {
                    consequent
                } else {
                    Type::Union(vec![consequent, alternate])
                }
            }
            Expression::ParenthesizedExpression(paren) => {
                self.infer_expression_type(&paren.expression)
            }
            Expression::StaticMemberExpression(member) => {
                self.infer_member_access(&member.object, member.property.name.as_str())
            }
            _ => Type::Any,
        }
    }

    fn infer_object_type(&self, obj: &ObjectExpression) -> Type {
        let mut properties = Vec::new();

        for prop in &obj.properties {
            match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    let name = match &p.key {
                        PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                        PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                        _ => None,
                    };

                    if let Some(name) = name {
                        let ty = self.infer_expression_type(&p.value);
                        // Widen literal types in object properties
                        let ty = self.widen_type(ty);
                        properties.push(Property::new(name, ty));
                    }
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    let spread_type = self.infer_expression_type(&spread.argument);
                    if let Type::Object { properties: spread_props, .. } = spread_type {
                        properties.extend(spread_props);
                    }
                }
            }
        }

        Type::Object {
            properties,
            index_signature: None,
        }
    }

    fn infer_array_element_type(&self, arr: &ArrayExpression) -> Type {
        if arr.elements.is_empty() {
            return Type::Never;
        }

        let mut types = Vec::new();
        for elem in &arr.elements {
            if let Some(expr) = elem.as_expression() {
                let ty = self.infer_expression_type(expr);
                let ty = self.widen_type(ty);
                if !types.contains(&ty) {
                    types.push(ty);
                }
            }
        }

        if types.is_empty() {
            Type::Any
        } else if types.len() == 1 {
            types.pop().unwrap()
        } else {
            Type::Union(types)
        }
    }

    fn infer_arrow_function_type(&self, arrow: &ArrowFunctionExpression) -> Type {
        let params: Vec<Param> = arrow
            .params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern {
                    BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty = self.resolve_type_annotation_oxc(&p.type_annotation);
                Param::new(name, ty)
            })
            .collect();

        let return_type = arrow
            .return_type
            .as_ref()
            .map(|ann| self.resolve_ts_type(&ann.type_annotation))
            .unwrap_or(Type::Any);

        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params: vec![],
        }
    }

    fn infer_function_expression_type(&self, func: &Function) -> Type {
        self.build_function_type(func)
    }

    fn infer_binary_type(&self, binary: &BinaryExpression) -> Type {
        match binary.operator {
            BinaryOperator::Addition => {
                let left = self.infer_expression_type(&binary.left);
                let right = self.infer_expression_type(&binary.right);
                if matches!(left, Type::String | Type::StringLiteral(_))
                    || matches!(right, Type::String | Type::StringLiteral(_))
                {
                    Type::String
                } else {
                    Type::Number
                }
            }
            BinaryOperator::Subtraction
            | BinaryOperator::Multiplication
            | BinaryOperator::Division
            | BinaryOperator::Remainder
            | BinaryOperator::Exponential => Type::Number,
            BinaryOperator::LessThan
            | BinaryOperator::LessEqualThan
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterEqualThan
            | BinaryOperator::Equality
            | BinaryOperator::Inequality
            | BinaryOperator::StrictEquality
            | BinaryOperator::StrictInequality
            | BinaryOperator::Instanceof
            | BinaryOperator::In => Type::Boolean,
            _ => Type::Number,
        }
    }

    fn infer_unary_type(&self, unary: &UnaryExpression) -> Type {
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

    fn infer_call_type(&self, call: &CallExpression) -> Type {
        let callee_type = self.infer_expression_type(&call.callee);
        if let Type::Function { return_type, .. } = callee_type {
            *return_type
        } else {
            Type::Any
        }
    }

    fn infer_member_access(&self, object: &Expression, prop_name: &str) -> Type {
        let obj_type = self.infer_expression_type(object);
        match obj_type {
            Type::Object { properties, .. } => {
                properties
                    .iter()
                    .find(|p| p.name == prop_name)
                    .map(|p| p.ty.clone())
                    .unwrap_or(Type::Any)
            }
            Type::Array(_) if prop_name == "length" => Type::Number,
            Type::String | Type::StringLiteral(_) if prop_name == "length" => Type::Number,
            _ => Type::Any,
        }
    }

    pub(super) fn widen_type(&self, ty: Type) -> Type {
        match ty {
            Type::StringLiteral(_) => Type::String,
            Type::NumberLiteral(_) => Type::Number,
            Type::BooleanLiteral(_) => Type::Boolean,
            other => other,
        }
    }
}
