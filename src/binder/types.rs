//! Convert AST type nodes to our Type representation.

use oxc_allocator::Box as OxcBox;
use oxc_ast::ast::*;

use crate::types::resolution;
use crate::types::{Param, Property, Type, TypeId};

use super::Binder;

impl Binder {
    /// Get type from annotation if present, otherwise infer from initializer.
    /// Falls back to `any` if neither exists.
    pub(super) fn resolve_binding_type(&mut self, declarator: &VariableDeclarator) -> TypeId {
        if let Some(annotation) = &declarator.type_annotation {
            return self.resolve_ts_type(&annotation.type_annotation);
        }

        if let Some(init) = &declarator.init {
            return self.infer_expression_type(init);
        }

        TypeId::ANY
    }

    /// Helper for oxc's arena-allocated Box<TSTypeAnnotation>.
    pub(super) fn resolve_type_annotation_oxc(
        &mut self,
        annotation: &Option<OxcBox<TSTypeAnnotation>>,
    ) -> TypeId {
        match annotation {
            Some(ann) => self.resolve_ts_type(&ann.type_annotation),
            None => TypeId::ANY,
        }
    }

    /// Convert oxc's TSType AST node to our Type representation.
    ///
    /// This method handles TSTypeQuery (typeof x) specially since it requires
    /// symbol table access. For types that can contain nested TSTypeQuery,
    /// we recursively resolve them here instead of delegating to resolution.
    pub(super) fn resolve_ts_type(&mut self, ts_type: &TSType) -> TypeId {
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

                let type_args: Vec<TypeId> = type_ref
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

                self.symbols.arena.type_ref(name, type_args)
            }

            // Handle union types with nested typeof
            TSType::TSUnionType(union) => {
                let types: Vec<TypeId> = union
                    .types
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect();
                self.symbols.arena.union(types)
            }

            // Handle intersection types with nested typeof
            TSType::TSIntersectionType(inter) => {
                let types: Vec<TypeId> = inter
                    .types
                    .iter()
                    .map(|t| self.resolve_ts_type(t))
                    .collect();
                self.symbols.arena.intersection(types)
            }

            // Handle array types with nested typeof
            TSType::TSArrayType(arr) => {
                let elem_id = self.resolve_ts_type(&arr.element_type);
                self.symbols.arena.array(elem_id)
            }

            // Handle tuple types with nested typeof
            TSType::TSTupleType(tuple) => {
                let types: Vec<TypeId> = tuple
                    .element_types
                    .iter()
                    .map(|elem| match elem {
                        TSTupleElement::TSOptionalType(opt) => {
                            self.resolve_ts_type(&opt.type_annotation)
                        }
                        TSTupleElement::TSRestType(rest) => {
                            self.resolve_ts_type(&rest.type_annotation)
                        }
                        _ => resolution::resolve_tuple_element(elem, &mut self.symbols.arena),
                    })
                    .collect();
                self.symbols.arena.tuple(types)
            }

            // Handle conditional types with nested typeof
            TSType::TSConditionalType(cond) => {
                let check_id = self.resolve_ts_type(&cond.check_type);
                let extends_id = self.resolve_ts_type(&cond.extends_type);
                let true_id = self.resolve_ts_type(&cond.true_type);
                let false_id = self.resolve_ts_type(&cond.false_type);
                self.symbols.arena.intern(Type::ConditionalType {
                    check_type: check_id,
                    extends_type: extends_id,
                    true_type: true_id,
                    false_type: false_id,
                })
            }

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

                let return_type_id = self.resolve_ts_type(&func.return_type.type_annotation);

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

                self.symbols.arena.intern(Type::Function {
                    params,
                    return_type: return_type_id,
                    type_params,
                    type_predicate: None,
                })
            }

            // Handle type literals (object types) with nested typeof
            TSType::TSTypeLiteral(lit) => {
                let mut properties = Vec::new();
                let mut index_signature = None;

                for member in &lit.members {
                    match member {
                        TSSignature::TSPropertySignature(prop) => {
                            if let Some(name) = resolution::get_property_key_name(&prop.key) {
                                let ty_id = prop
                                    .type_annotation
                                    .as_ref()
                                    .map(|ann| self.resolve_ts_type(&ann.type_annotation))
                                    .unwrap_or(TypeId::ANY);
                                let mut p = Property::new(name, ty_id);
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
                                let key_type_id =
                                    self.resolve_ts_type(&param.type_annotation.type_annotation);
                                let value_type_id =
                                    self.resolve_ts_type(&idx.type_annotation.type_annotation);
                                index_signature = Some(IndexSignature {
                                    key_type: key_type_id,
                                    value_type: value_type_id,
                                });
                            }
                        }
                        _ => {}
                    }
                }

                self.symbols.arena.intern(Type::Object {
                    properties,
                    index_signature,
                    extends: vec![],
                    type_params: vec![],
                })
            }

            // Handle parenthesized types
            TSType::TSParenthesizedType(paren) => self.resolve_ts_type(&paren.type_annotation),

            // Handle keyof with nested typeof
            TSType::TSTypeOperatorType(op) => match op.operator {
                TSTypeOperatorOperator::Keyof => {
                    let inner_id = self.resolve_ts_type(&op.type_annotation);
                    self.symbols.arena.intern(Type::KeyOf(inner_id))
                }
                _ => resolution::resolve_ts_type(ts_type, &mut self.symbols.arena),
            },

            // Handle indexed access types with nested typeof
            TSType::TSIndexedAccessType(access) => {
                let object_id = self.resolve_ts_type(&access.object_type);
                let index_id = self.resolve_ts_type(&access.index_type);
                self.symbols.arena.intern(Type::IndexedAccess {
                    object_type: object_id,
                    index_type: index_id,
                })
            }

            // For all other types, delegate to the stateless resolution
            _ => resolution::resolve_ts_type(ts_type, &mut self.symbols.arena),
        }
    }

    /// Resolve a typeof query to the type of the referenced value.
    ///
    /// `typeof getString` -> the function type of getString
    fn resolve_type_query(&self, query: &TSTypeQuery) -> TypeId {
        match &query.expr_name {
            TSTypeQueryExprName::IdentifierReference(ident) => {
                // Look up the identifier in the symbol table
                let name = ident.name.as_str();
                if let Some(symbol) = self.symbols.lookup(name) {
                    symbol.ty
                } else {
                    // Symbol not found - return any for now
                    TypeId::ANY
                }
            }
            TSTypeQueryExprName::QualifiedName(qual) => {
                // For qualified names like A.B.C, look up just the last part for simplicity
                // A proper implementation would resolve the full chain
                let name = qual.right.name.as_str();
                if let Some(symbol) = self.symbols.lookup(name) {
                    symbol.ty
                } else {
                    TypeId::ANY
                }
            }
            TSTypeQueryExprName::TSImportType(_) => {
                // import('foo') type query - not yet implemented
                TypeId::ANY
            }
            TSTypeQueryExprName::ThisExpression(_) => {
                // typeof this - return Any for now
                TypeId::ANY
            }
        }
    }

    /// Build a function type from a Function AST node.
    pub(super) fn build_function_type(&mut self, func: &Function) -> TypeId {
        resolution::build_function_type(func, &mut self.symbols.arena)
    }

    /// Build an object type from an interface declaration.
    pub(super) fn build_interface_type(&mut self, decl: &TSInterfaceDeclaration) -> TypeId {
        resolution::build_interface_type(decl, &mut self.symbols.arena)
    }

    /// Infer expression type for initializers (quick inference during binding).
    pub(super) fn infer_expression_type(&mut self, expr: &Expression) -> TypeId {
        match expr {
            Expression::StringLiteral(s) => {
                self.symbols
                    .arena
                    .intern(Type::StringLiteral(s.value.to_string()))
            }
            Expression::NumericLiteral(n) => {
                self.symbols.arena.intern(Type::NumberLiteral(n.value))
            }
            Expression::BooleanLiteral(b) => {
                if b.value {
                    TypeId::TRUE
                } else {
                    TypeId::FALSE
                }
            }
            Expression::NullLiteral(_) => TypeId::NULL,
            Expression::ArrayExpression(arr) => {
                let elem_type_id = self.infer_array_element_type(arr);
                self.symbols.arena.array(elem_type_id)
            }
            Expression::ObjectExpression(obj) => self.infer_object_type(obj),
            Expression::ArrowFunctionExpression(arrow) => self.infer_arrow_function_type(arrow),
            Expression::FunctionExpression(func) => self.build_function_type(func),
            Expression::Identifier(ident) => self
                .symbols
                .lookup(ident.name.as_str())
                .map(|s| s.ty)
                .unwrap_or(TypeId::ANY),
            Expression::BinaryExpression(binary) => self.infer_binary_type(binary),
            Expression::UnaryExpression(unary) => self.infer_unary_type(unary),
            Expression::CallExpression(call) => {
                let callee_type_id = self.infer_expression_type(&call.callee);
                let callee_type = self.symbols.arena.get(callee_type_id).clone();
                if let Type::Function { return_type, .. } = callee_type {
                    return_type
                } else {
                    TypeId::ANY
                }
            }
            Expression::ConditionalExpression(cond) => {
                let consequent_id = self.infer_expression_type(&cond.consequent);
                let alternate_id = self.infer_expression_type(&cond.alternate);
                if consequent_id == alternate_id {
                    consequent_id
                } else {
                    self.symbols.arena.union(vec![consequent_id, alternate_id])
                }
            }
            Expression::ParenthesizedExpression(paren) => {
                self.infer_expression_type(&paren.expression)
            }
            Expression::StaticMemberExpression(member) => {
                let obj_type_id = self.infer_expression_type(&member.object);
                let prop_name = member.property.name.as_str();
                let obj_type = self.symbols.arena.get(obj_type_id).clone();
                match obj_type {
                    Type::Object { properties, .. } => properties
                        .iter()
                        .find(|p| p.name == prop_name)
                        .map(|p| p.ty)
                        .unwrap_or(TypeId::ANY),
                    Type::Array(_) if prop_name == "length" => TypeId::NUMBER,
                    Type::String | Type::StringLiteral(_) if prop_name == "length" => {
                        TypeId::NUMBER
                    }
                    _ => TypeId::ANY,
                }
            }
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
                    self.symbols.arena.type_ref(ident.name.to_string(), type_args)
                } else {
                    TypeId::ANY
                }
            }
            // Class expression returns ClassConstructor type
            Expression::ClassExpression(class) => {
                let (instance_type_id, constructor_type_id, static_type_id) =
                    resolution::build_class_type(class, &mut self.symbols.arena);

                // Extract type params from instance_type
                let instance_type = self.symbols.arena.get(instance_type_id).clone();
                let class_type_params = if let Type::Object { type_params, .. } = &instance_type {
                    type_params.clone()
                } else {
                    vec![]
                };

                // Extract static members from static_type
                let static_type = self.symbols.arena.get(static_type_id).clone();
                let static_members = if let Type::Object { properties, .. } = static_type {
                    properties
                } else {
                    vec![]
                };

                // Return ClassConstructor type
                if let Some(ctor_id) = constructor_type_id {
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
                    self.symbols.arena.intern(Type::ClassConstructor {
                        params: vec![],
                        type_params: class_type_params,
                        static_members,
                    })
                }
            }
            _ => TypeId::ANY,
        }
    }

    fn infer_object_type(&mut self, obj: &ObjectExpression) -> TypeId {
        let mut properties = Vec::new();

        for prop in &obj.properties {
            match prop {
                ObjectPropertyKind::ObjectProperty(p) => {
                    if let Some(name) = resolution::get_property_key_name(&p.key) {
                        let ty_id = self.infer_expression_type(&p.value);
                        // Widen literal types in object properties
                        let ty_id = self.widen_type(ty_id);
                        properties.push(Property::new(name, ty_id));
                    }
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    let spread_type_id = self.infer_expression_type(&spread.argument);
                    let spread_type = self.symbols.arena.get(spread_type_id).clone();
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

        self.symbols.arena.object(properties)
    }

    fn infer_array_element_type(&mut self, arr: &ArrayExpression) -> TypeId {
        if arr.elements.is_empty() {
            return TypeId::NEVER;
        }

        let mut type_ids = Vec::new();
        for elem in &arr.elements {
            if let Some(expr) = elem.as_expression() {
                let ty_id = self.infer_expression_type(expr);
                let ty_id = self.widen_type(ty_id);
                if !type_ids.contains(&ty_id) {
                    type_ids.push(ty_id);
                }
            }
        }

        if type_ids.is_empty() {
            TypeId::ANY
        } else if type_ids.len() == 1 {
            type_ids.pop().unwrap()
        } else {
            self.symbols.arena.union(type_ids)
        }
    }

    fn infer_arrow_function_type(&mut self, arrow: &ArrowFunctionExpression) -> TypeId {
        let mut params: Vec<Param> = arrow
            .params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern {
                    BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty_id = self.resolve_type_annotation_oxc(&p.type_annotation);
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
                .unwrap_or_else(|| self.symbols.arena.array(TypeId::ANY));
            params.push(Param::new(name, ty_id).rest());
        }

        let return_type_id = arrow
            .return_type
            .as_ref()
            .map(|ann| self.resolve_ts_type(&ann.type_annotation))
            .unwrap_or(TypeId::ANY);

        self.symbols.arena.function(params, return_type_id)
    }

    fn infer_binary_type(&mut self, binary: &BinaryExpression) -> TypeId {
        match binary.operator {
            BinaryOperator::Addition => {
                let left_id = self.infer_expression_type(&binary.left);
                let right_id = self.infer_expression_type(&binary.right);
                let left = self.symbols.arena.get(left_id).clone();
                let right = self.symbols.arena.get(right_id).clone();
                if matches!(left, Type::String | Type::StringLiteral(_))
                    || matches!(right, Type::String | Type::StringLiteral(_))
                {
                    TypeId::STRING
                } else {
                    TypeId::NUMBER
                }
            }
            BinaryOperator::Subtraction
            | BinaryOperator::Multiplication
            | BinaryOperator::Division
            | BinaryOperator::Remainder
            | BinaryOperator::Exponential => TypeId::NUMBER,
            BinaryOperator::LessThan
            | BinaryOperator::LessEqualThan
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterEqualThan
            | BinaryOperator::Equality
            | BinaryOperator::Inequality
            | BinaryOperator::StrictEquality
            | BinaryOperator::StrictInequality
            | BinaryOperator::Instanceof
            | BinaryOperator::In => TypeId::BOOLEAN,
            _ => TypeId::NUMBER,
        }
    }

    fn infer_unary_type(&self, unary: &UnaryExpression) -> TypeId {
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

    /// Widen literal types to their base types.
    pub(super) fn widen_type(&mut self, ty_id: TypeId) -> TypeId {
        resolution::widen_type(ty_id, &mut self.symbols.arena)
    }
}
