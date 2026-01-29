//! Convert AST type nodes to our Type representation.

use oxc_allocator::Box as OxcBox;
use oxc_ast::ast::*;

use crate::types::resolution;
use crate::types::{Param, Property, Type};

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
    ///
    /// This method handles TSTypeQuery (typeof x) specially since it requires
    /// symbol table access. For types that can contain nested TSTypeQuery,
    /// we recursively resolve them here instead of delegating to resolution.
    pub(super) fn resolve_ts_type(&self, ts_type: &TSType) -> Type {
        use crate::types::{IndexSignature, TypeParam};

        match ts_type {
            // Handle TSTypeQuery (typeof x) - needs symbol table access
            TSType::TSTypeQuery(query) => self.resolve_type_query(query),

            // Handle TSTypeReference with type arguments that might contain typeof
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

            // Handle union types with nested typeof
            TSType::TSUnionType(union) => {
                let types: Vec<Type> = union
                    .types
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect();
                Type::Union(types)
            }

            // Handle intersection types with nested typeof
            TSType::TSIntersectionType(inter) => {
                let types: Vec<Type> = inter
                    .types
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect();
                Type::Intersection(types)
            }

            // Handle array types with nested typeof
            TSType::TSArrayType(arr) => {
                Type::Array(Box::new(self.resolve_ts_type(&arr.element_type)))
            }

            // Handle tuple types with nested typeof
            TSType::TSTupleType(tuple) => {
                let types: Vec<Type> = tuple
                    .element_types
                    .iter()
                    .map(|elem| match elem {
                        TSTupleElement::TSOptionalType(opt) => {
                            self.resolve_ts_type(&opt.type_annotation)
                        }
                        TSTupleElement::TSRestType(rest) => {
                            self.resolve_ts_type(&rest.type_annotation)
                        }
                        _ => resolution::resolve_tuple_element(elem),
                    })
                    .collect();
                Type::Tuple(types)
            }

            // Handle conditional types with nested typeof
            TSType::TSConditionalType(cond) => Type::ConditionalType {
                check_type: Box::new(self.resolve_ts_type(&cond.check_type)),
                extends_type: Box::new(self.resolve_ts_type(&cond.extends_type)),
                true_type: Box::new(self.resolve_ts_type(&cond.true_type)),
                false_type: Box::new(self.resolve_ts_type(&cond.false_type)),
            },

            // Handle function types with nested typeof
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
                        let ty = p
                            .type_annotation
                            .as_ref()
                            .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                            .unwrap_or(Type::Any);
                        let mut param = Param::new(name, ty);
                        if p.optional {
                            param = param.optional();
                        }
                        param
                    })
                    .collect();

                let return_type = self.resolve_ts_type(&func.return_type.type_annotation);

                let type_params: Vec<TypeParam> = func
                    .type_parameters
                    .as_ref()
                    .map(|tps| {
                        tps.params
                            .iter()
                            .map(|p| {
                                let mut tp = TypeParam::new(p.name.name.to_string());
                                if let Some(c) = &p.constraint {
                                    tp = tp.with_constraint(self.resolve_ts_type(c));
                                }
                                if let Some(d) = &p.default {
                                    tp = tp.with_default(self.resolve_ts_type(d));
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
                    type_predicate: None,
                }
            }

            // Handle type literals (object types) with nested typeof
            TSType::TSTypeLiteral(lit) => {
                let mut properties = Vec::new();
                let mut index_signature = None;

                for member in &lit.members {
                    match member {
                        TSSignature::TSPropertySignature(prop) => {
                            if let Some(name) = resolution::get_property_key_name(&prop.key) {
                                let ty = prop
                                    .type_annotation
                                    .as_ref()
                                    .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                                    .unwrap_or(Type::Any);
                                let mut p = Property::new(name, ty);
                                if prop.optional {
                                    p = p.optional();
                                }
                                if prop.readonly {
                                    p = p.readonly();
                                }
                                properties.push(p);
                            }
                        }
                        TSSignature::TSIndexSignature(idx) => {
                            // Index signature: [key: string]: T or [key: number]: T
                            if let Some(param) = idx.parameters.first() {
                                let key_type =
                                    self.resolve_ts_type(&param.type_annotation.type_annotation);
                                let value_type =
                                    self.resolve_ts_type(&idx.type_annotation.type_annotation);
                                index_signature = Some(IndexSignature {
                                    key_type: Box::new(key_type),
                                    value_type: Box::new(value_type),
                                });
                            }
                        }
                        _ => {}
                    }
                }

                Type::Object {
                    properties,
                    index_signature,
                    extends: vec![],
                    type_params: vec![],
                }
            }

            // Handle parenthesized types
            TSType::TSParenthesizedType(paren) => self.resolve_ts_type(&paren.type_annotation),

            // Handle keyof with nested typeof
            TSType::TSTypeOperatorType(op) => match op.operator {
                TSTypeOperatorOperator::Keyof => {
                    Type::KeyOf(Box::new(self.resolve_ts_type(&op.type_annotation)))
                }
                _ => resolution::resolve_ts_type(ts_type),
            },

            // Handle indexed access types with nested typeof
            TSType::TSIndexedAccessType(access) => Type::IndexedAccess {
                object_type: Box::new(self.resolve_ts_type(&access.object_type)),
                index_type: Box::new(self.resolve_ts_type(&access.index_type)),
            },

            // For all other types, delegate to the stateless resolution
            _ => resolution::resolve_ts_type(ts_type),
        }
    }

    /// Resolve a typeof query to the type of the referenced value.
    ///
    /// `typeof getString` -> the function type of getString
    fn resolve_type_query(&self, query: &TSTypeQuery) -> Type {
        match &query.expr_name {
            TSTypeQueryExprName::IdentifierReference(ident) => {
                // Look up the identifier in the symbol table
                let name = ident.name.as_str();
                if let Some(symbol) = self.symbols.lookup(name) {
                    symbol.ty.clone()
                } else {
                    // Symbol not found - return any for now
                    Type::Any
                }
            }
            TSTypeQueryExprName::QualifiedName(qual) => {
                // For qualified names like A.B.C, look up just the last part for simplicity
                // A proper implementation would resolve the full chain
                let name = qual.right.name.as_str();
                if let Some(symbol) = self.symbols.lookup(name) {
                    symbol.ty.clone()
                } else {
                    Type::Any
                }
            }
            TSTypeQueryExprName::TSImportType(_) => {
                // import('foo') type query - not yet implemented
                Type::Any
            }
            TSTypeQueryExprName::ThisExpression(_) => {
                // typeof this - return Any for now
                Type::Any
            }
        }
    }

    /// Build a function type from a Function AST node.
    pub(super) fn build_function_type(&self, func: &Function) -> Type {
        resolution::build_function_type(func)
    }

    /// Build an object type from an interface declaration.
    pub(super) fn build_interface_type(&self, decl: &TSInterfaceDeclaration) -> Type {
        resolution::build_interface_type(decl)
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
            Expression::FunctionExpression(func) => self.build_function_type(func),
            Expression::Identifier(ident) => self
                .symbols
                .lookup(ident.name.as_str())
                .map(|s| s.ty.clone())
                .unwrap_or(Type::Any),
            Expression::BinaryExpression(binary) => self.infer_binary_type(binary),
            Expression::UnaryExpression(unary) => self.infer_unary_type(unary),
            Expression::CallExpression(call) => {
                let callee_type = self.infer_expression_type(&call.callee);
                if let Type::Function { return_type, .. } = callee_type {
                    *return_type
                } else {
                    Type::Any
                }
            }
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
                let obj_type = self.infer_expression_type(&member.object);
                let prop_name = member.property.name.as_str();
                match obj_type {
                    Type::Object { properties, .. } => properties
                        .iter()
                        .find(|p| p.name == prop_name)
                        .map(|p| p.ty.clone())
                        .unwrap_or(Type::Any),
                    Type::Array(_) if prop_name == "length" => Type::Number,
                    Type::String | Type::StringLiteral(_) if prop_name == "length" => Type::Number,
                    _ => Type::Any,
                }
            }
            // New expression returns the class instance type with type arguments
            Expression::NewExpression(new_expr) => {
                if let Expression::Identifier(ident) = &new_expr.callee {
                    // Capture explicit type arguments if provided
                    let type_args = new_expr
                        .type_arguments
                        .as_ref()
                        .map(|args| {
                            args.params
                                .iter()
                                .map(|t| self.resolve_ts_type(t))
                                .collect()
                        })
                        .unwrap_or_default();
                    Type::TypeRef {
                        name: ident.name.to_string(),
                        type_args,
                    }
                } else {
                    Type::Any
                }
            }
            // Class expression returns ClassConstructor type
            Expression::ClassExpression(class) => {
                let (instance_type, constructor_type, static_type) =
                    resolution::build_class_type(class);

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

                // Return ClassConstructor type
                if let Some(Type::Function {
                    params,
                    type_params,
                    ..
                }) = constructor_type
                {
                    Type::ClassConstructor {
                        params,
                        type_params,
                        static_members,
                    }
                } else {
                    Type::ClassConstructor {
                        params: vec![],
                        type_params: class_type_params,
                        static_members,
                    }
                }
            }
            _ => Type::Any,
        }
    }

    fn infer_object_type(&self, obj: &ObjectExpression) -> Type {
        let mut properties = Vec::new();

        for prop in &obj.properties {
            match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    if let Some(name) = resolution::get_property_key_name(&p.key) {
                        let ty = self.infer_expression_type(&p.value);
                        // Widen literal types in object properties
                        let ty = self.widen_type(ty);
                        properties.push(Property::new(name, ty));
                    }
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    let spread_type = self.infer_expression_type(&spread.argument);
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
        let mut params: Vec<Param> = arrow
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

        // Handle rest parameter
        if let Some(rest_param) = &arrow.params.rest {
            let name = match &rest_param.rest.argument {
                BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                _ => "args".to_string(),
            };
            let ty = rest_param
                .type_annotation
                .as_ref()
                .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                .unwrap_or(Type::Array(Box::new(Type::Any)));
            params.push(Param::new(name, ty).rest());
        }

        let return_type = arrow
            .return_type
            .as_ref()
            .map(|ann| self.resolve_ts_type(&ann.type_annotation))
            .unwrap_or(Type::Any);

        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params: vec![],
            type_predicate: None,
        }
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

    /// Widen literal types to their base types.
    pub(super) fn widen_type(&self, ty: Type) -> Type {
        resolution::widen_type(ty)
    }
}
